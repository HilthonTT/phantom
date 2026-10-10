# The Justfile's recipes, for machines without just. Keep the two in step.

.PHONY: check check-rust check-go build fmt clean help

## check: everything CI runs, for both languages (the default)
check: check-rust check-go

## check-rust: rustfmt, clippy with -D warnings and the tests, with and without all features
check-rust:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy --workspace --all-targets --all-features -- -D warnings
	cargo test --workspace
	cargo test --workspace --all-features

## check-go: go vet and the race-enabled tests of the console
check-go:
	cd cli && go vet ./...
	cd cli && go test -race ./...

## build: the release server, and the console at target/phantom
build:
	cargo build --release
	cd cli && go build -o ../target/phantom ./cmd/phantom

## fmt: format both halves in place
fmt:
	cargo fmt --all
	cd cli && gofmt -w .

## clean: remove the build output of both halves
clean:
	cargo clean
	cd cli && go clean ./...

## help: list the targets
help:
	@sed -n 's/^## //p' $(MAKEFILE_LIST)
