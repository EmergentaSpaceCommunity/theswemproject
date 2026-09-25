#!/usr/bin/env sh
# The crate suites, which the product gate does not run.
#
# `scripts/gate.sh` proves the product through the front door and takes about
# ten minutes. It cannot see what a suite sees: a schema that drifted from the
# type beside it, a counter left behind, a control no walk touches. This runs
# in about a minute on a warm tree. Run it before you commit; run the gate
# after.
#
# The walks themselves are `#[ignore]`d, so they are listed as skipped here and
# run by the gate. That is the split, not a gap.
#
# The Workbench's node suite (`crates/swem-host/web/apps-host/test` and
# `web/view-kit`) goes first because it takes ten seconds and the rest take a
# minute; a fault it can see should not wait.
#
# Usage: scripts/suites.sh [extra cargo test arguments]
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

# The node suite refuses to pass by skipping, so it needs node and npm on
# PATH and says so here rather than a compile later.
if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
  echo "error: the Workbench's node suite needs node and npm on PATH." >&2
  exit 1
fi

# The node suite imports the page's `.ts` sources directly and relies on Node
# stripping the types, which Node does unflagged from 22.18 and behind a flag
# before that. So the flag is set here for the Node that needs it.
node_version=$(node --version | sed 's/^v//')
node_major=${node_version%%.*}
node_minor=${node_version#*.}; node_minor=${node_minor%%.*}
if [ "$node_major" -lt 22 ] || { [ "$node_major" -eq 22 ] && [ "$node_minor" -lt 18 ]; }; then
  echo "==> node $node_version strips types only behind a flag; setting it for the node suite"
  NODE_OPTIONS="${NODE_OPTIONS:-} --experimental-strip-types"
  export NODE_OPTIONS
fi

echo "==> the Workbench's node suite"
cargo test -p swem-host --test web_suite -- --ignored

echo "==> the crate suites"
exec cargo test -p swem-host -p swem-cli --no-fail-fast "$@"
