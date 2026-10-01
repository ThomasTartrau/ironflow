#!/usr/bin/env bash
# Decide whether the container image of a crate must be published. Prints, as a
# GitLab dotenv report, the image tag (the crate version) and whether to push it:
#
#   IMAGE_TAG=<version>
#   IMAGE_PUBLISH=true|false
#
# IMAGE_PUBLISH is true only when the git tag <crate>-v<version> exists on
# origin (release-plz released that version) and the registry holds no image
# under that tag: a published tag never changes content. Any answer other than
# "found" or "not found", from git or from the registry, fails the script
# instead of risking a republish. Logs go to stderr. Run by CI; usable locally.
#
#   scripts/crate-image-tag.sh <Cargo.toml> <image repository>
#   scripts/crate-image-tag.sh ironflow-auth-proxy/Cargo.toml \
#     registry.gitlab.com/thomastartrau/ironflow/ironflow-auth-proxy
#
# Registry credentials are read from CI_REGISTRY_USER / CI_REGISTRY_PASSWORD
# when set, anonymous otherwise (enough for a public project).
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <Cargo.toml> <image repository>" >&2
  exit 1
fi
MANIFEST="$1"
IMAGE="$2"

PACKAGE="$(cargo metadata --no-deps --format-version 1 --manifest-path "$MANIFEST" \
  | jq -r --arg manifest "$(cd "$(dirname "$MANIFEST")" && pwd -P)/Cargo.toml" \
    '.packages[] | select(.manifest_path == $manifest) | "\(.name) \(.version)"')"
if [ -z "$PACKAGE" ]; then
  echo "no package found in $MANIFEST" >&2
  exit 1
fi
NAME="${PACKAGE% *}"
VERSION="${PACKAGE#* }"
GIT_TAG="${NAME}-v${VERSION}"

publish() {
  echo "IMAGE_TAG=${VERSION}"
  echo "IMAGE_PUBLISH=$1"
}

# The tag is created by release-plz-release earlier in the same pipeline, after
# this checkout: ask the remote, not the local refs.
set +e
git ls-remote --exit-code --tags origin "refs/tags/${GIT_TAG}" > /dev/null
STATUS=$?
set -e
case "$STATUS" in
  0) ;;
  2)
    echo "git tag ${GIT_TAG} not found on origin: ${NAME} ${VERSION} is not released, nothing to publish" >&2
    publish false
    exit 0
    ;;
  *)
    echo "git ls-remote failed (exit ${STATUS}) while looking for ${GIT_TAG}" >&2
    exit 1
    ;;
esac

HOST="${IMAGE%%/*}"
REPOSITORY="${IMAGE#*/}"
MANIFEST_URL="https://${HOST}/v2/${REPOSITORY}/manifests/${VERSION}"
ACCEPT="application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# HEAD on the manifest. A registry with token auth answers 401 with a Bearer
# challenge naming the token endpoint: fetch a pull token there and ask again.
CODE="$(curl -sS -I -o /dev/null -D "$WORK/headers" -w '%{http_code}' \
  -H "Accept: ${ACCEPT}" "$MANIFEST_URL")"
if [ "$CODE" = "401" ]; then
  CHALLENGE="$(tr -d '\r' < "$WORK/headers" | sed -n 's/^[Ww][Ww][Ww]-[Aa]uthenticate: *Bearer //p')"
  REALM="$(echo "$CHALLENGE" | sed -n 's/.*realm="\([^"]*\)".*/\1/p')"
  SERVICE="$(echo "$CHALLENGE" | sed -n 's/.*service="\([^"]*\)".*/\1/p')"
  SCOPE="$(echo "$CHALLENGE" | sed -n 's/.*scope="\([^"]*\)".*/\1/p')"
  if [ -z "$REALM" ]; then
    echo "${HOST} answered 401 without a Bearer challenge" >&2
    exit 1
  fi
  CREDENTIALS=()
  if [ -n "${CI_REGISTRY_USER:-}" ]; then
    CREDENTIALS=(-u "${CI_REGISTRY_USER}:${CI_REGISTRY_PASSWORD:-}")
  fi
  TOKEN="$(curl -fsS -G "${CREDENTIALS[@]+"${CREDENTIALS[@]}"}" \
    --data-urlencode "service=${SERVICE}" --data-urlencode "scope=${SCOPE}" "$REALM" \
    | jq -r '.token // .access_token // empty')"
  if [ -z "$TOKEN" ]; then
    echo "no pull token returned by ${REALM}" >&2
    exit 1
  fi
  CODE="$(curl -sS -I -o /dev/null -D "$WORK/headers" -w '%{http_code}' \
    -H "Accept: ${ACCEPT}" -H "Authorization: Bearer ${TOKEN}" "$MANIFEST_URL")"
fi

# A 404 from anything else than a registry (a proxy, a web page) must not read
# as "image absent".
if ! tr -d '\r' < "$WORK/headers" | grep -qi '^docker-distribution-api-version: *registry/2\.0'; then
  echo "${HOST} answered HTTP ${CODE} without Docker-Distribution-API-Version: not a registry" >&2
  exit 1
fi

case "$CODE" in
  200)
    echo "${IMAGE}:${VERSION} already exists: a published tag is never rebuilt, nothing to publish" >&2
    publish false
    ;;
  404)
    echo "${GIT_TAG} is released and ${IMAGE}:${VERSION} does not exist: publish it" >&2
    publish true
    ;;
  *)
    echo "unexpected HTTP ${CODE} from ${MANIFEST_URL}" >&2
    exit 1
    ;;
esac
