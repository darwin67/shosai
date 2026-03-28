.PHONY: dev lint fmt test

## Run the application in debug mode
dev:
	cargo run -p shosai-app

## Run clippy lints on the workspace
lint:
	cargo clippy --workspace --all-targets -- -D warnings

## Format all Rust source files
fmt:
	cargo fmt --all

## Run all tests
test:
	cargo test --workspace
