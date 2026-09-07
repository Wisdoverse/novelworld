import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import json
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
    def lifecycle(self, containers, network=True):
        lifecycle = LIFECYCLE.Lifecycle.__new__(LIFECYCLE.Lifecycle)
        lifecycle.project = "nwq-abcdef1234"
        lifecycle.containers = set(containers)
        lifecycle.image_refs = set()
        lifecycle.source_images = {}
        lifecycle.network_created = network
        lifecycle.temporary = tempfile.TemporaryDirectory(prefix="cleanup-test-")
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
        lifecycle.docker = mock.Mock(side_effect=lambda *args, **kwargs: SimpleNamespace(
            returncode=0,
            stdout=(lifecycle.project + "\n").encode()
            if args[:3] == ("network", "ls", "--filter") else b"",
        ))
        temporary = Path(lifecycle.temporary.name)

        lifecycle.cleanup()

        self.assertFalse(temporary.exists())
        lifecycle.docker.assert_any_call("rm", "--force", "--volumes", "owned-a", check=False)
        lifecycle.docker.assert_any_call("rm", "--force", "--volumes", "owned-b", check=False)
        lifecycle.docker.assert_any_call("network", "rm", lifecycle.project, check=False)
        self.assertEqual(lifecycle.docker.call_count, 4)

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


if __name__ == "__main__":
    unittest.main()
