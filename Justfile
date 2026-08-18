default: check

check: check-rust check-go

check-rust:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

check-go:
    cd cli && go vet ./...
    cd cli && go test -race ./...

build:
    cargo build --release
    cd cli && go build -o ../target/phantom ./cmd/phantom
