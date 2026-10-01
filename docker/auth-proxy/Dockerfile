# ironflow-auth-proxy image, used with K8sEphemeralProvider::auth_proxy.
#
# The proxy holds the Claude credential so the agent pods never do: it swaps
# the opaque per-step token of a pod for the real credential and relays to
# api.anthropic.com only. Deploy it with examples/k8s/sandbox/auth-proxy.yaml.
#
# Build from the repository root (the whole workspace is the build context):
#
#   docker build -f docker/auth-proxy/Dockerfile -t ironflow-auth-proxy:<version> .
#
# Tag it with the crate version, never `latest`.
#
# Layout:
#   /ironflow-auth-proxy   the binary, the only file added to the base image.
#
# The runtime image is distroless and runs as uid 65532 (nonroot): no shell,
# no package manager, compatible with a read-only root filesystem.

FROM rust:1.94-bookworm AS builder

WORKDIR /src
COPY . .
RUN cargo build --release -p ironflow-auth-proxy

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /src/target/release/ironflow-auth-proxy /ironflow-auth-proxy

USER 65532
EXPOSE 8080
ENTRYPOINT ["/ironflow-auth-proxy"]
