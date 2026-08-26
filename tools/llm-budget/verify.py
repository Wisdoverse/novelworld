#!/usr/bin/env python3
import argparse
import hashlib
import json
import math
import re
import sys
from pathlib import Path


LINE = re.compile(r"^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{.*\})?\s+([^\s]+)$")
LABEL = re.compile(r'(?:^|,)([a-zA-Z_][a-zA-Z0-9_]*)="((?:\\.|[^"\\])*)"')
COMMIT = re.compile(r"^[0-9a-f]{40}$")
BOUNDED_LABEL = re.compile(r"^[A-Za-z0-9._/:-]{1,200}$")
USAGE_KEY = re.compile(r"^[0-9a-f]{64}$")
FORBIDDEN_LABELS = {
    "api_key", "character_id", "error", "message", "novel_id", "prompt",
    "trace_id", "url", "user_id",
}
BASE = {"contract", "service"}
COUNTER_LABELS = {
    "novelworld_llm_requests_started_total": {"provider", "model", "operation", "mode"},
    "novelworld_llm_attempts_total": {"provider", "model", "operation", "mode", "status"},
    "novelworld_llm_retries_total": {"provider", "model", "operation", "mode", "reason"},
    "novelworld_llm_requests_total": {"provider", "model", "operation", "mode", "status"},
    "novelworld_llm_usage_reports_total": {"provider", "model", "operation", "mode", "status"},
    "novelworld_llm_tokens_total": {"provider", "model", "operation", "type"},
    "novelworld_llm_billable_tokens_total": {
        "provider", "model", "operation", "class", "usage_key",
    },
}
HISTOGRAM_LABELS = {
    "novelworld_llm_attempt_duration_seconds": {"provider", "model", "operation", "mode", "status"},
    "novelworld_llm_stream_setup_duration_seconds": {"provider", "model", "operation", "status"},
    "novelworld_llm_request_duration_seconds": {"provider", "model", "operation", "mode", "status"},
    "novelworld_llm_first_token_duration_seconds": {"provider", "model", "operation"},
    "novelworld_llm_output_token_limit": {"provider", "model", "operation", "mode"},
    "novelworld_llm_tokens_per_request": {"provider", "model", "operation", "type"},
}
ENUM_LABELS = {
    "mode": {"sync", "stream"},
    "status": {
        "client_or_transport_error", "consumer_dropped", "empty_json_mode",
        "error", "missing", "present", "provider_error", "rate_limited",
        "rejected", "setup_error", "stream_error", "success",
    },
    "reason": {
        "client_or_transport_error", "json_mode_fallback", "provider_error",
        "rate_limited", "rejected",
    },
    "type": {"cached_input", "input", "output"},
    "class": {"cached_input", "uncached_input", "output"},
    "quantile": {"0", "0.5", "0.9", "0.95", "0.99", "0.999", "1"},
}


class BudgetError(ValueError):
    pass


def digest(data):
    return hashlib.sha256(data).hexdigest()


def labels(raw):
    if not raw:
        return {}
    inner = raw[1:-1]
    result, position = {}, 0
    for match in LABEL.finditer(inner):
        if match.start() != position:
            raise BudgetError("malformed Prometheus labels")
        key = match.group(1)
        if key in result:
            raise BudgetError(f"duplicate label {key}")
        result[key] = json.loads(f'"{match.group(2)}"')
        position = match.end()
    if position != len(inner):
        raise BudgetError("malformed Prometheus labels")
    return result


def parse_metrics(data):
    samples, seen = [], set()
    for number, raw in enumerate(data.decode("utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        match = LINE.fullmatch(line)
        if not match:
            raise BudgetError(f"malformed Prometheus sample on line {number}")
        name, tags = match.group(1), labels(match.group(2))
        try:
            value = float(match.group(3))
        except ValueError as error:
            raise BudgetError(f"invalid numeric sample on line {number}") from error
        if not math.isfinite(value) or value < 0:
            raise BudgetError(f"non-finite or negative sample on line {number}")
        key = name, tuple(sorted(tags.items()))
        if key in seen:
            raise BudgetError(f"duplicate Prometheus series on line {number}")
        seen.add(key)
        if name.startswith("novelworld_llm_"):
            leaked = FORBIDDEN_LABELS.intersection(tags)
            if leaked:
                raise BudgetError(f"forbidden LLM metric labels: {sorted(leaked)}")
            if "service" not in tags or "contract" not in tags:
                raise BudgetError(f"LLM metric lacks service/contract on line {number}")
            if name == "novelworld_llm_observability_info":
                allowed = BASE
            elif name == "novelworld_llm_operation_output_token_ceiling":
                allowed = BASE | {"operation"}
            elif name in COUNTER_LABELS:
                allowed = BASE | COUNTER_LABELS[name]
            else:
                base = name.removesuffix("_sum").removesuffix("_count")
                if base not in HISTOGRAM_LABELS:
                    raise BudgetError(f"unknown LLM metric {name}")
                allowed = BASE | HISTOGRAM_LABELS[base]
                if name == base:
                    allowed |= {"quantile"}
            if set(tags) != allowed:
                raise BudgetError(f"unexpected labels on {name}: {sorted(set(tags) - allowed)}")
            for key, allowed_values in ENUM_LABELS.items():
                if key in tags and tags[key] not in allowed_values:
                    raise BudgetError(f"unknown {key} label value on line {number}")
            for key in ("provider", "model"):
                label_value = tags.get(key)
                if label_value is not None and not BOUNDED_LABEL.fullmatch(label_value):
                    raise BudgetError(f"invalid {key} label on line {number}")
            usage_key = tags.get("usage_key")
            if usage_key is not None and not USAGE_KEY.fullmatch(usage_key):
                raise BudgetError(f"invalid usage_key label on line {number}")
        samples.append((name, tags, value))
    return samples


def matching(samples, name, **wanted):
    return [
        (tags, value) for metric, tags, value in samples
        if metric == name and all(tags.get(key) == value for key, value in wanted.items())
    ]


def total(samples, name, **wanted):
    return sum(value for _, value in matching(samples, name, **wanted))


def maximum(samples, name, **wanted):
    values = [value for _, value in matching(samples, name, **wanted)]
    if not values:
        raise BudgetError(f"missing {name} for {wanted}")
    return max(values)


def verify(policy_path, sample_path, commit):
    if not COMMIT.fullmatch(commit):
        raise BudgetError("commit must be a lowercase 40-character SHA")
    policy_data, sample_data = policy_path.read_bytes(), sample_path.read_bytes()
    policy = json.loads(policy_data)
    if policy.get("schema_version") != 1 or not policy.get("policy_version"):
        raise BudgetError("unsupported budget policy")
    operations = policy.get("operations")
    if not isinstance(operations, dict) or not operations:
        raise BudgetError("budget policy has no operations")
    samples = parse_metrics(sample_data)
    contract = policy["metrics_contract"]
    failures, results = [], {}

    info = matching(samples, "novelworld_llm_observability_info")
    observed_services = {
        tags.get("service") for tags, value in info
        if tags.get("contract") == contract and value == 1
    }
    missing_services = set(policy["required_services"]) - observed_services
    if missing_services:
        failures.append(f"missing services: {sorted(missing_services)}")
    for name, tags, _ in samples:
        if name.startswith("novelworld_llm_") and tags.get("contract") != contract:
            failures.append(f"wrong metrics contract on {name}")
        if name.startswith("novelworld_llm_") and tags.get("service") not in policy["required_services"]:
            failures.append(f"unknown service on {name}")
        if tags.get("operation") not in (None, *operations):
            failures.append(f"unknown operation on {name}")

    for operation, budget in operations.items():
        ceilings = matching(
            samples,
            "novelworld_llm_operation_output_token_ceiling",
            operation=operation,
        )
        expected_ceiling = budget["output_token_ceiling"]
        if not ceilings or any(value != expected_ceiling for _, value in ceilings):
            failures.append(f"{operation}: missing or mismatched static token ceiling")

        started = total(
            samples, "novelworld_llm_requests_started_total", operation=operation
        )
        terminal = matching(samples, "novelworld_llm_requests_total", operation=operation)
        successes = sum(value for tags, value in terminal if tags.get("status") == "success")
        errors = sum(value for tags, value in terminal if tags.get("status") != "success")
        attempts = total(samples, "novelworld_llm_attempts_total", operation=operation)
        retries = total(samples, "novelworld_llm_retries_total", operation=operation)
        present = total(
            samples,
            "novelworld_llm_usage_reports_total",
            operation=operation,
            status="present",
        )
        missing = total(
            samples,
            "novelworld_llm_usage_reports_total",
            operation=operation,
            status="missing",
        )
        result = {
            "started": started,
            "successes": successes,
            "errors": errors,
            "attempts": attempts,
            "retries": retries,
            "usage_present": present,
            "usage_missing": missing,
        }
        results[operation] = result

        if started < budget["min_samples"]:
            failures.append(f"{operation}: insufficient samples")
        if started == 0:
            continue
        if successes + errors != started:
            failures.append(f"{operation}: started/terminal count mismatch")
        if attempts != started + retries:
            failures.append(f"{operation}: attempt/retry count mismatch")
        if errors / started > policy["max_error_rate"]:
            failures.append(f"{operation}: error budget exceeded")
        if retries / started > policy["max_retry_rate"]:
            failures.append(f"{operation}: retry budget exceeded")
        if present + missing != successes:
            failures.append(f"{operation}: success/usage count mismatch")
        if successes and missing / successes > policy["max_missing_usage_rate"]:
            failures.append(f"{operation}: missing-usage budget exceeded")

        p95 = maximum(
            samples,
            "novelworld_llm_request_duration_seconds",
            operation=operation,
            quantile="0.95",
        )
        limit = maximum(
            samples,
            "novelworld_llm_output_token_limit",
            operation=operation,
            quantile="1",
        )
        output = maximum(
            samples,
            "novelworld_llm_tokens_per_request",
            operation=operation,
            type="output",
            quantile="1",
        )
        input_tokens = total(
            samples, "novelworld_llm_tokens_total", operation=operation, type="input"
        )
        output_tokens = total(
            samples, "novelworld_llm_tokens_total", operation=operation, type="output"
        )
        cached_tokens = total(
            samples, "novelworld_llm_tokens_total",
            operation=operation, type="cached_input",
        )
        cached_input = total(
            samples, "novelworld_llm_billable_tokens_total",
            operation=operation, **{"class": "cached_input"},
        )
        uncached_input = total(
            samples, "novelworld_llm_billable_tokens_total",
            operation=operation, **{"class": "uncached_input"},
        )
        billable_output = total(
            samples, "novelworld_llm_billable_tokens_total",
            operation=operation, **{"class": "output"},
        )
        required_token_series = (
            matching(samples, "novelworld_llm_tokens_total", operation=operation, type="input")
            and matching(samples, "novelworld_llm_tokens_total", operation=operation, type="output")
            and matching(samples, "novelworld_llm_billable_tokens_total", operation=operation, **{"class": "uncached_input"})
            and matching(samples, "novelworld_llm_billable_tokens_total", operation=operation, **{"class": "output"})
        )
        if not required_token_series:
            failures.append(f"{operation}: missing token accounting series")
        if (
            input_tokens != cached_input + uncached_input
            or output_tokens != billable_output
            or cached_tokens != cached_input
        ):
            failures.append(f"{operation}: token accounting mismatch")
        billable = cached_input + uncached_input + billable_output
        average_billable = billable / present if present else math.inf
        result.update({
            "p95_seconds": p95,
            "max_output_token_limit": limit,
            "max_observed_output_tokens": output,
            "average_billable_tokens": average_billable,
        })
        if limit <= 0:
            failures.append(f"{operation}: missing or expired output-token-limit samples")
        if p95 > budget["max_p95_seconds"]:
            failures.append(f"{operation}: latency budget exceeded")
        if limit > expected_ceiling or output > expected_ceiling:
            failures.append(f"{operation}: output-token ceiling exceeded")
        if average_billable > budget["max_average_billable_tokens"]:
            failures.append(f"{operation}: billable-token budget exceeded")
        if "max_first_token_p95_seconds" in budget:
            first = maximum(
                samples,
                "novelworld_llm_first_token_duration_seconds",
                operation=operation,
                quantile="0.95",
            )
            result["first_token_p95_seconds"] = first
            if first > budget["max_first_token_p95_seconds"]:
                failures.append(f"{operation}: first-token budget exceeded")

    return {
        "schema_version": 1,
        "policy_version": policy["policy_version"],
        "metrics_contract": contract,
        "commit": commit,
        "policy_sha256": digest(policy_data),
        "sample_sha256": digest(sample_data),
        "operations": results,
        "failures": sorted(set(failures)),
        "passed": not failures,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--policy", type=Path, required=True)
    parser.add_argument("--metrics", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    try:
        report = verify(args.policy, args.metrics, args.commit)
    except (BudgetError, json.JSONDecodeError, UnicodeDecodeError) as error:
        report = {"schema_version": 1, "passed": False, "error": str(error)}
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
