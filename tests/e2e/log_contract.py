#!/usr/bin/env python3
"""SPEC 14.1 log-contract checker.

Contract scope: lines whose target belongs to one of the five services must be
structured JSON carrying timestamp, level, message, and the service span fields
(service name and a trace_id, empty outside a request). Foreign-crate lines
(e.g. hyper internals) are out of contract scope and skipped. Every service
must also have logged at least one request-scoped line with a propagated
non-empty trace id, and the checker itself proves propagation end to end: it
sends a request stamped with a known X-Trace-Id and asserts the downstream
service logs that exact id.
"""
import json
import subprocess
import sys
import time
import uuid

SERVICES = ["gateway", "user-service", "novel-service", "agent-service", "narrative-service"]
SERVICE_TARGETS = ["gateway", "user_service", "novel_service", "agent_service", "narrative_service"]
API = "http://127.0.0.1/api"


def find_entries(obj, key):
    """Return every value found under key, nested anywhere."""
    values = []
    if isinstance(obj, dict):
        for entry_key, entry_value in obj.items():
            if entry_key == key:
                values.append(entry_value)
            values.extend(find_entries(entry_value, key))
    elif isinstance(obj, list):
        for item in obj:
            values.extend(find_entries(item, key))
    return values


def logs_of(service):
    return subprocess.run(
        ["docker", "logs", f"novel-{service}"], capture_output=True, text=True
    ).stdout


def main():
    api = sys.argv[1] if len(sys.argv) > 1 else API
    problems = []

    # Prove propagation end to end: a setup-status request stamped with a
    # known X-Trace-Id must surface in the user-service setup log with that
    # exact id (the public path needs no credentials).
    probe_id = f"log-contract-{uuid.uuid4()}"
    for attempt in range(5):
        subprocess.run(
            [
                "curl", "--silent", "--show-error", "--output", "/dev/null",
                "-H", f"X-Trace-Id: {probe_id}",
                f"{api}/setup/status",
            ],
            check=False,
        )
        deadline = time.time() + 10
        propagated = False
        while time.time() < deadline and not propagated:
            time.sleep(0.5)
            propagated = probe_id in logs_of("user-service")
        if propagated:
            break
        time.sleep(1.2)  # rate limiter spacing
    if not propagated:
        problems.append("propagation: the stamped X-Trace-Id never reached user-service logs")

    for service in SERVICES:
        lines = [line for line in logs_of(service).splitlines() if line.strip()]
        if not lines:
            problems.append(f"{service}: no log lines")
            continue
        non_empty_trace_ids = 0
        for line in lines:
            try:
                entry = json.loads(line)
            except ValueError:
                problems.append(f"{service}: non-JSON log line: {line[:80]}")
                continue
            target = entry.get("target", "")
            if not any(target == prefix or target.startswith(prefix + "::") for prefix in SERVICE_TARGETS):
                continue  # foreign-crate line, out of contract scope
            for required in ("timestamp", "level"):
                if required not in entry:
                    problems.append(f"{service}: missing {required}")
                    break
            else:
                messages = find_entries(entry, "message")
                if not messages:
                    problems.append(f"{service}: missing message field")
                    continue
                service_values = [v for v in find_entries(entry, "service") if v]
                if service not in service_values:
                    problems.append(f"{service}: missing or wrong service field")
                    continue
                trace_ids = find_entries(entry, "trace_id")
                if not trace_ids:
                    problems.append(f"{service}: missing trace_id field")
                    continue
                if any(value for value in trace_ids):
                    non_empty_trace_ids += 1
        else:
            if non_empty_trace_ids == 0:
                problems.append(
                    f"{service}: no request-scoped line with a propagated trace id"
                )
    if problems:
        print("log contract failed:\n  " + "\n  ".join(problems))
        sys.exit(1)
    print("log contract verified for " + ", ".join(SERVICES))


if __name__ == "__main__":
    main()
