"""Synthetic process-group and bounded-output tests for the offline harness."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

import runtime_validation as harness


def helper_process(mode: str, path: str) -> None:
    if mode == "parent":
        child = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "grandchild", path])
        Path(path + ".pid").write_text(str(child.pid), encoding="utf-8")
        time.sleep(30)
    elif mode == "grandchild":
        time.sleep(1.5)
        Path(path).write_text("survived", encoding="utf-8")
        time.sleep(30)
    elif mode == "flood":
        chunk = b"x" * 32768
        while True:
            os.write(1, chunk)
    elif mode == "burst":
        os.write(1, b"x" * (128 * 1024))


class RuntimeValidationRunnerTests(unittest.TestCase):
    def test_fixture_source_is_external_and_manifest_generation_does_not_rewrite_it(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-fixture-") as temporary:
            root = Path(temporary) / "synthetic-project"
            shutil.copytree(Path(__file__).resolve().parent / "fixtures/synthetic-project", root)
            source = root / "src" / "core.rs"
            before = {
                path.relative_to(root): path.read_bytes()
                for path in root.rglob("*")
                if path.is_file()
            }
            previous_fixture = harness.FIXTURE
            try:
                harness.FIXTURE = root
                manifest = harness.make_fixtures()
            finally:
                harness.FIXTURE = previous_fixture

            after = {
                path.relative_to(root): path.read_bytes()
                for path in root.rglob("*")
                if path.is_file()
            }
            self.assertEqual(after, before, "the runner must not rewrite checked-in fixture inputs")
            self.assertEqual(
                manifest["fixture"]["corpus_files"],
                ["docs/fixture.md", "src/core.rs", "src/long_symbol.rs", "src/recovery.rs"],
            )
            self.assertEqual(
                manifest["corpus_sha256"]["src/core.rs"],
                harness.sha256(source),
            )

    def test_model_cache_rejects_file_and_directory_symlinks(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-cache-") as temporary:
            root = Path(temporary)
            content = root / "content"
            content.mkdir()
            (content / "weights.onnx").write_bytes(b"pinned artifact fixture")
            cache = root / "cache"
            cache.mkdir()
            (cache / "weights.onnx").symlink_to(content / "weights.onnx")
            with self.assertRaisesRegex(SystemExit, "must not contain symlinks"):
                harness.inventory_model_cache(cache)

            (cache / "weights.onnx").unlink()
            (cache / "artifact-dir").symlink_to(content, target_is_directory=True)
            with self.assertRaisesRegex(SystemExit, "must not contain symlinks"):
                harness.inventory_model_cache(cache)

            (cache / "artifact-dir").unlink()
            linked_root = root / "linked-cache"
            linked_root.symlink_to(cache, target_is_directory=True)
            with self.assertRaisesRegex(SystemExit, "cache root must not be a symlink"):
                harness.inventory_model_cache(linked_root)

    def test_timeout_kills_the_owned_process_group(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-timeout-") as temporary:
            root = Path(temporary)
            marker = root / "grandchild-survived"
            log = root / "command.log"
            with self.assertRaisesRegex(RuntimeError, "watchdog"):
                harness.run_command(
                    [sys.executable, str(Path(__file__).resolve()), "parent", str(marker)],
                    os.environ.copy(), 1, log, time.monotonic() + 5,
                )
            self.assertTrue((root / "grandchild-survived.pid").is_file())
            time.sleep(1.8)
            self.assertFalse(marker.exists(), "grandchild must not outlive its owned process group")

    def test_captured_output_is_bounded_and_overflow_kills_the_group(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-output-") as temporary:
            root = Path(temporary)
            log = root / "command.log"
            previous_cap = harness.MAX_CAPTURE_BYTES
            harness.MAX_CAPTURE_BYTES = 64 * 1024
            try:
                with self.assertRaisesRegex(RuntimeError, "output cap"):
                    harness.run_command(
                        [sys.executable, str(Path(__file__).resolve()), "flood", str(root)],
                        os.environ.copy(), 5, log, time.monotonic() + 8,
                    )
            finally:
                harness.MAX_CAPTURE_BYTES = previous_cap
            self.assertLessEqual(log.stat().st_size, 2 * 64 * 1024 + 2048)

    def test_fast_finite_output_overflow_is_rejected_after_process_exit(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-burst-") as temporary:
            root = Path(temporary)
            log = root / "command.log"
            previous_cap = harness.MAX_CAPTURE_BYTES
            harness.MAX_CAPTURE_BYTES = 64 * 1024
            try:
                with self.assertRaisesRegex(RuntimeError, "output cap"):
                    harness.run_command(
                        [sys.executable, str(Path(__file__).resolve()), "burst", str(root)],
                        os.environ.copy(), 5, log, time.monotonic() + 8,
                    )
            finally:
                harness.MAX_CAPTURE_BYTES = previous_cap
            self.assertLessEqual(log.stat().st_size, 2 * 64 * 1024 + 2048)


import hashlib
import json
from unittest import mock


STUB_BINARY_SOURCE = '''#!/usr/bin/env python3
import json
import os
import sys
import uuid
from pathlib import Path


def load_state():
    state_path = Path(os.environ["HOME"]) / "stub-state.json"
    if state_path.is_file():
        return json.loads(state_path.read_text(encoding="utf-8"))
    return {"records": [], "governs": {}, "files": [], "observed_models": []}


def save_state(state):
    (Path(os.environ["HOME"]) / "stub-state.json").write_text(json.dumps(state), encoding="utf-8")


def touch_db():
    home = Path(os.environ["HOME"])
    config = json.loads((home / ".config" / "neurostrata" / "config.json").read_text(encoding="utf-8"))
    db = Path(config["db_path"])
    db.parent.mkdir(parents=True, exist_ok=True)
    with db.open("ab") as stream:
        stream.write(b"s" * 128)


def main():
    state = load_state()
    state["observed_models"].append(os.environ.get("NEUROSTRATA_MODEL"))
    command = sys.argv[1]
    if command == "add":
        namespace, memory_type, content = sys.argv[2], sys.argv[3], sys.argv[4]
        record_id = str(uuid.uuid5(uuid.NAMESPACE_URL, namespace + "|" + memory_type + "|" + content))
        state["records"].append({"id": record_id, "namespace": namespace,
                                  "memory_type": memory_type, "content": content})
        touch_db()
        save_state(state)
        print(f"Memory added successfully with ID: {record_id}")
    elif command == "set-metadata":
        payload = json.loads(sys.argv[4])
        state["governs"][sys.argv[3]] = payload["governs"]
        touch_db()
        save_state(state)
        print("metadata updated")
    elif command == "ingest":
        namespace = sys.argv[3]
        src = Path.cwd() / "src"
        state["files"] = [f"{namespace}::src/{path.name}" for path in sorted(src.rglob("*")) if path.is_file()]
        touch_db()
        save_state(state)
        print("ingested")
    elif command == "export-graph":
        out = Path(sys.argv[2])
        nodes = list(state["records"]) + [{"id": file_id, "namespace": "synthetic-validation"}
                                          for file_id in state["files"]]
        links = [{"source": record_id, "type": "GOVERNS", "target": target}
                 for record_id, targets in state["governs"].items() for target in targets]
        out.write_text(json.dumps({"nodes": nodes, "links": links,
                                   "observed_models": state["observed_models"]}), encoding="utf-8")
        save_state(state)
        print("graph exported")
    else:
        print(f"unknown command {command}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
'''

FAILING_BINARY_SOURCE = "#!/bin/sh\nexit 1\n"


def write_stub_binary(root: Path, source: str = STUB_BINARY_SOURCE) -> Path:
    binary = root / "stub-binary"
    binary.write_text(source, encoding="utf-8")
    binary.chmod(0o755)
    return binary


def make_spec(name: str, content: bytes, model_path: str) -> "harness.ModelSpec":
    return harness.ModelSpec(
        name=name,
        repository="example.invalid/stub-models",
        revision="stub-revision",
        sha256=hashlib.sha256(content).hexdigest(),
        model_path=model_path,
    )


class ModelSpecTests(unittest.TestCase):
    def test_default_spec_equals_todays_constants(self) -> None:
        spec = harness.DEFAULT_MODEL_SPEC
        self.assertEqual(spec.name, "NomicEmbedTextV15")
        self.assertEqual(spec.repository, harness.MODEL_REPO)
        self.assertEqual(spec.revision, harness.MODEL_REVISION)
        self.assertEqual(spec.sha256, harness.MODEL_SHA256)
        self.assertEqual(spec.model_path, harness.MODEL_PATH)
        self.assertEqual(
            spec.to_dict(),
            {
                "name": "NomicEmbedTextV15",
                "repository": "nomic-ai/nomic-embed-text-v1.5",
                "revision": "0a3f1bfa99207a5448900a6daaaf6add10eaa058",
                "sha256": "147d5aa88c2101237358e17796cf3a227cead1ec304ec34b465bb08e9d952965",
                "model_path": "onnx/model.onnx",
            },
        )

    def test_bge_spec_of_record_is_unverified_where_the_tree_has_no_source(self) -> None:
        # In-tree sources pin only the BGE name (src/config.rs, src/embed.rs)
        # and its 384 width; repository/revision/sha256/model path have no
        # in-tree source and must stay UNVERIFIED rather than invented.
        spec = harness.BGE_SMALL_MODEL_SPEC
        self.assertEqual(spec.name, "BGESmallENV15Q")
        self.assertEqual(spec.repository, "UNVERIFIED")
        self.assertEqual(spec.revision, "UNVERIFIED")
        self.assertEqual(spec.sha256, "UNVERIFIED")
        self.assertEqual(spec.model_path, "UNVERIFIED")

    def test_supplied_spec_parses_from_a_json_file(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-spec-") as temporary:
            path = Path(temporary) / "spec.json"
            path.write_text(json.dumps({
                "name": "SuppliedModel",
                "repository": "example.invalid/supplied",
                "revision": "abc123",
                "sha256": "0" * 64,
                "model_path": "onnx/model_quantized.onnx",
            }), encoding="utf-8")
            spec = harness.load_model_spec(path)
            self.assertEqual(spec.name, "SuppliedModel")
            self.assertEqual(spec.repository, "example.invalid/supplied")
            self.assertEqual(spec.revision, "abc123")
            self.assertEqual(spec.sha256, "0" * 64)
            self.assertEqual(spec.model_path, "onnx/model_quantized.onnx")

    def test_supplied_spec_missing_a_field_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-spec-") as temporary:
            path = Path(temporary) / "spec.json"
            path.write_text(json.dumps({"name": "Incomplete"}), encoding="utf-8")
            with self.assertRaises(SystemExit):
                harness.load_model_spec(path)

    def test_wrong_sha_artifact_is_rejected_for_its_leg(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-sha-") as temporary:
            artifact = Path(temporary) / "model.onnx"
            artifact.write_bytes(b"real artifact bytes")
            wrong = make_spec("WrongShaModel", b"different bytes", "model.onnx")
            with self.assertRaisesRegex(SystemExit, "model SHA-256 mismatch"):
                harness.check_model(artifact, wrong)
            right = make_spec("RightShaModel", b"real artifact bytes", "model.onnx")
            harness.check_model(artifact, right)  # must not raise

    def test_default_check_model_message_is_unchanged(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-sha-") as temporary:
            artifact = Path(temporary) / "model.onnx"
            artifact.write_bytes(b"not the pinned artifact")
            with self.assertRaisesRegex(SystemExit, "model SHA-256 mismatch; no inference started: expected "
                                        + harness.MODEL_SHA256):
                harness.check_model(artifact)


class MatrixModeTests(unittest.TestCase):
    def run_stub_matrix(self, root: Path, specs, cache: Path, deadline: float | None = None,
                        disk_free: int = 2**40):
        # Stub legs perform no real inference and this sandbox's /tmp cannot
        # meet the per-leg disk arithmetic, so the free-space reading is
        # stubbed here; test_matrix_insufficient_disk_space_leg_skips passes
        # disk_free=0 to exercise the real gate.
        binary = write_stub_binary(root)
        output = root / "matrix-output"
        workspace = root / "workspace"
        usage = shutil.disk_usage(root)
        stub_usage = type(usage)(usage.total, usage.used, disk_free)
        with mock.patch.object(harness.shutil, "disk_usage", return_value=stub_usage):
            report = harness.run_matrix(
                specs,
                baseline_bin=binary,
                candidate_bin=binary,
                candidate_revision="stub-candidate-revision",
                model_cache=cache,
                output=output,
                workspace=workspace,
                deadline=deadline if deadline is not None else time.monotonic() + 600,
                external_bounds={
                    "operator_confirmed_external_memory_and_temporary_disk_quotas": True,
                    "containment": "test-only stub legs; no real inference",
                },
            )
        return binary, output, workspace, report

    def build_cache(self, root: Path, artifacts: dict[str, bytes]) -> Path:
        cache = root / "cache"
        cache.mkdir()
        for relative, content in artifacts.items():
            target = cache / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        return cache

    def test_matrix_report_schema_two_stub_legs_and_model_env(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-matrix-") as temporary:
            root = Path(temporary)
            cache = self.build_cache(root, {"a/model.onnx": b"model-a-bytes", "b/model.onnx": b"model-b-bytes"})
            specs = [
                make_spec("StubModelA", b"model-a-bytes", "a/model.onnx"),
                make_spec("StubModelB", b"model-b-bytes", "b/model.onnx"),
            ]
            binary, output, workspace, report = self.run_stub_matrix(root, specs, cache)

            written = json.loads((output / "matrix-report.json").read_text(encoding="utf-8"))
            self.assertEqual(written, report)
            self.assertEqual(report["summary"], {"total": 2, "passed": 2, "failed": 0, "skipped": 0})
            self.assertEqual(harness.matrix_exit_code(report), 0)
            self.assertEqual([leg["status"] for leg in report["legs"]], ["PASS", "PASS"])
            environment = report["environment"]
            self.assertIn("host", environment)
            self.assertIn("onnx_runtime_version", environment)
            self.assertEqual(
                [entry["name"] for entry in environment["artifact_provenance"]],
                ["StubModelA", "StubModelB"],
            )
            self.assertEqual(
                environment["binary_provenance"]["candidate"]["sha256"],
                harness.sha256(binary),
            )
            self.assertEqual(
                environment["binary_provenance"]["baseline"]["sha256"],
                environment["binary_provenance"]["candidate"]["sha256"],
            )
            for leg in report["legs"]:
                self.assertIsNone(leg["reason"])
                self.assertIn("peak_rss_kib", leg)
                self.assertIn("idle_rss_kib", leg)
                self.assertGreater(leg["db_wal_bytes"]["candidate"], 0)
                smoke = leg["fixture_smoke"]
                self.assertTrue(smoke["governs_edge_verified"])
                self.assertEqual(smoke["preservation_record_count"], 5)
                self.assertIn("synthetic-validation::src/long_symbol.rs", smoke["ingested_file_ids"])
                self.assertTrue(smoke["long_symbol_ingested"])
            observed = set()
            for graph_path in workspace.rglob("graph.json"):
                graph = json.loads(graph_path.read_text(encoding="utf-8"))
                observed.add(tuple(sorted(set(graph["observed_models"]))))
            self.assertEqual(observed, {("StubModelA",), ("StubModelB",)},
                             "each binary under test must receive its leg spec's NEUROSTRATA_MODEL")

    def test_matrix_missing_artifact_leg_skips_and_is_not_counted_as_passed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-matrix-") as temporary:
            root = Path(temporary)
            cache = self.build_cache(root, {"a/model.onnx": b"model-a-bytes"})
            specs = [
                make_spec("StubModelA", b"model-a-bytes", "a/model.onnx"),
                make_spec("MissingModel", b"ghost bytes", "ghost/model.onnx"),
            ]
            _, _, _, report = self.run_stub_matrix(root, specs, cache)
            legs = {leg["spec"]["name"]: leg for leg in report["legs"]}
            self.assertEqual(legs["StubModelA"]["status"], "PASS")
            missing = legs["MissingModel"]
            self.assertEqual(missing["status"], "SKIPPED")
            self.assertEqual(missing["reason"]["code"], "model_artifact_missing")
            self.assertTrue(missing["reason"]["message"])
            self.assertEqual(report["summary"], {"total": 2, "passed": 1, "failed": 0, "skipped": 1})
            self.assertEqual(harness.matrix_exit_code(report), 1,
                             "all matrix legs are required: a SKIPPED required leg is a nonzero exit")

    def test_matrix_wrong_sha_leg_skips_with_reason(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-matrix-") as temporary:
            root = Path(temporary)
            cache = self.build_cache(root, {"a/model.onnx": b"model-a-bytes"})
            specs = [make_spec("StubModelA", b"forged bytes", "a/model.onnx")]
            _, _, workspace, report = self.run_stub_matrix(root, specs, cache)
            leg = report["legs"][0]
            self.assertEqual(leg["status"], "SKIPPED")
            self.assertEqual(leg["reason"]["code"], "model_sha_mismatch")
            self.assertEqual(report["summary"]["passed"], 0)
            self.assertFalse(list(workspace.rglob("graph.json")), "a skipped leg must not run the scenario")

    def test_matrix_model_file_outside_cache_leg_skips(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-matrix-") as temporary:
            root = Path(temporary)
            cache = self.build_cache(root, {"a/model.onnx": b"model-a-bytes"})
            outside = root / "outside.onnx"
            outside.write_bytes(b"outside bytes")
            specs = [make_spec("EscapeModel", b"outside bytes", "../outside.onnx")]
            _, _, _, report = self.run_stub_matrix(root, specs, cache)
            leg = report["legs"][0]
            self.assertEqual(leg["status"], "SKIPPED")
            self.assertEqual(leg["reason"]["code"], "model_file_outside_cache")

    def test_matrix_insufficient_disk_space_leg_skips(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-matrix-") as temporary:
            root = Path(temporary)
            cache = self.build_cache(root, {"a/model.onnx": b"model-a-bytes"})
            specs = [make_spec("StubModelA", b"model-a-bytes", "a/model.onnx")]
            _, _, _, report = self.run_stub_matrix(root, specs, cache, disk_free=0)
            leg = report["legs"][0]
            self.assertEqual(leg["status"], "SKIPPED")
            self.assertEqual(leg["reason"]["code"], "insufficient_disk_space")

    def test_matrix_deadline_expiry_skips_legs_with_reason(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-matrix-") as temporary:
            root = Path(temporary)
            cache = self.build_cache(root, {"a/model.onnx": b"model-a-bytes"})
            specs = [make_spec("StubModelA", b"model-a-bytes", "a/model.onnx")]
            _, _, _, report = self.run_stub_matrix(root, specs, cache, deadline=time.monotonic() - 1)
            leg = report["legs"][0]
            self.assertEqual(leg["status"], "SKIPPED")
            self.assertEqual(leg["reason"]["code"], "deadline_expired")
            self.assertEqual(harness.matrix_exit_code(report), 1)

    def test_matrix_failing_binary_leg_fails_without_crashing_the_run(self) -> None:
        with tempfile.TemporaryDirectory(prefix="runtime-runner-matrix-") as temporary:
            root = Path(temporary)
            cache = self.build_cache(root, {"a/model.onnx": b"model-a-bytes", "b/model.onnx": b"model-b-bytes"})
            specs = [
                make_spec("StubModelA", b"model-a-bytes", "a/model.onnx"),
                make_spec("StubModelB", b"model-b-bytes", "b/model.onnx"),
            ]
            binary = write_stub_binary(root, FAILING_BINARY_SOURCE)
            usage = shutil.disk_usage(root)
            stub_usage = type(usage)(usage.total, usage.used, 2**40)
            with mock.patch.object(harness.shutil, "disk_usage", return_value=stub_usage):
                report = harness.run_matrix(
                    specs,
                    baseline_bin=binary,
                    candidate_bin=binary,
                    candidate_revision="stub-candidate-revision",
                    model_cache=cache,
                    output=root / "matrix-output",
                    workspace=root / "workspace",
                    deadline=time.monotonic() + 600,
                    external_bounds={"operator_confirmed_external_memory_and_temporary_disk_quotas": True},
                )
            self.assertEqual([leg["status"] for leg in report["legs"]], ["FAIL", "FAIL"])
            self.assertEqual(report["legs"][0]["reason"]["code"], "scenario_failed")
            self.assertEqual(report["summary"], {"total": 2, "passed": 0, "failed": 2, "skipped": 0})
            self.assertEqual(harness.matrix_exit_code(report), 1)

    def test_skipped_legs_are_never_counted_as_passed(self) -> None:
        summary = harness.summarize_legs([
            {"status": "PASS"}, {"status": "SKIPPED"}, {"status": "FAIL"}, {"status": "SKIPPED"},
        ])
        self.assertEqual(summary, {"total": 4, "passed": 1, "failed": 1, "skipped": 2})

    def test_cli_help_lists_matrix_and_model_spec(self) -> None:
        completed = subprocess.run(
            [sys.executable, str(Path(harness.__file__).resolve()), "--help"],
            capture_output=True, text=True, timeout=30,
        )
        self.assertEqual(completed.returncode, 0)
        self.assertIn("--matrix", completed.stdout)
        self.assertIn("--model-spec", completed.stdout)


if __name__ == "__main__" and len(sys.argv) > 1 and sys.argv[1] in {"parent", "grandchild", "flood", "burst"}:
    helper_process(sys.argv[1], sys.argv[2])
elif __name__ == "__main__":
    unittest.main()
