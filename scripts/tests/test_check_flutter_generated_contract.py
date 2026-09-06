import pathlib
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts/check_flutter_generated_contract.py"


class FlutterGeneratedContractTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.dart = self.root / "flutter/lib/src/rust"
        self.rust = self.root / "crates/shosai-flutter-bridge/src"
        self.dart.mkdir(parents=True)
        self.rust.mkdir(parents=True)
        (self.dart / "api.dart").write_text("// generated\n")
        (self.dart / "frb_generated.io.dart").write_text("// generated\n")
        (self.rust / "frb_generated.rs").write_text(
            "fn wire__crate__api__FlutterBridge_take_buffer_impl() {\n"
            "    DcoCodec;\n"
            "}\n"
        )
        (self.dart / "frb_generated.dart").write_text(
            "@override\n"
            "  Uint8List crateApiFlutterBridgeTakeBuffer() {\n"
            "    DcoCodec(dco_decode_list_prim_u_8_strict);\n"
            "  }\n"
        )

    def tearDown(self):
        self.temporary.cleanup()

    def run_checker(self):
        return subprocess.run(
            [sys.executable, CHECKER, self.root],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_accepts_expected_files_and_dco_transfer(self):
        self.assertEqual(self.run_checker().returncode, 0)

    def test_rejects_nested_stale_generated_file(self):
        stale = self.dart / "legacy/old.dart"
        stale.parent.mkdir()
        stale.write_text("// stale\n")

        result = self.run_checker()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("legacy/old.dart", result.stderr)

    def test_rejects_sse_take_buffer_transfer(self):
        generated = self.rust / "frb_generated.rs"
        generated.write_text(generated.read_text().replace("DcoCodec", "SseCodec"))

        result = self.run_checker()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must use DCO", result.stderr)


if __name__ == "__main__":
    unittest.main()
