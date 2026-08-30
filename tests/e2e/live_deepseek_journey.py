#!/usr/bin/env python3
"""Isolated DeepSeek H4 diagnostic and qualification journey.

The public report contains only bounded aggregate evidence. Provider
configuration, model prose, identifiers, raw metrics, and account exports stay
in the operator-selected private output directory and must not be committed.
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
import re
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable


EXPECTED_PROVIDER = "deepseek"
EXPECTED_MODEL = "deepseek-v4-flash"
EXPECTED_API_URL = "https://api.deepseek.com"
EXPECTED_CANON_PROMPT = "canon-chunk-v7+event-grouping-v3"
EXPECTED_BRANCH_PROMPT = "narrative-transition-v1"
EXPECTED_WORLD_PROMPT = "world-turn-v2"
PROJECT_PATTERN = re.compile(r"^nwq-[a-f0-9]{10}$")
COMMIT_PATTERN = re.compile(r"^[0-9a-f]{40}$")
IMAGE_PATTERN = re.compile(r"^[a-z0-9][a-z0-9._/:@-]*@sha256:[0-9a-f]{64}$")
PUBLIC_MODEL_PATTERN = re.compile(r"^[A-Za-z0-9._/:-]{1,200}$")
RELEASE_IMAGE_KEYS = (
    "GATEWAY_IMAGE",
    "USER_SERVICE_IMAGE",
    "NOVEL_SERVICE_IMAGE",
    "AGENT_SERVICE_IMAGE",
    "NARRATIVE_SERVICE_IMAGE",
    "FRONTEND_IMAGE",
    "POSTGRES_IMAGE",
    "REDIS_IMAGE",
    "NGINX_IMAGE",
)
APPLICATION_IMAGE_KEYS = RELEASE_IMAGE_KEYS[:6]
INFRASTRUCTURE_IMAGE_KEYS = RELEASE_IMAGE_KEYS[6:]
APPLICATION_CONTAINERS = {
    "GATEWAY_IMAGE": "gateway",
    "USER_SERVICE_IMAGE": "user-service",
    "NOVEL_SERVICE_IMAGE": "novel-service",
    "AGENT_SERVICE_IMAGE": "agent-service",
    "NARRATIVE_SERVICE_IMAGE": "narrative-service",
    "FRONTEND_IMAGE": "frontend",
}
RELEASE_KEYS = {"RELEASE_VERSION", "RELEASE_GIT_SHA", *RELEASE_IMAGE_KEYS}
PRODUCT_INPUT = Path("tests/e2e/fixtures/h4-journey-v1.json")
PRIVATE_TABLES = (
    "user_novels",
    "reading_progress",
    "chat_messages",
    "chat_turns",
    "character_memories",
    "narrative_nodes",
    "user_choices",
    "world_states",
    "world_turns",
    "player_chapters",
    "refresh_tokens",
    "user_llm_configs",
)
SERVICE_PORTS = {
    "user-service": 8001,
    "novel-service": 8002,
    "agent-service": 8003,
    "narrative-service": 8004,
}
COHORT_VERSIONS = {
    "qualification": "private-preview-qualification-v1",
    "extraction": "extraction-quality-v1",
    "semantic": "h3-semantic-v1",
    "llm_budget": "h3-llm-budget-v2",
    "llm_metrics": "llm-observability-v1",
    "runner_report": "h4-runner-report-v2",
    "canon_prompt": EXPECTED_CANON_PROMPT,
    "branch_transition_prompt": EXPECTED_BRANCH_PROMPT,
    "world_transition_prompt": EXPECTED_WORLD_PROMPT,
    "world_transition_schema": 1,
}


class QualificationFailure(RuntimeError):
    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8")


def qualification_environment(
    root: Path, host_environment: dict[str, str]
) -> dict[str, str]:
    compose_text = (root / "docker-compose.yml").read_text(encoding="utf-8")
    example_text = (root / ".env.example").read_text(encoding="utf-8")
    product_keys = set(re.findall(r"\$\{([A-Z][A-Z0-9_]*)", compose_text))
    product_keys.update(
        re.findall(r"(?m)^\s*(?:#\s*)?([A-Z][A-Z0-9_]*)=", example_text)
    )
    return {
        key: value
        for key, value in host_environment.items()
        if key not in product_keys
    }


def expected_export_source(kind: str, data: dict[str, Any]) -> str | None:
    if kind == "canon_story_model":
        confidences: list[Any] = []

        def collect(value: Any) -> None:
            if isinstance(value, dict):
                if "confidence" in value:
                    confidences.append(value["confidence"])
                for nested in value.values():
                    collect(nested)
            elif isinstance(value, list):
                for nested in value:
                    collect(nested)

        collect(data.get("content"))
        return (
            "canon"
            if confidences
            and all(
                isinstance(value, (int, float))
                and not isinstance(value, bool)
                and value >= 1.0
                for value in confidences
            )
            else "uncertain"
        )
    if kind == "narrative_node":
        return "canon" if data.get("user_id") is None else "generated"
    if kind == "user_choice":
        return "reader"
    if kind == "world_state":
        state = data.get("state")
        return "mixed" if isinstance(state, dict) and "open_world" in state else "reader"
    if kind == "player_chapter":
        return "reader" if data.get("origin") == "choice" else "generated"
    if kind == "world_turn":
        return "mixed" if data.get("transition") is not None else "reader"
    return None


def valid_public_model(value: Any) -> bool:
    return (
        isinstance(value, str)
        and value != "invalid"
        and "://" not in value
        and PUBLIC_MODEL_PATTERN.fullmatch(value) is not None
    )


def write_private(path: Path, value: bytes) -> None:
    descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(value)
        stream.flush()
        os.fsync(stream.fileno())


def choice_replay_projection(value: Any) -> dict[str, Any]:
    world_state = value.get("world_state") if isinstance(value, dict) else None
    if not isinstance(world_state, dict) or not isinstance(world_state.get("updated_at"), str):
        raise QualificationFailure("branch_world_state_invalid")
    return {
        **value,
        "world_state": {
            key: item for key, item in world_state.items() if key != "updated_at"
        },
    }


def run(
    args: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    capture: bool = True,
    check: bool = True,
) -> str:
    result = subprocess.run(
        args,
        cwd=cwd,
        env=env,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )
    if check and result.returncode:
        command = Path(args[0]).name.replace(".exe", "")
        action = next((part for part in args[1:3] if not part.startswith("-")), "command")
        raise QualificationFailure(f"{command}_{action}_failed")
    return (result.stdout or "").strip()


def terminate_process_tree(process: subprocess.Popen[Any]) -> None:
    if process.poll() is not None:
        return
    if os.name == "nt":
        subprocess.run(
            ["taskkill", "/PID", str(process.pid), "/T", "/F"],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    else:
        with contextlib.suppress(ProcessLookupError):
            os.killpg(process.pid, signal.SIGKILL)
    try:
        process.wait(timeout=30)
    except subprocess.TimeoutExpired as error:
        raise QualificationFailure("release_process_tree_not_stopped") from error


def release_phase_durations(raw: str, expected: set[str]) -> dict[str, int]:
    events: dict[str, dict[str, int]] = defaultdict(dict)
    for phase, boundary, timestamp in re.findall(
        r"^qualification-phase ([a-z_]+) (start|end) ([0-9]+)$",
        raw,
        re.MULTILINE,
    ):
        if boundary in events[phase]:
            raise QualificationFailure("release_phase_timing_invalid")
        events[phase][boundary] = int(timestamp)
    if set(events) != expected or any(
        set(boundaries) != {"start", "end"}
        or boundaries["end"] < boundaries["start"]
        for boundaries in events.values()
    ):
        raise QualificationFailure("release_phase_timing_invalid")
    return {
        phase: boundaries["end"] - boundaries["start"]
        for phase, boundaries in sorted(events.items())
    }


def git(root: Path, *args: str) -> str:
    return run(["git", *args], cwd=root)


def load_config(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise QualificationFailure("invalid_provider_config") from error
    if not isinstance(value, dict):
        raise QualificationFailure("invalid_provider_config_shape")
    if set(value) != {"provider", "api_url", "model", "thinking_enabled", "api_key"}:
        raise QualificationFailure("invalid_provider_config_shape")
    key = value.get("api_key")
    if (
        value.get("provider") != EXPECTED_PROVIDER
        or value.get("api_url") != EXPECTED_API_URL
        or value.get("model") != EXPECTED_MODEL
        or value.get("thinking_enabled") is not True
        or not isinstance(key, str)
        or not key
        or len(key.encode("utf-8")) > 4096
        or any(ord(character) < 32 or ord(character) == 127 for character in key)
    ):
        raise QualificationFailure("provider_config_outside_slice")
    return value


def load_release_manifest(path: Path) -> dict[str, str]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise QualificationFailure("release_manifest_unreadable") from error
    values: dict[str, str] = {}
    for line in lines:
        if "=" not in line:
            raise QualificationFailure("release_manifest_invalid_line")
        key, value = line.split("=", 1)
        if key not in RELEASE_KEYS or not value or key in values:
            raise QualificationFailure("release_manifest_invalid_shape")
        values[key] = value
    if set(values) != RELEASE_KEYS:
        raise QualificationFailure("release_manifest_invalid_shape")
    if not COMMIT_PATTERN.fullmatch(values["RELEASE_GIT_SHA"]):
        raise QualificationFailure("release_manifest_invalid_git_sha")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]*", values["RELEASE_VERSION"]):
        raise QualificationFailure("release_manifest_invalid_version")
    if any(not IMAGE_PATTERN.fullmatch(values[key]) for key in RELEASE_IMAGE_KEYS):
        raise QualificationFailure("release_manifest_image_not_immutable")
    return values


def load_product_input(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise QualificationFailure("invalid_product_input") from error
    if set(value) != {
        "manifest_version",
        "case_id",
        "novel_title",
        "author",
        "deviation_mode",
        "chapters",
        "player",
        "branch_chat",
        "world_actions",
        "world_chats",
        "post_restart_chat",
    }:
        raise QualificationFailure("invalid_product_input_shape")
    chapters = value.get("chapters")
    player = value.get("player")
    if (
        value.get("manifest_version") != "h4-product-input-v1"
        or value.get("case_id") != "zh-self-world"
        or value.get("deviation_mode") != "creative"
        or any(
            not isinstance(value.get(key), str) or not value[key].strip()
            for key in ("novel_title", "author", "branch_chat", "post_restart_chat")
        )
        or not isinstance(chapters, list)
        or len(chapters) != 4
        or any(
            not isinstance(chapter, dict)
            or set(chapter) != {"title", "body"}
            or not isinstance(chapter["title"], str)
            or not isinstance(chapter["body"], str)
            or len(chapter["body"]) < 100
            for chapter in chapters
        )
        or not isinstance(player, dict)
        or set(player) != {"name", "background", "capabilities", "inventory"}
        or not isinstance(player.get("name"), str)
        or not player["name"].strip()
        or not isinstance(player.get("background"), str)
        or not player["background"].strip()
        or any(
            not isinstance(player.get(key), list)
            or not player[key]
            or not all(isinstance(item, str) and item.strip() for item in player[key])
            for key in ("capabilities", "inventory")
        )
        or not isinstance(value.get("world_actions"), list)
        or len(value["world_actions"]) != 12
        or not all(isinstance(item, str) and item.strip() for item in value["world_actions"])
        or not isinstance(value.get("world_chats"), list)
        or len(value["world_chats"]) != 11
        or not all(isinstance(item, str) and item.strip() for item in value["world_chats"])
    ):
        raise QualificationFailure("product_input_outside_slice")
    return value


def product_source(value: dict[str, Any]) -> bytes:
    return "\n\n".join(
        f"{chapter['title']}\n{chapter['body']}" for chapter in value["chapters"]
    ).encode("utf-8")


def load_cohort_manifest(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise QualificationFailure("invalid_cohort_manifest") from error
    if (
        not isinstance(value, dict)
        or set(value) != {"manifest_version", "cohort_id", "identity"}
        or value.get("manifest_version") != "h4-cohort-v1"
        or not isinstance(value.get("identity"), dict)
        or value.get("cohort_id") != sha256_bytes(canonical_json(value["identity"]))
    ):
        raise QualificationFailure("invalid_cohort_manifest_shape")
    return value


class QualificationLedger:
    def __init__(
        self,
        path: Path,
        cohort_id: str,
        attempt_id: str,
        journey_slice: str = "core",
    ):
        self.path = path
        self.cohort_id = cohort_id
        self.attempt_id = attempt_id
        self.journey_slice = journey_slice
        self.sequence: int | None = None
        self.lock_descriptor: int | None = None

    def _lock(self) -> None:
        lock_path = self.path.with_name(f"{self.path.name}.lock")
        descriptor = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
        try:
            if os.name == "nt":
                import msvcrt

                if os.fstat(descriptor).st_size == 0:
                    os.write(descriptor, b"\0")
                os.lseek(descriptor, 0, os.SEEK_SET)
                msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
            else:
                import fcntl

                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except (OSError, IOError) as error:
            os.close(descriptor)
            raise QualificationFailure("qualification_ledger_locked") from error
        self.lock_descriptor = descriptor

    def _unlock(self) -> None:
        descriptor = self.lock_descriptor
        if descriptor is None:
            return
        try:
            if os.name == "nt":
                import msvcrt

                os.lseek(descriptor, 0, os.SEEK_SET)
                msvcrt.locking(descriptor, msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(descriptor, fcntl.LOCK_UN)
        finally:
            os.close(descriptor)
            self.lock_descriptor = None

    def _records(self) -> list[dict[str, Any]]:
        try:
            lines = self.path.read_text(encoding="utf-8").splitlines()
            records = [json.loads(line) for line in lines if line.strip()]
        except (OSError, json.JSONDecodeError) as error:
            raise QualificationFailure("qualification_ledger_invalid") from error
        if any(not isinstance(record, dict) for record in records):
            raise QualificationFailure("qualification_ledger_invalid")
        return records

    def _append(self, status: str, failure_code: str | None = None) -> None:
        if self.sequence is None:
            raise QualificationFailure("qualification_ledger_not_started")
        record = {
            "schema_version": 1,
            "cohort_id": self.cohort_id,
            "attempt_sequence": self.sequence,
            "attempt_id": self.attempt_id,
            "evidence_class": "Qualification",
            "journey_slice": self.journey_slice,
            "status": status,
            "at": utc_now(),
            "failure_code": failure_code,
        }
        with self.path.open("a", encoding="utf-8", newline="\n") as stream:
            stream.write(canonical_json(record).decode("utf-8") + "\n")
            stream.flush()
            os.fsync(stream.fileno())

    def start(self) -> int:
        self._lock()
        keep_lock = False
        try:
            records = [
                record
                for record in self._records()
                if record.get("cohort_id") == self.cohort_id
                and record.get("evidence_class") == "Qualification"
            ]
            core_records = [
                record
                for record in records
                if record.get("journey_slice", "core") == "core"
            ]
            starts = [
                record for record in core_records if record.get("status") == "Started"
            ]
            terminals = {
                (record.get("attempt_sequence"), record.get("attempt_id")): record
                for record in core_records
                if record.get("status") in {"Passed", "Failed"}
            }
            compatibility_records = [
                record
                for record in records
                if record.get("journey_slice") == "legacy-character"
            ]
            if any(record.get("status") == "Failed" for record in records):
                raise QualificationFailure("cohort_terminal_failed")
            abandoned = next(
                (
                    record
                    for record in starts
                    if (record.get("attempt_sequence"), record.get("attempt_id"))
                    not in terminals
                ),
                None,
            )
            if self.journey_slice == "legacy-character":
                if abandoned is not None:
                    raise QualificationFailure("qualification_core_incomplete")
                passed = {
                    (record.get("attempt_sequence"), record.get("attempt_id"))
                    for record in core_records
                    if record.get("status") == "Passed"
                }
                if (
                    len(starts) != 3
                    or len(passed) != 3
                    or any(
                        (record.get("attempt_sequence"), record.get("attempt_id"))
                        not in passed
                        for record in starts
                    )
                ):
                    raise QualificationFailure("qualification_core_incomplete")
                compatibility_starts = [
                    record
                    for record in compatibility_records
                    if record.get("status") == "Started"
                ]
                compatibility_terminals = {
                    (record.get("attempt_sequence"), record.get("attempt_id")): record
                    for record in compatibility_records
                    if record.get("status") in {"Passed", "Failed"}
                }
                compatibility_abandoned = next(
                    (
                        record
                        for record in compatibility_starts
                        if (
                            record.get("attempt_sequence"),
                            record.get("attempt_id"),
                        )
                        not in compatibility_terminals
                    ),
                    None,
                )
                if compatibility_abandoned is not None:
                    self.sequence = int(compatibility_abandoned["attempt_sequence"])
                    self.attempt_id = str(compatibility_abandoned["attempt_id"])
                    self._append("Failed", "abandoned_attempt")
                    raise QualificationFailure("cohort_terminal_failed")
                if compatibility_starts:
                    raise QualificationFailure("compatibility_attempt_already_completed")
                self.sequence = 1
                self._append("Started")
                keep_lock = True
                return self.sequence
            if abandoned is not None:
                self.sequence = int(abandoned["attempt_sequence"])
                self.attempt_id = str(abandoned["attempt_id"])
                self._append("Failed", "abandoned_attempt")
                raise QualificationFailure("cohort_terminal_failed")
            if len(starts) >= 3:
                raise QualificationFailure("cohort_attempt_limit_reached")
            sequences = [record.get("attempt_sequence") for record in starts]
            if sequences != list(range(1, len(starts) + 1)):
                raise QualificationFailure("qualification_ledger_invalid")
            self.sequence = len(starts) + 1
            self._append("Started")
            keep_lock = True
            return self.sequence
        finally:
            if not keep_lock:
                self._unlock()

    def finish(self, passed: bool, failure_code: str | None) -> None:
        if self.lock_descriptor is None:
            return
        try:
            self._append("Passed" if passed else "Failed", failure_code)
        finally:
            self._unlock()


def request_bytes(
    url: str,
    *,
    method: str = "GET",
    token: str | None = None,
    body: bytes | None = None,
    headers: dict[str, str] | None = None,
    expected: Iterable[int] = (200,),
    timeout: float = 300,
) -> tuple[bytes, Any, int]:
    request_headers = {"Accept": "application/json", **(headers or {})}
    if token:
        request_headers["Authorization"] = f"Bearer {token}"
    request = urllib.request.Request(
        url,
        data=body,
        headers=request_headers,
        method=method,
    )
    try:
        response = urllib.request.urlopen(request, timeout=timeout)
        payload = response.read()
        status = response.status
        response_headers = response.headers
    except urllib.error.HTTPError as error:
        payload = error.read(64 * 1024)
        status = error.code
        response_headers = error.headers
    except (OSError, TimeoutError) as error:
        raise QualificationFailure("http_transport_failed") from error
    if status not in set(expected):
        code = "unknown"
        with contextlib.suppress(json.JSONDecodeError, UnicodeDecodeError, AttributeError):
            parsed = json.loads(payload)
            code = parsed.get("error", {}).get("code", "unknown")
        code = re.sub(r"[^a-z0-9_]+", "_", str(code).lower())[:80]
        raise QualificationFailure(f"http_{status}_{code}")
    return payload, response_headers, status


def request_json(
    url: str,
    *,
    method: str = "GET",
    token: str | None = None,
    value: Any | None = None,
    expected: Iterable[int] = (200,),
    timeout: float = 300,
) -> Any:
    body = None
    headers: dict[str, str] = {}
    if value is not None:
        body = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        headers["Content-Type"] = "application/json"
    payload, _, _ = request_bytes(
        url,
        method=method,
        token=token,
        body=body,
        headers=headers,
        expected=expected,
        timeout=timeout,
    )
    try:
        return json.loads(payload)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise QualificationFailure("invalid_json_response") from error


def request_no_content(url: str, *, method: str, token: str, value: Any) -> None:
    body = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    request_bytes(
        url,
        method=method,
        token=token,
        body=body,
        headers={"Content-Type": "application/json"},
        expected=(204,),
    )


def multipart(fields: dict[str, str], filename: str, content: bytes) -> tuple[bytes, str]:
    boundary = f"novelworld-{secrets.token_hex(16)}"
    chunks: list[bytes] = []
    for name, value in fields.items():
        chunks.extend(
            [
                f"--{boundary}\r\n".encode(),
                f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode(),
                value.encode("utf-8"),
                b"\r\n",
            ]
        )
    chunks.extend(
        [
            f"--{boundary}\r\n".encode(),
            (
                f'Content-Disposition: form-data; name="file"; filename="{filename}"\r\n'
                "Content-Type: text/plain; charset=utf-8\r\n\r\n"
            ).encode(),
            content,
            b"\r\n",
            f"--{boundary}--\r\n".encode(),
        ]
    )
    return b"".join(chunks), f"multipart/form-data; boundary={boundary}"


def parse_sse(payload: bytes) -> dict[str, Any]:
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as error:
        raise QualificationFailure("invalid_sse_encoding") from error
    event = "message"
    data: list[str] = []
    done: dict[str, Any] | None = None
    delta_chars = 0
    response_digest = hashlib.sha256()

    def finish() -> None:
        nonlocal event, data, done, delta_chars
        if not data:
            event = "message"
            return
        joined = "\n".join(data)
        try:
            value = json.loads(joined)
        except json.JSONDecodeError as error:
            raise QualificationFailure("invalid_sse_json") from error
        if event == "error":
            raise QualificationFailure("chat_stream_error")
        if event == "delta":
            content = value.get("content")
            if not isinstance(content, str):
                raise QualificationFailure("invalid_sse_delta")
            delta_chars += len(content)
            response_digest.update(content.encode("utf-8"))
        elif event == "done":
            done = value
        event = "message"
        data = []

    for raw in text.replace("\r\n", "\n").split("\n"):
        if not raw:
            finish()
        elif raw.startswith("event:"):
            event = raw[6:].strip()
        elif raw.startswith("data:"):
            data.append(raw[5:].lstrip())
    finish()
    if (
        done is None
        or done.get("committed") is not True
        or done.get("replayed") not in (True, False)
        or delta_chars < 1
    ):
        raise QualificationFailure("chat_stream_not_committed")
    return {
        "done": done,
        "response_chars": delta_chars,
        "response_sha256": response_digest.hexdigest(),
    }


def load_metric_parser(root: Path):
    path = root / "tools" / "llm-budget" / "verify.py"
    spec = importlib.util.spec_from_file_location("novelworld_llm_budget", path)
    if spec is None or spec.loader is None:
        raise QualificationFailure("metrics_parser_unavailable")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def summarize_metrics(root: Path, named_paths: list[tuple[str, Path]]) -> dict[str, Any]:
    parser = load_metric_parser(root)
    windows = []
    counter_totals: dict[tuple[str, str, str, str], float] = defaultdict(float)
    counter_names = {
        "novelworld_llm_requests_started_total": "requests_started",
        "novelworld_llm_attempts_total": "attempts",
        "novelworld_llm_retries_total": "retries",
        "novelworld_llm_requests_total": "requests",
        "novelworld_llm_usage_reports_total": "usage_reports",
        "novelworld_llm_tokens_total": "tokens",
        "novelworld_llm_billable_tokens_total": "billable_tokens",
    }
    histogram_names = {
        "novelworld_llm_attempt_duration_seconds": "attempt_duration_seconds",
        "novelworld_llm_stream_setup_duration_seconds": "stream_setup_duration_seconds",
        "novelworld_llm_request_duration_seconds": "request_duration_seconds",
        "novelworld_llm_first_token_duration_seconds": "first_token_duration_seconds",
        "novelworld_llm_output_token_limit": "output_token_limit",
        "novelworld_llm_tokens_per_request": "tokens_per_request",
    }
    for name, path in named_paths:
        raw = path.read_bytes()
        samples = parser.parse_metrics(raw)
        assert_metric_identity(samples)
        operations: dict[tuple[str, str, str, str], dict[str, Any]] = {}
        for metric, labels, value in samples:
            operation = labels.get("operation")
            provider = labels.get("provider")
            model = labels.get("model")
            service = labels.get("service")
            if not all((operation, provider, model, service)):
                continue
            key = service, provider, model, operation
            item = operations.setdefault(
                key,
                {
                    "service": service,
                    "provider": provider,
                    "model": model,
                    "operation": operation,
                    "counters": defaultdict(float),
                    "observed_quantiles": defaultdict(dict),
                },
            )
            if metric in counter_names:
                category = labels.get("status") or labels.get("reason") or labels.get("type") or labels.get("class") or "total"
                counter_key = f"{counter_names[metric]}.{category}"
                item["counters"][counter_key] += value
                counter_totals[(service, operation, counter_key, f"{provider}/{model}")] += value
            else:
                base = metric.removesuffix("_sum").removesuffix("_count")
                if base in histogram_names and metric == base and "quantile" in labels:
                    category = labels.get("status") or labels.get("type") or "all"
                    item["observed_quantiles"][f"{histogram_names[base]}.{category}"][labels["quantile"]] = value
        rendered = []
        for item in operations.values():
            if not item["counters"] and not item["observed_quantiles"]:
                continue
            item["counters"] = dict(sorted(item["counters"].items()))
            item["observed_quantiles"] = {
                key: dict(sorted(values.items()))
                for key, values in sorted(item["observed_quantiles"].items())
            }
            rendered.append(item)
        windows.append(
            {
                "name": name,
                "operations": sorted(rendered, key=lambda item: (item["service"], item["operation"])),
            }
        )
    totals = [
        {
            "service": service,
            "operation": operation,
            "provider_model": provider_model,
            "counter": counter,
            "value": value,
        }
        for (service, operation, counter, provider_model), value in sorted(counter_totals.items())
    ]
    return {"contract": "llm-observability-v1", "windows": windows, "counter_totals": totals}


def assert_metric_identity(
    samples: Iterable[tuple[str, dict[str, str], float]],
    *,
    service: str | None = None,
    operation: str | None = None,
) -> None:
    for metric, labels, _ in samples:
        if not metric.startswith("novelworld_llm_"):
            continue
        if service is not None and labels.get("service") != service:
            continue
        if operation is not None and labels.get("operation") != operation:
            continue
        if "provider" not in labels and "model" not in labels:
            continue
        if (
            labels.get("provider") != EXPECTED_PROVIDER
            or labels.get("model") != EXPECTED_MODEL
        ):
            raise QualificationFailure("provider_identity_changed")


def metric_counter_value(
    root: Path, raw: bytes, metric: str, required_labels: dict[str, str]
) -> float:
    parser = load_metric_parser(root)
    return sum(
        value
        for sample_metric, labels, value in parser.parse_metrics(raw)
        if sample_metric == metric
        and all(labels.get(key) == expected for key, expected in required_labels.items())
    )


def provider_started_delta(
    root: Path,
    before: bytes,
    after: bytes,
    *,
    service: str,
    operation: str | None = None,
) -> int:
    parser = load_metric_parser(root)
    assert_metric_identity(
        parser.parse_metrics(before), service=service, operation=operation
    )
    assert_metric_identity(
        parser.parse_metrics(after), service=service, operation=operation
    )
    labels = {
        "service": service,
        "provider": EXPECTED_PROVIDER,
        "model": EXPECTED_MODEL,
    }
    if operation is not None:
        labels["operation"] = operation
    metric = "novelworld_llm_requests_started_total"
    start = metric_counter_value(root, before, metric, labels)
    finish = metric_counter_value(root, after, metric, labels)
    if start < 0 or finish < start or not start.is_integer() or not finish.is_integer():
        raise QualificationFailure("provider_counter_epoch_invalid")
    return int(finish - start)


def selected_mid_from_logs(raw: str, trace_id: str) -> int:
    selected: list[int] = []
    for line in raw.splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        fields = entry.get("fields") if isinstance(entry, dict) else None
        if not isinstance(fields, dict):
            continue
        ancestors = entry.get("spans")
        spans = [
            entry.get("span"),
            *(ancestors if isinstance(ancestors, list) else []),
        ]
        trace_ids = {
            span.get("trace_id")
            for span in spans
            if isinstance(span, dict)
            and isinstance(span.get("trace_id"), str)
            and span.get("trace_id")
        }
        if trace_ids != {trace_id}:
            continue
        if fields.get("message") != "memory context selected":
            continue
        if fields.get("memory_layer") != "mid":
            continue
        value = fields.get("selected_count")
        if isinstance(value, int) and not isinstance(value, bool):
            selected.append(value)
    if len(selected) != 1 or selected[0] < 1:
        raise QualificationFailure("mid_selection_marker_missing")
    return selected[0]


def response_models_from_logs(raw: str, service: str) -> list[dict[str, str]]:
    observed = []
    for line in raw.splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        fields = entry.get("fields") if isinstance(entry, dict) else None
        if not isinstance(fields, dict) or fields.get("message") != "LLM response model observed":
            continue
        record = {
            "service": service,
            "provider": fields.get("provider"),
            "configured_model": fields.get("configured_model"),
            "response_model": fields.get("response_model"),
            "operation": fields.get("operation"),
            "mode": fields.get("mode"),
        }
        if (
            record["provider"] != EXPECTED_PROVIDER
            or record["configured_model"] != EXPECTED_MODEL
            or record["mode"] not in ("sync", "stream")
            or any(not isinstance(value, str) or not value for value in record.values())
        ):
            raise QualificationFailure("response_model_marker_invalid")
        observed.append(record)
    return observed


def unseen_response_models(
    raw: str, service: str, previous_count: int
) -> tuple[list[dict[str, str]], int]:
    observed = response_models_from_logs(raw, service)
    if previous_count < 0 or previous_count > len(observed):
        raise QualificationFailure("response_model_log_rewound")
    return observed[previous_count:], len(observed)


def verify_response_models(
    root: Path,
    named_paths: list[tuple[str, Path]],
    observations: list[dict[str, str]],
    allowed: list[str],
) -> dict[str, Any]:
    if not allowed or not all(valid_public_model(model) for model in allowed):
        raise QualificationFailure("response_model_allowlist_invalid")
    parser = load_metric_parser(root)
    expected: dict[tuple[str, str, str], int] = defaultdict(int)
    for _, path in named_paths:
        for metric, labels, value in parser.parse_metrics(path.read_bytes()):
            if metric != "novelworld_llm_requests_total" or labels.get("status") != "success":
                continue
            if labels.get("provider") != EXPECTED_PROVIDER or labels.get("model") != EXPECTED_MODEL:
                raise QualificationFailure("successful_provider_identity_invalid")
            if value < 0 or not value.is_integer():
                raise QualificationFailure("successful_provider_count_invalid")
            expected[(labels.get("service", ""), labels.get("operation", ""), labels.get("mode", ""))] += int(value)
    actual: dict[tuple[str, str, str], int] = defaultdict(int)
    models: dict[str, int] = defaultdict(int)
    for record in observations:
        model = record["response_model"]
        if model not in allowed:
            raise QualificationFailure("response_model_not_allowed")
        actual[(record["service"], record["operation"], record["mode"])] += 1
        models[model] += 1
    if not expected or actual != expected:
        raise QualificationFailure("response_model_observation_count_mismatch")
    return {
        "successful_calls": sum(actual.values()),
        "observed_response_models": dict(sorted(models.items())),
    }


def reserve_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def docker_inspect(kind: str, name: str) -> dict[str, Any]:
    command = ["docker"]
    if kind != "container":
        command.append(kind)
    command.extend(["inspect", name])
    try:
        value = json.loads(run(command))
    except json.JSONDecodeError as error:
        raise QualificationFailure("docker_inspect_invalid") from error
    if not isinstance(value, list) or len(value) != 1 or not isinstance(value[0], dict):
        raise QualificationFailure("docker_inspect_invalid")
    return value[0]


def docker_inventory_snapshot() -> dict[str, Any]:
    containers: dict[str, Any] = {}
    for name in sorted(
        filter(None, run(["docker", "ps", "-a", "--format", "{{.Names}}"]).splitlines())
    ):
        value = docker_inspect("container", name)
        state = value.get("State") or {}
        config = value.get("Config") or {}
        host = value.get("HostConfig") or {}
        networks = (value.get("NetworkSettings") or {}).get("Networks") or {}
        containers[name] = {
            "id": value.get("Id"),
            "image_id": value.get("Image"),
            "configured_image": config.get("Image"),
            "status": state.get("Status"),
            "started_at": state.get("StartedAt"),
            "restart_count": value.get("RestartCount"),
            "restart_policy": (host.get("RestartPolicy") or {}).get("Name"),
            "networks": sorted(networks),
            "labels": config.get("Labels") or {},
        }
    volumes: dict[str, Any] = {}
    for name in sorted(
        filter(
            None,
            run(["docker", "volume", "ls", "--format", "{{.Name}}"]).splitlines(),
        )
    ):
        value = docker_inspect("volume", name)
        volumes[name] = {
            key.lower(): value.get(key)
            for key in ("Name", "Driver", "Labels", "Options", "Scope")
        }
    networks: dict[str, Any] = {}
    for name in sorted(
        filter(
            None,
            run(["docker", "network", "ls", "--format", "{{.Name}}"]).splitlines(),
        )
    ):
        value = docker_inspect("network", name)
        networks[name] = {
            key.lower(): value.get(key)
            for key in (
                "Id",
                "Name",
                "Driver",
                "Scope",
                "Internal",
                "Attachable",
                "Ingress",
                "IPAM",
                "Labels",
            )
        }
        networks[name]["containers"] = {
            container_id: {
                "name": attachment.get("Name"),
                "endpoint_id": attachment.get("EndpointID"),
            }
            for container_id, attachment in sorted((value.get("Containers") or {}).items())
        }
    return {"containers": containers, "volumes": volumes, "networks": networks}


def attempt_resources(
    snapshot: dict[str, Any], project: str, prefix: str
) -> list[str]:
    found: list[str] = []
    for name, value in snapshot["containers"].items():
        labels = value.get("labels") or {}
        if labels.get("com.docker.compose.project") == project or name.startswith(
            f"{prefix}-"
        ):
            found.append(f"containers:{name}")
    for kind in ("volumes", "networks"):
        for name, value in snapshot[kind].items():
            labels = value.get("labels") or {}
            if labels.get("com.docker.compose.project") == project:
                found.append(f"{kind}:{name}")
    return sorted(found)


class Journey:
    def __init__(
        self,
        root: Path,
        config_path: Path,
        output: Path,
        git_sha: str,
        base_manifest_path: Path,
        candidate_manifest_path: Path,
        cohort_manifest_path: Path | None,
        ledger_path: Path | None,
        release_shell: str,
        evidence_class: str,
        journey_slice: str = "core",
    ):
        self.root = root
        self.config_path = config_path
        self.output = output
        self.git_sha = git_sha
        self.base_manifest_path = base_manifest_path
        self.candidate_manifest_path = candidate_manifest_path
        self.base_manifest = load_release_manifest(base_manifest_path)
        self.candidate_manifest = load_release_manifest(candidate_manifest_path)
        if (cohort_manifest_path is None) != (ledger_path is None):
            raise QualificationFailure("cohort_manifest_and_ledger_required_together")
        if evidence_class == "Qualification" and cohort_manifest_path is None:
            raise QualificationFailure("qualification_requires_cohort_and_ledger")
        self.cohort_manifest_path = cohort_manifest_path
        self.cohort_manifest = (
            load_cohort_manifest(cohort_manifest_path)
            if cohort_manifest_path is not None
            else None
        )
        self.ledger_path = ledger_path
        self.ledger: QualificationLedger | None = None
        self.release_shell = release_shell
        self.evidence_class = evidence_class
        self.journey_slice = journey_slice
        self.config = load_config(config_path)
        self.product_input_path = self.root / PRODUCT_INPUT
        self.product_input = load_product_input(self.product_input_path)
        suffix = secrets.token_hex(5)
        self.project = f"nwq-{suffix}"
        self.prefix = self.project
        self.port = reserve_port()
        self.api = f"http://127.0.0.1:{self.port}/api"
        self.compose_env: dict[str, str] = {}
        self.stack_started = False
        self.cleanup_required = False
        self.runtime_temp: tempfile.TemporaryDirectory[str] | None = None
        self.runtime_root: Path | None = None
        self.release_tool: Path | None = None
        self.release_state: Path | None = None
        self.current_stage = "preflight"
        self.metric_windows: dict[tuple[str, str], tuple[str, Path]] = {}
        self.response_model_observations: list[dict[str, str]] = []
        self.response_model_log_offsets: dict[str, tuple[str, int]] = {}
        self.user_stack_before: dict[str, Any] = {}
        self.inventory_captured = False
        self.attempt_id = str(uuid.uuid4())
        self.private_report: dict[str, Any] = {
            "attempt_id": self.attempt_id,
            "evaluated_git_sha": git_sha,
            "base_git_sha": self.base_manifest["RELEASE_GIT_SHA"],
            "base_manifest_sha256": sha256_bytes(base_manifest_path.read_bytes()),
            "candidate_manifest_sha256": sha256_bytes(
                candidate_manifest_path.read_bytes()
            ),
            "cohort_id": (
                self.cohort_manifest["cohort_id"]
                if self.cohort_manifest is not None
                else None
            ),
            "provider": {
                "api_origin": EXPECTED_API_URL,
                "product_thinking_enabled": True,
            },
        }
        self.report: dict[str, Any] = {
            "schema_version": 2,
            "report_kind": "h4-journey-qualification-v1",
            "evidence_class": evidence_class,
            "journey_slice": journey_slice,
            "qualification_claim": False,
            "attempt_id": self.attempt_id,
            "started_at": utc_now(),
            "completed_at": None,
            "outcome": "failed",
            "provider": {
                "name": EXPECTED_PROVIDER,
                "configured_model": EXPECTED_MODEL,
            },
            "environment": {
                "deployment": "production-compose",
                "network_bind": "loopback",
                "cache_mode": "postgres",
                "isolated_project": True,
                "existing_user_stack_unchanged": None,
            },
            "policy_identity": {
                "qualification": "private-preview-qualification-v1",
                "extraction": "extraction-quality-v1",
                "llm_metrics": "llm-observability-v1",
                "llm_budget": "h3-llm-budget-v2",
                "image_generation_configured": False,
                "image_generation_external_calls": 0,
                "journey": "h4-journey-qualification-v1",
                "product_input": self.product_input["manifest_version"],
                "product_input_manifest_sha256": sha256_bytes(
                    self.product_input_path.read_bytes()
                ),
            },
            "review": {
                "same_provider_automated_judgment_is_supporting_evidence_only": True,
                "human_quality_approval": False,
            },
            "stages": [],
            "journey": {},
        }

    @contextlib.contextmanager
    def stage(self, name: str):
        self.current_stage = name
        started = time.monotonic()
        record = {"name": name, "outcome": "failed", "duration_ms": None}
        self.report["stages"].append(record)
        try:
            yield
            record["outcome"] = "completed"
        finally:
            record["duration_ms"] = round((time.monotonic() - started) * 1000)

    def validate_release_inputs(self) -> None:
        base_sha = self.base_manifest["RELEASE_GIT_SHA"]
        if self.candidate_manifest["RELEASE_GIT_SHA"] != self.git_sha:
            raise QualificationFailure("candidate_manifest_sha_mismatch")
        if base_sha == self.git_sha:
            raise QualificationFailure("release_base_not_ancestor")
        for key in INFRASTRUCTURE_IMAGE_KEYS:
            if self.base_manifest[key] != self.candidate_manifest[key]:
                raise QualificationFailure("release_infrastructure_changed")
        if all(
            self.base_manifest[key] == self.candidate_manifest[key]
            for key in APPLICATION_IMAGE_KEYS
        ):
            raise QualificationFailure("release_application_images_unchanged")
        ancestry = subprocess.run(
            ["git", "merge-base", "--is-ancestor", base_sha, self.git_sha],
            cwd=self.root,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if ancestry.returncode != 0:
            raise QualificationFailure("release_base_not_ancestor")
        telemetry = git(
            self.root,
            "show",
            f"{base_sha}:crates/llm-client/src/telemetry.rs",
        )
        if "LLM response model observed" not in telemetry:
            raise QualificationFailure("base_response_model_observability_missing")

    def cohort_identity(
        self, docker_engine: str, docker_compose: str, declared: dict[str, Any]
    ) -> dict[str, Any]:
        controlled_keys = {
            "allowed_response_models",
            "base_application_image_ids",
            "candidate_application_image_ids",
            "browser_matrix",
            "assistive_technology_matrix",
            "viewport_device_matrix",
            "manual_review_role_matrix",
        }
        if not controlled_keys.issubset(declared):
            raise QualificationFailure("cohort_identity_incomplete")
        for key in (
            "allowed_response_models",
            "browser_matrix",
            "assistive_technology_matrix",
            "viewport_device_matrix",
            "manual_review_role_matrix",
        ):
            values = declared[key]
            if (
                not isinstance(values, list)
                or not values
                or not all(isinstance(value, str) and value.strip() for value in values)
                or len(set(values)) != len(values)
            ):
                raise QualificationFailure("cohort_identity_invalid_matrix")
        if not all(valid_public_model(value) for value in declared["allowed_response_models"]):
            raise QualificationFailure("cohort_identity_invalid_response_model")
        for key in ("base_application_image_ids", "candidate_application_image_ids"):
            values = declared[key]
            if (
                not isinstance(values, dict)
                or set(values) != set(APPLICATION_IMAGE_KEYS)
                or any(
                    not re.fullmatch(r"sha256:[0-9a-f]{64}", value)
                    for value in values.values()
                )
            ):
                raise QualificationFailure("cohort_identity_invalid_image_ids")
        base_ids = declared["base_application_image_ids"]
        candidate_ids = declared["candidate_application_image_ids"]
        if not any(
            self.base_manifest[key] != self.candidate_manifest[key]
            and base_ids[key] != candidate_ids[key]
            for key in APPLICATION_IMAGE_KEYS
        ):
            raise QualificationFailure("cohort_application_images_unchanged")
        schema_barriers = {
            str(path.relative_to(self.root)).replace("\\", "/"): sha256_bytes(
                path.read_bytes()
            )
            for path in (
                self.root / "infra/postgres/migrations/0021_world_turn_memory_projection.sql",
                self.root / "infra/postgres/migrations/0024_persona_provenance.sql",
                self.root / "infra/postgres/migrations/0025_chat_world_revision.sql",
            )
        }
        non_secret_config = {
            "provider": EXPECTED_PROVIDER,
            "api_origin": EXPECTED_API_URL,
            "configured_model": EXPECTED_MODEL,
            "product_thinking_enabled": True,
            "evaluation_thinking_enabled": False,
            "cache_mode": "postgres",
            "s3_enabled": False,
            "image_generation": "disabled-local-fail-closed",
            "image_generation_origin": "http://127.0.0.1:1",
            "http_bind": "127.0.0.1",
        }
        return {
            "evaluated_git_sha": self.git_sha,
            "clean_tree_proof": True,
            "compose_sha256": sha256_bytes((self.root / "docker-compose.yml").read_bytes()),
            "schema_barriers": schema_barriers,
            "docker_engine": docker_engine,
            "docker_compose": docker_compose,
            "profile": {
                "cache_mode": "postgres",
                "s3_enabled": False,
                "http_bind": "127.0.0.1",
            },
            "base_release_manifest_sha256": sha256_bytes(
                self.base_manifest_path.read_bytes()
            ),
            "candidate_release_manifest_sha256": sha256_bytes(
                self.candidate_manifest_path.read_bytes()
            ),
            "base_application_repositories": {
                key: self.base_manifest[key] for key in APPLICATION_IMAGE_KEYS
            },
            "candidate_application_repositories": {
                key: self.candidate_manifest[key] for key in APPLICATION_IMAGE_KEYS
            },
            "base_application_image_ids": declared["base_application_image_ids"],
            "candidate_application_image_ids": declared[
                "candidate_application_image_ids"
            ],
            "provider": {
                "name": EXPECTED_PROVIDER,
                "api_origin": EXPECTED_API_URL,
                "configured_model": EXPECTED_MODEL,
                "allowed_response_models": declared["allowed_response_models"],
                "product_thinking_enabled": True,
                "evaluation_thinking_enabled": False,
                "non_secret_config_sha256": sha256_bytes(
                    canonical_json(non_secret_config)
                ),
            },
            "versions": COHORT_VERSIONS,
            "registered_inputs": {
                "product": sha256_bytes(self.product_input_path.read_bytes()),
                "h1": sha256_bytes(
                    (self.root / "tools/h1-eval/corpus/v1.json").read_bytes()
                ),
                "h3": sha256_bytes(
                    (self.root / "tools/h3-eval/corpus/v1.json").read_bytes()
                ),
            },
            "browser_matrix": declared["browser_matrix"],
            "assistive_technology_matrix": declared[
                "assistive_technology_matrix"
            ],
            "viewport_device_matrix": declared["viewport_device_matrix"],
            "manual_review_role_matrix": declared["manual_review_role_matrix"],
        }

    def preflight(self) -> None:
        self.validate_release_inputs()
        docker_engine = run(
            [
                "docker",
                "version",
                "--format",
                "{{.Server.Version}}|{{.Server.Os}}|{{.Server.Arch}}",
            ]
        )
        parts = docker_engine.split("|")
        if len(parts) != 3 or parts[1] != "linux" or not all(parts):
            raise QualificationFailure("linux_docker_engine_required")
        docker_compose = run(["docker", "compose", "version", "--short"])
        self.user_stack_before = docker_inventory_snapshot()
        self.inventory_captured = True
        if attempt_resources(self.user_stack_before, self.project, self.prefix):
            raise QualificationFailure("qualification_project_not_empty")
        preflight_env = {
            **qualification_environment(self.root, dict(os.environ)),
            "POSTGRES_PASSWORD": "preflight-only",
            "JWT_SECRET": "preflight-only-32-characters-long",
            "RUNTIME_CONFIG_KEY": "0" * 64,
            "INTERNAL_SERVICE_TOKEN": "preflight-only",
            "CACHE_MODE": "postgres",
            "S3_ENABLED": "false",
            "CONTAINER_PREFIX": self.prefix,
            "NGINX_HTTP_BIND": "127.0.0.1",
            "NGINX_HTTP_PORT": str(self.port),
        }
        run(
            [
                "docker",
                "compose",
                "--project-name",
                self.project,
                "--project-directory",
                str(self.root),
                "-f",
                str(self.root / "docker-compose.yml"),
                "--env-file",
                str(self.candidate_manifest_path),
                "config",
                "--quiet",
            ],
            cwd=self.root,
            env=preflight_env,
        )
        self.private_report["environment"] = {
            "docker_engine": docker_engine,
            "docker_compose": docker_compose,
            "compose_sha256": sha256_bytes((self.root / "docker-compose.yml").read_bytes()),
            "existing_stack_before": self.user_stack_before,
        }
        if self.cohort_manifest is not None:
            declared = self.cohort_manifest["identity"]
            if declared != self.cohort_identity(docker_engine, docker_compose, declared):
                raise QualificationFailure("cohort_identity_mismatch")
        if self.evidence_class == "Qualification":
            if self.ledger_path is None or not self.ledger_path.is_file():
                raise QualificationFailure("qualification_ledger_missing")
            self.ledger = QualificationLedger(
                self.ledger_path,
                self.cohort_manifest["cohort_id"],
                self.attempt_id,
                self.journey_slice,
            )
            sequence = self.ledger.start()
            self.report["attempt_sequence"] = sequence
            self.private_report["attempt_sequence"] = sequence
        self.cleanup_required = True

    def prepare_runtime(self) -> None:
        self.runtime_temp = tempfile.TemporaryDirectory(prefix=f"{self.project}-")
        temporary_root = Path(self.runtime_temp.name)
        temporary_root.chmod(0o700)
        self.runtime_root = temporary_root / "repo"
        self.release_tool = temporary_root / "release.sh"
        self.release_tool.write_bytes(
            (self.root / "infra" / "docker" / "release.sh").read_bytes()
        )
        self.release_tool.chmod(0o700)
        self.release_state = temporary_root / "release-state"
        run(
            [
                "git",
                "clone",
                "--no-hardlinks",
                "--no-checkout",
                str(self.root),
                str(self.runtime_root),
            ]
        )
        git(self.runtime_root, "checkout", "--detach", self.git_sha)
        secrets_file = self.runtime_root / ".env"
        secret_lines = [
            "BOOTSTRAP_L0_COMPLETE=true",
            "POSTGRES_USER=novel",
            "POSTGRES_DB=novel_world",
            f"POSTGRES_PASSWORD={secrets.token_urlsafe(32)}",
            f"JWT_SECRET={secrets.token_urlsafe(48)}",
            f"RUNTIME_CONFIG_KEY={secrets.token_hex(32)}",
            f"INTERNAL_SERVICE_TOKEN={secrets.token_urlsafe(48)}",
            "LLM_API_KEY=",
            "CACHE_MODE=postgres",
            "REDIS_PASSWORD=",
            "REDIS_URL=memory://",
            "S3_ENABLED=false",
            "IMAGE_GEN_API_URL=http://127.0.0.1:1",
            "IMAGE_GEN_API_KEY=",
            f"CONTAINER_PREFIX={self.prefix}",
            "NGINX_HTTP_BIND=127.0.0.1",
            f"NGINX_HTTP_PORT={self.port}",
            f"CORS_ORIGINS=http://127.0.0.1:{self.port}",
            "",
        ]
        descriptor = os.open(
            secrets_file,
            os.O_CREAT | os.O_EXCL | os.O_WRONLY,
            0o600,
        )
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as stream:
            stream.write("\n".join(secret_lines))
        self.release_state.mkdir(mode=0o700)
        self.compose_env = {
            **qualification_environment(self.root, dict(os.environ)),
            "RELEASE_STATE_DIR": str(self.release_state),
            "RELEASE_COMPOSE_PROJECT": self.project,
            "RELEASE_CONTAINER_PREFIX": self.prefix,
            "RELEASE_HTTP_BIND": "127.0.0.1",
            "RELEASE_HTTP_PORT": str(self.port),
        }

    def compose_manifest(self) -> Path:
        if self.release_state is not None:
            current = self.release_state / "current.env"
            if current.is_file():
                return current
        return self.candidate_manifest_path

    def compose(self, *args: str, capture: bool = True, check: bool = True) -> str:
        if not self.compose_env or self.runtime_root is None:
            raise QualificationFailure("compose_environment_missing")
        return run(
            [
                "docker",
                "compose",
                "--project-name",
                self.project,
                "--project-directory",
                str(self.runtime_root),
                "-f",
                str(self.runtime_root / "docker-compose.yml"),
                "--env-file",
                str(self.runtime_root / ".env"),
                "--env-file",
                str(self.compose_manifest()),
                *args,
            ],
            cwd=self.runtime_root,
            env=self.compose_env,
            capture=capture,
            check=check,
        )

    def verify_release_images(
        self, stage: str, manifest: dict[str, str]
    ) -> dict[str, Any]:
        observed: dict[str, Any] = {}
        declared_ids = None
        if self.cohort_manifest is not None:
            declared_ids = self.cohort_manifest["identity"][
                f"{stage}_application_image_ids"
            ]
        for key, service in APPLICATION_CONTAINERS.items():
            reference = manifest[key]
            image_value = docker_inspect("image", reference)
            container_value = docker_inspect("container", f"{self.prefix}-{service}")
            image_id = image_value.get("Id")
            container_id = container_value.get("Id")
            if (
                not isinstance(image_id, str)
                or not re.fullmatch(r"sha256:[0-9a-f]{64}", image_id)
                or not isinstance(container_id, str)
                or not re.fullmatch(r"[0-9a-f]{64}", container_id)
                or reference not in (image_value.get("RepoDigests") or [])
                or (container_value.get("Config") or {}).get("Image") != reference
                or container_value.get("Image") != image_id
                or (declared_ids is not None and declared_ids[key] != image_id)
            ):
                raise QualificationFailure("release_image_identity_mismatch")
            observed[key] = {
                "repository_digest": reference,
                "image_id": image_id,
                "container_id": container_id,
            }
        self.private_report.setdefault("release_images", {})[stage] = observed
        self.report["environment"].setdefault(
            "application_image_content_digests", {}
        )[stage] = {key: value["image_id"] for key, value in observed.items()}
        base = self.private_report["release_images"].get("base")
        if stage == "candidate" and base is not None and not any(
            base[key]["repository_digest"] != observed[key]["repository_digest"]
            and base[key]["image_id"] != observed[key]["image_id"]
            for key in APPLICATION_IMAGE_KEYS
        ):
            raise QualificationFailure("release_application_content_unchanged")
        return observed

    def postgres_volume_identity(self) -> dict[str, Any]:
        container = docker_inspect("container", f"{self.prefix}-postgres")
        mounts = [
            mount
            for mount in container.get("Mounts") or []
            if mount.get("Type") == "volume"
            and mount.get("Destination") == "/var/lib/postgresql/data"
        ]
        if len(mounts) != 1 or not isinstance(mounts[0].get("Name"), str):
            raise QualificationFailure("postgres_authority_volume_missing")
        mount = mounts[0]
        volume = docker_inspect("volume", mount["Name"])
        identity = {
            "name": mount["Name"],
            "source": mount.get("Source"),
            "destination": mount.get("Destination"),
            "driver": volume.get("Driver"),
            "labels": volume.get("Labels") or {},
        }
        if any(value in (None, "") for value in identity.values()):
            raise QualificationFailure("postgres_authority_volume_invalid")
        return identity

    def prepare_compose(self) -> None:
        if not PROJECT_PATTERN.fullmatch(self.project):
            raise QualificationFailure("unsafe_compose_project")
        self.prepare_runtime()

    def wait_gateway(self, attempts: int = 180) -> None:
        for _ in range(attempts):
            try:
                request_json(f"{self.api}/setup/status", timeout=5)
                return
            except QualificationFailure:
                time.sleep(2)
        raise QualificationFailure("gateway_not_ready")

    def db_scalar(self, sql: str) -> str:
        return run(
            [
                "docker",
                "exec",
                f"{self.prefix}-postgres",
                "psql",
                "-U",
                "novel",
                "-d",
                "novel_world",
                "-v",
                "ON_ERROR_STOP=1",
                "-At",
                "-c",
                sql,
            ]
        )

    def authority_snapshot(
        self, user_id: str, novel_id: str, *, include_failed_turns: bool = True
    ) -> str:
        journal_filter = "" if include_failed_turns else " AND w.status <> 'failed'"
        return self.db_scalar(
            "SELECT jsonb_build_object("
            "'novel', (SELECT to_jsonb(n) - ARRAY['cover_url', 'created_at', 'updated_at'] "
            f"FROM novels n WHERE n.id = '{novel_id}'), "
            "'chapters', (SELECT COALESCE(jsonb_agg(to_jsonb(c) - ARRAY['created_at'] "
            "ORDER BY c.chapter_number, c.id), '[]') FROM chapters c "
            f"WHERE c.novel_id = '{novel_id}'), "
            "'chunks', (SELECT COALESCE(jsonb_agg(to_jsonb(chunk) ORDER BY chapter.chapter_number, chunk.chunk_index), '[]') "
            "FROM chapter_chunks chunk JOIN chapters chapter ON chapter.id = chunk.chapter_id "
            f"WHERE chapter.novel_id = '{novel_id}'), "
            "'characters', (SELECT COALESCE(jsonb_agg(to_jsonb(c) - ARRAY["
            "'avatar_url', 'avatar_status', 'created_at', 'updated_at'] ORDER BY c.id), '[]') "
            f"FROM characters c WHERE c.novel_id = '{novel_id}'), "
            "'canon_models', (SELECT COALESCE(jsonb_agg(to_jsonb(c) - ARRAY['created_at'] "
            "ORDER BY c.model_version, c.id), '[]') FROM canon_story_models c "
            f"WHERE c.novel_id = '{novel_id}'), "
            "'relationships', (SELECT COALESCE(jsonb_agg(to_jsonb(r) - ARRAY['created_at'] "
            "ORDER BY r.from_character_id, r.to_character_id, r.id), '[]') "
            f"FROM character_relationships r WHERE r.novel_id = '{novel_id}'), "
            "'shelf', (SELECT to_jsonb(s) - ARRAY['added_at'] FROM user_novels s "
            f"WHERE s.user_id = '{user_id}' AND s.novel_id = '{novel_id}'), "
            "'progress', (SELECT to_jsonb(p) - ARRAY['last_read_at', 'created_at'] FROM reading_progress p "
            f"WHERE p.user_id = '{user_id}' AND p.novel_id = '{novel_id}'), "
            "'nodes', (SELECT COALESCE(jsonb_agg(to_jsonb(n) - ARRAY['created_at'] ORDER BY n.chapter_number, n.id), '[]') "
            "FROM narrative_nodes n "
            f"WHERE n.novel_id = '{novel_id}' AND (n.user_id = '{user_id}' OR n.user_id IS NULL)), "
            "'choices', (SELECT COALESCE(jsonb_agg(to_jsonb(c) - ARRAY['created_at'] ORDER BY c.chapter_number, c.id), '[]') "
            "FROM user_choices c "
            f"WHERE c.user_id = '{user_id}' AND c.novel_id = '{novel_id}'), "
            "'player_chapters', (SELECT COALESCE(jsonb_agg(to_jsonb(p) - ARRAY['created_at', 'updated_at'] ORDER BY p.chapter_number, p.id), '[]') "
            "FROM player_chapters p "
            f"WHERE p.user_id = '{user_id}' AND p.novel_id = '{novel_id}'), "
            "'world_state', (SELECT to_jsonb(w) - ARRAY['updated_at'] FROM world_states w "
            f"WHERE w.user_id = '{user_id}' AND w.novel_id = '{novel_id}'), "
            "'journal', (SELECT COALESCE(jsonb_agg(to_jsonb(w) - ARRAY["
            "'attempt', 'lease_expires_at', 'created_at', 'updated_at', 'completed_at', "
            "'memory_projection_completed_at'] ORDER BY w.expected_turn_number, w.id), '[]') "
            "FROM world_turns w "
            f"WHERE w.user_id = '{user_id}' AND w.novel_id = '{novel_id}'{journal_filter}), "
            "'permanent_memories', (SELECT COALESCE(jsonb_agg(to_jsonb(m) - ARRAY["
            "'access_count', 'last_accessed', 'created_at', 'expires_at'] ORDER BY m.id), '[]') "
            "FROM character_memories m "
            f"WHERE m.user_id = '{user_id}' AND m.novel_id = '{novel_id}' "
            "AND m.layer = 'permanent'), "
            "'chat_turns', (SELECT COALESCE(jsonb_agg(to_jsonb(t) - ARRAY["
            "'attempt', 'lease_expires_at', 'created_at', 'updated_at', 'completed_at'] "
            "ORDER BY t.created_at, t.id), '[]') "
            "FROM chat_turns t "
            f"WHERE t.user_id = '{user_id}' AND t.novel_id = '{novel_id}'), "
            "'chat_messages', (SELECT COALESCE(jsonb_agg(to_jsonb(m) - ARRAY['created_at'] ORDER BY m.created_at, m.id), '[]') "
            "FROM chat_messages m "
            f"WHERE m.user_id = '{user_id}' AND m.novel_id = '{novel_id}'))::text"
        )

    def world_turn(
        self,
        token: str,
        novel_id: str,
        turn_id: str,
        action: dict[str, Any],
        *,
        expected: Iterable[int] = (200,),
        timeout: float = 600,
    ) -> tuple[Any, Any, int]:
        body = json.dumps(
            action, ensure_ascii=False, separators=(",", ":")
        ).encode("utf-8")
        payload, headers, status = request_bytes(
            f"{self.api}/narrative/{novel_id}/world/turns",
            method="POST",
            token=token,
            body=body,
            headers={"Content-Type": "application/json", "Idempotency-Key": turn_id},
            expected=expected,
            timeout=timeout,
        )
        try:
            return json.loads(payload), headers, status
        except (json.JSONDecodeError, UnicodeDecodeError) as error:
            raise QualificationFailure("invalid_world_turn_response") from error

    @staticmethod
    def require_error(value: Any, code: str) -> None:
        if not isinstance(value, dict) or value.get("error", {}).get("code") != code:
            raise QualificationFailure("typed_error_mismatch")

    def wait_agent_ready(self) -> float:
        for _ in range(120):
            started = time.monotonic()
            result = subprocess.run(
                [
                    "docker",
                    "exec",
                    f"{self.prefix}-agent-service",
                    "curl",
                    "--fail",
                    "--silent",
                    "--max-time",
                    "2",
                    "http://127.0.0.1:8003/ready",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                check=False,
            )
            if result.returncode == 0 and result.stdout.strip():
                return started
            time.sleep(1)
        raise QualificationFailure("agent_not_ready")

    def internal_character_context(self, user_id: str, novel_id: str, character_id: str) -> Any:
        raw = run(
            [
                "docker",
                "exec",
                "-e",
                f"QUAL_USER_ID={user_id}",
                "-e",
                f"QUAL_NOVEL_ID={novel_id}",
                "-e",
                f"QUAL_CHARACTER_ID={character_id}",
                f"{self.prefix}-narrative-service",
                "/bin/sh",
                "-ec",
                "curl --fail --silent --show-error "
                "-H \"X-Internal-Service-Token: $INTERNAL_SERVICE_TOKEN\" "
                "-H \"X-User-Id: $QUAL_USER_ID\" "
                "-H \"X-World-Context-Version: 4\" "
                "\"http://127.0.0.1:8004/internal/narrative/$QUAL_NOVEL_ID/characters/$QUAL_CHARACTER_ID/context\"",
            ]
        )
        try:
            return json.loads(raw)
        except json.JSONDecodeError as error:
            raise QualificationFailure("invalid_internal_context") from error

    def chat(
        self,
        token: str,
        novel_id: str,
        character_id: str,
        message: str,
        *,
        trace_id: str | None = None,
        turn_id: str | None = None,
        expected_replayed: bool = False,
    ) -> dict[str, Any]:
        turn_id = turn_id or str(uuid.uuid4())
        trace_id = trace_id or str(uuid.uuid4())
        body = json.dumps(
            {"message": message, "novel_id": novel_id},
            ensure_ascii=False,
            separators=(",", ":"),
        ).encode("utf-8")
        payload, headers, _ = request_bytes(
            f"{self.api}/chat/{character_id}/stream",
            method="POST",
            token=token,
            body=body,
            headers={
                "Content-Type": "application/json",
                "Idempotency-Key": turn_id,
                "X-Trace-Id": trace_id,
            },
            timeout=600,
        )
        if "text/event-stream" not in headers.get("Content-Type", ""):
            raise QualificationFailure("chat_not_sse")
        parsed = parse_sse(payload)
        if (
            parsed["done"].get("turn_id") != turn_id
            or parsed["done"].get("replayed") is not expected_replayed
        ):
            raise QualificationFailure("chat_turn_identity_mismatch")
        parsed["turn_id"] = turn_id
        parsed["trace_id"] = trace_id
        return parsed

    def chat_authority(self, user_id: str, novel_id: str) -> str:
        return self.db_scalar(
            "SELECT jsonb_build_object("
            "'turns', (SELECT COALESCE(jsonb_agg(to_jsonb(t) - ARRAY["
            "'attempt', 'lease_expires_at', 'created_at', 'updated_at', 'completed_at'] "
            "ORDER BY t.id), '[]') FROM chat_turns t "
            f"WHERE t.user_id = '{user_id}' AND t.novel_id = '{novel_id}'), "
            "'messages', (SELECT COALESCE(jsonb_agg(to_jsonb(m) - ARRAY['created_at'] "
            "ORDER BY m.id), '[]') FROM chat_messages m "
            f"WHERE m.user_id = '{user_id}' AND m.novel_id = '{novel_id}'))::text"
        )

    def assert_chat_revision(self, turn_id: str, expected_revision: list[int]) -> None:
        if len(expected_revision) != 32 or any(not isinstance(value, int) or not 0 <= value <= 255 for value in expected_revision):
            raise QualificationFailure("invalid_world_revision")
        expected = bytes(expected_revision).hex()
        observed = self.db_scalar(
            "SELECT status || ':' || encode(world_revision, 'hex') || ':' || "
            f"(SELECT COUNT(*) FROM chat_messages WHERE turn_id = chat_turns.id) FROM chat_turns WHERE id = '{turn_id}'"
        )
        if observed != f"completed:{expected}:2":
            raise QualificationFailure("chat_revision_not_exact")

    def export_user(
        self,
        *,
        token: str,
        password: str,
        artifact: str,
        required_kinds: set[str],
    ) -> tuple[list[dict[str, Any]], set[str]]:
        export, headers, _ = request_bytes(
            f"{self.api}/account/export", token=token, timeout=300
        )
        if "application/x-ndjson" not in headers.get("Content-Type", ""):
            raise QualificationFailure("export_content_type_invalid")
        try:
            records = [json.loads(line) for line in export.splitlines() if line]
        except json.JSONDecodeError as error:
            raise QualificationFailure("export_invalid_ndjson") from error
        if not records or records[-1].get("type") != "complete":
            raise QualificationFailure("export_incomplete")
        kinds = {
            record.get("kind")
            for record in records
            if record.get("type") == "record"
        }
        if not required_kinds.issubset(kinds):
            raise QualificationFailure("export_missing_records")
        source_counts = {source: 0 for source in ("canon", "uncertain", "reader", "generated", "mixed")}
        for record in records:
            if record.get("type") != "record":
                continue
            kind = record.get("kind")
            data = record.get("data")
            if not isinstance(kind, str) or not isinstance(data, dict):
                raise QualificationFailure("export_record_shape_invalid")
            expected_source = expected_export_source(kind, data)
            if expected_source is None:
                continue
            if data.get("source") != expected_source:
                raise QualificationFailure("export_source_provenance_invalid")
            source_counts[expected_source] += 1
        if any(
            secret_value in export
            for secret_value in (
                self.config["api_key"].encode(),
                token.encode(),
                password.encode(),
            )
        ):
            raise QualificationFailure("export_contains_secret")
        write_private(self.output / artifact, export)
        review = self.private_report.setdefault(
            "human_review", {"approval_recorded": False, "artifacts": {}}
        )
        review["artifacts"][artifact] = {
            "sha256": sha256_bytes(export),
            "record_count": len(records),
            "record_kinds": sorted(kinds),
            "source_counts": source_counts,
        }
        self.report["journey"].update(
            {
                f"export_source_{source}_records": count
                for source, count in source_counts.items()
            }
        )
        return records, kinds

    def delete_user(
        self,
        *,
        token: str,
        email: str,
        password: str,
        user_id: str,
        novel_id: str,
        character_id: str,
    ) -> dict[str, int]:
        request_bytes(
            f"{self.api}/auth/me",
            method="DELETE",
            token=token,
            expected=(204,),
        )
        request_bytes(
            f"{self.api}/auth/login",
            method="POST",
            body=json.dumps({"email": email, "password": password}).encode(),
            headers={"Content-Type": "application/json"},
            expected=(401,),
        )
        agent_before = self.service_metrics("agent-service")
        narrative_before = self.service_metrics("narrative-service")
        request_bytes(f"{self.api}/auth/me", token=token, expected=(404,))
        request_bytes(f"{self.api}/account/export", token=token, expected=(404,))
        request_bytes(f"{self.api}/progress/{novel_id}", token=token, expected=(404,))
        request_bytes(
            f"{self.api}/chat/{character_id}/stream",
            method="POST",
            token=token,
            body=json.dumps(
                {"message": "删除后的令牌不得触发模型调用", "novel_id": novel_id},
                ensure_ascii=False,
                separators=(",", ":"),
            ).encode("utf-8"),
            headers={
                "Content-Type": "application/json",
                "Idempotency-Key": str(uuid.uuid4()),
            },
            expected=(404,),
        )
        self.world_turn(
            token,
            novel_id,
            str(uuid.uuid4()),
            {
                "expected_turn_number": 0,
                "kind": "converse",
                "target_id": character_id,
                "intent": "删除后的令牌不得触发世界行动",
            },
            expected=(404,),
        )
        provider_calls = provider_started_delta(
            self.root,
            agent_before,
            self.service_metrics("agent-service"),
            service="agent-service",
        ) + provider_started_delta(
            self.root,
            narrative_before,
            self.service_metrics("narrative-service"),
            service="narrative-service",
        )
        private_counts = self.db_scalar(
            f"SELECT (SELECT COUNT(*) FROM users WHERE id = '{user_id}') + "
            + " + ".join(
                f"(SELECT COUNT(*) FROM {table} WHERE user_id = '{user_id}')"
                for table in PRIVATE_TABLES
            )
        )
        if private_counts != "0" or provider_calls != 0:
            raise QualificationFailure("account_erasure_incomplete")
        return {"negative_cases": 6, "provider_calls": provider_calls, "private_rows": 0}

    def collect_metrics(
        self,
        name: str,
        services: Iterable[str] = SERVICE_PORTS,
        *,
        best_effort: bool = False,
    ) -> None:
        errors = []
        for service in services:
            try:
                container = f"{self.prefix}-{service}"
                generation = run(
                    [
                        "docker",
                        "inspect",
                        "--format",
                        "{{.Id}}|{{.State.StartedAt}}",
                        container,
                    ]
                )
                generation_key = (service, generation)
                raw = self.service_metrics(service)
                payload = raw + b"\n"
                destination = self.output / f"product-{name}-{service}.prom"
                write_private(destination, payload)
                window = f"{name}:{service}"
                self.metric_windows[generation_key] = (window, destination)
                self.private_report.setdefault("metric_files", {})[
                    window
                ] = sha256_bytes(payload)
            except QualificationFailure as error:
                errors.append(f"{service}:{error.code}")
        if errors:
            self.private_report.setdefault("metric_collection_errors", {})[
                name
            ] = errors
            if not best_effort:
                raise QualificationFailure("metrics_collection_failed")
        self.private_report["metric_window_selection"] = [
            {
                "service": service,
                "process_generation": process_generation,
                "window": window,
                "artifact": path.name,
            }
            for (service, process_generation), (window, path) in sorted(
                self.metric_windows.items()
            )
        ]

    def service_metrics(self, service: str) -> bytes:
        port = SERVICE_PORTS[service]
        return run(
            [
                "docker",
                "exec",
                f"{self.prefix}-{service}",
                "curl",
                "--fail",
                "--silent",
                f"http://127.0.0.1:{port}/metrics",
            ]
        ).encode("utf-8")

    def collect_response_models(
        self,
        generation: str,
        services: Iterable[str] = SERVICE_PORTS,
        *,
        best_effort: bool = False,
    ) -> None:
        before = len(self.response_model_observations)
        errors = []
        for service in services:
            try:
                container = f"{self.prefix}-{service}"
                container_id = run(
                    ["docker", "inspect", "--format", "{{.Id}}", container]
                )
                raw = run(["docker", "logs", container])
                previous_id, previous_count = self.response_model_log_offsets.get(
                    service, ("", 0)
                )
                if previous_id != container_id:
                    previous_count = 0
                new, total = unseen_response_models(raw, service, previous_count)
                self.response_model_observations.extend(new)
                self.response_model_log_offsets[service] = (container_id, total)
            except QualificationFailure as error:
                errors.append(f"{service}:{error.code}")
        self.private_report.setdefault("response_model_marker_counts", {})[
            generation
        ] = len(self.response_model_observations) - before
        self.private_report["response_model_observations"] = list(
            self.response_model_observations
        )
        if errors:
            self.private_report.setdefault("response_model_collection_errors", {})[
                generation
            ] = errors
            if not best_effort:
                raise QualificationFailure("response_model_collection_failed")

    def finalize_observability(
        self, generation: str, *, best_effort: bool = False
    ) -> None:
        errors = []
        self.collect_metrics(generation, best_effort=True)
        self.collect_response_models(generation, best_effort=True)
        if generation in self.private_report.get("metric_collection_errors", {}):
            errors.append("metrics_collection_failed")
        if generation in self.private_report.get(
            "response_model_collection_errors", {}
        ):
            errors.append("response_model_collection_failed")

        windows = list(self.metric_windows.values())
        allowed = (
            self.cohort_manifest["identity"]["allowed_response_models"]
            if self.cohort_manifest is not None
            else [EXPECTED_MODEL]
        )
        try:
            self.report["provider"].update(
                verify_response_models(
                    self.root,
                    windows,
                    self.response_model_observations,
                    allowed,
                )
            )
        except QualificationFailure as error:
            errors.append(error.code)
        try:
            self.report["llm_metrics"] = summarize_metrics(self.root, windows)
        except (QualificationFailure, OSError, ValueError):
            errors.append("llm_metrics_invalid")
        try:
            budget = load_metric_parser(self.root).verify_many(
                self.root / "tools/llm-budget/policy-v2.json",
                [path for _, path in windows],
                self.git_sha,
            )
            self.private_report["llm_budget"] = budget
            if not budget.get("passed"):
                errors.append("llm_budget_failed")
        except Exception:
            errors.append("llm_budget_evidence_invalid")

        errors = list(dict.fromkeys(errors))
        if errors:
            self.private_report.setdefault("observability_errors", {})[
                generation
            ] = errors
            if not best_effort:
                raise QualificationFailure(errors[0])
            return
        self.private_report["observability_finalized"] = True
        self.report["journey"]["llm_budget_passed"] = True

    def release(
        self,
        command: str,
        manifest: Path,
        gate: Callable[[], None] | None = None,
        release_name: str | None = None,
    ) -> int:
        if self.runtime_root is None or self.release_tool is None:
            raise QualificationFailure("release_environment_missing")
        log_path = self.output / f"release-{command}.log"
        descriptor = os.open(log_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        started = time.monotonic()
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as log:
            process = subprocess.Popen(
                [self.release_shell, str(self.release_tool), command, str(manifest)],
                cwd=self.runtime_root,
                env=self.compose_env,
                stdin=subprocess.PIPE,
                stdout=log,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                creationflags=(
                    subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0
                ),
                start_new_session=os.name != "nt",
            )
            if gate is not None:
                prompt = f"then enter {self.git_sha}:"
                deadline = time.monotonic() + 1_800
                while process.poll() is None:
                    log.flush()
                    if prompt in log_path.read_text(encoding="utf-8", errors="replace"):
                        break
                    if time.monotonic() >= deadline:
                        terminate_process_tree(process)
                        raise QualificationFailure("release_gate_timeout")
                    time.sleep(0.2)
                else:
                    raise QualificationFailure(f"release_{command}_failed")
                try:
                    gate()
                except Exception:
                    if process.stdin is not None:
                        process.stdin.write("\n")
                        process.stdin.flush()
                        process.stdin.close()
                    try:
                        process.wait(timeout=600)
                    except subprocess.TimeoutExpired:
                        terminate_process_tree(process)
                    raise
                if process.stdin is None:
                    terminate_process_tree(process)
                    raise QualificationFailure("release_gate_stdin_missing")
                process.stdin.write(f"{self.git_sha}\n")
                process.stdin.flush()
                process.stdin.close()
            try:
                return_code = process.wait(timeout=1_800)
            except subprocess.TimeoutExpired as error:
                terminate_process_tree(process)
                raise QualificationFailure(f"release_{command}_timeout") from error
        if return_code:
            raise QualificationFailure(f"release_{command}_failed")
        duration = round((time.monotonic() - started) * 1000)
        expected = {"pull", "migration", "application_deployment", "readiness"}
        if command == "adopt":
            expected.add("database_start")
        phases = release_phase_durations(
            log_path.read_text(encoding="utf-8", errors="replace"), expected
        )
        release_name = release_name or ("base" if command == "adopt" else "candidate")
        self.private_report.setdefault("release_phase_durations_ms", {})[
            release_name
        ] = phases
        self.report["journey"].update(
            {
                f"{release_name}_{phase}_duration_ms": value
                for phase, value in phases.items()
            }
        )
        return duration

    def run_legacy_character_slice(
        self,
        *,
        password: str,
        novel_id: str,
        total_chapters: int,
        checkpoint: int,
    ) -> None:
        with self.stage("legacy_character_identity_compatibility"):
            compatibility_email = (
                f"character-{secrets.token_hex(8)}@qualification.invalid"
            )
            compatibility_reader = request_json(
                f"{self.api}/auth/register",
                method="POST",
                value={
                    "email": compatibility_email,
                    "password": password,
                    "name": "Character Compatibility Reader",
                },
                expected=(201,),
            )
            compatibility_token = compatibility_reader["access_token"]
            compatibility_user_id = compatibility_reader["user"]["id"]
            if compatibility_reader["user"].get("role") != "user":
                raise QualificationFailure("compatibility_reader_is_not_ordinary_user")
            request_no_content(
                f"{self.api}/novels/{novel_id}/shelf",
                method="POST",
                token=compatibility_token,
                value={"deviation_mode": self.product_input["deviation_mode"]},
            )
            request_no_content(
                f"{self.api}/progress/{novel_id}/identity",
                method="PUT",
                token=compatibility_token,
                value={
                    "identity_type": "self",
                    "identity_name": self.product_input["player"]["name"],
                    "character_id": None,
                },
            )
            request_no_content(
                f"{self.api}/progress/{novel_id}",
                method="PUT",
                token=compatibility_token,
                value={"current_chapter": total_chapters},
            )
            compatibility_entry = request_json(
                f"{self.api}/narrative/{novel_id}/player-entry?"
                + urllib.parse.urlencode({"checkpoint_chapter": checkpoint}),
                token=compatibility_token,
            )
            compatibility_locations = compatibility_entry.get("locations")
            if (
                not isinstance(compatibility_locations, list)
                or not compatibility_locations
                or not isinstance(compatibility_locations[0].get("id"), str)
            ):
                raise QualificationFailure("compatibility_player_entry_has_no_location")
            request_json(
                f"{self.api}/narrative/{novel_id}/player-entry",
                method="PUT",
                token=compatibility_token,
                value={
                    "checkpoint_chapter": checkpoint,
                    "name": self.product_input["player"]["name"],
                    "background": self.product_input["player"]["background"],
                    "capabilities": self.product_input["player"]["capabilities"],
                    "location_id": compatibility_locations[0]["id"],
                    "inventory": self.product_input["player"]["inventory"],
                },
            )
            compatibility_node = request_json(
                f"{self.api}/narrative/{novel_id}/{checkpoint}",
                token=compatibility_token,
                timeout=600,
            )
            compatibility_node_id = compatibility_node.get("id")
            if not isinstance(compatibility_node_id, str):
                raise QualificationFailure("compatibility_branch_node_missing")
            full_characters = request_json(
                f"{self.api}/novels/{novel_id}/characters", token=compatibility_token
            )
            if (
                len(full_characters) < 2
                or any(
                    character.get("persona_source_chapter_high_water")
                    != total_chapters
                    or "system_prompt" in character
                    for character in full_characters
                )
            ):
                raise QualificationFailure("compatibility_persona_provenance_invalid")
            provisional_identity_id = full_characters[0].get("id")
            if not isinstance(provisional_identity_id, str):
                raise QualificationFailure("compatibility_character_identity_missing")
            request_no_content(
                f"{self.api}/progress/{novel_id}/identity",
                method="PUT",
                token=compatibility_token,
                value={
                    "identity_type": "character",
                    "identity_name": None,
                    "character_id": provisional_identity_id,
                },
            )
            cached_authority = self.authority_snapshot(
                compatibility_user_id, novel_id
            )
            cached_narrative_before = self.service_metrics("narrative-service")
            cached_read = request_json(
                f"{self.api}/narrative/{novel_id}/{checkpoint}",
                token=compatibility_token,
                expected=(404,),
            )
            self.require_error(cached_read, "not_found")
            cached_choice = request_json(
                f"{self.api}/narrative/choose",
                method="POST",
                token=compatibility_token,
                value={
                    "novel_id": novel_id,
                    "node_id": compatibility_node_id,
                    "choice_index": 0,
                },
                expected=(409,),
            )
            self.require_error(cached_choice, "conflict")
            if (
                provider_started_delta(
                    self.root,
                    cached_narrative_before,
                    self.service_metrics("narrative-service"),
                    service="narrative-service",
                )
                != 0
                or self.authority_snapshot(compatibility_user_id, novel_id)
                != cached_authority
                or self.db_scalar(
                    "SELECT COUNT(*) FROM user_choices "
                    f"WHERE user_id = '{compatibility_user_id}' AND node_id = '{compatibility_node_id}'"
                )
                != "0"
            ):
                raise QualificationFailure("compatibility_cached_node_boundary_mutated")
            request_no_content(
                f"{self.api}/progress/{novel_id}/identity",
                method="PUT",
                token=compatibility_token,
                value={
                    "identity_type": "self",
                    "identity_name": self.product_input["player"]["name"],
                    "character_id": None,
                },
            )
            compatibility_branch_before = self.service_metrics("narrative-service")
            compatibility_choice = request_json(
                f"{self.api}/narrative/choose",
                method="POST",
                token=compatibility_token,
                value={
                    "novel_id": novel_id,
                    "node_id": compatibility_node_id,
                    "choice_index": 0,
                },
                timeout=600,
            )
            if provider_started_delta(
                self.root,
                compatibility_branch_before,
                self.service_metrics("narrative-service"),
                service="narrative-service",
                operation="narrative_transition",
            ) != 1:
                raise QualificationFailure("compatibility_branch_provider_delta_invalid")
            if (
                compatibility_choice.get("transition", {}).get("prompt_version")
                != EXPECTED_BRANCH_PROMPT
                or self.db_scalar(
                    "SELECT transition ->> 'prompt_version' FROM user_choices "
                    f"WHERE user_id = '{compatibility_user_id}' AND node_id = '{compatibility_node_id}'"
                )
                != EXPECTED_BRANCH_PROMPT
            ):
                raise QualificationFailure("compatibility_branch_prompt_identity_mismatch")
            actor_ids = [
                actor
                for event in compatibility_choice.get("transition", {}).get("events", [])
                for actor in event.get("actor_character_ids", [])
            ]
            character_by_id = {
                character["id"]: character for character in full_characters
            }
            character_id = next(
                (
                    actor
                    for actor in actor_ids
                    if actor in character_by_id
                    and character_by_id[actor].get("role") == "protagonist"
                ),
                None,
            )
            if character_id is None:
                raise QualificationFailure("compatibility_branch_has_no_witness")
            non_target_character_id = next(
                (candidate for candidate in character_by_id if candidate != character_id),
                None,
            )
            if non_target_character_id is None:
                raise QualificationFailure("compatibility_negative_character_missing")
            self_timeline = json.loads(
                self.authority_snapshot(compatibility_user_id, novel_id)
            )
            for volatile_identity_state in ("progress", "chat_turns", "chat_messages"):
                self_timeline.pop(volatile_identity_state)

            request_no_content(
                f"{self.api}/progress/{novel_id}/identity",
                method="PUT",
                token=compatibility_token,
                value={
                    "identity_type": "character",
                    "identity_name": None,
                    "character_id": non_target_character_id,
                },
            )
            compatibility_progress = request_json(
                f"{self.api}/progress/{novel_id}", token=compatibility_token
            )
            if (
                compatibility_progress.get("reader_identity_type") != "character"
                or compatibility_progress.get("reader_character_id")
                != non_target_character_id
                or compatibility_progress.get("current_chapter") != total_chapters
            ):
                raise QualificationFailure("compatibility_character_identity_invalid")
            compatibility_context = self.internal_character_context(
                compatibility_user_id, novel_id, character_id
            )
            if (
                compatibility_context.get("branch_context") is not None
                or compatibility_context.get("world_context") is not None
            ):
                raise QualificationFailure("compatibility_player_context_visible")
            agent_before = self.service_metrics("agent-service")
            compatibility_chat = self.chat(
                compatibility_token,
                novel_id,
                character_id,
                self.product_input["branch_chat"],
            )
            agent_after = self.service_metrics("agent-service")
            if provider_started_delta(
                self.root,
                agent_before,
                agent_after,
                service="agent-service",
                operation="character_chat",
            ) != 1:
                raise QualificationFailure("compatibility_chat_provider_delta_invalid")
            compatibility_chat_authority = self.chat_authority(
                compatibility_user_id, novel_id
            )
            self.assert_chat_revision(
                compatibility_chat["turn_id"],
                compatibility_context["world_revision"],
            )
            chat_authority = self.db_scalar(
                "SELECT status::text || ':' || reader_identity_type::text || ':' || "
                "COALESCE(reader_character_id::text, '') || ':' || chapter_context::text || ':' || "
                "persona_source_chapter_high_water::text || ':' || "
                f"(SELECT COUNT(*) FROM chat_messages WHERE turn_id = chat_turns.id) FROM chat_turns WHERE id = '{compatibility_chat['turn_id']}'"
            )
            if chat_authority != (
                f"completed:character:{non_target_character_id}:"
                f"{total_chapters}:{total_chapters}:2"
            ):
                raise QualificationFailure("compatibility_chat_authority_invalid")
            if self.db_scalar(
                "SELECT COUNT(*) FROM character_memories "
                f"WHERE user_id = '{compatibility_user_id}' AND novel_id = '{novel_id}'"
            ) != "0":
                raise QualificationFailure("compatibility_memory_projection_created")

            compatibility_authority = self.authority_snapshot(
                compatibility_user_id, novel_id
            )
            narrative_before = self.service_metrics("narrative-service")
            committed_node = request_json(
                f"{self.api}/narrative/{novel_id}/{checkpoint}",
                token=compatibility_token,
            )
            if committed_node != compatibility_node:
                raise QualificationFailure("compatibility_branch_read_not_exact")
            compatibility_replay = request_json(
                f"{self.api}/narrative/choose",
                method="POST",
                token=compatibility_token,
                value={
                    "novel_id": novel_id,
                    "node_id": compatibility_node_id,
                    "choice_index": 0,
                },
            )
            expected_compatibility_replay = json.loads(
                json.dumps(compatibility_choice, ensure_ascii=False)
            )
            expected_world = expected_compatibility_replay.get("world_state")
            expected_state = (
                expected_world.get("state") if isinstance(expected_world, dict) else None
            )
            if not isinstance(expected_state, dict) or not isinstance(
                expected_state.get("choices"), list
            ):
                raise QualificationFailure("compatibility_branch_world_state_invalid")
            expected_world["state"] = {
                "choices": expected_state["choices"],
                "world_events": [],
            }
            if choice_replay_projection(
                compatibility_replay
            ) != choice_replay_projection(expected_compatibility_replay):
                raise QualificationFailure("compatibility_branch_replay_mismatch")

            new_node = request_json(
                f"{self.api}/narrative/{novel_id}/{total_chapters}",
                token=compatibility_token,
                expected=(404,),
            )
            self.require_error(new_node, "not_found")
            different_choice = request_json(
                f"{self.api}/narrative/choose",
                method="POST",
                token=compatibility_token,
                value={
                    "novel_id": novel_id,
                    "node_id": compatibility_node_id,
                    "choice_index": 1,
                },
                expected=(409,),
            )
            self.require_error(different_choice, "choice_conflict")
            player_read = request_json(
                f"{self.api}/narrative/{novel_id}/player-entry",
                token=compatibility_token,
                expected=(409,),
            )
            self.require_error(player_read, "conflict")
            player_write = request_json(
                f"{self.api}/narrative/{novel_id}/player-entry",
                method="PUT",
                token=compatibility_token,
                value={
                    "checkpoint_chapter": checkpoint,
                    "name": self.product_input["player"]["name"],
                    "background": self.product_input["player"]["background"],
                    "capabilities": self.product_input["player"]["capabilities"],
                    "location_id": compatibility_locations[0]["id"],
                    "inventory": self.product_input["player"]["inventory"],
                },
                expected=(409,),
            )
            self.require_error(player_write, "conflict")
            choices_only = request_json(
                f"{self.api}/narrative/{novel_id}/world-state",
                token=compatibility_token,
            ).get("state")
            if (
                not isinstance(choices_only, dict)
                or set(choices_only) != {"choices", "world_events"}
                or not choices_only["choices"]
                or choices_only["world_events"] != []
            ):
                raise QualificationFailure("compatibility_world_state_not_choices_only")
            for method in ("POST", "GET"):
                world_error = request_json(
                    f"{self.api}/narrative/{novel_id}/world",
                    method=method,
                    token=compatibility_token,
                    expected=(409,),
                )
                self.require_error(world_error, "conflict")
            compatibility_turn_id = str(uuid.uuid4())
            world_turn_error, _, _ = self.world_turn(
                compatibility_token,
                novel_id,
                compatibility_turn_id,
                {
                    "expected_turn_number": 0,
                    "kind": "converse",
                    "target_id": character_id,
                    "intent": "角色身份不得取得开放世界权威",
                },
                expected=(409,),
            )
            self.require_error(world_turn_error, "turn_outcome_unknown")
            negative_provider_calls = provider_started_delta(
                self.root,
                narrative_before,
                self.service_metrics("narrative-service"),
                service="narrative-service",
            )
            if (
                negative_provider_calls != 0
                or self.authority_snapshot(compatibility_user_id, novel_id)
                != compatibility_authority
                or self.db_scalar(
                    f"SELECT COUNT(*) FROM world_turns WHERE id = '{compatibility_turn_id}'"
                )
                != "0"
            ):
                raise QualificationFailure("compatibility_negative_boundary_mutated")

            history_before_restart = request_json(
                f"{self.api}/chat/{character_id}/history?limit=20&offset=0",
                token=compatibility_token,
            )
            if history_before_restart.get("count") != 2:
                raise QualificationFailure("compatibility_history_incomplete")
            history_digest = sha256_bytes(
                canonical_json(history_before_restart.get("messages"))
            )
            self.collect_metrics(
                "compatibility-agent-before-restart", ["agent-service"]
            )
            self.collect_response_models(
                "compatibility-agent-before-restart", ["agent-service"]
            )
            self.compose("stop", "--timeout", "120", "agent-service")
            self.compose("up", "-d", "--no-deps", "agent-service")
            self.wait_agent_ready()
            compatibility_replay_before = self.service_metrics("agent-service")
            compatibility_chat_replay = self.chat(
                compatibility_token,
                novel_id,
                character_id,
                self.product_input["branch_chat"],
                turn_id=compatibility_chat["turn_id"],
                expected_replayed=True,
            )
            compatibility_history = request_json(
                f"{self.api}/chat/{character_id}/history?limit=20&offset=0",
                token=compatibility_token,
            )
            if (
                compatibility_chat_replay["response_sha256"]
                != compatibility_chat["response_sha256"]
                or provider_started_delta(
                    self.root,
                    compatibility_replay_before,
                    self.service_metrics("agent-service"),
                    service="agent-service",
                )
                != 0
                or self.chat_authority(compatibility_user_id, novel_id)
                != compatibility_chat_authority
                or compatibility_history.get("count") != 2
                or sha256_bytes(canonical_json(compatibility_history.get("messages")))
                != history_digest
                or self.db_scalar(
                    "SELECT COUNT(*) FROM character_memories "
                    f"WHERE user_id = '{compatibility_user_id}' AND novel_id = '{novel_id}'"
                )
                != "0"
            ):
                raise QualificationFailure("compatibility_restart_replay_mismatch")
            compatibility_required_kinds = {
                "profile",
                "novel",
                "canon_story_model",
                "chapter",
                "character",
                "reading_progress",
                "chat_message",
                "narrative_node",
                "user_choice",
                "world_state",
                "player_chapter",
            }
            compatibility_export, _ = self.export_user(
                token=compatibility_token,
                password=password,
                artifact="character-compatibility-export.ndjson",
                required_kinds=compatibility_required_kinds,
            )
            if not any(
                record.get("kind") == "reading_progress"
                and isinstance(record.get("data"), dict)
                and record.get("data", {}).get("reader_identity_type") == "character"
                and record.get("data", {}).get("reader_character_id")
                == non_target_character_id
                for record in compatibility_export
            ):
                raise QualificationFailure("compatibility_identity_not_exported")
            request_no_content(
                f"{self.api}/progress/{novel_id}/identity",
                method="PUT",
                token=compatibility_token,
                value={
                    "identity_type": "self",
                    "identity_name": self.product_input["player"]["name"],
                    "character_id": None,
                },
            )
            self_progress = request_json(
                f"{self.api}/progress/{novel_id}", token=compatibility_token
            )
            self_history = request_json(
                f"{self.api}/chat/{character_id}/history?limit=20&offset=0",
                token=compatibility_token,
            )
            restored_timeline = json.loads(
                self.authority_snapshot(compatibility_user_id, novel_id)
            )
            for volatile_identity_state in ("progress", "chat_turns", "chat_messages"):
                restored_timeline.pop(volatile_identity_state)
            if (
                self_progress.get("reader_identity_type") != "self"
                or self_progress.get("reader_character_id") is not None
                or self_progress.get("current_chapter") != total_chapters
                or self_history.get("count") != 0
                or restored_timeline != self_timeline
            ):
                raise QualificationFailure("compatibility_self_state_not_restored")
            compatibility_erasure = self.delete_user(
                token=compatibility_token,
                email=compatibility_email,
                password=password,
                user_id=compatibility_user_id,
                novel_id=novel_id,
                character_id=character_id,
            )
            self.report["journey"].update(
                {
                    "legacy_character_compatibility_cases": 1,
                    "legacy_character_chat_turns": 1,
                    "legacy_character_chat_replays": 1,
                    "legacy_character_chat_replay_provider_calls": 0,
                    "legacy_character_chat_replay_authority_delta": 0,
                    "legacy_character_negative_cases": 9,
                    "legacy_character_negative_provider_calls": negative_provider_calls,
                    "legacy_character_negative_authority_delta": 0,
                    "legacy_character_restart_messages": compatibility_history["count"],
                    "legacy_character_export_records": len(compatibility_export),
                    "legacy_character_erasure_private_rows": 0,
                    "legacy_character_deleted_token_negative_cases": compatibility_erasure[
                        "negative_cases"
                    ],
                    "legacy_character_deleted_token_provider_calls": compatibility_erasure[
                        "provider_calls"
                    ],
                    "legacy_character_branch_replay_exact": True,
                    "legacy_character_self_state_preserved": True,
                }
            )

    def verify_prompt_identity(self, novel_id: str, *, include_world: bool) -> None:
        prompt_checks = {
            "canon": (
                EXPECTED_CANON_PROMPT,
                "SELECT prompt_version AS version FROM canon_story_models "
                f"WHERE novel_id = '{novel_id}'",
            ),
            "branch": (
                EXPECTED_BRANCH_PROMPT,
                "SELECT transition ->> 'prompt_version' AS version FROM user_choices "
                f"WHERE novel_id = '{novel_id}'",
            ),
        }
        if include_world:
            prompt_checks["world"] = (
                EXPECTED_WORLD_PROMPT,
                "SELECT transition ->> 'prompt_version' AS version FROM world_turns "
                f"WHERE novel_id = '{novel_id}' AND status = 'completed'",
            )
        prompt_identity = {}
        for name, (expected, query) in prompt_checks.items():
            observed = self.db_scalar(
                "SELECT CASE WHEN COUNT(*) > 0 AND COUNT(*) FILTER "
                f"(WHERE version IS DISTINCT FROM '{expected}') = 0 "
                f"THEN 'ok' ELSE 'invalid' END FROM ({query}) versions"
            )
            if observed != "ok":
                raise QualificationFailure("prompt_identity_mismatch")
            prompt_identity[name] = expected
        self.report["journey"]["prompt_identity"] = prompt_identity

    def execute(self) -> None:
        self.preflight()
        self.prepare_compose()
        write_private(
            self.output / "docker-inventory-before.json",
            canonical_json(self.user_stack_before) + b"\n",
        )

        release_label = "candidate" if self.journey_slice == "legacy-character" else "base"
        release_manifest = (
            self.candidate_manifest_path
            if self.journey_slice == "legacy-character"
            else self.base_manifest_path
        )
        release_identity = (
            self.candidate_manifest
            if self.journey_slice == "legacy-character"
            else self.base_manifest
        )
        with self.stage(f"supported_{release_label}_adoption"):
            duration = self.release(
                "adopt", release_manifest, release_name=release_label
            )
            self.stack_started = True
            self.wait_gateway()
            self.verify_release_images(release_label, release_identity)
            release_postgres_volume = self.postgres_volume_identity()
            self.report["journey"][f"{release_label}_adoption_duration_ms"] = duration
            if self.journey_slice == "core":
                base_postgres_volume = release_postgres_volume

        admin_email = f"admin-{secrets.token_hex(8)}@qualification.invalid"
        reader_email = f"reader-{secrets.token_hex(8)}@qualification.invalid"
        password = f"Q!{secrets.token_urlsafe(24)}aA1"
        with self.stage("operator_setup_and_reader_registration"):
            setup = request_json(
                f"{self.api}/setup/init",
                method="POST",
                value={"email": admin_email, "password": password, "name": "Qualification Admin"},
                expected=(201,),
            )
            admin_token = setup["access_token"]
            settings = request_json(
                f"{self.api}/settings/llm",
                method="PUT",
                token=admin_token,
                value={
                    "provider": self.config["provider"],
                    "model": self.config["model"],
                    "thinking_enabled": self.config["thinking_enabled"],
                    "api_key": self.config["api_key"],
                },
                timeout=600,
            )
            if settings != {
                "provider": EXPECTED_PROVIDER,
                "model": EXPECTED_MODEL,
                "thinking_enabled": True,
                "api_key_configured": True,
                "scope": "platform",
            }:
                raise QualificationFailure("settings_identity_mismatch")
            reader = request_json(
                f"{self.api}/auth/register",
                method="POST",
                value={"email": reader_email, "password": password, "name": "Qualification Reader"},
                expected=(201,),
            )
            token = reader["access_token"]
            user_id = reader["user"]["id"]
            if reader["user"].get("role") != "user":
                raise QualificationFailure("reader_is_not_ordinary_user")

        source = product_source(self.product_input)
        self.report["journey"]["source"] = {
            "case_id": self.product_input["case_id"],
            "manifest_sha256": sha256_bytes(self.product_input_path.read_bytes()),
        }

        with self.stage("live_import"):
            upload_body, content_type = multipart(
                {
                    "title": self.product_input["novel_title"],
                    "author": self.product_input["author"],
                    "deviation_mode": self.product_input["deviation_mode"],
                },
                "qualification.txt",
                source,
            )
            payload, _, _ = request_bytes(
                f"{self.api}/novels/upload",
                method="POST",
                token=token,
                body=upload_body,
                headers={"Content-Type": content_type},
                expected=(202,),
                timeout=120,
            )
            novel_id = json.loads(payload)["novel_id"]
            status = None
            for _ in range(600):
                status = request_json(f"{self.api}/novels/{novel_id}/status", token=token)
                state = status.get("status")
                if state == "ready":
                    break
                if state == "error":
                    raise QualificationFailure("live_import_terminal_error")
                time.sleep(2)
            if status is None or status.get("status") != "ready":
                raise QualificationFailure("live_import_timeout")
            chapters = request_json(f"{self.api}/novels/{novel_id}/chapters", token=token)
            total_chapters = len(chapters)
            if total_chapters != 4 or any(not chapter.get("content") for chapter in chapters):
                raise QualificationFailure("import_not_source_proven")
            partial_characters = request_json(f"{self.api}/novels/{novel_id}/characters", token=token)
            partial_keys = {"id", "novel_id", "name", "first_appearance_chapter"}
            if not partial_characters or any(set(character) != partial_keys for character in partial_characters):
                raise QualificationFailure("partial_persona_leaked")
            self.report["journey"].update(
                {
                    "import_status": "ready",
                    "chapters": total_chapters,
                    "partial_persona_bounded": True,
                }
            )

        if self.journey_slice == "legacy-character":
            checkpoint_value = self.db_scalar(
                "SELECT MIN(chapter_number) FROM chapters "
                f"WHERE novel_id = '{novel_id}' AND is_key_node"
            )
            if not checkpoint_value.isdigit():
                raise QualificationFailure("canonical_key_node_missing")
            checkpoint = int(checkpoint_value)
            if checkpoint < 1 or checkpoint >= total_chapters:
                raise QualificationFailure("branch_checkpoint_unusable")
            self.run_legacy_character_slice(
                password=password,
                novel_id=novel_id,
                total_chapters=total_chapters,
                checkpoint=checkpoint,
            )
            self.finalize_observability("candidate-final")
            with self.stage("prompt_and_schema_identity"):
                self.verify_prompt_identity(novel_id, include_world=False)
            self.report["outcome"] = "completed"
            return

        with self.stage("branch_and_player_entry"):
            checkpoint_value = self.db_scalar(
                "SELECT MIN(chapter_number) FROM chapters "
                f"WHERE novel_id = '{novel_id}' AND is_key_node"
            )
            if not checkpoint_value.isdigit():
                raise QualificationFailure("canonical_key_node_missing")
            checkpoint = int(checkpoint_value)
            if checkpoint < 1 or checkpoint >= total_chapters:
                raise QualificationFailure("branch_checkpoint_unusable")
            request_no_content(
                f"{self.api}/progress/{novel_id}/identity",
                method="PUT",
                token=token,
                value={
                    "identity_type": "self",
                    "identity_name": self.product_input["player"]["name"],
                    "character_id": None,
                },
            )
            request_no_content(
                f"{self.api}/progress/{novel_id}",
                method="PUT",
                token=token,
                value={"current_chapter": checkpoint + 1},
            )
            entry = request_json(
                f"{self.api}/narrative/{novel_id}/player-entry?{urllib.parse.urlencode({'checkpoint_chapter': checkpoint})}",
                token=token,
            )
            locations = entry.get("locations")
            if (
                not isinstance(locations, list)
                or not locations
                or not isinstance(locations[0], dict)
                or not isinstance(locations[0].get("id"), str)
            ):
                raise QualificationFailure("player_entry_has_no_location")
            location_id = locations[0]["id"]
            player = request_json(
                f"{self.api}/narrative/{novel_id}/player-entry",
                method="PUT",
                token=token,
                value={
                    "checkpoint_chapter": checkpoint,
                    "name": self.product_input["player"]["name"],
                    "background": self.product_input["player"]["background"],
                    "capabilities": self.product_input["player"]["capabilities"],
                    "location_id": location_id,
                    "inventory": self.product_input["player"]["inventory"],
                },
            )
            if not player.get("player", {}).get("id"):
                raise QualificationFailure("player_entry_not_committed")
            node = request_json(f"{self.api}/narrative/{novel_id}/{checkpoint}", token=token)
            node_id = node.get("id")
            if not isinstance(node_id, str):
                raise QualificationFailure("canonical_branch_node_missing")
            branch_provider_before = self.service_metrics("narrative-service")
            choice = request_json(
                f"{self.api}/narrative/choose",
                method="POST",
                token=token,
                value={"novel_id": novel_id, "node_id": node_id, "choice_index": 0},
                timeout=600,
            )
            if provider_started_delta(
                self.root,
                branch_provider_before,
                self.service_metrics("narrative-service"),
                service="narrative-service",
                operation="narrative_transition",
            ) != 1:
                raise QualificationFailure("branch_provider_delta_invalid")
            transition = choice.get("transition", {})
            actor_ids = [
                actor
                for event in transition.get("events", [])
                for actor in event.get("actor_character_ids", [])
            ]
            if not actor_ids:
                raise QualificationFailure("branch_has_no_character_witness")
            branch_authority = self.authority_snapshot(user_id, novel_id)
            branch_replay_before = self.service_metrics("narrative-service")
            branch_replay = request_json(
                f"{self.api}/narrative/choose",
                method="POST",
                token=token,
                value={"novel_id": novel_id, "node_id": node_id, "choice_index": 0},
            )
            if (
                choice_replay_projection(branch_replay)
                != choice_replay_projection(choice)
                or provider_started_delta(
                    self.root,
                    branch_replay_before,
                    self.service_metrics("narrative-service"),
                    service="narrative-service",
                )
                != 0
                or self.authority_snapshot(user_id, novel_id) != branch_authority
            ):
                raise QualificationFailure("branch_replay_mismatch")
            self.report["journey"].update(
                {
                    "branch_committed": True,
                    "branch_replayed": True,
                    "branch_checkpoint": checkpoint,
                    "branch_replay_provider_calls": 0,
                    "branch_replay_authority_delta": 0,
                }
            )

        with self.stage("full_persona_and_branch_chat"):
            request_no_content(
                f"{self.api}/progress/{novel_id}",
                method="PUT",
                token=token,
                value={"current_chapter": total_chapters},
            )
            full_characters = request_json(f"{self.api}/novels/{novel_id}/characters", token=token)
            character_by_id = {character["id"]: character for character in full_characters}
            character_id = next(
                (
                    actor
                    for actor in actor_ids
                    if actor in character_by_id
                    and character_by_id[actor].get("role") == "protagonist"
                ),
                None,
            )
            if character_id is None:
                raise QualificationFailure("branch_has_no_protagonist_witness")
            non_target_character_id = next(
                (candidate for candidate in character_by_id if candidate != character_id),
                None,
            )
            if non_target_character_id is None:
                raise QualificationFailure("visibility_negative_character_missing")
            if any(
                character.get("persona_source_chapter_high_water") != total_chapters
                or "system_prompt" in character
                for character in full_characters
            ):
                raise QualificationFailure("full_persona_provenance_invalid")
            context = self.internal_character_context(user_id, novel_id, character_id)
            branch_context = context.get("branch_context")
            if (
                not branch_context
                or not branch_context.get("events")
                or not any(
                    character_id in event.get("actor_character_ids", [])
                    for event in branch_context["events"]
                )
            ):
                raise QualificationFailure("branch_context_not_character_visible")
            branch_chat_before = self.service_metrics("agent-service")
            branch_chat = self.chat(
                token,
                novel_id,
                character_id,
                self.product_input["branch_chat"],
            )
            branch_chat_after = self.service_metrics("agent-service")
            if provider_started_delta(
                self.root,
                branch_chat_before,
                branch_chat_after,
                service="agent-service",
                operation="character_chat",
            ) != 1:
                raise QualificationFailure("branch_chat_provider_delta_invalid")
            self.assert_chat_revision(branch_chat["turn_id"], context["world_revision"])
            branch_chat_authority = self.chat_authority(user_id, novel_id)
            replay_before = self.service_metrics("agent-service")
            branch_chat_replay = self.chat(
                token,
                novel_id,
                character_id,
                self.product_input["branch_chat"],
                turn_id=branch_chat["turn_id"],
                expected_replayed=True,
            )
            if (
                branch_chat_replay["response_sha256"]
                != branch_chat["response_sha256"]
                or provider_started_delta(
                    self.root,
                    replay_before,
                    self.service_metrics("agent-service"),
                    service="agent-service",
                )
                != 0
                or self.chat_authority(user_id, novel_id) != branch_chat_authority
            ):
                raise QualificationFailure("branch_chat_replay_mismatch")
            self.report["journey"].update(
                {
                    "full_persona_source_high_water": total_chapters,
                    "branch_context_visible_to_witness": True,
                    "branch_chat_world_revision_exact": True,
                    "branch_chat_replay_exact": True,
                    "branch_chat_replay_provider_calls": 0,
                    "branch_chat_replay_authority_delta": 0,
                }
            )

        with self.stage("base_world_trajectory"):
            world = request_json(
                f"{self.api}/narrative/{novel_id}/world",
                method="POST",
                token=token,
                value=None,
                timeout=600,
            )
            if world.get("session", {}).get("turn_number") != 0:
                raise QualificationFailure("world_not_fresh")
            chat_turns = 1

            def action_for(number: int) -> dict[str, Any]:
                return {
                    "expected_turn_number": number - 1,
                    "kind": "converse",
                    "target_id": character_id,
                    "intent": self.product_input["world_actions"][number - 1],
                }

            def submit_committed(
                number: int, turn_id: str | None = None
            ) -> tuple[str, dict[str, Any], Any]:
                turn_id = turn_id or str(uuid.uuid4())
                action = action_for(number)
                before = self.service_metrics("narrative-service")
                result, _, _ = self.world_turn(
                    token, novel_id, turn_id, action, timeout=600
                )
                after = self.service_metrics("narrative-service")
                if provider_started_delta(
                    self.root,
                    before,
                    after,
                    service="narrative-service",
                    operation="narrative_transition",
                ) != 1:
                    raise QualificationFailure("world_turn_provider_delta_invalid")
                if (
                    result.get("turn_id") != turn_id
                    or result.get("memory_projection_status") != "saved"
                    or result.get("world_state", {})
                    .get("state", {})
                    .get("open_world", {})
                    .get("turn_number")
                    != number
                ):
                    raise QualificationFailure("world_turn_not_committed")
                return turn_id, action, result

            for number in range(1, 7):
                turn_id, action, result = submit_committed(number)
                if number == 1:
                    replay_authority = self.authority_snapshot(user_id, novel_id)
                    before = self.service_metrics("narrative-service")
                    replay, _, _ = self.world_turn(
                        token, novel_id, turn_id, action
                    )
                    after = self.service_metrics("narrative-service")
                    if (
                        replay != result
                        or provider_started_delta(
                            self.root,
                            before,
                            after,
                            service="narrative-service",
                        )
                        != 0
                        or self.authority_snapshot(user_id, novel_id)
                        != replay_authority
                    ):
                        raise QualificationFailure("world_turn_replay_mismatch")
                chat_result = self.chat(
                    token,
                    novel_id,
                    character_id,
                    self.product_input["world_chats"][number - 1],
                )
                chat_turns += 1
                current = self.internal_character_context(
                    user_id, novel_id, character_id
                )
                self.assert_chat_revision(
                    chat_result["turn_id"], current["world_revision"]
                )

        self.collect_metrics("base")
        self.collect_response_models("base")
        pre_upgrade_authority = self.authority_snapshot(user_id, novel_id)
        self.private_report["pre_upgrade_authority_sha256"] = sha256_bytes(
            pre_upgrade_authority.encode("utf-8")
        )
        turn_seven_id = str(uuid.uuid4())
        turn_seven_action = action_for(7)

        with self.stage("supported_candidate_upgrade"):
            def fail_stopped_client_gate() -> None:
                request_no_content(
                    f"{self.api}/progress/{novel_id}",
                    method="PUT",
                    token=token,
                    value={"current_chapter": total_chapters},
                )
                body = json.dumps(
                    turn_seven_action,
                    ensure_ascii=False,
                    separators=(",", ":"),
                ).encode("utf-8")
                _, _, status = request_bytes(
                    f"{self.api}/narrative/{novel_id}/world/turns",
                    method="POST",
                    token=token,
                    body=body,
                    headers={
                        "Content-Type": "application/json",
                        "Idempotency-Key": turn_seven_id,
                    },
                    expected=tuple(range(500, 600)),
                )
                if status < 500 or self.authority_snapshot(
                    user_id, novel_id
                ) != pre_upgrade_authority:
                    raise QualificationFailure("release_client_gate_failed")

            duration = self.release(
                "upgrade", self.candidate_manifest_path, fail_stopped_client_gate
            )
            self.wait_gateway()
            self.verify_release_images("candidate", self.candidate_manifest)
            candidate_postgres_volume = self.postgres_volume_identity()
            if candidate_postgres_volume != base_postgres_volume:
                raise QualificationFailure("postgres_authority_volume_changed")
            self.private_report["postgres_authority_volume"] = {
                "base": base_postgres_volume,
                "candidate": candidate_postgres_volume,
            }
            post_upgrade_authority = self.authority_snapshot(user_id, novel_id)
            if post_upgrade_authority != pre_upgrade_authority:
                raise QualificationFailure("upgrade_authority_changed")
            self.private_report["post_upgrade_authority_sha256"] = sha256_bytes(
                post_upgrade_authority.encode("utf-8")
            )
            self.report["journey"].update(
                {
                    "candidate_upgrade_duration_ms": duration,
                    "upgrade_authority_unchanged": True,
                    "upgrade_same_authoritative_volumes": True,
                    "client_gate_fail_stopped": True,
                }
            )

        with self.stage("candidate_world_and_negative_matrix"):
            submit_committed(7, turn_seven_id)
            chat_result = self.chat(
                token,
                novel_id,
                character_id,
                self.product_input["world_chats"][6],
            )
            chat_turns += 1
            current = self.internal_character_context(
                user_id, novel_id, character_id
            )
            self.assert_chat_revision(
                chat_result["turn_id"], current["world_revision"]
            )

            authority_before = self.authority_snapshot(
                user_id, novel_id, include_failed_turns=False
            )
            narrative_provider_before = self.service_metrics("narrative-service")
            novel_provider_before = self.service_metrics("novel-service")
            unknown_id = str(uuid.uuid4())
            unknown, _, _ = self.world_turn(
                token,
                novel_id,
                unknown_id,
                {
                    "expected_turn_number": 7,
                    "kind": "converse",
                    "target_id": str(uuid.uuid4()),
                    "intent": "核对未知目标",
                },
                expected=(422,),
            )
            self.require_error(unknown, "validation_error")
            negative_turn_ids = [unknown_id]
            for expected_turn in (6, 8):
                conflict_id = str(uuid.uuid4())
                negative_turn_ids.append(conflict_id)
                conflict, _, _ = self.world_turn(
                    token,
                    novel_id,
                    conflict_id,
                    {
                        "expected_turn_number": expected_turn,
                        "kind": "converse",
                        "target_id": character_id,
                        "intent": "不应越过服务器回合顺序",
                    },
                    expected=(409,),
                )
                self.require_error(conflict, "conflict")
            invalid_id = str(uuid.uuid4())
            negative_turn_ids.append(invalid_id)
            invalid, _, _ = self.world_turn(
                token,
                novel_id,
                invalid_id,
                {
                    "expected_turn_number": 7,
                    "kind": "converse",
                    "target_id": character_id,
                    "intent": "",
                },
                expected=(422,),
            )
            self.require_error(invalid, "validation_error")
            unsupported_id = str(uuid.uuid4())
            negative_turn_ids.append(unsupported_id)
            unsupported_body = json.dumps(
                {
                    **action_for(8),
                    "expected_turn_number": 7,
                    "item_id": "unsupported",
                },
                ensure_ascii=False,
                separators=(",", ":"),
            ).encode("utf-8")
            unsupported, _, _ = request_bytes(
                f"{self.api}/narrative/{novel_id}/world/turns",
                method="POST",
                token=token,
                body=unsupported_body,
                headers={
                    "Content-Type": "application/json",
                    "Idempotency-Key": unsupported_id,
                },
                expected=(422,),
            )
            try:
                self.require_error(json.loads(unsupported), "validation_error")
            except (json.JSONDecodeError, UnicodeDecodeError) as error:
                raise QualificationFailure("unsupported_field_error_invalid") from error
            future, _, _ = request_bytes(
                f"{self.api}/progress/{novel_id}",
                method="PUT",
                token=token,
                body=json.dumps(
                    {"current_chapter": total_chapters + 1}
                ).encode("utf-8"),
                headers={"Content-Type": "application/json"},
                expected=(422,),
            )
            try:
                self.require_error(json.loads(future), "validation_error")
            except (json.JSONDecodeError, UnicodeDecodeError) as error:
                raise QualificationFailure("future_progress_error_invalid") from error
            progress = request_json(
                f"{self.api}/progress/{novel_id}", token=token
            )
            negative_provider_calls = provider_started_delta(
                self.root,
                narrative_provider_before,
                self.service_metrics("narrative-service"),
                service="narrative-service",
            ) + provider_started_delta(
                self.root,
                novel_provider_before,
                self.service_metrics("novel-service"),
                service="novel-service",
            )
            ids = ",".join(f"'{turn_id}'" for turn_id in negative_turn_ids)
            if self.db_scalar(
                "SELECT COUNT(*) FROM world_turns WHERE id IN ("
                f"{ids}) AND (status <> 'failed' OR transition IS NOT NULL "
                "OR resolution IS NOT NULL OR result IS NOT NULL)"
            ) != "0":
                raise QualificationFailure("negative_matrix_committed_audit")
            for failed_id in (unknown_id, invalid_id):
                if self.db_scalar(
                    "SELECT status || ':' || failure_code FROM world_turns "
                    f"WHERE id = '{failed_id}'"
                ) != "failed:validation_error":
                    raise QualificationFailure("negative_matrix_audit_missing")
            if (
                progress.get("current_chapter") != total_chapters
                or self.authority_snapshot(
                    user_id, novel_id, include_failed_turns=False
                )
                != authority_before
                or negative_provider_calls != 0
            ):
                raise QualificationFailure("negative_matrix_authority_changed")
            self.report["journey"].update(
                {
                    "hostile_input_authorized_once": True,
                    "typed_negative_cases": 6,
                    "negative_provider_calls": negative_provider_calls,
                    "negative_authority_delta": 0,
                }
            )

            mid_count = 0
            for number in range(8, 12):
                submit_committed(number)
                chat_result = self.chat(
                    token,
                    novel_id,
                    character_id,
                    self.product_input["world_chats"][number - 1],
                )
                chat_turns += 1
                current = self.internal_character_context(
                    user_id, novel_id, character_id
                )
                self.assert_chat_revision(
                    chat_result["turn_id"], current["world_revision"]
                )
                if number == 9:
                    for _ in range(180):
                        mid_count = int(
                            self.db_scalar(
                                "SELECT COUNT(*) FROM character_memories "
                                f"WHERE user_id = '{user_id}' "
                                f"AND novel_id = '{novel_id}' "
                                f"AND character_id = '{character_id}' "
                                "AND layer = 'mid'"
                            )
                        )
                        if mid_count >= 1:
                            break
                        time.sleep(2)
                    else:
                        raise QualificationFailure(
                            "mid_memory_window_not_projected"
                        )

        self.collect_metrics(
            "candidate-agent-before-restart", ["agent-service"]
        )
        self.collect_response_models(
            "candidate-agent-before-restart", ["agent-service"]
        )

        with self.stage("pending_projection_recovery"):
            permanent_before = int(
                self.db_scalar(
                    "SELECT COUNT(*) FROM character_memories "
                    f"WHERE user_id = '{user_id}' AND novel_id = '{novel_id}' "
                    f"AND character_id = '{character_id}' "
                    "AND layer = 'permanent' "
                    "AND content::jsonb ->> 'source' = 'committed_world_turn'"
                )
            )
            pending_turn_id = str(uuid.uuid4())
            pending_action = action_for(12)
            narrative_before = self.service_metrics("narrative-service")
            self.compose("stop", "--timeout", "120", "agent-service")
            pending, _, _ = self.world_turn(
                token,
                novel_id,
                pending_turn_id,
                pending_action,
                expected=(409,),
                timeout=600,
            )
            self.require_error(pending, "turn_outcome_unknown")
            narrative_after_commit = self.service_metrics("narrative-service")
            if provider_started_delta(
                self.root,
                narrative_before,
                narrative_after_commit,
                service="narrative-service",
                operation="narrative_transition",
            ) != 1:
                raise QualificationFailure("pending_turn_provider_delta_invalid")
            row = self.db_scalar(
                "SELECT status || ':' || memory_projection_status "
                f"FROM world_turns WHERE id = '{pending_turn_id}'"
            )
            if row != "completed:pending":
                raise QualificationFailure("pending_turn_not_durable")
            if int(
                self.db_scalar(
                    "SELECT COUNT(*) FROM character_memories "
                    f"WHERE user_id = '{user_id}' AND novel_id = '{novel_id}' "
                    f"AND character_id = '{character_id}' "
                    "AND layer = 'permanent' "
                    "AND content::jsonb ->> 'source' = 'committed_world_turn'"
                )
            ) != permanent_before:
                raise QualificationFailure("pending_projection_wrote_memory")

            overtaking_id = str(uuid.uuid4())
            overtaking, overtaking_headers, _ = self.world_turn(
                token,
                novel_id,
                overtaking_id,
                pending_action,
                expected=(409,),
            )
            self.require_error(overtaking, "turn_in_progress")
            if not overtaking_headers.get("Retry-After"):
                raise QualificationFailure("pending_barrier_retry_after_missing")
            if (
                self.db_scalar(
                    f"SELECT COUNT(*) FROM world_turns WHERE id = '{overtaking_id}'"
                )
                != "0"
                or provider_started_delta(
                    self.root,
                    narrative_after_commit,
                    self.service_metrics("narrative-service"),
                    service="narrative-service",
                    operation="narrative_transition",
                )
                != 0
            ):
                raise QualificationFailure("pending_barrier_overtaken")

            self.compose("up", "-d", "--no-deps", "agent-service")
            ready_at = self.wait_agent_ready()
            deadline = ready_at + 90
            projection_status = "pending"
            while time.monotonic() <= deadline:
                projection_status = self.db_scalar(
                    "SELECT memory_projection_status FROM world_turns "
                    f"WHERE id = '{pending_turn_id}'"
                )
                if projection_status != "pending":
                    break
                time.sleep(1)
            recovery_ms = round((time.monotonic() - ready_at) * 1000)
            if projection_status != "saved" or recovery_ms > 90_000:
                raise QualificationFailure("pending_projection_recovery_timeout")

            replay_before = self.service_metrics("narrative-service")
            replay, _, _ = self.world_turn(
                token, novel_id, pending_turn_id, pending_action
            )
            replay_after = self.service_metrics("narrative-service")
            stored = json.loads(
                self.db_scalar(
                    f"SELECT result::text FROM world_turns WHERE id = '{pending_turn_id}'"
                )
            )
            replay_result = dict(replay)
            replay_result.pop("memory_projection_status", None)
            permanent_after = int(
                self.db_scalar(
                    "SELECT COUNT(*) FROM character_memories "
                    f"WHERE user_id = '{user_id}' AND novel_id = '{novel_id}' "
                    f"AND character_id = '{character_id}' "
                    "AND layer = 'permanent' "
                    "AND content::jsonb ->> 'source' = 'committed_world_turn'"
                )
            )
            if (
                replay.get("memory_projection_status") != "saved"
                or replay_result != stored
                or provider_started_delta(
                    self.root,
                    replay_before,
                    replay_after,
                    service="narrative-service",
                )
                != 0
                or permanent_after != permanent_before + 1
                or self.db_scalar(
                    "SELECT COUNT(*) FROM world_turns "
                    f"WHERE user_id = '{user_id}' AND novel_id = '{novel_id}' "
                    "AND status = 'completed'"
                )
                != "12"
            ):
                raise QualificationFailure("pending_replay_not_exact")
            self.report["journey"].update(
                {
                    "pending_projection_observed": True,
                    "pending_overtake_blocked": True,
                    "projection_recovery_ms": recovery_ms,
                    "projection_recovered_by_scanner": True,
                    "pending_replay_provider_calls": 0,
                    "pending_world_commits": 1,
                }
            )

        with self.stage("post_restart_mid_continuity"):
            resumed_context = self.internal_character_context(
                user_id, novel_id, character_id
            )
            trace_id = str(uuid.uuid4())
            agent_before = self.service_metrics("agent-service")
            resumed_chat = self.chat(
                token,
                novel_id,
                character_id,
                self.product_input["post_restart_chat"],
                trace_id=trace_id,
            )
            agent_after = self.service_metrics("agent-service")
            if provider_started_delta(
                self.root,
                agent_before,
                agent_after,
                service="agent-service",
                operation="character_chat",
            ) != 1:
                raise QualificationFailure("post_restart_chat_provider_delta_invalid")
            marker_count = selected_mid_from_logs(
                run(["docker", "logs", f"{self.prefix}-agent-service"]),
                trace_id,
            )
            self.assert_chat_revision(
                resumed_chat["turn_id"], resumed_context["world_revision"]
            )
            history = request_json(
                f"{self.api}/chat/{character_id}/history?limit=100&offset=0",
                token=token,
            )
            if history.get("count") != 26:
                raise QualificationFailure("restart_chat_history_incomplete")
            self.report["journey"].update(
                {
                    "pre_restart_chat_turns": chat_turns,
                    "post_restart_chat_turns": 1,
                    "total_chat_turns": 13,
                    "mid_memory_windows": mid_count,
                    "mid_candidates_selected": marker_count,
                    "mid_selection_trace_correlated": True,
                    "restart_chat_revision_exact": True,
                    "durable_chat_messages": history["count"],
                }
            )

        with self.stage("final_world_visibility"):
            world_view = request_json(
                f"{self.api}/narrative/{novel_id}/world", token=token
            )
            if (
                world_view.get("session", {}).get("turn_number") != 12
                or len(world_view.get("journal", [])) != 12
            ):
                raise QualificationFailure("world_trajectory_incomplete")
            final_context = self.internal_character_context(
                user_id, novel_id, character_id
            )
            recent_actions = (
                final_context.get("world_context") or {}
            ).get("recent_actions", [])
            if (
                not 1 <= len(recent_actions) <= 4
                or recent_actions[-1].get("turn_number") != 12
                or any(
                    action.get("action", {}).get("target_id") != character_id
                    for action in recent_actions
                )
            ):
                raise QualificationFailure("character_visibility_window_invalid")
            actor_events = json.loads(
                self.db_scalar(
                    "SELECT COALESCE(jsonb_agg(event), '[]')::text "
                    "FROM world_states state, "
                    "LATERAL jsonb_array_elements(state.state -> 'world_events') event "
                    f"WHERE state.user_id = '{user_id}' AND state.novel_id = '{novel_id}'"
                )
            )
            def require_explicit_events(context: dict[str, Any], observed_id: str) -> None:
                events = (context.get("world_context") or {}).get(
                    "recent_player_events", []
                )
                authorized = {
                    (
                        event.get("id"),
                        event.get("turn_id"),
                        event.get("turn_number"),
                        event.get("world_time"),
                        event.get("summary"),
                        event.get("location_id"),
                    )
                    for event in actor_events
                    if isinstance(event, dict)
                    and observed_id in event.get("actor_character_ids", [])
                }
                if len(events) > 4 or any(
                    not isinstance(event, dict)
                    or observed_id not in event.get("actor_character_ids", [])
                    or (
                        event.get("id"),
                        event.get("turn_id"),
                        event.get("turn_number"),
                        event.get("world_time"),
                        event.get("summary"),
                        event.get("location_id"),
                    )
                    not in authorized
                    for event in events
                ):
                    raise QualificationFailure("unwitnessed_event_visible")

            require_explicit_events(final_context, character_id)
            non_target_context = self.internal_character_context(
                user_id, novel_id, non_target_character_id
            )
            non_target_actions = (
                non_target_context.get("world_context") or {}
            ).get("recent_actions", [])
            if non_target_actions:
                raise QualificationFailure("unwitnessed_action_visible")
            require_explicit_events(non_target_context, non_target_character_id)
            self.report["journey"].update(
                {
                    "world_turns": 12,
                    "world_turn_replay_exact": True,
                    "world_turn_replay_authority_delta": 0,
                    "character_recent_action_window": len(recent_actions),
                    "unwitnessed_direct_action_window": 0,
                    "unwitnessed_event_window": 0,
                }
            )

        self.finalize_observability("candidate-final")
        with self.stage("prompt_and_schema_identity"):
            self.verify_prompt_identity(novel_id, include_world=True)

        with self.stage("export_and_account_erasure"):
            required_kinds = {
                "profile",
                "novel",
                "canon_story_model",
                "chapter",
                "character",
                "reading_progress",
                "chat_message",
                "character_memory",
                "narrative_node",
                "user_choice",
                "world_state",
                "player_chapter",
                "world_turn",
            }
            records, _ = self.export_user(
                token=token,
                password=password,
                artifact="account-export.ndjson",
                required_kinds=required_kinds,
            )
            erasure = self.delete_user(
                token=token,
                email=reader_email,
                password=password,
                user_id=user_id,
                novel_id=novel_id,
                character_id=character_id,
            )
            self.report["journey"].update(
                {
                    "account_export_complete": True,
                    "export_record_kinds": sorted(required_kinds),
                    "account_export_records": len(records),
                    "account_erasure_private_rows": 0,
                    "deleted_reader_cannot_login": True,
                    "deleted_token_negative_cases": erasure["negative_cases"],
                    "deleted_token_provider_calls": erasure["provider_calls"],
                }
            )

        self.report["outcome"] = "completed"

    def cleanup(self) -> None:
        if not self.inventory_captured:
            if self.runtime_temp is not None:
                self.runtime_temp.cleanup()
                self.runtime_temp = None
            return
        cleanup_ok = True
        if self.cleanup_required:
            if not PROJECT_PATTERN.fullmatch(self.project):
                cleanup_ok = False
            elif self.compose_env and self.runtime_root is not None:
                try:
                    self.compose(
                        "down",
                        "--volumes",
                        "--remove-orphans",
                        capture=False,
                    )
                    self.stack_started = False
                except QualificationFailure:
                    cleanup_ok = False
            else:
                cleanup_ok = not attempt_resources(
                    docker_inventory_snapshot(), self.project, self.prefix
                )
        after = docker_inventory_snapshot()
        residue = attempt_resources(after, self.project, self.prefix)
        cleanup_ok = cleanup_ok and not residue
        after_without_attempt = {
            kind: {
                name: value
                for name, value in after[kind].items()
                if f"{kind}:{name}" not in residue
            }
            for kind in ("containers", "volumes", "networks")
        }
        unchanged = after_without_attempt == self.user_stack_before
        self.private_report.setdefault("environment", {}).update(
            {
                "existing_stack_after": after,
                "attempt_resource_residue": residue,
            }
        )
        self.report["environment"]["isolated_cleanup_completed"] = (
            cleanup_ok
        )
        self.report["environment"]["existing_user_stack_unchanged"] = unchanged
        self.report["environment"]["isolated_resource_residue_count"] = len(residue)
        if not unchanged and self.report["outcome"] == "completed":
            self.report["outcome"] = "failed"
            self.report["failure"] = {"stage": "cleanup", "code": "existing_user_stack_changed"}
        if not cleanup_ok and self.report["outcome"] == "completed":
            self.report["outcome"] = "failed"
            self.report["failure"] = {"stage": "cleanup", "code": "isolated_cleanup_failed"}
        if self.runtime_temp is not None:
            self.runtime_temp.cleanup()
            self.runtime_temp = None

    def public_report(self) -> dict[str, Any]:
        journey = self.report["journey"]
        count_keys = (
            "chapters",
            "typed_negative_cases",
            "negative_provider_calls",
            "negative_authority_delta",
            "pending_replay_provider_calls",
            "pending_world_commits",
            "pre_restart_chat_turns",
            "post_restart_chat_turns",
            "total_chat_turns",
            "mid_memory_windows",
            "mid_candidates_selected",
            "durable_chat_messages",
            "world_turns",
            "world_turn_replay_authority_delta",
            "character_recent_action_window",
            "unwitnessed_direct_action_window",
            "unwitnessed_event_window",
            "legacy_character_compatibility_cases",
            "legacy_character_chat_turns",
            "legacy_character_chat_replays",
            "legacy_character_chat_replay_provider_calls",
            "legacy_character_chat_replay_authority_delta",
            "legacy_character_negative_cases",
            "legacy_character_negative_provider_calls",
            "legacy_character_negative_authority_delta",
            "legacy_character_restart_messages",
            "legacy_character_export_records",
            "legacy_character_erasure_private_rows",
            "legacy_character_deleted_token_negative_cases",
            "legacy_character_deleted_token_provider_calls",
            "account_export_records",
            "account_erasure_private_rows",
            "branch_chat_replay_provider_calls",
            "branch_chat_replay_authority_delta",
            "branch_replay_provider_calls",
            "branch_replay_authority_delta",
            "deleted_token_negative_cases",
            "deleted_token_provider_calls",
            "export_source_canon_records",
            "export_source_uncertain_records",
            "export_source_reader_records",
            "export_source_generated_records",
            "export_source_mixed_records",
        )
        latency_keys = (
            "base_adoption_duration_ms",
            "candidate_adoption_duration_ms",
            "base_pull_duration_ms",
            "base_database_start_duration_ms",
            "base_migration_duration_ms",
            "base_application_deployment_duration_ms",
            "base_readiness_duration_ms",
            "candidate_upgrade_duration_ms",
            "candidate_pull_duration_ms",
            "candidate_migration_duration_ms",
            "candidate_application_deployment_duration_ms",
            "candidate_readiness_duration_ms",
            "projection_recovery_ms",
        )

        provider = {
            "name": EXPECTED_PROVIDER,
            "configured_model": EXPECTED_MODEL,
        }
        successful_calls = self.report["provider"].get("successful_calls")
        if isinstance(successful_calls, int) and not isinstance(successful_calls, bool):
            provider["successful_calls"] = successful_calls
        observed = self.report["provider"].get("observed_response_models")
        if observed is not None:
            if not isinstance(observed, dict) or any(
                not valid_public_model(model)
                or not isinstance(count, int)
                or isinstance(count, bool)
                or count < 0
                for model, count in observed.items()
            ):
                raise QualificationFailure("public_response_model_invalid")
            provider["observed_response_models"] = dict(sorted(observed.items()))

        aggregate: dict[str, Any] = {
            "counts": {
                key: journey[key]
                for key in count_keys
                if isinstance(journey.get(key), (int, float))
                and not isinstance(journey[key], bool)
            },
            "latencies_ms": {
                key: journey[key]
                for key in latency_keys
                if isinstance(journey.get(key), (int, float))
                and not isinstance(journey[key], bool)
            },
            "stage_durations_ms": {
                stage["name"]: stage["duration_ms"]
                for stage in self.report["stages"]
                if re.fullmatch(r"[a-z0-9_]{1,100}", stage.get("name", ""))
                and isinstance(stage.get("duration_ms"), (int, float))
                and not isinstance(stage["duration_ms"], bool)
            },
        }
        if "llm_metrics" in self.report:
            aggregate["llm"] = self.report["llm_metrics"]

        public = {
            "schema_version": 2,
            "report_kind": "h4-journey-qualification-v1",
            "evidence_class": self.evidence_class,
            "journey_slice": self.journey_slice,
            "qualification_claim": False,
            "attempt_id": self.attempt_id,
            "outcome": self.report["outcome"],
            "provider": provider,
            "fixture_manifest_sha256": sha256_bytes(
                self.product_input_path.read_bytes()
            ),
            "application_image_content_digests": self.report["environment"].get(
                "application_image_content_digests", {}
            ),
            "aggregate": aggregate,
        }
        failure = self.report.get("failure")
        if failure is not None:
            code = failure.get("code") if isinstance(failure, dict) else None
            if not isinstance(code, str) or not re.fullmatch(
                r"[a-z0-9_]{1,100}", code
            ):
                code = "unexpected_runner_failure"
            public["failure"] = {"code": code}
        return public

    def write_report(self) -> None:
        self.report["completed_at"] = utc_now()
        self.private_report["completed_at"] = self.report["completed_at"]
        self.private_report["runner_report"] = self.report
        path = self.output / "journey-report.json"
        write_private(
            self.output / "journey-private.json",
            json.dumps(
                self.private_report, ensure_ascii=False, indent=2, sort_keys=True
            ).encode("utf-8")
            + b"\n",
        )
        temporary = self.output / ".journey-report.tmp"
        write_private(
            temporary,
            json.dumps(
                self.public_report(), ensure_ascii=False, indent=2, sort_keys=True
            ).encode("utf-8")
            + b"\n",
        )
        os.replace(temporary, path)
        print(f"live qualification report: {path}")


def self_test(root: Path) -> None:
    sanitized = qualification_environment(
        root,
        {
            "PATH": "tool-path",
            "DOCKER_HOST": "engine",
            "LLM_API_KEY": "host-secret",
            "IMAGE_GEN_API_URL": "https://unregistered.invalid",
            "S3_ENABLED": "true",
            "POSTGRES_PASSWORD": "host-secret",
        },
    )
    assert sanitized == {"PATH": "tool-path", "DOCKER_HOST": "engine"}
    assert expected_export_source(
        "canon_story_model", {"content": {"facts": [{"confidence": 1.0}]}}
    ) == "canon"
    assert expected_export_source(
        "canon_story_model", {"content": {"facts": [{"confidence": 0.9}]}}
    ) == "uncertain"
    assert expected_export_source("narrative_node", {"user_id": "reader"}) == "generated"
    assert expected_export_source("user_choice", {}) == "reader"
    assert expected_export_source("world_state", {"state": {"open_world": {}}}) == "mixed"
    with tempfile.TemporaryDirectory() as directory:
        config_path = Path(directory) / "config.json"
        config_path.write_text(
            json.dumps(
                {
                    "provider": EXPECTED_PROVIDER,
                    "api_url": EXPECTED_API_URL,
                    "model": EXPECTED_MODEL,
                    "thinking_enabled": True,
                    "api_key": "test-only-key",
                }
            ),
            encoding="utf-8",
        )
        assert load_config(config_path)["model"] == EXPECTED_MODEL
        valid_config = config_path.read_text(encoding="utf-8")
        config_path.write_text("[]", encoding="utf-8")
        try:
            load_config(config_path)
        except QualificationFailure as error:
            assert error.code == "invalid_provider_config_shape"
        else:
            raise AssertionError("non-object provider config was accepted")
        config_path.write_text(valid_config, encoding="utf-8")
        parsed = parse_sse(
            b'event: delta\ndata: {"content":"ok"}\n\nevent: done\ndata: {"turn_id":"00000000-0000-4000-8000-000000000000","committed":true,"replayed":false}\n\n'
        )
        assert parsed["response_chars"] == 2
        assert parsed["response_sha256"] == sha256_bytes(b"ok")
        metrics = Path(directory) / "recorded-release.prom"
        metrics.write_bytes(
            (root / "tools" / "llm-budget" / "recorded-release.prom")
            .read_bytes()
            .replace(
                b'provider="environment",model="e2e"',
                f'provider="{EXPECTED_PROVIDER}",model="{EXPECTED_MODEL}"'.encode(),
            )
        )
        summary = summarize_metrics(root, [("fixture", metrics)])
        encoded = json.dumps(summary)
        assert "usage_key" not in encoded
        assert summary["counter_totals"]
        assert PROJECT_PATTERN.fullmatch("nwq-0123456789")
        release_phases = {
            "pull": (10, 25),
            "database_start": (25, 40),
            "migration": (40, 55),
            "application_deployment": (55, 85),
            "readiness": (85, 100),
        }
        release_log = "".join(
            f"qualification-phase {phase} {boundary} {timestamp}\n"
            for phase, (started, ended) in release_phases.items()
            for boundary, timestamp in (("start", started), ("end", ended))
        )
        assert release_phase_durations(release_log, set(release_phases)) == {
            phase: ended - started
            for phase, (started, ended) in release_phases.items()
        }
        for invalid_release_log in (
            release_log.replace("qualification-phase pull end 25\n", ""),
            release_log + "qualification-phase pull end 25\n",
        ):
            try:
                release_phase_durations(invalid_release_log, set(release_phases))
            except QualificationFailure as error:
                assert error.code == "release_phase_timing_invalid"
            else:
                raise AssertionError("invalid release timing was accepted")
        committed = {
            "chapter_number": 1,
            "world_state": {"state": {"choices": [1]}, "updated_at": "first"},
        }
        replayed = {
            "chapter_number": 1,
            "world_state": {"state": {"choices": [1]}, "updated_at": "replayed"},
        }
        assert choice_replay_projection(committed) == choice_replay_projection(replayed)
        replayed["world_state"]["state"]["choices"].append(2)
        assert choice_replay_projection(committed) != choice_replay_projection(replayed)

        manifest_path = Path(directory) / "release.env"
        manifest_path.write_text(
            "\n".join(
                [
                    "RELEASE_VERSION=test-v1",
                    f"RELEASE_GIT_SHA={'1' * 40}",
                    *[
                        f"{key}=registry.example/novel/{key.lower()}@sha256:{index:064x}"
                        for index, key in enumerate(RELEASE_IMAGE_KEYS, 1)
                    ],
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        loaded_manifest = load_release_manifest(manifest_path)
        assert loaded_manifest["GATEWAY_IMAGE"].rsplit("@", 1)[1].startswith("sha256:")
        report_probe = Journey(
            root,
            config_path,
            Path(directory),
            "1" * 40,
            manifest_path,
            manifest_path,
            None,
            None,
            "bash",
            "Diagnostic",
        )
        report_probe.report["failure"] = {
            "stage": "private-stage",
            "code": "test_failure",
        }
        public = report_probe.public_report()
        assert set(public) == {
            "schema_version",
            "report_kind",
            "evidence_class",
            "journey_slice",
            "qualification_claim",
            "attempt_id",
            "outcome",
            "provider",
            "fixture_manifest_sha256",
            "application_image_content_digests",
            "aggregate",
            "failure",
        }
        assert public["failure"] == {"code": "test_failure"}
        assert "private-stage" not in json.dumps(public)
        fixture = load_product_input(root / PRODUCT_INPUT)
        assert len(product_source(fixture)) > 400

        labels = (
            'contract="llm-observability-v1",service="narrative-service",'
            'provider="deepseek",model="deepseek-v4-flash",'
            'operation="narrative_transition",mode="sync"'
        )
        before = (
            "# TYPE novelworld_llm_requests_started_total counter\n"
            f"novelworld_llm_requests_started_total{{{labels}}} 4\n"
        ).encode()
        after = (
            "# TYPE novelworld_llm_requests_started_total counter\n"
            f"novelworld_llm_requests_started_total{{{labels}}} 5\n"
        ).encode()
        wrong_model = after.replace(b'deepseek-v4-flash', b'other-model')
        assert provider_started_delta(
            root,
            before,
            after,
            service="narrative-service",
            operation="narrative_transition",
        ) == 1
        assert provider_started_delta(
            root,
            before,
            after,
            service="narrative-service",
        ) == 1
        assert provider_started_delta(
            root,
            before,
            before,
            service="narrative-service",
            operation="narrative_transition",
        ) == 0
        try:
            provider_started_delta(
                root,
                wrong_model,
                wrong_model,
                service="narrative-service",
                operation="narrative_transition",
            )
        except QualificationFailure as error:
            assert error.code == "provider_identity_changed"
        else:
            raise AssertionError("wrong provider identity was ignored")

        model_log = json.dumps(
            {
                "fields": {
                    "message": "LLM response model observed",
                    "provider": EXPECTED_PROVIDER,
                    "configured_model": EXPECTED_MODEL,
                    "response_model": EXPECTED_MODEL,
                    "operation": "narrative_transition",
                    "mode": "sync",
                }
            }
        )
        observations = response_models_from_logs(model_log, "narrative-service")
        repeated_log = f"{model_log}\n{model_log}"
        unseen, total = unseen_response_models(
            repeated_log, "narrative-service", 1
        )
        assert len(unseen) == 1 and total == 2
        replaced, total = unseen_response_models(
            repeated_log, "narrative-service", 0
        )
        assert len(replaced) == 2 and total == 2
        try:
            unseen_response_models(model_log, "narrative-service", 2)
        except QualificationFailure as error:
            assert error.code == "response_model_log_rewound"
        else:
            raise AssertionError("rewound response-model log was accepted")
        assert valid_public_model(EXPECTED_MODEL)
        assert not valid_public_model("invalid")
        assert not valid_public_model("https://model.invalid")
        assert not valid_public_model("x" * 201)
        success_metrics = Path(directory) / "success.prom"
        success_metrics.write_text(
            "# TYPE novelworld_llm_requests_total counter\n"
            f"novelworld_llm_requests_total{{{labels},status=\"success\"}} 1\n",
            encoding="utf-8",
        )
        assert verify_response_models(
            root,
            [("success", success_metrics)],
            observations,
            [EXPECTED_MODEL],
        ) == {
            "successful_calls": 1,
            "observed_response_models": {EXPECTED_MODEL: 1},
        }
        wrong_metrics = Path(directory) / "wrong.prom"
        wrong_metrics.write_bytes(success_metrics.read_bytes().replace(
            EXPECTED_MODEL.encode(), b"other-model"
        ))
        try:
            summarize_metrics(root, [("wrong", wrong_metrics)])
        except QualificationFailure as error:
            assert error.code == "provider_identity_changed"
        else:
            raise AssertionError("wrong metric identity entered the public summary")
        try:
            verify_response_models(
                root,
                [("success", success_metrics)],
                observations,
                ["https://model.invalid"],
            )
        except QualificationFailure as error:
            assert error.code == "response_model_allowlist_invalid"
        else:
            raise AssertionError("unsafe response-model allowlist was accepted")

        trace_id = "00000000-0000-4000-8000-000000000001"
        marker = json.dumps(
            {
                "fields": {
                    "message": "memory context selected",
                    "memory_layer": "mid",
                    "selected_count": 2,
                },
                "span": {"name": "chat_handler"},
                "spans": [
                    {"name": "global", "trace_id": ""},
                    {"name": "request", "trace_id": trace_id},
                ],
            }
        )
        assert selected_mid_from_logs(marker, trace_id) == 2
        for invalid_logs in (
            marker + "\n" + marker,
            marker.replace('"selected_count": 2', '"selected_count": 0'),
            marker.replace(trace_id, str(uuid.uuid4())),
            json.dumps(
                {
                    "fields": {
                        "message": "unrelated",
                        "memory_layer": "mid",
                        "selected_count": 2,
                    },
                    "span": {"name": "chat_handler"},
                    "spans": [{"trace_id": trace_id}],
                }
            ),
        ):
            try:
                selected_mid_from_logs(invalid_logs, trace_id)
            except QualificationFailure as error:
                assert error.code == "mid_selection_marker_missing"
            else:
                raise AssertionError("invalid Mid marker was accepted")

        identity = {"registered": True}
        cohort_path = Path(directory) / "cohort.json"
        cohort_path.write_text(
            json.dumps(
                {
                    "manifest_version": "h4-cohort-v1",
                    "cohort_id": sha256_bytes(canonical_json(identity)),
                    "identity": identity,
                }
            ),
            encoding="utf-8",
        )
        cohort = load_cohort_manifest(cohort_path)
        invalid_ledger_path = Path(directory) / "invalid-ledger.jsonl"
        invalid_ledger_path.write_text("{", encoding="utf-8")
        invalid_ledger = QualificationLedger(
            invalid_ledger_path, cohort["cohort_id"], str(uuid.uuid4())
        )
        try:
            invalid_ledger.start()
        except QualificationFailure as error:
            assert error.code == "qualification_ledger_invalid"
        else:
            raise AssertionError("invalid ledger was accepted")
        invalid_ledger_path.write_text("", encoding="utf-8")
        recovered_ledger = QualificationLedger(
            invalid_ledger_path, cohort["cohort_id"], str(uuid.uuid4())
        )
        assert recovered_ledger.start() == 1
        recovered_ledger.finish(True, None)
        recovered_records = [
            json.loads(line)
            for line in invalid_ledger_path.read_text(encoding="utf-8").splitlines()
        ]
        assert recovered_records[-1]["status"] == "Passed"
        assert recovered_records[-1]["failure_code"] is None
        ledger_path = Path(directory) / "ledger.jsonl"
        ledger_path.write_text("", encoding="utf-8")
        first = QualificationLedger(
            ledger_path, cohort["cohort_id"], str(uuid.uuid4())
        )
        assert first.start() == 1
        first.finish(True, None)
        abandoned = QualificationLedger(
            ledger_path, cohort["cohort_id"], str(uuid.uuid4())
        )
        assert abandoned.start() == 2
        abandoned._unlock()
        recovery = QualificationLedger(
            ledger_path, cohort["cohort_id"], str(uuid.uuid4())
        )
        try:
            recovery.start()
        except QualificationFailure as error:
            assert error.code == "cohort_terminal_failed"
        else:
            raise AssertionError("abandoned attempt did not fail its cohort")

        def pass_core_attempts(path: Path) -> None:
            path.write_text("", encoding="utf-8")
            for sequence in range(1, 4):
                attempt = QualificationLedger(
                    path, cohort["cohort_id"], str(uuid.uuid4())
                )
                assert attempt.start() == sequence
                attempt.finish(True, None)

        compatibility_path = Path(directory) / "compatibility-ledger.jsonl"
        compatibility_path.write_text("", encoding="utf-8")
        early_compatibility = QualificationLedger(
            compatibility_path,
            cohort["cohort_id"],
            str(uuid.uuid4()),
            "legacy-character",
        )
        try:
            early_compatibility.start()
        except QualificationFailure as error:
            assert error.code == "qualification_core_incomplete"
        else:
            raise AssertionError("early compatibility attempt was accepted")
        pass_core_attempts(compatibility_path)
        compatibility = QualificationLedger(
            compatibility_path,
            cohort["cohort_id"],
            str(uuid.uuid4()),
            "legacy-character",
        )
        assert compatibility.start() == 1
        compatibility.finish(True, None)
        second_compatibility = QualificationLedger(
            compatibility_path,
            cohort["cohort_id"],
            str(uuid.uuid4()),
            "legacy-character",
        )
        try:
            second_compatibility.start()
        except QualificationFailure as error:
            assert error.code == "compatibility_attempt_already_completed"
        else:
            raise AssertionError("second compatibility attempt was accepted")

        failed_compatibility_path = Path(directory) / "failed-compatibility-ledger.jsonl"
        pass_core_attempts(failed_compatibility_path)
        failed_compatibility = QualificationLedger(
            failed_compatibility_path,
            cohort["cohort_id"],
            str(uuid.uuid4()),
            "legacy-character",
        )
        assert failed_compatibility.start() == 1
        failed_compatibility.finish(False, "test_failure")
        failed_records = [
            json.loads(line)
            for line in failed_compatibility_path.read_text(encoding="utf-8").splitlines()
        ]
        core_failed_records = [
            record
            for record in failed_records
            if record.get("journey_slice", "core") == "core"
        ]
        assert sum(record["status"] == "Started" for record in core_failed_records) == 3
        assert sum(record["status"] == "Passed" for record in core_failed_records) == 3
        assert failed_records[-1]["failure_code"] == "test_failure"

        abandoned_compatibility_path = (
            Path(directory) / "abandoned-compatibility-ledger.jsonl"
        )
        pass_core_attempts(abandoned_compatibility_path)
        abandoned_compatibility = QualificationLedger(
            abandoned_compatibility_path,
            cohort["cohort_id"],
            str(uuid.uuid4()),
            "legacy-character",
        )
        assert abandoned_compatibility.start() == 1
        abandoned_compatibility._unlock()
        recovered_compatibility = QualificationLedger(
            abandoned_compatibility_path,
            cohort["cohort_id"],
            str(uuid.uuid4()),
            "legacy-character",
        )
        try:
            recovered_compatibility.start()
        except QualificationFailure as error:
            assert error.code == "cohort_terminal_failed"
        else:
            raise AssertionError("abandoned compatibility attempt was accepted")
    print("live DeepSeek journey self-test passed")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--config", type=Path)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--git-sha")
    parser.add_argument("--base-manifest", type=Path)
    parser.add_argument("--candidate-manifest", type=Path)
    parser.add_argument("--cohort-manifest", type=Path)
    parser.add_argument("--ledger", type=Path)
    parser.add_argument("--release-shell", default="bash")
    parser.add_argument(
        "--evidence-class",
        choices=("Diagnostic", "Qualification"),
        default="Diagnostic",
    )
    parser.add_argument(
        "--slice", choices=("core", "legacy-character"), default="core"
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(__file__).resolve().parents[2]
    if args.self_test:
        self_test(root)
        return 0
    required = (
        args.config,
        args.output_dir,
        args.git_sha,
        args.base_manifest,
        args.candidate_manifest,
    )
    if any(value is None for value in required):
        raise QualificationFailure("required_runner_input_missing")
    if not COMMIT_PATTERN.fullmatch(args.git_sha):
        raise QualificationFailure("invalid_git_sha")
    if git(root, "rev-parse", "HEAD") != args.git_sha:
        raise QualificationFailure("git_sha_does_not_match_checkout")
    if git(root, "status", "--porcelain=v1", "--untracked-files=all"):
        raise QualificationFailure("qualification_requires_clean_checkout")
    output = args.output_dir.resolve()
    root_resolved = root.resolve()
    if output == root_resolved or root_resolved in output.parents:
        raise QualificationFailure("output_dir_must_be_outside_checkout")
    private_paths = [args.config, args.cohort_manifest, args.ledger]
    if any(
        path is not None
        and (path.resolve() == root_resolved or root_resolved in path.resolve().parents)
        for path in private_paths
    ):
        raise QualificationFailure("private_input_must_be_outside_checkout")
    if args.evidence_class == "Qualification":
        if not output.is_dir() or any(output.iterdir()):
            raise QualificationFailure("qualification_output_must_be_precreated_and_empty")
    else:
        output.mkdir(parents=True, exist_ok=True)
        if any(output.iterdir()):
            raise QualificationFailure("diagnostic_output_must_be_empty")
    journey = Journey(
        root,
        args.config.resolve(),
        output,
        args.git_sha,
        args.base_manifest.resolve(),
        args.candidate_manifest.resolve(),
        args.cohort_manifest.resolve() if args.cohort_manifest else None,
        args.ledger.resolve() if args.ledger else None,
        args.release_shell,
        args.evidence_class,
        args.slice,
    )
    try:
        journey.execute()
    except QualificationFailure as error:
        journey.report["failure"] = {"stage": journey.current_stage, "code": error.code}
    except Exception:
        journey.report["failure"] = {"stage": journey.current_stage, "code": "unexpected_runner_failure"}
    finally:
        if journey.stack_started and not journey.private_report.get(
            "observability_finalized"
        ):
            try:
                journey.finalize_observability("failure", best_effort=True)
            except Exception:
                journey.private_report.setdefault("observability_errors", {})[
                    "failure"
                ] = ["unexpected_observability_failure"]
        try:
            journey.cleanup()
        except Exception:
            journey.report["outcome"] = "failed"
            journey.report["failure"] = {"stage": "cleanup", "code": "unexpected_cleanup_failure"}
        report_written = False
        try:
            journey.write_report()
            report_written = True
        finally:
            if journey.ledger is not None:
                failure = journey.report.get("failure", {}).get("code")
                passed = journey.report["outcome"] == "completed" and report_written
                journey.ledger.finish(
                    passed,
                    None
                    if passed
                    else (failure if isinstance(failure, str) else "report_write_failed"),
                )
    return 0 if journey.report["outcome"] == "completed" else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except QualificationFailure as error:
        print(f"live qualification failed: {error.code}", file=sys.stderr)
        raise SystemExit(1)
