#!/usr/bin/env python3
"""Exercise the Jenkins record with disposable Git repositories and a Docker spy."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SERVICES = ("gateway", "user-service", "novel-service", "agent-service", "narrative-service", "frontend")
SCRIPT = "infra/ops/jenkins-image-record.sh"
SPY = r'''#!/usr/bin/env python3
import hashlib, os, pathlib, subprocess, sys
args = sys.argv[1:]
mode = os.environ.get("DOCKER_CASE", "")
services = ("gateway", "user-service", "novel-service", "agent-service", "narrative-service", "frontend")
def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()
if args[:3] == ["compose", "--env-file", ".env"]:
    action = args[3:]
    if action == ["build", *services]:
        if mode == "build_failure": sys.exit(41)
        if mode == "source_during_build": pathlib.Path("tracked.txt").write_text("changed")
    elif action[:2] == ["config", "--images"] and len(action) == 3:
        service = action[2]
        print("uncached-dependency:local")
        print("postgres:local")
        if mode != "missing_image" or service != "frontend": print(service + ":local")
        if mode == "ambiguous_image" and service == "frontend": print("alternate-frontend:local")
    elif action[:3] == ["ps", "--all", "--quiet"] and len(action) == 4:
        if mode != "missing_container": print(digest("container-" + action[3]))
        if mode == "multiple_containers": print(digest("extra"))
    else: sys.exit("Unexpected Compose call: " + repr(action))
elif args[:3] == ["image", "inspect", "--format"]:
    assert args[3] == '{{.Id}} {{index .Config.Labels "com.docker.compose.service"}}'
    service = args[4].split(":")[0]
    if service == "uncached-dependency": sys.exit(1)
    label = "frontend" if service == "alternate-frontend" else service
    image = "sha256:" + digest(service)
    if mode == "invalid_image" and service == "frontend": image = "invalid"
    print(image, label)
elif args[:3] == ["container", "inspect", "--format"]:
    assert args[3] == '{{.Image}} {{.State.Running}} {{index .Config.Labels "com.docker.compose.service"}}'
    service = next(s for s in services if digest("container-" + s) == args[4])
    image = "sha256:" + digest("other" if mode == "mismatch" else service)
    running = "false" if mode == "stopped" else "true"
    label = "wrong" if mode == "wrong_service" else service
    print(image, running, label)
    if mode == "source_during_verify": pathlib.Path("tracked.txt").write_text("changed")
else: sys.exit("Unexpected Docker call: " + repr(args))
'''


class RecordTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="jenkins-record-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        (self.repo / "infra/ops").mkdir(parents=True)
        shutil.copyfile(ROOT / SCRIPT, self.repo / SCRIPT)
        (self.repo / ".gitignore").write_text(".env\n.local/\n")
        (self.repo / "tracked.txt").write_text("original")
        (self.repo / ".env").write_text("PRIVATE_VALUE=synthetic-do-not-archive\n")
        self.git("init", "-q")
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture")
        self.source = self.git("rev-parse", "HEAD").strip()
        fake_bin = self.root / "bin"
        fake_bin.mkdir()
        (fake_bin / "docker").write_text(SPY)
        (fake_bin / "docker").chmod(0o755)
        self.env = {**os.environ, "PATH": str(fake_bin) + os.pathsep + os.environ["PATH"], "BUILD_NUMBER": "31"}
        self.record = self.repo / ".local/jenkins-deployment/deployment.tsv"
        self.build_record = self.record.with_name("build.tsv")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, text=True, stderr=subprocess.PIPE)

    def run_record(self, action, case="", success=True):
        result = subprocess.run(["bash", SCRIPT, action], cwd=self.repo,
                                env={**self.env, "DOCKER_CASE": case}, text=True, capture_output=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.record.exists(), "A failure retained a success record")
        self.assertNotIn("synthetic-do-not-archive", result.stdout + result.stderr)
        self.assertEqual(list(self.record.parent.glob(".record.*")), [])

    def seed_stale_record(self):
        self.record.parent.mkdir(parents=True, exist_ok=True)
        self.record.write_text("stale-success")

    def test_success_records_only_source_and_six_identities(self):
        self.seed_stale_record()
        self.run_record("build")
        self.assertFalse(self.record.exists())
        self.run_record("verify")
        expected = [f"source\t{self.source}"]
        for service in SERVICES:
            image = hashlib.sha256(service.encode()).hexdigest()
            container = hashlib.sha256(("container-" + service).encode()).hexdigest()
            expected.append(f"{service}\tsha256:{image}\t{container}")
        self.assertEqual(self.record.read_text(), "\n".join(expected) + "\n")

    def test_build_failures_remove_stale_records(self):
        for case in ("build_failure", "missing_image", "ambiguous_image", "invalid_image", "source_during_build"):
            with self.subTest(case=case):
                (self.repo / "tracked.txt").write_text("original")
                self.seed_stale_record()
                self.build_record.write_text("stale-build")
                self.run_record("build", case, success=False)
                self.assertFalse(self.build_record.exists())

    def test_verification_failures_never_publish_partial_records(self):
        self.run_record("build")
        for case in ("missing_container", "multiple_containers", "mismatch", "stopped", "wrong_service", "source_during_verify"):
            with self.subTest(case=case):
                self.seed_stale_record()
                self.run_record("verify", case, success=False)

    def test_dirty_and_untracked_source_rejected(self):
        for name in ("tracked.txt", "untracked.txt"):
            with self.subTest(name=name):
                path = self.repo / name
                path.write_text("modified")
                self.seed_stale_record()
                self.run_record("build", success=False)
                if name == "tracked.txt": path.write_text("original")
                else: path.unlink()

    def test_source_and_build_number_must_match(self):
        self.run_record("build")
        self.env["BUILD_NUMBER"] = "32"
        self.run_record("verify", success=False)
        self.env["BUILD_NUMBER"] = "31"
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-qm", "new source")
        self.run_record("verify", success=False)

    def test_missing_truncated_and_extra_build_data_rejected(self):
        self.run_record("verify", success=False)
        self.run_record("build")
        original = self.build_record.read_text()
        for value in (original.splitlines()[0] + "\n", original + "unexpected\n"):
            self.build_record.write_text(value)
            self.run_record("verify", success=False)

    def test_pipeline_archives_only_after_health_and_verification(self):
        pipeline = (ROOT / "Jenkinsfile").read_text()
        ordered = ["jenkins-image-record.sh build", "bash start.sh </dev/null", "bash infra/ops/health-checks.sh",
                   "jenkins-image-record.sh verify", "archiveArtifacts artifacts: '.local/jenkins-deployment/deployment.tsv', onlyIfSuccessful: true"]
        offsets = [pipeline.index(value) for value in ordered]
        self.assertEqual(offsets, sorted(offsets))
        self.assertEqual(pipeline.count("archiveArtifacts"), 1)


if __name__ == "__main__":
    unittest.main()
