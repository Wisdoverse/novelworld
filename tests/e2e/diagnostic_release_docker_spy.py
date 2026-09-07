#!/usr/bin/env python3
"""Fail-closed Docker argv observer for the diagnostic release tests."""

import json
import os
from pathlib import Path
import re
import sys


def _fail(message):
    raise SystemExit(message)


def _environment():
    real = os.environ.get("NWQ_REAL_DOCKER", "")
    trace = os.environ.get("NWQ_DOCKER_TRACE", "")
    project = os.environ.get("NWQ_PROJECT", "")
    if not Path(real).is_absolute():
        _fail("NWQ_REAL_DOCKER must be an absolute path")
    if not trace:
        _fail("NWQ_DOCKER_TRACE is required")
    if not re.fullmatch(r"nwq-[0-9a-f]{10}", project):
        _fail("invalid NWQ_PROJECT")
    return real, Path(trace), project


def _probe_name(value, project):
    return bool(re.fullmatch(re.escape(project) + r"-budget-probe-[0-9a-f]{12}", value))


def _compose_allowed(argv, project):
    if not argv or argv[0] != "compose":
        return False
    try:
        command = next(index for index, token in enumerate(argv[1:], 1)
                       if token in ("config", "pull"))
    except StopIteration:
        return False
    options = argv[1:command]
    index = 0
    while index < len(options):
        token = options[index]
        if token == "--project-name":
            if index + 1 >= len(options) or options[index + 1] != project:
                return False
        elif token in ("--project-directory", "-f", "--env-file"):
            if index + 1 >= len(options) or not Path(options[index + 1]).is_absolute():
                return False
        elif token == "--profile":
            if index + 1 >= len(options) or options[index + 1] != "redis":
                return False
        else:
            return False
        index += 2
    if "--project-name" not in options:
        return False
    if argv[command] == "config":
        return argv[command + 1:] == ["--format", "json"]
    return bool(argv[command + 1:]) and all(
        service in {"postgres-migrate", "nginx", "frontend", "user-service",
                    "novel-service", "agent-service", "narrative-service", "gateway"}
        for service in argv[command + 1:]
    )


def _probe_allowed(argv, project):
    if len(argv) != 19 or argv[:4] != ["create", "--pull", "never", "--name"]:
        return False
    name = argv[4]
    if not _probe_name(name, project):
        return False
    expected = ["--network", "none", "--no-healthcheck", "--log-driver", "none",
                "--read-only", "--cap-drop", "ALL", "--security-opt",
                "no-new-privileges", "--entrypoint", "/app/service"]
    if argv[5:17] != expected:
        return False
    return bool(re.fullmatch(r"[a-z0-9][a-z0-9._/:@-]*@sha256:[0-9a-f]{64}", argv[17])) \
        and argv[18] == "--diagnostic-budget-contract"


def _start_allowed(argv, project):
    return len(argv) == 3 and argv[:2] == ["start", "--attach"] \
        and _probe_name(argv[2], project)


def _cleanup_allowed(argv, project):
    if len(argv) == 3 and argv[0:2] == ["rm", "--force"]:
        return _probe_name(argv[2], project)
    return (len(argv) == 5 and argv[:4] == ["ps", "--all", "--quiet", "--filter"]
            and argv[4].startswith("name=^/")
            and argv[4].endswith("$")
            and _probe_name(argv[4][len("name=^/"):-1], project))


def main(argv=None):
    real, trace, project = _environment()
    argv = list(sys.argv[1:] if argv is None else argv)
    trace.parent.mkdir(parents=True, exist_ok=True)
    with trace.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(argv, separators=(",", ":")) + "\n")
    if not (_compose_allowed(argv, project) or _probe_allowed(argv, project)
            or _start_allowed(argv, project)
            or _cleanup_allowed(argv, project)):
        _fail("unexpected Docker invocation")
    os.execv(real, [real, *argv])


if __name__ == "__main__":
    main()
