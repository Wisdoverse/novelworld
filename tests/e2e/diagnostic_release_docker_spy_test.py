import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock


SPY_PATH = Path(__file__).with_name("diagnostic_release_docker_spy.py")
SPEC = importlib.util.spec_from_file_location("diagnostic_release_docker_spy", SPY_PATH)
SPY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SPY)


class DockerSpyTest(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        self.trace = Path(self.tempdir.name) / "docker.jsonl"
        self.project = "nwq-abcdef1234"
        self.env = {
            "NWQ_REAL_DOCKER": "/usr/bin/docker",
            "NWQ_DOCKER_TRACE": str(self.trace),
            "NWQ_PROJECT": self.project,
        }

    def tearDown(self):
        self.tempdir.cleanup()

    def invoke(self, argv):
        with mock.patch.dict(os.environ, self.env, clear=True), \
             mock.patch.object(SPY.os, "execv", side_effect=RuntimeError("forwarded")) as execv:
            try:
                SPY.main(argv)
            except RuntimeError:
                pass
            return execv

    def compose(self, command):
        return ["compose", "--project-name", self.project, "--project-directory", "/repo",
                "-f", "/repo/docker-compose.yml", "-f", "/repo/docker-compose.release.yml",
                "--env-file", "/private/secrets", "--env-file", "/private/manifest", *command]

    def test_forwards_only_release_compose_config_and_pull(self):
        for command in (["config", "--format", "json"],
                        ["pull", "postgres-migrate", "gateway"]):
            argv = self.compose(command)
            execv = self.invoke(argv)
            execv.assert_called_once_with("/usr/bin/docker", ["/usr/bin/docker", *argv])

    def test_forwards_exact_capability_probe_and_cleanup(self):
        name = self.project + "-budget-probe-0123456789ab"
        image = "ghcr.io/wisdoverse/novelworld/user-service@sha256:" + "a" * 64
        probe = ["create", "--pull", "never", "--name", name,
                 "--network", "none", "--no-healthcheck", "--log-driver", "none",
                 "--read-only", "--cap-drop", "ALL", "--security-opt",
                 "no-new-privileges", "--entrypoint", "/app/service", image,
                 "--diagnostic-budget-contract"]
        for argv in (probe, ["start", "--attach", name], ["rm", "--force", name],
                     ["ps", "--all", "--quiet", "--filter", "name=^/" + name + "$"]):
            execv = self.invoke(argv)
            execv.assert_called_once_with("/usr/bin/docker", ["/usr/bin/docker", *argv])

    def test_rejects_dangerous_or_unscoped_calls_without_exec(self):
        for argv in (self.compose(["up", "-d"]), self.compose(["run", "user-service"]),
                     self.compose(["stop"]), ["version"],
                     ["run", "--rm", "--env", "SECRET=x"],
                     ["start", "--attach", "nwq-abcdef1234-budget-probe-ffffffffffff-extra"]):
            with mock.patch.dict(os.environ, self.env, clear=True), \
                 mock.patch.object(SPY.os, "execv") as execv:
                with self.assertRaises(SystemExit):
                    SPY.main(argv)
                execv.assert_not_called()

    def test_trace_records_rejected_argv_without_exposing_extra_output(self):
        argv = ["compose", "--project-name", self.project, "up"]
        with self.assertRaises(SystemExit):
            with mock.patch.dict(os.environ, self.env, clear=True):
                SPY.main(argv)
        records = [json.loads(line) for line in self.trace.read_text().splitlines()]
        self.assertEqual(records, [argv])


if __name__ == "__main__":
    unittest.main()
