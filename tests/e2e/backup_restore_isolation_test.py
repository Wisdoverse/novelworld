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
        for overrides, ports, prefix in (
            ({}, (18080, 18081), "novel"),
            ({"COMPOSE_PROJECT_NAME": "nw-isolation-test",
              "CONTAINER_PREFIX": "nw-isolation-test",
              "E2E_LLM_STUB_PORT": "28080", "E2E_GATEWAY_PORT": "28081",
              "NGINX_HTTP_BIND": "127.0.0.1", "NGINX_HTTP_PORT": "28082"},
             (28080, 28081), "nw-isolation-test"),
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
                    self.assertEqual(config["name"], "nw-isolation-test")
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


if __name__ == "__main__":
    unittest.main()
