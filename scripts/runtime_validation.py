#!/usr/bin/env python3
"""Prepare and optionally run bounded base-vs-candidate runtime checks.

The fixture is synthetic and intentionally small; it is suitable for checking
workflow correctness and measurement plumbing, not production-scale claims.
The runner sets common Hugging Face offline flags and refuses to start unless
the caller provides already-built binaries and a verified local model artifact.
It does not change network controls; only run it where outbound inference/model
downloads are independently disabled.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
try:
    import resource
except ImportError:  # Windows does not expose resource limits.
    resource = None
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
FIXTURE = REPO / "tests" / "fixtures" / "runtime_validation"
BASELINE = "219e47de604a283c6fb8d4249db77b9824e35188"
MODEL_REPO = "nomic-ai/nomic-embed-text-v1.5"
MODEL_REVISION = "0a3f1bfa99207a5448900a6daaaf6add10eaa058"
MODEL_PATH = "onnx/model.onnx"
MODEL_SHA256 = "147d5aa88c2101237358e17796cf3a227cead1ec304ec34b465bb08e9d952965"

# Conservative machine-specific budgets. Address space and CPU time are
# OS-limited per child; RSS and DB/WAL are checked after each process.
MAX_RUN_SECONDS = 20 * 60
COMMAND_TIMEOUT_SECONDS = 120
MAX_RSS_KIB = 4 * 1024 * 1024
MAX_ADDRESS_SPACE_BYTES = 8 * 1024 * 1024 * 1024
MAX_DB_BUNDLE_BYTES = 1024 * 1024 * 1024
MAX_FIXTURE_BYTES = 2 * 1024 * 1024


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def inventory_model_cache(root: Path) -> tuple[int, list[dict[str, str | int]]]:
    """Hash every cache file and reject symlinks that evade disk budgeting."""
    entries: list[dict[str, str | int]] = []
    total = 0
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise SystemExit(f"model cache must not contain symlinks (budget and identity would be ambiguous): {path}")
        if path.is_file():
            size = path.stat().st_size
            total += size
            entries.append({"path": path.relative_to(root).as_posix(), "bytes": size, "sha256": sha256(path)})
    return total, entries


def read_limit(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8").strip()
    except OSError:
        return None


def make_fixtures() -> dict:
    FIXTURE.mkdir(parents=True, exist_ok=True)
    sources = {
        "src/core.rs": """//! Synthetic multilingual fixture: привет, こんにちは, مرحبا, café.
pub struct MemoryLedger { pub sequence: u64 }
impl MemoryLedger {
    /// Keep edge targets stable across replacement and restore.
    pub fn publish_generation(&self, generation: &str) -> bool {
        !generation.is_empty() && self.sequence > 0
    }
}
pub fn stable_lookup(key: &str) -> Option<&'static str> {
    match key { "alpha" => Some("first"), "beta" => Some("second"), _ => None }
}
""",
        "src/recovery.rs": """//! Recovery cases: naïve café, 東京, Ελληνικά, and हिन्दी.
pub fn retain_unowned_namespace(name: &str) -> bool {
    !name.starts_with("__neurostrata_ingest_stage__")
}
pub fn checkpoint_after_commit(committed: bool) -> &'static str {
    if committed { "new-generation" } else { "old-generation" }
}
""",
        "src/long_symbol.rs": (
            "//! Bounded long-symbol input, synthetic only.\n"
            "pub fn long_identifier_"
            + "x" * 768
            + "(input: &str) -> usize {\n"
            + "    input.chars().map(|ch| ch.len_utf8()).sum()\n}\n"
        ),
        "docs/fixture.md": """# Synthetic workload

This fixture contains deterministic source and documentation, no customer data.
It covers multilingual text, a long symbol, stable path identifiers, and an
ingestion replacement that can be repeated without changing unrelated memory.
""",
    }
    for relative, content in sources.items():
        target = FIXTURE / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8", newline="\n")

    inventory = {
        "classification": "synthetic, small workflow fixture; not production representative",
        "corpus_files": sorted(sources),
        "preservation_records": [
            {"namespace": "validation-preservation", "type": "fact", "content": "synthetic fact: keep exact text"},
            {"namespace": "validation-preservation", "type": "task", "content": "synthetic task: keep status"},
            {"namespace": "validation-preservation", "type": "procedure", "content": "synthetic procedure: keep steps"},
            {"namespace": "validation-preservation", "type": "history", "content": "synthetic history: keep provenance"},
            {"namespace": "validation-preservation", "type": "rule", "content": "synthetic rule: governs src/core.rs", "edge": {"type": "GOVERNS", "target": "src/core.rs"}},
        ],
        "runner_cases": ["one initial real CLI ingest", "preservation readback of five hand-written memory types", "manual rule edge relinks to synthetic source"],
        "planned_but_not_run_here": ["identical reingest", "source removal/replacement", "backup and restore", "process crash boundaries", "reader concurrency"],
        "implemented_assertions": ["preserved record content and types", "manual rule GOVERNS edge points to ingested src/core.rs", "database and WAL fit predeclared cap"],
        "not_implemented_in_runner": ["old/new generation visibility at process-crash boundary", "no mixed IDs, vectors, or payloads", "restore round-trip", "source removal and reingest"],
    }
    manifest = {
        "baseline_revision": BASELINE,
        "candidate_revision": "supplied to runner",
        "model": {
            "repository": MODEL_REPO,
            "revision": MODEL_REVISION,
            "path": MODEL_PATH,
            "sha256": MODEL_SHA256,
            "note": "official pinned file; runtime compatibility must be verified before loading",
        },
        "budget": {
            "wall_seconds": MAX_RUN_SECONDS,
            "command_timeout_seconds": COMMAND_TIMEOUT_SECONDS,
            "address_space_bytes": MAX_ADDRESS_SPACE_BYTES,
            "peak_rss_kib": MAX_RSS_KIB,
            "db_wal_bundle_bytes": MAX_DB_BUNDLE_BYTES,
            "fixture_bytes": MAX_FIXTURE_BYTES,
        },
        "host": {
            "os": platform.platform(),
            "python": sys.version.split()[0],
            "cpu_count": os.cpu_count(),
            "cgroup_memory_limit": read_limit(Path("/sys/fs/cgroup/memory.max")),
            "cgroup_cpu_quota": read_limit(Path("/sys/fs/cgroup/cpu.max")),
            "temporary_filesystem_free_bytes": shutil.disk_usage(Path(tempfile.gettempdir())).free,
        },
        "fixture": inventory,
        "corpus_sha256": {},
    }
    total = 0
    for relative in sorted(sources):
        path = FIXTURE / relative
        total += path.stat().st_size
        manifest["corpus_sha256"][relative] = sha256(path)
    if total > MAX_FIXTURE_BYTES:
        raise SystemExit(f"fixture exceeds {MAX_FIXTURE_BYTES} bytes")
    (FIXTURE / "inventory.json").write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (FIXTURE / "manifest.template.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return manifest


def check_model(path: Path) -> None:
    if not path.is_file():
        raise SystemExit(f"model artifact missing; no inference started: {path}")
    actual = sha256(path)
    if actual != MODEL_SHA256:
        raise SystemExit(f"model SHA-256 mismatch; no inference started: expected {MODEL_SHA256}, got {actual}")


def run_command(command: list[str], env: dict[str, str], timeout: int, log: Path,
                deadline: float) -> tuple[str, int | None, float]:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise RuntimeError("20-minute overall benchmark budget expired before command start")
    timeout = min(timeout, max(1, int(remaining)))
    time_binary = shutil.which("/usr/bin/time")
    timed_command = ([time_binary, "-f", "__RUNTIME_VALIDATION_RSS_KIB__=%M", *command]
                     if time_binary else command)
    started = time.monotonic()

    def apply_child_limits() -> None:
        if resource is None:
            return
        if hasattr(resource, "RLIMIT_AS"):
            resource.setrlimit(resource.RLIMIT_AS,
                               (MAX_ADDRESS_SPACE_BYTES, MAX_ADDRESS_SPACE_BYTES))
        if hasattr(resource, "RLIMIT_CPU"):
            resource.setrlimit(resource.RLIMIT_CPU, (timeout, timeout + 2))

    try:
        result = subprocess.run(timed_command, env=env, capture_output=True, text=True,
                                timeout=timeout, check=False,
                                preexec_fn=apply_child_limits if os.name == "posix" else None)
    except subprocess.TimeoutExpired as error:
        with log.open("a", encoding="utf-8") as output:
            output.write("$ " + " ".join(timed_command) + "\nCOMMAND TIMED OUT\n")
            output.write(str(error.stdout or ""))
            output.write(str(error.stderr or ""))
        raise RuntimeError(f"command exceeded its {timeout}s watchdog; see {log}") from error
    elapsed = time.monotonic() - started
    with log.open("a", encoding="utf-8") as output:
        output.write("$ " + " ".join(timed_command) + "\n")
        output.write(result.stdout)
        output.write(result.stderr)
        output.flush()
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}); see {log}: {' '.join(command)}")
    import re
    rss = re.search(r"__RUNTIME_VALIDATION_RSS_KIB__=(\d+)", result.stderr)
    return result.stdout, int(rss.group(1)) if rss else None, elapsed


def run_binary(label: str, binary: Path, revision: str, root: Path, model: Path,
               model_cache: Path, deadline: float) -> dict:
    binary = binary.resolve(strict=True)
    work = root / label
    home = work / "home"
    home.mkdir(parents=True)
    (home / ".config" / "neurostrata").mkdir(parents=True)
    db = work / "db" / "ladybug.store"
    db.parent.mkdir()
    (home / ".config" / "neurostrata" / "config.json").write_text(
        json.dumps({"db_path": str(db),
                    "store": {"buffer_pool_bytes": 268435456,
                              "max_db_bytes": MAX_DB_BUNDLE_BYTES}}), encoding="utf-8")
    # Only reuse caller-provided artifacts. Offline flags reduce accidental
    # fetches; operators must also ensure network egress is disabled externally.
    cache = home / ".cache" / "neuro" / "models" / "fastembed"
    cache.parent.mkdir(parents=True)
    # Cache is copied for each candidate to keep runtime metadata isolated from
    # the caller's source cache and from the other measured binary.
    shutil.copytree(model_cache, cache)
    env = os.environ.copy()
    env.update({"HOME": str(home), "HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1",
                "NEUROSTRATA_MODEL": "NomicEmbedTextV15"})
    log = work / "run.log"
    commands = [
        [str(binary), "add", "validation-preservation", "fact", "synthetic fact: keep exact text"],
        [str(binary), "add", "validation-preservation", "task", "synthetic task: keep status"],
        [str(binary), "add", "validation-preservation", "procedure", "synthetic procedure: keep steps"],
        [str(binary), "add", "validation-preservation", "history", "synthetic history: keep provenance"],
        [str(binary), "add", "validation-preservation", "rule", "synthetic rule: governs src/core.rs"],
    ]
    max_rss_kib = 0
    command_timings = []
    rule_id = None
    for index, command in enumerate(commands):
        if time.monotonic() >= deadline:
            raise RuntimeError("20-minute overall benchmark budget expired")
        command_output, rss_kib, elapsed = run_command(command, env, COMMAND_TIMEOUT_SECONDS, log, deadline)
        command_timings.append({"phase": "seed_memory_cli", "command": command[2:5],
                                "wall_seconds": round(elapsed, 4), "peak_rss_kib": rss_kib})
        if rss_kib is not None:
            max_rss_kib = max(max_rss_kib, rss_kib)
            if max_rss_kib > MAX_RSS_KIB:
                raise RuntimeError(f"RSS budget exceeded: {max_rss_kib} KiB > {MAX_RSS_KIB} KiB")
        if index == len(commands) - 1:
            import re
            match = re.search(r"Memory added successfully with ID: ([0-9a-fA-F-]{36})", command_output)
            rule_id = match.group(1) if match else None
    import re
    if not rule_id:
        raise RuntimeError(f"could not identify rule id in command log {log}")
    output, rss_kib, elapsed = run_command(
        [str(binary), "set-metadata", "validation-preservation", rule_id,
         json.dumps({"governs": ["src/core.rs"]})], env, COMMAND_TIMEOUT_SECONDS, log, deadline)
    if rss_kib is not None:
        max_rss_kib = max(max_rss_kib, rss_kib)
        if max_rss_kib > MAX_RSS_KIB:
            raise RuntimeError(f"RSS budget exceeded: {max_rss_kib} KiB > {MAX_RSS_KIB} KiB")
    command_timings.append({"phase": "set_rule_edge", "wall_seconds": round(elapsed, 4),
                            "peak_rss_kib": rss_kib})
    if not output:
        raise RuntimeError("set-metadata returned no confirmation")
    _, rss_kib, elapsed = run_command(
        [str(binary), "ingest", str(FIXTURE / "src"), "synthetic-validation"], env,
        COMMAND_TIMEOUT_SECONDS, log, deadline)
    if rss_kib is not None:
        max_rss_kib = max(max_rss_kib, rss_kib)
        if max_rss_kib > MAX_RSS_KIB:
            raise RuntimeError(f"RSS budget exceeded: {max_rss_kib} KiB > {MAX_RSS_KIB} KiB")
    command_timings.append({"phase": "ingest", "wall_seconds": round(elapsed, 4),
                            "peak_rss_kib": rss_kib,
                            "synthetic_source_files_per_second": round(3 / elapsed, 4) if elapsed else None})
    graph_path = work / "graph.json"
    _, rss_kib, elapsed = run_command(
        [str(binary), "export-graph", str(graph_path)], env, COMMAND_TIMEOUT_SECONDS, log, deadline)
    if rss_kib is not None:
        max_rss_kib = max(max_rss_kib, rss_kib)
        if max_rss_kib > MAX_RSS_KIB:
            raise RuntimeError(f"RSS budget exceeded: {max_rss_kib} KiB > {MAX_RSS_KIB} KiB")
    command_timings.append({"phase": "graph_readback", "wall_seconds": round(elapsed, 4),
                            "peak_rss_kib": rss_kib})
    graph = json.loads(graph_path.read_text(encoding="utf-8"))
    nodes = graph.get("nodes", [])
    links = graph.get("links", [])
    preserve_nodes = [node for node in nodes if node.get("namespace") == "validation-preservation"]
    expected_records = json.loads((FIXTURE / "inventory.json").read_text())["preservation_records"]
    for expected in expected_records:
        if not any(node.get("content") == expected["content"]
                   and node.get("memory_type") == expected["type"] for node in preserve_nodes):
            raise RuntimeError(f"preservation assertion failed for {expected!r}; see {graph_path}")
    if not any(link.get("source") == rule_id and link.get("type") == "GOVERNS"
               and link.get("target", "").endswith("src/core.rs") for link in links):
        raise RuntimeError(f"GOVERNS preservation/relink assertion failed; see {graph_path}")
    bundle_size = sum(path.stat().st_size for path in work.glob("db/ladybug.store*"))
    if bundle_size > MAX_DB_BUNDLE_BYTES:
        raise RuntimeError(f"DB/WAL budget exceeded: {bundle_size} bytes")
    return {"label": label, "binary": str(binary), "revision": revision,
            "binary_sha256": sha256(binary),
            "model_sha256": sha256(model), "peak_rss_kib": max_rss_kib or None,
            "db_bundle_bytes": bundle_size, "graph_nodes": len(nodes),
            "preservation_record_count": len(preserve_nodes), "governs_edge_verified": True,
            "command_timings": command_timings,
            "log": str(log), "disclaimer": "one CLI lifecycle per binary; timing includes process/model startup, synthetic scale only"}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-bin", type=Path)
    parser.add_argument("--candidate-bin", type=Path)
    parser.add_argument("--candidate-revision", default="uncommitted")
    parser.add_argument("--baseline-build-settings", default="caller must record exact build command/features")
    parser.add_argument("--candidate-build-settings", default="caller must record exact build command/features")
    parser.add_argument("--model-file", type=Path)
    parser.add_argument("--model-cache", type=Path,
                        help="existing complete FastEmbed cache root; copied into isolated per-binary homes")
    parser.add_argument("--output", type=Path, default=Path("runtime-validation-results"))
    args = parser.parse_args()
    if not args.baseline_bin and not args.candidate_bin:
        make_fixtures()
        print(f"Synthetic fixtures prepared at {FIXTURE}")
        print("Preparation only; no inference or measurements were run.")
        return
    if not all((args.baseline_bin, args.candidate_bin, args.model_file, args.model_cache)):
        raise SystemExit("running requires both binaries, --model-file, and --model-cache; no inference started")
    check_model(args.model_file.resolve())
    if not args.model_cache.is_dir():
        raise SystemExit(f"complete FastEmbed model cache missing; no inference started: {args.model_cache}")
    if not args.model_file.resolve().is_relative_to(args.model_cache.resolve()):
        raise SystemExit("--model-file must be inside --model-cache; no inference started")
    if args.baseline_build_settings.startswith("caller must") or args.candidate_build_settings.startswith("caller must"):
        raise SystemExit("provide exact --baseline-build-settings and --candidate-build-settings; no inference started")
    cache_bytes, cache_manifest = inventory_model_cache(args.model_cache)
    required_free = 2 * cache_bytes + 2 * MAX_DB_BUNDLE_BYTES
    if shutil.disk_usage(Path(tempfile.gettempdir())).free < required_free:
        raise SystemExit(f"temporary filesystem needs {required_free} free bytes for isolated caches and DB budgets; no inference started")
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = make_fixtures()
    print(f"Synthetic fixtures prepared at {FIXTURE}")
    started = time.monotonic()
    deadline = started + MAX_RUN_SECONDS
    results = []
    with tempfile.TemporaryDirectory(prefix="neurostrata-runtime-validation-") as temporary:
        root = Path(temporary)
        if shutil.disk_usage(root).free < required_free:
            raise RuntimeError("temporary filesystem budget changed before model-cache copy")
        try:
            results.append(run_binary("baseline", args.baseline_bin, BASELINE, root,
                                      args.model_file, args.model_cache, deadline))
            results.append(run_binary("candidate", args.candidate_bin, args.candidate_revision,
                                      root, args.model_file, args.model_cache, deadline))
            if time.monotonic() > deadline:
                raise RuntimeError("20-minute overall benchmark budget expired")
        finally:
            # Preserve diagnostic logs even when a candidate, cap, or watchdog
            # fails. The temporary databases and private caches are discarded.
            logs_dir = args.output / "logs"
            logs_dir.mkdir(exist_ok=True)
            for label in ("baseline", "candidate"):
                temporary_log = root / label / "run.log"
                if temporary_log.is_file():
                    stored_log = logs_dir / f"{label}.log"
                    shutil.copyfile(temporary_log, stored_log)
                    for result in results:
                        if result["label"] == label:
                            result["log"] = str(stored_log)
    manifest.update({"candidate_revision": args.candidate_revision,
                     "build_settings": {"baseline": args.baseline_build_settings,
                                        "candidate": args.candidate_build_settings},
                     "model_artifact_sha256": sha256(args.model_file),
                     "model_cache_bytes": cache_bytes,
                     "model_cache_files": cache_manifest,
                     "binary_sha256": {"baseline": sha256(args.baseline_bin.resolve()),
                                       "candidate": sha256(args.candidate_bin.resolve())},
                     "commands": "scripts/runtime_validation.py; see per-binary run.log files",
                     "results": results,
                     "elapsed_seconds": round(time.monotonic() - started, 3),
                     "execution": "real production CLI with isolated copies of supplied local model cache; one synthetic run each",
                     "not_measured": ["idle daemon RSS", "steady-state in-process inference latency",
                                      "reader responsiveness during publication", "three-warm-run variance"]})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(f"Results recorded at {args.output / 'manifest.json'}")


if __name__ == "__main__":
    main()
