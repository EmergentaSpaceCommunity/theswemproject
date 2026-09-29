#!/usr/bin/env sh
# The runner as one static program for a Linux host, for both
# architectures. It is what is put into an agent's machine, so it links
# against nothing the machine has to have.
#
# No cross compiler is needed: the runner is Rust alone and the toolchain's
# own linker links it. The targets are added when they are missing.
#
# Usage: scripts/build-runner.sh [out-directory]
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
out=${1:-"$root/target/runner"}
mkdir -p "$out"

for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
  if ! rustup target list --installed | grep -qx "$target"; then
    rustup target add "$target"
  fi
  RUSTFLAGS="-C linker=rust-lld -C link-self-contained=yes -C target-feature=+crt-static -C strip=symbols" \
    cargo build -p swem-runner --bin swem-runner --release --target "$target"
  arch=${target%%-*}
  cp "target/$target/release/swem-runner" "$out/swem-runner-linux-$arch"
  echo "$out/swem-runner-linux-$arch"
done
