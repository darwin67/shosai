#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
temporary="$(mktemp -d)"
temporary="$(cd "$temporary" && pwd -P)"
rust_root="$temporary/rust/crates/shosai-core"
rust_output="$rust_root/src/frb_codegen_smoke_generated.rs"
trap 'rm -rf "$temporary"' EXIT

mkdir -p "$rust_root"
cp "$root/Cargo.lock" "$temporary/rust/Cargo.lock"
awk '
  /^members = \[$/ { print "members = [\"crates/shosai-core\"]"; in_members = 1; next }
  in_members && /^\]$/ { in_members = 0; next }
  !in_members { print }
' "$root/Cargo.toml" >"$temporary/rust/Cargo.toml"
sed '/^\[dependencies\]$/a flutter_rust_bridge = "=2.11.1"' \
  "$root/crates/shosai-core/Cargo.toml" >"$rust_root/Cargo.toml"
cp -R "$root/crates/shosai-core/src" "$rust_root/src"

mkdir -p "$temporary/dart/lib/generated"
cat >"$temporary/dart/pubspec.yaml" <<'YAML'
name: shosai_bridge_codegen_check
environment:
  sdk: ">=3.0.0 <4.0.0"
dependencies:
  flutter_rust_bridge: any
  freezed_annotation: any
dev_dependencies:
  build_runner: any
  freezed: any
YAML
cat >"$temporary/dart/pubspec.lock" <<'YAML'
packages:
  build_runner:
    dependency: "direct dev"
    description: {name: build_runner, url: "https://pub.dev"}
    source: hosted
    version: "2.4.0"
  flutter_rust_bridge:
    dependency: "direct main"
    description: {name: flutter_rust_bridge, url: "https://pub.dev"}
    source: hosted
    version: "2.11.1"
  freezed:
    dependency: "direct dev"
    description: {name: freezed, url: "https://pub.dev"}
    source: hosted
    version: "2.5.0"
  freezed_annotation:
    dependency: "direct main"
    description: {name: freezed_annotation, url: "https://pub.dev"}
    source: hosted
    version: "2.4.0"
sdks:
  dart: ">=3.0.0 <4.0.0"
YAML

flutter_rust_bridge_codegen generate \
  --rust-root "$rust_root" \
  --rust-input crate::bridge \
  --dart-root "$temporary/dart" \
  --dart-output "$temporary/dart/lib/generated" \
  --rust-output "$rust_output" \
  --no-deps-check \
  --no-auto-upgrade-dependency \
  --no-build-runner \
  --no-web \
  --no-dart-format \
  --no-dart-fix \
  --no-add-mod-to-lib \
  --stop-on-error

test -s "$rust_output"
test -n "$(find "$temporary/dart/lib/generated" -type f -print -quit)"
for declaration in openDocument renderPage selectionSurface takeBuffer releaseDocument releaseBuffer cancel; do
  grep -Rq "$declaration" "$temporary/dart/lib/generated"
done
