#!/usr/bin/env python3
"""Exercise release record validation and SBOM identity with a fake registry.

This runs the real shell entrypoints, not native image builds or Trivy scans.
The registry's mutable tag and first RepoDigests entry deliberately point at
a different image from the build records.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
RECORD = ROOT / "infra/docker/record-application-images.sh"
SBOM = ROOT / "infra/security/generate-sboms.sh"
RELEASE = ROOT / "infra/docker/release.sh"
SERVICES = ("gateway", "user-service", "novel-service", "agent-service", "narrative-service", "frontend")
PREFIX = "ghcr.io/wisdoverse/novelworld"
BUILT = "sha256:" + "a" * 64
MOVED = "sha256:" + "b" * 64
LOCAL = "sha256:" + "c" * 64

DOCKER = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ["DOCKER_LOG"], "a") as log:
    log.write(json.dumps(args) + "\n")
if os.environ.get("DOCKER_FAILURE") == args[0]:
    sys.exit(83)
if args[0] == "inspect":
    if ".Id" in args[2]:
        print(os.environ["LOCAL_ID"])
    elif not args[-1].startswith("novel-world-"):
        print("ghcr.io/wisdoverse/novelworld-gateway@" + os.environ["MOVED_DIGEST"])
elif args[0] == "pull":
    pass
elif args[0] == "run":
    mounts = [args[i + 1] for i, value in enumerate(args[:-1]) if value == "-v"]
    output_dir = pathlib.Path(next(mount[:-5] for mount in mounts if mount.endswith(":/out")))
    output_name = pathlib.Path(args[args.index("--output") + 1]).name
    image = args[-1]
    observed = image.split("@", 1)[1] if "@" in image else os.environ["MOVED_DIGEST"]
    (output_dir / output_name).write_text(json.dumps({"image": image, "observed_digest": observed}))
else:
    sys.exit("unexpected docker command")
'''


class ReleaseImageDigestTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.records = self.root / "records"
        self.records.mkdir()
        self.write_records()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        docker = self.bin / "docker"
        docker.write_text(DOCKER)
        docker.chmod(0o755)
        self.log = self.root / "docker.jsonl"
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        DOCKER_LOG=str(self.log), MOVED_DIGEST=MOVED, LOCAL_ID=LOCAL)

    def write_records(self):
        for service in SERVICES:
            (self.records / f"{service}.txt").write_text(f"{PREFIX}-{service}@{BUILT}\n")

    def record(self):
        return subprocess.run(["bash", str(RECORD), str(self.records), PREFIX],
                              capture_output=True, text=True, check=False, timeout=10)

    def sboms(self, output, images):
        return subprocess.run(["bash", str(SBOM), str(output), *images], env=self.env,
                              capture_output=True, text=True, check=False, timeout=20)

    def test_rejects_incomplete_ambiguous_or_wrong_records(self):
        gateway = self.records / "gateway.txt"
        valid = gateway.read_text()
        for invalid in ("", valid + valid, valid + "\n", valid.replace("gateway@", "frontend@"),
                        valid.replace("wisdoverse/", "untrusted/"),
                        valid.replace("@" + BUILT, "@sha256:garbage@" + BUILT),
                        valid.replace("@" + BUILT, ":moving-tag"),
                        valid.replace(BUILT, "sha256:1234")):
            with self.subTest(invalid=invalid):
                gateway.write_text(invalid)
                result = self.record()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")
        gateway.unlink()
        self.assertNotEqual(self.record().returncode, 0)
        gateway.symlink_to(self.records / "frontend.txt")
        self.assertNotEqual(self.record().returncode, 0)
        gateway.unlink()
        gateway.write_text(valid)
        (self.records / ".unexpected").write_text(valid)
        self.assertNotEqual(self.record().returncode, 0)

    def test_tag_movement_cannot_change_manifest_or_sbom_identity(self):
        result = self.record()
        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = dict(line.split("=", 1) for line in result.stdout.splitlines())
        self.assertEqual(set(manifest), {service.upper().replace("-", "_") + "_IMAGE" for service in SERVICES})
        images = list(manifest.values())
        self.assertEqual(images, [f"{PREFIX}-{service}@{BUILT}" for service in SERVICES])
        release_manifest = self.root / "release.env"
        release_manifest.write_text("RELEASE_VERSION=test\nRELEASE_GIT_SHA=" + "1" * 40 + "\n" +
                                    result.stdout + "".join(f"{key}_IMAGE=example/{key.lower()}@{BUILT}\n"
                                    for key in ("POSTGRES", "REDIS", "NGINX")))
        validation = subprocess.run(["bash", str(RELEASE), "validate", str(release_manifest)],
                                    cwd=ROOT, capture_output=True, text=True, check=False, timeout=10)
        self.assertEqual(validation.returncode, 0, validation.stderr)
        output = self.root / "sboms"
        result = self.sboms(output, images)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(dict(line.split() for line in (output / "digests.txt").read_text().splitlines()),
                         dict.fromkeys(SERVICES, BUILT))
        for service, image in zip(SERVICES, images):
            payload = json.loads((output / f"{service}.cdx.json").read_text())
            self.assertEqual(payload, {"image": image, "observed_digest": BUILT})
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertFalse(any(call[0] == "inspect" for call in calls))
        self.assertEqual([call[1] for call in calls if call[0] == "pull"], images)
        self.assertEqual([call[-1] for call in calls if call[0] == "run"], images)

    def test_local_image_id_mode_and_bad_pinned_input(self):
        output = self.root / "local-sboms"
        result = self.sboms(output, ["novel-world-gateway:local"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((output / "digests.txt").read_text(), f"gateway {LOCAL}\n")
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertFalse(any(call[0] == "pull" for call in calls))
        self.log.unlink()
        result = self.sboms(self.root / "invalid-sboms", [PREFIX + "-gateway@sha256:1234"])
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.log.exists())
        for command in ("pull", "run"):
            with self.subTest(command=command):
                self.env["DOCKER_FAILURE"] = command
                output = self.root / f"failed-{command}"
                result = self.sboms(output, [f"{PREFIX}-gateway@{BUILT}"])
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual((output / "digests.txt").read_text(), "")


if __name__ == "__main__":
    unittest.main()
