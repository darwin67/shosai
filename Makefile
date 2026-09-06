.PHONY: dev reset lint lint-flutter fmt test test-flutter test-scripts check-frb check-flutter-codegen flutter-codegen check-flutter flutter-dev flutter-linux-debug flutter-macos-debug flutter-macos-smoke flutter-release check-rfds changelog next-version

DEV_DATA_HOME := $(CURDIR)/target

## Run the application in debug mode
dev:
	XDG_DATA_HOME="$(DEV_DATA_HOME)" SHOSAI_DEV_BUILD=1 cargo run -p shosai-app

## Delete development-only Shosai data and development-owned managed copies
reset:
	@XDG_DATA_HOME="$(DEV_DATA_HOME)" python3 scripts/reset-local-data.py

## Run clippy lints on the workspace
lint:
	cargo clippy --workspace --all-targets -- -D warnings
	$(MAKE) lint-flutter

## Run Flutter static analysis
lint-flutter:
	cd flutter && flutter analyze

## Format all Rust and Dart source files
fmt:
	cargo fmt --all
	cd flutter && dart format lib test

## Run all tests
test:
	cargo test --workspace --no-fail-fast
	$(MAKE) test-scripts
	$(MAKE) check-frb
	$(MAKE) test-flutter

## Verify bindings and run Flutter unit and native bridge tests
test-flutter: check-flutter-codegen
	cargo build --package shosai-flutter-bridge
	cd flutter && \
		if [ "$$(uname -s)" = Darwin ]; then \
			export SHOSAI_PDFIUM_LIBRARY="$${DYLD_LIBRARY_PATH%%:*}/libpdfium.dylib"; \
		fi; \
		flutter test

## Run tests for repository scripts
test-scripts:
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
		-s benchmarks/epub-page-turn/2026-08-17/tests
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
		-s scripts/tests

## Verify that the core bridge API is accepted by flutter_rust_bridge codegen
check-frb:
	@./scripts/check-frb-codegen.sh

## Verify checked-in Flutter bindings match the bridge API
check-flutter-codegen:
	@./scripts/check-flutter-codegen.sh

## Generate Rust/Dart bindings
flutter-codegen:
	cd flutter && flutter_rust_bridge_codegen generate
	cargo fmt --package shosai-flutter-bridge

## Validate generated bindings and the Flutter host
check-flutter: check-flutter-codegen
	cd flutter && dart format --output=none --set-exit-if-changed lib test
	cd flutter && flutter analyze
	$(MAKE) test-flutter

## Run the Linux Flutter host in debug mode
flutter-dev: flutter-codegen
	cd flutter && flutter run -d linux

## Build the Linux Flutter host in debug mode
flutter-linux-debug: check-flutter-codegen
	cd flutter && flutter build linux --debug

## Build the macOS Flutter host in debug mode
flutter-macos-debug: check-flutter-codegen
	cd flutter && env -i \
		HOME="$$HOME" \
		PATH="$$PATH" \
		TMPDIR="$${TMPDIR:-/tmp}" \
		DEVELOPER_DIR="$$DEVELOPER_DIR" \
		SDKROOT="$$(/usr/bin/xcrun --sdk macosx --show-sdk-path)" \
		MACOSX_DEPLOYMENT_TARGET=13.0 \
		DYLD_LIBRARY_PATH="$$DYLD_LIBRARY_PATH" \
		CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$$CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER" \
		CC_aarch64_apple_darwin="$$CC_aarch64_apple_darwin" \
		CXX_aarch64_apple_darwin="$$CXX_aarch64_apple_darwin" \
		flutter build macos --debug

## Launch the packaged macOS host and verify that it remains running
flutter-macos-smoke: flutter-macos-debug
	@log="$$(mktemp)"; \
		app="flutter/build/macos/Build/Products/Debug/shosai_flutter.app/Contents/MacOS/shosai_flutter"; \
		"$$app" >"$$log" 2>&1 & pid=$$!; \
		trap 'kill "$$pid" 2>/dev/null || true; wait "$$pid" 2>/dev/null || true; rm -f "$$log"' EXIT; \
		sleep 5; \
		if ! kill -0 "$$pid" 2>/dev/null; then \
			cat "$$log"; \
			exit 1; \
		fi

## Build the Linux Flutter host in release mode
flutter-release: flutter-codegen
	cd flutter && flutter build linux --release

## Validate RFD sources and the checker regression fixtures
check-rfds:
	@./scripts/check-rfd-status.sh
	@./scripts/check-rfd-status-test.sh

## Regenerate CHANGELOG.md from conventional commits
changelog:
	git cliff -o CHANGELOG.md

## Print the next semantic version inferred from conventional commits
next-version:
	git cliff --bumped-version
