"""Private release adapter for the fixed diagnostic budget; no provider I/O.

release.sh captures this source and the one profile before any git checkout.
Compose JSON (including credentials) is consumed only in memory, never reported.
"""
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import re
import selectors
import subprocess
import sys
import time
import uuid

SERVICES = ("user-service", "novel-service", "agent-service", "narrative-service")
MAX_OUTPUT = 4096
PROBE_REASONS = frozenset(("child_nonzero", "deadline", "output_overflow", "spawn_error",
                           "read_error", "wait_error", "invalid_capability", "create_ack_invalid",
                           "create_unconfirmed", "cleanup_nonzero", "cleanup_timeout",
                           "cleanup_query_deadline", "cleanup_query_error", "cleanup_residue",
                           "reap_timeout", "reap_error", "unknown"))
PROBE_PHASES = frozenset(("probe_create", "probe_start", "probe_expectation", "probe_cleanup_rm",
                          "probe_cleanup_ps", "probe_cleanup_unproven"))


class Invalid(Exception):
    pass


def _closed_detail(value):
    if value is None:
        return None
    value = value if isinstance(value, dict) else {}
    reason, phase = value.get("reason"), value.get("phase")
    exit_code, elapsed = value.get("exit_code"), value.get("elapsed")
    return {
        "reason": reason if isinstance(reason, str) and reason in PROBE_REASONS else "unknown",
        "phase": phase if isinstance(phase, str) and phase in PROBE_PHASES else "unknown",
        "exit_code": exit_code if type(exit_code) is int and -128 <= exit_code <= 255 else None,
        "elapsed": round(elapsed, 3) if type(elapsed) in (int, float)
        and 0 <= elapsed <= 86400 and math.isfinite(elapsed) else None,
    }


class ProbeInvalid(Invalid):
    """A capability refusal; arbitrary child text never enters its evidence."""

    def __init__(self, reason, *, exit_code=None, elapsed=None):
        super().__init__()
        self.primary_evidence = _closed_detail({"reason": reason, "exit_code": exit_code,
                                               "elapsed": elapsed})
        self.reason = self.primary_evidence["reason"]
        self.exit_code = self.primary_evidence["exit_code"]
        self.elapsed = self.primary_evidence["elapsed"]


def probe_evidence(error):
    """Allowlist every exported field, including error attributes and nested outcomes."""
    phase, name = getattr(error, "probe_phase", None), getattr(error, "probe_name", None)
    identifier = getattr(error, "probe_container_id", None)
    cleanup = getattr(error, "cleanup_evidence", None)
    return {
        "outcome": "invalid" if isinstance(error, ProbeInvalid) else
                   "cleanup_uncertain" if isinstance(error, OSError) else "unknown",
        "phase": phase if isinstance(phase, str) and phase in PROBE_PHASES else "unknown",
        "name": name if isinstance(name, str) and re.fullmatch(
            r"nwq-[a-f0-9]{10}-budget-probe-[a-f0-9]{12}", name) else None,
        "container_id": identifier if isinstance(identifier, str) and re.fullmatch(
            r"[0-9a-f]{64}", identifier) else None,
        "primary": _closed_detail(getattr(error, "primary_evidence", None)),
        "reap": _closed_detail(getattr(error, "reap_evidence", None)),
        "cleanup": [_closed_detail(item) for item in cleanup[:4]]
                   if isinstance(cleanup, list) else [],
    }


def require(condition):
    if not condition:
        raise Invalid()


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result)
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=pairs)


def registration(profile_bytes, environment):
    profile = strict_json(profile_bytes)
    budget_id = environment.get("LLM_DIAGNOSTIC_BUDGET_ID", "")
    parsed_id = uuid.UUID(budget_id)
    require(str(parsed_id) == budget_id and parsed_id.version == 4
            and parsed_id.variant == uuid.RFC_4122)
    raw = environment.get("LLM_DIAGNOSTIC_BUDGET_LIMITS", "")
    require(len(raw.encode()) <= MAX_OUTPUT)
    limits = strict_json(raw)
    require(set(limits) == {"profile", "max_attempts", "max_tokens", "max_cost_micro_cny", "expires_at"})
    require(limits["profile"] == profile["profile"])
    for key in ("attempts", "tokens", "cost_micro_cny"):
        value = limits["max_" + key]
        require(type(value) is int and 0 <= value <= profile["max_limits"][key])
    expiry = limits["expires_at"]
    require(isinstance(expiry, str) and re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", expiry))
    datetime.datetime.strptime(expiry, "%Y-%m-%dT%H:%M:%SZ")
    # Only the owner/database checks first-provision future/+4h. Restarts never renew expiry.
    return {"binding": {"contract": profile["contract"], "profile": profile["profile"],
                        "profile_sha256": hashlib.sha256(profile_bytes).hexdigest(),
                        "budget_id": budget_id}, "limits": limits}


def sync_directory(directory):
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def read_marker(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= MAX_OUTPUT)
    return strict_json(path.read_bytes())


def marker_action(action, state, registered):
    path = state / "diagnostic-provisioning.json"
    started = {**registered, "completed": False}
    complete = {**registered, "completed": True}
    if action == "adopt":
        require(not os.path.lexists(path))
    elif action == "provision-start":
        # O_EXCL is also the lost-ACK fence: this file is never removed automatically.
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(json.dumps(started, sort_keys=True).encode())
            stream.flush()
            os.fsync(stream.fileno())
        sync_directory(state)
    elif action == "provision-complete":
        require(read_marker(path) == started)
        temporary = state / (".diagnostic-complete-" + uuid.uuid4().hex)
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(json.dumps(complete, sort_keys=True).encode())
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        sync_directory(state)
    else:
        require(action in ("upgrade", "rollback", "preflight"))
        require(read_marker(path) == complete)


def bounded_output(command, timeout, limit=MAX_OUTPUT):
    """Bound output while reading, not after communicate() has buffered it."""
    started = time.monotonic()
    try:
        process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL)
    except OSError:
        raise ProbeInvalid("spawn_error", elapsed=time.monotonic() - started)
    output = bytearray()
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ProbeInvalid("deadline", elapsed=time.monotonic() - started)
                try:
                    ready = selector.select(remaining)
                except OSError:
                    raise ProbeInvalid("read_error", elapsed=time.monotonic() - started)
                if not ready:
                    raise ProbeInvalid("deadline", elapsed=time.monotonic() - started)
                try:
                    chunk = os.read(process.stdout.fileno(), min(4096, limit + 1 - len(output)))
                except OSError:
                    raise ProbeInvalid("read_error", elapsed=time.monotonic() - started)
                if not chunk:
                    try:
                        exit_code = process.wait(timeout=max(0.001, deadline - time.monotonic()))
                    except subprocess.TimeoutExpired:
                        raise ProbeInvalid("deadline", elapsed=time.monotonic() - started)
                    except OSError:
                        raise ProbeInvalid("wait_error", elapsed=time.monotonic() - started)
                    if exit_code != 0:
                        raise ProbeInvalid("child_nonzero", exit_code=exit_code,
                                           elapsed=time.monotonic() - started)
                    return bytes(output)
                output.extend(chunk)
                if len(output) > limit:
                    raise ProbeInvalid("output_overflow", elapsed=time.monotonic() - started)
    except OSError as error:
        raise ProbeInvalid("read_error", elapsed=time.monotonic() - started) from error
    finally:
        # Capture the original exception before handling any stop/reap exception.
        active = sys.exc_info()[1]
        stop_started = time.monotonic()
        try:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
        except (OSError, subprocess.SubprocessError) as error:
            failure = OSError("diagnostic probe process reap unproven")
            failure.primary_evidence = _closed_detail(getattr(active, "primary_evidence", None))
            failure.reap_evidence = _closed_detail({
                "reason": "reap_timeout" if isinstance(error, subprocess.TimeoutExpired) else "reap_error",
                "elapsed": time.monotonic() - stop_started,
            })
            raise failure from error
        finally:
            process.stdout.close()


def probe(image, project, expected):
    require(isinstance(image, str) and re.fullmatch(r"[a-z0-9][a-z0-9._/:@-]*@sha256:[0-9a-f]{64}", image))
    name = project + "-budget-probe-" + uuid.uuid4().hex[:12]
    created = False
    primary_error = None
    acknowledged_id = None
    deadline = time.monotonic() + 10
    phase = "probe_create"
    try:
        # A killed `docker run` client can leave a late-created daemon object.
        # Confirm creation before starting; a lost create ACK is not absence.
        identifier = bounded_output([
            "docker", "create", "--pull", "never", "--name", name,
            "--network", "none", "--no-healthcheck", "--log-driver", "none",
            "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--entrypoint", "/app/service", image, "--diagnostic-budget-contract",
        ], 10)
        if not re.fullmatch(rb"[0-9a-f]{64}\n?", identifier):
            raise ProbeInvalid("create_ack_invalid")
        acknowledged_id = identifier.decode("ascii").strip()
        created = True
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ProbeInvalid("deadline", elapsed=10 - remaining)
        phase = "probe_start"
        raw = bounded_output(["docker", "start", "--attach", name], remaining)
        phase = "probe_expectation"
        try:
            observed = strict_json(raw)
        except (Invalid, ValueError, TypeError, KeyError):
            raise ProbeInvalid("invalid_capability")
        if observed != expected:
            raise ProbeInvalid("invalid_capability")
    except (Invalid, ValueError, OSError, subprocess.SubprocessError) as error:
        primary_error = error
        for attribute in ("primary_evidence", "reap_evidence"):
            detail = _closed_detail(getattr(error, attribute, None))
            if detail is not None:
                detail["phase"] = phase
                setattr(error, attribute, detail)
        error.probe_phase = phase
        error.probe_name = name
        error.probe_container_id = acknowledged_id
        print("diagnostic phase=" + phase, file=sys.stderr)
        if isinstance(error, ProbeInvalid):
            print("diagnostic reason=" + error.reason, file=sys.stderr)
        raise
    finally:
        # Exact random isolated name only. Keep every bounded uncertainty independently.
        cleanup_details = []
        cleanup_cause = None
        try:
            phase = "probe_cleanup_rm"
            started = time.monotonic()
            cleanup = subprocess.run(["docker", "rm", "--force", name], stdin=subprocess.DEVNULL,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
            if cleanup.returncode != 0:
                cleanup_details.append({"reason": "cleanup_nonzero", "phase": phase,
                                        "exit_code": cleanup.returncode,
                                        "elapsed": time.monotonic() - started})
            phase = "probe_cleanup_ps"
            started = time.monotonic()
            remaining = bounded_output(["docker", "ps", "--all", "--quiet", "--filter", "name=^/" + name + "$"], 5)
            if remaining.strip():
                cleanup_details.append({"reason": "cleanup_residue", "phase": phase,
                                        "elapsed": time.monotonic() - started})
        except (Invalid, OSError, subprocess.SubprocessError) as error:
            cleanup_cause = error
            print("diagnostic phase=" + phase, file=sys.stderr)
            detail = _closed_detail(getattr(error, "primary_evidence", None))
            if detail is None:
                detail = _closed_detail({"reason": "cleanup_timeout"
                    if isinstance(error, subprocess.TimeoutExpired) else "cleanup_query_error",
                    "elapsed": time.monotonic() - started})
            if detail["reason"] == "deadline":
                detail["reason"] = "cleanup_query_deadline"
            detail["phase"] = phase
            cleanup_details.append(detail)
            reap = _closed_detail(getattr(error, "reap_evidence", None))
            if reap is not None:
                reap["phase"] = phase
                cleanup_details.append(reap)
        if not created:
            cleanup_details.append({"reason": "create_unconfirmed", "phase": "probe_create"})
        if cleanup_details:
            # Still OSError: missing create ACK and cleanup uncertainty cannot pass a negative probe.
            if cleanup_cause is None:
                phase = "probe_cleanup_unproven"
                print("diagnostic phase=" + phase, file=sys.stderr)
            failure = OSError("diagnostic probe cleanup unproven")
            failure.primary_evidence = _closed_detail(getattr(primary_error, "primary_evidence", None))
            failure.reap_evidence = _closed_detail(getattr(primary_error, "reap_evidence", None))
            failure.cleanup_evidence = cleanup_details
            failure.probe_phase, failure.probe_name = phase, name
            failure.probe_container_id = acknowledged_id
            raise failure from cleanup_cause


def preflight(raw, manifest, registered, project):
    config = strict_json(raw)
    services = config["services"]
    require(isinstance(services, dict))
    expected = {key: value for key, value in registered["binding"].items() if key != "budget_id"}
    tokens = []
    images = []
    for service in SERVICES:
        actual = services[service]
        environment = actual["environment"]
        require(environment.get("LLM_DIAGNOSTIC_BUDGET_ID") == registered["binding"]["budget_id"])
        owner = service == "user-service"
        require(environment.get("USER_SERVICE_URL") == ("http://127.0.0.1:8001" if owner else "http://user-service:8001"))
        if owner:
            require(strict_json(environment.get("LLM_DIAGNOSTIC_BUDGET_LIMITS", "")) == registered["limits"])
        else:
            require(not environment.get("LLM_DIAGNOSTIC_BUDGET_LIMITS"))
        token = environment.get("INTERNAL_SERVICE_TOKEN")
        require(isinstance(token, str) and len(token) >= 32 and len(set(token)) >= 8
                and all(33 <= ord(char) <= 126 for char in token)
                and not any(word in token.lower() for word in ("placeholder", "change_me", "runtime-smoke")))
        tokens.append(token)
        image = manifest[service.upper().replace("-", "_") + "_IMAGE"]
        require(actual["image"] == image)
        images.append(image)
    require(len(set(tokens)) == 1)
    # Complete configuration validation precedes all probes; no partial configuration can start work.
    for image in images:
        probe(image, project, expected)


def main():
    profile_bytes, action, state, project, *paths = sys.argv[1:]
    require(re.fullmatch(r"nwq-[a-f0-9]{10}", project))
    registered = registration(profile_bytes.encode(), os.environ)
    if action == "probe":
        require(len(paths) == 1)
        manifest = {}
        for line in Path(paths[0]).read_text().splitlines():
            key, value = line.split("=", 1)
            require(key not in manifest)
            manifest[key] = value
        raw = sys.stdin.buffer.read(1048577)
        require(len(raw) <= 1048576)
        preflight(raw, manifest, registered, project)
    else:
        marker_action(action, Path(state), registered)


if __name__ == "__main__":
    try:
        main()
    except (Invalid, ValueError, TypeError, KeyError, OSError, subprocess.SubprocessError):
        sys.exit("release: diagnostic budget preflight failed")
