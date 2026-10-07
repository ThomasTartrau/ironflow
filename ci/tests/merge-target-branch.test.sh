#!/bin/sh
# Black-box tests for ci/merge-target-branch.sh.
#
# Each case builds an isolated git fixture (a bare "origin" plus a CI-style
# checkout of the feature branch), runs the script with the CI variables it
# reads, and asserts the merge outcome. No network, no GitLab.
#
#   sh ci/tests/merge-target-branch.test.sh
set -u

SCRIPT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)/merge-target-branch.sh"
fail=0
tmp_dirs=""
trap 'for t in $tmp_dirs; do rm -rf "$t"; done' EXIT

check() {
  if [ "$1" = "$2" ]; then
    printf 'ok   - %s\n' "$3"
  else
    printf 'FAIL - %s (expected [%s], got [%s])\n' "$3" "$2" "$1"
    fail=1
  fi
}

new_dir() {
  _d="$(mktemp -d)"
  tmp_dirs="$tmp_dirs $_d"
  echo "$_d"
}

exists() { [ -e "$1" ] && echo yes || echo no; }

# git wrapper that never depends on the host's global identity: the script under
# test must supply its own for the merge commit, so the fixture stays neutral.
g() { git -c user.email=t@t -c user.name=t -c init.defaultBranch=main "$@"; }

# mk_origin <dir>: bare origin with a base commit `A` on main and a `feature`
# branch one commit ahead of it. Leaves a work clone at <dir>/seed.
mk_origin() {
  _o="$1"
  g init --bare -q "$_o/origin.git"
  g clone -q "file://$_o/origin.git" "$_o/seed" 2>/dev/null
  ( cd "$_o/seed" || exit 1
    echo base > base.txt
    g add base.txt && g commit -q -m A
    g push -q origin main
    g checkout -q -b feature
    echo feat > feat.txt
    g add feat.txt && g commit -q -m feat
    g push -q origin feature )
}

# advance <dir> <branch> <count>: pushes <count> new commits to <branch>, one
# file each, named <branch>-<n>.txt.
advance() {
  ( cd "$1/seed" || exit 1
    g checkout -q "$2"
    _i=1
    while [ "$_i" -le "$3" ]; do
      echo "c$_i" > "$2-$_i.txt" && g add "$2-$_i.txt" && g commit -q -m "$2 $_i"
      _i=$((_i + 1))
    done
    g push -q origin "$2" )
}

run_mr() {
  ( cd "$1" && CI_MERGE_REQUEST_IID=1 CI_MERGE_REQUEST_TARGET_BRANCH_NAME=main sh "$SCRIPT" )
}

# ---------------------------------------------------------------------------
# 1. No MR context: CI_MERGE_REQUEST_IID unset -> no-op (exit 0), HEAD untouched,
#    so the script is safe on a main-branch pipeline.
# ---------------------------------------------------------------------------
d="$(new_dir)"; mk_origin "$d"; advance "$d" main 1
g clone -q "file://$d/origin.git" --branch feature "$d/ci"
before="$( cd "$d/ci" && g rev-parse HEAD )"
( cd "$d/ci" && env -u CI_MERGE_REQUEST_IID sh "$SCRIPT" >/dev/null 2>&1 ); check "$?" "0" "no MR context exits 0"
check "$( cd "$d/ci" && g rev-parse HEAD )" "$before" "no MR context leaves HEAD untouched"
check "$(exists "$d/ci/main-1.txt")" "no" "no MR context merges nothing"

# ---------------------------------------------------------------------------
# 2. MR 10 commits behind main, no conflict, in a single-branch checkout (the
#    remote tracks only the MR branch, as on GitLab): the script merges main,
#    no rebase needed.
# ---------------------------------------------------------------------------
d="$(new_dir)"; mk_origin "$d"; advance "$d" main 10
g clone -q --single-branch "file://$d/origin.git" --branch feature "$d/ci"
run_mr "$d/ci" >/dev/null 2>&1; check "$?" "0" "10 commits behind: merge exits 0"
check "$(exists "$d/ci/main-10.txt")" "yes" "10 commits behind: main's commits are merged in"
check "$(exists "$d/ci/feat.txt")" "yes" "10 commits behind: the branch's own commit is kept"
check "$( cd "$d/ci" && g rev-parse HEAD^2 )" "$( cd "$d/seed" && g rev-parse main )" \
  "10 commits behind: HEAD merges the tip of main"

# ---------------------------------------------------------------------------
# 3. Conflict: both sides edit the same file -> the job fails, says what to do,
#    and leaves no merge in progress.
# ---------------------------------------------------------------------------
d="$(new_dir)"
g init --bare -q "$d/origin.git"
g clone -q "file://$d/origin.git" "$d/seed" 2>/dev/null
( cd "$d/seed" || exit 1
  echo shared-base > shared.txt && g add shared.txt && g commit -q -m A && g push -q origin main
  g checkout -q -b feature && echo shared-feat > shared.txt && g commit -q -am feat && g push -q origin feature
  g checkout -q main && echo shared-main > shared.txt && g commit -q -am B && g push -q origin main )
g clone -q "file://$d/origin.git" --branch feature "$d/ci"
out="$(run_mr "$d/ci" 2>&1)"; rc=$?
check "$( [ "$rc" -ne 0 ] && echo nonzero || echo zero )" "nonzero" "conflict exits non-zero"
check "$( echo "$out" | grep -c 'Merge or rebase origin/main locally, resolve the conflict' )" "1" \
  "conflict message says what to do"
check "$(exists "$d/ci/.git/MERGE_HEAD")" "no" "conflict leaves no merge in progress"

# ---------------------------------------------------------------------------
# 4. Shallow checkout at the CI depth (GIT_DEPTH: 50) whose merge base with main
#    sits below it: the branch is 60 commits deep and main 60 commits ahead. A
#    plain fetch + merge refuses "unrelated histories"; the script deepens the
#    clone and merges. The fetch before the unshallow leaves enough objects to
#    start a detached auto maintenance, the race the script guards against.
# ---------------------------------------------------------------------------
d="$(new_dir)"; mk_origin "$d"; advance "$d" feature 60; advance "$d" main 60
g clone -q --depth 50 "file://$d/origin.git" --branch feature "$d/ci"
run_mr "$d/ci" >/dev/null 2>&1; check "$?" "0" "shallow checkout: script deepens and merges (exit 0)"
check "$(exists "$d/ci/main-60.txt")" "yes" "shallow checkout: main's commits are merged in"

if [ "$fail" -eq 0 ]; then
  printf '\nAll merge-target-branch tests passed.\n'
else
  printf '\nmerge-target-branch tests FAILED.\n'
fi
exit "$fail"
