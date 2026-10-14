default: check

check: check-rust check-go

check-rust:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace
    cargo test --workspace --all-features

check-go:
    cd cli && go vet ./...
    cd cli && go test -race ./...

build:
    cargo build --release
    cd cli && go build -o ../target/phantom ./cmd/phantom

# Builds the Complement image and runs Matrix's test suite against it; needs
# Docker and Go. Arguments go to `go test`, e.g. `just complement -run TestLogin`.
complement *args:
    complement/run.sh {{args}}

fonts:
    mkdir -p target/fonts
    curl -sL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/JetBrainsMono.tar.xz | tar -xJ -C target/fonts JetBrainsMonoNerdFontMono-Regular.ttf JetBrainsMonoNerdFontMono-Bold.ttf JetBrainsMonoNerdFontMono-Italic.ttf

media:
    test -f target/fonts/JetBrainsMonoNerdFontMono-Regular.ttf || just fonts
    cargo build -p phantom-server
    cd cli && go run ./cmd/phantom-media -server-bin ../target/debug/phantom-server -fonts ../target/fonts -out ../website/public/media

website:
    cd website && npm ci && npm run build
