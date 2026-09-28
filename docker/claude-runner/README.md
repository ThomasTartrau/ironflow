# ironflow-claude-runner

Container image running the Claude Code CLI for
`K8sEphemeralProvider::sandboxed`. Built by the `build-claude-runner-image` CI
job and pushed to the project registry.

## Tags

`ironflow-claude-runner:<claude-code-version>-<n>`, for instance
`ironflow-claude-runner:2.0.14-1`.

- `<claude-code-version>` is the `@anthropic-ai/claude-code` npm version baked
  in (`RUNNER_CLAUDE_CODE_VERSION` in `.gitlab-ci.yml`).
- `<n>` is the image revision for that version: bump it to rebuild the same
  Claude Code version after a change to the Dockerfile
  (`CLAUDE_RUNNER_IMAGE_TAG` in `.gitlab-ci.yml`).

There is no `latest` tag. Pin the full tag in the provider.

## User

The image runs as uid/gid `10001` (`claude`), matching the provider's
`SANDBOX_UID`. `HOME` is `/home/claude`.

## Mount points

| Path | Owner | Content |
|------|-------|---------|
| `/home/claude` | provider (`emptyDir`, 1Gi by default) | `HOME`; `~/.claude` is written here |
| `/tmp` | provider (`emptyDir`, 512Mi by default) | `TMPDIR` |
| `/etc/claude-code` | root, 0755 | `managed-settings.json`; a baked default, replaced by a managed-settings preset ConfigMap |
| `/etc/ironflow/claude-profile` | root, 0755 | Claude profile ConfigMap, copied into `~/.claude` at startup |

The root filesystem is read-only in the sandbox: anything the agent writes
goes to `HOME`, `/tmp` or a volume.

## Baked defaults

- `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` and `DISABLE_AUTOUPDATER=1`.
- `/etc/claude-code/managed-settings.json`:
  `{"strictKnownMarketplaces": [], "disableAllHooks": true}` (no plugin
  marketplace, no hooks).

## Local build

```sh
docker build --build-arg CLAUDE_CODE_VERSION=2.0.14 \
  -t ironflow-claude-runner:2.0.14-1 docker/claude-runner
```
