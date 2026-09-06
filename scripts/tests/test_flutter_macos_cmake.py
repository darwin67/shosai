import os
import pathlib
import shutil
import stat
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / "flutter/rust_builder/macos"


class FlutterMacosCmakeTest(unittest.TestCase):
    def setUp(self):
        self.cmake = shutil.which("cmake")
        if self.cmake is None:
            self.skipTest("cmake is not installed")
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name).resolve()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.selected_pdfium = self.root / "selected/libpdfium.dylib"
        self.selected_pdfium.parent.mkdir()
        self.selected_pdfium.write_text("fixture")
        self.capture = self.root / "lipo-input"
        self._write_executable("cargo", "#!/bin/sh\nexit 0\n")
        self._write_executable(
            "lipo",
            "#!/bin/sh\nprintf '%s' \"$2\" > \"$CAPTURE\"\nuname -m\n",
        )

    def tearDown(self):
        self.temporary.cleanup()

    def _write_executable(self, name: str, body: str):
        path = self.bin / name
        path.write_text(body)
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def _configure(
        self,
        pdfium: pathlib.Path | None,
        *arguments: str,
        include_dynamic_loader_path: bool = True,
    ):
        environment = os.environ.copy()
        environment.update(
            {
                "PATH": f"{self.bin}:{environment['PATH']}",
                "CAPTURE": str(self.capture),
            }
        )
        if pdfium is None:
            environment.pop("SHOSAI_PDFIUM_LIBRARY", None)
        else:
            environment["SHOSAI_PDFIUM_LIBRARY"] = str(pdfium)
        if include_dynamic_loader_path:
            environment["DYLD_LIBRARY_PATH"] = str(
                self.root / "wrong-library-directory"
            )
        else:
            environment.pop("DYLD_LIBRARY_PATH", None)
        return subprocess.run(
            [self.cmake, "-S", SOURCE, "-B", self.root / "build", *arguments],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_prefers_explicit_pdfium_library_over_dynamic_loader_path(self):
        result = self._configure(self.selected_pdfium)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.capture.read_text(), str(self.selected_pdfium))

    def test_uses_explicit_pdfium_when_dynamic_loader_path_is_absent(self):
        result = self._configure(
            self.selected_pdfium, include_dynamic_loader_path=False
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.capture.read_text(), str(self.selected_pdfium))

    def test_explicit_pdfium_replaces_cached_library(self):
        cached_pdfium = self.root / "cached/libpdfium.dylib"
        cached_pdfium.parent.mkdir()
        cached_pdfium.write_text("old fixture")
        first_result = self._configure(
            None, f"-DPDFIUM_LIBRARY={cached_pdfium}"
        )
        self.assertEqual(first_result.returncode, 0, first_result.stderr)
        self.assertEqual(self.capture.read_text(), str(cached_pdfium))

        second_result = self._configure(self.selected_pdfium)

        self.assertEqual(second_result.returncode, 0, second_result.stderr)
        self.assertEqual(self.capture.read_text(), str(self.selected_pdfium))

    def test_rejects_missing_explicit_pdfium_library(self):
        missing = self.root / "missing/libpdfium.dylib"

        result = self._configure(missing)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("PDFium library does not exist:", result.stderr)
        self.assertIn(str(missing), result.stderr)


if __name__ == "__main__":
    unittest.main()
