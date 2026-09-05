.PHONY: dev reset lint lint-flutter fmt test test-flutter test-scripts check-frb flutter-codegen check-flutter flutter-dev flutter-release check-rfds changelog next-version

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

## Generate bindings and run Flutter tests
test-flutter: flutter-codegen
	cd flutter && flutter test

## Run tests for repository scripts
test-scripts:
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
		-s benchmarks/epub-page-turn/2026-08-17/tests
	PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
		-s scripts/tests

## Verify that the core bridge API is accepted by flutter_rust_bridge codegen
check-frb:
	@./scripts/check-frb-codegen.sh

## Generate Rust/Dart bindings
flutter-codegen:
	cd flutter && flutter_rust_bridge_codegen generate
	cargo fmt --package shosai-flutter-bridge

## Generate bindings and validate the Flutter host
check-flutter: flutter-codegen
	cd flutter && dart format --output=none --set-exit-if-changed lib test
	cd flutter && flutter analyze
	cd flutter && flutter test

## Run the Linux Flutter host in debug mode
flutter-dev: flutter-codegen
	cd flutter && flutter run -d linux

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
