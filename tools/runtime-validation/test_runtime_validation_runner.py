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


if __name__ == "__main__" and len(sys.argv) > 1 and sys.argv[1] in {"parent", "grandchild", "flood", "burst"}:
    helper_process(sys.argv[1], sys.argv[2])
elif __name__ == "__main__":
    unittest.main()
