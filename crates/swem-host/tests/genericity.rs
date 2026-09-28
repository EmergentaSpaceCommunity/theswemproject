//! The harness owns no domain knowledge.
//!
//! `swem-cycle` proves this mechanically for the kernel
//! (`crates/swem-cycle/tests/genericity.rs`); `swem-host` promised it in prose
//! only, and prose does not stop a media word from being typed into a session,
//! a permission or a workbench route. The harness runs agents, sessions,
//! environments, permissions and MCP servers; what those agents work on is a
//! package's business, reaching the host as data it never reads.
//!
//! Two things are deliberately outside the scan. `src/bin/` holds fixture
//! agents and servers, which exist to speak a domain's shapes at the host and
//! must be free to name them. Unit-test bodies may spell fixture literals, as
//! in the kernel's own oracle.

use std::path::{Path, PathBuf};

fn offenders(path: &Path, forbidden: &[&str]) -> Vec<(usize, String)> {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|_| panic!("read {}", path.display()));
    let body = source
        .split("\n#[cfg(test)]")
        .next()
        .unwrap_or(source.as_str());
    body.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .filter(|(_, line)| forbidden.iter().any(|literal| line.contains(literal)))
        .map(|(index, line)| (index + 1, line.to_owned()))
        .collect()
}

fn source_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every Rust source of the harness except the fixtures under `src/bin/`.
fn harness_sources() -> Vec<PathBuf> {
    fn walk(directory: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).expect("read source directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "bin") {
                    continue;
                }
                walk(&path, into);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                into.push(path);
            }
        }
    }
    let mut sources = Vec::new();
    walk(&source_root(), &mut sources);
    sources.sort();
    sources
}

/// Words that name a domain rather than the harness. The music package is the
/// one this repository actually ships, so its vocabulary is the list that
/// matters; the others are here because they were the previous flagship and a
/// paste from that era is exactly the mistake this catches.
const DOMAIN_WORDS: &[&str] = &[
    "swem.node:",
    "swem.output:",
    "swem.music",
    "mixdown",
    "soundfont",
    "dawproject",
    "opendaw",
    "wclap",
    "master.wav",
    "audio/wav",
    "midi",
    "move_clip",
    "trim_clip",
    "split_clip",
    "landing_page",
    "end_card",
    "site_realization",
    "rust_realization",
    "lower_software",
    "ffmpeg",
    // The vocabulary of the server that serves projects. The harness once
    // drew a Project space of its own from these; that space is the
    // server's App now, and the harness reads none of them.
    "swem://project",
    "swem-project-envelope",
    "record_set_digest",
    "create_project",
    "project_vision",
];

/// Domain packages the harness must not name. It carries `swem-domain-music`
/// as a *dev*-dependency for its fixtures, which is why this is a rule about
/// the harness's own sources and not about the manifest.
const DOMAIN_CRATES: &[&str] = &["swem_domain_music", "swem_domain_software"];

#[test]
fn the_harness_names_no_domain_word() {
    let sources = harness_sources();
    assert!(
        sources
            .iter()
            .any(|path| path.ends_with("workbench_shell.rs")),
        "the scan must cover the workbench shell: {sources:?}"
    );
    assert!(
        sources.iter().any(|path| path.ends_with("session.rs")),
        "the scan must cover the session surface: {sources:?}"
    );
    let mut found = Vec::new();
    for path in &sources {
        for (line, text) in offenders(path, DOMAIN_WORDS) {
            found.push(format!("{}:{line}: {}", path.display(), text.trim()));
        }
    }
    assert!(
        found.is_empty(),
        "the harness names a domain:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_harness_names_no_domain_package() {
    let mut found = Vec::new();
    for path in harness_sources() {
        for (line, text) in offenders(&path, DOMAIN_CRATES) {
            found.push(format!("{}:{line}: {}", path.display(), text.trim()));
        }
    }
    assert!(
        found.is_empty(),
        "the harness reaches into a domain package:\n{}",
        found.join("\n")
    );
}

/// The harness's own surface, which is where a domain word leaks most easily:
/// somebody adding a panel types the word for the thing they are looking at.
///
/// Media types are not on this list. A file upload has to name one to send a
/// file at all, and knowing that `.wav` is `audio/wav` is not knowing what a
/// composition is; a domain's own surface lives in that domain's package.
#[test]
fn the_harness_surface_names_no_domain_word() {
    fn walk(directory: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(directory).expect("read surface directory") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                walk(&path, into);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "ts" || extension == "tsx")
            {
                into.push(path);
            }
        }
    }
    let mut sources = Vec::new();
    walk(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/apps-host/src"),
        &mut sources,
    );
    sources.sort();
    assert!(
        sources.iter().any(|path| path.ends_with("ChatView.tsx")),
        "the scan must cover the page of chats: {sources:?}"
    );
    let mut found = Vec::new();
    for path in &sources {
        for (line, text) in offenders(path, SURFACE_DOMAIN_WORDS) {
            found.push(format!("{}:{line}: {}", path.display(), text.trim()));
        }
    }
    assert!(
        found.is_empty(),
        "the harness surface names a domain:\n{}",
        found.join("\n")
    );
}

/// What the harness surface must not name. Narrower than [`DOMAIN_WORDS`]
/// because a page legitimately handles files and media types.
const SURFACE_DOMAIN_WORDS: &[&str] = &[
    "swem.node:",
    "swem.music",
    "mixdown",
    "soundfont",
    "opendaw",
    "dawproject",
    "move_clip",
    "trim_clip",
    "split_clip",
    "swem://project",
    "swem-project-envelope",
    "record_set_digest",
    "create_project",
    "project_vision",
    "add_slot",
];

/// The rule can fail. A scan whose word list never matches anything is a
/// scan that would keep passing after somebody quietly narrowed it.
#[test]
fn the_rule_can_fail() {
    let sources = harness_sources();
    let found: Vec<_> = sources
        .iter()
        .flat_map(|path| offenders(path, &["WorkbenchShellError"]))
        .collect();
    assert!(
        !found.is_empty(),
        "the scan finds nothing even for a word the harness certainly spells"
    );
}
