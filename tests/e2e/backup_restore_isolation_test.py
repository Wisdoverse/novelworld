#!/usr/bin/env python3
"""Check real Compose isolation and target-image refusal before fixture work.

No containers, database or providers are started. The full native drill owns
image-transition, table-preservation and encrypted-recovery evidence.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tests/e2e/backup_restore_drill.sh"
IMAGE = "127.0.0.1:5000/restore:fixture@sha256:" + "a" * 64
ENV = {
    "PATH": os.environ["PATH"],
    "POSTGRES_PASSWORD": "SyntheticPostgresOnly123456",
    "JWT_SECRET": "SyntheticJwtOnly12345678901234567890",
    "RUNTIME_CONFIG_KEY": "a" * 64,
    "INTERNAL_SERVICE_TOKEN": "SyntheticInternalOnly123456789012",
}


class BackupRestoreIsolation(unittest.TestCase):
    def test_actual_compose_defaults_and_isolated_host_ports(self):
        # Execute only the documented naming statements, never its drill/cleanup.
        docs = (ROOT / "CONTRIBUTING.md").read_text().split(
            "### Disposable PostgreSQL recovery drill", 1)[1].split("```bash", 1)[1].split("```", 1)[0]
        naming = "\n".join(line for line in docs.splitlines() if line.startswith((
            "suffix=", "export COMPOSE_PROJECT_NAME=", "export CONTAINER_PREFIX=")))
        result = subprocess.run(["bash", "-c", naming + '\nprintf "%s" "$COMPOSE_PROJECT_NAME"'],
                                env=ENV, capture_output=True, text=True, timeout=5, check=True)
        documented_project = result.stdout
        for overrides, ports, prefix in (
            ({}, (18080, 18081), "novel"),
            ({"COMPOSE_PROJECT_NAME": documented_project,
              "CONTAINER_PREFIX": documented_project,
              "E2E_LLM_STUB_PORT": "28080", "E2E_GATEWAY_PORT": "28081",
              "NGINX_HTTP_BIND": "127.0.0.1", "NGINX_HTTP_PORT": "28082"},
             (28080, 28081), documented_project),
        ):
            with self.subTest(prefix=prefix):
                result = subprocess.run(
                    ["docker", "compose", "--env-file", "/dev/null", "-f",
                     str(ROOT / "docker-compose.yml"), "-f",
                     str(ROOT / "docker-compose.e2e.yml"), "config", "--format", "json"],
                    env=dict(ENV, **overrides), capture_output=True, text=True, timeout=30,
                    check=True,
                )
                config = json.loads(result.stdout)
                services = config["services"]
                for name, published, target in (("llm-stub", ports[0], 18080),
                                                ("gateway", ports[1], 8080)):
                    self.assertEqual(services[name]["ports"], [{
                        "mode": "ingress", "host_ip": "127.0.0.1",
                        "target": target, "published": str(published), "protocol": "tcp",
                    }])
                self.assertEqual(services["postgres"]["container_name"], prefix + "-postgres")
                self.assertEqual(services["gateway"]["container_name"], prefix + "-gateway")
                self.assertEqual(services["postgres"]["image"],
                                 services["postgres-migrate"]["image"])
                for name in ("user-service", "novel-service", "agent-service", "narrative-service"):
                    self.assertEqual(services[name]["environment"]["LLM_API_URL"],
                                     "http://llm-stub:18080")
                if overrides:
                    self.assertEqual(config["name"], documented_project)
                    self.assertEqual(services["nginx"]["ports"][0]["host_ip"], "127.0.0.1")
                    self.assertEqual(services["nginx"]["ports"][0]["published"], "28082")

    def test_invalid_or_missing_cached_target_never_creates_fixtures(self):
        with tempfile.TemporaryDirectory(prefix="nw-restore-image-preflight-") as directory:
            root = Path(directory)
            calls = root / "calls.jsonl"
            for tool in ("docker", "curl"):
                command = root / tool
                command.write_text(
                    "#!/usr/bin/env python3\nimport json,os,sys\nfrom pathlib import Path\n"
                    "with Path(os.environ['ISOLATION_TEST_LOG']).open('a') as log:\n"
                    "    log.write(json.dumps([Path(sys.argv[0]).name,*sys.argv[1:]])+'\\n')\n"
                    "if os.environ.get('ISOLATION_TEST_IMAGE_ID'):\n"
                    "    print(os.environ['ISOLATION_TEST_IMAGE_ID']); sys.exit(0)\n"
                    "sys.exit(91)\n"
                )
                command.chmod(0o700)
            cases = [(image, "", "immutable image reference") for image in (
                "postgres:latest", "registry/postgres@sha256:1234",
                "registry/postgres@sha256:garbage@sha256:" + "a" * 64,
                IMAGE + "\n", " " + IMAGE, IMAGE.replace("a" * 64, "A" * 64),
            )]
            cases += [(IMAGE, "", "already be cached locally"),
                      (IMAGE, "malformed-image-id", "exact cached restore image ID")]
            for image, image_id, message in cases:
                with self.subTest(image=image, image_id=image_id):
                    calls.unlink(missing_ok=True)
                    result = subprocess.run(
                        ["bash", str(SCRIPT)], cwd=ROOT,
                        env=dict(ENV, PATH=str(root) + os.pathsep + ENV["PATH"],
                                 E2E_RESTORE_POSTGRES_IMAGE=image,
                                 ISOLATION_TEST_LOG=str(calls), ISOLATION_TEST_IMAGE_ID=image_id),
                        stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=5,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(message, result.stderr)
                    actual = [json.loads(line) for line in calls.read_text().splitlines()] \
                        if calls.exists() else []
                    expected = [["docker", "image", "inspect", "--format", "{{.Id}}", IMAGE]] \
                        if image == IMAGE else []
                    self.assertEqual(actual, expected)

    def test_pgdata_guard_rejects_redirecting_submounts(self):
        # Run the real shell function with metadata fixtures and a Docker substitute.
        source = SCRIPT.read_text()
        start = source.index("  pgdata_identity() {")
        function = source[start:source.index("  table_snapshot() {", start)]
        project = "nw-pgdata-test"
        volume = {"Name": project + "_postgres_data", "Driver": "local",
                  "CreatedAt": "2026-10-06T00:00:00Z", "Mountpoint": "/synthetic/pgdata",
                  "Scope": "local", "Labels": {"com.docker.compose.project": project,
                  "com.docker.compose.volume": "postgres_data"}}
        container = {"Config": {"Labels": {"com.docker.compose.project": project,
                     "com.docker.compose.service": "postgres"},
                     "Env": ["PGDATA=/var/lib/postgresql/data/pgdata"]}, "Mounts": [
                         {"Type": "volume", "Name": "anonymous-parent",
                          "Destination": "/var/lib/postgresql"},
                         {"Type": "volume", "Name": volume["Name"],
                          "Destination": "/var/lib/postgresql/data"}]}
        with tempfile.TemporaryDirectory(prefix="nw-pgdata-mount-test-") as directory:
            root = Path(directory)
            log = root / "calls.jsonl"
            docker = root / "docker"
            docker.write_text(
                "#!/usr/bin/env python3\nimport json,os,sys\nfrom pathlib import Path\n"
                "with Path(os.environ['ISOLATION_TEST_LOG']).open('a') as log:\n"
                "    log.write(json.dumps(sys.argv[1:])+'\\n')\n"
                "if sys.argv[1] == 'inspect': print(os.environ['ISOLATION_CONTAINER'])\n"
                "elif sys.argv[1:3] == ['volume','inspect']: print(os.environ['ISOLATION_VOLUME'])\n"
                "else: sys.exit(92)\n"
            )
            docker.chmod(0o700)
            for destination in (None, "/var/lib/postgresql/data/pgdata",
                                "/var/lib/postgresql/data/pgdata/base"):
                with self.subTest(destination=destination):
                    metadata = dict(container, Mounts=list(container["Mounts"]))
                    if destination:
                        metadata["Mounts"].append({"Type": "volume", "Name": "redirected",
                                                   "Destination": destination})
                    log.unlink(missing_ok=True)
                    result = subprocess.run(
                        ["bash", "-c", "set -euo pipefail\n" + function + "\npgdata_identity"],
                        env=dict(ENV, PATH=str(root) + os.pathsep + ENV["PATH"],
                                 POSTGRES_CONTAINER=project + "-postgres", PYTHONOPTIMIZE="1",
                                 ISOLATION_TEST_LOG=str(log), ISOLATION_CONTAINER=json.dumps(metadata),
                                 ISOLATION_VOLUME=json.dumps([volume])),
                        capture_output=True, text=True, timeout=5,
                    )
                    calls = [json.loads(line) for line in log.read_text().splitlines()]
                    self.assertEqual(calls[0], ["inspect", "--format", "{{json .}}", project + "-postgres"])
                    if destination:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertIn("cannot identify the selected PGDATA volume", result.stderr)
                        self.assertEqual(len(calls), 1)
                        self.assertEqual(result.stdout, "")
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual(calls[1], ["volume", "inspect", volume["Name"]])
                        self.assertRegex(result.stdout, r"^[0-9a-f]{64}\n$")


if __name__ == "__main__":
    unittest.main()
