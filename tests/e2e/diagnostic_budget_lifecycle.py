#!/usr/bin/env python3
"""Offline #320 boundary test: actual owner/clients/PG on one internal network.

Not a product journey runner or a release-upgrade qualification. Only synthetic
credentials enter these containers. The TLS mock has no outbound provider path.
Uses the diagnostic_budget_driver example and either four normal service
binaries or existing release-built images; see --help. Real release refusal
checks do not constitute a successful different-version upgrade journey.
"""
import argparse
import copy
import datetime
import importlib.util
from http.client import HTTPConnection
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import ipaddress
import os
from pathlib import Path
import re
import socket
import signal
import shutil
import shlex
import sys
import ssl
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[2]
MODEL = "deepseek-v4-flash-vision-exp"
KEY = "test-only"
TOKEN = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
PROFILE = "vision-journey-diagnostic-v1"
CONTRACT = "llm-diagnostic-budget-v1"
PYTHON_IMAGE = "python@sha256:cad9a2c871761c413caa6fdd6441c783451e740a48aaeba60ae62a8b53525ef6"
PG_IMAGE = "pgvector/pgvector:pg18@sha256:2ba9ca5f2e7daa0f0e7723cba1ee9167bab54efd3640516a44ac1a928dd67e7a"
REGISTRY_IMAGE = "registry@sha256:1be55279f18a2fe1a74edf2664cac61c1bea305b7b4642dab412e7affdcb3e33"
SERVICES = ("user-service", "novel-service", "agent-service", "narrative-service")
NO_PROXY = "owner,mock,postgres,127.0.0.1,localhost"


class Failure(Exception):
    pass


def require(condition, code):
    if not condition:
        raise Failure(code)


def report_cold_adoption_status(journey, zero):
    """Best-effort stage presence only; never publish private diagnostic data."""
    status = {"case": "zero" if zero else "nonzero",
              "base_images_recorded": False,
              **{name + "_snapshot_present": False
                 for name in ("initial", "settings", "restart", "terminal")},
              "payers_stopped": False, "metrics_reconciled": False,
              "existing_stack_unchanged": False}
    try:
        private = journey.private_report
        images = private.get("release_images", {})
        snapshots = private.get("diagnostic_budget_snapshots", {})
        environment = journey.report.get("environment", {})
        status.update(
            base_images_recorded=isinstance(images, dict) and "base" in images,
            payers_stopped=private.get("diagnostic_payers_stopped") is True,
            metrics_reconciled=private.get("diagnostic_metrics_reconciled") is True,
            existing_stack_unchanged=isinstance(environment, dict)
                and environment.get("existing_user_stack_unchanged") is True,
            **{name + "_snapshot_present": isinstance(snapshots, dict) and name in snapshots
               for name in ("initial", "settings", "restart", "terminal")})
    except (AttributeError, TypeError):
        pass
    try:
        print(json.dumps(status, sort_keys=True), flush=True)
    except Exception:
        # A broken CI output pipe must not replace the original failure or cleanup.
        pass


def report_cold_release_status(journey, zero):
    """Project fixed release markers only; raw private logs never reach CI."""
    phases = ("pull", "database_start", "migration", "application_deployment", "readiness")
    refusals = {
        "preflight_refused": b"release: diagnostic budget preflight failed",
        "worktree_dirty": b"release: working tree is not clean",
        "provision_uncertain": b"release: diagnostic provisioning uncertain; attempt frozen",
    }
    status = {"case": "zero" if zero else "nonzero", "release_log_present": False,
              "release_log_complete": False,
              **{phase + "_" + boundary: False for phase in phases for boundary in ("start", "end")},
              **{name: False for name in refusals}, "capability_probe_failed": False,
              "curl_failed": False, "release_adopt_failed": False, "image_identity_failed": False}
    try:
        failures = journey.diagnostic_failures
        status["release_adopt_failed"] = "release_adopt_failed" in failures
        status["image_identity_failed"] = "release_image_identity_mismatch" in failures
        with (journey.output / "release-adopt.log").open("rb") as stream:
            status["release_log_present"] = True
            raw = stream.read(1048577)
        if len(raw) <= 1048576:
            status["release_log_complete"] = True
            for line in raw.splitlines():
                marker = re.fullmatch(rb"qualification-phase (pull|database_start|migration|application_deployment|readiness) (start|end) [0-9]+", line)
                if marker:
                    status[(marker[1] + b"_" + marker[2]).decode("ascii")] = True
                for name, literal in refusals.items():
                    status[name] |= line == literal
                status["capability_probe_failed"] |= re.fullmatch(
                    rb"diagnostic phase=probe_(create|start|expectation|cleanup_rm|cleanup_ps|cleanup_unproven)", line) is not None
                status["curl_failed"] |= re.match(rb"curl: \([0-9]+\) ", line) is not None
    except (AttributeError, TypeError, OSError):
        pass
    try:
        print(json.dumps(status, sort_keys=True), flush=True)
    except Exception:
        pass  # Observability cannot supersede terminal failure or cleanup.


def ingress_address(network, project):
    configurations = [item for item in network["IPAM"]["Config"]
                      if ipaddress.ip_network(item["Subnet"]).version == 4]
    require(len(configurations) == 1 and network["Internal"] is True
            and network["Name"] == project, "fixture_ingress_network_invalid")
    config = configurations[0]
    subnet = ipaddress.ip_network(config["Subnet"])
    require(subnet.prefixlen <= 28 and any(subnet.subnet_of(ipaddress.ip_network(block))
            for block in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")),
            "fixture_ingress_network_invalid")
    candidate = subnet[-2]
    reserved = [config.get("Gateway"), *(config.get("AuxiliaryAddresses") or {}).values()]
    reserved += [item["IPv4Address"].split("/")[0] for item in network["Containers"].values()]
    require(str(candidate) not in reserved, "fixture_ingress_address_occupied")
    return candidate


def isolated_journey_compose(compose, project, ca_path, nginx_ip):
    network = "  novel-net:\n    driver: bridge\n"
    require(compose.count(network) == 1, "fixture_compose_network_shape_changed")
    compose = compose.replace(network, "  novel-net:\n    external: true\n    name: " + project + "\n")
    for service in SERVICES:
        # Anchor at exactly two spaces: dependencies also contain service names.
        marker = re.search(r"(?m)^  " + re.escape(service) + r":\n", compose)
        require(marker is not None, "fixture_service_missing")
        start = marker.start()
        following = re.search(r"(?m)^  [a-z][a-z-]*:\s*$", compose[marker.end():])
        end = marker.end() + following.start() if following else len(compose)
        block = compose[start:end]
        require(block.count("    environment:\n") == 1 and "    volumes:\n" not in block,
                "fixture_service_shape_changed")
        block = block.replace("    environment:\n", "    volumes:\n      - " + json.dumps(
            str(ca_path) + ":/fixture/ca.pem:ro") + "\n    environment:\n"
            "      HTTPS_PROXY: http://mock:3128\n      NO_PROXY: localhost,127.0.0.1,user-service\n"
            "      SSL_CERT_FILE: /fixture/ca.pem\n")
        compose = compose[:start] + block + compose[end:]
    ports = '    ports:\n      - "${NGINX_HTTP_BIND:-0.0.0.0}:${NGINX_HTTP_PORT:-80}:80"\n'
    require(compose.count(ports) == 1, "fixture_nginx_ports_changed")
    compose = compose.replace(ports, "")
    start = re.search(r"(?m)^  nginx:\n", compose)
    require(start is not None, "fixture_nginx_missing")
    block = compose[start.start():]
    require(block.count("    networks:\n      - novel-net\n") == 1, "fixture_nginx_network_changed")
    block = block.replace("    networks:\n      - novel-net\n",
        "    networks:\n      novel-net:\n        ipv4_address: " + str(nginx_ip) + "\n")
    compose = compose[:start.start()] + block
    return compose


def command(args, *, timeout=30, input=None, check=True, cwd=None, env=None):
    result = subprocess.run(args, input=input, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=timeout, cwd=cwd, env=env)
    if check:
        require(result.returncode == 0,
                f"fixture_command_failed_{Path(args[0]).name}_{args[1]}_exit_{result.returncode}")
    return result


def http(origin, path, method="GET", body=None, headers=None):
    request = urllib.request.Request(origin + path, method=method,
                                     data=None if body is None else json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json", **(headers or {})})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        response = opener.open(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read(32769)
        require(len(raw) <= 32768, "fixture_response_oversized")
        return response.status, json.loads(raw) if raw else None


def mock_server():
    state = {"connects": 0, "providers": 0, "reserve": 0, "settle": 0,
             "errors": 0, "provider_mode": "ok", "control_mode": "ok"}
    lock = threading.Lock()
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain("/fixture/server.pem", "/fixture/server.key")

    class Handler(BaseHTTPRequestHandler):
        def setup(self):
            self.request.settimeout(5)
            super().setup()

        def log_message(self, *_):
            pass

        def body(self, maximum):
            size = int(self.headers.get("Content-Length", "0"))
            require(0 <= size <= maximum, "fixture_bad_request_size")
            return self.rfile.read(size)

        def reply(self, status, body, content_type="application/json"):
            raw = body if isinstance(body, bytes) else json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(raw)))
            self.send_header("Connection", "close")
            self.send_header("Retry-After", "0")
            self.end_headers()
            self.wfile.write(raw)

    class Provider(Handler):
        def do_POST(self):
            require(self.path == "/v1/chat/completions", "unexpected_provider_path")
            require(self.headers.get("Authorization") == "Bearer " + KEY, "non_synthetic_key")
            body = json.loads(self.body(262144))
            require(body["model"] == MODEL and body["max_tokens"] == 8, "unexpected_provider_identity")
            require(body["thinking"] == {"type": "disabled"}, "unexpected_thinking")
            with lock:
                state["providers"] += 1
                mode = state["provider_mode"]
                if mode in ("retry", "empty"):
                    state["provider_mode"] = "ok"
            if mode == "retry":
                self.reply(429, {})
                return
            usage = None if mode == "missing" else {"prompt_tokens": 10, "completion_tokens": 2}
            if mode in ("malformed", "empty-malformed"):
                usage = {"prompt_tokens": "10", "completion_tokens": 2}
            elif mode == "conflicting":
                usage.update(prompt_cache_hit_tokens=8, prompt_cache_miss_tokens=8)
            if body.get("stream"):
                frames = [
                    {"model": MODEL, "choices": [{"delta": {"content": "OK"}, "finish_reason": None}]},
                    {"model": MODEL, "choices": [], "usage": usage},
                ]
                raw = "".join("data: " + json.dumps(frame) + "\n\n" for frame in frames) + "data: [DONE]\n\n"
                self.reply(200, raw.encode(), "text/event-stream")
            else:
                self.reply(200, {"model": MODEL, "usage": usage,
                                 "choices": [{"message": {"content": "" if mode.startswith("empty") else "OK"},
                                              "finish_reason": "stop"}]})

    class Proxy(Handler):
        def do_CONNECT(self):
            require(self.path == "api.deepseek.com:443", "proxy_target_not_allowed")
            with lock:
                state["connects"] += 1
            self.send_response(200)
            self.end_headers()
            self.wfile.flush()
            with context.wrap_socket(self.connection, server_side=True) as secure:
                Provider(secure, self.client_address, self.server)
            self.close_connection = True

    class Control(Handler):
        def forward(self):
            require(self.path == "/internal/runtime/llm" or self.path.startswith("/internal/llm-budget/"),
                    "unexpected_control_path")
            require(self.headers.get("X-Internal-Service-Token") == TOKEN, "unexpected_control_token")
            raw = self.body(4096)
            upstream = HTTPConnection("owner", 8001, timeout=5)
            try:
                upstream.request(self.command, self.path, raw, {
                    "Content-Type": "application/json", "X-Internal-Service-Token": TOKEN,
                    "X-LLM-Budget-Contract": CONTRACT,
                })
                response = upstream.getresponse()
                body = response.read(8193)
                require(len(body) <= 8192, "upstream_control_oversized")
                operation = self.path.rsplit("/", 1)[-1]
                with lock:
                    if operation in ("reserve", "settle"):
                        state[operation] += 1
                    lose = state["control_mode"] == "lose_" + operation
                if lose:
                    # The actual owner has replied after commit. Lose only its ACK.
                    self.close_connection = True
                    return
                self.reply(response.status, body)
            finally:
                upstream.close()

        do_GET = forward
        do_POST = forward

    class Fixture(Handler):
        def do_GET(self):
            require(self.path == "/state", "fixture_path")
            with lock:
                self.reply(200, dict(state))

        def do_POST(self):
            require(self.path == "/mode", "fixture_path")
            mode = json.loads(self.body(1024))
            require(set(mode) <= {"provider_mode", "control_mode"}, "fixture_mode")
            with lock:
                state.update(mode)
            self.reply(200, {})

    class Server(ThreadingHTTPServer):
        daemon_threads = True

        def handle_error(self, *_):
            with lock:
                state["errors"] += 1

    for port, handler in ((3128, Proxy), (8081, Control)):
        server = Server(("0.0.0.0", port), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
    Server(("0.0.0.0", 8082), Fixture).serve_forever()


class Lifecycle:
    def __init__(self, owner, driver, subnet=None):
        self.owner_binary = owner.resolve() if owner else None
        self.driver_binary = driver.resolve() if driver else None
        self.project = "nwq-" + uuid.uuid4().hex[:10]
        self.containers = set()
        self.image_refs = set()
        self.images = {}
        self.source_images = {}
        self.network_created = False
        self.temporary = tempfile.TemporaryDirectory(prefix=self.project + "-")
        self.files = Path(self.temporary.name)
        self.budget_id = str(uuid.uuid4())
        self.environment = {}
        self.subnet = subnet
        self.journeys = []
        self.ingresses = []

    def docker(self, *args, **kwargs):
        deadline = getattr(self, "cleanup_deadline", None)
        if deadline is not None:
            remaining = deadline - time.monotonic()
            require(remaining > 0, "fixture_cleanup_timeout")
            kwargs["timeout"] = min(kwargs.get("timeout", 10), 10, remaining)
        return command(["docker", *args], **kwargs)

    def run(self, name, image, args, environment=None, mounts=(), alias=None):
        full_name = self.project + "-" + name
        self.containers.add(full_name)
        cmd = ["run", "--detach", "--name", full_name, "--network", self.project,
               "--label", "novelworld.issue=320", "--label", "novelworld.purpose=offline-budget-lifecycle",
               "--pull", "never", "--cap-drop", "ALL", "--security-opt", "no-new-privileges"]
        if alias:
            cmd += ["--network-alias", alias]
        if name == "postgres":
            cmd += ["--user", "postgres"]
        else:
            cmd += ["--user", f"{os.getuid()}:{os.getgid()}"]
        for source, target in mounts:
            cmd += ["--mount", f"type=bind,src={source},dst={target},readonly"]
        for key, value in (environment or {}).items():
            cmd += ["--env", key + "=" + value]
        cmd += [image, *args]
        self.docker(*cmd)
        networks = json.loads(self.docker("inspect", "--format", "{{json .NetworkSettings.Networks}}", full_name).stdout)
        require(set(networks) == {self.project}, "extra_container_network")
        return full_name

    def origin(self, name, port):
        data = json.loads(self.docker("inspect", "--format", "{{json .NetworkSettings.Networks}}", name).stdout)
        require(set(data) == {self.project}, "unexpected_network")
        address = ipaddress.ip_address(data[self.project]["IPAddress"])
        require(address.is_private, "unsafe_fixture_address")
        # Local Linux host reaches its internal bridge directly; no host/public
        # port is published and containers have no external gateway.
        return f"http://{address}:{port}"

    def wait_http(self, origin, path):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                if http(origin, path)[0] == 200:
                    return
            except (OSError, ValueError):
                pass
            time.sleep(0.1)
        raise Failure("fixture_readiness_timeout")

    def exit_status(self, container, timeout=15):
        result = self.docker("wait", container, timeout=timeout)
        return int(result.stdout)

    def remove(self, container):
        self.docker("rm", "--force", container)
        self.containers.remove(container)

    def sql(self, sql):
        return self.docker("exec", "-i", self.project + "-postgres", "psql", "-U", "test", "-d", "novelworld_test",
                           "-v", "ON_ERROR_STOP=1", "-At", input=sql.encode()).stdout.decode().strip()

    def state(self):
        return http(self.mock_origin, "/state")[1]

    def mode(self, provider="ok", control="ok"):
        require(http(self.mock_origin, "/mode", "POST", {"provider_mode": provider, "control_mode": control})[0] == 200,
                "fixture_mode_failed")

    def charged(self):
        status, snapshot = http(self.owner_origin, "/internal/llm-budget/" + self.budget_id,
                                headers={"X-Internal-Service-Token": TOKEN, "X-LLM-Budget-Contract": CONTRACT})
        require(status == 200, "snapshot_failed")
        return tuple(snapshot["charged"][key] for key in ("attempts", "tokens", "cost_micro_cny"))

    def prepare_images(self, sources=None, *, journey=False):
        services = (*SERVICES, "gateway", "frontend") if journey else SERVICES
        self.docker("image", "inspect", REGISTRY_IMAGE)
        self.docker("image", "inspect", PYTHON_IMAGE)
        if sources is not None:
            require(isinstance(sources, dict) and set(sources) == set(services)
                    and all(isinstance(value, str) and re.fullmatch(r"[a-z0-9][a-z0-9._/:@-]*", value)
                            for value in sources.values()), "invalid_service_image_map")
            # Resolve mutable local names before creating/pushing our own refs.
            self.source_images = {value: self.docker("image", "inspect", "--format", "{{.Id}}", value).stdout.decode().strip()
                                  for value in sources.values()}
            sources = {name: self.source_images[value] for name, value in sources.items()}
        registry = self.project + "-registry"
        self.containers.add(registry)
        self.docker("run", "--detach", "--name", registry, "--pull", "never",
                    "--network", "bridge", "--publish", "127.0.0.1::5000",
                    "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                    "--label", "novelworld.purpose=offline-budget-lifecycle", REGISTRY_IMAGE)
        ports = json.loads(self.docker("inspect", "--format", "{{json .NetworkSettings.Ports}}", registry).stdout)
        binding, = ports["5000/tcp"]
        require(binding["HostIp"] == "127.0.0.1", "registry_not_loopback")
        registry_host = "127.0.0.1:" + binding["HostPort"]
        self.wait_http("http://" + registry_host, "/v2/")
        for service in services:
            tag = f"{registry_host}/{self.project}/{service}:fixture"
            self.image_refs.add(tag)
            if sources is None:
                context = self.files / ("build-" + service)
                context.mkdir()
                shutil.copyfile(self.owner_binary.parent / service, context / "service")
                (context / "service").chmod(0o755)
                (context / "Dockerfile").write_text(
                    f"FROM {PYTHON_IMAGE}\nCOPY service /app/service\n"
                    f"LABEL novelworld.fixture={self.project}\n")
                self.docker("build", "--network", "none", "--pull=false", "--tag", tag, str(context), timeout=120)
            else:
                self.docker("image", "tag", sources[service], tag)
            self.docker("push", tag, timeout=180)
            digests = json.loads(self.docker("image", "inspect", "--format", "{{json .RepoDigests}}", tag).stdout)
            digest, = [item for item in digests if item.startswith(tag.rsplit(":", 1)[0] + "@sha256:")]
            self.image_refs.add(digest)
            self.images[service] = digest
        if sources is not None:
            for service in services:
                require(self.docker("image", "inspect", "--format", "{{.Id}}", self.images[service]).stdout.decode().strip()
                        == sources[service], "published_image_identity_changed")
        print(f"diagnostic lifecycle: {len(self.images)} service images published only to isolated loopback registry", flush=True)

    def preflight_images(self):
        spec = importlib.util.spec_from_file_location("diagnostic_release", ROOT / "infra/docker/diagnostic_budget.py")
        adapter = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(adapter)
        registered = adapter.registration((ROOT / "tools/llm-budget/diagnostic-v1.json").read_bytes(), self.environment)
        manifest, config = {}, {"services": {}}
        for service in SERVICES:
            manifest[service.upper().replace("-", "_") + "_IMAGE"] = self.images[service]
            environment = {"LLM_DIAGNOSTIC_BUDGET_ID": self.budget_id, "INTERNAL_SERVICE_TOKEN": TOKEN,
                           "USER_SERVICE_URL": "http://127.0.0.1:8001" if service == "user-service" else "http://user-service:8001"}
            if service == "user-service":
                environment["LLM_DIAGNOSTIC_BUDGET_LIMITS"] = self.environment["LLM_DIAGNOSTIC_BUDGET_LIMITS"]
            config["services"][service] = {"image": self.images[service], "environment": environment}
        adapter.preflight(json.dumps(config), manifest, registered, self.project)
        for service in SERVICES:
            invalid_config, invalid_manifest = copy.deepcopy(config), dict(manifest)
            invalid_config["services"][service]["image"] = PYTHON_IMAGE
            invalid_manifest[service.upper().replace("-", "_") + "_IMAGE"] = PYTHON_IMAGE
            try:
                adapter.preflight(json.dumps(invalid_config), invalid_manifest, registered, self.project)
            except adapter.Invalid:
                pass
            else:
                raise Failure("unsupported_image_accepted")
        wrong_profile = copy.deepcopy(registered)
        wrong_profile["binding"]["profile_sha256"] = "0" * 64
        try:
            adapter.preflight(json.dumps(config), manifest, wrong_profile, self.project)
        except adapter.Invalid:
            pass
        else:
            raise Failure("mismatched_image_profile_accepted")
        remaining = self.docker("ps", "--all", "--quiet", "--filter", "name=^/" + self.project + "-budget-probe-").stdout
        require(not remaining.strip(), "capability_probe_container_remains")
        print("diagnostic lifecycle: real four-digest preflight and mixed/noncapable/profile rejection passed", flush=True)

    def prepare(self, image_sources=None):
        require(self.driver_binary.is_file(), "built_client_binary_required")
        self.docker("image", "inspect", PYTHON_IMAGE)
        self.docker("image", "inspect", PG_IMAGE)
        self.docker("image", "inspect", REGISTRY_IMAGE)
        if image_sources is None:
            require(self.owner_binary and all((self.owner_binary.parent / service).is_file() for service in SERVICES),
                    "all_service_binaries_required")
        self.prepare_images(image_sources)
        self.configure_budget(zero=True)
        self.preflight_images()
        self.prepare_network_mock()
        self.run("postgres", PG_IMAGE, ["postgres"],
                 {"POSTGRES_USER": "test", "POSTGRES_PASSWORD": "test", "POSTGRES_DB": "novelworld_test"}, alias="postgres")
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.docker("exec", self.project + "-postgres", "pg_isready", "-h", "127.0.0.1", "-U", "test", check=False).returncode == 0:
                break
            time.sleep(0.2)
        else:
            raise Failure("postgres_readiness_timeout")
        self.sql((ROOT / "infra/postgres/init.sql").read_text())

    def prepare_network_mock(self):
        """Shared no-egress provider fixture; never starts a product database."""
        network_args = []
        if self.subnet:
            selected = ipaddress.ip_network(self.subnet, strict=True)
            require(selected.version == 4 and selected.prefixlen == 28
                    and any(selected.subnet_of(ipaddress.ip_network(block))
                            for block in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")), "unsafe_fixture_subnet")
            networks = self.docker("network", "ls", "--quiet").stdout.decode().split()
            allocated = json.loads(self.docker("network", "inspect", *networks).stdout)
            occupied = [item["Subnet"] for network in allocated for item in (network["IPAM"]["Config"] or []) if item.get("Subnet")]
            routes = json.loads(command(["ip", "-j", "route"]).stdout)
            occupied += [route["dst"] for route in routes if route.get("dst") not in (None, "default")]
            require(not any(selected.overlaps(ipaddress.ip_network(block, strict=False)) for block in occupied),
                    "fixture_subnet_overlaps_existing_route")
            network_args = ["--subnet", str(selected)]
        # A timed-out Docker call can still have created this exact network.
        self.network_created = True
        self.docker("network", "create", "--internal", *network_args, "--label", "novelworld.issue=320", self.project)
        require(self.docker("network", "inspect", "--format", "{{.Internal}}", self.project).stdout.strip() == b"true",
                "network_not_internal")
        command(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                 "-subj", "/CN=NovelWorld isolated fixture CA", "-keyout", str(self.files / "ca.key"),
                 "-out", str(self.files / "ca.pem")])
        command(["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=api.deepseek.com",
                 "-addext", "subjectAltName=DNS:api.deepseek.com", "-keyout", str(self.files / "server.key"),
                 "-out", str(self.files / "server.csr")])
        command(["openssl", "x509", "-req", "-in", str(self.files / "server.csr"), "-CA", str(self.files / "ca.pem"),
                 "-CAkey", str(self.files / "ca.key"), "-CAcreateserial", "-days", "1", "-copy_extensions", "copy",
                 "-out", str(self.files / "server.pem")])
        mock_path = "/test/tests/e2e/diagnostic_budget_lifecycle.py"
        mock = self.run("mock", PYTHON_IMAGE, ["python3", mock_path, "--mock"],
                        mounts=((Path(__file__).resolve(), mock_path), (self.files, "/fixture")),
                        alias="mock")
        self.mock_origin = self.origin(mock, 8082)
        self.wait_http(self.mock_origin, "/state")
        # Same network namespace as the proxy: Docker internal network is the
        # egress authority, not HTTPS_PROXY. There is no public forwarding code.
        denied = self.docker("exec", mock, "python3", "-c",
                             "import socket; socket.create_connection(('1.1.1.1',443),timeout=1)", check=False)
        require(denied.returncode != 0, "public_egress_available")

    def configure_budget(self, zero=False):
        self.budget_id = str(uuid.uuid4())
        limits = {"profile": PROFILE, "max_attempts": 0 if zero else 30,
                  "max_tokens": 0 if zero else 20000000, "max_cost_micro_cny": 0 if zero else 35000000,
                  "expires_at": (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=1))
                      .strftime("%Y-%m-%dT%H:%M:%SZ")}
        self.environment = {"DATABASE_URL": "postgres://test:test@postgres:5432/novelworld_test",
                            "JWT_SECRET": TOKEN, "RUNTIME_CONFIG_KEY": TOKEN,
                            "INTERNAL_SERVICE_TOKEN": TOKEN, "LLM_API_KEY": "",
                            "LLM_DIAGNOSTIC_BUDGET_ID": self.budget_id,
                            "LLM_DIAGNOSTIC_BUDGET_LIMITS": json.dumps(limits),
                            "USER_SERVICE_URL": "http://mock:8081", "PORT": "8001",
                            "HTTPS_PROXY": "http://mock:3128", "NO_PROXY": NO_PROXY,
                            "SSL_CERT_FILE": "/fixture/ca.pem", "RUST_LOG": "error"}

    def journey_wiring(self, image_sources, evidence):
        """Real single-image cold adoption, not a semantic/two-version journey.

        Use the runner's registration, environment, release, snapshot, seal and
        cleanup methods. A private synthetic checkout changes only network/CA
        wiring. Full-journey quality gates intentionally fail on these partial
        fixtures, exercising retained PG evidence rather than inventing samples.
        """
        import live_deepseek_journey as runner
        control = runner.diagnostic
        require(shutil.which("socat") is not None, "fixture_socat_required")
        evidence = control.private_path(evidence, ROOT, directory=True)
        require(not any(evidence.iterdir()), "journey_evidence_not_empty")
        self.ingress_evidence = evidence
        self.journey_user_stack_before = runner.docker_inventory_snapshot()
        runner.write_private(evidence / "fixture-boundary.json", control.canonical({
            "kind": "offline-cold-adopt-wiring", "qualification_claim": False,
            "registry": "loopback-published bridge; local image transport only, no product/provider credentials",
            "product_network": "external Docker internal network, no public egress",
            "ingress": "host-only socat loopback to reserved internal nginx IPv4:80; no Docker published product port",
            "fixture_source_commit": runner.git(ROOT, "rev-parse", "HEAD"),
        }) + b"\n")
        control.sync_directory(evidence)
        self.prepare_images(image_sources, journey=True)
        self.prepare_network_mock()
        network = runner.docker_inspect("network", self.project)
        self.nginx_ip = ingress_address(network, self.project)
        checkout = self.files / "cold-adopt-source"
        command(["git", "clone", "--no-hardlinks", str(ROOT), str(checkout)], timeout=60)
        # No runtime implementation or release script is replaced. The explicit
        # test-only commit cannot be used as a genuine application upgrade pair.
        compose = isolated_journey_compose((checkout / "docker-compose.yml").read_text(),
                                          self.project, self.files / "ca.pem", self.nginx_ip)
        (checkout / "docker-compose.yml").write_text(compose)
        command(["git", "add", "docker-compose.yml"], cwd=checkout)
        command(["git", "-c", "user.name=Offline Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "core.hooksPath=/dev/null", "commit", "-m", "test-only isolated cold adoption network"], cwd=checkout)
        sha = command(["git", "rev-parse", "HEAD"], cwd=checkout).stdout.decode().strip()
        infrastructure = {
            "POSTGRES_IMAGE": PG_IMAGE,
            "REDIS_IMAGE": "redis:8.10.1-alpine@sha256:becdda6c7f4b3fb42e42fd7f120bbf5c54c4caaaf16f26da24e4563d2c1f0576",
            "NGINX_IMAGE": "nginx:alpine@sha256:db35bfc6b2951e7f8a72db5db120288c127ffaeeb4a6d4b95a26fead017d5913",
        }
        manifest = {"RELEASE_VERSION": "offline-cold-adopt", "RELEASE_GIT_SHA": sha, **infrastructure,
                    **{service.upper().replace("-", "_") + "_IMAGE": reference
                       for service, reference in self.images.items()}}
        manifest_path = evidence / "fixture-release.env"
        runner.write_private(manifest_path, "".join(f"{key}={value}\n" for key, value in manifest.items()).encode())
        image_ids = {key: runner.docker_inspect("image", manifest[key])["Id"]
                     for key in runner.APPLICATION_IMAGE_KEYS}
        profile_bytes = (ROOT / control.PROFILE_PATH).read_bytes()
        for zero in (True, False):
            output = evidence / ("zero" if zero else "nonzero")
            output.mkdir(mode=0o700)
            identifier = str(uuid.uuid4())
            registration_value = {
                "schema": control.REGISTRATION_SCHEMA, "budget_id": identifier,
                "hypothesis": "Synthetic offline cold adoption and budget wiring only",
                "candidate_git_sha": sha,
                "base_manifest_sha256": control.digest(manifest_path.read_bytes()),
                "candidate_manifest_sha256": control.digest(manifest_path.read_bytes()),
                "base_application_image_ids": image_ids, "candidate_application_image_ids": image_ids,
                "profile_sha256": control.digest(profile_bytes),
                "product_fixture_sha256": control.digest((checkout / runner.PRODUCT_INPUT).read_bytes()),
                "prompt_schema_identities": control.source_identities(checkout, sha, sha),
                "limits": {"profile": PROFILE, "max_attempts": 0 if zero else 5,
                           "max_tokens": 0 if zero else 2000000, "max_cost_micro_cny": 0 if zero else 5000000,
                           "expires_at": (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(hours=1))
                                         .strftime("%Y-%m-%dT%H:%M:%SZ")},
                "output_dir": str(output), "ledger_path": str(evidence / (identifier + ".jsonl")),
            }
            registration_path = evidence / (identifier + ".json")
            encoded = control.canonical(registration_value)
            runner.write_private(registration_path, encoded)
            registration = control.load_registration(
                registration_path, control.digest(encoded), root=checkout, git_sha=sha,
                output=output, base_manifest=manifest_path, candidate_manifest=manifest_path,
                prompt_schema_identities=control.source_identities(checkout, sha, sha),
            )
            config = evidence / (identifier + "-synthetic-config.json")
            runner.write_private(config, control.canonical({"provider": "deepseek", "model": MODEL,
                "api_url": "https://api.deepseek.com", "api_key": KEY, "thinking_enabled": False}))
            journey = runner.Journey(checkout, config, output, sha, manifest_path, manifest_path,
                                     None, None, "bash", "Diagnostic", diagnostic_registration=registration)
            self.journeys.append(journey)

            def cold_adopt():
                journey.user_stack_before = runner.docker_inventory_snapshot()
                require(not runner.attempt_resources(journey.user_stack_before, journey.project, journey.prefix),
                        "journey_project_collision")
                journey.inventory_captured = True
                journey.cleanup_required = True
                runner.write_private(output / "docker-inventory-before.json",
                                     control.canonical(journey.user_stack_before) + b"\n")
                control.sync_directory(output)
                journey.prepare_runtime()
                self.start_ingress(journey.port)
                journey.release("adopt", manifest_path, release_name="base")
                journey.stack_started = True
                journey.wait_gateway()
                journey.verify_release_images("base", manifest)
                for service in (*SERVICES, "gateway", "frontend", "nginx", "postgres"):
                    container = runner.docker_inspect("container", journey.prefix + "-" + service)
                    networks = container["NetworkSettings"]["Networks"]
                    require(set(networks) == {self.project}, "journey_public_network_attached")
                    if service == "nginx":
                        require(container["Config"]["Labels"]["com.docker.compose.project"] == journey.project
                                and re.fullmatch(r"[0-9a-f]{64}", container["Id"])
                                and networks[self.project]["IPAddress"] == str(self.nginx_ip),
                                "fixture_ingress_target_changed")
                require(journey.diagnostic_checkpoint("initial")["charged"]["attempts"] == 0, "cold_budget_not_empty")
                require(journey.diagnostic_owner_control()["charged"]["attempts"] == 0, "owner_budget_not_empty")
                admin = runner.request_json(journey.api + "/setup/init", method="POST",
                    value={"email": "fixture@example.invalid", "password": "FixtureOnlyPassword12345", "name": "Fixture"},
                    expected=(201,))
                before = self.state()
                result = runner.request_json(journey.api + "/settings/llm", method="PUT", token=admin["access_token"],
                    value={"provider": "deepseek", "model": MODEL, "thinking_enabled": False, "api_key": KEY},
                    expected=(422,) if zero else (200,), timeout=15)
                after = self.state()
                require(after["connects"] - before["connects"] == (0 if zero else 1), "settings_connection_count")
                if zero:
                    require(result.get("error", {}).get("code") == "llm_unavailable", "settings_rejection_changed")
                else:
                    require(result == {"provider": "deepseek", "model": MODEL, "thinking_enabled": False,
                                       "api_key_configured": True, "scope": "platform"}, "settings_identity_changed")
                budget = journey.diagnostic_checkpoint("settings")
                require(budget["charged"] == {"attempts": 0 if zero else 1, "tokens": 0 if zero else 12,
                                              "cost_micro_cny": 0 if zero else 48}, "settings_receipts_differ")
                # Readiness and ledger persistence, not a second paid call.
                journey.collect_metrics("before-restart", ["user-service", "agent-service"])
                journey.collect_response_models("before-restart", ["user-service", "agent-service"])
                journey.compose("restart", "user-service", "agent-service")
                journey.wait_gateway()
                journey.wait_agent_ready()
                require(journey.diagnostic_checkpoint("restart") == budget, "restart_changed_receipts")
            # The fixture supplies only the partial product steps; use the real
            # single-start/cancellation/failure-safe terminal wrapper unchanged.
            journey.execute = cold_adopt
            try:
                require(runner.run_diagnostic(journey) == 1, "partial_journey_must_not_pass")
            finally:
                report_cold_adoption_status(journey, zero)
                report_cold_release_status(journey, zero)
                self.stop_ingresses()
            require(set(journey.diagnostic_failures) <= {
                "response_model_observation_count_mismatch", "llm_budget_failed", "diagnostic_cleanup_residue",
                "diagnostic_completion_unproven",
            }, "unexpected_journey_terminal_failure")
            require(journey.private_report.get("diagnostic_payers_stopped") is True, "payer_stop_not_proven")
            require(journey.private_report.get("diagnostic_metrics_reconciled") is True,
                    "terminal_metrics_not_reconciled")
            final_snapshot = json.loads((output / "budget-terminal.json").read_bytes())
            require(final_snapshot == journey.diagnostic_last_snapshot
                    and control.reconcile_snapshot(registration, final_snapshot)["sealed"] is True,
                    "terminal_not_sealed")
            require((output / "pre-cleanup-private.json").is_file(), "terminal_receipts_not_durable")
            residue = journey.private_report["environment"]["attempt_resource_residue"]
            require(residue == ["volumes:" + journey.project + "_postgres_data"], "unexpected_retained_resources")
            volume = journey.project + "_postgres_data"
            require(runner.docker_inspect("volume", volume)["Labels"]["com.docker.compose.project"] == journey.project,
                    "retained_volume_owner_changed")
            # Explicit non-paid evidence-recovery cleanup after the failure-path
            # retention assertion, never resume/refill the failed registration.
            self.docker("volume", "rm", volume)
            print("diagnostic cold adoption: " + ("zero" if zero else "nonzero") +
                  " Settings, restart, terminal evidence and retained-volume cleanup passed", flush=True)

    def start_ingress(self, port):
        network = json.loads(self.docker("network", "inspect", self.project).stdout)[0]
        require(ingress_address(network, self.project) == self.nginx_ip, "fixture_ingress_target_changed")
        # The listener must exist before release.sh's single cold readiness curl.
        # Only this synthetic Compose assigns nginx the reserved internal address.
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", port))
        process = subprocess.Popen([shutil.which("socat"), "-T", "10",
            f"TCP4-LISTEN:{port},bind=127.0.0.1,reuseaddr,fork",
            f"TCP4:{self.nginx_ip}:80,connect-timeout=2"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True)
        self.ingresses.append((process, port))
        import live_deepseek_journey as runner
        runner.write_private(self.ingress_evidence / f"ingress-{port}.json", runner.diagnostic.canonical({
            "pid": process.pid, "pgid": process.pid, "bind": "127.0.0.1", "port": port,
            "nginx_ip": str(self.nginx_ip), "target_port": 80, "network_id": network["Id"],
            "network_name": self.project, "phase": "started; absence must be independently verified",
        }) + b"\n")
        runner.diagnostic.sync_directory(self.ingress_evidence)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            require(process.poll() is None, "fixture_ingress_exited")
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    require(process.poll() is None, "fixture_ingress_exited")
                    return
            except OSError:
                time.sleep(0.05)
        raise Failure("fixture_ingress_start_timeout")

    def stop_ingresses(self):
        failures = []
        for process, port in getattr(self, "ingresses", ()):
            try:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                deadline = getattr(self, "cleanup_deadline", None)
                remaining = 5 if deadline is None else min(5, deadline - time.monotonic())
                require(remaining > 0, "fixture_cleanup_timeout")
                process.wait(timeout=remaining)
                with socket.socket() as listener:
                    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                    listener.bind(("127.0.0.1", port))
            except (Failure, OSError, subprocess.SubprocessError):
                failures.append(port)
        if not failures:
            self.ingresses = []
        require(not failures, "fixture_ingress_stop_unproven")

    def register(self, zero=False):
        if not zero:
            self.configure_budget()
        provision = self.run("provision", self.images["user-service"], ["/app/service", "--provision-diagnostic-budget"],
                             self.environment, ((self.files, "/fixture"),))
        require(self.exit_status(provision) == 0, "provision_failed")
        self.remove(provision)
        self.start_owner()

    def start_owner(self):
        self.owner = self.run("owner", self.images["user-service"], ["/app/service"], self.environment,
                              ((self.files, "/fixture"),), alias="owner")
        self.owner_origin = self.origin(self.owner, 8001)
        self.wait_http(self.owner_origin, "/setup/status")

    def driver(self, kind="direct", mode="sync", expect="success", json_mode=False):
        environment = {key: value for key, value in self.environment.items()
                       if key in ("LLM_DIAGNOSTIC_BUDGET_ID", "USER_SERVICE_URL", "INTERNAL_SERVICE_TOKEN",
                                  "HTTPS_PROXY", "NO_PROXY", "SSL_CERT_FILE")}
        environment.update(NOVELWORLD_TEST_DIAGNOSTIC_ISOLATED="1", NOVELWORLD_TEST_BUDGET_CLIENT=kind,
                           NOVELWORLD_TEST_BUDGET_MODE=mode, NOVELWORLD_TEST_BUDGET_EXPECT=expect,
                           NOVELWORLD_TEST_BUDGET_JSON=str(json_mode).lower())
        driver = self.run("driver", PYTHON_IMAGE,
                          ["/app/test"],
                          environment, ((self.driver_binary, "/app/test"), (self.files, "/fixture")))
        require(self.exit_status(driver) == 0, "production_client_driver_failed")
        self.remove(driver)

    def verify_delta(self, before, stats, expected, provider_count, settles, reserves=None):
        require(tuple(now - old for now, old in zip(self.charged(), before)) == expected, "ledger_delta_mismatch")
        after = self.state()
        require(after["reserve"] - stats["reserve"] == (expected[0] if reserves is None else reserves),
                "reserve_rpc_delta_mismatch")
        require(after["providers"] - stats["providers"] == provider_count, "provider_delta_mismatch")
        require(after["settle"] - stats["settle"] == settles, "settlement_delta_mismatch")
        require(after["errors"] == 0, "mock_contract_error")
        # Read the authoritative receipts, not only the HTTP aggregate. Both
        # ACK-loss cases must retain exactly the actual committed owner state.
        receipts = json.loads(self.sql(f"""
            SELECT coalesce(json_agg(row_to_json(a)), '[]'::json)
            FROM diagnostic_llm_attempts a
            WHERE budget_id='{self.budget_id}' AND ordinal>{before[0]};
        """))
        require(len(receipts) == expected[0], "receipt_count_mismatch")
        require(sum(row["settled"] for row in receipts) == settles, "receipt_settlement_mismatch")
        for row in receipts:
            require(row["operation"] == "setup_connection" and row["output_limit"] == 8
                    and row["reservation_tokens"] == 1048584
                    and row["reservation_cost_micro_cny"] == 3145800, "receipt_reservation_mismatch")
            actual = tuple(row[key] for key in ("settlement_model", "input_tokens", "output_tokens", "cached_input_tokens"))
            require(actual == ((MODEL, 10, 2, None) if row["settled"] else (None,) * 4),
                    "receipt_usage_mismatch")

    def exercise(self, image_sources=None):
        self.prepare(image_sources)
        self.register(zero=True)
        zero_environment = dict(self.environment)
        status, admin = http(self.owner_origin, "/setup/init", "POST",
                             {"email": "budget-fixture@example.invalid", "password": "FixtureOnlyPassword12345", "name": "Fixture"})
        require(status == 201, "admin_bootstrap_failed")
        headers = {"X-User-Id": admin["user"]["id"]}
        settings = {"provider": "deepseek", "model": MODEL, "api_key": KEY, "thinking_enabled": False}
        before, stats = self.charged(), self.state()
        require(http(self.owner_origin, "/settings/llm", "PUT", settings, headers)[0] != 200, "zero_settings_dispatched")
        self.driver(expect="control")
        self.driver(mode="stream", expect="control")
        self.driver(mode="embed", expect="control")
        self.verify_delta(before, stats, (0, 0, 0), 0, 0, reserves=3)
        require(self.state()["connects"] == 0, "zero_budget_opened_provider_connection")
        print("diagnostic lifecycle: zero budget denies Settings/chat/stream/embed before provider IO", flush=True)
        self.remove(self.owner)
        self.register()
        before, stats = self.charged(), self.state()
        require(http(self.owner_origin, "/settings/llm", "PUT", settings, headers)[0] == 200, "settings_test_failed")
        self.verify_delta(before, stats, (1, 12, 48), 1, 1)
        for kind, mode, provider_mode, control_mode, expect, delta, count, settles in (
            ("direct", "sync", "ok", "ok", "success", (1, 12, 48), 1, 1),
            ("static", "sync", "ok", "ok", "success", (1, 12, 48), 1, 1),
            ("runtime", "sync", "ok", "ok", "success", (1, 12, 48), 1, 1),
            ("runtime", "stream", "ok", "ok", "success", (1, 12, 48), 1, 1),
            ("direct", "sync", "empty", "ok", "success", (2, 24, 96), 2, 2),
            ("direct", "sync", "retry", "ok", "success", (2, 1048596, 3145848), 2, 1),
            ("direct", "drop", "ok", "ok", "success", (1, 1048584, 3145800), 1, 0),
            ("direct", "sync", "missing", "ok", "evidence", (1, 1048584, 3145800), 1, 0),
            ("direct", "sync", "malformed", "ok", "evidence", (1, 1048584, 3145800), 1, 0),
            ("direct", "sync", "conflicting", "ok", "evidence", (1, 1048584, 3145800), 1, 0),
            ("direct", "sync", "empty-malformed", "ok", "evidence", (1, 1048584, 3145800), 1, 0),
            ("direct", "sync", "ok", "lose_reserve", "control", (1, 1048584, 3145800), 0, 0),
            ("direct", "sync", "ok", "lose_settle", "evidence", (1, 12, 48), 1, 1),
            ("runtime", "stream", "missing", "ok", "evidence", (1, 1048584, 3145800), 1, 0),
            ("runtime", "stream", "malformed", "ok", "evidence", (1, 1048584, 3145800), 1, 0),
            ("runtime", "stream", "ok", "lose_settle", "evidence", (1, 12, 48), 1, 1),
        ):
            self.mode(provider_mode, control_mode)
            before, stats = self.charged(), self.state()
            self.driver(kind, mode, expect, provider_mode.startswith("empty"))
            self.verify_delta(before, stats, delta, count, settles)
            print(f"diagnostic lifecycle: {kind}/{mode}/{provider_mode}/{control_mode} passed", flush=True)
        self.mode()
        retained = self.charged()
        old_container = self.docker("inspect", "--format", "{{.Id}}", self.owner).stdout
        self.remove(self.owner)
        self.start_owner()
        require(self.docker("inspect", "--format", "{{.Id}}", self.owner).stdout != old_container, "owner_not_recreated")
        require(self.charged() == retained, "recreation_refilled_budget")
        before, stats = self.charged(), self.state()
        self.driver("runtime")
        self.verify_delta(before, stats, (1, 12, 48), 1, 1)
        self.release_refusal_checks()
        # Missing registration refuses normal startup; it cannot recreate the row.
        self.remove(self.owner)
        missing = str(uuid.uuid4())
        self.environment["LLM_DIAGNOSTIC_BUDGET_ID"] = missing
        failed = self.run("owner", self.images["user-service"], ["/app/service"], self.environment,
                          ((self.files, "/fixture"),), alias="owner")
        require(self.exit_status(failed) != 0, "missing_registration_started")
        require(self.sql(f"SELECT count(*) FROM diagnostic_llm_budgets WHERE budget_id='{missing}';") == "0",
                "missing_registration_recreated")
        self.remove(failed)
        # Delete only this fixture's empty zero-budget row, then prove ordinary
        # startup cannot refill a previously provisioned registration either.
        deleted = zero_environment["LLM_DIAGNOSTIC_BUDGET_ID"]
        self.sql(f"DELETE FROM diagnostic_llm_budgets WHERE budget_id='{deleted}';")
        stats = self.state()
        failed = self.run("owner", self.images["user-service"], ["/app/service"], zero_environment,
                          ((self.files, "/fixture"),), alias="owner")
        require(self.exit_status(failed) != 0, "deleted_registration_started")
        require(self.sql(f"SELECT count(*) FROM diagnostic_llm_budgets WHERE budget_id='{deleted}';") == "0",
                "deleted_registration_recreated")
        require(self.state()["providers"] == stats["providers"], "deleted_registration_dispatched")
        require(self.state()["errors"] == 0, "mock_contract_error")

    def release_refusal_checks(self):
        """Real release commands over explicitly negative Git/state fixtures.

        The two commits describe test metadata, never two product implementations.
        Only config/pull and isolated probes may execute; an attempted deployment
        fails the test before it can start an externally networked service.
        """
        repository = self.files / "release-checkout"
        repository.mkdir()
        for relative in ("docker-compose.yml", "docs/adr/0002-minimal-bootstrap-and-deferred-runtime-configuration.md"):
            target = repository / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        shutil.copytree(ROOT / "infra/postgres/migrations", repository / "infra/postgres/migrations")
        (repository / ".gitignore").write_text(".env\n.release/\n")
        def git(*args):
            return command(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", *args], cwd=repository).stdout.decode().strip()
        git("init", "-q", "--template=")
        git("config", "user.name", "Negative release fixture")
        git("config", "user.email", "release-fixture@example.invalid")
        git("add", ".")
        git("commit", "-qm", "Negative release metadata fixture; not a product artifact")
        current_sha = git("rev-parse", "HEAD")
        git("commit", "--allow-empty", "-qm", "Second negative metadata identity; not an upgrade implementation")
        other_sha = git("rev-parse", "HEAD")
        git("checkout", "--detach", current_sha)

        state_dir = repository / ".release"
        state_dir.mkdir(mode=0o700)
        limits = json.loads(self.environment["LLM_DIAGNOSTIC_BUDGET_LIMITS"])
        profile = (ROOT / "tools/llm-budget/diagnostic-v1.json").read_bytes()
        import hashlib
        marker = {"binding": {"budget_id": self.budget_id, "contract": CONTRACT, "profile": PROFILE,
                              "profile_sha256": hashlib.sha256(profile).hexdigest()},
                  "limits": limits, "completed": True}
        marker_path = state_dir / "diagnostic-provisioning.json"
        marker_path.write_text(json.dumps(marker))
        marker_path.chmod(0o600)
        environment = {"POSTGRES_USER": "test", "POSTGRES_PASSWORD": "test", "POSTGRES_DB": "novelworld_test",
                       "CACHE_MODE": "postgres", "JWT_SECRET": TOKEN,
                       "RUNTIME_CONFIG_KEY": TOKEN, "INTERNAL_SERVICE_TOKEN": TOKEN,
                       "LLM_DIAGNOSTIC_BUDGET_ID": self.budget_id,
                       "LLM_DIAGNOSTIC_BUDGET_LIMITS": json.dumps(limits), "LLM_API_KEY": ""}
        (repository / ".env").write_text("".join(f"{key}={value}\n" for key, value in environment.items()))
        (repository / ".env").chmod(0o600)
        def manifest(sha, invalid=False):
            data = {"RELEASE_VERSION": "negative-fixture", "RELEASE_GIT_SHA": sha}
            for key in ("GATEWAY", "USER_SERVICE", "NOVEL_SERVICE", "AGENT_SERVICE", "NARRATIVE_SERVICE",
                        "FRONTEND", "POSTGRES", "REDIS", "NGINX"):
                service = key.lower().replace("_", "-")
                data[key + "_IMAGE"] = self.images.get(service, PYTHON_IMAGE)
            data["POSTGRES_IMAGE"] = PG_IMAGE
            if invalid:
                data["NARRATIVE_SERVICE_IMAGE"] = PYTHON_IMAGE
            return "".join(f"{key}={value}\n" for key, value in data.items())
        current_path, previous_path = state_dir / "current.env", state_dir / "previous.env"
        candidate = self.files / "negative-candidate.env"
        trace = self.files / "release-docker-trace.jsonl"
        spy_bin = self.files / "release-spy-bin"
        spy_bin.mkdir()
        executable = spy_bin / "docker"
        executable.write_text("#!/bin/sh\nexec " + shlex.quote(sys.executable) + " "
                              + shlex.quote(str(ROOT / "tests/e2e/diagnostic_release_docker_spy.py")) + ' "$@"\n')
        executable.chmod(0o700)
        release_environment = {"PATH": str(spy_bin) + ":" + os.environ["PATH"], "HOME": os.environ["HOME"],
                               "RELEASE_STATE_DIR": str(state_dir), "RELEASE_COMPOSE_PROJECT": self.project,
                               "RELEASE_CONTAINER_PREFIX": self.project, "RELEASE_HTTP_BIND": "127.0.0.1",
                               "RELEASE_HTTP_PORT": "18080", "NWQ_REAL_DOCKER": shutil.which("docker"),
                               "NWQ_DOCKER_TRACE": str(trace), "NWQ_PROJECT": self.project}
        for key in ("DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH", "DOCKER_CONFIG"):
            if key in os.environ:
                release_environment[key] = os.environ[key]
        def snapshot():
            containers = []
            for name in (self.owner, self.project + "-postgres"):
                data = json.loads(self.docker("inspect", "--format", "{{json .}}", name).stdout)
                containers.append((data["Id"], data["Image"], data["Config"]["Image"], data["State"]["Status"], data["State"]["StartedAt"],
                                   data["State"].get("Health", {}).get("Status")))
            ledger = self.sql(f"""
                SELECT row_to_json(b) FROM diagnostic_llm_budgets b WHERE budget_id='{self.budget_id}';
                SELECT row_to_json(a) FROM diagnostic_llm_attempts a WHERE budget_id='{self.budget_id}' ORDER BY ordinal;
            """)
            return containers, ledger, self.state(), marker_path.read_bytes()
        def release(*args):
            return command(["bash", str(ROOT / "infra/docker/release.sh"), *args], cwd=repository,
                           env=release_environment, check=False, timeout=120)
        for scenario, action, bad_current, bad_target, expected_probes in (
            ("candidate", "upgrade", False, True, 4),
            ("current", "upgrade", True, False, 8),
            ("previous", "rollback", False, True, 4),
        ):
            current_path.write_text(manifest(current_sha, bad_current))
            previous_path.write_text(manifest(other_sha, bad_target))
            candidate.write_text(manifest(other_sha, bad_target))
            before = snapshot()
            trace.write_text("")
            result = release(action, str(candidate) if action == "upgrade" else other_sha)
            require(result.returncode != 0 and b"release: diagnostic budget preflight failed" in result.stderr,
                    "release_did_not_reach_capability_refusal_" + scenario)
            require(snapshot() == before, "release_refusal_changed_authority_" + scenario)
            require(current_path.read_text() == manifest(current_sha, bad_current)
                    and previous_path.read_text() == manifest(other_sha, bad_target), "release_manifest_changed")
            require(not (state_dir / "schema-transition.pending").exists()
                    and not (state_dir / "rollback.pending").exists(), "release_transition_started")
            staged = state_dir / "candidate.env"
            require(staged.read_text() == candidate.read_text() if action == "upgrade" else not staged.exists(),
                    "unexpected_candidate_retention")
            require(git("rev-parse", "HEAD") == other_sha and not git("status", "--porcelain"),
                    "unexpected_release_checkout")
            calls = [json.loads(line) for line in trace.read_text().splitlines()]
            require(sum(call[0] == "create" for call in calls) == expected_probes
                    and sum(call[0] == "start" for call in calls) == expected_probes,
                    "release_probe_order_mismatch")
            require(any(call[0] == "compose" and "pull" in call for call in calls), "release_did_not_pull")
            require(all(call[0] != "compose" or any(part in call for part in ("config", "pull")) for call in calls),
                    "release_attempted_deployment")
            # A failed preflight leaves HEAD at the attempted metadata revision.
            # Re-enter the actual supported preflight on a correct current set:
            # no bypass via the stale checkout and no data/marker change.
            current_path.write_text(manifest(current_sha))
            before_recheck = snapshot()
            reentry = release("preflight", str(current_path))
            if reentry.returncode != 0:
                phases = re.findall(rb"(?m)^diagnostic phase=(probe_(?:create|start|expectation|cleanup_rm|cleanup_ps|cleanup_unproven))$",
                                    reentry.stderr)
                phase = phases[-1].decode() if phases else "unknown"
                raise Failure("release_reentry_failed_" + scenario + "_" + phase)
            require(snapshot() == before_recheck and git("rev-parse", "HEAD") == other_sha,
                    "release_reentry_changed_authority")
            print(f"diagnostic lifecycle: real release {scenario} rejection and safe preflight reentry passed", flush=True)

    def cleanup(self):
        self.cleanup_deadline = time.monotonic() + 60
        failures = []
        try:
            self.stop_ingresses()
        except Failure:
            try:
                self.stop_ingresses()
            except Failure:
                failures.append("ingress_cleanup_unproven")
        # The runner normally cleans each child project. Also track its exact
        # residual container IDs for exceptional terminal/report/ledger paths.
        # Never delete a named PG volume here: unproven evidence stays retained.
        for journey in getattr(self, "journeys", ()):
            try:
                require(re.fullmatch(r"nwq-[a-f0-9]{10}", journey.project), "child_project_invalid")
                rows = self.docker("ps", "--all", "--no-trunc", "--filter",
                    "label=com.docker.compose.project=" + journey.project,
                    "--format", "{{.ID}} {{.Names}}").stdout.decode().splitlines()
                for row in rows:
                    identifier, name = row.split()
                    require(re.fullmatch(r"[0-9a-f]{64}", identifier) and name.startswith(journey.project + "-"),
                            "child_container_ownership_unproven")
                    self.containers.add(identifier)
            except (Failure, OSError, ValueError, subprocess.SubprocessError):
                failures.append("child_cleanup_unproven")
        for container in sorted(self.containers):
            try:
                require(self.docker("rm", "--force", "--volumes", container, check=False).returncode == 0,
                        "container_cleanup_failed")
                require(not self.docker("ps", "--all", "--quiet", "--filter",
                                       "id=" + container if re.fullmatch(r"[0-9a-f]{64}", container)
                                       else "name=^/" + container + "$"
                                       ).stdout.strip(), "container_removal_unproven")
            except (Failure, OSError, subprocess.SubprocessError):
                failures.append(container)
        try:
            if self.network_created:
                networks = self.docker("network", "ls", "--filter", "label=novelworld.issue=320",
                                       "--format", "{{.Name}}").stdout.decode().splitlines()
                if self.project in networks:
                    require(self.docker("network", "rm", self.project, check=False).returncode == 0,
                            "network_cleanup_failed")
                    remaining = self.docker("network", "ls", "--filter", "label=novelworld.issue=320",
                                            "--format", "{{.Name}}").stdout.decode().splitlines()
                    require(self.project not in remaining, "network_removal_unproven")
        except (Failure, OSError, subprocess.SubprocessError):
            failures.append(self.project)
        finally:
            for reference in sorted(self.image_refs):
                try:
                    references = self.docker("image", "ls", "--digests", "--format",
                                             "{{.Repository}}:{{.Tag}} {{.Repository}}@{{.Digest}}").stdout.decode().split()
                    if reference in references:
                        require(self.docker("image", "rm", reference, check=False).returncode == 0,
                                "image_cleanup_failed")
                except (Failure, OSError, subprocess.SubprocessError):
                    failures.append(reference)
            for reference, original_id in self.source_images.items():
                try:
                    require(self.docker("image", "inspect", "--format", "{{.Id}}", reference).stdout.decode().strip()
                            == original_id, "source_image_reference_changed")
                except (Failure, OSError, subprocess.SubprocessError):
                    failures.append(reference)
            try:
                self.temporary.cleanup()
            finally:
                self.cleanup_deadline = None
        require(not failures, "isolated_cleanup_failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mock", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--owner-binary", type=Path)
    parser.add_argument("--client-binary", type=Path)
    parser.add_argument("--capability-images", type=Path,
                        help="Probe only: JSON map of four existing service images; no database or provider setup")
    parser.add_argument("--runtime-images", type=Path,
                        help="Full fixture using four existing service images instead of packaging normal binaries")
    parser.add_argument("--journey-images", type=Path,
                        help="Cold-adopt wiring only: six release-built application images, no semantic journey")
    parser.add_argument("--journey-output", type=Path,
                        help="Pre-created empty private directory for retained cold-adopt evidence")
    parser.add_argument("--subnet", help="Optional unused RFC1918 /28 for hosts with exhausted Docker default pools")
    args = parser.parse_args()
    if args.mock:
        mock_server()
        return
    if args.journey_images or args.journey_output:
        require(args.journey_images and args.journey_output
                and not any((args.owner_binary, args.client_binary, args.capability_images, args.runtime_images)),
                "journey_fixture_inputs_required")
        lifecycle = Lifecycle(None, None, args.subnet)
        def cancel_journey(*_):
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            signal.signal(signal.SIGINT, signal.SIG_IGN)
            raise Failure("fixture_cancelled")
        signal.signal(signal.SIGTERM, cancel_journey)
        signal.signal(signal.SIGINT, cancel_journey)
        try:
            lifecycle.journey_wiring(json.loads(args.journey_images.read_bytes()), args.journey_output)
        finally:
            lifecycle.cleanup()
        import live_deepseek_journey as runner
        require(runner.docker_inventory_snapshot() == lifecycle.journey_user_stack_before,
                "user_docker_inventory_changed")
        print("diagnostic cold adoption: offline wiring passed; no live/upgrade/qualification claim")
        return
    require(bool(args.capability_images) != bool(args.owner_binary or args.client_binary or args.runtime_images),
            "choose_binaries_or_images")
    if not args.capability_images:
        require(args.client_binary and bool(args.owner_binary) != bool(args.runtime_images), "built_binary_paths_required")
    lifecycle = Lifecycle(args.owner_binary, args.client_binary, args.subnet)
    def cancel(*_):
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        raise Failure("fixture_cancelled")
    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    try:
        if args.capability_images:
            lifecycle.prepare_images(json.loads(args.capability_images.read_text()))
            lifecycle.configure_budget(zero=True)
            lifecycle.preflight_images()
        else:
            lifecycle.exercise(json.loads(args.runtime_images.read_text()) if args.runtime_images else None)
    finally:
        lifecycle.cleanup()
    print("diagnostic lifecycle: " + ("existing-image capability checks" if args.capability_images else
          "actual Settings/client/PG, lost ACK and owner recreation") + " passed; isolated resources removed")


if __name__ == "__main__":
    try:
        main()
    except (Failure, OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
        # Never echo raw HTTP bodies, process output, environment or credentials.
        raise SystemExit("diagnostic lifecycle: " + (str(error) if isinstance(error, Failure) else "fixture_failed_" + type(error).__name__))
