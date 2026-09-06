#!/usr/bin/env python3
"""Validate generated Flutter bridge files and the bounded raster codec."""

from __future__ import annotations

import pathlib
import re
import sys

EXPECTED_DART_FILES = {
    pathlib.PurePosixPath("api.dart"),
    pathlib.PurePosixPath("frb_generated.dart"),
    pathlib.PurePosixPath("frb_generated.io.dart"),
}


def check(root: pathlib.Path) -> list[str]:
    errors: list[str] = []
    dart_root = root / "flutter/lib/src/rust"
    actual_files = {
        pathlib.PurePosixPath(path.relative_to(dart_root).as_posix())
        for path in dart_root.rglob("*")
        if path.is_file()
    }
    if actual_files != EXPECTED_DART_FILES:
        errors.append(
            "unexpected generated Dart files: "
            f"expected {sorted(map(str, EXPECTED_DART_FILES))}, "
            f"found {sorted(map(str, actual_files))}"
        )

    rust = (root / "crates/shosai-flutter-bridge/src/frb_generated.rs").read_text()
    dart = (dart_root / "frb_generated.dart").read_text()
    rust_entry = re.search(
        r"fn wire__crate__api__FlutterBridge_take_buffer_impl\(.*?\n}\n",
        rust,
        re.DOTALL,
    )
    dart_entry = re.search(
        r"@override\n  Uint8List crateApiFlutterBridgeTakeBuffer\(.*?\n  }\n",
        dart,
        re.DOTALL,
    )
    if rust_entry is None or "DcoCodec" not in rust_entry.group():
        errors.append("Rust take_buffer binding must use DCO")
    if dart_entry is None or not all(
        value in dart_entry.group()
        for value in ("DcoCodec", "dco_decode_list_prim_u_8_strict")
    ):
        errors.append("Dart takeBuffer binding must decode a DCO Uint8List")
    return errors


if __name__ == "__main__":
    repository = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    failures = check(repository)
    if failures:
        print("\n".join(failures), file=sys.stderr)
        raise SystemExit(1)
