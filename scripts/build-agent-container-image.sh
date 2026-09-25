#!/usr/bin/env sh
# The image this product runs an agent in, built from what is on this machine.
#
# A container environment needs an image holding the agent, and the way a
# person gets one is normally to pull it. On a machine that cannot reach a
# registry - this workspace's own container is one; its proxy refuses every
# registry blob host - there is still an honest way to have one: build
# it here, from the fixture agent this repository already builds and the few
# libraries it is linked against.
#
# The product's convention is what this script fills: the image runs the agent
# at /usr/local/bin/swem-agent. Any image that does that can be named with
# `swem workbench serve --container-image`.
#
# Usage: scripts/build-agent-container-image.sh [agent executable]
# Prints the image's pinned id (sha256:<64 hex>) and nothing else on stdout.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
agent=${1:-$root/target/debug/swem-hands-agent}

if ! command -v podman >/dev/null 2>&1; then
  echo "podman is not installed, so there is no container to build an image for" >&2
  exit 3
fi
if [ ! -x "$agent" ]; then
  echo "no agent at $agent; cargo build -p swem-host --bin swem-hands-agent" >&2
  exit 3
fi

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/usr/local/bin" "$staging/lib/x86_64-linux-gnu" "$staging/lib64" \
         "$staging/workspace" "$staging/home/swem" "$staging/tmp"
cp "$agent" "$staging/usr/local/bin/swem-agent"

# Everything the agent is linked against, by asking the loader rather than
# guessing: an image missing one library fails as "no such file or directory"
# on a path that exists, which reads like a product bug.
ldd "$agent" | while read -r line; do
  case $line in
    *"=>"*) library=$(printf '%s' "$line" | awk '{print $3}') ;;
    /*) library=$(printf '%s' "$line" | awk '{print $1}') ;;
    *) continue ;;
  esac
  [ -f "$library" ] || continue
  case $library in
    /lib64/*) cp -L "$library" "$staging/lib64/" ;;
    *) cp -L "$library" "$staging/lib/x86_64-linux-gnu/" ;;
  esac
done

image=$(tar -C "$staging" -c . | podman import \
  --change 'ENTRYPOINT ["/usr/local/bin/swem-agent"]' \
  - swem-gate-agent:latest)
# `podman import` prints `sha256:<64 hex>`, which is what the product accepts
# as a pinned image; a tag is not pinned and the product refuses one.
printf '%s\n' "$image"
