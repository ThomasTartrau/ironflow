# Official ironflow Claude Code runner image, used by
# K8sEphemeralProvider::sandboxed.
#
# Published as ironflow-claude-runner:<claude-code-version>-<n>. Never `latest`.
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

ARG CLAUDE_CODE_VERSION=2.0.14

FROM node:22-bookworm-slim

ARG CLAUDE_CODE_VERSION

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git curl \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
    && npm cache clean --force

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
