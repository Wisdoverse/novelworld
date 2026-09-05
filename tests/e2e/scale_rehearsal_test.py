#!/usr/bin/env python3
"""Command-boundary checks for scale_rehearsal.sh.

The fixture substitutes Docker, backup/restore, and byte counters. It never
creates a database or a 5 GiB artifact; a passing test is only script evidence.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SOURCE = Path(__file__).resolve().parents[2]
SCRIPT = SOURCE / "infra/backup/scale_rehearsal.sh"
KEY = "SyntheticScaleRehearsalKeyOnly123456789"
CONTAINER = "nw-scale-postgres"


DOCKER = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
a = sys.argv[1:]
scenario = os.environ["SCALE_SCENARIO"]
Path(os.environ["SCALE_LOG"]).open("a").write(json.dumps(a) + "\n")
if a and a[0] == "compose" and "config" in a:
    print(json.dumps({"services": {"postgres": {"environment": {
        "POSTGRES_USER": "novel", "POSTGRES_DB": "novel_world"}}}}))
    sys.exit(0)
if a and a[0] == "compose" and "ps" in a:
    if scenario == "compose_error": sys.exit(1)
    if a[-1] == "postgres": print("a" * 64)
    elif scenario == "running_writer": print("c" * 64)
    sys.exit(0)
if a and a[0] == "compose" and "up" in a:
    if "JWT_SECRET" in os.environ: sys.exit(1)
    assert "--wait" in a and "--no-build" in a
    env_file = a[a.index("--env-file") + 1]
    if "synthetic-new" not in Path(env_file).read_text() or "synthetic-old" in Path(env_file).read_text():
        sys.exit(1)
    Path(os.environ["SCALE_EVENTS"]).open("a").write("up-env-ok wait no-build\\n")
    import time; time.sleep(0.1)
    sys.exit(1 if scenario == "readiness_failure" else 0)
if a and a[0] == "inspect":
    fmt, name = a[a.index("--format") + 1], a[-1]
    if fmt == "{{.Id}}":
        print("b" * 64 if scenario == "target_mismatch" and name == os.environ["POSTGRES_CONTAINER"] else "a" * 64)
        sys.exit(0)
    if fmt == "{{.State.Health.Status}}": print("healthy"); sys.exit(0)
    if fmt == "{{.State.Status}}": print("running" if scenario == "running_writer" else "exited"); sys.exit(0)
if a and a[0] == "exec":
    sql = a[a.index("-c") + 1] if "-c" in a else ""
    if "NOT EXISTS" in sql: print("f" if scenario == "existing_userdata" else "t")
    elif "octet_length" in sql: print("600000000")
    elif "pg_database_size" in sql: print("700000000")
    elif "POSTGRES_USER" in sql: print("novel\nnovel_world")
    elif "INSERT" in sql: print("600000000")
    sys.exit(0)
sys.exit(2)
'''


class ScaleRehearsal(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="nw-scale-test-")
        self.root = Path(self.tmp.name)
        (self.root / "infra/backup").mkdir(parents=True)
        (self.root / "tests/e2e").mkdir(parents=True)
        shutil.copy2(SCRIPT, self.root / "infra/backup/scale_rehearsal.sh")
        (self.root / "infra/backup/backup.sh").write_text(
            """#!/bin/sh
set -eu
[ \"${SCALE_SCENARIO}\" = backup_failure ] && exit 1
base=\"$BACKUP_DIR/synthetic\"
printf x >\"$base.dump.gz.enc\"
printf 'schema=backup-artifact-v1\\ndump=synthetic.dump.gz.enc\\n' >\"$base.manifest\"
"""
        )
        (self.root / "infra/backup/restore.sh").write_text(
            """#!/bin/sh
set -eu
[ \"$1\" = --manifest ] && [ -f \"$2\" ]
[ \"$3\" = --env-file ] && [ -f \"$4\" ]
sed -i 's/^JWT_SECRET=.*/JWT_SECRET=synthetic-new/' \"$4\"
printf '%s\\n' restore-ok >>\"$SCALE_EVENTS\"
[ \"${SCALE_SCENARIO}\" = restore_failure ] && exit 1
exit 0
"""
        )
        for name in ("backup.sh", "restore.sh", "scale_rehearsal.sh"):
            (self.root / "infra/backup" / name).chmod(0o700)
        bindir = self.root / "bin"
        bindir.mkdir()
        (bindir / "docker").write_text(DOCKER)
        (bindir / "docker").chmod(0o700)
        (bindir / "openssl").write_text(
            """#!/bin/sh
[ \"${SCALE_SCENARIO}\" = decrypt_failure ] && exit 1
printf synthetic-plain
"""
        )
        (bindir / "openssl").chmod(0o700)
        (bindir / "gzip").write_text("#!/bin/sh\ncat >/dev/null\nprintf plain-stream\n")
        (bindir / "gzip").chmod(0o700)
        (bindir / "wc").write_text("""#!/bin/sh
[ \"$1\" = -c ] || exit 1
data=$(cat)
case \"$data\" in
plain-stream*) [ \"${SCALE_SCENARIO}\" = undersized ] && echo 100 || echo 5368709121 ;;
*) echo 123 ;;
esac
""")
        (bindir / "wc").chmod(0o700)
        (bindir / "df").write_text("#!/bin/sh\nprintf '%s\\n' 'Filesystem 1024-blocks Used Available Capacity Mounted on' 'synthetic 999999999 0 999999999 0% /'\n")
        (bindir / "df").chmod(0o700)
        (bindir / "git").write_text("#!/bin/sh\nprintf 'synthetic-head\\n'\n")
        (bindir / "git").chmod(0o700)
        self.env = dict(os.environ, PATH=str(bindir) + os.pathsep + os.environ["PATH"],
                        BACKUP_ENCRYPTION_KEY=KEY, POSTGRES_CONTAINER=CONTAINER,
                        SCALE_LOG=str(self.root / "docker.log"), SCALE_EVENTS=str(self.root / "events.log"),
                        TMPDIR=str(self.root / "tmp"),
                        BACKUP_DIR=str(self.root / "backups"))
        (self.root / "tmp").mkdir()
        (self.root / "backups").mkdir()
        self.env_file = self.root / "preserved.env"
        self.env_file.write_text("JWT_SECRET=synthetic-old\n")
        self.env["JWT_SECRET"] = "synthetic-old"

    def run_scale(self, scenario, *extra):
        env = dict(self.env, SCALE_SCENARIO=scenario)
        return subprocess.run(
            ["bash", str(self.root / "infra/backup/scale_rehearsal.sh"),
             "--env-file", str(self.env_file), "--confirm-target", CONTAINER,
             "--target-gb", "5", *extra], env=env, cwd=self.root,
            capture_output=True, text=True, timeout=30)

    def test_rejects_target_and_dirty_database_before_seeding(self):
        for scenario, confirmation in (("default", "wrong"), ("target_mismatch", CONTAINER),
                                       ("compose_error", CONTAINER), ("existing_userdata", CONTAINER),
                                       ("running_writer", CONTAINER)):
            with self.subTest(scenario=scenario):
                (self.root / "docker.log").unlink(missing_ok=True)
                env = dict(self.env, SCALE_SCENARIO=scenario)
                result = subprocess.run(
                    ["bash", str(self.root / "infra/backup/scale_rehearsal.sh"),
                     "--env-file", str(self.env_file), "--confirm-target", confirmation,
                     "--target-gb", "5"], env=env, cwd=self.root,
                    capture_output=True, text=True, timeout=30)
                self.assertNotEqual(result.returncode, 0)
                calls = [json.loads(line) for line in (self.root / "docker.log").read_text().splitlines()] if (self.root / "docker.log").exists() else []
                self.assertFalse(any("INSERT" in " ".join(call) for call in calls))

    def test_logical_fixture_and_restore_readiness_path(self):
        result = self.run_scale("default")
        self.assertEqual(result.returncode, 0, result.stderr)
        log = (self.root / "docker.log").read_text()
        self.assertIn("octet_length", log)
        calls = [json.loads(line) for line in log.splitlines() if line.startswith("[")]
        batch_calls = [call for call in calls if call[0] == "exec" and "new_novels" in " ".join(call)]
        self.assertEqual(len(batch_calls), 9)
        physical_calls = [call for call in calls if call[0] == "exec" and "pg_database_size" in " ".join(call)]
        self.assertEqual(len(physical_calls), 1)
        events = (self.root / "events.log").read_text()
        self.assertIn("restore-ok", events)
        self.assertIn("up-env-ok wait no-build", events)
        self.assertEqual(self.env_file.read_text(), "JWT_SECRET=synthetic-new\n")
        report = next((self.root / "backups").glob("scale-*/measurement.json"))
        data = json.loads(report.read_text())
        self.assertEqual(data["status"], "passed")
        self.assertGreaterEqual(data["plain_dump_bytes"], 5 * 1024**3)
        self.assertLess(data["database_physical_bytes"], data["plain_dump_bytes"])
        self.assertGreaterEqual(data["restore_to_readiness_seconds"], 0.1)
        self.assertFalse(data["reference_hardware_verified"])

    def test_backup_decrypt_size_restore_and_readiness_fail_closed(self):
        for scenario in ("backup_failure", "decrypt_failure", "undersized", "restore_failure", "readiness_failure"):
            with self.subTest(scenario=scenario):
                (self.root / "docker.log").unlink(missing_ok=True)
                (self.root / "events.log").unlink(missing_ok=True)
                before = set((self.root / "backups").glob("scale-*/measurement.json"))
                result = self.run_scale(scenario)
                self.assertNotEqual(result.returncode, 0)
                after = set((self.root / "backups").glob("scale-*/measurement.json"))
                reports = after - before
                self.assertEqual(len(reports), 1)
                self.assertEqual(json.loads(next(iter(reports)).read_text())["status"], "failed")
                log = (self.root / "docker.log").read_text() if (self.root / "docker.log").exists() else ""
                calls = [json.loads(line) for line in log.splitlines() if line.startswith("[")]
                events = (self.root / "events.log").read_text() if (self.root / "events.log").exists() else ""
                if scenario == "restore_failure":
                    self.assertIn("restore-ok", events)
                    self.assertFalse(any(call[0] == "compose" and "up" in call for call in calls))
                if scenario == "undersized":
                    self.assertNotIn("restore-ok", events)
                    self.assertFalse(any(call[0] == "compose" and "up" in call for call in calls))
                if scenario == "readiness_failure":
                    self.assertIn("restore-ok", events)
                    up_calls = [call for call in calls if call[0] == "compose" and "up" in call]
                    self.assertTrue(up_calls)
                    self.assertTrue(all("--wait" in call and "--no-build" in call for call in up_calls))


if __name__ == "__main__":
    unittest.main()
