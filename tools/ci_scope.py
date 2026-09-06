#!/usr/bin/env python3
"""Select affected CI jobs; unknown paths or unavailable history run everything."""

import json
import os
from pathlib import Path
import subprocess


SCOPES = ("backend", "frontend", "desktop", "integration", "runtime", "launchers")


def select_scopes(paths):
    selected = set()
    for path in paths:
        if (path.startswith("docs/") and Path(path).suffix in {".md", ".png", ".svg", ".jpg"}) or (
            "/" not in path and path.endswith(".md")
        ) or path.startswith((".github/ISSUE_TEMPLATE/", ".github/pull_request_template")):
            continue
        if path.startswith("frontend/src-tauri/"):
            # The backend job also audits the separate Tauri lockfile.
            selected.update(("backend", "desktop"))
        elif path.startswith("frontend/"):
            selected.update(("frontend", "desktop", "runtime"))
        elif path.startswith("infra/postgres/"):
            # Desktop embeds the schema and migrations with include_str!.
            selected.update(("backend", "integration", "desktop", "runtime"))
        elif path.startswith(("gateway/", "services/", "crates/", "tests/integration/")):
            selected.update(("backend", "integration", "runtime"))
        elif path.startswith("tools/desktop/"):
            selected.add("desktop")
        elif path.startswith(("tools/h1-eval/", "tools/h3-eval/", "tools/architecture/", "tools/llm-budget/", "tools/capacity/", "tests/e2e/")):
            selected.update(("backend", "runtime"))
        elif path in {"start.sh", "start.ps1", "start.cmd"}:
            selected.add("launchers")
        else:
            # Includes workflows, shared build/config files and new directories.
            return set(SCOPES)
    return selected


def changed_paths(event, event_name):
    if event_name == "pull_request":
        base = event.get("pull_request", {}).get("base", {}).get("sha")
        head = event.get("pull_request", {}).get("head", {}).get("sha")
        separator = "..."
    elif event_name == "push" and event.get("ref", "").startswith("refs/heads/"):
        base, head = event.get("before"), event.get("after")
        separator = ".."
    else:
        return None
    if not all(isinstance(sha, str) and len(sha) == 40 and
               all(c in "0123456789abcdef" for c in sha) and sha != "0" * 40
               for sha in (base, head)):
        return None
    try:
        output = subprocess.check_output(
            ["git", "diff", "--no-renames", "--name-only", "-z", f"{base}{separator}{head}", "--"],
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError:
        return None
    return [os.fsdecode(path) for path in output.split(b"\0") if path]


def main():
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
    # Reusable release callers can explicitly force full validation even when
    # GitHub exposes the caller's event rather than workflow_call.
    paths = None if os.environ.get("CI_FULL") == "true" else changed_paths(event, os.environ["GITHUB_EVENT_NAME"])
    selected = set(SCOPES) if paths is None else select_scopes(paths)
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        for scope in SCOPES:
            print(f"{scope}={str(scope in selected).lower()}", file=output)
    print("CI scope:", ", ".join(sorted(selected)) or "documentation/policy checks only")


if __name__ == "__main__":
    main()
