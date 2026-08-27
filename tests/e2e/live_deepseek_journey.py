#!/usr/bin/env python3
"""Isolated, baseline-only DeepSeek golden journey.

The runner deliberately emits only bounded aggregate evidence. Raw provider
configuration, source text, model prose, user IDs, and conversations never
enter the report. The two raw Prometheus files stay in the operator-selected
output directory and must not be committed because they contain a stable
usage-key fingerprint.
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
from typing import Any, Iterable


EXPECTED_PROVIDER = "deepseek"
EXPECTED_MODEL = "deepseek-v4-flash"
EXPECTED_API_URL = "https://api.deepseek.com"
PROJECT_PATTERN = re.compile(r"^nwq-[a-f0-9]{10}$")
COMMIT_PATTERN = re.compile(r"^[0-9a-f]{40}$")
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


class QualificationFailure(RuntimeError):
    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


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


def git(root: Path, *args: str) -> str:
    return run(["git", *args], cwd=root)


def load_config(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise QualificationFailure("invalid_provider_config") from error
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
    return {"done": done, "response_chars": delta_chars}


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
                "raw_sha256": sha256_bytes(raw),
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


def reserve_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def existing_stack_snapshot() -> dict[str, str]:
    names = run(["docker", "ps", "-a", "--format", "{{.Names}}"])
    result = {}
    for name in sorted(line for line in names.splitlines() if line.startswith("novel-")):
        result[name] = run(
            [
                "docker",
                "inspect",
                "--format",
                "{{.Id}}|{{.Image}}|{{.State.Status}}|{{.State.StartedAt}}|{{.RestartCount}}",
                name,
            ]
        )
    return result


class Journey:
    def __init__(
        self,
        root: Path,
        config_path: Path,
        output: Path,
        git_sha: str,
        keep_stack: bool,
    ):
        self.root = root
        self.config_path = config_path
        self.output = output
        self.git_sha = git_sha
        self.keep_stack = keep_stack
        self.config = load_config(config_path)
        suffix = secrets.token_hex(5)
        self.project = f"nwq-{suffix}"
        self.prefix = self.project
        self.port = reserve_port()
        self.api = f"http://127.0.0.1:{self.port}/api"
        self.compose_env: dict[str, str] = {}
        self.env_file: Path | None = None
        self.stack_started = False
        self.current_stage = "preflight"
        self.user_stack_before: dict[str, str] = {}
        self.report: dict[str, Any] = {
            "schema_version": 1,
            "report_kind": "deepseek-v4-flash-live-baseline-v1",
            "evidence_class": "baseline",
            "baseline_only": True,
            "evaluated_git_sha": git_sha,
            "evidence_commit": None,
            "started_at": utc_now(),
            "completed_at": None,
            "outcome": "failed",
            "provider": {
                "name": EXPECTED_PROVIDER,
                "configured_model": EXPECTED_MODEL,
                "api_origin": EXPECTED_API_URL,
                "product_thinking_enabled": True,
                "api_key_configured": True,
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
                "image_generation_calls": 0,
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

    def compose(self, *args: str, capture: bool = True, check: bool = True) -> str:
        if self.env_file is None:
            raise QualificationFailure("compose_environment_missing")
        return run(
            [
                "docker",
                "compose",
                "--project-name",
                self.project,
                "--env-file",
                str(self.env_file),
                "-f",
                str(self.root / "docker-compose.yml"),
                *args,
            ],
            cwd=self.root,
            env=self.compose_env,
            capture=capture,
            check=check,
        )

    def prepare_compose(self, temporary: Path) -> None:
        if not PROJECT_PATTERN.fullmatch(self.project):
            raise QualificationFailure("unsafe_compose_project")
        short_sha = self.git_sha[:12]
        password = secrets.token_urlsafe(32)
        variables = {
            "CONTAINER_PREFIX": self.prefix,
            "POSTGRES_USER": "novel",
            "POSTGRES_DB": "novel_world",
            "POSTGRES_PASSWORD": password,
            "JWT_SECRET": secrets.token_urlsafe(48),
            "RUNTIME_CONFIG_KEY": secrets.token_hex(32),
            "INTERNAL_SERVICE_TOKEN": secrets.token_urlsafe(48),
            "LLM_API_KEY": "",
            "CACHE_MODE": "postgres",
            "REDIS_URL": "memory://",
            "S3_ENABLED": "false",
            "IMAGE_GEN_API_KEY": "",
            "NGINX_HTTP_BIND": "127.0.0.1",
            "NGINX_HTTP_PORT": str(self.port),
            "CORS_ORIGINS": f"http://127.0.0.1:{self.port}",
            "GATEWAY_IMAGE": f"{self.project}-gateway:{short_sha}",
            "USER_SERVICE_IMAGE": f"{self.project}-user-service:{short_sha}",
            "NOVEL_SERVICE_IMAGE": f"{self.project}-novel-service:{short_sha}",
            "AGENT_SERVICE_IMAGE": f"{self.project}-agent-service:{short_sha}",
            "NARRATIVE_SERVICE_IMAGE": f"{self.project}-narrative-service:{short_sha}",
            "FRONTEND_IMAGE": f"{self.project}-frontend:{short_sha}",
        }
        self.env_file = temporary / "compose.env"
        self.env_file.write_text(
            "".join(f"{key}={value}\n" for key, value in variables.items()),
            encoding="utf-8",
        )
        self.compose_env = {**os.environ, **variables, "DOCKER_BUILDKIT": "1"}

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

    def chat(self, token: str, novel_id: str, character_id: str, message: str) -> dict[str, Any]:
        turn_id = str(uuid.uuid4())
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
            headers={"Content-Type": "application/json", "Idempotency-Key": turn_id},
            timeout=600,
        )
        if "text/event-stream" not in headers.get("Content-Type", ""):
            raise QualificationFailure("chat_not_sse")
        parsed = parse_sse(payload)
        if parsed["done"].get("turn_id") != turn_id:
            raise QualificationFailure("chat_turn_identity_mismatch")
        parsed["turn_id"] = turn_id
        return parsed

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

    def collect_metrics(self, name: str) -> Path:
        destination = self.output / f"product-{name}.prom"
        chunks = []
        for service, port in SERVICE_PORTS.items():
            chunks.append(
                run(
                    [
                        "docker",
                        "exec",
                        f"{self.prefix}-{service}",
                        "curl",
                        "--fail",
                        "--silent",
                        f"http://127.0.0.1:{port}/metrics",
                    ]
                )
            )
        destination.write_text("\n\n".join(chunks) + "\n", encoding="utf-8")
        return destination

    def restart_services(self) -> None:
        names = [f"{self.prefix}-{service}" for service in SERVICE_PORTS]
        names.append(f"{self.prefix}-gateway")
        run(["docker", "restart", *names])
        self.wait_gateway()

    def execute(self, temporary: Path) -> None:
        self.prepare_compose(temporary)
        self.user_stack_before = existing_stack_snapshot()
        self.report["environment"].update(
            {
                "docker_server": run(
                    ["docker", "version", "--format", "{{.Server.Version}}|{{.Server.Os}}|{{.Server.Arch}}"]
                ),
                "docker_compose": run(["docker", "compose", "version", "--short"]),
                "compose_sha256": sha256_bytes((self.root / "docker-compose.yml").read_bytes()),
            }
        )

        with self.stage("isolated_compose_start"):
            self.compose("config", "--quiet")
            self.compose("up", "--build", "--detach", capture=False)
            self.stack_started = True
            self.wait_gateway()
            tags = {
                key: value
                for key, value in self.compose_env.items()
                if key.endswith("_IMAGE") and value.startswith(self.project)
            }
            self.report["environment"]["image_ids"] = {
                key.lower(): run(["docker", "image", "inspect", "--format", "{{.Id}}", tag])
                for key, tag in sorted(tags.items())
            }

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

        corpus_path = self.root / "tools" / "h1-eval" / "corpus" / "v1.json"
        corpus_bytes = corpus_path.read_bytes()
        corpus = json.loads(corpus_bytes)
        case = next(item for item in corpus["positive_cases"] if item["id"] == "zh-utf8")
        source = case["source"].encode("utf-8")
        self.report["journey"]["source"] = {
            "slice": "zh-utf8",
            "corpus_version": corpus["corpus_version"],
            "corpus_sha256": sha256_bytes(corpus_bytes),
            "source_sha256": sha256_bytes(source),
        }

        with self.stage("live_import"):
            upload_body, content_type = multipart(
                {"title": case["novel_title"], "author": "Qualification", "deviation_mode": "creative"},
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
                value={"identity_type": "self", "identity_name": "云舟", "character_id": None},
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
                    "name": "云舟",
                    "background": "北塔附近的地图学徒。",
                    "capabilities": ["辨认星图"],
                    "location_id": location_id,
                    "inventory": ["旧地图"],
                },
            )
            if not player.get("player", {}).get("id"):
                raise QualificationFailure("player_entry_not_committed")
            node = request_json(f"{self.api}/narrative/{novel_id}/{checkpoint}", token=token)
            node_id = node.get("id")
            if not isinstance(node_id, str):
                raise QualificationFailure("canonical_branch_node_missing")
            choice = request_json(
                f"{self.api}/narrative/choose",
                method="POST",
                token=token,
                value={"novel_id": novel_id, "node_id": node_id, "choice_index": 0},
                timeout=600,
            )
            transition = choice.get("transition", {})
            actor_ids = [
                actor
                for event in transition.get("events", [])
                for actor in event.get("actor_character_ids", [])
            ]
            if not actor_ids:
                raise QualificationFailure("branch_has_no_character_witness")
            branch_replay = request_json(
                f"{self.api}/narrative/choose",
                method="POST",
                token=token,
                value={"novel_id": novel_id, "node_id": node_id, "choice_index": 0},
            )
            if branch_replay != choice:
                raise QualificationFailure("branch_replay_mismatch")
            self.report["journey"].update(
                {"branch_committed": True, "branch_replayed": True, "branch_checkpoint": checkpoint}
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
            branch_chat = self.chat(token, novel_id, character_id, "你如何看待我们刚才作出的选择？")
            self.assert_chat_revision(branch_chat["turn_id"], context["world_revision"])
            self.report["journey"].update(
                {
                    "full_persona_source_high_water": total_chapters,
                    "branch_context_visible_to_witness": True,
                    "branch_chat_world_revision_exact": True,
                }
            )

        with self.stage("twelve_turn_world_trajectory"):
            world = request_json(
                f"{self.api}/narrative/{novel_id}/world",
                method="POST",
                token=token,
                value=None,
                timeout=600,
            )
            if world.get("session", {}).get("turn_number") != 0:
                raise QualificationFailure("world_not_fresh")
            world_turn_ids: list[str] = []
            first_action: dict[str, Any] | None = None
            first_result: Any = None
            chat_turns = 1
            for number in range(1, 13):
                turn_id = str(uuid.uuid4())
                action = {
                    "expected_turn_number": number - 1,
                    "kind": "converse",
                    "target_id": character_id,
                    "intent": f"第{number}次与守塔人核对星图、当前线索和彼此的下一步计划",
                }
                body = json.dumps(action, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
                payload, _, _ = request_bytes(
                    f"{self.api}/narrative/{novel_id}/world/turns",
                    method="POST",
                    token=token,
                    body=body,
                    headers={"Content-Type": "application/json", "Idempotency-Key": turn_id},
                    timeout=600,
                )
                result = json.loads(payload)
                if (
                    result.get("turn_id") != turn_id
                    or result.get("memory_projection_status") != "saved"
                    or result.get("world_state", {}).get("state", {}).get("open_world", {}).get("turn_number") != number
                ):
                    raise QualificationFailure("world_turn_not_committed")
                world_turn_ids.append(turn_id)
                if number == 1:
                    first_action, first_result = action, result
                    replay_payload, _, _ = request_bytes(
                        f"{self.api}/narrative/{novel_id}/world/turns",
                        method="POST",
                        token=token,
                        body=body,
                        headers={"Content-Type": "application/json", "Idempotency-Key": turn_id},
                    )
                    if json.loads(replay_payload) != result:
                        raise QualificationFailure("world_turn_replay_mismatch")
                if number <= 11:
                    chat_result = self.chat(
                        token,
                        novel_id,
                        character_id,
                        f"世界推进到第{number}回合后，你认为哪些已发生的事实最重要？",
                    )
                    chat_turns += 1
                    current = self.internal_character_context(user_id, novel_id, character_id)
                    self.assert_chat_revision(chat_result["turn_id"], current["world_revision"])
                    if chat_turns == 10:
                        for _ in range(180):
                            mid_count = int(
                                self.db_scalar(
                                    "SELECT COUNT(*) FROM character_memories "
                                    f"WHERE user_id = '{user_id}' AND novel_id = '{novel_id}' "
                                    f"AND character_id = '{character_id}' AND layer = 'mid'"
                                )
                            )
                            if mid_count >= 1:
                                break
                            time.sleep(2)
                        else:
                            raise QualificationFailure("mid_memory_window_not_projected")
            world_view = request_json(f"{self.api}/narrative/{novel_id}/world", token=token)
            if world_view.get("session", {}).get("turn_number") != 12 or len(world_view.get("journal", [])) != 12:
                raise QualificationFailure("world_trajectory_incomplete")
            permanent_count = int(
                self.db_scalar(
                    "SELECT COUNT(*) FROM character_memories "
                    f"WHERE user_id = '{user_id}' AND novel_id = '{novel_id}' "
                    f"AND character_id = '{character_id}' AND layer = 'permanent' "
                    "AND content::jsonb ->> 'source' = 'committed_world_turn'"
                )
            )
            if permanent_count < 12:
                raise QualificationFailure("journey_memory_trajectory_incomplete")
            final_context = self.internal_character_context(user_id, novel_id, character_id)
            recent_actions = (final_context.get("world_context") or {}).get("recent_actions", [])
            if len(recent_actions) != 4 or any(action.get("target_id") != character_id for action in recent_actions):
                raise QualificationFailure("character_visibility_window_invalid")
            self.report["journey"].update(
                {
                    "world_turns": 12,
                    "world_turn_replay_exact": True,
                    "pre_restart_chat_turns": chat_turns,
                    "mid_memory_windows": mid_count,
                    "committed_world_memories": permanent_count,
                    "character_recent_action_window": len(recent_actions),
                }
            )
            if first_action is None or first_result is None:
                raise QualificationFailure("world_replay_fixture_missing")

        time.sleep(2)
        pre_metrics = self.collect_metrics("pre-restart")

        with self.stage("restart_and_exact_replay"):
            self.restart_services()
            last_turn_id = world_turn_ids[-1]
            last_action = {
                "expected_turn_number": 11,
                "kind": "converse",
                "target_id": character_id,
                "intent": "第12次与守塔人核对星图、当前线索和彼此的下一步计划",
            }
            replay_body = json.dumps(last_action, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            replay_payload, _, _ = request_bytes(
                f"{self.api}/narrative/{novel_id}/world/turns",
                method="POST",
                token=token,
                body=replay_body,
                headers={"Content-Type": "application/json", "Idempotency-Key": last_turn_id},
            )
            replay = json.loads(replay_payload)
            if replay.get("turn_id") != last_turn_id or replay.get("world_state", {}).get("state", {}).get("open_world", {}).get("turn_number") != 12:
                raise QualificationFailure("restart_world_replay_failed")
            resumed_context = self.internal_character_context(user_id, novel_id, character_id)
            resumed_chat = self.chat(token, novel_id, character_id, "服务重启后，请只依据已提交事实回顾我们的长期旅程。")
            self.assert_chat_revision(resumed_chat["turn_id"], resumed_context["world_revision"])
            history = request_json(
                f"{self.api}/chat/{character_id}/history?limit=100&offset=0",
                token=token,
            )
            if history.get("count") != 26:
                raise QualificationFailure("restart_chat_history_incomplete")
            self.report["journey"].update(
                {
                    "post_restart_chat_turns": 1,
                    "total_chat_turns": 13,
                    "restart_world_replay": True,
                    "restart_chat_revision_exact": True,
                    "durable_chat_messages": history["count"],
                }
            )

        post_metrics = self.collect_metrics("post-restart")
        self.report["llm_metrics"] = summarize_metrics(
            self.root,
            [("pre_restart", pre_metrics), ("post_restart", post_metrics)],
        )

        with self.stage("prompt_and_schema_identity"):
            prompt_identity = {
                "canon": self.db_scalar(
                    f"SELECT prompt_version FROM canon_story_models WHERE novel_id = '{novel_id}'"
                ),
                "branch": self.db_scalar(
                    f"SELECT DISTINCT transition ->> 'prompt_version' FROM user_choices WHERE novel_id = '{novel_id}'"
                ),
                "world": self.db_scalar(
                    f"SELECT DISTINCT transition ->> 'prompt_version' FROM world_turns WHERE novel_id = '{novel_id}'"
                ),
            }
            if any(not value for value in prompt_identity.values()):
                raise QualificationFailure("prompt_identity_missing")
            self.report["journey"]["prompt_identity"] = prompt_identity

        with self.stage("export_and_account_erasure"):
            export, headers, _ = request_bytes(f"{self.api}/account/export", token=token, timeout=300)
            if "application/x-ndjson" not in headers.get("Content-Type", ""):
                raise QualificationFailure("export_content_type_invalid")
            try:
                records = [json.loads(line) for line in export.splitlines() if line]
            except json.JSONDecodeError as error:
                raise QualificationFailure("export_invalid_ndjson") from error
            if not records or records[-1].get("type") != "complete":
                raise QualificationFailure("export_incomplete")
            kinds = {record.get("kind") for record in records if record.get("type") == "record"}
            required_kinds = {"profile", "novel", "chapter", "character", "chat_message", "character_memory", "user_choice", "world_state", "world_turn"}
            if not required_kinds.issubset(kinds):
                raise QualificationFailure("export_missing_records")
            forbidden = [
                self.config["api_key"].encode(),
                token.encode(),
                password.encode(),
            ]
            if any(secret_value in export for secret_value in forbidden):
                raise QualificationFailure("export_contains_secret")
            request_bytes(
                f"{self.api}/auth/me",
                method="DELETE",
                token=token,
                expected=(204,),
            )
            request_bytes(
                f"{self.api}/auth/login",
                method="POST",
                body=json.dumps({"email": reader_email, "password": password}).encode(),
                headers={"Content-Type": "application/json"},
                expected=(401,),
            )
            private_counts = self.db_scalar(
                f"SELECT (SELECT COUNT(*) FROM users WHERE id = '{user_id}') + "
                + " + ".join(
                    f"(SELECT COUNT(*) FROM {table} WHERE user_id = '{user_id}')"
                    for table in PRIVATE_TABLES
                )
            )
            if private_counts != "0":
                raise QualificationFailure("account_erasure_incomplete")
            self.report["journey"].update(
                {
                    "account_export_complete": True,
                    "export_record_kinds": sorted(required_kinds),
                    "account_erasure_private_rows": 0,
                    "deleted_reader_cannot_login": True,
                }
            )

        self.report["outcome"] = "completed"

    def cleanup(self) -> None:
        cleanup_ok = True
        if self.stack_started and not self.keep_stack:
            if not PROJECT_PATTERN.fullmatch(self.project):
                cleanup_ok = False
            else:
                try:
                    self.compose("down", "--volumes", "--remove-orphans", capture=False)
                    self.stack_started = False
                except QualificationFailure:
                    cleanup_ok = False
        self.report["environment"]["isolated_cleanup_completed"] = cleanup_ok and not self.keep_stack
        after = existing_stack_snapshot()
        unchanged = after == self.user_stack_before
        self.report["environment"]["existing_user_stack_unchanged"] = unchanged
        if not unchanged and self.report["outcome"] == "completed":
            self.report["outcome"] = "failed"
            self.report["failure"] = {"stage": "cleanup", "code": "existing_user_stack_changed"}
        if not cleanup_ok and self.report["outcome"] == "completed":
            self.report["outcome"] = "failed"
            self.report["failure"] = {"stage": "cleanup", "code": "isolated_cleanup_failed"}

    def write_report(self) -> None:
        self.report["completed_at"] = utc_now()
        path = self.output / "journey-report.json"
        path.write_text(
            json.dumps(self.report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(f"live qualification report: {path}")


def self_test(root: Path) -> None:
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
        parsed = parse_sse(
            b'event: delta\ndata: {"content":"ok"}\n\nevent: done\ndata: {"turn_id":"00000000-0000-4000-8000-000000000000","committed":true,"replayed":false}\n\n'
        )
        assert parsed["response_chars"] == 2
        metrics = root / "tools" / "llm-budget" / "recorded-release.prom"
        summary = summarize_metrics(root, [("fixture", metrics)])
        encoded = json.dumps(summary)
        assert "usage_key" not in encoded
        assert summary["counter_totals"]
        assert PROJECT_PATTERN.fullmatch("nwq-0123456789")
    print("live DeepSeek journey self-test passed")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--config", type=Path)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--git-sha")
    parser.add_argument("--keep-stack", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(__file__).resolve().parents[2]
    if args.self_test:
        self_test(root)
        return 0
    if args.config is None or args.output_dir is None or args.git_sha is None:
        raise QualificationFailure("config_output_and_git_sha_are_required")
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
    output.mkdir(parents=True, exist_ok=True)
    journey = Journey(root, args.config.resolve(), output, args.git_sha, args.keep_stack)
    with tempfile.TemporaryDirectory(prefix="novelworld-live-") as directory:
        try:
            journey.execute(Path(directory))
        except QualificationFailure as error:
            journey.report["failure"] = {"stage": journey.current_stage, "code": error.code}
        except Exception:
            journey.report["failure"] = {"stage": journey.current_stage, "code": "unexpected_runner_failure"}
        finally:
            try:
                journey.cleanup()
            except Exception:
                journey.report["outcome"] = "failed"
                journey.report["failure"] = {"stage": "cleanup", "code": "unexpected_cleanup_failure"}
            journey.write_report()
    return 0 if journey.report["outcome"] == "completed" else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except QualificationFailure as error:
        print(f"live qualification failed: {error.code}", file=sys.stderr)
        raise SystemExit(1)
