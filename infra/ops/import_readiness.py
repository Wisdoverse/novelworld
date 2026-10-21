#!/usr/bin/env python3
"""Capture one private, fixed-cohort import-readiness observation. No SLO verdict.

Bad stays zero: current owner rows cannot rule out prior on-time readiness.
"""

import argparse
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import uuid

MAX_BYTES = 1024 * 1024
MAX_JOBS = 1000
MAX_CUTOFF = 365 * 24 * 60 * 60


class Failure(Exception):
    """Only fixed, content-free codes may cross the CLI boundary."""


def require(condition, code):
    if not condition:
        raise Failure(code)


def keys(value, expected, code):
    require(isinstance(value, dict) and set(value) == set(expected.split()), code)


def timestamp(value):
    require(isinstance(value, str) and re.fullmatch(
        r"\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:\.\d{1,6})?(?:Z|[+-](?:[01]\d|2[0-3]):[0-5]\d)", value),
        "invalid_timestamp")
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)
    except (ValueError, OverflowError):
        raise Failure("invalid_timestamp") from None


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def parse_json(value):
    try:
        return json.loads(value, object_pairs_hook=unique_object)
    except (ValueError, UnicodeError, RecursionError):
        raise Failure("invalid_json") from None


def read_json(path, private=False):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_size <= MAX_BYTES, "invalid_file")
        if private:
            require(info.st_uid == os.geteuid() and stat.S_IMODE(info.st_mode) == 0o600
                    and info.st_nlink == 1, "unsafe_evidence_permissions")
        value = stream.read(MAX_BYTES + 1)
        require(len(value) <= MAX_BYTES, "invalid_file")
    return parse_json(value)


def registration(path):
    value = read_json(path)
    keys(value, "version source_sha profile database cutoff_seconds jobs", "invalid_registration")
    require(type(value["version"]) is int and value["version"] == 1, "invalid_registration")
    require(isinstance(value["source_sha"], str)
            and re.fullmatch(r"[0-9a-f]{40}", value["source_sha"]), "invalid_registration")
    require(value["profile"] == "single-node-v1", "invalid_registration")
    keys(value["database"], "system_identifier name", "invalid_registration")
    require(isinstance(value["database"]["system_identifier"], str)
            and re.fullmatch(r"[1-9][0-9]{0,19}", value["database"]["system_identifier"]),
            "invalid_registration")
    require(isinstance(value["database"]["name"], str)
            and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]{0,62}", value["database"]["name"]),
            "invalid_registration")
    require(type(value["cutoff_seconds"]) is int
            and 0 < value["cutoff_seconds"] <= MAX_CUTOFF, "invalid_registration")
    require(isinstance(value["jobs"], list) and 0 < len(value["jobs"]) <= MAX_JOBS,
            "invalid_registration")
    seen = set()
    for job in value["jobs"]:
        keys(job, "novel_id created_at", "invalid_registration")
        try:
            require(str(uuid.UUID(job["novel_id"])) == job["novel_id"], "invalid_registration")
        except (ValueError, TypeError, AttributeError):
            raise Failure("invalid_registration") from None
        require(job["novel_id"] not in seen, "duplicate_job")
        seen.add(job["novel_id"])
        origin = timestamp(job["created_at"])
        try:
            origin + timedelta(seconds=value["cutoff_seconds"])
        except OverflowError:
            raise Failure("invalid_registration") from None
    digest = hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return value, digest


def query(reg):
    ids = ",".join("'" + job["novel_id"] + "'::uuid" for job in reg["jobs"])
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL statement_timeout = '5s';
SET LOCAL lock_timeout = '1s';
SET LOCAL TIME ZONE 'UTC';
SELECT json_build_object(
    'database', json_build_object('name', current_database(),
        'system_identifier', (SELECT system_identifier::text FROM pg_control_system())),
    'read_only', current_setting('transaction_read_only'),
    'rows', (SELECT json_agg(json_build_object(
        'novel_id', enrolled.id, 'created_at', job.created_at,
        'job_status', job.status, 'novel_status', novel.status) ORDER BY enrolled.id)
        FROM unnest(ARRAY[{ids}]) AS enrolled(id)
        LEFT JOIN public.novel_import_jobs AS job ON job.novel_id = enrolled.id
        LEFT JOIN public.novels AS novel ON novel.id = enrolled.id));
SELECT json_build_object('observed_at', clock_timestamp());
COMMIT;
"""


def collect(reg, container, db_user):
    require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", container), "invalid_container")
    require(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]{0,62}", db_user), "invalid_database_user")
    try:
        result = subprocess.run(
            ["docker", "exec", "-i", container, "psql", "-X", "-qAt", "-w",
             "-v", "ON_ERROR_STOP=1", "-U", db_user, "-d", reg["database"]["name"]],
            input=query(reg), text=True, capture_output=True, timeout=15, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise Failure("collection_failed") from None
    require(result.returncode == 0 and len(result.stdout) <= MAX_BYTES, "collection_failed")
    lines = result.stdout.splitlines()
    require(len(lines) == 2, "invalid_snapshot")
    snapshot, clock = map(parse_json, lines)
    keys(snapshot, "database read_only rows", "invalid_snapshot")
    keys(clock, "observed_at", "invalid_snapshot")
    return {**snapshot, **clock}


def validate_evidence(value, reg, digest):
    keys(value, "version registration_sha256 first_observed_at last_observed_at observations jobs",
         "invalid_evidence")
    require(type(value["version"]) is int and value["version"] == 1, "invalid_evidence")
    require(value["registration_sha256"] == digest, "registration_changed")
    require(type(value["observations"]) is int and value["observations"] > 0, "invalid_evidence")
    first, last = timestamp(value["first_observed_at"]), timestamp(value["last_observed_at"])
    require(first <= last, "invalid_evidence")
    expected = {job["novel_id"]: timestamp(job["created_at"]) for job in reg["jobs"]}
    require(isinstance(value["jobs"], dict) and set(value["jobs"]) == set(expected), "invalid_evidence")
    for job_id, origin in expected.items():
        cutoff = origin + timedelta(seconds=reg["cutoff_seconds"])
        require(origin <= first < cutoff, "invalid_evidence")
        good = value["jobs"][job_id]
        require(good is None or first <= timestamp(good) <= min(last, cutoff), "invalid_evidence")


def merge(reg, digest, previous, snapshot):
    keys(snapshot, "database read_only rows observed_at", "invalid_snapshot")
    require(snapshot["database"] == reg["database"], "database_identity_mismatch")
    require(snapshot["read_only"] == "on", "snapshot_not_read_only")
    observed = timestamp(snapshot["observed_at"])
    if previous:
        validate_evidence(previous, reg, digest)
        require(observed >= timestamp(previous["last_observed_at"]), "database_clock_regressed")
    expected = {job["novel_id"]: timestamp(job["created_at"]) for job in reg["jobs"]}
    require(isinstance(snapshot["rows"], list) and len(snapshot["rows"]) == len(expected),
            "invalid_snapshot")
    rows = {}
    for row in snapshot["rows"]:
        keys(row, "novel_id created_at job_status novel_status", "invalid_snapshot")
        job_id = row["novel_id"]
        require(isinstance(job_id, str) and job_id in expected and job_id not in rows, "invalid_snapshot")
        require(row["job_status"] in (None, "pending", "in_progress", "failed", "completed")
                and row["novel_status"] in (None, "pending", "parsing", "error", "ready"),
                "invalid_snapshot")
        require((row["created_at"] is None) == (row["job_status"] is None), "invalid_snapshot")
        if row["created_at"] is not None:
            require(timestamp(row["created_at"]) == expected[job_id], "job_origin_mismatch")
        require(observed >= expected[job_id], "database_clock_regressed")
        rows[job_id] = row
    if previous is None:
        require(all(row["job_status"] is not None and row["novel_status"] is not None
                    for row in rows.values()), "initial_enrollment_incomplete")
        require(all(observed < origin + timedelta(seconds=reg["cutoff_seconds"])
                    for origin in expected.values()), "initial_enrollment_late")
    good = dict(previous["jobs"]) if previous else dict.fromkeys(expected)
    for job_id, row in rows.items():
        if (good[job_id] is None and row["job_status"] == "completed"
                and row["novel_status"] == "ready"
                and observed <= expected[job_id] + timedelta(seconds=reg["cutoff_seconds"])):
            good[job_id] = observed.isoformat()
    evidence = {
        "version": 1, "registration_sha256": digest,
        "first_observed_at": previous["first_observed_at"] if previous else observed.isoformat(),
        "last_observed_at": observed.isoformat(),
        "observations": previous["observations"] + 1 if previous else 1, "jobs": good,
    }
    counts = {"denominator": len(expected), "good": 0, "bad": 0, "unknown": 0, "open": 0}
    latencies = []
    for job_id, origin in expected.items():
        if good[job_id] is not None:
            counts["good"] += 1
            latencies.append((timestamp(good[job_id]) - origin).total_seconds())
        elif observed < origin + timedelta(seconds=reg["cutoff_seconds"]):
            counts["open"] += 1
        else:
            counts["unknown"] += 1
    counts["observed_jobs"] = sum(row["job_status"] is not None and row["novel_status"] is not None
                                  for row in rows.values())
    counts["missing_jobs"] = len(expected) - counts["observed_jobs"]
    # shortcut: owner rows have no first-ready history; Bad needs separate historical evidence.
    counts["readiness_latency_upper_bound_seconds"] = {
        "count": len(latencies), "min": min(latencies, default=None), "max": max(latencies, default=None)}
    return evidence, counts


@contextmanager
def evidence_lock(path):
    fd = os.open(str(path) + ".lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    try:
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and info.st_uid == os.geteuid()
                and stat.S_IMODE(info.st_mode) == 0o600 and info.st_nlink == 1, "unsafe_lock_file")
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Failure("observation_in_progress") from None
        yield
    finally:
        os.close(fd)


def save(path, value):
    fd, temporary = tempfile.mkstemp(prefix=".import-readiness-", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            os.fchmod(stream.fileno(), 0o600)
            json.dump(value, stream, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def observe(registration_path, evidence_path, container, db_user):
    path = Path(evidence_path)
    with evidence_lock(path):
        reg, digest = registration(registration_path)
        try:
            previous = read_json(path, private=True)
        except FileNotFoundError:
            previous = None
        else:
            validate_evidence(previous, reg, digest)
        evidence, counts = merge(reg, digest, previous, collect(reg, container, db_user))
        save(path, evidence)
        return counts


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise Failure("invalid_arguments")


def main(argv=None):
    parser = Parser(description=__doc__)
    for name in ("registration", "evidence", "container", "db-user"):
        parser.add_argument("--" + name, required=True)
    try:
        args = parser.parse_args(argv)
        result = observe(args.registration, args.evidence, args.container, args.db_user)
    except Failure as exc:
        print(json.dumps({"error": str(exc)}), file=sys.stderr)
        return 1
    except (OSError, ValueError, TypeError, OverflowError, RecursionError, KeyboardInterrupt):
        print('{"error": "observation_failed"}', file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
