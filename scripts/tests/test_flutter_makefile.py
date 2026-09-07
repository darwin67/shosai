import os
import pathlib
import shutil
import stat
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class FlutterMakefileTest(unittest.TestCase):
    def setUp(self):
        self.make = shutil.which("make")
        if self.make is None:
            self.skipTest("make is not installed")
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name).resolve()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.root / "flutter").mkdir()
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copy2(ROOT / "Makefile", self.root / "Makefile")
        self._write_executable(scripts / "check-flutter-codegen.sh", "exit 0")
        self._write_executable(self.bin / "cargo", "exit 0")
        self.capture = self.root / "capture"
        self._write_executable(
            self.bin / "flutter",
            'printf "%s\\n" "$SHOSAI_PDFIUM_LIBRARY" "$@" > "$CAPTURE"',
        )

    def tearDown(self):
        self.temporary.cleanup()

    def _write_executable(self, path: pathlib.Path, body: str):
        path.write_text(f"#!/bin/sh\n{body}\n")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def test_flutter_tests_preserve_explicit_pdfium_without_dynamic_loader_path(self):
        environment = os.environ.copy()
        environment.pop("DYLD_LIBRARY_PATH", None)
        environment.update(
            {
                "PATH": f"{self.bin}:{environment['PATH']}",
                "CAPTURE": str(self.capture),
                "SHOSAI_PDFIUM_LIBRARY": "/pinned/libpdfium.dylib",
            }
        )

        result = subprocess.run(
            [self.make, "test-flutter"],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.capture.read_text().splitlines(),
            ["/pinned/libpdfium.dylib", "test"],
        )

    def test_aggregate_tests_remain_sequential_with_parallel_make(self):
        overlap = self.root / "overlap"
        lock = self.root / "command-lock"
        guarded_command = "\n".join(
            [
                'if ! mkdir "$LOCK"; then touch "$OVERLAP"; exit 42; fi',
                "sleep 0.05",
                'rmdir "$LOCK"',
            ]
        )
        self._write_executable(self.bin / "cargo", guarded_command)
        self._write_executable(self.bin / "flutter", guarded_command)
        self._write_executable(self.bin / "python3", guarded_command)
        self._write_executable(
            self.root / "scripts" / "check-frb-codegen.sh", guarded_command
        )
        self._write_executable(
            self.root / "scripts" / "check-flutter-codegen.sh", guarded_command
        )
        environment = os.environ.copy()
        environment.update(
            {
                "PATH": f"{self.bin}:{environment['PATH']}",
                "LOCK": str(lock),
                "OVERLAP": str(overlap),
            }
        )

        result = subprocess.run(
            [self.make, "-j8", "test"],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(overlap.exists(), "aggregate test commands overlapped")

    def test_macos_smoke_rebuilds_and_verifies_the_complete_app_signature(self):
        makefile = (ROOT / "Makefile").read_text()
        smoke = makefile.split("flutter-macos-smoke:", 1)[1].split(
            "\n## ", 1
        )[0]

        self.assertIn("@set -eu;", smoke)
        self.assertIn("$(MAKE) flutter-macos-debug", smoke)
        self.assertEqual(smoke.count("verify_signatures;"), 2)
        self.assertIn("codesign --verify --deep --strict --verbose=2", smoke)


if __name__ == "__main__":
    unittest.main()
