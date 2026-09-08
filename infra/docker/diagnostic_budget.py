"""Private release adapter for the fixed diagnostic budget; no provider I/O.

release.sh captures this source and the one profile before any git checkout.
Compose JSON (including credentials) is consumed only in memory, never reported.
"""
import datetime
import hashlib
import json
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


class ProbeInvalid(Invalid):
    """Closed, bounded evidence for a capability command refusal."""

    def __init__(self, reason, *, exit_code=None, elapsed=None, container_id=None):
        super().__init__()
        self.reason = reason if reason in PROBE_REASONS else "unknown"
        self.exit_code = exit_code if isinstance(exit_code, int) and -128 <= exit_code <= 255 else None
        self.elapsed = round(min(max(float(elapsed or 0), 0), 10), 3)
        self.container_id = container_id if isinstance(container_id, str) and re.fullmatch(r"[0-9a-f]{64}", container_id) else None


def _probe_detail(error):
    if isinstance(error, ProbeInvalid):
        return {"reason": error.reason, "exit_code": error.exit_code,
                "elapsed": error.elapsed, "container_id": error.container_id}
    return None


def _closed_detail(value):
    if not isinstance(value, dict):
        return {"reason": "unknown", "exit_code": None, "elapsed": 0, "container_id": None}
    reason = value.get("reason") if value.get("reason") in PROBE_REASONS else "unknown"
    exit_code = value.get("exit_code")
    if not isinstance(exit_code, int) or not -128 <= exit_code <= 255:
        exit_code = None
    elapsed = value.get("elapsed", 0)
    if not isinstance(elapsed, (int, float)) or not 0 <= elapsed <= 10:
        elapsed = 0
    container_id = value.get("container_id")
    if not isinstance(container_id, str) or not re.fullmatch(r"[0-9a-f]{64}", container_id):
        container_id = None
    return {"reason": reason, "exit_code": exit_code, "elapsed": round(elapsed, 3),
            "container_id": container_id}


def probe_evidence(error):
    """Return only closed fields suitable for the private journey report."""
    primary = getattr(error, "primary_evidence", None)
    cleanup = getattr(error, "cleanup_evidence", None)
    phase = getattr(error, "probe_phase", None)
    name = getattr(error, "probe_name", None)
    container_id = getattr(error, "probe_container_id", None)
    phase = phase if phase in PROBE_PHASES else "unknown"
    name = name if isinstance(name, str) and re.fullmatch(r"nwq-[a-f0-9]{10}-budget-probe-[a-f0-9]{12}", name) else None
    container_id = container_id if isinstance(container_id, str) and re.fullmatch(r"[0-9a-f]{64}", container_id) else None
    if isinstance(error, ProbeInvalid):
        primary = _probe_detail(error)
        container_id = container_id or primary["container_id"]
        return {"outcome": "invalid", "phase": phase, "name": name,
                "container_id": container_id, "primary": primary}
    if isinstance(error, OSError):
        return {"outcome": "cleanup_uncertain", "phase": phase, "name": name,
                "container_id": container_id, "primary": _closed_detail(primary),
                "cleanup": _closed_detail(cleanup)}
    return {"outcome": "unknown"}


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
    finally:
        if process.poll() is None:
            process.kill()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired as error:
            failure = OSError("diagnostic probe process reap unproven")
            active = sys.exc_info()[1]
            failure.primary_evidence = (_probe_detail(active)
                                        if isinstance(active, ProbeInvalid)
                                        else {"reason": "deadline", "exit_code": None,
                                              "elapsed": 0, "container_id": None})
            failure.cleanup_evidence = {"reason": "reap_timeout", "exit_code": None,
                                        "elapsed": 5, "container_id": None}
            raise failure from error
        except OSError as error:
            failure = OSError("diagnostic probe process reap unproven")
            active = sys.exc_info()[1]
            failure.primary_evidence = (_probe_detail(active)
                                        if isinstance(active, ProbeInvalid)
                                        else {"reason": "wait_error", "exit_code": None,
                                              "elapsed": 0, "container_id": None})
            failure.cleanup_evidence = {"reason": "reap_error", "exit_code": None,
                                        "elapsed": 0, "container_id": None}
            raise failure from error
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
            raise ProbeInvalid("deadline", container_id=acknowledged_id)
        phase = "probe_start"
        try:
            raw = bounded_output(["docker", "start", "--attach", name], remaining)
        except ProbeInvalid as error:
            error.container_id = acknowledged_id
            raise
        phase = "probe_expectation"
        try:
            observed = strict_json(raw)
        except (Invalid, ValueError, TypeError, KeyError):
            raise ProbeInvalid("invalid_capability")
        if observed != expected:
            raise ProbeInvalid("invalid_capability")
    except (Invalid, ValueError, OSError, subprocess.SubprocessError) as error:
        primary_error = error
        error.probe_phase = phase
        error.probe_name = name
        error.probe_container_id = acknowledged_id
        print("diagnostic phase=" + phase, file=sys.stderr)
        if isinstance(error, ProbeInvalid):
            print("diagnostic reason=" + error.reason, file=sys.stderr)
        raise
    finally:
        # Exact random isolated name only. No volumes, credentials or user deployment involved.
        try:
            phase = "probe_cleanup_rm"
            cleanup = subprocess.run(["docker", "rm", "--force", name], stdin=subprocess.DEVNULL,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
            phase = "probe_cleanup_ps"
            remaining = bounded_output(["docker", "ps", "--all", "--quiet", "--filter", "name=^/" + name + "$"], 5)
        except (Invalid, OSError, subprocess.SubprocessError) as error:
            print("diagnostic phase=" + phase, file=sys.stderr)
            failure = OSError("diagnostic probe cleanup unproven")
            failure.primary_evidence = _probe_detail(primary_error) if isinstance(primary_error, ProbeInvalid) else None
            query_reason = "cleanup_query_deadline" if isinstance(error, ProbeInvalid) and error.reason == "deadline" else "cleanup_query_error"
            failure.cleanup_evidence = {"reason": "cleanup_timeout" if isinstance(error, subprocess.TimeoutExpired)
                                        else query_reason, "exit_code": getattr(error, "exit_code", None),
                                        "elapsed": getattr(error, "elapsed", 5),
                                        "container_id": acknowledged_id}
            failure.probe_phase = phase
            failure.probe_name = name
            failure.probe_container_id = acknowledged_id
            raise failure from error
        if not created or cleanup.returncode != 0 or remaining.strip():
            # Distinguish uncertain lifecycle cleanup from expected capability
            # refusal. Callers must not count it as a passing negative probe.
            print("diagnostic phase=probe_cleanup_unproven", file=sys.stderr)
            failure = OSError("diagnostic probe cleanup unproven")
            failure.primary_evidence = (_probe_detail(primary_error)
                                        if isinstance(primary_error, ProbeInvalid)
                                        else probe_evidence(primary_error)) if primary_error is not None else None
            failure.cleanup_evidence = {"reason": "create_unconfirmed" if not created
                                        else "cleanup_nonzero" if cleanup.returncode != 0
                                        else "cleanup_residue", "exit_code": cleanup.returncode,
                                        "elapsed": 0, "container_id": acknowledged_id}
            failure.probe_phase = "probe_cleanup_unproven"
            failure.probe_name = name
            failure.probe_container_id = acknowledged_id
            raise failure


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
