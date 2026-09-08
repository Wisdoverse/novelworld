"""Fixed qualification-only IPAM overlay and one-network continuity guard."""
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys


class NetworkFailure(RuntimeError):
    def __init__(self, code):
        super().__init__(code)
        self.code = code


def require(condition, code="qualification_network_invalid"):
    if not condition:
        raise NetworkFailure(code)


def subnet(value):
    if value is None:
        return None
    require(isinstance(value, str))
    try:
        selected = ipaddress.ip_network(value, strict=True)
    except ValueError as error:
        raise NetworkFailure("qualification_subnet_invalid") from error
    require(selected.version == 4 and selected.prefixlen == 28 and str(selected) == value
            and any(selected.subnet_of(ipaddress.ip_network(block))
                    for block in ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16")),
            "qualification_subnet_invalid")
    return value


def overlay_bytes(value):
    require(subnet(value) is not None)
    return ("networks:\n  novel-net:\n    ipam:\n      config:\n        - subnet: " + value + "\n").encode()


def read_command(argv):
    try:
        result = subprocess.run(argv, check=True, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=5)
        require(len(result.stdout) <= 1048576, "qualification_topology_too_large")
        return result.stdout
    except (OSError, subprocess.SubprocessError) as error:
        raise NetworkFailure("qualification_topology_unproven") from error


def decode(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result)
            result[key] = value
        return result
    try:
        return json.loads(raw, object_pairs_hook=pairs,
                          parse_constant=lambda _: require(False))
    except (ValueError, TypeError) as error:
        raise NetworkFailure("qualification_topology_invalid") from error


def inventory():
    # Local route evidence is meaningful only for a local Linux Docker engine.
    context = decode(read_command(["docker", "context", "inspect"]))
    require(isinstance(context, list) and len(context) == 1 and isinstance(context[0], dict)
            and context[0].get("Name") == "default", "qualification_local_engine_required")
    host = context[0].get("Endpoints", {}).get("docker", {}).get("Host")
    if os.environ.get("DOCKER_HOST") and not os.environ.get("DOCKER_CONTEXT"):
        host = os.environ["DOCKER_HOST"]
    require(host in ("unix:///var/run/docker.sock", "unix:///run/docker.sock"),
            "qualification_local_engine_required")
    engine = decode(read_command(["docker", "info", "--format",
        '{"os":{{json .OSType}},"name":{{json .Name}},"kernel":{{json .KernelVersion}},'
        '"distribution":{{json .OperatingSystem}},"security":{{json .SecurityOptions}}}']))
    host_identity = os.uname()
    require(isinstance(engine, dict) and engine.get("os") == "linux"
            and engine.get("name") == host_identity.nodename and engine.get("kernel") == host_identity.release
            and isinstance(engine.get("distribution"), str) and "desktop" not in engine["distribution"].lower()
            and isinstance(engine.get("security"), list)
            and all(isinstance(item, str) and "rootless" not in item.lower() for item in engine["security"]),
            "qualification_local_engine_required")
    ids = read_command(["docker", "network", "ls", "--quiet", "--no-trunc"]).decode().split()
    require(len(ids) == len(set(ids)) and len(ids) <= 1024
            and all(re.fullmatch(r"(?:[0-9a-f]{64}|[a-z0-9]{25})", item) for item in ids))
    networks = decode(read_command(["docker", "network", "inspect", *ids])) if ids else []
    require(isinstance(networks, list) and len(networks) == len(ids)
            and all(isinstance(item, dict) for item in networks)
            and {item.get("Id") for item in networks} == set(ids))
    routes = decode(read_command(["ip", "-j", "-4", "route", "show", "table", "all"]))
    require(isinstance(routes, list) and all(isinstance(item, dict) for item in routes))
    # A local Unix transport alone also admits Desktop/VM proxies. Bind the
    # daemon's existing default bridge to this host's route evidence. This is a
    # narrow operator-environment check, not attestation against malicious root.
    bridges = [item for item in networks if item.get("Name") == "bridge"
               and item.get("Driver") == "bridge" and item.get("Scope") == "local"]
    require(len(bridges) == 1, "qualification_local_topology_unproven")
    bridge = bridges[0]
    device = (bridge.get("Options") or {}).get("com.docker.network.bridge.name")
    config = (bridge.get("IPAM") or {}).get("Config")
    require(isinstance(device, str) and bool(device) and isinstance(config, list)
            and len(config) == 1 and isinstance(config[0], dict), "qualification_local_topology_unproven")
    require(any(route.get("dev") == device and route.get("dst") == config[0].get("Subnet")
                and route.get("prefsrc") == config[0].get("Gateway")
                and isinstance(route.get("prefsrc"), str) for route in routes),
            "qualification_local_topology_unproven")
    return networks, routes


def owned_network(networks, project, value):
    name = project + "_novel-net"
    matches = [item for item in networks if item.get("Name") == name
               or (item.get("Labels") or {}).get("com.docker.compose.project") == project]
    require(len(matches) == 1, "qualification_network_missing_or_ambiguous")
    item = matches[0]
    require(re.fullmatch(r"[0-9a-f]{64}", item["Id"]) is not None
            and item.get("Name") == name and item.get("Driver") == "bridge"
            and item.get("Scope") == "local" and item.get("Internal") is False
            and item.get("EnableIPv6") is False
            and (item.get("Labels") or {}).get("com.docker.compose.project") == project
            and (item.get("Labels") or {}).get("com.docker.compose.network") == "novel-net"
            and not (item.get("Options") or {}).get("com.docker.network.bridge.name"))
    config = item.get("IPAM", {}).get("Config")
    require(isinstance(config, list) and len(config) == 1 and config[0].get("Subnet") == value)
    return {"id": item["Id"], "name": name, "subnet": value,
            "overlay_sha256": hashlib.sha256(overlay_bytes(value)).hexdigest()}


def topology(value, networks, routes, owned=None):
    selected = ipaddress.ip_network(subnet(value))
    try:
        for item in networks:
            if owned and item["Id"] == owned["id"]:
                continue
            ipam = item.get("IPAM")
            require(isinstance(ipam, dict) and "Config" in ipam)
            config = ipam["Config"]
            require(config is None or isinstance(config, list))
            for allocation in config or []:
                require(isinstance(allocation, dict) and isinstance(allocation.get("Subnet"), str))
                other = ipaddress.ip_network(allocation["Subnet"], strict=False)
                require(other.version != 4 or not selected.overlaps(other), "qualification_subnet_overlap")
        for route in routes:
            destination = route.get("dst")
            require(isinstance(destination, str))
            if destination == "default":
                continue
            other = ipaddress.ip_network(destination, strict=False)
            require(other.version == 4)
            if owned and route.get("dev") == "br-" + owned["id"][:12] and other.subnet_of(selected):
                continue
            require(not selected.overlaps(other), "qualification_subnet_overlap")
    except (ValueError, TypeError, KeyError) as error:
        raise NetworkFailure("qualification_topology_invalid") from error


def preflight(value):
    if subnet(value) is not None:
        topology(value, *inventory())


def private_file(path, expected=None):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid()
            and stat.S_IMODE(info.st_mode) == 0o600, "qualification_network_file_unsafe")
    data = path.read_bytes()
    require(len(data) <= 4096 and (expected is None or data == expected),
            "qualification_network_file_changed")
    return data


def write_once(path, data):
    if os.path.lexists(path):
        private_file(path, data)
        return
    descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(parent)
    finally:
        os.close(parent)


def state_paths(state, project, root):
    require(re.fullmatch(r"nwq-[a-f0-9]{10}", project) is not None)
    require(state.is_absolute() and state == state.resolve() and state != root.resolve()
            and root.resolve() not in state.parents, "qualification_network_state_must_be_external")
    if state.exists():
        info = state.stat()
        require(state.is_dir() and info.st_uid == os.getuid() and stat.S_IMODE(info.st_mode) == 0o700)
    return (state / "qualification-network.yml", state / "qualification-network.json",
            state / "qualification-network-attempt.json")


def guard(action, value, state, project, root):
    subnet(value)
    require(value is not None)
    overlay, receipt, attempt = state_paths(state, project, root)
    encoded = overlay_bytes(value)
    if action == "overlay":
        require(state.is_dir())
        write_once(overlay, encoded)
        return str(overlay)
    require(action in ("check", "before", "after"))
    if os.path.lexists(overlay):
        private_file(overlay, encoded)
    attempt_bytes = json.dumps({"project": project, "subnet": value,
        "overlay_sha256": hashlib.sha256(encoded).hexdigest()}, sort_keys=True).encode()
    if os.path.lexists(attempt):
        private_file(attempt, attempt_bytes)
    networks, routes = inventory()
    if os.path.lexists(receipt):
        require(os.path.lexists(attempt), "qualification_network_attempt_missing")
        saved = decode(private_file(receipt))
        observed = owned_network(networks, project, value)
        require(saved == observed, "qualification_network_identity_changed")
        topology(value, networks, routes, observed)
    elif action == "after":
        require(os.path.lexists(attempt), "qualification_network_attempt_missing")
        observed = owned_network(networks, project, value)
        topology(value, networks, routes, observed)
        write_once(receipt, json.dumps(observed, sort_keys=True, separators=(",", ":")).encode())
    else:
        require(not os.path.lexists(attempt), "qualification_network_creation_unresolved")
        require(not any(item.get("Name") == project + "_novel-net"
                        or (item.get("Labels") or {}).get("com.docker.compose.project") == project
                        for item in networks), "qualification_project_not_empty")
        topology(value, networks, routes)
        if action == "before":
            write_once(attempt, attempt_bytes)
    return None


if __name__ == "__main__":
    try:
        require(len(sys.argv) == 6)
        result = guard(sys.argv[1], sys.argv[2], Path(sys.argv[3]), sys.argv[4], Path(sys.argv[5]))
        if result is not None:
            print(result)
    except (NetworkFailure, OSError, ValueError, TypeError, KeyError) as error:
        print("release: qualification network guard failed", file=sys.stderr)
        sys.exit(1)
