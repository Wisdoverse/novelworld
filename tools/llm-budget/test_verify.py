#!/usr/bin/env python3
import json
import tempfile
import unittest
from pathlib import Path

from verify import BudgetError, verify, verify_many


HERE = Path(__file__).parent
COMMIT = "0" * 40


class BudgetVerifierTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.policy = (HERE / "policy-v2.json").read_text()
        cls.sample = (HERE / "recorded-release.prom").read_text()

    def run_verify(self, sample=None, policy=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            policy_path, sample_path = root / "policy.json", root / "metrics.prom"
            policy_path.write_text(policy or self.policy)
            sample_path.write_text(sample or self.sample)
            return verify(policy_path, sample_path, COMMIT)

    def test_recorded_sample_is_deterministic_and_passes(self):
        first = self.run_verify()
        second = self.run_verify()
        self.assertTrue(first["passed"])
        self.assertEqual(first, second)

    def test_separate_process_generations_are_aggregated_without_duplicate_series(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            policy_path = root / "policy.json"
            first = root / "first.prom"
            second = root / "second.prom"
            policy_path.write_text(self.policy)
            first.write_text(self.sample)
            second.write_text(self.sample)
            report = verify_many(policy_path, [first, second], COMMIT)
        self.assertTrue(report["passed"], report)
        self.assertEqual(
            report["operations"]["branch_generation"]["started"], 2
        )

    def test_embedding_transport_metrics_do_not_pollute_the_chat_budget_contract(self):
        sample = self.sample + (
            '\nnovelworld_embedding_requests_total{contract="llm-observability-v1",'
            'service="agent-service",provider="environment",model="embedding-model",'
            'status="success"} 1\n'
        )
        self.assertTrue(self.run_verify(sample=sample)["passed"])

    def test_budget_contract_fails_closed(self):
        branch = 'operation="branch_generation"'
        mutations = {
            "wrong contract": self.sample.replace(
                "llm-observability-v1", "llm-observability-v0"
            ),
            "missing operation": "\n".join(
                line
                for line in self.sample.splitlines()
                if branch not in line
                or "operation_output_token_ceiling" in line
            ),
            "token ceiling": self.sample.replace(
                f'novelworld_llm_output_token_limit{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",quantile="1"}} 4096',
                f'novelworld_llm_output_token_limit{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",quantile="1"}} 4097',
            ),
            "latency": self.sample.replace(
                f'novelworld_llm_request_duration_seconds{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",status="success",quantile="0.95"}} 0.2',
                f'novelworld_llm_request_duration_seconds{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",status="success",quantile="0.95"}} 31',
            ),
            "provider error": self.sample.replace(
                f'novelworld_llm_requests_total{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",status="success"}} 1',
                f'novelworld_llm_requests_total{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",status="error"}} 1',
            ),
            "missing usage": self.sample.replace(
                f'novelworld_llm_usage_reports_total{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",status="present"}} 1',
                f'novelworld_llm_usage_reports_total{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",status="missing"}} 1',
            ),
            "missing cost series": "\n".join(
                line for line in self.sample.splitlines()
                if branch not in line or 'class="output"' not in line
            ),
        }
        retry = self.sample.replace(
            f'novelworld_llm_attempts_total{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",status="success"}} 1',
            f'novelworld_llm_attempts_total{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",status="success"}} 2',
        )
        mutations["retry"] = retry + (
            '\nnovelworld_llm_retries_total{contract="llm-observability-v1",'
            f'service="narrative-service",provider="environment",model="e2e",{branch},'
            'mode="sync",reason="rate_limited"} 1\n'
        )

        for name, sample in mutations.items():
            with self.subTest(name=name):
                report = self.run_verify(sample=sample)
                self.assertFalse(report["passed"], report)

    def test_started_operation_with_expired_summary_fails_closed(self):
        branch = 'operation="branch_generation"'
        sample = self.sample.replace(
            f'novelworld_llm_output_token_limit{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",quantile="1"}} 4096',
            f'novelworld_llm_output_token_limit{{contract="llm-observability-v1",service="narrative-service",provider="environment",model="e2e",{branch},mode="sync",quantile="1"}} 0',
        )
        report = self.run_verify(sample=sample)
        self.assertFalse(report["passed"], report)
        self.assertIn(
            "branch_generation: missing or expired output-token-limit samples",
            report["failures"],
        )

    def test_malformed_or_sensitive_samples_are_rejected(self):
        for sample in [
            self.sample.replace("} 80\n", '} NaN\n'),
            self.sample.replace(
                'service="user-service"} 1',
                'service="user-service",user_id="secret"} 1',
                1,
            ),
            self.sample.replace('class="output"', 'class="unbounded-new-class"', 1),
            self.sample.replace(
                'usage_key="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"',
                'usage_key="not-a-fingerprint"',
                1,
            ),
        ]:
            with self.assertRaises(BudgetError):
                self.run_verify(sample=sample)

    def test_unknown_policy_version_and_commit_are_rejected(self):
        policy = json.loads(self.policy)
        policy["schema_version"] = 2
        with self.assertRaises(BudgetError):
            self.run_verify(policy=json.dumps(policy))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            policy_path, sample_path = root / "policy.json", root / "metrics.prom"
            policy_path.write_text(self.policy)
            sample_path.write_text(self.sample)
            with self.assertRaises(BudgetError):
                verify(policy_path, sample_path, "main")


if __name__ == "__main__":
    unittest.main()
