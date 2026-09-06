import os
import pathlib
import shutil
import stat
import subprocess
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
BUILDER = ROOT / "scripts/build-flutter-macos.sh"


class FlutterMacosBuildTest(unittest.TestCase):
    SCRUBBED_VARIABLES = (
        "SDKROOT",
        "IPHONEOS_DEPLOYMENT_TARGET",
        "TVOS_DEPLOYMENT_TARGET",
        "WATCHOS_DEPLOYMENT_TARGET",
        "XROS_DEPLOYMENT_TARGET",
        "DRIVERKIT_DEPLOYMENT_TARGET",
        "CC",
        "CXX",
        "CC_FOR_BUILD",
        "CXX_FOR_BUILD",
        "AR",
        "AS",
        "LD",
        "LD_FOR_BUILD",
        "NM",
        "RANLIB",
        "STRIP",
        "OBJCOPY",
        "OBJDUMP",
        "READELF",
        "CFLAGS",
        "CXXFLAGS",
        "CPPFLAGS",
        "LDFLAGS",
        "NIX_CFLAGS_COMPILE",
        "NIX_CFLAGS_COMPILE_FOR_BUILD",
        "NIX_LDFLAGS",
        "NIX_LDFLAGS_FOR_BUILD",
    )

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name).resolve()
        self.repository = self.root / "repository"
        self.scripts = self.repository / "scripts"
        self.flutter = self.repository / "flutter"
        self.sdk = self.root / "flutter-sdk"
        self.engine_link = (
            self.sdk
            / "bin/cache/artifacts/engine/darwin-x64/FlutterMacOS.xcframework"
        )
        self.engine = self.root / "engine-source/FlutterMacOS.xcframework"
        self.scripts.mkdir(parents=True)
        self.flutter.mkdir()
        self.engine.mkdir(parents=True)
        self.engine_link.parent.mkdir(parents=True)
        self.engine_link.symlink_to(self.engine)
        shutil.copy2(BUILDER, self.scripts / BUILDER.name)

        self.capture = self.root / "capture"
        scrubbed_values = " \\\n  ".join(
            f'"${{{variable}-unset}}"' for variable in self.SCRUBBED_VARIABLES
        )
        self._write_executable(
            self.sdk / "bin/flutter",
            f"""#!/usr/bin/env bash
printf '%s\n' \\
  "$PWD" "$FLUTTER_ROOT" "$MACOSX_DEPLOYMENT_TARGET" "$PATH" \\
  {scrubbed_values} \\
  "$SHOSAI_PDFIUM_LIBRARY" "$CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER" \\
  "$@" > "$CAPTURE"
""",
        )
        self._write_executable(self.sdk / "bin/dart", "#!/bin/sh\nexit 0\n")
        self._write_executable(
            self.sdk / "bin/uname",
            "#!/bin/sh\nprintf 'Darwin\\n'\n",
        )

    def tearDown(self):
        self.temporary.cleanup()

    def _write_executable(self, path: pathlib.Path, body: str):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)
        path.chmod(path.stat().st_mode | stat.S_IXUSR)

    def _run_builder(self, **overrides):
        environment = os.environ.copy()
        environment.pop("MACOSX_DEPLOYMENT_TARGET", None)
        environment.update(
            {variable: f"polluted-{variable}" for variable in self.SCRUBBED_VARIABLES}
        )
        environment.update(
            {
                "PATH": f"{self.sdk / 'bin'}:{environment['PATH']}",
                "CAPTURE": str(self.capture),
                "SHOSAI_PDFIUM_LIBRARY": "/pinned/libpdfium.dylib",
                "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER": "/usr/bin/clang",
            }
        )
        environment.update(overrides)
        return subprocess.run(
            [self.scripts / BUILDER.name],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_scrubs_compiler_environment_and_preserves_pinned_tools(self):
        result = self._run_builder()

        self.assertEqual(result.returncode, 0, result.stderr)
        values = self.capture.read_text().splitlines()
        self.assertEqual(values[0], str(self.flutter))
        self.assertEqual(values[1], str(self.sdk))
        self.assertEqual(values[2], "13.0")
        self.assertEqual(
            values[3],
            f"/usr/bin:/bin:/usr/sbin:/sbin:{self.sdk / 'bin'}:{os.environ['PATH']}",
        )
        scrubbed_end = 4 + len(self.SCRUBBED_VARIABLES)
        self.assertEqual(
            values[4:scrubbed_end], ["unset"] * len(self.SCRUBBED_VARIABLES)
        )
        self.assertEqual(values[scrubbed_end], "/pinned/libpdfium.dylib")
        self.assertEqual(values[scrubbed_end + 1], "/usr/bin/clang")
        self.assertEqual(values[scrubbed_end + 2 :], ["build", "macos", "--debug"])

    def test_preserves_explicit_deployment_target(self):
        result = self._run_builder(MACOSX_DEPLOYMENT_TARGET="14.2")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.capture.read_text().splitlines()[2], "14.2")

    def test_read_only_engine_uses_writable_sdk_view_and_keeps_symlinks(self):
        versions = self.engine / "macos-arm64/FlutterMacOS.framework/Versions"
        (versions / "A").mkdir(parents=True)
        (versions / "Current").symlink_to("A")
        nested_file = versions / "A/FlutterMacOS"
        nested_file.write_text("immutable engine")
        source_mode = nested_file.stat().st_mode
        for path in [self.sdk, self.engine, *self.sdk.rglob("*"), *self.engine.rglob("*")]:
            if not path.is_symlink():
                path.chmod(path.stat().st_mode & ~stat.S_IWUSR)
        if os.access(self.engine, os.W_OK):
            self.skipTest("filesystem permissions do not model a read-only Nix store")

        result = self._run_builder()

        self.assertEqual(result.returncode, 0, result.stderr)
        writable_sdk = self.repository / "target/flutter-macos-sdk"
        values = self.capture.read_text().splitlines()
        self.assertEqual(values[1], str(writable_sdk))
        copied_engine = (
            writable_sdk
            / "bin/cache/artifacts/engine/darwin-x64/FlutterMacOS.xcframework"
        )
        self.assertTrue(os.access(copied_engine, os.W_OK))
        self.assertTrue(os.access(copied_engine / "macos-arm64", os.W_OK))
        self.assertTrue(
            os.access(
                copied_engine
                / "macos-arm64/FlutterMacOS.framework/Versions/A/FlutterMacOS",
                os.W_OK,
            )
        )
        current = (
            copied_engine / "macos-arm64/FlutterMacOS.framework/Versions/Current"
        )
        self.assertTrue(current.is_symlink())
        self.assertEqual(os.readlink(current), "A")
        self.assertTrue((writable_sdk / "bin/dart").is_symlink())
        self.assertEqual(
            (writable_sdk / "bin/dart").resolve(),
            (self.sdk / "bin/dart").resolve(),
        )
        self.assertTrue(self.engine_link.is_symlink())
        self.assertEqual(nested_file.read_text(), "immutable engine")
        self.assertEqual(nested_file.stat().st_mode, source_mode & ~stat.S_IWUSR)

        second_result = self._run_builder()
        self.assertEqual(second_result.returncode, 0, second_result.stderr)


if __name__ == "__main__":
    unittest.main()
