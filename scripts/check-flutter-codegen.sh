#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

mkdir -p "$temporary/dart"
cp -R "$root/flutter/lib/src/rust/." "$temporary/dart/"
cp "$root/crates/shosai-flutter-bridge/src/frb_generated.rs" "$temporary/frb_generated.rs"

(cd "$root/flutter" && flutter_rust_bridge_codegen generate)
(cd "$root" && cargo fmt --package shosai-flutter-bridge)

status=0
diff -ru "$temporary/dart" "$root/flutter/lib/src/rust" || status=1
diff -u \
  "$temporary/frb_generated.rs" \
  "$root/crates/shosai-flutter-bridge/src/frb_generated.rs" || status=1

expected_files=$'api.dart\nfrb_generated.dart\nfrb_generated.io.dart'
actual_files="$(find "$root/flutter/lib/src/rust" -maxdepth 1 -type f -exec basename {} \; | sort)"
if [[ "$actual_files" != "$expected_files" ]]; then
  echo "Unexpected Flutter generated file set:" >&2
  diff -u <(printf '%s\n' "$expected_files") <(printf '%s\n' "$actual_files") || true
  status=1
fi

python3 - "$root" <<'PY' || status=1
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
rust = (root / "crates/shosai-flutter-bridge/src/frb_generated.rs").read_text()
dart = (root / "flutter/lib/src/rust/frb_generated.dart").read_text()

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
    raise SystemExit("Rust take_buffer binding must use DCO")
if dart_entry is None or not all(
    value in dart_entry.group()
    for value in ("DcoCodec", "dco_decode_list_prim_u_8_strict")
):
    raise SystemExit("Dart takeBuffer binding must decode a DCO Uint8List")
PY

if [[ "$status" -ne 0 ]]; then
  echo "Flutter bindings are stale; run 'make flutter-codegen' and commit the result." >&2
fi
exit "$status"
