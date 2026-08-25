#!/usr/bin/env python3
"""Run the versioned NovelWorld single-node capacity contract."""

from __future__ import annotations

import argparse
import copy
import json
import math
import os
import platform
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


PASSWORD = "CapacityOnly123!"
ANCHOR = "林岚握紧手中的旧地图，望向被风暴笼罩的北塔，决定在天黑前寻找失踪的守门人。"
ENDING = "北塔的石门布满潮湿苔痕，林岚在门边发现守门人留下的铜铃。"
FILLER = "边城的钟声穿过风暴，地图学徒逐一核对石桥、塔灯与河岸足迹，并记录每个不会替同伴作决定的行动。"
SECRET_ENV_NAMES = (
    "POSTGRES_PASSWORD",
    "REDIS_PASSWORD",
    "JWT_SECRET",
    "RUNTIME_CONFIG_KEY",
    "INTERNAL_SERVICE_TOKEN",
    "LLM_API_KEY",
    "DATABASE_URL",
    "REDIS_URL",
)


class ProfileError(RuntimeError):
    pass


@dataclass(frozen=True)
class HttpResult:
    status: int
    headers: dict[str, str]
    body: bytes
    elapsed: float

    def json(self):
        return json.loads(self.body)


def require_keys(value, expected, where):
    if not isinstance(value, dict) or set(value) != set(expected):
        raise ValueError(f"{where} must contain exactly {sorted(expected)}")


def positive_number(value, where):
    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or not math.isfinite(value)
        or value <= 0
    ):
        raise ValueError(f"{where} must be positive")


def validate_policy(policy):
    require_keys(
        policy,
        {"version", "topology", "workload", "objectives"},
        "policy",
    )
    if policy["version"] != "single-node-v1":
        raise ValueError("unsupported capacity policy version")
    topology_keys = {
        "gateway_instances",
        "user_service_instances",
        "novel_service_instances",
        "agent_service_instances",
        "narrative_service_instances",
        "postgres_instances",
        "redis_instances",
    }
    require_keys(policy["topology"], topology_keys, "topology")
    if any(
        not isinstance(value, int) or isinstance(value, bool) or value != 1
        for value in policy["topology"].values()
    ):
        raise ValueError(
            "single-node-v1 requires exactly one instance of every component"
        )

    workload_keys = {
        "users",
        "source_bytes_min",
        "import_concurrency",
        "stream_concurrency",
        "world_turn_concurrency",
        "provider_delay_ms",
        "world_turn_history",
        "read_requests",
        "read_concurrency",
        "chat_turns",
    }
    require_keys(policy["workload"], workload_keys, "workload")
    for key, value in policy["workload"].items():
        positive_number(value, f"workload.{key}")
        if not isinstance(value, int) or isinstance(value, bool):
            raise ValueError(f"workload.{key} must be an integer")

    objectives_keys = {
        "import_admission_seconds_max",
        "import_ready_seconds_max",
        "overload_rejection_seconds_max",
        "stream_first_event_p95_seconds_max",
        "world_turn_p95_seconds_max",
        "world_read_p95_seconds_max",
        "redis_messages",
        "redis_bytes_max",
    }
    require_keys(policy["objectives"], objectives_keys, "objectives")
    for key, value in policy["objectives"].items():
        positive_number(value, f"objectives.{key}")
    for key in ("redis_messages", "redis_bytes_max"):
        if not isinstance(policy["objectives"][key], int):
            raise ValueError(f"objectives.{key} must be an integer")

    workload = policy["workload"]
    objectives = policy["objectives"]
    if workload["users"] < max(
        workload["import_concurrency"] + 1,
        workload["stream_concurrency"] + 1,
        workload["world_turn_concurrency"] + 1,
    ):
        raise ValueError(
            "users must cover every saturation workload plus one overload caller"
        )
    if workload["read_requests"] % workload["read_concurrency"]:
        raise ValueError(
            "read_requests must be an exact number of closed concurrency batches"
        )
    if not 1 <= workload["world_turn_history"] <= 100:
        raise ValueError("world_turn_history must fit the bounded journal contract")
    if workload["chat_turns"] * 2 <= objectives["redis_messages"]:
        raise ValueError("chat workload must exceed the Redis projection bound")
    if workload["source_bytes_min"] < 16 * 1024:
        raise ValueError("the import fixture must be at least 16 KiB")
    return policy


def nearest_rank(samples, percentile=0.95):
    if not samples:
        raise ValueError("percentile requires at least one sample")
    if not 0 < percentile <= 1:
        raise ValueError("percentile must be in (0, 1]")
    ordered = sorted(samples)
    return ordered[math.ceil(percentile * len(ordered)) - 1]


def http_request(
    base,
    method,
    path,
    *,
    token=None,
    payload=None,
    body=None,
    headers=None,
    timeout=120,
    released_at=None,
):
    request_headers = dict(headers or {})
    if token:
        request_headers["Authorization"] = f"Bearer {token}"
    if payload is not None:
        body = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode()
        request_headers["Content-Type"] = "application/json"
    started = released_at if released_at is not None else time.perf_counter()
    request = urllib.request.Request(
        f"{base}{path}", data=body, headers=request_headers, method=method
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            response_body = response.read()
            return HttpResult(
                response.status,
                {key.lower(): value for key, value in response.headers.items()},
                response_body,
                time.perf_counter() - started,
            )
    except urllib.error.HTTPError as error:
        return HttpResult(
            error.code,
            {key.lower(): value for key, value in error.headers.items()},
            error.read(),
            time.perf_counter() - started,
        )
    except urllib.error.URLError as error:
        raise ProfileError(f"request to {path} failed: {error.reason}") from error


def expect(result, status, label):
    if result.status != status:
        detail = result.body.decode(errors="replace")[:500]
        raise ProfileError(
            f"{label}: expected HTTP {status}, got {result.status}: {detail}"
        )
    return result


def barrier_batch(items, callback):
    items = list(items)
    release = {}
    barrier = threading.Barrier(
        len(items), action=lambda: release.update(at=time.perf_counter())
    )

    def run(item):
        barrier.wait()
        return callback(item, release["at"])

    with ThreadPoolExecutor(max_workers=len(items)) as executor:
        return list(executor.map(run, items))


def multipart_upload(source, title):
    boundary = f"----NovelWorldCapacity{uuid.uuid4().hex}"
    chunks = []
    for name, value in (
        ("title", title),
        ("author", "Capacity Gate"),
        ("deviation_mode", "creative"),
    ):
        chunks.append(
            f"--{boundary}\r\n"
            f'Content-Disposition: form-data; name="{name}"\r\n\r\n'
            f"{value}\r\n".encode()
        )
    chunks.append(
        f"--{boundary}\r\n"
        'Content-Disposition: form-data; name="file"; filename="capacity.txt"\r\n'
        "Content-Type: text/plain\r\n\r\n".encode()
    )
    chunks.extend((source, b"\r\n", f"--{boundary}--\r\n".encode()))
    return b"".join(chunks), f"multipart/form-data; boundary={boundary}"


def build_source(minimum_bytes):
    per_chapter = max(8_500, minimum_bytes // 2 + 512)

    def fill(prefix, suffix):
        content = prefix
        while len((content + suffix).encode()) < per_chapter:
            content += FILLER
        return content + suffix

    first = fill(ANCHOR, "")
    second = fill("", ENDING)
    if max(len(first.encode()), len(second.encode())) >= 16_000:
        raise ValueError("capacity source chapter exceeds the canonical chunk boundary")
    source = f"第一章 风暴前夜\n{first}\n第二章 北塔回声\n{second}\n".encode()
    if len(source) < minimum_bytes:
        raise ValueError("capacity source is smaller than policy")
    return source


def provider_reset(provider_url, delays=None, failures=None):
    deadline = time.monotonic() + 10
    payload = {
        "delays_ms": delays or {},
        "failures_remaining": failures
        or {"canon": 0, "narrative_transition": 0, "world_turn": 0},
    }
    while True:
        result = http_request(
            provider_url, "POST", "/__control__/reset", payload=payload, timeout=5
        )
        if result.status == 200:
            return result.json()
        if result.status != 409 or time.monotonic() >= deadline:
            expect(result, 200, "reset provider controls")
        time.sleep(0.05)


def provider_stats(provider_url):
    return expect(
        http_request(provider_url, "GET", "/__control__/stats", timeout=5),
        200,
        "provider stats",
    ).json()


def wait_provider_calls(provider_url, expected_calls, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        stats = provider_stats(provider_url)
        if stats["calls"] == expected_calls and not any(stats["active"].values()):
            return stats
        if any(
            stats["calls"].get(operation, 0) > count
            for operation, count in expected_calls.items()
        ) or any(operation not in expected_calls for operation in stats["calls"]):
            return stats
        if time.monotonic() >= deadline:
            return stats
        time.sleep(0.05)


def stream_chat(api_url, fixture, turn_id, released_at=None, timeout=30):
    payload = json.dumps(
        {"message": "容量门禁对话。", "novel_id": fixture["novel_id"]},
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode()
    request = urllib.request.Request(
        f"{api_url}/chat/{fixture['character_id']}/stream",
        data=payload,
        headers={
            "Authorization": f"Bearer {fixture['token']}",
            "Content-Type": "application/json",
            "Idempotency-Key": turn_id,
        },
        method="POST",
    )
    started = released_at if released_at is not None else time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            first_event = None
            body = bytearray()
            for line in response:
                body.extend(line)
                if first_event is None and line.strip():
                    first_event = time.perf_counter() - started
            return {
                "status": response.status,
                "elapsed": time.perf_counter() - started,
                "first_event": first_event,
                "retry_after": response.headers.get("Retry-After"),
                "body": bytes(body),
            }
    except urllib.error.HTTPError as error:
        return {
            "status": error.code,
            "elapsed": time.perf_counter() - started,
            "first_event": None,
            "retry_after": error.headers.get("Retry-After"),
            "body": error.read(),
        }


def pg_scalar(sql):
    command = [
        "docker",
        "exec",
        "novel-postgres",
        "psql",
        "-U",
        os.environ.get("POSTGRES_USER", "novel"),
        "-d",
        os.environ.get("POSTGRES_DB", "novel_world"),
        "-At",
        "-c",
        sql,
    ]
    result = subprocess.run(command, check=True, capture_output=True, text=True)
    return result.stdout.strip()


def redis_scalar(*arguments):
    environment = os.environ.copy()
    environment["REDISCLI_AUTH"] = os.environ["REDIS_PASSWORD"]
    command = [
        "docker",
        "exec",
        "-e",
        "REDISCLI_AUTH",
        "novel-redis",
        "redis-cli",
        "--raw",
        *map(str, arguments),
    ]
    result = subprocess.run(
        command, check=True, capture_output=True, text=True, env=environment
    )
    return result.stdout.strip()


def sql_uuid_list(values):
    return ",".join(f"'{uuid.UUID(value)}'::uuid" for value in values)


def cgroup_limit(path):
    try:
        value = Path(path).read_text().strip()
    except OSError:
        return None
    if value == "max":
        return None
    try:
        return int(value)
    except ValueError:
        return None


def environment_snapshot():
    try:
        host_memory = os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
    except (OSError, ValueError):
        host_memory = None
    cpu_quota = None
    try:
        quota, period = Path("/sys/fs/cgroup/cpu.max").read_text().split()
        if quota != "max":
            cpu_quota = int(quota) / int(period)
    except (OSError, ValueError):
        pass
    return {
        "cpu_count": os.cpu_count(),
        "cpu_quota": cpu_quota,
        "host_memory_bytes": host_memory,
        "memory_limit_bytes": cgroup_limit("/sys/fs/cgroup/memory.max"),
        "platform": platform.platform(),
        "python": platform.python_version(),
    }


def redact_secrets(value, secrets):
    for secret in filter(None, secrets):
        value = value.replace(secret, "[REDACTED]")
    return value


class CapacityProfile:
    def __init__(self, policy, api_url, provider_url, git_sha):
        self.policy = policy
        self.api_url = api_url.rstrip("/")
        self.provider_url = provider_url.rstrip("/")
        self.secrets = [PASSWORD]
        self.secrets.extend(os.environ.get(name, "") for name in SECRET_ENV_NAMES)
        self.fixtures = []
        self.report = {
            "schema": "novelworld-capacity-report-v1",
            "policy_version": policy["version"],
            "cache_mode": os.environ.get("CACHE_MODE", "unknown"),
            "commit": git_sha,
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "environment": environment_snapshot(),
            "phases": {},
            "checks": [],
            "passed": False,
        }

    def check(self, name, passed, **evidence):
        entry = {"name": name, "passed": bool(passed), **evidence}
        self.report["checks"].append(entry)
        if not passed:
            raise ProfileError(f"capacity predicate failed: {name}")

    def wait_ready(self):
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            result = http_request(
                self.api_url.removesuffix("/api"), "GET", "/ready", timeout=5
            )
            if result.status == 200:
                return
            time.sleep(1)
        raise ProfileError("Gateway did not become ready")

    def setup_users(self):
        self.wait_ready()
        status = expect(
            http_request(self.api_url, "GET", "/setup/status", timeout=5),
            200,
            "setup status",
        ).json()
        if status.get("configured"):
            raise ProfileError(
                "capacity profile requires empty PostgreSQL and Redis volumes"
            )

        users = []
        for index in range(self.policy["workload"]["users"]):
            path = "/setup/init" if index == 0 else "/auth/register"
            response = expect(
                http_request(
                    self.api_url,
                    "POST",
                    path,
                    payload={
                        "email": f"capacity-{index}@test.invalid",
                        "password": PASSWORD,
                        "name": f"Capacity {index}",
                    },
                    timeout=30,
                ),
                201,
                f"create capacity user {index}",
            ).json()
            self.secrets.extend((response["access_token"], response["refresh_token"]))
            users.append(
                {
                    "index": index,
                    "user_id": response["user"]["id"],
                    "token": response["access_token"],
                }
            )
        self.fixtures = users

    def upload(self, fixture, source, released_at=None):
        body, content_type = multipart_upload(
            source, f"Capacity Novel {fixture['index']}"
        )
        return http_request(
            self.api_url,
            "POST",
            "/novels/upload",
            token=fixture["token"],
            body=body,
            headers={"Content-Type": content_type},
            timeout=30,
            released_at=released_at,
        )

    def admit_fixture_upload(self, fixture, source):
        deadline = time.monotonic() + 10
        while True:
            result = self.upload(fixture, source)
            if result.status == 202:
                return result
            if (
                result.status != 503
                or result.headers.get("retry-after") != "1"
                or time.monotonic() >= deadline
            ):
                expect(result, 202, f"import fixture {fixture['index']}")
            time.sleep(1)

    def wait_novel_ready(self, fixture, released_at=None):
        deadline = (
            time.monotonic() + self.policy["objectives"]["import_ready_seconds_max"]
        )
        while time.monotonic() < deadline:
            result = expect(
                http_request(
                    self.api_url,
                    "GET",
                    f"/novels/{fixture['novel_id']}/status",
                    token=fixture["token"],
                    timeout=5,
                ),
                200,
                "poll novel status",
            )
            state = result.json()["status"]
            if state == "ready":
                return (
                    time.perf_counter() - released_at if released_at else result.elapsed
                )
            if state == "error":
                raise ProfileError(f"novel {fixture['index']} entered error state")
            time.sleep(0.1)
        raise ProfileError(f"novel {fixture['index']} did not become ready")

    def import_phase(self):
        workload = self.policy["workload"]
        objectives = self.policy["objectives"]
        source = build_source(workload["source_bytes_min"])
        provider_reset(self.provider_url, {"default": 500})
        participants = self.fixtures[: workload["import_concurrency"] + 1]
        results = barrier_batch(
            participants,
            lambda fixture, released_at: (
                fixture,
                released_at,
                self.upload(fixture, source, released_at),
            ),
        )
        accepted = [entry for entry in results if entry[2].status == 202]
        rejected = [entry for entry in results if entry[2].status == 503]
        self.check(
            "import_admission_shape",
            len(accepted) == workload["import_concurrency"] and len(rejected) == 1,
            statuses=[entry[2].status for entry in results],
        )
        self.check(
            "import_admission_latency",
            all(
                entry[2].elapsed <= objectives["import_admission_seconds_max"]
                for entry in accepted
            ),
            samples_seconds=[entry[2].elapsed for entry in accepted],
            maximum_seconds=objectives["import_admission_seconds_max"],
        )
        self.check(
            "import_overload_rejection",
            rejected[0][2].elapsed <= objectives["overload_rejection_seconds_max"]
            and rejected[0][2].headers.get("retry-after") == "1",
            sample_seconds=rejected[0][2].elapsed,
            maximum_seconds=objectives["overload_rejection_seconds_max"],
        )

        for fixture, _, result in accepted:
            fixture["novel_id"] = result.json()["novel_id"]
        rejected_fixture = rejected[0][0]
        owned = expect(
            http_request(
                self.api_url,
                "GET",
                "/novels",
                token=rejected_fixture["token"],
                timeout=5,
            ),
            200,
            "rejected import ownership check",
        ).json()
        self.check(
            "rejected_import_not_persisted", owned == [], owned_novels=len(owned)
        )

        ready_samples = [
            self.wait_novel_ready(fixture, released_at)
            for fixture, released_at, _ in accepted
        ]
        self.check(
            "import_ready_latency",
            all(
                value <= objectives["import_ready_seconds_max"]
                for value in ready_samples
            ),
            samples_seconds=ready_samples,
            maximum_seconds=objectives["import_ready_seconds_max"],
        )
        expected_calls = {
            "canon": workload["import_concurrency"] * 2,
            "character_chunk": workload["import_concurrency"],
            "characters": workload["import_concurrency"],
            "image": workload["import_concurrency"],
            "nodes": workload["import_concurrency"],
        }
        initial_stats = wait_provider_calls(self.provider_url, expected_calls)
        self.check(
            "rejected_import_reaches_no_provider",
            initial_stats["calls"] == expected_calls,
            calls=initial_stats["calls"],
            expected_calls=expected_calls,
        )

        provider_reset(self.provider_url)
        for fixture in self.fixtures:
            if "novel_id" in fixture:
                continue
            result = self.admit_fixture_upload(fixture, source)
            fixture["novel_id"] = result.json()["novel_id"]
            self.wait_novel_ready(fixture)

        self.report["phases"]["import"] = {
            "source_bytes": len(source),
            "admission_seconds": [entry[2].elapsed for entry in results],
            "ready_seconds": ready_samples,
            "provider": initial_stats,
        }

    def prepare_worlds(self):
        for fixture in self.fixtures:
            token = fixture["token"]
            novel_id = fixture["novel_id"]
            expect(
                http_request(
                    self.api_url,
                    "PUT",
                    f"/progress/{novel_id}",
                    token=token,
                    payload={"current_chapter": 2},
                ),
                204,
                "unlock capacity novel",
            )
            expect(
                http_request(
                    self.api_url,
                    "PUT",
                    f"/progress/{novel_id}/identity",
                    token=token,
                    payload={
                        "identity_type": "self",
                        "identity_name": f"云舟{fixture['index']}",
                        "character_id": None,
                    },
                ),
                204,
                "set reader identity",
            )
            entry = expect(
                http_request(
                    self.api_url,
                    "GET",
                    f"/narrative/{novel_id}/player-entry?checkpoint_chapter=1",
                    token=token,
                ),
                200,
                "load player entry",
            ).json()
            location_id = entry["locations"][0]["id"]
            expect(
                http_request(
                    self.api_url,
                    "PUT",
                    f"/narrative/{novel_id}/player-entry",
                    token=token,
                    payload={
                        "checkpoint_chapter": 1,
                        "name": f"云舟{fixture['index']}",
                        "background": "来自边城的地图学徒。",
                        "capabilities": ["辨认古地图"],
                        "location_id": location_id,
                        "inventory": ["旧地图"],
                    },
                ),
                200,
                "create PlayerEntity",
            )
            world = expect(
                http_request(
                    self.api_url,
                    "POST",
                    f"/narrative/{novel_id}/world",
                    token=token,
                ),
                200,
                "start open world",
            ).json()
            if world["session"]["turn_number"] != 0:
                raise ProfileError("fresh world did not start at turn zero")
            fixture["canon_event_id"] = world["session"]["canonical_events"][0]["id"]
            characters = expect(
                http_request(
                    self.api_url,
                    "GET",
                    f"/novels/{novel_id}/characters",
                    token=token,
                ),
                200,
                "load capacity character",
            ).json()
            fixture["character_id"] = characters[0]["id"]

    def chat_saturation_phase(self):
        workload = self.policy["workload"]
        objectives = self.policy["objectives"]
        provider_reset(
            self.provider_url,
            {"stream": workload["provider_delay_ms"]},
        )
        participants = [
            (fixture, str(uuid.uuid4()))
            for fixture in self.fixtures[: workload["stream_concurrency"] + 1]
        ]
        results = barrier_batch(
            participants,
            lambda participant, released_at: (
                participant[0],
                participant[1],
                stream_chat(
                    self.api_url,
                    participant[0],
                    participant[1],
                    released_at=released_at,
                ),
            ),
        )
        succeeded = [entry for entry in results if entry[2]["status"] == 200]
        rejected = [entry for entry in results if entry[2]["status"] == 503]
        self.check(
            "stream_admission_shape",
            len(succeeded) == workload["stream_concurrency"] and len(rejected) == 1,
            statuses=[entry[2]["status"] for entry in results],
        )
        self.check(
            "stream_commits",
            all(
                b"event: done" in entry[2]["body"]
                and b'"committed":true' in entry[2]["body"]
                for entry in succeeded
            ),
            committed=len(succeeded),
        )
        turn_ids = sql_uuid_list(entry[1] for entry in succeeded)
        persisted = pg_scalar(
            "SELECT "
            f"(SELECT COUNT(*) FROM chat_turns WHERE id IN ({turn_ids}) AND status = 'completed') || ':' || "
            f"(SELECT COUNT(*) FROM chat_messages WHERE turn_id IN ({turn_ids}))"
        )
        self.check(
            "stream_persistence",
            persisted
            == f"{workload['stream_concurrency']}:{workload['stream_concurrency'] * 2}",
            persisted_counts=persisted,
        )
        first_events = [entry[2]["first_event"] for entry in succeeded]
        first_event_p95 = nearest_rank(first_events)
        self.check(
            "stream_first_event_p95",
            first_event_p95 <= objectives["stream_first_event_p95_seconds_max"],
            samples_seconds=first_events,
            p95_seconds=first_event_p95,
            maximum_seconds=objectives["stream_first_event_p95_seconds_max"],
        )
        self.check(
            "stream_overload_rejection",
            rejected[0][2]["elapsed"] <= objectives["overload_rejection_seconds_max"]
            and rejected[0][2]["retry_after"] == "1",
            sample_seconds=rejected[0][2]["elapsed"],
            maximum_seconds=objectives["overload_rejection_seconds_max"],
        )
        stats = provider_stats(self.provider_url)
        self.check(
            "stream_provider_saturation",
            stats["calls"].get("stream") == workload["stream_concurrency"]
            and stats["peak"].get("stream") == workload["stream_concurrency"]
            and stats["active"].get("stream") == 0,
            calls=stats["calls"].get("stream", 0),
            peak=stats["peak"].get("stream", 0),
        )
        self.report["phases"]["stream"] = {
            "first_event_seconds": first_events,
            "completion_seconds": [entry[2]["elapsed"] for entry in succeeded],
            "overload_seconds": rejected[0][2]["elapsed"],
            "provider": stats,
        }
        return succeeded[0][0]

    @staticmethod
    def world_action(intent="查清北塔换防并阻止伏击", target_id=None):
        return {"kind": "investigate", "target_id": target_id, "intent": intent}

    def submit_world_turn(self, fixture, turn_id, action, released_at=None):
        return http_request(
            self.api_url,
            "POST",
            f"/narrative/{fixture['novel_id']}/world/turns",
            token=fixture["token"],
            payload=action,
            headers={"Idempotency-Key": turn_id},
            timeout=30,
            released_at=released_at,
        )

    def world_saturation_phase(self):
        workload = self.policy["workload"]
        objectives = self.policy["objectives"]
        provider_reset(
            self.provider_url,
            {"world_turn": workload["provider_delay_ms"]},
        )
        participants = [
            (fixture, str(uuid.uuid4()))
            for fixture in self.fixtures[: workload["world_turn_concurrency"]]
        ]
        results = barrier_batch(
            participants,
            lambda participant, released_at: (
                participant[0],
                participant[1],
                self.submit_world_turn(
                    participant[0],
                    participant[1],
                    self.world_action(target_id=participant[0]["canon_event_id"]),
                    released_at,
                ),
            ),
        )
        self.check(
            "world_turn_success",
            all(entry[2].status == 200 for entry in results),
            statuses=[entry[2].status for entry in results],
        )
        self.check(
            "world_turn_advances_once",
            all(
                entry[2].json()["world_state"]["state"]["open_world"]["turn_number"]
                == 1
                for entry in results
            ),
            turn_numbers=[
                entry[2].json()["world_state"]["state"]["open_world"]["turn_number"]
                for entry in results
            ],
        )
        turn_ids = sql_uuid_list(entry[1] for entry in results)
        novel_ids = sql_uuid_list(entry[0]["novel_id"] for entry in results)
        persisted = pg_scalar(
            "SELECT "
            f"(SELECT COUNT(*) FROM world_turns WHERE id IN ({turn_ids}) AND status = 'completed') || ':' || "
            f"(SELECT COUNT(*) FROM world_states WHERE novel_id IN ({novel_ids}) AND (state #>> '{{open_world,turn_number}}')::BIGINT = 1)"
        )
        self.check(
            "world_turn_persistence",
            persisted
            == f"{workload['world_turn_concurrency']}:{workload['world_turn_concurrency']}",
            persisted_counts=persisted,
        )
        samples = [entry[2].elapsed for entry in results]
        latency_p95 = nearest_rank(samples)
        self.check(
            "world_turn_p95",
            latency_p95 <= objectives["world_turn_p95_seconds_max"],
            samples_seconds=samples,
            p95_seconds=latency_p95,
            maximum_seconds=objectives["world_turn_p95_seconds_max"],
        )
        stats = provider_stats(self.provider_url)
        self.check(
            "world_provider_saturation",
            stats["calls"].get("world_turn") == workload["world_turn_concurrency"]
            and stats["peak"].get("world_turn") == workload["world_turn_concurrency"]
            and stats["active"].get("world_turn") == 0,
            calls=stats["calls"].get("world_turn", 0),
            peak=stats["peak"].get("world_turn", 0),
        )
        self.report["phases"]["world_turn"] = {
            "completion_seconds": samples,
            "provider": stats,
        }

    def failure_replay_phase(self):
        fixture = self.fixtures[self.policy["workload"]["world_turn_concurrency"]]
        provider_reset(
            self.provider_url,
            failures={"canon": 0, "narrative_transition": 0, "world_turn": 1},
        )
        turn_id = str(uuid.uuid4())
        action = self.world_action(target_id=fixture["canon_event_id"])
        failed = self.submit_world_turn(fixture, turn_id, action)
        self.check("world_failure_injected", failed.status == 502, status=failed.status)
        state = expect(
            http_request(
                self.api_url,
                "GET",
                f"/narrative/{fixture['novel_id']}/world",
                token=fixture["token"],
            ),
            200,
            "world after injected failure",
        ).json()
        self.check(
            "world_failure_does_not_advance",
            state["session"]["turn_number"] == 0 and len(state["journal"]) == 0,
            turn_number=state["session"]["turn_number"],
            journal_entries=len(state["journal"]),
        )
        retried = expect(
            self.submit_world_turn(fixture, turn_id, action),
            200,
            "retry failed world turn",
        )
        calls_after_retry = provider_stats(self.provider_url)["calls"].get(
            "world_turn", 0
        )
        replayed = expect(
            self.submit_world_turn(fixture, turn_id, action),
            200,
            "replay completed world turn",
        )
        stats = provider_stats(self.provider_url)
        committed = expect(
            http_request(
                self.api_url,
                "GET",
                f"/narrative/{fixture['novel_id']}/world",
                token=fixture["token"],
            ),
            200,
            "world after successful retry",
        ).json()
        committed_turn = committed["session"]["turn_number"]
        committed_journal = len(committed["journal"])
        row = pg_scalar(
            "SELECT status || ':' || attempt FROM world_turns "
            f"WHERE id = '{uuid.UUID(turn_id)}'"
        )
        self.check(
            "world_retry_and_exact_replay",
            retried.body == replayed.body
            and calls_after_retry == 2
            and stats["calls"].get("world_turn") == 2
            and row == "completed:2"
            and committed_turn == 1
            and committed_journal == 1,
            provider_calls=stats["calls"].get("world_turn", 0),
            persisted_state=row,
            committed_turn=committed_turn,
            committed_journal=committed_journal,
            byte_identical=retried.body == replayed.body,
        )
        self.report["phases"]["failure_replay"] = {
            "failed_status": failed.status,
            "provider": stats,
            "persisted_state": row,
            "committed_turn": committed_turn,
            "committed_journal": committed_journal,
        }

    def history_and_read_phase(self):
        fixture = self.fixtures[0]
        provider_reset(self.provider_url)
        target = self.policy["workload"]["world_turn_history"]
        for turn_number in range(2, target + 1):
            result = expect(
                self.submit_world_turn(
                    fixture,
                    str(uuid.uuid4()),
                    {
                        "kind": "pursue_goal",
                        "target_id": None,
                        "intent": f"整理地下回廊线索 {turn_number}",
                    },
                ),
                200,
                f"seed world turn {turn_number}",
            )
            actual = result.json()["world_state"]["state"]["open_world"]["turn_number"]
            if actual != turn_number:
                raise ProfileError(
                    f"world history advanced to {actual}, expected {turn_number}"
                )

        workload = self.policy["workload"]
        objectives = self.policy["objectives"]
        all_results = []
        for _ in range(workload["read_requests"] // workload["read_concurrency"]):
            batch = barrier_batch(
                range(workload["read_concurrency"]),
                lambda _index, released_at: http_request(
                    self.api_url,
                    "GET",
                    f"/narrative/{fixture['novel_id']}/world",
                    token=fixture["token"],
                    timeout=10,
                    released_at=released_at,
                ),
            )
            all_results.extend(batch)
        self.check(
            "world_read_success",
            all(result.status == 200 for result in all_results),
            successes=sum(result.status == 200 for result in all_results),
            requests=len(all_results),
        )
        views = [result.json() for result in all_results]
        self.check(
            "world_read_completeness",
            all(
                view["session"]["turn_number"] == target
                and len(view["journal"]) == target
                for view in views
            ),
            expected_turns=target,
        )
        samples = [result.elapsed for result in all_results]
        latency_p95 = nearest_rank(samples)
        self.check(
            "world_read_p95",
            latency_p95 <= objectives["world_read_p95_seconds_max"],
            samples_seconds=samples,
            p95_seconds=latency_p95,
            maximum_seconds=objectives["world_read_p95_seconds_max"],
        )
        self.report["phases"]["world_read"] = {
            "latency_seconds": samples,
            "p95_seconds": latency_p95,
            "turns": target,
        }

    def complete_chat_turn(self, fixture, turn_id):
        deadline = time.monotonic() + 10
        while True:
            result = stream_chat(self.api_url, fixture, turn_id, timeout=30)
            if result["status"] == 200 and b"event: done" in result["body"]:
                return
            if result["status"] not in (409, 503) or time.monotonic() >= deadline:
                detail = result["body"].decode(errors="replace")[:500]
                raise ProfileError(
                    f"chat turn failed with {result['status']}: {detail}"
                )
            time.sleep(0.05)

    def redis_projection_phase(self, fixture):
        provider_reset(self.provider_url)
        target_turns = self.policy["workload"]["chat_turns"]
        objectives = self.policy["objectives"]
        user_id = uuid.UUID(fixture["user_id"])
        character_id = uuid.UUID(fixture["character_id"])
        novel_id = uuid.UUID(fixture["novel_id"])
        existing = int(
            pg_scalar(
                "SELECT COUNT(*) FROM chat_turns "
                f"WHERE user_id = '{user_id}' AND character_id = '{character_id}' "
                f"AND novel_id = '{novel_id}' AND status = 'completed'"
            )
        )
        for _ in range(existing, target_turns):
            self.complete_chat_turn(fixture, str(uuid.uuid4()))

        key = f"chat:{character_id}:{user_id}"
        deadline = time.monotonic() + 30
        observed = None
        while time.monotonic() < deadline:
            turns, messages = map(
                int,
                pg_scalar(
                    "SELECT "
                    f"(SELECT COUNT(*) FROM chat_turns WHERE user_id = '{user_id}' "
                    f"AND character_id = '{character_id}' AND novel_id = '{novel_id}' "
                    "AND status = 'completed') || ':' || "
                    f"(SELECT COUNT(*) FROM chat_messages WHERE user_id = '{user_id}' "
                    f"AND character_id = '{character_id}' AND novel_id = '{novel_id}')"
                ).split(":"),
            )
            redis_messages = int(redis_scalar("LLEN", key))
            redis_bytes_raw = redis_scalar("MEMORY", "USAGE", key)
            redis_bytes = int(redis_bytes_raw) if redis_bytes_raw else 0
            stats = provider_stats(self.provider_url)
            observed = (turns, messages, redis_messages, redis_bytes, stats)
            if (
                turns == target_turns
                and messages == target_turns * 2
                and redis_messages == objectives["redis_messages"]
                and not any(stats["active"].values())
            ):
                break
            time.sleep(0.1)
        turns, messages, redis_messages, redis_bytes, stats = observed
        self.check(
            "postgres_chat_complete",
            turns == target_turns and messages == target_turns * 2,
            turns=turns,
            messages=messages,
        )
        self.check(
            "redis_projection_bound",
            redis_messages == objectives["redis_messages"]
            and redis_bytes <= objectives["redis_bytes_max"],
            messages=redis_messages,
            bytes=redis_bytes,
            maximum_bytes=objectives["redis_bytes_max"],
        )
        self.report["phases"]["redis_projection"] = {
            "postgres_turns": turns,
            "postgres_messages": messages,
            "redis_messages": redis_messages,
            "redis_bytes": redis_bytes,
            "provider": stats,
        }

    def run(self):
        provider_reset(self.provider_url)
        self.setup_users()
        self.import_phase()
        self.prepare_worlds()
        chat_fixture = self.chat_saturation_phase()
        self.world_saturation_phase()
        self.failure_replay_phase()
        self.history_and_read_phase()
        self.redis_projection_phase(chat_fixture)
        self.report["passed"] = all(check["passed"] for check in self.report["checks"])
        return self.report

    def write_report(self, path):
        serialized = (
            json.dumps(self.report, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
        )
        for secret in filter(None, self.secrets):
            if secret in serialized:
                raise ProfileError("capacity report contains a credential")
        Path(path).write_text(serialized)


def self_test(policy):
    validate_policy(policy)
    assert nearest_rank([8, 1, 2, 3, 4, 5, 6, 7]) == 8
    assert nearest_rank(list(range(1, 21))) == 19
    invalid = copy.deepcopy(policy)
    invalid["workload"]["users"] = 1
    try:
        validate_policy(invalid)
    except ValueError:
        pass
    else:
        raise AssertionError("invalid workload was accepted")
    invalid = copy.deepcopy(policy)
    invalid["topology"]["gateway_instances"] = True
    try:
        validate_policy(invalid)
    except ValueError:
        pass
    else:
        raise AssertionError("boolean topology value was accepted")
    assert redact_secrets("before token after", ["token"]) == (
        "before [REDACTED] after"
    )
    print("single-node-v1 policy and percentile self-check passed")


def git_sha(value):
    if value:
        return value
    return subprocess.run(
        ["git", "rev-parse", "HEAD"], check=True, capture_output=True, text=True
    ).stdout.strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--policy", default="tools/capacity/policy-v1.json")
    parser.add_argument("--report", default="capacity-report.json")
    parser.add_argument("--api-url", default="http://127.0.0.1:18081/api")
    parser.add_argument("--provider-url", default="http://127.0.0.1:18080")
    parser.add_argument("--git-sha")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    policy = validate_policy(json.loads(Path(args.policy).read_text()))
    if args.self_test:
        self_test(policy)
        return 0

    if os.environ.get("CACHE_MODE") != "redis":
        raise ProfileError(
            "single-node-v1 requires an explicitly selected CACHE_MODE=redis deployment"
        )
    if not os.environ.get("REDIS_PASSWORD"):
        raise ProfileError("single-node-v1 requires the active Redis credential")

    profile = CapacityProfile(
        policy, args.api_url, args.provider_url, git_sha(args.git_sha)
    )
    exit_code = 0
    try:
        profile.run()
    except Exception as error:  # report the first decision-relevant failure
        profile.report["passed"] = False
        profile.report["error"] = redact_secrets(
            f"{type(error).__name__}: {error}", profile.secrets
        )
        exit_code = 1
    finally:
        profile.write_report(args.report)
    if exit_code:
        print(profile.report["error"], file=sys.stderr)
    else:
        print(
            json.dumps({"passed": True, "report": args.report}, separators=(",", ":"))
        )
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
