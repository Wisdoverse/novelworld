"""Private, single-attempt controls for the existing Vision journey runner.

No provider client or executable entrypoint. Qualification keeps its own ledger.
Only synthetic callers may use this module before a separately approved live run.
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import re
import selectors
import signal
import stat
import subprocess
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REGISTRATION_SCHEMA = "vision-journey-registration-v1"
LEDGER_SCHEMA = "vision-journey-ledger-v1"
PROFILE_PATH = Path("tools/llm-budget/diagnostic-v1.json")
MODEL = "deepseek-v4-flash-vision-exp"
CONTRACT = "llm-diagnostic-budget-v1"
PROFILE = "vision-journey-diagnostic-v1"
APP_KEYS = {
    "GATEWAY_IMAGE", "USER_SERVICE_IMAGE", "NOVEL_SERVICE_IMAGE",
    "AGENT_SERVICE_IMAGE", "NARRATIVE_SERVICE_IMAGE", "FRONTEND_IMAGE",
}
REGISTRATION_KEYS = {
    "schema", "budget_id", "hypothesis", "candidate_git_sha",
    "base_manifest_sha256", "candidate_manifest_sha256",
    "base_application_image_ids", "candidate_application_image_ids",
    "profile_sha256", "product_fixture_sha256", "prompt_schema_identities",
    "limits", "output_dir", "ledger_path",
}


class DiagnosticFailure(RuntimeError):
    def __init__(self, code: str):
        super().__init__(code)
        self.code = code


def require(condition: bool, code: str = "diagnostic_registration_invalid") -> None:
    if not condition:
        raise DiagnosticFailure(code)


def canonical(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(",", ":"), allow_nan=False).encode("utf-8")


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def strict_json(value: bytes | str) -> Any:
    def pairs(items):
        result = {}
        for key, item in items:
            require(key not in result)
            result[key] = item
        return result

    def invalid_constant(_):
        raise DiagnosticFailure("diagnostic_registration_invalid")

    try:
        return json.loads(value, object_pairs_hook=pairs, parse_constant=invalid_constant)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise DiagnosticFailure("diagnostic_registration_invalid") from error


def integer(value: Any, maximum: int, minimum: int = 0) -> bool:
    return type(value) is int and minimum <= value <= maximum


def uuid4(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    try:
        parsed = uuid.UUID(value)
        return (str(parsed) == value and parsed.version == 4
                and parsed.variant == uuid.RFC_4122)
    except ValueError:
        return False


def expiry(value: Any) -> datetime:
    require(isinstance(value, str) and bool(re.fullmatch(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", value)))
    try:
        return datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except ValueError as error:
        raise DiagnosticFailure("diagnostic_registration_invalid") from error


def private_path(path: Path, root: Path, *, directory: bool = False) -> Path:
    require(path.is_absolute() and path == path.resolve(), "diagnostic_private_path_invalid")
    require(not any(part.is_symlink() for part in (path, *path.parents)),
            "diagnostic_private_path_invalid")
    require(path != root.resolve() and root.resolve() not in path.parents,
            "diagnostic_private_path_in_checkout")
    try:
        info = path.stat()
    except OSError as error:
        raise DiagnosticFailure("diagnostic_private_path_invalid") from error
    require((stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
            and not info.st_mode & 0o077, "diagnostic_private_permissions_invalid")
    return path


def sync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


class Registration:
    """Canonical immutable bytes bind every run input to the reviewed digest."""

    def __init__(self, encoded: bytes, profile: dict[str, Any]):
        self.encoded = encoded
        self.sha256 = digest(encoded)
        self.profile = profile

    @property
    def value(self) -> dict[str, Any]:
        return strict_json(self.encoded)

    @property
    def binding(self) -> dict[str, str]:
        value = self.value
        return {"contract": CONTRACT, "profile": PROFILE,
                "profile_sha256": value["profile_sha256"], "budget_id": value["budget_id"]}

    def environment(self) -> dict[str, str]:
        value = self.value
        return {"LLM_DIAGNOSTIC_BUDGET_ID": value["budget_id"],
                "LLM_DIAGNOSTIC_BUDGET_LIMITS": canonical(value["limits"]).decode()}


def load_registration(
    path: Path, approved_sha256: str, *, root: Path, git_sha: str,
    output: Path, base_manifest: Path, candidate_manifest: Path,
    prompt_schema_identities: dict[str, Any], now: datetime | None = None,
) -> Registration:
    private_path(path, root)
    require(isinstance(approved_sha256, str) and bool(re.fullmatch(
        r"[0-9a-f]{64}", approved_sha256)))
    try:
        with path.open("rb") as stream:
            raw = stream.read(65537)
        require(len(raw) <= 65536)
        value = strict_json(raw)
        require(isinstance(value, dict) and set(value) == REGISTRATION_KEYS)
        encoded = canonical(value)
        require(digest(encoded) == approved_sha256, "diagnostic_registration_digest_mismatch")
        require(value["schema"] == REGISTRATION_SCHEMA and uuid4(value["budget_id"]))
        hypothesis = value["hypothesis"]
        require(isinstance(hypothesis, str) and 1 <= len(hypothesis) <= 2000
                and hypothesis == hypothesis.strip() and all(char.isprintable() for char in hypothesis))
        require(value["candidate_git_sha"] == git_sha and bool(re.fullmatch(r"[0-9a-f]{40}", git_sha)))
        require(value["base_manifest_sha256"] == digest(base_manifest.read_bytes())
                and value["candidate_manifest_sha256"] == digest(candidate_manifest.read_bytes()),
                "diagnostic_manifest_mismatch")
        profile_raw = (root / PROFILE_PATH).read_bytes()
        profile = strict_json(profile_raw)
        require(value["profile_sha256"] == digest(profile_raw)
                and profile["contract"] == CONTRACT and profile["profile"] == PROFILE
                and profile["model"] == MODEL and profile["provider"] == "deepseek"
                and profile["origin"] == "https://api.deepseek.com"
                and profile["thinking_enabled"] is False, "diagnostic_profile_mismatch")
        require(value["product_fixture_sha256"] == digest(
            (root / "tests/e2e/fixtures/h4-journey-v1.json").read_bytes()),
            "diagnostic_fixture_mismatch")
        require(value["prompt_schema_identities"] == prompt_schema_identities,
                "diagnostic_prompt_identity_mismatch")
        for key in ("base_application_image_ids", "candidate_application_image_ids"):
            images = value[key]
            require(isinstance(images, dict) and set(images) == APP_KEYS
                    and all(isinstance(image, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", image)
                            for image in images.values()), "diagnostic_artifact_identity_invalid")
        limits = value["limits"]
        require(isinstance(limits, dict) and set(limits) == {
            "profile", "max_attempts", "max_tokens", "max_cost_micro_cny", "expires_at"})
        require(limits["profile"] == PROFILE)
        for name, ceiling in profile["max_limits"].items():
            require(integer(limits["max_" + name], ceiling))
        remaining = (expiry(limits["expires_at"]) - (now or datetime.now(timezone.utc))).total_seconds()
        require(0 < remaining <= profile["max_lifetime_seconds"], "diagnostic_expiry_invalid")
        require(isinstance(value["output_dir"], str) and Path(value["output_dir"]) == output)
        private_path(output, root, directory=True)
        require(not any(output.iterdir()), "diagnostic_output_not_empty")
        require(isinstance(value["ledger_path"], str))
        ledger = Path(value["ledger_path"])
        require(ledger.is_absolute() and ledger == ledger.resolve()
                and ledger.name == value["budget_id"] + ".jsonl")
        private_path(ledger.parent, root, directory=True)
        require(ledger.parent != output and output not in ledger.parents,
                "diagnostic_ledger_inside_output")
        require(not os.path.lexists(ledger), "diagnostic_registration_already_started")
        return Registration(encoded, profile)
    except (OSError, KeyError, TypeError, ValueError, RecursionError) as error:
        raise DiagnosticFailure("diagnostic_registration_invalid") from error


def source_identities(root: Path, base_sha: str, candidate_sha: str) -> dict[str, Any]:
    """Freeze actual domain source and schema, not caller-supplied version labels."""
    require(all(isinstance(sha, str) and re.fullmatch(r"[0-9a-f]{40}", sha)
                for sha in (base_sha, candidate_sha)), "diagnostic_source_identity_invalid")
    prompts = {
        "canon": ("services/novel-service/src/domain/services/canon_story_extractor.rs",
                  "CANON_EXTRACTION_PROMPT_VERSION"),
        "branch": ("services/narrative-service/src/domain/services/narrative_transition.rs",
                   "TRANSITION_PROMPT_VERSION"),
        "world": ("services/narrative-service/src/domain/entities/world_session.rs",
                  "WORLD_TURN_PROMPT_VERSION"),
    }
    trees = ["infra/postgres", "crates/llm-client/src"] + [
        "services/" + service + "/src/domain"
        for service in ("novel-service", "agent-service", "narrative-service")
    ]
    result = {}
    for label, sha in (("base", base_sha), ("candidate", candidate_sha)):
        versions = {}
        for name, (path, constant) in prompts.items():
            raw = bounded_command(["git", "-C", str(root), "show", f"{sha}:{path}"])
            matches = re.findall(rb'^pub const ' + constant.encode() + rb': &str = "([a-z0-9+_-]+)";$',
                                 raw, re.MULTILINE)
            require(len(matches) == 1, "diagnostic_prompt_source_invalid")
            versions[name] = matches[0].decode()
        identities = {}
        for path in trees:
            object_id = bounded_command(["git", "-C", str(root), "rev-parse", f"{sha}:{path}"]).decode().strip()
            require(bool(re.fullmatch(r"[0-9a-f]{40}", object_id)), "diagnostic_source_identity_invalid")
            identities[path] = object_id
        result[label] = {"prompt_versions": versions, "source_trees": identities}
    return result


def affected_application_images(paths: list[str]) -> set[str]:
    """Known inputs to the two release Dockerfiles, not a build dependency graph.

    Eligibility is conservative; the corresponding image must still change,
    and prospective review must establish a useful change rather than churn.
    """
    rust_images = APP_KEYS - {"FRONTEND_IMAGE"}
    affected = set()
    frontend_inputs = {
        "Dockerfile", ".dockerignore", "package.json", "pnpm-lock.yaml",
        "pnpm-workspace.yaml", "vite.config.ts", "postcss.config.js",
        "tsconfig.json", "tsconfig.node.json", "index.html", "nginx-spa.conf",
    }
    for path in paths:
        if path in {"Cargo.toml", "Cargo.lock", ".dockerignore",
                    "infra/docker/Dockerfile.rust-service"} or re.fullmatch(
                        r"crates/[^/]+/(?:src/.*|Cargo\.toml|build\.rs)", path
                    ):
            affected.update(rust_images)
        if path == PROFILE_PATH.as_posix():
            # Compiled by llm-client and user-service; ordinary tools are not inputs.
            # Existing registration/profile and all payer capability checks still apply.
            affected.update(rust_images - {"GATEWAY_IMAGE"})
        for service in ("gateway", "user-service", "novel-service", "agent-service", "narrative-service"):
            prefix = "gateway/" if service == "gateway" else f"services/{service}/"
            if path.startswith(prefix + "src/") or path in {prefix + "Cargo.toml", prefix + "build.rs"}:
                affected.add(service.upper().replace("-", "_") + "_IMAGE")
        if path.startswith(("frontend/src/", "frontend/public/")) or (
            path.startswith("frontend/") and path.removeprefix("frontend/") in frontend_inputs
        ):
            affected.add("FRONTEND_IMAGE")
    return affected


def verify_artifacts(registration: Registration, root: Path,
                     base: dict[str, str], candidate: dict[str, str]) -> None:
    """Read-only, locally pre-pulled artifact checks before protected config read.

    Changed application inputs and filesystem layers are necessary, not a semantic
    proof of a meaningful version change; prospective review still owns that.
    """
    value = registration.value
    base_sha, candidate_sha = base["RELEASE_GIT_SHA"], candidate["RELEASE_GIT_SHA"]
    require(base_sha != candidate_sha and candidate_sha == value["candidate_git_sha"],
            "diagnostic_release_identity_invalid")
    bounded_command(["git", "-C", str(root), "merge-base", "--is-ancestor", base_sha, candidate_sha])
    changed_paths = bounded_command([
        "git", "-C", str(root), "diff", "--no-renames", "--name-only", "-z",
        base_sha, candidate_sha, "--",
    ])
    affected = affected_application_images([
        os.fsdecode(path) for path in changed_paths.split(b"\0") if path
    ])
    require(bool(affected), "diagnostic_application_source_unchanged")
    observed = {}
    for label, manifest in (("base", base), ("candidate", candidate)):
        observed[label] = {}
        for key in sorted(APP_KEYS):
            reference = manifest[key]
            require(isinstance(reference, str) and bool(re.fullmatch(
                r"[a-z0-9][a-z0-9._/:@-]*@sha256:[0-9a-f]{64}", reference)),
                "diagnostic_artifact_identity_invalid")
            images = strict_json(bounded_command(["docker", "image", "inspect", reference]))
            require(isinstance(images, list) and len(images) == 1 and isinstance(images[0], dict),
                    "diagnostic_artifact_identity_invalid")
            image = images[0]
            require(image.get("Id") == value[label + "_application_image_ids"][key]
                    and reference in (image.get("RepoDigests") or []), "diagnostic_artifact_identity_mismatch")
            layers = (image.get("RootFS") or {}).get("Layers")
            require(isinstance(layers, list) and bool(layers) and all(
                isinstance(layer, str) and re.fullmatch(r"sha256:[0-9a-f]{64}", layer)
                for layer in layers), "diagnostic_artifact_layers_invalid")
            observed[label][key] = layers
    require(any(base[key] != candidate[key]
                and value["base_application_image_ids"][key] != value["candidate_application_image_ids"][key]
                and observed["base"][key] != observed["candidate"][key]
                for key in affected), "diagnostic_application_content_unchanged")


class DiagnosticLedger:
    """One exclusive durable Started; existing/crash residue is never resumed."""

    def __init__(self, registration: Registration):
        self.registration = registration
        self.descriptor: int | None = None

    def _append(self, status: str, codes: list[str]) -> None:
        require(self.descriptor is not None, "diagnostic_ledger_not_started")
        require(status in ("Started", "Passed", "Failed") and all(
            isinstance(code, str) and re.fullmatch(r"[a-z][a-z0-9_]{0,100}", code) for code in codes),
            "diagnostic_failure_code_invalid")
        record = {"schema": LEDGER_SCHEMA, "evidence_class": "Diagnostic",
                  "registration_sha256": self.registration.sha256,
                  "status": status, "at": datetime.now(timezone.utc).isoformat(),
                  "failure_codes": codes}
        if status == "Started":
            record["registration"] = self.registration.value
        remaining = canonical(record) + b"\n"
        while remaining:
            size = os.write(self.descriptor, remaining)
            require(size > 0, "diagnostic_ledger_write_failed")
            remaining = remaining[size:]
        os.fsync(self.descriptor)

    def start(self) -> None:
        require(self.descriptor is None, "diagnostic_registration_already_started")
        path = Path(self.registration.value["ledger_path"])
        try:
            self.descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL
                                      | os.O_NOFOLLOW | os.O_APPEND, 0o600)
        except OSError as error:
            raise DiagnosticFailure("diagnostic_registration_already_started") from error
        try:
            self._append("Started", [])
            sync_directory(path.parent)
        except BaseException:
            self.close()
            raise

    def finish(self, passed: bool, codes: list[str]) -> None:
        try:
            require(type(passed) is bool and (not codes if passed else bool(codes)),
                    "diagnostic_terminal_status_invalid")
            self._append("Passed" if passed else "Failed", codes)
        finally:
            self.close()

    def close(self) -> None:
        if self.descriptor is not None:
            descriptor, self.descriptor = self.descriptor, None
            os.close(descriptor)


def reconcile_snapshot(registration: Registration, snapshot: Any,
                       previous: Any = None) -> dict[str, Any]:
    """Check one consistent PG budget/receipt snapshot, not process metrics."""
    require(isinstance(snapshot, dict) and set(snapshot) == {"budget", "receipts"},
            "diagnostic_snapshot_invalid")
    budget, rows = snapshot["budget"], snapshot["receipts"]
    require(isinstance(budget, dict) and isinstance(rows, list) and len(rows) <= 2000,
            "diagnostic_snapshot_invalid")
    value, profile = registration.value, registration.profile
    expected = {**registration.binding,
                **{"max_" + key: value["limits"]["max_" + key] for key in profile["max_limits"]},
                "expires_at": value["limits"]["expires_at"]}
    require(all(budget.get(key) == item and type(budget.get(key)) is type(item)
                for key, item in expected.items()) and type(budget.get("sealed")) is bool,
            "diagnostic_budget_identity_changed")
    require(all(integer(budget.get("charged_" + key), maximum)
                for key, maximum in profile["max_limits"].items()), "diagnostic_charges_invalid")
    require(len(rows) == budget["charged_attempts"], "diagnostic_receipt_count_mismatch")
    totals = {"attempts": len(rows), "tokens": 0, "cost_micro_cny": 0}
    settled = 0
    indexed = {}
    for ordinal, row in enumerate(rows, 1):
        require(isinstance(row, dict), "diagnostic_receipt_invalid")
        attempt = row.get("attempt_id")
        operation = row.get("operation")
        limit = row.get("output_limit")
        require(row.get("budget_id") == value["budget_id"] and uuid4(attempt)
                and attempt not in indexed and type(row.get("ordinal")) is int
                and row["ordinal"] == ordinal and isinstance(operation, str)
                and operation in profile["operations"]
                and integer(limit, profile["operations"][operation], 1)
                and type(row.get("settled")) is bool, "diagnostic_receipt_invalid")
        tokens = profile["input_token_ceiling"] + limit
        cost = profile["input_token_ceiling"] * profile["input_micro_cny"] + limit * profile["output_micro_cny"]
        require(type(row.get("reservation_tokens")) is int and row["reservation_tokens"] == tokens
                and type(row.get("reservation_cost_micro_cny")) is int
                and row["reservation_cost_micro_cny"] == cost, "diagnostic_reservation_mismatch")
        fields = ("settlement_model", "input_tokens", "output_tokens", "cached_input_tokens")
        require(all(key in row for key in fields), "diagnostic_settlement_invalid")
        if row["settled"]:
            require(row["settlement_model"] == MODEL
                    and integer(row["input_tokens"], profile["input_token_ceiling"])
                    and integer(row["output_tokens"], limit)
                    and (row["cached_input_tokens"] is None
                         or integer(row["cached_input_tokens"], row["input_tokens"])),
                    "diagnostic_settlement_invalid")
            tokens = row["input_tokens"] + row["output_tokens"]
            cost = row["input_tokens"] * profile["input_micro_cny"] + row["output_tokens"] * profile["output_micro_cny"]
            settled += 1
        else:
            require(all(row[key] is None for key in fields), "diagnostic_settlement_invalid")
        totals["tokens"] += tokens
        totals["cost_micro_cny"] += cost
        indexed[attempt] = row
    require(all(totals[key] == budget["charged_" + key]
                and totals[key] <= value["limits"]["max_" + key] for key in totals),
            "diagnostic_accounting_mismatch")
    if previous is not None:
        reconcile_snapshot(registration, previous)
        require(not previous["budget"]["sealed"] or budget["sealed"], "diagnostic_budget_reopened")
        if previous["budget"]["sealed"]:
            require(set(indexed) == {row["attempt_id"] for row in previous["receipts"]},
                    "diagnostic_reservation_after_seal")
        for old in previous["receipts"]:
            current = indexed.get(old["attempt_id"])
            require(current is not None, "diagnostic_receipt_disappeared")
            immutable = ("budget_id", "attempt_id", "ordinal", "operation", "output_limit",
                         "reservation_tokens", "reservation_cost_micro_cny")
            require(all(old[key] == current[key] for key in immutable), "diagnostic_receipt_changed")
            if old["settled"]:
                require(current == old, "diagnostic_settlement_changed")
    return {"charged": totals, "settled_attempts": settled,
            "unresolved_attempts": len(rows) - settled, "sealed": budget["sealed"]}


def reconcile_metrics(registration: Registration, snapshot: Any, summary: Any) -> None:
    """Compare settled receipts with all selected process-generation counters.

    Usage-report counts are logical-call counters, not receipt counts: empty-JSON
    retries record additional token usage without another usage-report sample.
    """
    aggregate = reconcile_snapshot(registration, snapshot)
    require(aggregate["sealed"] and not aggregate["unresolved_attempts"],
            "diagnostic_metrics_receipts_unresolved")
    require(isinstance(summary, dict) and isinstance(summary.get("counter_totals"), list),
            "diagnostic_metrics_missing")
    expected, observed = {}, {}

    def add(totals, operation, counter, value):
        if value:
            key = operation, counter
            totals[key] = totals.get(key, 0) + value

    for row in snapshot["receipts"]:
        cached = row["cached_input_tokens"] or 0
        values = {"attempts": 1, "tokens.input": row["input_tokens"],
                  "tokens.output": row["output_tokens"], "tokens.cached_input": cached,
                  "billable_tokens.cached_input": cached,
                  "billable_tokens.uncached_input": row["input_tokens"] - cached,
                  "billable_tokens.output": row["output_tokens"]}
        for counter, value in values.items():
            add(expected, row["operation"], counter, value)
    for item in summary["counter_totals"]:
        counter = item.get("counter", "")
        if not counter.startswith(("attempts.", "tokens.", "billable_tokens.")):
            continue
        value = item.get("value")
        require(item.get("provider_model") == "deepseek/" + MODEL
                and item.get("operation") in registration.profile["operations"]
                and type(value) in (int, float) and math.isfinite(value)
                and value >= 0 and int(value) == value,
                "diagnostic_metrics_invalid")
        add(observed, item["operation"], "attempts" if counter.startswith("attempts.") else counter,
            int(value))
    require(observed == expected, "diagnostic_metrics_receipt_mismatch")


def bounded_command(command: list[str], *, stdin: bytes = b"", timeout: float = 10,
                    maximum: int = 4 * 1024 * 1024) -> bytes:
    """Bound private control/PG output while reading; no stderr or command leaks."""
    require(len(stdin) <= 4096 and 0 < timeout <= 10 and 0 < maximum <= 4 * 1024 * 1024,
            "diagnostic_command_bounds_invalid")
    started = time.monotonic()
    io_deadline = started + timeout * 0.9
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, start_new_session=True)
    output = bytearray()
    try:
        # At most PIPE_BUF bytes into a fresh empty pipe; no unbounded input writer.
        if stdin:
            process.stdin.write(stdin)
        process.stdin.close()
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while True:
                remaining = io_deadline - time.monotonic()
                require(remaining > 0 and bool(selector.select(max(0, remaining))),
                        "diagnostic_command_timeout")
                chunk = os.read(process.stdout.fileno(), min(65536, maximum + 1 - len(output)))
                if not chunk:
                    remaining = io_deadline - time.monotonic()
                    require(remaining > 0 and process.wait(timeout=remaining) == 0,
                            "diagnostic_command_failed")
                    return bytes(output)
                output.extend(chunk)
                require(len(output) <= maximum, "diagnostic_command_output_oversized")
    except (OSError, subprocess.SubprocessError) as error:
        raise DiagnosticFailure("diagnostic_command_failed") from error
    finally:
        # Kill descendants too if a CLI exited but a child retained the output pipe.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=max(0.001, started + timeout - time.monotonic()))
        except subprocess.TimeoutExpired as error:
            raise DiagnosticFailure("diagnostic_command_stop_unproven") from error
        finally:
            if not process.stdin.closed:
                process.stdin.close()
            process.stdout.close()


def snapshot_command(prefix: str, budget_id: str) -> tuple[list[str], bytes]:
    require(bool(re.fullmatch(r"nwq-[a-f0-9]{10}", prefix)) and uuid4(budget_id),
            "diagnostic_snapshot_target_invalid")
    # psql quotes the UUID variable as a SQL literal; never interpolate SQL input.
    sql = b'''BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL lock_timeout = '2s';
SET LOCAL statement_timeout = '5s';
SET LOCAL timezone = 'UTC';
SELECT json_build_object(
  'budget', (SELECT json_build_object(
    'budget_id', b.budget_id, 'contract', b.contract, 'profile', b.profile,
    'profile_sha256', b.profile_sha256,
    'max_attempts', b.max_attempts, 'max_tokens', b.max_tokens,
    'max_cost_micro_cny', b.max_cost_micro_cny,
    'charged_attempts', b.charged_attempts, 'charged_tokens', b.charged_tokens,
    'charged_cost_micro_cny', b.charged_cost_micro_cny,
    'expires_at', to_char(b.expires_at, 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
    'sealed', b.sealed)
    FROM public.diagnostic_llm_budgets b WHERE b.budget_id = :'budget_id'::uuid),
  'receipts', (SELECT coalesce(json_agg(row_to_json(a) ORDER BY a.ordinal), '[]'::json)
    FROM (SELECT budget_id, attempt_id, ordinal, operation, output_limit,
      reservation_tokens, reservation_cost_micro_cny, settled, settlement_model,
      input_tokens, output_tokens, cached_input_tokens
      FROM public.diagnostic_llm_attempts WHERE budget_id = :'budget_id'::uuid
      ORDER BY ordinal LIMIT 2001) a));
COMMIT;
'''
    return (["docker", "exec", "-i", prefix + "-postgres", "psql", "-X", "-qAt",
             "-U", "novel", "-d", "novel_world", "-v", "ON_ERROR_STOP=1",
             "-v", "budget_id=" + budget_id], sql)
