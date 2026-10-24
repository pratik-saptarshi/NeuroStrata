#!/usr/bin/env python3
"""Prepare and optionally run bounded base-vs-candidate runtime checks.

The fixture is synthetic and intentionally small; it is suitable for checking
workflow correctness and measurement plumbing, not production-scale claims.
The runner sets common Hugging Face offline flags and refuses to start unless
the caller provides already-built binaries and a verified local model artifact.
It also requires confirmation that an independent memory and temporary-disk
quota surrounds the whole run; RSS and DB/WAL budgets are post-command checks.
The overall wall-time deadline is checked after synchronous setup copies and
before/after CLI phases; a single file-copy call itself cannot be interrupted.
It does not change network controls; only run it where outbound inference/model
downloads are independently disabled.

The model under test is a spec {name, repository, revision, sha256, model
path}: the built-in default is the pinned Nomic artifact, a supplied spec
arrives as a JSON file (--model-spec, single run), and --matrix runs the full
baseline+candidate scenario per spec against the same candidate binary,
writing a self-labeling matrix-report.json whose legs are PASS, FAIL, or
SKIPPED with a machine-readable reason — a SKIPPED leg is never a pass.
"""

from __future__ import annotations

import argparse
import hashlib
from dataclasses import dataclass
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
import threading
import time
from pathlib import Path


FIXTURE = Path(__file__).resolve().parent / "fixtures" / "synthetic-project"
BASELINE = "219e47de604a283c6fb8d4249db77b9824e35188"
MODEL_REPO = "nomic-ai/nomic-embed-text-v1.5"
MODEL_REVISION = "0a3f1bfa99207a5448900a6daaaf6add10eaa058"
MODEL_PATH = "onnx/model.onnx"
MODEL_SHA256 = "147d5aa88c2101237358e17796cf3a227cead1ec304ec34b465bb08e9d952965"


@dataclass(frozen=True)
class ModelSpec:
    """One model's validation identity: the fastembed model name the binary
    is told to load (NEUROSTRATA_MODEL), where the artifact is published, and
    the pinned artifact the SHA gate verifies, at its path inside the cache."""

    name: str
    repository: str
    revision: str
    sha256: str
    model_path: str

    def to_dict(self) -> dict[str, str]:
        return {"name": self.name, "repository": self.repository, "revision": self.revision,
                "sha256": self.sha256, "model_path": self.model_path}


MODEL_SPEC_FIELDS = ("name", "repository", "revision", "sha256", "model_path")

# The built-in default spec is exactly the pre-matrix Nomic constants above:
# invoking the runner with no new flags keeps the same gates and messages.
DEFAULT_MODEL_SPEC = ModelSpec(
    name="NomicEmbedTextV15",
    repository=MODEL_REPO,
    revision=MODEL_REVISION,
    sha256=MODEL_SHA256,
    model_path=MODEL_PATH,
)

# Second spec of record. Only the name (src/config.rs profile table,
# src/embed.rs) and the 384 width have in-tree sources; the repository,
# revision, sha256, and model path appear nowhere in this tree, so they are
# recorded as UNVERIFIED rather than invented. A leg using this spec SKIPs at
# preflight (its artifact cannot be SHA-verified) until a real run supplies
# verified values through a supplied spec file.
BGE_SMALL_MODEL_SPEC = ModelSpec(
    name="BGESmallENV15Q",
    repository="UNVERIFIED",
    revision="UNVERIFIED",
    sha256="UNVERIFIED",
    model_path="UNVERIFIED",
)

BUILTIN_MODEL_SPECS = {spec.name: spec for spec in (DEFAULT_MODEL_SPEC, BGE_SMALL_MODEL_SPEC)}


def load_model_spec(path: Path) -> ModelSpec:
    """Load a supplied model spec from a JSON file (the --model-spec /
    --matrix argument encoding)."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"cannot read model spec {path}: {error}")
    if not isinstance(data, dict):
        raise SystemExit(f"model spec must be a JSON object: {path}")
    missing = [field for field in MODEL_SPEC_FIELDS if field not in data]
    if missing:
        raise SystemExit(f"model spec {path} is missing required field(s): {', '.join(missing)}")
    values: dict[str, str] = {}
    for field in MODEL_SPEC_FIELDS:
        value = data[field]
        if not isinstance(value, str) or not value.strip():
            raise SystemExit(f"model spec field {field!r} must be a non-empty string: {path}")
        values[field] = value
    return ModelSpec(**values)

# Conservative machine-specific budgets. Address space and CPU time are
# OS-limited per child. RSS and the combined DB/WAL size are post-command
# acceptance thresholds; operators must also use an external memory/disk cap.
MAX_RUN_SECONDS = 20 * 60
COMMAND_TIMEOUT_SECONDS = 120
MAX_RSS_KIB = 4 * 1024 * 1024
MAX_ADDRESS_SPACE_BYTES = 8 * 1024 * 1024 * 1024
MAX_DB_BUNDLE_BYTES = 1024 * 1024 * 1024
MAX_FIXTURE_BYTES = 2 * 1024 * 1024
MAX_CAPTURE_BYTES = 4 * 1024 * 1024
MAX_EXTERNAL_MEMORY_BYTES = 16 * 1024 * 1024 * 1024


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def inventory_model_cache(root: Path) -> tuple[int, list[dict[str, str | int]]]:
    """Hash every cache file and reject symlinks that evade disk budgeting."""
    if root.is_symlink():
        raise SystemExit(f"model cache root must not be a symlink: {root}")
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


def require_bounded_environment(confirmed: bool) -> dict[str, object]:
    if not confirmed:
        raise SystemExit(
            "real CLI runs require --confirm-external-bounds after applying independent memory and temporary-disk quotas"
        )
    if os.name != "posix" or resource is None or not hasattr(os, "waitid") or not hasattr(os, "WNOWAIT"):
        raise SystemExit("bounded process-group execution requires POSIX waitid/WNOWAIT and resource limits")
    observed_memory_limit = None
    cgroup_limit = read_limit(Path("/sys/fs/cgroup/memory.max"))
    if cgroup_limit and cgroup_limit != "max":
        try:
            observed_memory_limit = int(cgroup_limit)
        except ValueError:
            raise SystemExit("could not parse the active cgroup memory limit")
        if observed_memory_limit > MAX_EXTERNAL_MEMORY_BYTES:
            raise SystemExit(
                f"active cgroup memory limit exceeds the {MAX_EXTERNAL_MEMORY_BYTES}-byte safe budget"
            )
    return {
        "operator_confirmed_external_memory_and_temporary_disk_quotas": True,
        "observed_cgroup_memory_limit_bytes": observed_memory_limit,
        "rss_and_db_wal_are_post_command_acceptance_thresholds": True,
    }


def _waitid_exited(process: subprocess.Popen[bytes]) -> bool:
    info = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
    return info is not None and info.si_pid == process.pid


def _signal_owned_group(process: subprocess.Popen[bytes], sig: int) -> None:
    # The group leader is not reaped until all group signals have been sent,
    # so its PID/PGID cannot be recycled between observation and signal.
    try:
        os.killpg(process.pid, sig)
    except ProcessLookupError:
        pass


def _terminate_owned_group(process: subprocess.Popen[bytes]) -> None:
    import signal

    _signal_owned_group(process, signal.SIGTERM)
    grace_end = time.monotonic() + 0.25
    while time.monotonic() < grace_end and not _waitid_exited(process):
        time.sleep(0.02)
    # Also remove descendants left behind if the direct wrapper already exited.
    # The leader remains waitable-but-unreaped until after this signal.
    _signal_owned_group(process, signal.SIGKILL)
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        _signal_owned_group(process, signal.SIGKILL)
        process.wait(timeout=2)


def make_fixtures() -> dict:
    """Validate checked-in fixture inputs and build an in-memory run manifest."""
    if FIXTURE.is_symlink() or not FIXTURE.is_dir():
        raise SystemExit(f"checked-in synthetic fixture directory is missing or unsafe: {FIXTURE}")
    sources: dict[str, Path] = {}
    for folder in ("src", "docs"):
        root = FIXTURE / folder
        if root.is_symlink() or not root.is_dir():
            raise SystemExit(f"synthetic fixture folder is missing or unsafe: {root}")
        for path in sorted(root.rglob("*")):
            if path.is_symlink():
                raise SystemExit(f"synthetic fixture must not contain symlinks: {path}")
            if path.is_file():
                sources[path.relative_to(FIXTURE).as_posix()] = path
    if not sources:
        raise SystemExit(f"synthetic fixture contains no source files: {FIXTURE}")
    try:
        inventory = json.loads((FIXTURE / "inventory.json").read_text(encoding="utf-8"))
        template = json.loads((FIXTURE / "manifest.template.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"cannot read checked-in fixture metadata under {FIXTURE}: {error}")
    if not isinstance(inventory, dict) or not isinstance(template, dict):
        raise SystemExit(f"checked-in fixture metadata must be JSON objects under {FIXTURE}")
    if inventory.get("corpus_files") != sorted(sources):
        raise SystemExit("fixture inventory does not match checked-in source/docs files")

    manifest = dict(template)
    manifest["baseline_revision"] = BASELINE
    manifest["candidate_revision"] = "supplied to runner"
    manifest["budget"] = {
        "wall_seconds_acceptance_window": MAX_RUN_SECONDS,
        "setup_copy_timeout": "synchronous copy is not interruptible; acceptance deadline checked immediately afterward",
        "command_timeout_seconds": COMMAND_TIMEOUT_SECONDS,
        "address_space_bytes": MAX_ADDRESS_SPACE_BYTES,
        "peak_rss_kib": MAX_RSS_KIB,
        "db_wal_bundle_bytes": MAX_DB_BUNDLE_BYTES,
        "fixture_bytes": MAX_FIXTURE_BYTES,
    }
    manifest["host"] = {
        "os": platform.platform(),
        "python": sys.version.split()[0],
        "cpu_count": os.cpu_count(),
        "cgroup_memory_limit": read_limit(Path("/sys/fs/cgroup/memory.max")),
        "cgroup_cpu_quota": read_limit(Path("/sys/fs/cgroup/cpu.max")),
        "temporary_filesystem_free_bytes": shutil.disk_usage(Path(tempfile.gettempdir())).free,
    }
    manifest["fixture"] = inventory
    manifest["corpus_sha256"] = {}
    total = 0
    for relative, path in sorted(sources.items()):
        total += path.stat().st_size
        manifest["corpus_sha256"][relative] = sha256(path)
    if total > MAX_FIXTURE_BYTES:
        raise SystemExit(f"fixture exceeds {MAX_FIXTURE_BYTES} bytes")
    return manifest


def check_model(path: Path, spec: ModelSpec = DEFAULT_MODEL_SPEC) -> None:
    if not path.is_file():
        raise SystemExit(f"model artifact missing; no inference started: {path}")
    actual = sha256(path)
    if actual != spec.sha256:
        raise SystemExit(f"model SHA-256 mismatch; no inference started: expected {spec.sha256}, got {actual}")


def run_command(command: list[str], env: dict[str, str], timeout: int, log: Path,
                deadline: float, cwd: Path | None = None) -> tuple[str, int | None, float]:
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

    if os.name != "posix" or not hasattr(os, "waitid") or not hasattr(os, "WNOWAIT"):
        raise RuntimeError("refusing command without safe owned-process-group reaping support")
    process = subprocess.Popen(
        timed_command,
        env=env,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=0,
        start_new_session=True,
        preexec_fn=apply_child_limits,
    )
    output_exceeded = threading.Event()
    captured = {"stdout": bytearray(), "stderr": bytearray()}

    def drain(stream, key: str) -> None:
        assert stream is not None
        while True:
            chunk = stream.read(64 * 1024)
            if not chunk:
                return
            output = captured[key]
            remaining = MAX_CAPTURE_BYTES - len(output)
            if len(chunk) > remaining:
                output_exceeded.set()
            if remaining > 0:
                output.extend(chunk[:remaining])

    readers = [
        threading.Thread(target=drain, args=(process.stdout, "stdout"), daemon=True),
        threading.Thread(target=drain, args=(process.stderr, "stderr"), daemon=True),
    ]
    for reader in readers:
        reader.start()
    end = time.monotonic() + timeout
    failure = None
    while True:
        if _waitid_exited(process):
            # The command may have left descendants in its session. Kill
            # them while the leader is still a zombie and anchors the PGID.
            _terminate_owned_group(process)
            break
        if output_exceeded.is_set():
            failure = f"command exceeded its {MAX_CAPTURE_BYTES}-byte per-stream output cap"
            _terminate_owned_group(process)
            break
        if time.monotonic() >= end:
            failure = f"command exceeded its {timeout}s watchdog"
            _terminate_owned_group(process)
            break
        time.sleep(0.01)
    for reader in readers:
        reader.join(timeout=2)
    if any(reader.is_alive() for reader in readers):
        failure = failure or "command descendants kept captured output pipes open after group cleanup"
    if output_exceeded.is_set():
        failure = failure or f"command exceeded its {MAX_CAPTURE_BYTES}-byte per-stream output cap"
    if process.stdout is not None:
        process.stdout.close()
    if process.stderr is not None:
        process.stderr.close()
    elapsed = time.monotonic() - started
    stdout = captured["stdout"].decode("utf-8", errors="replace")
    stderr = captured["stderr"].decode("utf-8", errors="replace")
    returncode = process.returncode
    elapsed = time.monotonic() - started
    with log.open("a", encoding="utf-8") as output:
        output.write("$ " + " ".join(timed_command) + "\n")
        if failure:
            output.write(f"COMMAND STOPPED: {failure}\n")
        output.write(stdout)
        output.write(stderr)
        output.flush()
    if failure:
        raise RuntimeError(f"{failure}; see {log}")
    if returncode:
        raise RuntimeError(f"command failed ({returncode}); see {log}: {' '.join(command)}")
    import re
    rss = re.search(r"__RUNTIME_VALIDATION_RSS_KIB__=(\d+)", stderr)
    return stdout, int(rss.group(1)) if rss else None, elapsed


def check_database_bundle(work: Path, phase: str) -> int:
    bundle_size = sum(path.stat().st_size for path in work.glob("db/ladybug.store*"))
    if bundle_size > MAX_DB_BUNDLE_BYTES:
        raise RuntimeError(
            f"DB/WAL post-phase acceptance threshold exceeded after {phase}: "
            f"{bundle_size} bytes > {MAX_DB_BUNDLE_BYTES} bytes"
        )
    return bundle_size


def check_measured_rss(rss_kib: int | None, maximum: int, phase: str) -> int:
    if rss_kib is None:
        return maximum
    maximum = max(maximum, rss_kib)
    if maximum > MAX_RSS_KIB:
        raise RuntimeError(
            f"measured RSS post-command acceptance threshold exceeded after {phase}: "
            f"{maximum} KiB > {MAX_RSS_KIB} KiB"
        )
    return maximum


def run_binary(label: str, binary: Path, revision: str, root: Path, model: Path,
               model_cache: Path, deadline: float, spec: ModelSpec = DEFAULT_MODEL_SPEC) -> dict:
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
    if time.monotonic() >= deadline:
        raise RuntimeError("20-minute overall acceptance deadline expired while copying the model cache")
    env = os.environ.copy()
    env.update({"HOME": str(home), "HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1",
                "NEUROSTRATA_MODEL": spec.name})
    log = work / "run.log"
    commands = [
        [str(binary), "add", "validation-preservation", "fact", "synthetic fact: keep exact text"],
        [str(binary), "add", "validation-preservation", "task", "synthetic task: keep status"],
        [str(binary), "add", "validation-preservation", "procedure", "synthetic procedure: keep steps"],
        [str(binary), "add", "validation-preservation", "history", "synthetic history: keep provenance"],
        [str(binary), "add", "synthetic-validation", "rule", "synthetic rule: governs src/core.rs"],
    ]
    max_rss_kib = 0
    command_timings = []
    rule_id = None
    seeded_records = []
    source_root = work / "project"
    source_root.mkdir()
    shutil.copytree(FIXTURE / "src", source_root / "src")
    if time.monotonic() >= deadline:
        raise RuntimeError("20-minute overall acceptance deadline expired while preparing the source fixture")
    expected_file_id = "synthetic-validation::src/core.rs"
    for index, command in enumerate(commands):
        if time.monotonic() >= deadline:
            raise RuntimeError("20-minute overall benchmark budget expired")
        command_output, rss_kib, elapsed = run_command(command, env, COMMAND_TIMEOUT_SECONDS, log, deadline)
        phase = f"seed_memory_cli_{index + 1}"
        max_rss_kib = check_measured_rss(rss_kib, max_rss_kib, phase)
        bundle_size = check_database_bundle(work, phase)
        command_timings.append({"phase": phase, "command": command[2:5],
                                "wall_seconds": round(elapsed, 4), "peak_rss_kib": rss_kib,
                                "db_wal_bundle_bytes_after_phase": bundle_size})
        if index == len(commands) - 1:
            import re
            match = re.search(r"Memory added successfully with ID: ([0-9a-fA-F-]{36})", command_output)
            rule_id = match.group(1) if match else None
        import re
        match = re.search(r"Memory added successfully with ID: ([0-9a-fA-F-]{36})", command_output)
        if not match:
            raise RuntimeError(f"could not record exact memory id after CLI add; see {log}")
        seeded_records.append({"id": match.group(1), "namespace": command[2],
                               "memory_type": command[3], "content": command[4]})
    import re
    if not rule_id:
        raise RuntimeError(f"could not identify rule id in command log {log}")
    output, rss_kib, elapsed = run_command(
        [str(binary), "set-metadata", "synthetic-validation", rule_id,
         json.dumps({"governs": [expected_file_id]})], env, COMMAND_TIMEOUT_SECONDS, log, deadline)
    max_rss_kib = check_measured_rss(rss_kib, max_rss_kib, "set_rule_edge")
    bundle_size = check_database_bundle(work, "set_rule_edge")
    command_timings.append({"phase": "set_rule_edge", "wall_seconds": round(elapsed, 4),
                            "peak_rss_kib": rss_kib,
                            "db_wal_bundle_bytes_after_phase": bundle_size})
    if not output:
        raise RuntimeError("set-metadata returned no confirmation")
    _, rss_kib, elapsed = run_command(
        [str(binary), "ingest", "./src", "synthetic-validation"], env,
        COMMAND_TIMEOUT_SECONDS, log, deadline, cwd=source_root)
    max_rss_kib = check_measured_rss(rss_kib, max_rss_kib, "ingest")
    bundle_size = check_database_bundle(work, "ingest")
    command_timings.append({"phase": "ingest", "wall_seconds": round(elapsed, 4),
                            "peak_rss_kib": rss_kib,
                            "db_wal_bundle_bytes_after_phase": bundle_size,
                            "synthetic_source_files_per_second": round(3 / elapsed, 4) if elapsed else None})
    graph_path = work / "graph.json"
    _, rss_kib, elapsed = run_command(
        [str(binary), "export-graph", str(graph_path)], env, COMMAND_TIMEOUT_SECONDS, log, deadline)
    max_rss_kib = check_measured_rss(rss_kib, max_rss_kib, "graph_readback")
    bundle_size = check_database_bundle(work, "graph_readback")
    command_timings.append({"phase": "graph_readback", "wall_seconds": round(elapsed, 4),
                            "peak_rss_kib": rss_kib,
                            "db_wal_bundle_bytes_after_phase": bundle_size})
    graph = json.loads(graph_path.read_text(encoding="utf-8"))
    nodes = graph.get("nodes", [])
    links = graph.get("links", [])
    expected_namespaces = {record["namespace"] for record in seeded_records}
    preserve_nodes = [node for node in nodes if node.get("namespace") in expected_namespaces]
    expected_records = json.loads((FIXTURE / "inventory.json").read_text())["preservation_records"]
    for expected in expected_records:
        if not any(node.get("namespace") == expected["namespace"]
                   and node.get("content") == expected["content"]
                   and node.get("memory_type") == expected["type"] for node in preserve_nodes):
            raise RuntimeError(f"preservation assertion failed for {expected!r}; see {graph_path}")
    for expected in seeded_records:
        if not any(node.get("id") == expected["id"]
                   and node.get("namespace") == expected["namespace"]
                   and node.get("content") == expected["content"]
                   and node.get("memory_type") == expected["memory_type"] for node in nodes):
            raise RuntimeError(f"CLI memory id/content changed across ingest: {expected!r}")
    if not any(link.get("source") == rule_id and link.get("type") == "GOVERNS"
               and link.get("target") == expected_file_id for link in links):
        raise RuntimeError(f"GOVERNS preservation/relink assertion failed; see {graph_path}")
    if not any(node.get("id") == expected_file_id
               and node.get("namespace") == "synthetic-validation" for node in nodes):
        raise RuntimeError(f"ingested target id {expected_file_id!r} is absent from graph export")
    return {"label": label, "binary": str(binary), "revision": revision,
            "binary_sha256": sha256(binary),
            "model_sha256": sha256(model), "peak_rss_kib": max_rss_kib or None,
            "db_bundle_bytes": bundle_size, "graph_nodes": len(nodes),
            "preservation_record_count": len(seeded_records), "governs_edge_verified": True,
            "command_timings": command_timings,
            "log": str(log), "disclaimer": "one CLI lifecycle per binary; timing includes process/model startup, synthetic scale only"}


def onnx_runtime_version() -> str:
    """The ONNX Runtime version visible to this harness, for the matrix
    report's environment block. The harness itself links no ONNX Runtime
    (the binary under test does), so an unimportable runtime is reported
    as UNVERIFIED rather than guessed."""
    try:
        import onnxruntime
    except ImportError:
        return "UNVERIFIED"
    return str(getattr(onnxruntime, "__version__", "UNVERIFIED"))


def summarize_legs(legs: list[dict]) -> dict[str, int]:
    """Leg counts by status. A SKIPPED leg is never counted as passed."""
    return {
        "total": len(legs),
        "passed": sum(1 for leg in legs if leg["status"] == "PASS"),
        "failed": sum(1 for leg in legs if leg["status"] == "FAIL"),
        "skipped": sum(1 for leg in legs if leg["status"] == "SKIPPED"),
    }


def matrix_exit_code(report: dict) -> int:
    """Every matrix leg is required: the run exits 0 only when all legs
    PASS. Any FAIL or SKIPPED leg is a nonzero exit, with the report
    already written so the reason survives."""
    summary = report["summary"]
    return 0 if summary["total"] > 0 and summary["passed"] == summary["total"] else 1


def leg_preflight(spec: ModelSpec, model_cache: Path) -> tuple[str, str] | None:
    """The model-specific refuse-to-start gates for one matrix leg, as a
    (machine-readable code, message) pair, or None when the leg may run.
    Only model-specific gates live here — cache presence, model-file-inside-
    cache, the spec's SHA gate, and disk-space arithmetic recomputed against
    this leg's cache. The bounded-environment confirmation, build-settings
    requirement, and output handling stay global in main()."""
    if not model_cache.is_dir():
        return ("model_cache_missing",
                f"complete FastEmbed model cache missing; leg skipped: {model_cache}")
    model_file = (model_cache / spec.model_path).resolve()
    if not model_file.is_relative_to(model_cache.resolve()):
        return ("model_file_outside_cache",
                f"model file for spec {spec.name} resolves outside the model cache; leg skipped: {model_file}")
    if not model_file.is_file():
        return ("model_artifact_missing",
                f"model artifact missing; leg skipped: {model_file}")
    actual = sha256(model_file)
    if actual != spec.sha256:
        return ("model_sha_mismatch",
                f"model SHA-256 mismatch; leg skipped: expected {spec.sha256}, got {actual}")
    try:
        cache_bytes, _ = inventory_model_cache(model_cache)
    except SystemExit as error:
        return ("model_cache_invalid", f"model cache rejected; leg skipped: {error}")
    required_free = 2 * cache_bytes + 2 * MAX_DB_BUNDLE_BYTES
    if shutil.disk_usage(Path(tempfile.gettempdir())).free < required_free:
        return ("insufficient_disk_space",
                f"temporary filesystem needs {required_free} free bytes for this leg's isolated "
                "caches and DB budgets; leg skipped")
    return None


def build_environment_block(specs: list[ModelSpec], baseline_bin: Path, candidate_bin: Path,
                            candidate_revision: str, external_bounds: dict | None) -> dict:
    """The self-labeling environment block inside matrix-report.json: the
    report cannot be quoted as tier evidence without its environment."""
    return {
        "host": {
            "os": platform.platform(),
            "machine": platform.machine(),
            "python": sys.version.split()[0],
            "cpu_count": os.cpu_count(),
        },
        "onnx_runtime_version": onnx_runtime_version(),
        "artifact_provenance": [spec.to_dict() for spec in specs],
        "binary_provenance": {
            "baseline": {"path": str(baseline_bin), "sha256": sha256(baseline_bin.resolve()),
                         "revision": BASELINE},
            "candidate": {"path": str(candidate_bin), "sha256": sha256(candidate_bin.resolve()),
                          "revision": candidate_revision},
        },
        "containment": external_bounds,
        "deadline": {"overall_seconds": MAX_RUN_SECONDS, "shared_across_legs": True},
    }


def _leg_fixture_smoke(leg_root: Path, results: dict[str, dict]) -> dict:
    ingested_file_ids: list[str] = []
    for label in ("baseline", "candidate"):
        graph_path = leg_root / label / "graph.json"
        if not graph_path.is_file():
            continue
        try:
            graph = json.loads(graph_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        ingested_file_ids = sorted(
            {node["id"] for node in graph.get("nodes", [])
             if isinstance(node.get("id"), str) and node["id"].startswith("synthetic-validation::")}
        )
    primary = results.get("candidate") or results.get("baseline") or {}
    return {
        "preservation_record_count": primary.get("preservation_record_count"),
        "governs_edge_verified": bool(primary.get("governs_edge_verified")),
        "graph_nodes": primary.get("graph_nodes"),
        "ingested_file_ids": ingested_file_ids,
        "long_symbol_ingested": "synthetic-validation::src/long_symbol.rs" in ingested_file_ids,
    }


def run_matrix(specs: list[ModelSpec], *, baseline_bin: Path, candidate_bin: Path,
               candidate_revision: str, model_cache: Path, output: Path, workspace: Path,
               deadline: float, external_bounds: dict | None = None,
               run_binary_fn=None) -> dict:
    """Run the full baseline+candidate scenario pair per model spec against
    the same candidate binary and write matrix-report.json under output.
    The single overall deadline is shared across all legs. A leg whose
    model-specific preflight fails, or that the shared deadline preempts,
    is SKIPPED with a machine-readable reason — never a pass, and never a
    crash of the whole run."""
    run_binary_fn = run_binary_fn or run_binary
    baseline_bin = Path(baseline_bin)
    candidate_bin = Path(candidate_bin)
    model_cache = Path(model_cache)
    workspace.mkdir(parents=True, exist_ok=True)
    legs: list[dict] = []
    for index, spec in enumerate(specs):
        leg: dict = {
            "spec": spec.to_dict(),
            "status": "SKIPPED",
            "reason": None,
            "required": True,
            "baseline": None,
            "candidate": None,
            "peak_rss_kib": None,
            "idle_rss_kib": None,
            "db_wal_bytes": {"baseline": None, "candidate": None},
            "fixture_smoke": _leg_fixture_smoke(workspace, {}),
        }
        legs.append(leg)
        if time.monotonic() >= deadline:
            leg["reason"] = {
                "code": "deadline_expired",
                "message": "the shared 20-minute overall deadline expired before this leg "
                           "started; leg skipped",
            }
            continue
        preflight = leg_preflight(spec, model_cache)
        if preflight is not None:
            leg["reason"] = {"code": preflight[0], "message": preflight[1]}
            continue
        model_file = (model_cache / spec.model_path).resolve()
        leg_root = workspace / f"leg-{index}-{spec.name}"
        results: dict[str, dict] = {}
        try:
            results["baseline"] = run_binary_fn(
                "baseline", baseline_bin, BASELINE, leg_root, model_file, model_cache,
                deadline, spec=spec)
            if time.monotonic() >= deadline:
                raise RuntimeError("20-minute overall benchmark budget expired between matrix leg runs")
            results["candidate"] = run_binary_fn(
                "candidate", candidate_bin, candidate_revision, leg_root, model_file,
                model_cache, deadline, spec=spec)
        except RuntimeError as error:
            message = str(error)
            if "deadline" in message or "budget expired" in message:
                leg["status"] = "SKIPPED"
                leg["reason"] = {"code": "deadline_expired", "message": message}
            else:
                leg["status"] = "FAIL"
                leg["reason"] = {"code": "scenario_failed", "message": message}
        else:
            leg["status"] = "PASS"
        leg["baseline"] = results.get("baseline")
        leg["candidate"] = results.get("candidate")
        rss_values = [result["peak_rss_kib"] for result in results.values()
                      if result.get("peak_rss_kib") is not None]
        leg["peak_rss_kib"] = max(rss_values) if rss_values else None
        leg["db_wal_bytes"] = {
            label: (results[label]["db_bundle_bytes"] if label in results else None)
            for label in ("baseline", "candidate")
        }
        leg["fixture_smoke"] = _leg_fixture_smoke(leg_root, results)
    report: dict = {
        "kind": "runtime-validation-matrix",
        "candidate_revision": candidate_revision,
        "environment": build_environment_block(
            specs, baseline_bin, candidate_bin, candidate_revision, external_bounds),
        "legs": legs,
        "summary": summarize_legs(legs),
        "execution": "matrix plumbing: each leg runs the harness baseline+candidate scenario "
                     "pair for one model spec against the same candidate binary",
        "not_measured": [
            "idle daemon RSS: idle_rss_kib is null per leg; the harness measures CLI "
            "lifecycle peak RSS only",
        ],
    }
    report["exit_code"] = matrix_exit_code(report)
    output.mkdir(parents=True, exist_ok=True)
    (output / "matrix-report.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def run_matrix_cli(args: argparse.Namespace) -> None:
    """Matrix-mode entry: global gates (bounded environment, build settings,
    output handling, binary presence) run once here; the model-specific
    gates are per-leg preflight inside run_matrix."""
    if len(args.matrix) < 2:
        raise SystemExit("--matrix requires at least two model spec files; no inference started")
    if args.model_spec:
        raise SystemExit("--model-spec is the single-run spec surface; in matrix mode the spec "
                         "files are the --matrix arguments themselves; no inference started")
    if args.model_file:
        raise SystemExit("--model-file is not used in matrix mode; each leg's artifact is its "
                         "spec's model_path inside --model-cache; no inference started")
    if not all((args.baseline_bin, args.candidate_bin, args.model_cache)):
        raise SystemExit("matrix mode requires both binaries and --model-cache; no inference started")
    for binary in (args.baseline_bin, args.candidate_bin):
        if not binary.is_file():
            raise SystemExit(f"binary missing; no inference started: {binary}")
    specs = [load_model_spec(path) for path in args.matrix]
    external_bounds = require_bounded_environment(args.confirm_external_bounds)
    if args.baseline_build_settings.startswith("caller must") or args.candidate_build_settings.startswith("caller must"):
        raise SystemExit("provide exact --baseline-build-settings and --candidate-build-settings; no inference started")
    make_fixtures()
    print(f"Checked-in synthetic fixtures validated at {FIXTURE}")
    args.output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    deadline = started + MAX_RUN_SECONDS
    with tempfile.TemporaryDirectory(prefix="neurostrata-runtime-validation-matrix-") as temporary:
        workspace = Path(temporary)
        report = run_matrix(
            specs,
            baseline_bin=args.baseline_bin,
            candidate_bin=args.candidate_bin,
            candidate_revision=args.candidate_revision,
            model_cache=args.model_cache,
            output=args.output,
            workspace=workspace,
            deadline=deadline,
            external_bounds=external_bounds,
        )
        # Preserve per-leg diagnostic logs like the single-run path does; the
        # temporary workspaces (and their private caches/DBs) are discarded.
        logs_dir = args.output / "logs"
        logs_dir.mkdir(exist_ok=True)
        for index, (spec, leg) in enumerate(zip(specs, report["legs"])):
            for label in ("baseline", "candidate"):
                temporary_log = workspace / f"leg-{index}-{spec.name}" / label / "run.log"
                if temporary_log.is_file():
                    stored_log = logs_dir / f"leg-{index}-{spec.name}-{label}.log"
                    shutil.copyfile(temporary_log, stored_log)
                    if leg.get(label):
                        leg[label]["log"] = str(stored_log)
        (args.output / "matrix-report.json").write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"Matrix report recorded at {args.output / 'matrix-report.json'}")
    exit_code = matrix_exit_code(report)
    if exit_code:
        raise SystemExit(exit_code)


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
    parser.add_argument("--model-spec", type=Path,
                        help="path to a JSON model spec {name, repository, revision, sha256, "
                             "model_path} driving the single run's SHA gate and NEUROSTRATA_MODEL; "
                             "default is the built-in Nomic spec")
    parser.add_argument("--matrix", type=Path, nargs="+", metavar="SPEC",
                        help="matrix mode: paths to two or more JSON model spec files; runs the "
                             "full scenario per spec against the same candidate binary and "
                             "writes matrix-report.json")
    parser.add_argument(
        "--confirm-external-bounds",
        action="store_true",
        help="confirm an independent memory and temporary-disk quota bounds the whole run",
    )
    parser.add_argument("--output", type=Path, default=Path("runtime-validation-results"))
    args = parser.parse_args()
    if not args.baseline_bin and not args.candidate_bin and not args.matrix:
        make_fixtures()
        print(f"Checked-in synthetic fixtures validated at {FIXTURE}")
        print("Preparation only; no inference or measurements were run.")
        return
    if args.matrix:
        run_matrix_cli(args)
        return
    if not all((args.baseline_bin, args.candidate_bin, args.model_file, args.model_cache)):
        raise SystemExit("running requires both binaries, --model-file, and --model-cache; no inference started")
    spec = load_model_spec(args.model_spec) if args.model_spec else DEFAULT_MODEL_SPEC
    external_bounds = require_bounded_environment(args.confirm_external_bounds)
    check_model(args.model_file.resolve(), spec)
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
    print(f"Checked-in synthetic fixtures validated at {FIXTURE}")
    started = time.monotonic()
    deadline = started + MAX_RUN_SECONDS
    results = []
    with tempfile.TemporaryDirectory(prefix="neurostrata-runtime-validation-") as temporary:
        root = Path(temporary)
        if shutil.disk_usage(root).free < required_free:
            raise RuntimeError("temporary filesystem budget changed before model-cache copy")
        try:
            results.append(run_binary("baseline", args.baseline_bin, BASELINE, root,
                                      args.model_file, args.model_cache, deadline, spec=spec))
            results.append(run_binary("candidate", args.candidate_bin, args.candidate_revision,
                                      root, args.model_file, args.model_cache, deadline, spec=spec))
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
                     "external_bounds": external_bounds,
                     "binary_sha256": {"baseline": sha256(args.baseline_bin.resolve()),
                                       "candidate": sha256(args.candidate_bin.resolve())},
                     "commands": "tools/runtime-validation/runtime_validation.py; see per-binary run.log files",
                     "results": results,
                     "elapsed_seconds": round(time.monotonic() - started, 3),
                     "execution": "real production CLI with isolated copies of supplied local model cache; one synthetic run each",
                     "not_measured": ["idle daemon RSS", "steady-state in-process inference latency",
                                      "reader responsiveness during publication", "three-warm-run variance"]})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(f"Results recorded at {args.output / 'manifest.json'}")


if __name__ == "__main__":
    main()
