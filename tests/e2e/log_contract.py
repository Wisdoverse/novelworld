#!/usr/bin/env python3
"""SPEC 14.1 log-contract checker.

Every line of every service's stdout must be structured JSON carrying
timestamp, level, message, and the service span fields (the service name and a
trace_id, empty outside a request). Every service must also have logged at
least one request-scoped line with a propagated non-empty trace id, proving
the X-Trace-Id header reaches request handlers.
"""
import json
import subprocess
import sys

SERVICES = ["gateway", "user-service", "novel-service", "agent-service", "narrative-service"]


def find_entry(obj, key):
    """Return (present, value) searching nested dicts/lists."""
    if isinstance(obj, dict):
        for entry_key, entry_value in obj.items():
            if entry_key == key:
                return True, entry_value
            present, value = find_entry(entry_value, key)
            if present:
                return True, value
    elif isinstance(obj, list):
        for item in obj:
            present, value = find_entry(item, key)
            if present:
                return True, value
    return False, None


def main():
    problems = []
    for service in SERVICES:
        logs = subprocess.run(
            ["docker", "logs", f"novel-{service}"], capture_output=True, text=True
        ).stdout
        lines = [line for line in logs.splitlines() if line.strip()]
        if not lines:
            problems.append(f"{service}: no log lines")
            continue
        non_null_trace_ids = 0
        for line in lines:
            try:
                entry = json.loads(line)
            except ValueError:
                problems.append(f"{service}: non-JSON log line: {line[:80]}")
                continue
            for required in ("timestamp", "level"):
                if required not in entry:
                    problems.append(f"{service}: missing {required}")
                    break
            else:
                message_present, _ = find_entry(entry, "message")
                if not message_present:
                    problems.append(f"{service}: missing message field")
                    continue
                service_present, service_value = find_entry(entry, "service")
                if not service_present or service_value != service:
                    problems.append(f"{service}: missing or wrong service field")
                    continue
                trace_present, trace_value = find_entry(entry, "trace_id")
                if not trace_present:
                    problems.append(f"{service}: missing trace_id field")
                    continue
                if trace_value:
                    non_null_trace_ids += 1
        else:
            if non_null_trace_ids == 0:
                problems.append(
                    f"{service}: no request-scoped line with a propagated trace id"
                )
    if problems:
        print("log contract failed:\n  " + "\n  ".join(problems))
        sys.exit(1)
    print("log contract verified for " + ", ".join(SERVICES))


if __name__ == "__main__":
    main()
