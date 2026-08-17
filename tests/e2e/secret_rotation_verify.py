#!/usr/bin/env python3
"""Verifier for the secret-rotation drill: one copy shared by the drill's
runtime verification and its --self-test mutation checks."""
import json
import sys


def verify(record):
    problems = []

    def require(name, condition):
        if not condition:
            problems.append(name)

    require("old access token rejected", record["old_access_status"] == 401)
    require("old refresh token rejected", record["old_refresh_status"] == 401)
    require("old internal token rejected", record["old_internal_status"] == 401)
    require("new login works", record["new_login_status"] == 200)
    require("account export crosses internal calls", record["export_status"] == 200)
    return problems


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        base = {
            "old_access_status": 401,
            "old_refresh_status": 401,
            "old_internal_status": 401,
            "new_login_status": 200,
            "export_status": 200,
        }
        tampered = [
            ("old access token must fail", dict(base, old_access_status=200)),
            ("old refresh token must fail", dict(base, old_refresh_status=200)),
            ("old internal token must fail", dict(base, old_internal_status=200)),
            ("new login must succeed", dict(base, new_login_status=401)),
            ("export must succeed", dict(base, export_status=500)),
        ]
        for label, record in tampered:
            if not verify(record):
                print(f"self-test failed: {label} passed the weakened verifier")
                sys.exit(1)
        print("secret rotation verifier self-test passed")
        return
    if len(sys.argv) != 6:
        print("usage: secret_rotation_verify.py (--self-test | old_access old_refresh old_internal new_login export)")
        sys.exit(2)
    record = {
        "old_access_status": int(sys.argv[1]),
        "old_refresh_status": int(sys.argv[2]),
        "old_internal_status": int(sys.argv[3]),
        "new_login_status": int(sys.argv[4]),
        "export_status": int(sys.argv[5]),
    }
    problems = verify(record)
    if problems:
        print("secret rotation failed: " + ", ".join(problems))
        sys.exit(1)
    print("secret rotation verified")


if __name__ == "__main__":
    main()
