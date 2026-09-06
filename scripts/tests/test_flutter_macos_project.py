import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
PROJECT = ROOT / "flutter/macos/Runner.xcodeproj/project.pbxproj"
BRIDGE_PHASE = "53484F534149425249444745"
FLUTTER_EMBED_PHASE = "3399D490228B24CF009A79C7"


class FlutterMacosProjectTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.project = PROJECT.read_text()

    def test_bridge_bundle_precedes_flutter_embed(self):
        runner = re.search(
            r"33CC10EC2044A3C60003C045 /\* Runner \*/ = \{.*?"
            r"buildPhases = \((.*?)\);",
            self.project,
            re.DOTALL,
        )
        self.assertIsNotNone(runner)
        phases = runner.group(1)

        self.assertLess(phases.index(BRIDGE_PHASE), phases.index(FLUTTER_EMBED_PHASE))

    def test_bridge_declares_signed_bundle_outputs(self):
        phase = re.search(
            rf"{BRIDGE_PHASE} /\* Build and Bundle Rust Bridge \*/ = \{{(.*?)\n\t\t\}};",
            self.project,
            re.DOTALL,
        )
        self.assertIsNotNone(phase)
        definition = phase.group(1)
        outputs = re.search(r"outputPaths = \((.*?)\);", definition, re.DOTALL)
        self.assertIsNotNone(outputs)

        self.assertIn("alwaysOutOfDate = 1;", definition)
        self.assertIn(
            '"$(TARGET_BUILD_DIR)/$(FRAMEWORKS_FOLDER_PATH)/'
            'libshosai_flutter_bridge.dylib"',
            outputs.group(1),
        )
        self.assertIn(
            '"$(TARGET_BUILD_DIR)/$(FRAMEWORKS_FOLDER_PATH)/libpdfium.dylib"',
            outputs.group(1),
        )


if __name__ == "__main__":
    unittest.main()
