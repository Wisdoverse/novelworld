#!/usr/bin/env python3
"""Fault-inject scanner completion; real image/fixture coverage is in the shell self-test."""
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
COMPLETE = "12:00AM INF 2 commits scanned.\n12:00AM INF no leaks found\n"


class SecretScanTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / "tools").mkdir()
        shutil.copy(ROOT / "tools/scan-secrets.sh", self.root / "tools/scan-secrets.sh")
        (self.root / "bin").mkdir()
        stub = self.root / "bin/docker"
        stub.write_text("""#!/usr/bin/env python3
import os, sys
args = sys.argv[1:]
assert args[0:2] == ['run', '--rm']
assert any(arg.startswith('ghcr.io/gitleaks/gitleaks:v8.30.1@sha256:') for arg in args)
assert '--redact=100' in args
assert args[args.index('--log-level') + 1] == 'debug'
assert args[args.index('--log-opts') + 1] == 'HEAD'
assert args[args.index('--timeout') + 1] == '300'
print(os.environ['SCAN_TEST_OUTPUT'], end='')
sys.exit(int(os.environ.get('SCAN_TEST_STATUS', '0')))
""")
        stub.chmod(0o755)
        self.env = dict(os.environ, SCAN_TEST_OUTPUT=COMPLETE, SCAN_TEST_STATUS="0",
                        PATH=str(self.root / "bin") + os.pathsep + os.environ['PATH'])
        for name in ("GIT_DIR", "GIT_COMMON_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"):
            self.env.pop(name, None)
        self.git("init", "-q")
        self.git("add", ".")
        self.git("-c", "user.email=test@example.invalid", "-c", "user.name=Test",
                 "commit", "-qm", "fixture")

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.root), *args], env=self.env,
                              capture_output=True, check=True, timeout=10)

    def scan(self, source=None):
        return subprocess.run(["bash", str(self.root / "tools/scan-secrets.sh"),
                               str(source or self.root)], env=self.env,
                              capture_output=True, text=True, timeout=15)

    def test_complete_and_detected_are_distinct(self):
        # A repository-local executable must never replace the pinned image.
        (self.root / ".tools-bin").mkdir()
        untrusted = self.root / ".tools-bin/gitleaks"
        untrusted.write_text('#!/bin/sh\nexit 99\n')
        untrusted.chmod(0o755)
        result = self.scan()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "secret-scan: 2 commits scanned; no leaks found\n")
        self.env["SCAN_TEST_OUTPUT"] = COMPLETE.replace("INF no leaks found", "WRN leaks found: 1")
        self.env["SCAN_TEST_STATUS"] = "42"
        result = self.scan()
        self.assertEqual(result.returncode, 42)
        self.assertIn("credentials detected", result.stderr)

    def test_exit_zero_never_overrides_incomplete_evidence(self):
        for output in ("", COMPLETE.replace("2 commits", "0 commits"),
                       "12:00AM INF 2 commits scanned.\n", COMPLETE + COMPLETE,
                       COMPLETE + "12:00AM ERR Git failed\n",
                       COMPLETE + "12:00AM FTL failed\n",
                       COMPLETE + "12:00AM DBG command aborted error=exit status 128\n",
                       COMPLETE + "12:00AM WRN partial scan completed\n"):
            with self.subTest(case=repr(output)):
                self.env["SCAN_TEST_OUTPUT"] = output
                self.assertEqual(self.scan().returncode, 1)
        self.env["SCAN_TEST_OUTPUT"] = COMPLETE
        for status in ("1", "42", "124", "137"):
            self.env["SCAN_TEST_STATUS"] = status
            self.assertEqual(self.scan().returncode, 1)

    def test_diagnostics_never_reach_console(self):
        value = "sk-" + secrets.token_hex(16)
        for status in ("0", "1", "42"):
            self.env["SCAN_TEST_STATUS"] = status
            self.env["SCAN_TEST_OUTPUT"] = COMPLETE + "12:00AM ERR " + value
            result = self.scan()
            self.assertEqual(result.returncode, 1)
            self.assertNotIn(value, result.stdout + result.stderr)

    def test_invalid_and_shallow_sources_fail_before_scanner(self):
        self.assertEqual(self.scan(self.root / "missing").returncode, 1)
        unborn = self.root / "unborn"
        subprocess.run(["git", "init", "-q", str(unborn)], check=True, env=self.env)
        self.assertEqual(self.scan(unborn).returncode, 1)
        shallow = self.root / "shallow"
        subprocess.run(["git", "clone", "-q", "--depth=1", self.root.as_uri(), str(shallow)],
                       check=True, env=self.env, capture_output=True)
        self.assertEqual(self.scan(shallow).returncode, 1)


if __name__ == "__main__":
    unittest.main()
