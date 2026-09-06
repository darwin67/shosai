import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]


class FlutterDesktopTitleTest(unittest.TestCase):
    def test_linux_header_and_compositor_title_are_empty(self):
        runner = (ROOT / "flutter/linux/runner/my_application.cc").read_text()

        self.assertNotIn("gtk_header_bar_set_title", runner)
        self.assertRegex(
            runner,
            re.compile(
                r"gtk_window_set_titlebar\(window, GTK_WIDGET\(header_bar\)\);\s*"
                r"}\s*gtk_window_set_title\(window, \"\"\);"
            ),
        )

    def test_macos_hides_the_window_title(self):
        window = (ROOT / "flutter/macos/Runner/MainFlutterWindow.swift").read_text()

        self.assertIn("titleVisibility = .hidden", window)


if __name__ == "__main__":
    unittest.main()
