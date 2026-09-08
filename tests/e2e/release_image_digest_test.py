#!/usr/bin/env python3
"""Exercise release record validation and SBOM identity with a fake registry.

This runs the real shell entrypoints, not native image builds or Trivy scans.
The registry's mutable tag and first RepoDigests entry deliberately point at
a different image from the build records.
"""
import json
import ast
import contextlib
import io
import re
from types import SimpleNamespace
import copy
import hashlib
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/docker.yml"
RECORD = ROOT / "infra/docker/record-application-images.sh"
SBOM = ROOT / "infra/security/generate-sboms.sh"
RELEASE = ROOT / "infra/docker/release.sh"
SERVICES = ("gateway", "user-service", "novel-service", "agent-service", "narrative-service", "frontend")
PREFIX = "ghcr.io/wisdoverse/novelworld"
BUILT = "sha256:" + "a" * 64
MOVED = "sha256:" + "b" * 64
LOCAL = "sha256:" + "c" * 64

DIAGNOSTIC_BUDGET = ROOT / "infra/docker/diagnostic_budget.py"
_budget_spec = importlib.util.spec_from_file_location("novelworld_diagnostic_budget", DIAGNOSTIC_BUDGET)
assert _budget_spec and _budget_spec.loader
BUDGET = importlib.util.module_from_spec(_budget_spec)
_budget_spec.loader.exec_module(BUDGET)

PROFILE_BYTES = (ROOT / "tools/llm-budget/diagnostic-v1.json").read_bytes()
BUDGET_ID = "550e8400-e29b-41d4-a716-446655440000"
TOKEN = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
REGISTERED_ENV = {
    "LLM_DIAGNOSTIC_BUDGET_ID": BUDGET_ID,
    "LLM_DIAGNOSTIC_BUDGET_LIMITS": json.dumps({
        "profile": "vision-journey-diagnostic-v1",
        "max_attempts": 2,
        "max_tokens": 100,
        "max_cost_micro_cny": 200,
        "expires_at": "2099-01-01T00:00:00Z",
    }, separators=(",", ":")),
}

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

    def test_publication_is_new_tag_push_only(self):
        workflow = WORKFLOW.read_text()
        publish = workflow.split("\n  github-release:\n", 1)[1]
        self.assertIn(
            "if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')",
            publish,
        )
        self.assertIn('gh release create "$tag" ./* "${flags[@]}"', publish)
        for forbidden in ("gh release view", "gh release upload", "gh release edit",
                          "--draft", "--clobber"):
            self.assertNotIn(forbidden, publish)

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

    def test_diagnostic_registration_is_strict_and_bounded(self):
        registered = BUDGET.registration(PROFILE_BYTES, REGISTERED_ENV)
        self.assertEqual(registered["binding"]["budget_id"], BUDGET_ID)
        self.assertEqual(
            registered["binding"]["profile_sha256"],
            hashlib.sha256(PROFILE_BYTES).hexdigest(),
        )
        self.assertEqual(BUDGET.registration(PROFILE_BYTES, REGISTERED_ENV)["limits"],
                         json.loads(REGISTERED_ENV["LLM_DIAGNOSTIC_BUDGET_LIMITS"]))

        for key, value in (
            ("LLM_DIAGNOSTIC_BUDGET_ID", BUDGET_ID.upper()),
            ("LLM_DIAGNOSTIC_BUDGET_ID", "550e8400e29b41d4a716446655440000"),
            ("LLM_DIAGNOSTIC_BUDGET_ID", "00000000-0000-0000-0000-000000000000"),
        ):
            invalid = dict(REGISTERED_ENV, **{key: value})
            with self.subTest(key=key, value=value), self.assertRaises(BUDGET.Invalid):
                BUDGET.registration(PROFILE_BYTES, invalid)

        for extra in ("unexpected",):
            limits = json.loads(REGISTERED_ENV["LLM_DIAGNOSTIC_BUDGET_LIMITS"])
            limits[extra] = 1
            invalid = dict(REGISTERED_ENV, LLM_DIAGNOSTIC_BUDGET_LIMITS=json.dumps(limits))
            with self.subTest(extra=extra), self.assertRaises(BUDGET.Invalid):
                BUDGET.registration(PROFILE_BYTES, invalid)
        duplicate_limits = (
            '{"profile":"vision-journey-diagnostic-v1","max_attempts":2,'
            '"max_attempts":2,"max_tokens":100,"max_cost_micro_cny":200,'
            '"expires_at":"2099-01-01T00:00:00Z"}'
        )
        with self.assertRaises(BUDGET.Invalid):
            BUDGET.registration(
                PROFILE_BYTES,
                dict(REGISTERED_ENV, LLM_DIAGNOSTIC_BUDGET_LIMITS=duplicate_limits),
            )

        for key, value in (
            ("max_attempts", json.loads(PROFILE_BYTES)["max_limits"]["attempts"] + 1),
            ("max_tokens", json.loads(PROFILE_BYTES)["max_limits"]["tokens"] + 1),
            ("max_cost_micro_cny", json.loads(PROFILE_BYTES)["max_limits"]["cost_micro_cny"] + 1),
            ("max_attempts", -1),
            ("max_tokens", 1.5),
        ):
            limits = json.loads(REGISTERED_ENV["LLM_DIAGNOSTIC_BUDGET_LIMITS"])
            limits[key] = value
            invalid = dict(REGISTERED_ENV, LLM_DIAGNOSTIC_BUDGET_LIMITS=json.dumps(limits))
            with self.subTest(key=key, value=value), self.assertRaises(BUDGET.Invalid):
                BUDGET.registration(PROFILE_BYTES, invalid)

        for expiry in ("2099-01-01T00:00:00+00:00", "2099-02-30T00:00:00Z"):
            limits = json.loads(REGISTERED_ENV["LLM_DIAGNOSTIC_BUDGET_LIMITS"])
            limits["expires_at"] = expiry
            invalid = dict(REGISTERED_ENV, LLM_DIAGNOSTIC_BUDGET_LIMITS=json.dumps(limits))
            with self.subTest(expiry=expiry), self.assertRaises((BUDGET.Invalid, ValueError)):
                BUDGET.registration(PROFILE_BYTES, invalid)

    def test_diagnostic_marker_lifecycle_is_one_shot_and_binding_exact(self):
        registered = BUDGET.registration(PROFILE_BYTES, REGISTERED_ENV)
        state = self.root / "diagnostic-state"
        state.mkdir()
        BUDGET.marker_action("adopt", state, registered)
        BUDGET.marker_action("provision-start", state, registered)
        for action in ("upgrade", "rollback", "preflight"):
            with self.subTest(action=action), self.assertRaises(BUDGET.Invalid):
                BUDGET.marker_action(action, state, registered)
        with self.assertRaises(OSError):
            BUDGET.marker_action("provision-start", state, registered)
        BUDGET.marker_action("provision-complete", state, registered)
        self.assertEqual(BUDGET.read_marker(state / "diagnostic-provisioning.json"),
                         {**registered, "completed": True})
        for changed in (dict(registered, binding=dict(registered["binding"], budget_id="550e8400-e29b-41d4-a716-446655440001")),
                        dict(registered, limits=dict(registered["limits"], max_tokens=99))):
            with self.assertRaises(BUDGET.Invalid):
                BUDGET.marker_action("upgrade", state, changed)

    def test_diagnostic_preflight_validates_all_services_before_any_probe(self):
        registered = BUDGET.registration(PROFILE_BYTES, REGISTERED_ENV)
        images = {service: f"example/{service}@{BUILT}" for service in BUDGET.SERVICES}
        manifest = {service.upper().replace("-", "_") + "_IMAGE": image
                    for service, image in images.items()}
        services = {}
        for service in BUDGET.SERVICES:
            environment = {
                "LLM_DIAGNOSTIC_BUDGET_ID": BUDGET_ID,
                "USER_SERVICE_URL": "http://127.0.0.1:8001" if service == "user-service" else "http://user-service:8001",
                "INTERNAL_SERVICE_TOKEN": TOKEN,
            }
            if service == "user-service":
                environment["LLM_DIAGNOSTIC_BUDGET_LIMITS"] = REGISTERED_ENV["LLM_DIAGNOSTIC_BUDGET_LIMITS"]
            services[service] = {"environment": environment, "image": images[service]}
        raw = json.dumps({"services": services}).encode()
        with mock.patch.object(BUDGET, "probe") as probe:
            BUDGET.preflight(raw, manifest, registered, "nwq-abcdef1234")
            self.assertEqual(probe.call_count, 4)
            self.assertEqual({call.args[0] for call in probe.call_args_list}, set(images.values()))

        mutations = []
        for service in BUDGET.SERVICES:
            invalid = copy.deepcopy(services)
            invalid[service]["environment"]["LLM_DIAGNOSTIC_BUDGET_ID"] = "550e8400-e29b-41d4-a716-446655440001"
            mutations.append((f"{service} budget id", invalid, manifest))

            invalid = copy.deepcopy(services)
            invalid[service]["environment"]["USER_SERVICE_URL"] = "https://wrong.example"
            mutations.append((f"{service} service URL", invalid, manifest))

            invalid = copy.deepcopy(services)
            invalid[service]["environment"]["INTERNAL_SERVICE_TOKEN"] = "f" * 64
            mutations.append((f"{service} token", invalid, manifest))

            invalid_manifest = dict(manifest)
            invalid_manifest[service.upper().replace("-", "_") + "_IMAGE"] = f"example/wrong-{service}@{BUILT}"
            mutations.append((f"{service} image", services, invalid_manifest))

        invalid = copy.deepcopy(services)
        invalid["user-service"]["environment"]["LLM_DIAGNOSTIC_BUDGET_LIMITS"] = json.dumps(
            dict(json.loads(REGISTERED_ENV["LLM_DIAGNOSTIC_BUDGET_LIMITS"]), max_tokens=99)
        )
        mutations.append(("owner limits", invalid, manifest))
        for service in BUDGET.SERVICES:
            if service == "user-service":
                continue
            invalid = copy.deepcopy(services)
            invalid[service]["environment"]["LLM_DIAGNOSTIC_BUDGET_LIMITS"] = REGISTERED_ENV["LLM_DIAGNOSTIC_BUDGET_LIMITS"]
            mutations.append((f"{service} stray limits", invalid, manifest))

        invalid = copy.deepcopy(services)
        invalid["agent-service"]["environment"]["INTERNAL_SERVICE_TOKEN"] = "f" * 64
        mutations.append(("token mismatch", invalid, manifest))

        for label, invalid, invalid_manifest in mutations:
            with self.subTest(configuration=label), mock.patch.object(BUDGET, "probe") as probe:
                with self.assertRaises(BUDGET.Invalid):
                    BUDGET.preflight(
                        json.dumps({"services": invalid}).encode(),
                        invalid_manifest,
                        registered,
                        "nwq-abcdef1234",
                    )
                probe.assert_not_called()

    def test_bounded_output_success_failure_timeout_and_overflow(self):
        self.assertEqual(BUDGET.bounded_output([sys.executable, "-c", "print('ok', end='')"], 2), b"ok")
        with self.assertRaises(BUDGET.ProbeInvalid) as rejected:
            BUDGET.bounded_output([sys.executable, "-c", "raise SystemExit(3)"], 2)
        self.assertEqual((rejected.exception.reason, rejected.exception.exit_code), ("child_nonzero", 3))
        with self.assertRaises(BUDGET.ProbeInvalid) as rejected:
            BUDGET.bounded_output([sys.executable, "-c", "import time; time.sleep(2)"], 0.05)
        self.assertEqual(rejected.exception.reason, "deadline")
        with self.assertRaises(BUDGET.ProbeInvalid) as rejected:
            BUDGET.bounded_output([sys.executable, "-c", "import sys; sys.stdout.buffer.write(b'x' * 4097)"], 2)
        self.assertEqual(rejected.exception.reason, "output_overflow")

    def test_probe_confirms_create_before_start_and_verifies_cleanup(self):
        image = "registry/service@sha256:" + "a" * 64
        for response in (b'{"contract":"ok"}', BUDGET.Invalid()):
            with self.subTest(response=type(response).__name__), \
                 mock.patch.object(BUDGET, "bounded_output", side_effect=[b"b" * 64 + b"\n", response, b""]) as output, \
                 mock.patch.object(BUDGET.subprocess, "run") as cleanup:
                cleanup.return_value.returncode = 0
                if isinstance(response, Exception):
                    with self.assertRaises(BUDGET.Invalid):
                        BUDGET.probe(image, "nwq-abcdef1234", {"contract": "ok"})
                else:
                    BUDGET.probe(image, "nwq-abcdef1234", {"contract": "ok"})
                create, start, verify = [call.args[0] for call in output.call_args_list]
                self.assertEqual(create[:2], ["docker", "create"])
                name = create[create.index("--name") + 1]
                self.assertEqual(start, ["docker", "start", "--attach", name])
                self.assertEqual(cleanup.call_args.args[0], ["docker", "rm", "--force", name])
                self.assertEqual(verify, ["docker", "ps", "--all", "--quiet", "--filter", "name=^/" + name + "$"])
                self.assertLess(output.call_args_list[1].args[1], 10)

    def test_unconfirmed_create_or_failed_cleanup_is_not_capability_refusal(self):
        image = "registry/service@sha256:" + "a" * 64
        for responses in ([BUDGET.Invalid(), b""], [b"invalid-id", b""],
                          [b"b" * 64, BUDGET.Invalid(), b"remaining-container"],
                          [b"b" * 64, BUDGET.Invalid(), BUDGET.Invalid()]):
            with self.subTest(responses=len(responses)), \
                 mock.patch.object(BUDGET, "bounded_output", side_effect=responses), \
                 mock.patch.object(BUDGET.subprocess, "run") as cleanup:
                with self.assertRaises(OSError):
                    BUDGET.probe(image, "nwq-abcdef1234", {})
                cleanup.assert_called_once()

    def test_failed_removal_is_unproven_even_when_query_is_empty(self):
        with mock.patch.object(BUDGET, "bounded_output", side_effect=[b"b" * 64, b"{}", b""]) as output, \
             mock.patch.object(BUDGET.subprocess, "run") as cleanup:
            cleanup.return_value.returncode = 1
            with self.assertRaises(OSError):
                BUDGET.probe("registry/service@sha256:" + "a" * 64, "nwq-abcdef1234", {})
            self.assertEqual(output.call_count, 3)

    def test_cleanup_uncertainty_preserves_primary_probe_evidence(self):
        primary = BUDGET.ProbeInvalid("child_nonzero", exit_code=3, elapsed=0.2)
        with mock.patch.object(BUDGET, "bounded_output", side_effect=[b"b" * 64, primary, b""]), \
             mock.patch.object(BUDGET.subprocess, "run") as cleanup:
            cleanup.return_value.returncode = 1
            with self.assertRaises(OSError) as rejected:
                BUDGET.probe("registry/service@sha256:" + "a" * 64, "nwq-abcdef1234", {})
        self.assertEqual(rejected.exception.primary_evidence["reason"], "child_nonzero")
        self.assertEqual(rejected.exception.cleanup_evidence[0]["reason"], "cleanup_nonzero")

    def test_probe_evidence_closes_identity_and_reason_fields(self):
        error = BUDGET.ProbeInvalid("arbitrary-child-text", elapsed=float("nan"))
        error.probe_phase = "arbitrary"
        error.probe_name = "child-output"
        evidence = BUDGET.probe_evidence(error)
        self.assertEqual(evidence["primary"]["reason"], "unknown")
        self.assertIsNone(evidence["primary"]["elapsed"])
        json.dumps(evidence, allow_nan=False)
        self.assertIsNone(evidence["name"])
        self.assertIsNone(evidence["container_id"])

    def test_probe_expectation_failure_retains_phase_name_and_acknowledged_id(self):
        identifier = b"b" * 64
        with mock.patch.object(BUDGET, "bounded_output", side_effect=[identifier, b"{}", b""]), \
             mock.patch.object(BUDGET.subprocess, "run") as cleanup:
            cleanup.return_value.returncode = 0
            with self.assertRaises(BUDGET.ProbeInvalid) as rejected:
                BUDGET.probe("registry/service@sha256:" + "a" * 64, "nwq-abcdef1234", {"contract": "ok"})
        evidence = BUDGET.probe_evidence(rejected.exception)
        self.assertEqual(evidence["phase"], "probe_expectation")
        self.assertEqual(evidence["name"].split("-budget-probe-")[0], "nwq-abcdef1234")
        self.assertEqual(evidence["container_id"], "b" * 64)

    def test_cleanup_query_deadline_is_distinct_and_retains_identity(self):
        with mock.patch.object(BUDGET, "bounded_output", side_effect=[b"b" * 64, BUDGET.ProbeInvalid("child_nonzero", exit_code=3), BUDGET.ProbeInvalid("deadline")]), \
             mock.patch.object(BUDGET.subprocess, "run") as cleanup:
            cleanup.return_value.returncode = 0
            with self.assertRaises(OSError) as rejected:
                BUDGET.probe("registry/service@sha256:" + "a" * 64, "nwq-abcdef1234", {})
        evidence = BUDGET.probe_evidence(rejected.exception)
        self.assertEqual(evidence["primary"]["reason"], "child_nonzero")
        self.assertEqual(evidence["cleanup"][0]["reason"], "cleanup_query_deadline")
        self.assertEqual(evidence["phase"], "probe_cleanup_ps")
        self.assertEqual(evidence["container_id"], "b" * 64)

    def test_reap_timeout_is_uncertain_and_preserves_primary(self):
        process = mock.Mock()
        process.poll.return_value = None
        process.wait.side_effect = [3, subprocess.TimeoutExpired(["x"], 5)]
        process.stdout.fileno.return_value = 1
        selector = mock.Mock()
        selector.__enter__ = mock.Mock(return_value=selector)
        selector.__exit__ = mock.Mock(return_value=False)
        selector.select.return_value = [object()]
        with mock.patch.object(BUDGET.subprocess, "Popen", return_value=process), \
             mock.patch.object(BUDGET.selectors, "DefaultSelector", return_value=selector), \
             mock.patch.object(BUDGET.os, "read", return_value=b""):
            with self.assertRaises(OSError) as rejected:
                BUDGET.bounded_output(["docker", "start"], 1)
        self.assertEqual(rejected.exception.primary_evidence["reason"], "child_nonzero")
        self.assertEqual(rejected.exception.primary_evidence["exit_code"], 3)
        self.assertEqual(rejected.exception.reap_evidence["reason"], "reap_timeout")
        process.stdout.close.assert_called_once()

    def test_ack_deadline_and_residue_classification(self):
        for kind in ("bad_ack", "shared_deadline", "residue"):
            with self.subTest(kind=kind):
                responses = [b"invalid" if kind == "bad_ack" else b"b" * 64]
                if kind == "residue":
                    responses += [b"{}", b"present"]
                else:
                    responses += [b""]
                ticks = iter([0, 11] if kind == "shared_deadline" else [0, 0])
                with mock.patch.object(BUDGET, "bounded_output", side_effect=responses), \
                     mock.patch.object(BUDGET.time, "monotonic", side_effect=lambda: next(ticks, 11)), \
                     mock.patch.object(BUDGET.subprocess, "run") as cleanup:
                    cleanup.return_value.returncode = 0
                    exception = BUDGET.ProbeInvalid if kind == "shared_deadline" else OSError
                    with self.assertRaises(exception) as rejected:
                        BUDGET.probe("registry/service@sha256:" + "a" * 64, "nwq-abcdef1234", {})
                evidence = BUDGET.probe_evidence(rejected.exception)
                self.assertIsNotNone(evidence["name"])
                self.assertEqual(evidence["container_id"], None if kind == "bad_ack" else "b" * 64)
                if kind == "bad_ack":
                    self.assertEqual(evidence["primary"]["reason"], "create_ack_invalid")
                    self.assertEqual(evidence["cleanup"][0]["reason"], "create_unconfirmed")
                elif kind == "shared_deadline":
                    self.assertEqual(evidence["primary"]["reason"], "deadline")
                    self.assertEqual(evidence["primary"]["phase"], "probe_create")
                else:
                    self.assertIsNone(evidence["primary"])
                    self.assertEqual(evidence["cleanup"][0]["reason"], "cleanup_residue")

    def test_compound_overflow_reap_and_cleanup_failure_preserve_all_causes(self):
        original = BUDGET.bounded_output
        process, selector = mock.Mock(), mock.MagicMock()
        process.poll.return_value = None
        process.wait.side_effect = subprocess.TimeoutExpired(["private-command"], 5)
        selector.__enter__.return_value = selector
        selector.select.return_value = [object()]

        def command(argv, timeout):
            if argv[1] == "create":
                return b"b" * 64
            if argv[1] == "start":
                return original(argv, timeout)
            raise BUDGET.ProbeInvalid("deadline", elapsed=5)

        with mock.patch.object(BUDGET, "bounded_output", side_effect=command), \
             mock.patch.object(BUDGET.subprocess, "Popen", return_value=process), \
             mock.patch.object(BUDGET.subprocess, "run") as cleanup, \
             mock.patch.object(BUDGET.selectors, "DefaultSelector", return_value=selector), \
             mock.patch.object(BUDGET.os, "read", return_value=b"x" * 4097):
            cleanup.return_value.returncode = 7
            with self.assertRaises(OSError) as rejected:
                BUDGET.probe("registry/service@sha256:" + "a" * 64, "nwq-abcdef1234", {})
        evidence = BUDGET.probe_evidence(rejected.exception)
        self.assertEqual(evidence["primary"]["reason"], "output_overflow")
        self.assertEqual(evidence["primary"]["phase"], "probe_start")
        self.assertEqual(evidence["reap"]["reason"], "reap_timeout")
        self.assertEqual([d["reason"] for d in evidence["cleanup"]],
                         ["cleanup_nonzero", "cleanup_query_deadline"])
        self.assertEqual(evidence["cleanup"][0]["exit_code"], 7)
        self.assertEqual(evidence["container_id"], "b" * 64)
        self.assertNotIn("private-command", json.dumps(evidence, allow_nan=False))
        process.stdout.close.assert_called_once()

    def test_evidence_revalidates_mutated_and_malformed_fields(self):
        for value in ([], {"reason": [], "phase": {}, "elapsed": float("nan"),
                           "exit_code": True, "raw": "private-child-text"}):
            for error in (BUDGET.ProbeInvalid("deadline"), OSError("private-child-text")):
                error.primary_evidence = value
                error.reap_evidence = value
                error.cleanup_evidence = [value] * 10
                error.probe_phase = []
                error.probe_name = "private-child-text"
                error.probe_container_id = "private-child-text"
                evidence = BUDGET.probe_evidence(error)
                encoded = json.dumps(evidence, allow_nan=False)
                self.assertNotIn("private-child-text", encoded)
                self.assertEqual(len(evidence["cleanup"]), 4)
                self.assertIsNone(evidence["primary"]["exit_code"])

    def test_probe_phase_lines_still_feed_both_anchored_lifecycle_consumers(self):
        log = io.StringIO()
        with contextlib.redirect_stderr(log), \
             mock.patch.object(BUDGET, "bounded_output", side_effect=[
                 b"b" * 64, BUDGET.ProbeInvalid("child_nonzero", exit_code=3),
                 BUDGET.ProbeInvalid("deadline")]), \
             mock.patch.object(BUDGET.subprocess, "run") as cleanup:
            cleanup.return_value.returncode = 0
            with self.assertRaises(OSError):
                BUDGET.probe("registry/service@sha256:" + "a" * 64, "nwq-abcdef1234", {})
        source = ROOT / "tests/e2e/diagnostic_budget_lifecycle.py"
        spec = importlib.util.spec_from_file_location("probe_lifecycle_test", source)
        lifecycle = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(lifecycle)
        with tempfile.TemporaryDirectory() as directory:
            Path(directory, "release-adopt.log").write_text(log.getvalue())
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                lifecycle.report_cold_release_status(
                    SimpleNamespace(output=Path(directory), diagnostic_failures=[]), False)
            self.assertTrue(json.loads(output.getvalue())["capability_probe_failed"])
        # Evaluate the actual reentry consumer's literal, without copying its pattern.
        patterns = [node.args[0].value for node in ast.walk(ast.parse(source.read_text()))
                    if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                    and node.func.attr == "findall" and node.args
                    and isinstance(node.args[0], ast.Constant)
                    and isinstance(node.args[0].value, bytes)
                    and b"diagnostic phase=" in node.args[0].value]
        self.assertEqual(len(patterns), 1)
        self.assertEqual(re.findall(patterns[0], log.getvalue().encode()),
                         [b"probe_start", b"probe_cleanup_ps"])


if __name__ == "__main__":
    unittest.main()
