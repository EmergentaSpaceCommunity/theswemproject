#!/usr/bin/env sh
# Whether a product that has sqlx can link the harness in.
#
# A product built on the harness links it into a workspace of its own, and
# that workspace resolves dependencies afresh. Two things broke that once:
# two versions of SQLite's bindings (`rusqlite` against `sqlx`), and the
# harness's `process-wrap` against the one a newer `rmcp` wants. Cargo
# refuses the first and the second fails to compile. This makes a program
# that depends on the harness and on `sqlx` with SQLite and Postgres, with a
# fresh lock, and builds it. It takes a while the first time; run it when a
# dependency of the harness moves.
#
# Usage: scripts/links-beside-sqlx.sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/swem-links-XXXXXX")
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/src"
cp "$root/rust-toolchain.toml" "$scratch/"
cat > "$scratch/Cargo.toml" <<MANIFEST
[package]
name = "links-beside-sqlx"
version = "0.0.0"
edition = "2024"

[dependencies]
swem-host = { path = "$root/crates/swem-host" }
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "sqlite", "postgres", "macros", "migrate"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
MANIFEST
cat > "$scratch/src/main.rs" <<'PROGRAM'
fn main() {
    let _ = swem_host::product::DataRoot::at("nowhere");
    let _ = sqlx::sqlite::SqliteConnectOptions::new();
    println!("the harness links beside sqlx");
}
PROGRAM
echo "==> resolving and building a program with the harness and sqlx"
(cd "$scratch" && cargo run --quiet)
echo "==> one SQLite binding:"
(cd "$scratch" && cargo tree -i libsqlite3-sys --depth 0)
