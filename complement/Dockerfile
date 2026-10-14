# The phantom image Complement (github.com/matrix-org/complement) tests.
#
# Complement's contract for a homeserver image: plain HTTP for clients on 8008,
# HTTPS for federation on 8448, the server name taken from `SERVER_NAME` at
# run time, and a federation certificate signed by the CA Complement mounts at
# /complement/ca. `entrypoint.sh` does the last two; `complement.toml` the rest.
#
# Build from the repository root:
#
#   docker build -f complement/Dockerfile -t complement-phantom .
#
# `just complement` builds it and runs the suite against it.

FROM debian:bookworm AS builder

# The toolchain the workspace builds with, plus what the bundled RocksDB needs:
# a C++ compiler and libclang for bindgen.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates clang curl libclang-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
# No default toolchain: `rust-toolchain.toml` names the one to install, so the
# image cannot drift from what CI builds with.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain none

WORKDIR /src
COPY . .

# A release build for speed under test, without the release profile's LTO and
# single codegen unit, which double the build time for a few percent that
# Complement's timeouts do not need. The cache mounts keep the registry and
# the target directory across local rebuilds.
ENV CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p phantom-server \
    && cp target/release/phantom-server /usr/local/bin/phantom-server

FROM debian:bookworm-slim

# openssl issues the federation certificate at startup.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates openssl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/phantom-server /usr/local/bin/phantom-server
COPY complement/complement.toml /complement/phantom.toml
COPY complement/entrypoint.sh /usr/local/bin/entrypoint.sh

RUN mkdir -p /var/lib/phantom /run/phantom

EXPOSE 8008 8448

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
