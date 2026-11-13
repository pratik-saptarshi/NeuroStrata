#!/usr/bin/env python3
"""Emit the failing-test list of a CI test job as a stable JSON artifact.

CI telemetry (remediation programme, judge item 10): every test job tees
its cargo output to ``$RUNNER_TEMP/test-output.log``; when the job fails,
a follow-up step runs this tool over that log and uploads the result as
a workflow artifact. The point is that a later failure census reads
artifacts in this one format instead of digging through raw job logs
again.

The output schema matches the Phase 0 failing-test inventory exactly:
``repo``, ``run_id``, ``branch``, ``job_name``, ``failing_tests``,
``log_source`` (always ``"artifact"`` for records produced here). The
identity fields come from the GitHub Actions default environment
(``GITHUB_REPOSITORY``, ``GITHUB_RUN_ID``, ``GITHUB_REF_NAME``); the job
display name is passed explicitly because Actions exposes no context
for it.

Extraction is deliberately narrow: only lines of the shape
``test <name> ... FAILED`` from libtest output count. Compile errors and
mid-run crashes produce an empty ``failing_tests`` list, which is the
truthful record for those shapes -- the inventory records them the same
way.

This tool is a telemetry side-channel. It must never fail the job that
invokes it: any error is reported on stderr and exits 0, leaving an
artifact behind only when the parse succeeded. The workflow step that
calls it is also marked continue-on-error.

Usage: ``python3 tools/failing_tests_artifact.py --log PATH --out PATH
--job-name NAME``. Stdlib only; Python 3.11+.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

#: libtest result line: "test some::path::name ... FAILED". The name is
#: a single token; harness output with a trailing "(...)" suffix or a
#: "[ignore]"-style annotation does not occur in this suite's runs.
FAILED_RE = re.compile(r"^test (\S+) \.\.\. FAILED$")


def extract_failing_tests(log_text: str) -> list[str]:
    """Return the failing test names, in first-seen order, deduplicated."""
    seen: set[str] = set()
    names: list[str] = []
    for line in log_text.splitlines():
        # CI logs may prefix the cargo output (gh log downloads do);
        # search rather than match so both shapes parse identically.
        m = FAILED_RE.search(line.strip()) or re.search(
            r"test (\S+) \.\.\. FAILED", line
        )
        if m and m.group(1) not in seen:
            seen.add(m.group(1))
            names.append(m.group(1))
    return names


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True, help="tee'd cargo test output")
    parser.add_argument("--out", required=True, help="where to write the JSON")
    parser.add_argument("--job-name", required=True, help="job display name")
    args = parser.parse_args(argv)

    try:
        log_text = Path(args.log).read_text(errors="replace")
    except OSError as exc:
        print(f"failing_tests_artifact: cannot read log: {exc}", file=sys.stderr)
        return 0

    try:
        run_id: int | str = int(os.environ.get("GITHUB_RUN_ID", ""))
    except ValueError:
        run_id = os.environ.get("GITHUB_RUN_ID", "")

    record = {
        "repo": os.environ.get("GITHUB_REPOSITORY", ""),
        "run_id": run_id,
        "branch": os.environ.get("GITHUB_REF_NAME", ""),
        "job_name": args.job_name,
        "failing_tests": extract_failing_tests(log_text),
        "log_source": "artifact",
    }
    try:
        Path(args.out).write_text(json.dumps(record, indent=2) + "\n")
    except OSError as exc:
        print(f"failing_tests_artifact: cannot write output: {exc}", file=sys.stderr)
        return 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
