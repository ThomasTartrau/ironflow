FROM rust:1.94.1-bookworm AS builder

ARG BINSTALL_VERSION=1.14.0
RUN case "$(uname -m)" in \
      aarch64|arm64) ARCH="aarch64-unknown-linux-musl" ;; \
      *)             ARCH="x86_64-unknown-linux-musl" ;; \
    esac && \
    curl -sSfL "https://github.com/cargo-bins/cargo-binstall/releases/download/v${BINSTALL_VERSION}/cargo-binstall-${ARCH}.tgz" \
    | tar -xz -C /usr/local/cargo/bin cargo-binstall

RUN cargo binstall --no-confirm \
    cargo-nextest@0.9.140 \
    cargo-llvm-cov@0.6.23 \
    cargo-audit@0.22.2 \
    sccache@0.10.0

ARG MDBOOK_VERSION=0.5.4
ARG MDBOOK_MERMAID_VERSION=0.17.1
RUN case "$(uname -m)" in \
      aarch64|arm64) ARCH="aarch64-unknown-linux-musl" ;; \
      *)             ARCH="x86_64-unknown-linux-gnu" ;; \
    esac && \
    curl -sSfL "https://github.com/rust-lang/mdBook/releases/download/v${MDBOOK_VERSION}/mdbook-v${MDBOOK_VERSION}-${ARCH}.tar.gz" | tar -xz -C /usr/local/cargo/bin mdbook && \
    curl -sSfL "https://github.com/badboy/mdbook-mermaid/releases/download/v${MDBOOK_MERMAID_VERSION}/mdbook-mermaid-v${MDBOOK_MERMAID_VERSION}-${ARCH}.tar.gz" | tar -xz -C /usr/local/cargo/bin mdbook-mermaid

FROM rust:1.94.1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    lld \
    postgresql-client \
    jq \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt

COPY --from=builder /usr/local/cargo/bin/cargo-binstall /usr/local/cargo/bin/
COPY --from=builder /usr/local/cargo/bin/cargo-nextest /usr/local/cargo/bin/
COPY --from=builder /usr/local/cargo/bin/cargo-llvm-cov /usr/local/cargo/bin/
COPY --from=builder /usr/local/cargo/bin/cargo-audit /usr/local/cargo/bin/
COPY --from=builder /usr/local/cargo/bin/sccache /usr/local/cargo/bin/
COPY --from=builder /usr/local/cargo/bin/mdbook /usr/local/cargo/bin/
COPY --from=builder /usr/local/cargo/bin/mdbook-mermaid /usr/local/cargo/bin/

