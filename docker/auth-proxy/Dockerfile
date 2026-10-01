# ironflow-auth-proxy image, used with K8sEphemeralProvider::auth_proxy.
#
# The proxy holds the Claude credential so the agent pods never do: it swaps
# the opaque per-step token of a pod for the real credential and relays to
# api.anthropic.com only. Deploy it with examples/k8s/sandbox/auth-proxy.yaml.
#
# Published image:
#
#   registry.gitlab.com/thomastartrau/ironflow/ironflow-auth-proxy:<version>
#
# <version> is the version of the ironflow-auth-proxy crate. The
# build-auth-proxy-image CI job publishes it on main once release-plz has tagged
# ironflow-auth-proxy-v<version>, and never rebuilds a published tag. There is
# no `latest` tag.
#
# Local build, from the repository root (the workspace is the build context,
# trimmed by .dockerignore):
#
#   docker build -f docker/auth-proxy/Dockerfile -t ironflow-auth-proxy:<version> .
#
# RUST_VERSION follows CI_RUST_IMAGE_TAG in .gitlab-ci.yml: CI passes it as a
# build argument, keep the default in step for local builds.
#
# Layout:
#   /ironflow-auth-proxy   the binary, the only file added to the base image.
#
# The runtime image is distroless and runs as uid 65532 (nonroot): no shell,
# no package manager, compatible with a read-only root filesystem.

ARG RUST_VERSION=1.94.1

FROM rust:${RUST_VERSION}-bookworm AS builder

WORKDIR /src
COPY . .
RUN cargo build --release --locked -p ironflow-auth-proxy

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /src/target/release/ironflow-auth-proxy /ironflow-auth-proxy

USER 65532
EXPOSE 8080
ENTRYPOINT ["/ironflow-auth-proxy"]
