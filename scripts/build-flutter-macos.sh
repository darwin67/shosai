#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
  echo "error: the Flutter macOS host must be built on macOS" >&2
  exit 1
fi

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
flutter_executable="$(command -v flutter)"
flutter_root="$(cd "$(dirname "$flutter_executable")/.." && pwd -P)"
dart_executable="$(command -v dart)"
engine_relative_path="bin/cache/artifacts/engine/darwin-x64/FlutterMacOS.xcframework"
engine_source="$flutter_root/$engine_relative_path"

# Nix supplies a pinned Flutter SDK from its immutable store. Flutter copies the
# engine framework's read-only modes into the build directory, then lipo needs
# to create a temporary sibling while selecting the host architecture. Give
# only that SDK view and engine artifact a writable location.
if [[ ! -w "$engine_source" ]]; then
  writable_flutter_root="$repository_root/target/flutter-macos-sdk"
  resolved_engine_source="$(cd "$engine_source" && pwd -P)"

  if [[ -d "$writable_flutter_root" ]]; then
    find "$writable_flutter_root" -type d -exec chmod u+w {} +
  fi
  rm -rf "$writable_flutter_root"
  mkdir -p "$(dirname "$writable_flutter_root")"
  cp -R "$flutter_root" "$writable_flutter_root"
  find "$writable_flutter_root" -type d -exec chmod u+w {} +
  rm "$writable_flutter_root/$engine_relative_path" \
    "$writable_flutter_root/bin/dart"
  cp -R "$resolved_engine_source" \
    "$writable_flutter_root/$engine_relative_path"
  chmod -R u+w "$writable_flutter_root/$engine_relative_path"
  ln -s "$dart_executable" "$writable_flutter_root/bin/dart"

  flutter_root="$writable_flutter_root"
  flutter_executable="$writable_flutter_root/bin/flutter"
fi

export FLUTTER_ROOT="$flutter_root"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

cd "$repository_root/flutter"
exec env \
  -u SDKROOT \
  -u IPHONEOS_DEPLOYMENT_TARGET \
  -u TVOS_DEPLOYMENT_TARGET \
  -u WATCHOS_DEPLOYMENT_TARGET \
  -u XROS_DEPLOYMENT_TARGET \
  -u DRIVERKIT_DEPLOYMENT_TARGET \
  -u CC -u CXX -u CC_FOR_BUILD -u CXX_FOR_BUILD \
  -u AR -u AS -u LD -u LD_FOR_BUILD -u NM -u RANLIB -u STRIP \
  -u OBJCOPY -u OBJDUMP -u READELF \
  -u CFLAGS -u CXXFLAGS -u CPPFLAGS -u LDFLAGS \
  -u NIX_CFLAGS_COMPILE -u NIX_CFLAGS_COMPILE_FOR_BUILD \
  -u NIX_LDFLAGS -u NIX_LDFLAGS_FOR_BUILD \
  "$flutter_executable" build macos --debug
