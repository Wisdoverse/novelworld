#!/usr/bin/env python3
"""Verifier for the S3 source-boundary drill.

One copy shared by the drill's runtime verification and its --self-test
mutation checks, so weakening one path weakens the other and the recorded
claims cannot drift apart.
"""
import json
import sys


def verify(record):
    problems = []

    def require(name, condition):
        if not condition:
            problems.append(name)

    require("job completed", record["job_status"] == "completed")
    require("no failure code", record["failure_code"] is None)
    require("novel ready", record["novel_status"] == "ready")
    require("attempts inside import-provider-budget-v1",
            1 <= record["attempt"] <= 3)
    require("chapters exactly committed total",
            record["chapter_count"] == record["total_chapters"] == 2)
    require("chapters rebuilt from the retained object",
            record["chapter_md5s"] == record["expected_md5s"])
    require("exactly one character", record["character_count"] == 1)
    require("exactly one canon model", record["canon_count"] == 1)
    require("zero chapters at the source-boundary kill",
            record["chapters_at_kill"] == 0)
    require("no provider calls for completed work",
            record["calls_after"] == record["calls_before"] == 0)
    require("retry on ready rejected", record["retry_status"] == 409)
    return problems


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        base = {
            "phase": "source",
            "job_status": "completed",
            "failure_code": None,
            "novel_status": "ready",
            "chapter_count": 2,
            "total_chapters": 2,
            "expected_md5s": "aa,bb",
            "chapter_md5s": "aa,bb",
            "character_count": 1,
            "canon_count": 1,
            "chapters_at_kill": 0,
            "calls_before": 0,
            "calls_after": 0,
            "retry_status": 409,
            "attempt": 2,
        }
        tampered = [
            ("duplicated chapters must fail", dict(base, chapter_count=3)),
            ("false ready must fail", dict(base, novel_status="parsing")),
            ("chapters not from the retained object must fail",
             dict(base, chapter_md5s="cc,dd")),
            ("chapters committed at kill must fail",
             dict(base, chapters_at_kill=2)),
            ("duplicate canon must fail", dict(base, canon_count=2)),
            ("provider call for completed work must fail",
             dict(base, calls_after=1)),
            ("ready import retry must fail", dict(base, retry_status=200)),
            ("attempt beyond the budget ceiling must fail", dict(base, attempt=4)),
        ]
        for label, record in tampered:
            if not verify(record):
                print(f"self-test failed: {label} passed the weakened verifier")
                sys.exit(1)
        print("source recovery verifier self-test passed")
        return
    if len(sys.argv) != 2:
        print("usage: source_recovery_verify.py (--self-test | <record-json>)")
        sys.exit(2)
    record = json.loads(sys.argv[1])
    problems = verify(record)
    if problems:
        print("source recovery failed: " + ", ".join(problems))
        sys.exit(1)
    print("source recovery phase verified: " + record["phase"])


if __name__ == "__main__":
    main()
