#!/usr/bin/env python3
import io
import json
import os
import subprocess
import sys
import tempfile
import textwrap
from contextlib import chdir, contextmanager, redirect_stdout
from pathlib import Path
from unittest.mock import patch
try:
    import ci_scope as scope
except ModuleNotFoundError:
    import tools.ci_scope as scope
ALL = set(scope.SCOPES)
JOBS = (
    "changes", "unix-launcher", "windows-launcher", "desktop-contract",
    "backend", "frontend", "frontend-a11y-browser", "integration", "runtime",
)

@contextmanager
def repo():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)

        def git(*args):
            return subprocess.run(["git", *args], cwd=root, check=True,
                                  capture_output=True, text=True)

        git("init", "-q")
        git("config", "user.email", "ci-scope-test@example.invalid")
        git("config", "user.name", "CI Scope Test")
        yield root, git

def commit(git, message):
    git("add", "--all")
    git("commit", "-qm", message)
    return git("rev-parse", "HEAD").stdout.strip()

def test_scopes():
    cases = [
        (["README.md", "docs/PRODUCT_CONTRACT.md"], set()),
        (["services/novel-service/src/lib.rs"], {"backend", "integration", "runtime"}),
        (["frontend/src/app.tsx"], {"frontend", "desktop", "runtime"}),
        (["infra/postgres/migrations/0001.sql"], {"backend", "integration", "desktop", "runtime"}),
        (["start.ps1"], {"launchers"}),
        (["unknown/new-file.txt"], ALL),
        ([".github/workflows/ci.yml"], ALL),
        (["services/a.rs", "frontend/src/app.tsx", "docs/README.md"],
         {"backend", "integration", "runtime", "frontend", "desktop"}),
    ]
    for paths, expected in cases:
        assert scope.select_scopes(paths) == expected, (paths, scope.select_scopes(paths))

def test_pr_push_rename_and_history():
    with repo() as (root, git), chdir(root):
        (root / "README.md").write_text("initial\n")
        initial = commit(git, "initial")
        git("branch", "base")
        git("switch", "base")
        (root / "docs").mkdir()
        (root / "docs" / "base-only.md").write_text("base\n")
        base = commit(git, "base-only change")
        git("switch", "-c", "feature", initial)
        (root / "services" / "novel-service").mkdir(parents=True)
        (root / "services" / "novel-service" / "lib.rs").write_text("backend\n")
        commit(git, "service change")
        (root / "docs").mkdir()
        (root / "docs" / "guide.md").write_text("docs\n")
        head = commit(git, "docs change")
        paths = scope.changed_paths({"pull_request": {
            "base": {"sha": base}, "head": {"sha": head},
        }}, "pull_request")
        assert set(paths or []) == {"services/novel-service/lib.rs", "docs/guide.md"}
    with repo() as (root, git), chdir(root):
        (root / "README.md").write_text("base\n")
        before = commit(git, "base")
        (root / "frontend" / "src").mkdir(parents=True)
        (root / "frontend" / "src" / "App.tsx").write_text("frontend\n")
        after = commit(git, "frontend change")
        paths = scope.changed_paths(
            {"before": before, "after": after, "ref": "refs/heads/main"}, "push"
        )
        assert paths == ["frontend/src/App.tsx"]
    with repo() as (root, git), chdir(root):
        (root / "services").mkdir()
        (root / "services" / "legacy.rs").write_text("legacy\n")
        (root / "infra" / "postgres").mkdir(parents=True)
        (root / "infra" / "postgres" / "old.sql").write_text("old\n")
        before = commit(git, "base")
        (root / "frontend" / "src").mkdir(parents=True)
        (root / "services" / "legacy.rs").rename(root / "frontend" / "src" / "moved.ts")
        (root / "infra" / "postgres" / "old.sql").unlink()
        after = commit(git, "rename and delete")
        paths = scope.changed_paths(
            {"before": before, "after": after, "ref": "refs/heads/main"}, "push"
        )
        assert set(paths or []) == {
            "services/legacy.rs", "frontend/src/moved.ts", "infra/postgres/old.sql"
        }
    with repo() as (root, git), chdir(root):
        (root / "README.md").write_text("initial\n")
        after = commit(git, "initial")
        missing = {"pull_request": {"base": {"sha": "f" * 40}, "head": {"sha": after}}}
        assert scope.changed_paths(missing, "pull_request") is None
        assert scope.changed_paths(
            {"before": "0" * 40, "after": after, "ref": "refs/heads/main"}, "push"
        ) is None
    for event_name, event in (
        ("workflow_dispatch", {}), ("workflow_call", {}),
        ("push", {"ref": "refs/tags/v1.2.3", "before": "1" * 40, "after": "2" * 40}),
    ):
        assert scope.changed_paths(event, event_name) is None

def output_for(event, event_name, paths, full):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        event_path, output_path = root / "event.json", root / "output"
        event_path.write_text(json.dumps(event))
        env = {
            "GITHUB_EVENT_PATH": str(event_path), "GITHUB_EVENT_NAME": event_name,
            "GITHUB_OUTPUT": str(output_path), "CI_FULL": full,
        }
        with patch.dict(os.environ, env, clear=False), patch.object(
            scope, "changed_paths", return_value=paths
        ):
            with redirect_stdout(io.StringIO()):
                scope.main()
        return dict(line.split("=", 1) for line in output_path.read_text().splitlines())

def gate_script():
    workflow = Path(__file__).parents[1] / ".github" / "workflows" / "ci.yml"
    source = workflow.read_text()
    return textwrap.dedent(source.split("          python3 - <<'PY'\n", 1)[1].split("\n          PY\n", 1)[0])

def gate(results):
    env = os.environ | {"RESULTS": json.dumps(results)}
    return subprocess.run([sys.executable, "-c", gate_script()], env=env,
                          capture_output=True, text=True)

def test_outputs_and_gate():
    assert output_for({"pull_request": {}}, "pull_request", ["README.md"], "false") == {
        name: "false" for name in scope.SCOPES
    }
    assert output_for({"pull_request": {}}, "pull_request", ["README.md"], "true") == {
        name: "true" for name in scope.SCOPES
    }
    for name in JOBS:
        for result in ("failure", "cancelled"):
            statuses = {job: {"result": "success"} for job in JOBS}
            statuses[name] = {"result": result}
            failed = gate(statuses)
            assert failed.returncode, (name, result)
    statuses = {job: {"result": "success"} for job in JOBS}
    statuses["frontend"] = {"result": "skipped"}
    statuses["integration"] = {"result": "skipped"}
    assert gate(statuses).returncode == 0

if __name__ == "__main__":
    test_scopes()
    test_pr_push_rename_and_history()
    test_outputs_and_gate()
    print("ci_scope self-check: OK")
