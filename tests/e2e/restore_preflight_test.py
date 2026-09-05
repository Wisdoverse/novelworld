#!/usr/bin/env python3
"""Exercise the full restore script with real artifacts and a Docker substitute.

No Docker daemon or database is contacted. SQL correctness and erasure/lineage
behavior remain covered by backup_restore_drill.sh against real PostgreSQL.
"""
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = Path(sys.argv.pop(1)).resolve() if len(sys.argv) > 1 else ROOT / "infra/backup/restore.sh"
TOKEN = "11111111-1111-4111-8111-111111111111"
KEY = "SyntheticRestoreRegressionKeyOnly123456"
DOCKER = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
a = sys.argv[1:]
scenario = os.environ["RESTORE_TEST_SCENARIO"]
target = os.environ["POSTGRES_CONTAINER"]
with Path(os.environ["RESTORE_TEST_LOG"]).open("a") as log:
    log.write(json.dumps(a) + "\n")
pg, writer, replica = "a" * 64, "c" * 64, "d" * 64
if a[0] == "compose":
    if "config" in a: sys.exit(1 if scenario == "config_error" else 0)
    if "ps" in a:
        if scenario == "compose_error": sys.exit(1)
        if a[-1] == "postgres":
            if scenario == "missing_postgres": sys.exit(0)
            if scenario == "ambiguous_postgres": print(pg + "\n" + "b" * 64)
            else: print(pg)
        elif "--services" not in a:
            if scenario == "inventory_error": sys.exit(1)
            if scenario != "absent_writers": print(writer)
            if scenario == "running_replica": print(replica)
            if scenario == "running_migrator" and "postgres-migrate" in a: print(replica)
        sys.exit(0)
    if a[-3:] == ["run", "--rm", "postgres-migrate"]: sys.exit(0)
elif a[0] == "inspect":
    fmt, name = a[a.index("--format") + 1], a[-1]
    if fmt == "{{.Id}}":
        if scenario == "postgres_inspect_error" and name == pg: sys.exit(1)
        if scenario == "target_inspect_error" and name == target: sys.exit(1)
        print("b" * 64 if scenario == "mismatched_postgres" and name == target else pg)
        sys.exit(0)
    if fmt == "{{.State.Status}}" and name in (writer, replica):
        if scenario == "writer_inspect_error": sys.exit(1)
        if name == replica: print("running")
        else: print({"running": "running", "paused": "paused",
                     "restarting": "restarting", "unknown_state": "unknown"}.get(scenario, "exited"))
        sys.exit(0)
    if fmt == "{{.State.Health.Status}}" and name == target:
        if scenario == "health_inspect_error": sys.exit(1)
        print("starting" if scenario == "unhealthy" else "healthy"); sys.exit(0)
    # Old fixed-name lookup models an unrelated default deployment.
    if fmt == "{{.State.Running}}" and name.startswith("novel-"):
        print("true" if scenario == "unrelated_running" else "false"); sys.exit(0)
elif a[0] == "exec":
    if "-c" in a:
        sql = a[a.index("-c") + 1]
        if "SELECT token FROM" in sql: print("11111111-1111-4111-8111-111111111111")
        elif "SELECT token ||" in sql: print("22222222-2222-4222-8222-222222222222 parent=11111111-1111-4111-8111-111111111111")
        elif sql.startswith("SELECT "): print("2026-09-01 00:00:01+00")
        elif not sql.startswith(("COPY ", "DROP DATABASE ", "CREATE DATABASE ")): sys.exit(91)
    else:
        sys.stdin.read()
    sys.exit(0)
sys.exit(92)
'''


class RestorePreflight(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix="nw-restore-preflight-")
        cls.addClassCleanup(cls.directory.cleanup)
        cls.path = Path(cls.directory.name)
        (cls.path / "bin").mkdir()
        docker = cls.path / "bin/docker"
        docker.write_text(DOCKER)
        docker.chmod(0o700)
        cls.env = dict(os.environ, BACKUP_ENCRYPTION_KEY=KEY,
                       PATH=str(cls.path / "bin") + os.pathsep + os.environ["PATH"])
        # Empty account inventory keeps this command-boundary fixture small.
        dump = ("COPY public.users (id, role) FROM stdin;\n\\.\n"
                "COPY public.database_lineage (token, parent) FROM stdin;\n"
                f"{TOKEN}\t\\N\n\\.\n").encode()
        fields = ["schema=backup-artifact-v1", "covered_through=2026-09-01 00:00:00+00",
                  f"lineage_token={TOKEN}"]
        for kind, data in (("dump", dump), ("erasure", b"")):
            artifact = cls.path / f"{kind}.gz.enc"
            subprocess.run(["openssl", "enc", "-aes-256-cbc", "-pbkdf2", "-iter", "200000",
                            "-salt", "-pass", "env:BACKUP_ENCRYPTION_KEY", "-out", str(artifact)],
                           input=gzip.compress(data), env=cls.env, check=True)
            fields += [f"{kind}={artifact.name}",
                       f"{kind}_sha256={hashlib.sha256(artifact.read_bytes()).hexdigest()}"]
        cls.manifest = cls.path / "backup.manifest"
        cls.manifest.write_text("\n".join(fields) + "\n")

    def run_restore(self, scenario, target="nwq-0123456789-postgres"):
        case = self.path / scenario
        case.mkdir()
        env_file = case / "preserved environment.env"
        original = ("JWT_SECRET=SyntheticOnly123456789012345678901234\n"
                    "RUNTIME_CONFIG_KEY=" + "a" * 64 + "\n"
                    "INTERNAL_SERVICE_TOKEN=SyntheticOnlyInternalToken1234567890\n")
        env_file.write_text(original)
        env_file.chmod(0o600)
        log = case / "docker.jsonl"
        result = subprocess.run(["bash", str(SCRIPT), "--manifest", str(self.manifest),
                                 "--env-file", str(env_file)],
                                env=dict(self.env, POSTGRES_CONTAINER=target,
                                         RESTORE_TEST_SCENARIO=scenario, RESTORE_TEST_LOG=str(log)),
                                stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=30)
        calls = [json.loads(line) for line in log.read_text().splitlines()]
        return result, calls, env_file, original

    def test_failed_or_unsafe_discovery_never_reaches_database(self):
        scenarios = ("config_error", "compose_error", "missing_postgres", "ambiguous_postgres",
                     "mismatched_postgres", "postgres_inspect_error", "target_inspect_error",
                     "inventory_error", "writer_inspect_error", "running", "paused",
                     "restarting", "running_replica", "running_migrator", "unknown_state",
                     "health_inspect_error", "unhealthy")
        for scenario in scenarios:
            with self.subTest(scenario=scenario):
                result, calls, env_file, original = self.run_restore(scenario)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertFalse(any(call[0] == "exec" for call in calls), calls)
                self.assertFalse(any("run" in call for call in calls), calls)
                self.assertEqual(env_file.read_text(), original)
                self.assertFalse(list(env_file.parent.glob("*.pre-restore.*")))

    def test_stopped_selected_deployment_keeps_one_context_through_restore(self):
        for scenario, target in (("default", "novel-postgres"),
                                 ("unrelated_running", "nwq-0123456789-postgres"),
                                 ("absent_writers", "nwq-0123456789-postgres")):
            with self.subTest(scenario=scenario):
                result, calls, env_file, original = self.run_restore(scenario, target)
                self.assertEqual(result.returncode, 0, result.stderr)
                compose_calls = [call for call in calls if call[0] == "compose"]
                self.assertTrue(compose_calls)
                for call in compose_calls:
                    self.assertEqual(call[1:3], ["--env-file", str(env_file)])
                migrations = [call for call in compose_calls if "run" in call]
                self.assertEqual(len(migrations), 2)
                self.assertTrue(all(call[-3:] == ["run", "--rm", "postgres-migrate"] for call in migrations))
                self.assertTrue(any(call[0] == "exec" for call in calls))
                self.assertNotEqual(env_file.read_text(), original)


if __name__ == "__main__":
    unittest.main()
