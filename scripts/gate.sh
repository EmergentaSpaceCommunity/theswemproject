#!/usr/bin/env sh
# The product gate: everything a person can do with SWEM, walked through the
# front door on the real binary and a real browser.
#
# It builds what `cargo test` does not - the Workbench's web bundle, the
# editor's ACP client, the fixture agents - and then runs the Workbench's own
# browser walks and the product's front-door walks. Ten minutes on a warm
# tree. Run `scripts/suites.sh` first: it takes a minute and catches what a
# walk cannot see.
#
# Usage: scripts/gate.sh [extra cargo test arguments]
#   scripts/gate.sh a_person_with_nothing_installs_an_agent   one walk by name
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

# Before anything is built: a lint failure should cost seconds, not the ten
# minutes the walks take.
echo "==> the lint, which the walks cannot see"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

echo "==> the Workbench's web bundle"
(cd crates/swem-host/web/apps-host && npm ci && npm run build)

echo "==> the editor's own ACP client, which the editor-door walk is proved by"
(cd crates/swem-cli/tests/acp-client && npm ci)

echo "==> the fixture agents and servers the walks use"
cargo build -p swem-host --bin swem-hands-agent --bin swem-echo-agent \
  --bin swem-mcp-echo --bin swem-mcp-observer-fixture --bin swem-mcp-apps-fixture \
  --bin swem-mcp-taker-fixture --bin swem-channel-fixture --bin swem-telegram-api-fixture

echo "==> the Telegram channel, which ships beside the product binary"
cargo build -p swem-channel-telegram

# The container walk builds its own image from the fixture agent, and skips
# cleanly when this machine has no Podman: see
# scripts/build-agent-container-image.sh.

echo "==> the product binary, and the example product with the harness built in"
cargo build -p swem-cli --bin swem
cargo build -p swem-host --example built_in

# A browser refuses to start as root without `--no-sandbox`, and says so in
# Chromium's own words about a sandbox. Containers run as root, so the walks
# fail there on a tree that is perfectly fine. The flag is for a container,
# not for a person's machine, so it is set only when this runs as root.
if [ -z "${SWEM_BROWSER_NO_SANDBOX:-}" ] && [ "$(id -u)" = "0" ]; then
  SWEM_BROWSER_NO_SANDBOX=1
  export SWEM_BROWSER_NO_SANDBOX
fi

if [ -z "${SWEM_BROWSER:-}" ]; then
  for candidate in /opt/pw-browsers/chromium /usr/bin/google-chrome \
                   /usr/bin/chromium /usr/bin/chromium-browser \
                   /usr/bin/microsoft-edge \
                   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"; do
    if [ -x "$candidate" ]; then
      SWEM_BROWSER=$candidate
      export SWEM_BROWSER
      echo "==> the browser the walks drive: $SWEM_BROWSER"
      break
    fi
  done
fi
if [ -z "${SWEM_BROWSER:-}" ]; then
  echo "note: no browser found. Set SWEM_BROWSER to a Chromium or Chrome" >&2
  echo "      binary, or the browser walks will skip." >&2
fi

# The Workbench's own `#[ignore]`d walks: the shell, the Apps host, sessions,
# authentication, the committed web bundle against its source. What is left
# out is left out because a machine cannot honestly run it, and each one says
# so itself when called:
#   live_* / *_a_live_*  a vendor agent, a podman lease, or the network
#   the registry test    needs the official ACP Agent Registry
#   web_suite            `scripts/suites.sh` runs it, in ten seconds
#   manual debugging     a server for a person to look at, not a walk
# `live_` and `a_live_` carry their underscores on purpose: `--skip live` is
# a substring match and "delivers" contains "live".
skips="--skip live_ --skip a_live_ --skip web_suite --skip manual_debugging"
skips="$skips --skip the_managed_kimi_product_resolves_against_the_official_registry"
echo "==> the Workbench's own walks"
# shellcheck disable=SC2086
cargo test -p swem-host --tests --no-fail-fast -- --ignored $skips

echo "==> the product gate"
exec cargo test -p swem-cli --test product_front_door -- --ignored "$@"
