#!/usr/bin/env bash
# Seeds the empty workspace with the fixture crate, as if it were the user's repository.
set -euo pipefail
"$(dirname "${BASH_SOURCE[0]}")/../_shared/seed.sh" "$(dirname "${BASH_SOURCE[0]}")/fixture"
