//! The committed bundle must still be what its source builds.
//!
//! `workbench_shell.rs` embeds `web/apps-host/dist/workbench.js` with
//! `include_str!`, and nothing rebuilds it: there is no build script, and the
//! real-browser gates build only the untracked bridge. So a stale bundle
//! compiles, ships and passes every browser gate silently, against a shell
//! that no longer corresponds to its TypeScript. Until now freshness was
//! carried by a README sentence.
//!
//! The build goes to a temporary directory on purpose: the Vite config sets
//! `emptyOutDir`, so rebuilding in place would delete the untracked
//! `dist/apps-bridge.js` the browser gates depend on.
//!
//! Ignored by default because it needs Node.js, and asserts availability
//! rather than returning early - a gate that passes by skipping proves
//! nothing.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run(program: &str, args: &[&str], directory: &Path) -> Output {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(program);
        command
    };
    #[cfg(not(windows))]
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap_or_else(|error| panic!("run {program}: {error}"))
}

fn temporary(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-bundle-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Install once, then rebuild and compare the exact bytes.
fn assert_bundle_is_current(package: &Path, args: &[&str], built: &Path, committed: &Path) {
    // Existing node_modules is not evidence of the current lockfile.
    let install = run("npm", &["ci", "--no-audit", "--no-fund"], package);
    assert!(
        install.status.success(),
        "npm ci failed:\n{}",
        String::from_utf8_lossy(&install.stderr)
    );
    let rebuild = run("npx", args, package);
    assert!(
        rebuild.status.success(),
        "rebuild failed:\n{}\n{}",
        String::from_utf8_lossy(&rebuild.stdout),
        String::from_utf8_lossy(&rebuild.stderr)
    );
    let fresh = std::fs::read(built).expect("the rebuild produced no bundle");
    let shipped = std::fs::read(committed).expect("no committed bundle");
    assert!(
        fresh == shipped,
        "{} is stale: {} committed bytes, {} rebuilt. Run the package's build \
         and commit the result.",
        committed.display(),
        shipped.len(),
        fresh.len()
    );
}

#[test]
#[ignore = "bundle freshness: requires Node.js on PATH"]
fn the_committed_workbench_bundle_matches_its_source() {
    assert!(
        node_available(),
        "this gate needs Node.js on PATH; it does not pass by skipping"
    );
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/apps-host");
    let out = temporary("apps-host");
    let out_arg = format!("--outDir={}", out.display());
    assert_bundle_is_current(
        &package,
        &["vite", "build", &out_arg, "--emptyOutDir"],
        &out.join("workbench.js"),
        &package.join("dist/workbench.js"),
    );
}

/// The page a bot opens inside the messenger is committed as one file to
/// host anywhere (`web/mini-app/dist/index.html`), written by
/// `scripts/mini-app.mjs` from the source page with the palette and kit
/// inlined. The same composition is done here, so a stale copy fails without
/// Node.
#[test]
fn the_committed_mini_app_matches_its_source() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |path: &str| {
        std::fs::read_to_string(root.join(path)).unwrap_or_else(|_| panic!("read {path}"))
    };
    let expected = read("web/mini-app/index.html")
        .replace("__PALETTE_CSS__", &read("web/view-kit/palette.css"))
        .replace("__KIT_CSS__", &read("web/view-kit/kit.css"))
        .replace("__CHANNEL_ID__", "");
    let committed = read("web/mini-app/dist/index.html");
    assert!(
        committed == expected,
        "web/mini-app/dist/index.html is stale: run `node scripts/mini-app.mjs` and commit the result"
    );
}
