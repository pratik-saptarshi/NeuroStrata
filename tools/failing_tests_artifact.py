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
(``GITHUB_REPOSITORY``, ``GITHUB_RUN_ID``, and ``GITHUB_HEAD_REF`` /
``GITHUB_REF_NAME`` for the branch); the job display name is passed
explicitly because Actions exposes no context for it.

Extraction is deliberately narrow: only lines of the shape
``test <name> ... FAILED`` from libtest output count, anchored to the
whole line (after stripping a ``gh run view --log`` prefix, for local
use against downloaded logs). Doctest result lines, whose names carry
spaces (``test src/lib.rs - foo (line 3) ... FAILED``), count too.
Anything merely containing the shape -- a ``FAILEDNESS`` line, an
echoed example in some other output -- does not. Compile errors and
mid-run crashes produce an empty ``failing_tests`` list, which is the
truthful record for those shapes -- the inventory records them the
same way.

This tool is a telemetry side-channel. It must never fail the job that
invokes it: runtime errors (unreadable log, unwritable output) are
reported on stderr and exit 0, leaving an artifact behind only when
the parse succeeded, and the workflow step that calls it is also
marked continue-on-error. Argument errors are workflow programming
errors and keep argparse's exit 2, so a typo'd invocation is visible
in the step log instead of silently producing an empty artifact.

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

#: libtest result line: "test some::path::name ... FAILED". Unit-test
#: names are single tokens; doctest names contain spaces and a
#: "(line N)" suffix, so the name group is ".+" under a full-line
#: anchor rather than "\S+".
FAILED_RE = re.compile(r"^test (.+) \.\.\. FAILED$")

#: ``gh run view --log`` prefix: "<job>\t<step>\t<ISO timestamp> " ahead
#: of the raw cargo output. Stripped so downloaded logs parse exactly
#: like the raw tee'd output this tool reads in CI.
GH_PREFIX_RE = re.compile(r"^[^\t]+\t[^\t]+\t\d{4}-\d{2}-\d{2}T[\d:.]+Z ")


def extract_failing_tests(log_text: str) -> list[str]:
    """Return the failing test names, in first-seen order, deduplicated."""
    seen: set[str] = set()
    names: list[str] = []
    for line in log_text.splitlines():
        m = FAILED_RE.match(GH_PREFIX_RE.sub("", line).strip())
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
        # On pull_request runs GITHUB_REF_NAME is "<pr>/merge", which is
        # useless for branch correlation; the head ref is the real branch.
        "branch": os.environ.get("GITHUB_HEAD_REF")
        or os.environ.get("GITHUB_REF_NAME", ""),
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
