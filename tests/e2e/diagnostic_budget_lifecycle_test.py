import importlib.util
import ipaddress
import os
import contextlib
import io
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
        lifecycle.subnet = None
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
        lifecycle.nginx_ip = ipaddress.ip_address("10.254.241.14")
        lifecycle.ingresses = []
        evidence = tempfile.TemporaryDirectory(prefix="ingress-evidence-")
        self.addCleanup(evidence.cleanup)
        lifecycle.ingress_evidence = Path(evidence.name)
        return lifecycle

    @staticmethod
    def ingress_network(*, gateway=None, auxiliary=None, container=None):
        reserved = {}
        if container is not None:
            reserved["container-id"] = {"IPv4Address": str(container) + "/28"}
        return SimpleNamespace(stdout=json.dumps([{
            "Name": "nwq-abcdef1234",
            "Id": "network-id",
            "Internal": True,
            "IPAM": {"Config": [{
                "Subnet": "10.254.241.0/28",
                "Gateway": gateway,
                "AuxiliaryAddresses": auxiliary or {},
            }]},
            "Containers": reserved,
        }]).encode())

    def test_start_ingress_rejects_reserved_address_without_starting_process(self):
        for kind in ("gateway", "auxiliary", "container"):
            with self.subTest(kind=kind):
                lifecycle = self.ingress_lifecycle()
                occupied = {kind: str(lifecycle.nginx_ip)}
                if kind == "auxiliary":
                    occupied[kind] = {"nginx": str(lifecycle.nginx_ip)}
                lifecycle.docker = mock.Mock(return_value=self.ingress_network(**occupied))
                with mock.patch.object(LIFECYCLE.subprocess, "Popen") as popen:
                    with self.assertRaises(LIFECYCLE.Failure):
                        lifecycle.start_ingress(80)
                popen.assert_not_called()
                self.assertEqual(lifecycle.ingresses, [])

    @staticmethod
    def fake_runner():
        def write_private(path, value):
            descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(value)
        diagnostic = SimpleNamespace(
            canonical=lambda value: json.dumps(value, sort_keys=True).encode(),
            sync_directory=lambda _path: None,
        )
        return SimpleNamespace(write_private=write_private, diagnostic=diagnostic)

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
             mock.patch.object(LIFECYCLE.socket, "create_connection", return_value=connection), \
             mock.patch.dict(sys.modules, {"live_deepseek_journey": self.fake_runner()}):
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
        metadata = json.loads((lifecycle.ingress_evidence / "ingress-80.json").read_bytes())
        self.assertEqual(metadata, {"bind": "127.0.0.1", "network_id": "network-id",
                                    "network_name": lifecycle.project, "nginx_ip": str(lifecycle.nginx_ip),
                                    "phase": "started; absence must be independently verified",
                                    "pgid": 1234, "pid": 1234, "port": 80, "target_port": 80})
        self.assertEqual((lifecycle.ingress_evidence / "ingress-80.json").stat().st_mode & 0o777, 0o600)

    def test_start_failure_keeps_process_tracked_for_final_cleanup(self):
        lifecycle = self.ingress_lifecycle()
        lifecycle.docker = mock.Mock(return_value=self.ingress_network())
        listener = mock.MagicMock()
        listener.__enter__.return_value = listener
        process = mock.Mock(pid=1234)
        process.poll.return_value = 1
        with mock.patch.object(LIFECYCLE.subprocess, "Popen", return_value=process), \
             mock.patch.object(LIFECYCLE.socket, "socket", return_value=listener), \
             mock.patch.dict(sys.modules, {"live_deepseek_journey": self.fake_runner()}):
            with self.assertRaises(LIFECYCLE.Failure):
                lifecycle.start_ingress(80)
        self.assertEqual(lifecycle.ingresses, [(process, 80)])

    def test_start_metadata_failure_keeps_process_tracked(self):
        lifecycle = self.ingress_lifecycle()
        lifecycle.docker = mock.Mock(return_value=self.ingress_network())
        listener = mock.MagicMock()
        listener.__enter__.return_value = listener
        connection = mock.MagicMock()
        connection.__enter__.return_value = connection
        process = mock.Mock(pid=1234)
        process.poll.return_value = None
        runner = self.fake_runner()
        runner.write_private = mock.Mock(side_effect=OSError("evidence write failed"))
        with mock.patch.object(LIFECYCLE.subprocess, "Popen", return_value=process), \
             mock.patch.object(LIFECYCLE.socket, "socket", return_value=listener), \
             mock.patch.object(LIFECYCLE.socket, "create_connection", return_value=connection), \
             mock.patch.dict(sys.modules, {"live_deepseek_journey": runner}):
            with self.assertRaises(OSError):
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
        with mock.patch.object(lifecycle, "stop_ingresses",
                               side_effect=[LIFECYCLE.Failure("ingress"), LIFECYCLE.Failure("ingress")]):
            with self.assertRaises(LIFECYCLE.Failure):
                lifecycle.cleanup()
        self.assertIn(("rm", "--force", "--volumes", "owned"), calls)
        self.assertIn(("network", "rm", lifecycle.project), calls)
        self.assertFalse(lifecycle.files.exists())

    def test_cleanup_retries_ingress_failure_before_completing(self):
        lifecycle = self.ingress_lifecycle(("owned",), network=True)
        network_present = True

        def docker(*args, **_):
            nonlocal network_present
            if args[:2] == ("network", "rm"):
                network_present = False
            return SimpleNamespace(returncode=0, stdout=(lifecycle.project + "\n").encode()
                                   if args[:2] == ("network", "ls") and network_present else b"")

        lifecycle.docker = mock.Mock(side_effect=docker)
        with mock.patch.object(lifecycle, "stop_ingresses",
                               side_effect=[LIFECYCLE.Failure("ingress"), None]) as stop:
            lifecycle.cleanup()
        self.assertEqual(stop.call_count, 2)

    def test_cold_adoption_status_is_private_boolean_summary_and_best_effort(self):
        def journey(complete=True):
            private = {
                "release_images": {"base": {"secret_id": "private-image-id"}},
                "diagnostic_budget_snapshots": {
                    name: {"private_budget_id": "private-budget-id"}
                    for name in ("initial", "settings", "restart", "terminal")
                },
                "diagnostic_payers_stopped": True if complete else "yes",
                "diagnostic_metrics_reconciled": True if complete else 1,
                "unknown_private_field": "must-not-be-emitted",
            }
            report = {"environment": {
                "existing_user_stack_unchanged": True if complete else "yes",
                "internal_id": "private-report-id",
            }}
            return SimpleNamespace(private_report=private, report=report)

        def read_line(text):
            return next(json.loads(line) for line in text.splitlines()
                        if line.lstrip().startswith("{"))

        expected_keys = {
            "case", "base_images_recorded", "initial_snapshot_present",
            "settings_snapshot_present", "restart_snapshot_present",
            "terminal_snapshot_present", "payers_stopped", "metrics_reconciled",
            "existing_stack_unchanged",
        }
        for case in ("zero", "nonzero"):
            with self.subTest(case=case):
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    LIFECYCLE.report_cold_adoption_status(journey(), case == "zero")
                encoded = output.getvalue()
                summary = read_line(encoded)
                self.assertEqual(set(summary), expected_keys)
                self.assertEqual(summary["case"], case)
                self.assertTrue(all(isinstance(summary[key], bool) for key in expected_keys - {"case"}))
                self.assertNotIn("private-image-id", encoded)
                self.assertNotIn("private-budget-id", encoded)
                self.assertNotIn("must-not-be-emitted", encoded)

        for malformed in (
            SimpleNamespace(private_report={}, report={}),
            SimpleNamespace(private_report={"release_images": [], "diagnostic_budget_snapshots": "bad",
                                            "diagnostic_payers_stopped": True,
                                            "diagnostic_metrics_reconciled": True},
                            report={"environment": {"existing_user_stack_unchanged": True}}),
        ):
            with self.subTest(malformed=malformed):
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    LIFECYCLE.report_cold_adoption_status(malformed, False)
                summary = read_line(output.getvalue())
                self.assertEqual(set(summary), expected_keys)
                if not malformed.private_report:
                    self.assertFalse(any(summary[key] for key in expected_keys - {"case"}))
                else:
                    self.assertFalse(summary["base_images_recorded"])
                    self.assertFalse(any(summary[name + "_snapshot_present"]
                                         for name in ("initial", "settings", "restart", "terminal")))
                    self.assertTrue(summary["payers_stopped"])
                    self.assertTrue(summary["metrics_reconciled"])
                    self.assertTrue(summary["existing_stack_unchanged"])

        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            LIFECYCLE.report_cold_adoption_status(journey(complete=False), False)
        summary = read_line(output.getvalue())
        self.assertTrue(all(isinstance(summary[key], bool) for key in expected_keys - {"case"}))
        self.assertTrue(summary["base_images_recorded"])
        self.assertTrue(all(summary[name + "_snapshot_present"]
                            for name in ("initial", "settings", "restart", "terminal")))
        self.assertFalse(summary["payers_stopped"])
        self.assertFalse(summary["metrics_reconciled"])
        self.assertFalse(summary["existing_stack_unchanged"])

        with mock.patch("builtins.print", side_effect=BrokenPipeError):
            LIFECYCLE.report_cold_adoption_status(journey(), False)

    def test_cold_release_status_is_allowlisted_and_bounded(self):
        phases = ("pull", "database_start", "migration", "application_deployment", "readiness")
        refusal_lines = (
            "release: diagnostic budget preflight failed",
            "release: working tree is not clean",
            "release: diagnostic provisioning uncertain; attempt frozen",
        )
        expected_keys = {
            "case", "release_log_present", "release_log_complete",
            *(phase + "_" + boundary for phase in phases for boundary in ("start", "end")),
            "preflight_refused", "worktree_dirty", "provision_uncertain",
            "capability_probe_failed", "curl_failed", "release_adopt_failed",
            "image_identity_failed",
        }
        legal = "\n".join(
            [*(f"qualification-phase {phase} {boundary} 1"
               for phase in phases for boundary in ("start", "end")),
             *refusal_lines, "diagnostic phase=probe_cleanup_ps",
             "curl: (7) private-key=/private/key path=/private/report id=secret-id"]
        ) + "\n"
        with tempfile.TemporaryDirectory() as directory:
            output_dir = Path(directory)
            (output_dir / "release-adopt.log").write_text(legal)
            journey = SimpleNamespace(
                output=output_dir,
                diagnostic_failures=["release_adopt_failed", "release_image_identity_mismatch"],
            )
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                LIFECYCLE.report_cold_release_status(journey, True)
            encoded = output.getvalue()
            summary = next(json.loads(line) for line in encoded.splitlines()
                            if line.lstrip().startswith("{"))
            self.assertEqual(set(summary), expected_keys)
            self.assertEqual(summary["case"], "zero")
            self.assertTrue(all(isinstance(summary[key], bool) for key in expected_keys - {"case"}))
            self.assertTrue(all(summary[phase + "_" + boundary]
                                for phase in phases for boundary in ("start", "end")))
            self.assertTrue(all(summary[key] for key in ("preflight_refused", "worktree_dirty",
                                                         "provision_uncertain", "capability_probe_failed",
                                                         "curl_failed", "release_adopt_failed",
                                                         "image_identity_failed")))
            for private in ("private-key", "/private/key", "/private/report", "secret-id"):
                self.assertNotIn(private, encoded)

            (output_dir / "release-adopt.log").write_bytes(b"qualification-phase unknown start 1\n"
                                                            b"diagnostic phase=probe_unknown\n"
                                                            b"release: unknown refusal\n")
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                LIFECYCLE.report_cold_release_status(SimpleNamespace(output=output_dir,
                                                                      diagnostic_failures=[]), False)
            summary = next(json.loads(line) for line in output.getvalue().splitlines()
                           if line.lstrip().startswith("{"))
            self.assertEqual(set(summary), expected_keys)
            self.assertFalse(any(summary[key] for key in expected_keys - {"case", "release_log_present",
                                                                            "release_log_complete"}))
            self.assertTrue(summary["release_log_present"] and summary["release_log_complete"])

            (output_dir / "release-adopt.log").write_bytes(b"x" * (1048576 + 1))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                LIFECYCLE.report_cold_release_status(SimpleNamespace(output=output_dir,
                                                                      diagnostic_failures=[]), False)
            summary = next(json.loads(line) for line in output.getvalue().splitlines()
                           if line.lstrip().startswith("{"))
            self.assertTrue(summary["release_log_present"])
            self.assertFalse(summary["release_log_complete"])
            self.assertFalse(any(summary[key] for key in expected_keys - {"case", "release_log_present"}))

            (output_dir / "release-adopt.log").unlink()
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                LIFECYCLE.report_cold_release_status(SimpleNamespace(output=output_dir,
                                                                      diagnostic_failures=[]), False)
            summary = next(json.loads(line) for line in output.getvalue().splitlines()
                           if line.lstrip().startswith("{"))
            self.assertFalse(any(summary[key] for key in expected_keys - {"case"}))

        with mock.patch("builtins.print", side_effect=BrokenPipeError):
            LIFECYCLE.report_cold_release_status(SimpleNamespace(output=Path("/missing"),
                                                                  diagnostic_failures=[]), False)

    def test_cold_startup_status_is_bounded_allowlisted_and_foreign_safe(self):
        services = (*LIFECYCLE.SERVICES, "gateway", "frontend", "nginx")
        expected_fields = {
            "observed", "present", "running", "health", "exit_nonzero",
            "oom_killed", "logs_observed", "budget_invalid", "budget_unavailable",
            "budget_control_failed", "permission_denied", "certificate",
            "client_builder_error", "panicked",
        }
        project = "nwq-abcdef1234"
        names = [project + "-" + service for service in services if service != "nginx"]
        foreign = project + "-gateway"
        verified_ids = {name: f"{index:064x}" for index, name in enumerate(names, 1)}
        calls = []
        control = SimpleNamespace()

        def bounded(command, **kwargs):
            calls.append((command, kwargs))
            if command[:4] == ["docker", "ps", "--all", "--format"]:
                return ("\n".join(names)).encode()
            if command[:3] == ["docker", "container", "inspect"]:
                name = command[-1]
                value = {
                    "id": verified_ids[name], "name": "/" + name,
                    "project": "foreign-project" if name == foreign else project,
                    "running": True, "exit": 0, "oom": False,
                    "health": "healthy",
                }
                return json.dumps(value).encode()
            if command[:3] == ["sh", "-c", "exec docker logs --tail 80 \"$1\" 2>&1"]:
                self.assertIn(command[-1], verified_ids.values())
                return b"permission denied certificate builder error panicked private-key=/secret/key"
            raise AssertionError(command)

        control.bounded_command = bounded
        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.dict(sys.modules, {"diagnostic_journey": control}), \
             mock.patch.object(LIFECYCLE.time, "monotonic", side_effect=[0] + [14] * 100), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            journey = SimpleNamespace(project=project, prefix=project)
            LIFECYCLE.report_cold_startup_status(journey, False)
        summary = json.loads(output.getvalue())
        self.assertEqual(set(summary), {"case", "startup_observation_unproven", "services"})
        self.assertEqual(set(summary["services"]), set(services))
        self.assertTrue(all(set(item) == expected_fields for item in summary["services"].values()))
        self.assertEqual(summary["services"]["nginx"]["observed"], True)
        self.assertFalse(summary["services"]["nginx"]["present"])
        self.assertTrue(summary["startup_observation_unproven"])
        self.assertTrue(all(item["health"] in {"healthy", "unhealthy", "starting", "none", "unknown"}
                            for item in summary["services"].values()))
        logs = [command for command, _ in calls if command[:2] == ["sh", "-c"]]
        self.assertTrue(logs)
        self.assertNotIn(verified_ids[foreign], [command[-1] for command in logs])
        self.assertTrue(all(kwargs["timeout"] <= 2 and kwargs["maximum"] == 65536
                            for _, kwargs in calls))
        self.assertNotIn("private-key", output.getvalue())
        self.assertNotIn("/secret/key", output.getvalue())
        self.assertNotIn(f"{project}-gateway", output.getvalue())

        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.dict(sys.modules, {"diagnostic_journey": control}), \
             mock.patch.object(LIFECYCLE.time, "monotonic", return_value=0):
            control.bounded_command = mock.Mock(side_effect=lambda command, **_: b"not-json"
                                                if command[1] == "container" else b"\n".join(
                                                    name.encode() for name in names))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                LIFECYCLE.report_cold_startup_status(SimpleNamespace(project=project, prefix=project), True)
            malformed = json.loads(output.getvalue())
            self.assertTrue(malformed["startup_observation_unproven"])

        class TimeoutFailure(Exception):
            pass

        control.bounded_command = mock.Mock(side_effect=TimeoutFailure("timeout"))
        with mock.patch.dict(sys.modules, {"diagnostic_journey": control}), \
             mock.patch.object(LIFECYCLE.time, "monotonic", return_value=0):
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                LIFECYCLE.report_cold_startup_status(SimpleNamespace(project=project, prefix=project), False)
            self.assertTrue(json.loads(output.getvalue())["startup_observation_unproven"])

        control.bounded_command = mock.Mock()
        with mock.patch.dict(sys.modules, {"diagnostic_journey": control}), \
             mock.patch.object(LIFECYCLE.time, "monotonic", side_effect=[0, 16]):
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                LIFECYCLE.report_cold_startup_status(SimpleNamespace(project=project, prefix=project), False)
        self.assertTrue(json.loads(output.getvalue())["startup_observation_unproven"])
        control.bounded_command.assert_not_called()

    def test_cold_adopt_release_preserves_original_error_and_skips_on_interrupt(self):
        control_spec = importlib.util.spec_from_file_location(
            "diagnostic_journey", ROOT / "tests/e2e/diagnostic_journey.py"
        )
        self.assertIsNotNone(control_spec and control_spec.loader)
        control = importlib.util.module_from_spec(control_spec)
        control_spec.loader.exec_module(control)
        live_spec = importlib.util.spec_from_file_location(
            "live_deepseek_journey_wrapper_test", ROOT / "tests/e2e/live_deepseek_journey.py"
        )
        self.assertIsNotNone(live_spec and live_spec.loader)
        with mock.patch.dict(sys.modules, {"diagnostic_journey": control}):
            live = importlib.util.module_from_spec(live_spec)
            live_spec.loader.exec_module(live)

        original = live.QualificationFailure("release_adopt_failed")
        journey = SimpleNamespace(release=mock.Mock(side_effect=original))
        with mock.patch.object(LIFECYCLE, "report_cold_startup_status",
                               side_effect=BrokenPipeError) as observe:
            with self.assertRaises(RuntimeError) as raised:
                LIFECYCLE.cold_adopt_release(journey, Path("/private/manifest"), False)
        self.assertIs(raised.exception, original)
        observe.assert_called_once()

        cancelled = live.QualificationFailure("diagnostic_cancelled")
        journey.release.side_effect = cancelled
        with mock.patch.object(LIFECYCLE, "report_cold_startup_status") as observe:
            with self.assertRaises(type(cancelled)) as raised:
                LIFECYCLE.cold_adopt_release(journey, Path("/private/manifest"), False)
        self.assertIs(raised.exception, cancelled)
        observe.assert_not_called()

        journey.release.side_effect = KeyboardInterrupt()
        with mock.patch.object(LIFECYCLE, "report_cold_startup_status") as observe:
            with self.assertRaises(KeyboardInterrupt):
                LIFECYCLE.cold_adopt_release(journey, Path("/private/manifest"), False)
        observe.assert_not_called()

    def test_journey_mode_requires_subnet_before_reading_inputs_or_constructing_lifecycle(self):
        with tempfile.TemporaryDirectory() as directory:
            image_path = Path(directory) / "missing-images.json"
            output_path = Path(directory) / "evidence"
            with mock.patch.object(LIFECYCLE, "Lifecycle") as lifecycle, \
                 mock.patch.object(sys, "argv", [str(SCRIPT), "--journey-images", str(image_path),
                                                   "--journey-output", str(output_path)]):
                with self.assertRaises(LIFECYCLE.Failure) as failure:
                    LIFECYCLE.main()
            self.assertEqual(str(failure.exception), "journey_static_ingress_requires_explicit_subnet")
            lifecycle.assert_not_called()

    def test_journey_wiring_requires_subnet_without_docker_or_evidence_write(self):
        lifecycle = self.ingress_lifecycle(network=False)
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory) / "evidence"
            evidence.mkdir(mode=0o700)
            lifecycle.docker = mock.Mock()
            with self.assertRaises(LIFECYCLE.Failure) as failure:
                lifecycle.journey_wiring(None, evidence)
            self.assertEqual(str(failure.exception), "journey_static_ingress_requires_explicit_subnet")
            lifecycle.docker.assert_not_called()
            self.assertEqual(list(evidence.iterdir()), [])

    def test_explicit_subnet_rejects_docker_overlap_and_host_route_before_create(self):
        for route_kind in ("docker", "host"):
            with self.subTest(route_kind=route_kind):
                lifecycle = self.ingress_lifecycle(network=False)
                lifecycle.subnet = "10.254.241.0/28"
                network = [{"IPAM": {"Config": [{"Subnet": "10.254.241.0/28"}]}}]
                if route_kind == "docker":
                    routes = []
                else:
                    network = [{"IPAM": {"Config": []}}]
                    routes = [{"dst": "10.254.241.0/28"}]

                def docker(*args, **_kwargs):
                    if args[:3] == ("network", "ls", "--quiet"):
                        return SimpleNamespace(stdout=b"network-id\n")
                    if args[:2] == ("network", "inspect"):
                        return SimpleNamespace(stdout=json.dumps(network).encode())
                    return SimpleNamespace(stdout=b"", returncode=0)

                lifecycle.docker = mock.Mock(side_effect=docker)
                with mock.patch.object(LIFECYCLE, "command",
                                       return_value=SimpleNamespace(stdout=json.dumps(routes).encode())) as command:
                    with self.assertRaises(LIFECYCLE.Failure) as failure:
                        lifecycle.prepare_network_mock()
                self.assertEqual(str(failure.exception), "fixture_subnet_overlaps_existing_route")
                self.assertFalse(any(call.args[:2] == ("network", "create")
                                     for call in lifecycle.docker.call_args_list))
                if route_kind == "docker":
                    command.assert_called_once_with(["ip", "-j", "route"])

    def test_cleanup_docker_calls_are_clamped_to_remaining_deadline(self):
        lifecycle = self.ingress_lifecycle()
        lifecycle.cleanup_deadline = 105
        result = SimpleNamespace(returncode=0, stdout=b"")
        with mock.patch.object(LIFECYCLE.time, "monotonic", return_value=100), \
             mock.patch.object(LIFECYCLE, "command", return_value=result) as command:
            lifecycle.docker("ps", timeout=30)
        command.assert_called_once_with(["docker", "ps"], timeout=5, **{})

    def test_expired_cleanup_deadline_fails_before_dispatch(self):
        lifecycle = self.ingress_lifecycle()
        lifecycle.cleanup_deadline = 100
        with mock.patch.object(LIFECYCLE.time, "monotonic", return_value=100), \
             mock.patch.object(LIFECYCLE, "command") as command:
            with self.assertRaises(LIFECYCLE.Failure):
                lifecycle.docker("ps")
        command.assert_not_called()

    def test_cleanup_resets_deadline_and_ordinary_calls_have_no_timeout(self):
        lifecycle = self.ingress_lifecycle(network=False)
        result = SimpleNamespace(returncode=0, stdout=b"")
        with mock.patch.object(LIFECYCLE.time, "monotonic", return_value=100), \
             mock.patch.object(LIFECYCLE, "command", return_value=result) as command:
            lifecycle.cleanup()
            self.assertIsNone(lifecycle.cleanup_deadline)
            lifecycle.docker("ps")
        command.assert_called_once_with(["docker", "ps"])

    def test_ingress_wait_uses_same_cleanup_deadline(self):
        lifecycle = self.ingress_lifecycle()
        lifecycle.cleanup_deadline = 105
        process = mock.Mock(pid=4321)
        listener = mock.MagicMock()
        listener.__enter__.return_value = listener
        lifecycle.ingresses = [(process, 80)]
        with mock.patch.object(LIFECYCLE.os, "killpg"), \
             mock.patch.object(LIFECYCLE.time, "monotonic", return_value=102), \
             mock.patch.object(LIFECYCLE.socket, "socket", return_value=listener):
            lifecycle.stop_ingresses()
        process.wait.assert_called_once_with(timeout=3)


if __name__ == "__main__":
    unittest.main()
