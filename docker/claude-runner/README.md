# ironflow-claude-runner

Container image running the Claude Code CLI for
`K8sEphemeralProvider::sandboxed`. Built by the `build-claude-runner-image` CI
job and pushed to
`registry.gitlab.com/thomastartrau/ironflow/ironflow-claude-runner`.

## Tags

`ironflow-claude-runner:<claude-code-version>-<n>`, read from
[`IMAGE_TAG`](IMAGE_TAG), the only place the version is written.

- `<claude-code-version>` is the `@anthropic-ai/claude-code` npm version baked
  in. Take the one behind the `stable` dist-tag
  (`npm view @anthropic-ai/claude-code dist-tags.stable`), with `-1`.
- `<n>` is the image revision for that version: bump it after any other change
  to the Dockerfile, the base image digest included.

The build fails if the installed CLI does not report `<claude-code-version>`.
On merge requests, the `check-claude-runner-image` job builds the image
without pushing it, and fails when the Dockerfile changes without a new
`IMAGE_TAG`, so a merge never overwrites a published tag.

There is no `latest` tag. Pin the full tag in the provider.

## Base image

`node:22-bookworm-slim`, pinned by digest. To update it, take the digest of
the multi-arch index and bump `<n>`:

```sh
docker buildx imagetools inspect node:22-bookworm-slim --format '{{json .Manifest.Digest}}'
```

## User

The image runs as uid/gid `10001` (`claude`), matching the provider's
`SANDBOX_UID`. `HOME` is `/home/claude`.

## Mount points

| Path | Owner | Content |
|------|-------|---------|
| `/home/claude` | provider (`emptyDir`, 1Gi by default) | `HOME`; `~/.claude` is written here |
| `/tmp` | provider (`emptyDir`, 512Mi by default) | `TMPDIR` |
| `/etc/claude-code` | root, 0755 | `managed-settings.json`; a baked default, replaced by a managed-settings preset ConfigMap |
| `/etc/ironflow/claude-profile` | root, 0755 | One read-only mount per Claude profile ConfigMap (`<n>/`), the keys of the n-th copied into `~/.claude/<subdir>` at startup |

The root filesystem is read-only in the sandbox: anything the agent writes
goes to `HOME`, `/tmp` or a volume.

## Baked defaults

- `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` and `DISABLE_AUTOUPDATER=1`.
- `/etc/claude-code/managed-settings.json`:
  `{"strictKnownMarketplaces": [], "disableAllHooks": true}` (no plugin
  marketplace, no hooks).

## Local build

```sh
tag="$(cat docker/claude-runner/IMAGE_TAG)"
docker build --build-arg CLAUDE_CODE_VERSION="${tag%-*}" \
  -t "ironflow-claude-runner:${tag}" docker/claude-runner
```
