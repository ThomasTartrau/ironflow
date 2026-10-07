#!/bin/sh
# Merge the MR's target branch into the current checkout before the CI checks run.
#
# GitLab CI (without the Premium "merged results pipelines") checks out the branch
# tip alone, so an MR pipeline validates a state that is not the post-merge one.
# Two MRs each green on their own can break main once both are merged: two
# migrations sharing a version shipped that way in ironflow-store 2.46.0 (#190).
# Merging the target branch first makes every job validate the code as it will be
# once merged. An MR behind main never needs a manual rebase: rerunning its
# pipeline merges the current main.
#
# Called first in the before_script of the MR jobs. Safe to call unconditionally:
# outside an MR pipeline (main, tag) it is a no-op.
set -eu

# No MR context (main-branch pipeline, tag): nothing to merge.
if [ -z "${CI_MERGE_REQUEST_IID:-}" ]; then
  echo "merge-target-branch: not an MR pipeline, skipping."
  exit 0
fi

TARGET="${CI_MERGE_REQUEST_TARGET_BRANCH_NAME:-main}"
echo "merge-target-branch: merging origin/$TARGET into $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo HEAD)"

# Explicit refspec: GitLab (like a single-branch clone) sets up the remote to track
# only the MR branch, so a bare `git fetch origin main` updates FETCH_HEAD but never
# creates refs/remotes/origin/main, and `git merge origin/main` fails with "not
# something we can merge". Forcing the destination ref makes origin/$TARGET exist.
REFSPEC="+$TARGET:refs/remotes/origin/$TARGET"

# Auto maintenance is off: a fetch starts a detached `git maintenance` (or gc) that
# rewrites .git/shallow while the next fetch runs, and the unshallow then dies on
# "shallow file has changed since we read it", leaving the merge base out of reach.
fetch() { git -c maintenance.auto=false -c gc.auto=0 fetch -q "$@"; }

fetch origin "$REFSPEC"

# GIT_DEPTH truncates history: the merge base with the target branch can sit below
# the clone depth, and `git merge` then refuses "unrelated histories". Deepen to
# full history so the merge base exists.
if [ -f "$(git rev-parse --git-dir)/shallow" ]; then
  fetch --unshallow origin "$REFSPEC"
fi

# Identity is passed inline rather than written to the config: the merge commit
# needs an author, but the runner's git config must stay untouched.
if ! git -c user.email="ci@ironflow.invalid" -c user.name="GitLab CI" \
       merge --no-edit "origin/$TARGET"; then
  git merge --abort 2>/dev/null || true
  echo "merge-target-branch: this branch conflicts with origin/$TARGET." >&2
  echo "Merge or rebase origin/$TARGET locally, resolve the conflict, push, and the pipeline reruns." >&2
  exit 1
fi
