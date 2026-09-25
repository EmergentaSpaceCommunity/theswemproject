//! The Workbench web package's own suite, reached from a gate.
//!
//! `theme.test.mjs` checks that the host emits the standard MCP Apps style
//! vocabulary whole and nothing besides, that it validates against the official
//! SDK schema, and that both shipped palettes declare every name and keep text
//! readable. `web/view-kit` adds the other half: that the shared kit expresses
//! every value as one of those variables, so a surface cannot introduce a
//! twelfth radius. Until this gate existed nothing ran either: `cargo test`
//! never invoked npm and the repository has no CI, so the README sentence
//! claiming the mapping "is checked" was true only of a command a human might
//! type.
//!
//! Ignored by default because it needs Node.js. It asserts availability rather
//! than returning early: a suite that passes by skipping is exactly the
//! silent-return live test the delete-first rules forbid.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Web packages this gate runs, relative to this crate, with whether the
/// package needs its lockfile installed first. The view kit lives at the
/// repository root because the host and the domain modules are peers that both
/// consume it; it has no dependencies, so it runs without an install.
const PACKAGES: &[(&str, bool)] = &[("web/apps-host", true), ("../../web/view-kit", false)];

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn npm(args: &[&str], directory: &Path) -> Output {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.arg("/C").arg("npm");
        command
    };
    #[cfg(not(windows))]
    let mut command = Command::new("npm");
    command
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run npm")
}

/// Tests the runner reported as passed, over either reporter node may pick:
/// TAP prints `# pass N`, the spec reporter prints a marked `pass N`.
fn reported_passes(output: &str) -> Option<u32> {
    output.lines().rev().find_map(|line| {
        let line = line.trim();
        let line = line.trim_start_matches(['#', '\u{2139}']).trim_start();
        line.strip_prefix("pass ")?.trim().parse().ok()
    })
}

#[test]
#[ignore = "web suite: requires Node.js on PATH"]
fn the_web_packages_node_suite_passes() {
    assert!(
        node_available(),
        "this gate needs Node.js on PATH; it does not pass by skipping"
    );
    for (package, needs_install) in PACKAGES {
        let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(package);
        if *needs_install {
            // Install from the lockfile even when a previous node_modules exists.
            let install = npm(&["ci", "--no-audit", "--no-fund"], &web);
            assert!(
                install.status.success(),
                "{package}: npm ci failed:\n{}",
                String::from_utf8_lossy(&install.stderr)
            );
        }
        let run = npm(&["test"], &web);
        let stdout = String::from_utf8_lossy(&run.stdout);
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(
            run.status.success(),
            "{package}: npm test failed:\n{stdout}\n{stderr}"
        );
        // A runner whose file list resolved to nothing also exits zero.
        let passed = reported_passes(&stdout)
            .or_else(|| reported_passes(&stderr))
            .expect("the runner reported no test count");
        assert!(passed > 0, "{package}: the suite ran no tests:\n{stdout}");
    }
}

#[test]
fn a_runner_that_ran_nothing_is_not_mistaken_for_a_pass() {
    assert_eq!(reported_passes("# pass 3\n# fail 0"), Some(3));
    assert_eq!(reported_passes("\u{2139} pass 2"), Some(2));
    assert_eq!(reported_passes("# pass 0"), Some(0));
    assert_eq!(reported_passes("no counts here"), None);
}
