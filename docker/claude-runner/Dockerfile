# Official ironflow Claude Code runner image, used by
# K8sEphemeralProvider::sandboxed.
#
# Published as ironflow-claude-runner:<claude-code-version>-<n>, the content of
# IMAGE_TAG next to this file. Never `latest`. CLAUDE_CODE_VERSION is the
# <claude-code-version> part: see README.md for a local build.
#
# Layout:
#   /home/claude                 HOME of uid 10001. The sandboxed provider mounts
#                                an emptyDir here: the root filesystem is read-only.
#   /tmp                         emptyDir mounted by the sandboxed provider.
#   /etc/claude-code             managed-settings.json, read by Claude Code on
#                                Linux. A default is baked in; a managed-settings
#                                ConfigMap mounted by the provider replaces it.
#   /etc/ironflow/claude-profile Claude profile ConfigMap (CLAUDE.md, settings,
#                                agents), copied into ~/.claude before the agent
#                                starts because ~/.claude must stay writable.
#
# Both /etc directories are owned by root with mode 0755: the agent can read
# them, never change them.

# Pinned by digest so a rebuild of the same tag starts from the same base. To
# move to a newer node image, replace the digest and bump <n> in IMAGE_TAG.
FROM node:22-bookworm-slim@sha256:43ac6c60b8f89723f746e8a92ce91abd5017e627ce1ddfe4238355d3a30b772c

ARG CLAUDE_CODE_VERSION

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git curl \
    && rm -rf /var/lib/apt/lists/*

# Fails the build when the version is missing, or when the installed CLI
# reports another one.
RUN : "${CLAUDE_CODE_VERSION:?pass --build-arg CLAUDE_CODE_VERSION=<version>}" \
    && npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
    && npm cache clean --force \
    && test "$(claude --version | cut -d' ' -f1)" = "${CLAUDE_CODE_VERSION}"

RUN groupadd -g 10001 claude \
    && useradd -u 10001 -g 10001 -m -d /home/claude claude

# Marketplace neutralisation: no plugin marketplace, no hooks, no auto-update,
# no non-essential traffic. A mounted managed-settings ConfigMap replaces the
# baked file.
RUN mkdir -p /etc/claude-code /etc/ironflow/claude-profile \
    && printf '%s\n' '{"strictKnownMarketplaces": [], "disableAllHooks": true}' \
        > /etc/claude-code/managed-settings.json \
    && chown -R root:root /etc/claude-code /etc/ironflow \
    && chmod 0755 /etc/claude-code /etc/ironflow /etc/ironflow/claude-profile \
    && chmod 0644 /etc/claude-code/managed-settings.json

ENV CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    DISABLE_AUTOUPDATER=1 \
    HOME=/home/claude

USER 10001:10001
WORKDIR /home/claude
