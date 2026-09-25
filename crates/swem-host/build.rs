//! Say what is missing before the compiler says something else.
//!
//! `workbench_shell.rs` embeds `web/apps-host/dist/apps-bridge.js`, which is
//! esbuild's output and is not in the repository - committing a bundle hides
//! the fact that its sources stopped building, which this workspace has been
//! caught by before. So a clean checkout cannot compile this crate until the
//! bundle is built, and without this script the person sees
//! `couldn't read .../apps-bridge.js`, which does not say what to run.

use std::path::Path;

fn main() {
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR")).join("web/apps-host/dist/apps-bridge.js");
    println!("cargo::rerun-if-changed={}", bundle.display());
    if !bundle.is_file() {
        println!(
            "cargo::error=swem-host embeds web/apps-host/dist/apps-bridge.js, which is not built yet. \
             Run it once: (cd crates/swem-host/web/apps-host && npm ci && npm run build). \
             scripts/gate.sh does this and the rest of the gate's prerequisites."
        );
    }
}
