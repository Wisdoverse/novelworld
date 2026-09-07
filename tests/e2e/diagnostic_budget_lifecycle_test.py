import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import json
import re
from types import SimpleNamespace
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tests/e2e/diagnostic_budget_lifecycle.py"
SPEC = importlib.util.spec_from_file_location("diagnostic_budget_lifecycle", SCRIPT)
assert SPEC and SPEC.loader
LIFECYCLE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LIFECYCLE)


class DiagnosticBudgetLifecycleCleanupTest(unittest.TestCase):
    def test_journey_isolation_changes_services_not_nested_dependencies(self):
        original = (ROOT / "docker-compose.yml").read_text()
        isolated = LIFECYCLE.isolated_journey_compose(original, "nwq-abcdef1234", Path("/private/ca.pem"), "10.254.241.14")
        self.assertEqual(isolated.count("      SSL_CERT_FILE: /fixture/ca.pem\n"), 4)
        for service in LIFECYCLE.SERVICES:
            block = re.search(r"(?ms)^  " + re.escape(service) + r":\n(.*?)(?=^  [a-z]|\Z)", isolated)[1]
            self.assertIn("      HTTPS_PROXY: http://mock:3128\n", block)
            self.assertIn('"/private/ca.pem:/fixture/ca.pem:ro"', block)
        self.assertEqual(re.findall(r"(?ms)^    depends_on:\n.*?(?=^    [a-z]|\Z)", original),
                         re.findall(r"(?ms)^    depends_on:\n.*?(?=^    [a-z]|\Z)", isolated))
        self.assertIn("  novel-net:\n    external: true\n    name: nwq-abcdef1234\n", isolated)
        self.assertIn("        ipv4_address: 10.254.241.14\n", isolated)
        self.assertNotIn("${NGINX_HTTP_PORT:-80}", isolated)

    def lifecycle(self, containers, network=True):
        lifecycle = LIFECYCLE.Lifecycle.__new__(LIFECYCLE.Lifecycle)
        lifecycle.project = "nwq-abcdef1234"
        lifecycle.containers = set(containers)
        lifecycle.image_refs = set()
        lifecycle.source_images = {}
        lifecycle.network_created = network
        lifecycle.temporary = tempfile.TemporaryDirectory(prefix="cleanup-test-")
        self.addCleanup(lifecycle.temporary.cleanup)
        lifecycle.files = Path(lifecycle.temporary.name)
        (lifecycle.files / "temporary").write_text("fixture")
        return lifecycle

    def test_cleanup_continues_after_container_timeout_and_fixes_failure(self):
        lifecycle = self.lifecycle(("owned-a", "owned-b"))
        calls = []

        def docker(*args, **kwargs):
            calls.append(args)
            if args[:2] == ("rm", "--force") and args[-1] == "owned-a":
                raise subprocess.TimeoutExpired(["docker", *args], 1)
            if args[:3] == ("network", "ls", "--filter"):
                return SimpleNamespace(returncode=0, stdout=(lifecycle.project + "\n").encode())
            return SimpleNamespace(returncode=0, stdout=b"")

        lifecycle.docker = docker
        temporary = Path(lifecycle.temporary.name)
        with self.assertRaises(LIFECYCLE.Failure):
            lifecycle.cleanup()

        self.assertIn(("rm", "--force", "--volumes", "owned-a"), calls)
        self.assertIn(("rm", "--force", "--volumes", "owned-b"), calls)
        self.assertIn(("network", "rm", lifecycle.project), calls)
        self.assertFalse(temporary.exists())

    def test_cleanup_removes_all_owned_resources_without_error(self):
        lifecycle = self.lifecycle(("owned-a", "owned-b"))
        network_present = True

        def docker(*args, **_):
            nonlocal network_present
            if args[:2] == ("network", "rm"):
                network_present = False
            return SimpleNamespace(returncode=0, stdout=(lifecycle.project + "\n").encode()
                if args[:3] == ("network", "ls", "--filter") and network_present else b"")

        lifecycle.docker = mock.Mock(side_effect=docker)
        temporary = Path(lifecycle.temporary.name)

        lifecycle.cleanup()

        self.assertFalse(temporary.exists())
        lifecycle.docker.assert_any_call("rm", "--force", "--volumes", "owned-a", check=False)
        lifecycle.docker.assert_any_call("rm", "--force", "--volumes", "owned-b", check=False)
        lifecycle.docker.assert_any_call("network", "rm", lifecycle.project, check=False)
        self.assertEqual(lifecycle.docker.call_count, 7)
        lifecycle.docker.assert_any_call("ps", "--all", "--quiet", "--filter", "name=^/owned-a$")
        lifecycle.docker.assert_any_call("ps", "--all", "--quiet", "--filter", "name=^/owned-b$")

    def test_child_terminal_failure_cleanup_removes_exact_containers_never_named_pg_volume(self):
        lifecycle = self.lifecycle((), network=False)
        project, identifier = "nwq-0123456789", "a" * 64
        lifecycle.journeys = [SimpleNamespace(project=project)]

        def docker(*args, **_):
            return SimpleNamespace(returncode=0, stdout=(f"{identifier} {project}-postgres\n".encode()
                if args[:3] == ("ps", "--all", "--no-trunc") else b""))

        lifecycle.docker = mock.Mock(side_effect=docker)
        lifecycle.cleanup()
        lifecycle.docker.assert_any_call("rm", "--force", "--volumes", identifier, check=False)
        self.assertFalse(any(call.args[:2] == ("volume", "rm") for call in lifecycle.docker.call_args_list))

    def test_container_remove_ack_without_absence_is_failure(self):
        lifecycle = self.lifecycle(("owned",), network=False)
        lifecycle.docker = mock.Mock(side_effect=lambda *args, **_: SimpleNamespace(
            returncode=0, stdout=b"still-present" if args[0] == "ps" else b""))
        with self.assertRaises(LIFECYCLE.Failure):
            lifecycle.cleanup()

    def test_shared_external_network_remove_ack_requires_absence(self):
        lifecycle = self.lifecycle((), network=True)
        lifecycle.journeys = [SimpleNamespace(project="nwq-0123456789")]
        lifecycle.docker = mock.Mock(side_effect=lambda *args, **_: SimpleNamespace(
            returncode=0, stdout=(lifecycle.project + "\n").encode()
            if args[:2] == ("network", "ls") else b""))
        with self.assertRaises(LIFECYCLE.Failure):
            lifecycle.cleanup()
        lifecycle.docker.assert_any_call("network", "rm", lifecycle.project, check=False)
        self.assertFalse(any(call.args[:2] == ("volume", "rm") for call in lifecycle.docker.call_args_list))

    def test_image_cleanup_continues_after_timeout_and_skips_missing_image(self):
        lifecycle = self.lifecycle((), network=False)
        lifecycle.image_refs = {
            "registry/nwq-abcdef1234/service-a:fixture",
            "registry/nwq-abcdef1234/service-a@sha256:" + "a" * 64,
            "registry/nwq-abcdef1234/service-b:fixture",
        }
        ordered = sorted(lifecycle.image_refs)
        missing = ordered[1]
        timed_out = ordered[0]
        calls = []

        def docker(*args, **kwargs):
            calls.append(args)
            if args[:2] == ("image", "ls"):
                return SimpleNamespace(
                    returncode=0,
                    stdout=(" ".join(reference for reference in ordered if reference != missing) + "\n").encode(),
                )
            if args[:3] == ("image", "rm", timed_out):
                raise subprocess.TimeoutExpired(["docker", *args], 1)
            return SimpleNamespace(returncode=0, stdout=b"")

        lifecycle.docker = docker
        temporary = Path(lifecycle.temporary.name)
        with self.assertRaises(LIFECYCLE.Failure):
            lifecycle.cleanup()

        self.assertFalse(temporary.exists())
        self.assertIn(("image", "ls", "--digests", "--format", "{{.Repository}}:{{.Tag}} {{.Repository}}@{{.Digest}}"), calls)
        self.assertIn(("image", "rm", timed_out), calls)
        self.assertEqual(sum(call[:2] == ("image", "ls") for call in calls), 3)
        self.assertNotIn(("image", "rm", missing), calls)
        self.assertIn(("image", "rm", ordered[2]), calls)

    def test_image_list_failure_continues_cleanup_and_fixes_failure(self):
        lifecycle = self.lifecycle(("owned",), network=True)
        lifecycle.image_refs = {
            "registry/nwq-abcdef1234/service-a:fixture",
            "registry/nwq-abcdef1234/service-b:fixture",
        }
        calls = []

        def docker(*args, **kwargs):
            calls.append(args)
            if args[:2] == ("image", "ls"):
                raise subprocess.TimeoutExpired(["docker", *args], 1)
            if args[:3] == ("network", "ls", "--filter"):
                return SimpleNamespace(returncode=0, stdout=(lifecycle.project + "\n").encode())
            return SimpleNamespace(returncode=0, stdout=b"")

        lifecycle.docker = docker
        temporary = Path(lifecycle.temporary.name)
        with self.assertRaises(LIFECYCLE.Failure):
            lifecycle.cleanup()

        self.assertFalse(temporary.exists())
        self.assertIn(("rm", "--force", "--volumes", "owned"), calls)
        self.assertIn(("network", "rm", lifecycle.project), calls)
        self.assertEqual(sum(call[:2] == ("image", "ls") for call in calls), 2)

    def test_help_is_cli_only_and_does_not_invoke_docker(self):
        with mock.patch.object(LIFECYCLE, "command") as command, mock.patch.object(
            sys, "argv", [str(SCRIPT), "--help"]
        ):
            with self.assertRaises(SystemExit) as exit_info:
                LIFECYCLE.main()
        self.assertEqual(exit_info.exception.code, 0)
        command.assert_not_called()

    def test_cleanup_preserves_and_verifies_source_image_identity(self):
        for actual in ("original-id", "changed-id"):
            with self.subTest(actual=actual):
                lifecycle = self.lifecycle((), network=False)
                lifecycle.source_images = {"original/service:release": "original-id"}
                lifecycle.docker = mock.Mock(return_value=SimpleNamespace(returncode=0, stdout=actual.encode()))
                if actual == "original-id":
                    lifecycle.cleanup()
                else:
                    with self.assertRaises(LIFECYCLE.Failure):
                        lifecycle.cleanup()
                lifecycle.docker.assert_called_once_with("image", "inspect", "--format", "{{.Id}}", "original/service:release")
                self.assertFalse(lifecycle.files.exists())

    def test_runtime_images_are_passed_to_exercise_without_capability_probe(self):
        images = {
            "user-service": "registry/user@sha256:" + "a" * 64,
            "novel-service": "registry/novel@sha256:" + "b" * 64,
            "agent-service": "registry/agent@sha256:" + "c" * 64,
            "narrative-service": "registry/narrative@sha256:" + "d" * 64,
        }
        with tempfile.TemporaryDirectory() as directory:
            image_file = Path(directory) / "runtime.json"
            image_file.write_text(json.dumps(images))
            fake = mock.Mock()
            with mock.patch.object(LIFECYCLE, "Lifecycle", return_value=fake) as lifecycle, \
                 mock.patch.object(sys, "argv", [
                     str(SCRIPT), "--client-binary", "/tmp/client",
                     "--runtime-images", str(image_file),
                 ]), mock.patch.object(LIFECYCLE.signal, "signal"):
                LIFECYCLE.main()

        lifecycle.assert_called_once_with(None, Path("/tmp/client"), None)
        fake.exercise.assert_called_once_with(images)
        fake.prepare_images.assert_not_called()
        fake.preflight_images.assert_not_called()
        fake.cleanup.assert_called_once_with()

    def test_invalid_image_mode_combinations_create_no_lifecycle(self):
        with tempfile.TemporaryDirectory() as directory:
            runtime = Path(directory) / "runtime.json"
            capability = Path(directory) / "capability.json"
            runtime.write_text("{}")
            capability.write_text("{}")
            cases = (
                ["--journey-images", str(runtime)],
                ["--journey-output", directory],
                ["--journey-images", str(runtime), "--journey-output", directory,
                 "--client-binary", "/tmp/client"],
                ["--runtime-images", str(runtime)],
                ["--owner-binary", "/tmp/owner", "--client-binary", "/tmp/client",
                 "--runtime-images", str(runtime)],
                ["--client-binary", "/tmp/client", "--capability-images", str(capability),
                 "--runtime-images", str(runtime)],
                ["--client-binary", "/tmp/client", "--runtime-images", str(runtime),
                 "--unexpected"],
            )
            for arguments in cases:
                with self.subTest(arguments=arguments), \
                     mock.patch.object(LIFECYCLE, "Lifecycle") as lifecycle, \
                     mock.patch.object(sys, "argv", [str(SCRIPT), *arguments]):
                    with self.assertRaises((LIFECYCLE.Failure, SystemExit)):
                        LIFECYCLE.main()
                    lifecycle.assert_not_called()

    def ingress_lifecycle(self, containers=(), network=True):
        lifecycle = self.lifecycle(containers, network)
        lifecycle.nginx_ip = "10.254.241.14"
        lifecycle.ingresses = []
        return lifecycle

    @staticmethod
    def ingress_network(ip=None):
        return SimpleNamespace(stdout=json.dumps([{
            "Name": "nwq-abcdef1234",
            "Internal": True,
            "Containers": ({
                "container-id": {"IPv4Address": ip + "/28"},
            } if ip else {}),
        }]).encode())

    def test_start_ingress_rejects_reserved_address_without_starting_process(self):
        lifecycle = self.ingress_lifecycle()
        lifecycle.docker = mock.Mock(return_value=self.ingress_network(lifecycle.nginx_ip))
        with mock.patch.object(LIFECYCLE.subprocess, "Popen") as popen:
            with self.assertRaises(LIFECYCLE.Failure):
                lifecycle.start_ingress(80)
        popen.assert_not_called()
        self.assertEqual(lifecycle.ingresses, [])

    def test_start_ingress_uses_loopback_fixed_port_and_new_process_group(self):
        lifecycle = self.ingress_lifecycle()
        lifecycle.docker = mock.Mock(return_value=self.ingress_network())
        listener = mock.MagicMock()
        listener.__enter__.return_value = listener
        connection = mock.MagicMock()
        connection.__enter__.return_value = connection
        process = mock.Mock(pid=1234)
        process.poll.return_value = None
        with mock.patch.object(LIFECYCLE.shutil, "which", return_value="/usr/bin/socat"), \
             mock.patch.object(LIFECYCLE.subprocess, "Popen", return_value=process) as popen, \
             mock.patch.object(LIFECYCLE.socket, "socket", return_value=listener) as socket_factory, \
             mock.patch.object(LIFECYCLE.socket, "create_connection", return_value=connection):
            lifecycle.start_ingress(80)
        popen.assert_called_once_with(
            ["/usr/bin/socat", "-T", "10",
             "TCP4-LISTEN:80,bind=127.0.0.1,reuseaddr,fork",
             "TCP4:10.254.241.14:80,connect-timeout=2"],
            stdin=LIFECYCLE.subprocess.DEVNULL,
            stdout=LIFECYCLE.subprocess.DEVNULL,
            stderr=LIFECYCLE.subprocess.DEVNULL,
            start_new_session=True,
        )
        socket_factory.assert_called_once_with()
        listener.bind.assert_called_once_with(("127.0.0.1", 80))
        self.assertEqual(lifecycle.ingresses, [(process, 80)])

    def test_start_failure_keeps_process_tracked_for_final_cleanup(self):
        lifecycle = self.ingress_lifecycle()
        lifecycle.docker = mock.Mock(return_value=self.ingress_network())
        listener = mock.MagicMock()
        listener.__enter__.return_value = listener
        process = mock.Mock(pid=1234)
        process.poll.return_value = 1
        with mock.patch.object(LIFECYCLE.subprocess, "Popen", return_value=process), \
             mock.patch.object(LIFECYCLE.socket, "socket", return_value=listener):
            with self.assertRaises(LIFECYCLE.Failure):
                lifecycle.start_ingress(80)
        self.assertEqual(lifecycle.ingresses, [(process, 80)])

    def test_stop_ingresses_kills_reaps_and_retains_failed_listener(self):
        lifecycle = self.ingress_lifecycle()
        process = mock.Mock(pid=4321)
        lifecycle.ingresses = [(process, 80)]
        listener = mock.MagicMock()
        listener.__enter__.return_value = listener
        listener.bind.side_effect = OSError("still listening")
        with mock.patch.object(LIFECYCLE.os, "killpg") as killpg, \
             mock.patch.object(LIFECYCLE.socket, "socket", return_value=listener):
            with self.assertRaises(LIFECYCLE.Failure):
                lifecycle.stop_ingresses()
        killpg.assert_called_once_with(4321, LIFECYCLE.signal.SIGKILL)
        process.wait.assert_called_once_with(timeout=5)
        self.assertEqual(lifecycle.ingresses, [(process, 80)])

    def test_cleanup_continues_resources_after_ingress_failure(self):
        lifecycle = self.ingress_lifecycle(("owned",), network=True)
        calls = []

        def docker(*args, **kwargs):
            calls.append(args)
            if args[:2] == ("network", "ls"):
                return SimpleNamespace(returncode=0, stdout=(lifecycle.project + "\n").encode())
            return SimpleNamespace(returncode=0, stdout=b"")

        lifecycle.docker = docker
        with mock.patch.object(lifecycle, "stop_ingresses", side_effect=LIFECYCLE.Failure("ingress")):
            with self.assertRaises(LIFECYCLE.Failure):
                lifecycle.cleanup()
        self.assertIn(("rm", "--force", "--volumes", "owned"), calls)
        self.assertIn(("network", "rm", lifecycle.project), calls)
        self.assertFalse(lifecycle.files.exists())


if __name__ == "__main__":
    unittest.main()
