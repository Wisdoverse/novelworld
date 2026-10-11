#!/usr/bin/env python3
"""Check notification retries, resolution and restart dedup in an owned stack."""

from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import ipaddress
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import uuid


ROOT = Path(__file__).resolve().parent.parent.parent
MONITORING = ROOT / "infra/monitoring"
PLACEHOLDER = "https://alerts.example.invalid/novelworld"
CLIENT = """import json,sys,urllib.request
r=json.load(sys.stdin)
body=None if r['data'] is None else json.dumps(r['data']).encode()
req=urllib.request.Request(r['url'],data=body,method=r['method'],headers={'Content-Type':'application/json'})
with urllib.request.urlopen(req,timeout=2) as response:
    print(response.read(65536).decode())
"""


def receiver():
    state = {"down": False, "attempts": [], "deliveries": []}
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def reply(self, status, body=b"", content_type="application/json"):
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def do_GET(self):
            with lock:
                if self.path == "/state":
                    self.reply(200, json.dumps(state).encode())
                elif self.path == "/metrics":
                    down = state["down"] and self.server.server_port == 8004
                    self.reply(503 if down else 200, b"fixture_available 1\n",
                               "text/plain; version=0.0.4")
                else:
                    self.reply(404)

        def do_POST(self):
            size = int(self.headers.get("Content-Length", "0"))
            if not 0 <= size <= 65536:
                self.reply(413)
                return
            data = self.rfile.read(size)
            with lock:
                if self.path in {"/down", "/up"}:
                    state["down"] = self.path == "/down"
                    self.reply(200, b"{}")
                    return
                if self.path != "/notify" or len(state["attempts"]) >= 64:
                    self.reply(404)
                    return
                message = json.loads(data)
                assert message["version"] == "4"
                attempt = len(state["attempts"]) + 1
                state["attempts"].append({"number": attempt, "at": time.monotonic()})
                if attempt > 2:
                    state["deliveries"].append(message)
            if attempt == 2:
                # The delayed rejection commits nothing after the client times out.
                time.sleep(5)
            self.reply(503 if attempt <= 2 else 200, b"{}")

    for port in (8080, 8001, 8002, 8003, 8004):
        server = ThreadingHTTPServer(("0.0.0.0", port), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
    threading.Event().wait()


def run(*args, env=None, data=None, timeout=30, check=True):
    result = subprocess.run(args, input=data, capture_output=True, text=True,
                            env=env, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"{args[0]} failed: {result.stderr[-2000:]}")
    return result


def wait_for(description, predicate, seconds=40):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.5)
    raise AssertionError(f"Timed out: {description}")


def choose_subnets(networks, routes):
    occupied = [ipaddress.ip_network(item["Subnet"], strict=False)
                for network in networks for item in (network["IPAM"].get("Config") or [])
                if item.get("Subnet")]
    occupied.extend(ipaddress.ip_network(route["dst"], strict=False) for route in routes
                    if route.get("dst") not in (None, "default"))
    selected = []
    for subnet in ipaddress.ip_network("10.254.240.0/20").subnets(new_prefix=28):
        if not any(block.version == 4 and subnet.overlaps(block) for block in occupied):
            selected.append(str(subnet))
            if len(selected) == 2:
                return selected
    raise RuntimeError("No unused private /28 pair is available for the notification drill")


def drill():
    spec = importlib.util.spec_from_file_location(
        "qualification_network", ROOT / "infra/docker/qualification_network.py")
    assert spec and spec.loader
    adapter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(adapter)
    subnets = choose_subnets(*adapter.inventory())
    project = "nw-alert-drill-" + uuid.uuid4().hex[:12]
    print("notification drill: owned project " + project, flush=True)
    compose_env = {key: value for key, value in os.environ.items() if key in {
        "PATH", "HOME", "DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_CONFIG",
        "DOCKER_TLS_VERIFY", "DOCKER_CERT_PATH", "XDG_RUNTIME_DIR",
    }}
    compose_env.update(POSTGRES_PASSWORD="fixture-only", JWT_SECRET="fixture-only",
                       RUNTIME_CONFIG_KEY="0" * 64, INTERNAL_SERVICE_TOKEN="fixture-only",
                       GRAFANA_ADMIN_PASSWORD="fixture-only", CACHE_MODE="postgres",
                       CONTAINER_PREFIX=project)

    def render(files, env):
        args = ["docker", "compose", "--env-file", "/dev/null", "-p", project]
        for name in files:
            args.extend(("-f", str(ROOT / name)))
        return run(*args, "config", "--format", "json", env=env, check=False)

    files = ["docker-compose.yml", "docker-compose.monitoring.yml"]
    baseline = render(files, compose_env)
    assert baseline.returncode == 0, baseline.stderr
    assert "alertmanager" not in json.loads(baseline.stdout)["services"]
    files.append("docker-compose.alerting.yml")
    absent = render(files, compose_env)
    assert absent.returncode != 0 and "ALERTMANAGER_CONFIG_FILE" in absent.stderr

    base = (MONITORING / "prometheus.yml").read_text()
    alerting = (MONITORING / "prometheus-alerting.yml").read_text()
    assert alerting.partition("\nalerting:\n")[0] == base, "Scrape config drift"
    template = (MONITORING / "alertmanager.example.yml").read_text()
    assert template.count(PLACEHOLDER) == 1
    with tempfile.TemporaryDirectory(prefix=project + "-") as temporary:
        folder = Path(temporary)
        config_file = folder / "alertmanager.yml"
        config_file.write_text(template.replace(PLACEHOLDER, "http://receiver:8080/notify")
                               .replace("group_wait: 30s", "group_wait: 1s")
                               .replace("group_wait: 10s", "group_wait: 1s")
                               .replace("group_interval: 5m", "group_interval: 10s")
                               .replace("timeout: 10s", "timeout: 200ms"))
        config_file.chmod(0o644)
        compose_env["ALERTMANAGER_CONFIG_FILE"] = str(config_file)
        rendered = render(files, compose_env)
        assert rendered.returncode == 0, rendered.stderr
        config = json.loads(rendered.stdout)
        services = config["services"]
        am, prom = services["alertmanager"], services["prometheus"]
        assert not am.get("ports") and am["cap_drop"] == ["ALL"]
        mount = next(v for v in am["volumes"] if v["target"].endswith(".yml"))
        assert mount["read_only"] and not mount.get("bind", {}).get("create_host_path", False)
        # Compose JSON can omit false fields. Keep the synthetic bind explicit.
        mount.setdefault("bind", {})["create_host_path"] = False
        assert mount["source"] == str(config_file)
        assert prom["depends_on"]["alertmanager"]["condition"] == "service_healthy"
        assert set(am["networks"]) == {"alerting-net"}
        assert set(prom["networks"]) == {"novel-net", "alerting-net"}
        py_image = json.loads(render(["docker-compose.yml", "docker-compose.e2e.yml"],
                                     compose_env).stdout)["services"]["llm-stub"]["image"]
        for image in (am["image"], prom["image"], py_image):
            assert "@sha256:" in image
            if run("docker", "image", "inspect", image, check=False).returncode:
                run("docker", "pull", image, timeout=60)
            run("docker", "image", "inspect", image)
        for name in ("prometheus.yml", "prometheus-alerting.yml"):
            run("docker", "run", "--rm", "--network", "none", "--entrypoint", "promtool",
                "-v", f"{MONITORING}:/etc/prometheus:ro", prom["image"],
                "check", "config", "/etc/prometheus/" + name)
        run("docker", "run", "--rm", "--network", "none", "--entrypoint", "amtool",
            "-v", f"{MONITORING / 'alertmanager.example.yml'}:/config.yml:ro",
            am["image"], "check-config", "/config.yml")

        for v in prom["volumes"]:
            original = Path(v["source"])
            target = folder / original.name
            target.write_text(original.read_text().replace("interval: 15s", "interval: 1s")
                              .replace("for: 30s", "for: 2s"))
            target.chmod(0o644)
            v["source"] = str(target)
        prom["command"] = ["--config.file=/etc/prometheus/prometheus.yml",
                           "--storage.tsdb.path=/prometheus", "--rules.alert.resend-delay=1s"]
        for name, service in (("alertmanager", am), ("prometheus", prom)):
            service["container_name"] = project + "-" + name
        fixture = {
            "image": py_image, "container_name": project + "-fixture", "user": "65534:65534",
            "command": ["python3", "-B", "/notification_drill.py", "--receiver"],
            "volumes": [str(Path(__file__).resolve()) + ":/notification_drill.py:ro"],
            "networks": {"novel-net": {"aliases": [
                "gateway", "user-service", "novel-service", "agent-service", "narrative-service",
            ]}}, "cap_drop": ["ALL"], "security_opt": ["no-new-privileges:true"],
            "read_only": True,
        }
        receiver_fixture = {**fixture, "container_name": project + "-receiver",
                            "networks": ["alerting-net"]}
        config = {"name": project, "services": {
            "alertmanager": am, "prometheus": prom, "fixture": fixture,
            "receiver": receiver_fixture,
        }, "networks": {name: {"name": project + "-" + name, "internal": True,
                              "ipam": {"config": [{"subnet": subnet}]}}
                        for name, subnet in zip(("novel-net", "alerting-net"), subnets)},
            "volumes": {"alertmanager_data": {"name": project + "-state"}}}
        stack = folder / "compose.json"

        def compose(*args, **kwargs):
            return run("docker", "compose", "--env-file", "/dev/null", "-p", project,
                       "-f", str(stack), *args, **kwargs)

        def request(url, data=None, service="fixture"):
            body = json.dumps({"url": url, "data": data,
                               "method": "GET" if data is None else "POST"})
            response = run("docker", "exec", "-i", project + "-" + service,
                           "python3", "-c", CLIENT, data=body, timeout=5).stdout
            return json.loads(response) if response.strip() else None

        def messages(alert, status):
            return [message for message in request("http://receiver:8080/state", service="receiver")["deliveries"]
                    if message["status"] == status and
                    message["groupLabels"].get("alertname") == alert]

        def submit(alert):
            run("docker", "exec", project + "-prometheus", "wget", "-qO-", "-T", "2",
                "--header=Content-Type: application/json", "--post-data=" + json.dumps([alert]),
                "http://alertmanager:9093/api/v2/alerts", timeout=5)

        signal.signal(signal.SIGTERM, lambda *_args: sys.exit("Notification drill terminated"))
        volume_names = set()
        try:
            missing = folder / "missing.yml"
            mount["source"] = str(missing)
            stack.write_text(json.dumps(config))
            rejected = compose("up", "-d", "alertmanager", check=False)
            assert rejected.returncode != 0 and not missing.exists()
            assert str(missing) in rejected.stderr and "does not exist" in rejected.stderr
            mount["source"] = str(config_file)
            stack.write_text(json.dumps(config))
            compose("up", "-d", "--wait", "--wait-timeout", "40", timeout=55)
            assert run("docker", "exec", project + "-alertmanager", "id", "-u").stdout.strip() == "65534"
            containers = compose("ps", "-aq").stdout.split()
            assert len(containers) == 4
            inventory = json.loads(run("docker", "inspect", *containers).stdout)
            for container in inventory:
                assert container["Config"]["Labels"]["com.docker.compose.project"] == project
                assert not container["HostConfig"]["PortBindings"]
                volume_names.update(v["Name"] for v in container["Mounts"] if v["Type"] == "volume")
            networks = json.loads(run("docker", "network", "inspect", *[
                project + "-" + name for name in ("novel-net", "alerting-net")
            ]).stdout)
            assert all(network["Internal"] for network in networks)
            am_container = next(c for c in inventory if c["Name"] == "/" + project + "-alertmanager")
            am_ip = am_container["NetworkSettings"]["Networks"][project + "-alerting-net"]["IPAddress"]
            run("docker", "exec", project + "-fixture", "python3", "-c",
                "import socket,sys\ntry:\n socket.create_connection((sys.argv[1],9093),timeout=1)\n"
                "except OSError:\n sys.exit(0)\nsys.exit('Application network can reach Alertmanager API')",
                am_ip, timeout=5)
            wait_for("five healthy scrape targets", lambda: len([
                target for target in request("http://prometheus:9090/api/v1/targets")["data"]["activeTargets"]
                if target["health"] == "up"
            ]) == 5)
            assert len(request("http://prometheus:9090/api/v1/alertmanagers")["data"]["activeAlertmanagers"]) == 1
            request("http://fixture:8080/down", {})
            wait_for("InstanceDown delivery after retries", lambda: messages("InstanceDown", "firing"))
            events = request("http://receiver:8080/state", service="receiver")
            assert len(events["attempts"]) >= 3
            assert events["attempts"][2]["at"] - events["attempts"][1]["at"] < 5
            now = datetime.now(timezone.utc)
            warning = {"labels": {"alertname": "NotificationDrillWarning", "severity": "warning", "job": "fixture"},
                       "startsAt": now.isoformat(), "endsAt": (now + timedelta(minutes=5)).isoformat()}
            submit(warning)
            wait_for("warning root route", lambda: messages("NotificationDrillWarning", "firing"))
            submit(warning)
            end = time.monotonic() + 12
            while time.monotonic() < end:
                assert len(messages("NotificationDrillWarning", "firing")) == 1
                time.sleep(0.5)
            compose("restart", "--timeout", "5", "alertmanager")
            wait_for("restarted Alertmanager readiness", lambda: json.loads(run(
                "docker", "inspect", "--format", "{{json .State.Health.Status}}",
                project + "-alertmanager").stdout) == "healthy")
            submit(warning)
            end = time.monotonic() + 12
            while time.monotonic() < end:
                assert len(messages("NotificationDrillWarning", "firing")) == 1
                time.sleep(0.5)
            request("http://fixture:8080/up", {})
            warning["endsAt"] = datetime.now(timezone.utc).isoformat()
            submit(warning)
            wait_for("critical recovery notification", lambda: messages("InstanceDown", "resolved"))
            wait_for("warning recovery notification", lambda: messages("NotificationDrillWarning", "resolved"))
            deliveries = request("http://receiver:8080/state", service="receiver")["deliveries"]
            for alert in ("InstanceDown", "NotificationDrillWarning"):
                firing, resolved = messages(alert, "firing"), messages(alert, "resolved")
                assert firing and resolved
                assert firing[0]["receiver"] == resolved[0]["receiver"] == "operator"
                assert firing[0]["groupKey"] == resolved[0]["groupKey"]
                assert firing[0]["alerts"][0]["fingerprint"] == resolved[0]["alerts"][0]["fingerprint"]
            assert all(message["truncatedAlerts"] == 0 for message in deliveries)
            print("notification drill: isolation, config, retry, timeout, restart dedup and resolution passed")
        finally:
            compose("down", "--volumes", "--timeout", "5", timeout=20)
            for kind in ("container", "volume", "network"):
                remaining = run("docker", kind, "ls", "-q", "--filter",
                                "label=com.docker.compose.project=" + project, timeout=2)
                assert not remaining.stdout.strip(), f"Owned {kind} cleanup incomplete"
            for name in volume_names:
                remaining = run("docker", "volume", "ls", "-q", "--filter", "name=" + name, timeout=2)
                assert name not in remaining.stdout.split(), f"Owned volume cleanup incomplete: {name}"
            print("notification drill: owned containers, networks and volumes removed")


if __name__ == "__main__":
    if sys.argv[1:] == ["--receiver"]:
        receiver()
    elif not sys.argv[1:]:
        drill()
    else:
        sys.exit("usage: notification_drill.py")
