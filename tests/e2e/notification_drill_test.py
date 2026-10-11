#!/usr/bin/env python3
"""Refuse occupied fixture networks before creating Docker resources."""

import importlib.util
import ipaddress
from pathlib import Path
import unittest

SOURCE = Path(__file__).resolve().parents[2] / "infra/monitoring/notification_drill.py"
spec = importlib.util.spec_from_file_location("notification_drill", SOURCE)
assert spec and spec.loader
drill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drill)


class SubnetTest(unittest.TestCase):
    def choose(self, blocks, routes):
        networks = [{"IPAM": {"Config": [
            {"Subnet": block} for block in blocks
        ]}}, {"IPAM": {"Config": None}}]
        return [ipaddress.ip_network(value) for value in drill.choose_subnets(networks, routes)]

    def test_docker_subnet_and_host_address_are_excluded(self):
        selected = self.choose(["10.254.240.0/28", "fd00::/64"], [
            {"dst": "default"}, {"dst": "10.254.240.17", "type": "local"},
        ])
        self.assertEqual(len(selected), 2)
        self.assertFalse(selected[0].overlaps(selected[1]))
        for network in selected:
            self.assertEqual(network.prefixlen, 28)
            self.assertFalse(network.overlaps(ipaddress.ip_network("10.254.240.0/28")))
            self.assertNotIn(ipaddress.ip_address("10.254.240.17"), network)

    def test_only_one_available_subnet_refuses_partial_allocation(self):
        with self.assertRaisesRegex(RuntimeError, "No unused private"):
            self.choose([str(block) for block in ipaddress.ip_network("10.254.240.0/20")
                         .subnets(new_prefix=28)][1:], [])

    def test_full_host_route_refuses_allocation(self):
        with self.assertRaisesRegex(RuntimeError, "No unused private"):
            self.choose([], [{"dst": "10.0.0.0/8"}])


if __name__ == "__main__":
    unittest.main()
