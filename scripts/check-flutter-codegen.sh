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

if [[ "$status" -ne 0 ]]; then
  echo "Flutter bindings are stale; run 'make flutter-codegen' and commit the result." >&2
fi
exit "$status"
