#!/usr/bin/env python3
"""Fail unless every expected Kani harness has a completed, successful result.

Usage:
    kani-json-check.py EXPECTED_HARNESS [EXPECTED_HARNESS ...]

Reads `kani-*.json` files in the current directory (Kani --export-json output).
The harness_id in the JSON is the fully qualified path, e.g.
`secrets::kani_proofs::tier1_placeholder_battery`; a short name matches if the
fully qualified id ends with `::<short name>`.
"""

import glob
import json
import sys


def load_results(paths):
    results = {}
    summaries = []
    for path in paths:
        try:
            with open(path, encoding="utf-8") as fh:
                doc = json.load(fh)
        except (OSError, json.JSONDecodeError) as exc:
            raise SystemExit(f"{path}: cannot read Kani JSON: {exc}")
        vr = doc.get("verification_results")
        if not isinstance(vr, dict):
            raise SystemExit(f"{path}: no verification_results object (incomplete run?)")
        summaries.append((path, vr.get("summary", {})))
        for item in vr.get("results", []):
            hid = item.get("harness_id")
            if hid:
                results[hid] = item
    return results, summaries


def main(argv):
    expected = argv[1:]
    if not expected:
        raise SystemExit(__doc__)

    paths = sorted(glob.glob("kani-*.json"))
    if not paths:
        raise SystemExit("no kani-*.json files found")

    results, summaries = load_results(paths)
    failed = False

    for path, summary in summaries:
        status = summary.get("status")
        if status != "completed":
            print(f"FAIL {path}: run status is {status!r}, not 'completed'")
            failed = True

    rows = []
    for name in expected:
        matches = [item for hid, item in results.items() if hid == name or hid.endswith("::" + name)]
        if not matches:
            rows.append((name, "MISSING", ""))
            failed = True
            continue
        item = matches[0]
        status = item.get("status")
        duration = item.get("duration_ms")
        rows.append((name, status, f"{duration} ms" if duration is not None else ""))
        if status != "Success":
            failed = True

    width = max(len(r[0]) for r in rows)
    for name, status, duration in rows:
        print(f"{name:<{width}}  {status:<10}  {duration}")

    if failed:
        print("Kani result check: FAILED")
        return 1
    print("Kani result check: all expected harnesses succeeded")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
