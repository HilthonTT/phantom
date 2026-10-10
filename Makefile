# The Justfile's recipes, for machines without just. Keep the two in step.

.PHONY: check check-rust check-go build fmt clean fonts media website help

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

FONT_DIR := target/fonts
NERD_FONT := https://github.com/ryanoasis/nerd-fonts/releases/latest/download/JetBrainsMono.tar.xz

## fonts: download the font the docs media is recorded with
fonts:
	mkdir -p $(FONT_DIR)
	curl -sL $(NERD_FONT) | tar -xJ -C $(FONT_DIR) JetBrainsMonoNerdFontMono-Regular.ttf JetBrainsMonoNerdFontMono-Bold.ttf JetBrainsMonoNerdFontMono-Italic.ttf

## media: record the docs GIFs and screenshots against a scratch server
media:
	@test -f $(FONT_DIR)/JetBrainsMonoNerdFontMono-Regular.ttf || $(MAKE) fonts
	cargo build -p phantom-server
	cd cli && go run ./cmd/phantom-media -server-bin ../target/debug/phantom-server -fonts ../$(FONT_DIR) -out ../website/public/media

## website: build the documentation site into website/out
website:
	cd website && npm ci && npm run build

## help: list the targets
help:
	@sed -n 's/^## //p' $(MAKEFILE_LIST)
