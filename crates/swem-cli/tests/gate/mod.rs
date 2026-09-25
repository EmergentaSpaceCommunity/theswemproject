//! What every product gate shares: starting the real binary on a fresh data
//! root, the fixtures it needs beside it, the browser and node it drives,
//! and the readers of what the product wrote.
//!
//! The Cycle's own gate uses the same module on its side of the split, so
//! some of it is used there and not here.
#![allow(
    dead_code,
    reason = "shared by two gate binaries that each use part of it"
)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Each gate in this file starts the real product and drives a real browser.
/// Run two at once and they contend for the machine hard enough that Chromium
/// can miss its debugging port, which fails as "the journey stopped here" and
/// sends the next person hunting a product bug that does not exist. Taking
/// turns is cheap and keeps the gate honest however it is invoked - a test
/// that only passes under `--test-threads=1` is a test people will mis-run.
pub static PRODUCT_GATE: Mutex<()> = Mutex::new(());

pub fn one_at_a_time() -> MutexGuard<'static, ()> {
    // A panic in one gate must not disable the other.
    PRODUCT_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Where the product put a project, found rather than assumed: the data-root
/// layout differs per platform and the promise under test is that a durable
/// project exists, not that it sits at one spelling of one path.
pub fn find_project(data_root: &Path, slug: &str) -> Option<PathBuf> {
    let mut pending = vec![data_root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.file_name().is_some_and(|name| name == slug)
                && path.join("project.json").is_file()
            {
                return Some(path);
            }
            pending.push(path);
        }
    }
    None
}

/// The composition a person ended up with: every edit wrote a child revision,
/// so the head of the chain is the one no other revision names as a parent.
/// Reading the head rather than guessing at "the biggest snapshot" is what lets
/// a gate assert that something was removed and stayed removed.
pub fn head_composition(project_root: &Path) -> serde_json::Value {
    head_of_line(project_root, None)
}

/// The head composition of one line of a project, by the logical id the line
/// was started with - which is the slot's own name. A project with two slots
/// of one kind has two heads, and asking for "the" head would answer with
/// whichever came first.
pub fn composition_of(project_root: &Path, logical_id: &str) -> serde_json::Value {
    head_of_line(project_root, Some(logical_id))
}

pub fn head_of_line(project_root: &Path, logical_id: Option<&str>) -> serde_json::Value {
    let mut compositions: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();
    let mut parents: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut lines: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(project_root.join("journal").join("records"))
        .expect("the project has a records directory")
        .flatten()
    {
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(record) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        if record["body"]["snapshot"].get("composition").is_none() {
            continue;
        }
        if let Some(wanted) = logical_id
            && record["body"]["logical_id"] != serde_json::json!(wanted)
        {
            continue;
        }
        let reference = entry
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| format!("sha256:{stem}"))
            .unwrap_or_default();
        for parent in record["body"]["parents"].as_array().into_iter().flatten() {
            if let Some(parent) = parent.as_str() {
                parents.insert(parent.to_owned());
            }
        }
        lines.insert(
            reference.clone(),
            record["body"]["logical_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
        compositions.insert(reference, record["body"]["snapshot"]["composition"].clone());
    }
    // One line ends in one revision. Two leaves on the same line is a fork:
    // two writers took the same revision as their base, and one person's edit
    // is now in a revision nothing reads. Picking the first leaf by digest -
    // which is what this did - turns that into a coin toss: the notes walk
    // was green or red by which of two hashes sorted first, and it was red
    // three runs in three on a machine that had slowed down.
    //
    // Across lines, several heads are ordinary: a project with two music
    // slots has two of them, which is why `composition_of` exists.
    let mut leaves_by_line: std::collections::BTreeMap<&str, Vec<&String>> =
        std::collections::BTreeMap::new();
    for (reference, line) in &lines {
        if !parents.contains(reference) {
            leaves_by_line
                .entry(line.as_str())
                .or_default()
                .push(reference);
        }
    }
    for (line, leaves) in &leaves_by_line {
        assert!(
            leaves.len() < 2,
            "the line {line} forked and has {} heads, so no single revision is what the person ended with: {leaves:?}",
            leaves.len()
        );
    }
    compositions
        .iter()
        .find(|(reference, _)| !parents.contains(*reference))
        .map_or_else(
            || {
                panic!(
                    "no composition head{} in {}",
                    logical_id.map(|id| format!(" on {id}")).unwrap_or_default(),
                    project_root.display()
                )
            },
            |(_, composition)| composition.clone(),
        )
}

/// Every reading of the intent in a project's journal, by exact ref. The
/// record is the oracle for a re-read: what the page drew is not evidence
/// that one projection supersedes another.
pub fn projections(project_root: &Path) -> Vec<(String, serde_json::Value)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(project_root.join("journal").join("records"))
        .expect("the project has a records directory")
        .flatten()
    {
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(record) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        if record["schema"] != "swem:vision-projection@0.1" {
            continue;
        }
        let reference = entry
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| format!("sha256:{stem}"))
            .unwrap_or_default();
        found.push((reference, record["body"].clone()));
    }
    found
}

/// The browser the gate drives, or `None` when this machine has none.
pub fn browser() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SWEM_BROWSER") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    // The places a Chromium-family browser lives on the three platforms a
    // person runs this on. Without the Windows entries this gate skipped
    // itself on every Windows machine and reported success.
    [
        "/opt/pw-browsers/chromium",
        "/usr/bin/chromium",
        "/usr/bin/google-chrome",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|candidate| candidate.is_file())
}

pub fn node() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|directory| [directory.join("node"), directory.join("node.exe")])
        .find(|candidate| candidate.is_file())
}

/// The running product. Dropping it kills the process, so a driver that
/// panics mid-journey does not leave the product and every Cycle it spawned
/// running on the machine: their stdin pipes close with it and they leave.
/// Before this guard a failed gate leaked nine processes on one run.
pub struct Product {
    pub child: Child,
}

impl Product {
    /// Stop the product now, at the point the journey says so.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Product {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Lay the distribution's own packages out beside the binary, which is what a
/// distribution's packaging step does and what `cargo build` does not.
///
/// A SWEM distribution is a directory: the binary with `plugins/` beside it.
/// Music is a package now - the binary names no domain - so without this the
/// product the gate starts serves the kernel and software alone, and every
/// music journey below would be walking a Cycle that has no music in it.
/// Binaries this gate needs that `cargo test` does not build for it, named
/// together with the command that builds them.
///
/// Without this, a person who has just cloned the repository watches every
/// walk in this file fail in under a second on `NoEngine` and reads it as a
/// completely broken product. One refusal that says what to run is the
/// difference between a missing step and a bug hunt.
pub fn require_the_binaries_cargo_does_not_build() {
    let beside = Path::new(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .to_path_buf();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let missing: Vec<&str> = [(
        "swem-hands-agent",
        "cargo build -p swem-host --bin swem-hands-agent",
    )]
    .into_iter()
    .filter(|(name, _)| !beside.join(format!("{name}{suffix}")).is_file())
    .map(|(_, command)| command)
    .collect();
    assert!(
        missing.is_empty(),
        "this gate needs binaries cargo test does not build. Run:\n  {}\n\
         scripts/gate.sh does this and the rest.",
        missing.join("\n  ")
    );
}

pub fn install_the_distributions_packages() {
    require_the_binaries_cargo_does_not_build();
}

/// The product, started the way a person starts it, on `data_root`.
///
/// Returns the running product and the URL it printed, so the gate uses the
/// address the product chose rather than one the test assumed.
pub fn start_product(data_root: &Path) -> (Product, String) {
    start_product_against(data_root, None)
}

/// The product, started against a named agent registry index. A person points
/// the product at a mirror, an internal copy or a file with the same flag; the
/// gate uses it so the walk describes a real registry it can also stand up.
pub fn start_product_against(data_root: &Path, registry_index: Option<&str>) -> (Product, String) {
    install_the_distributions_packages();
    let mut arguments = vec![
        "workbench".to_owned(),
        "serve".to_owned(),
        "--no-open".to_owned(),
        "--port".to_owned(),
        "0".to_owned(),
    ];
    if let Some(index) = registry_index {
        arguments.push("--acp-registry".to_owned());
        arguments.push(index.to_owned());
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_swem"))
        .args(&arguments)
        .env("XDG_DATA_HOME", data_root)
        .env("LOCALAPPDATA", data_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start the swem binary");
    let stdout = child.stdout.take().expect("product stdout");
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let Some(Ok(line)) = lines.next() else { break };
        println!("product: {line}");
        if let Some(url) = line.strip_prefix("SWEM Workbench: ") {
            // Keep reading: dropping the pipe here would close its read end,
            // and the product's next line (its sandbox origin) would then be
            // a broken pipe that kills the product mid-gate.
            std::thread::spawn(move || {
                for line in lines.map_while(Result::ok) {
                    println!("product: {line}");
                }
            });
            return (Product { child }, url.trim().to_owned());
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("the product never printed its Workbench URL");
}

/// Drive the front door in a real browser. Panics with the driver's own output
/// when a step does not happen.
pub fn walk_front_door(
    url: &str,
    project: &str,
    expect_existing: Option<&str>,
    browser: &Path,
    node: &Path,
) {
    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/product_front_door_driver.mjs");
    let mut command = Command::new(node);
    command.arg(&driver).arg(url).arg(project);
    if let Some(existing) = expect_existing {
        command.arg(existing);
    }
    let output = command
        .env("SWEM_BROWSER", browser)
        // The gate's own container runs as root, where Chromium needs this.
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the front-door driver");
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "front-door driver failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("front door OK"),
        "the driver did not reach the end of the journey"
    );
}

/// How long a gate's leavings are kept before the next run sweeps them: long
/// enough that a run under way, or one whose failure someone is still looking
/// at, is never touched.
pub const KEEP_LEAVINGS: Duration = Duration::from_secs(6 * 60 * 60);

/// How many leavings survive the sweep whatever their age. A walk's data root
/// holds the browser profile and the release it built, which runs to hundreds
/// of megabytes, and the gate has twenty walks - so three runs inside the age
/// window is several gigabytes, and the fourth run dies of a full disk rather
/// than of anything about the product. This keeps the newest run's evidence
/// and part of the one before it, and lets the age rule handle the rest.
pub const KEEP_NEWEST_LEAVINGS: usize = 24;

/// An isolated data root nothing else is using.
///
/// Each gate leaves its data root behind on purpose - a failed journey is read
/// from the project the product actually wrote. Nothing ever removed them, and
/// nothing had to: one full pass of this file left 11 GB in the temp directory
/// and filled the disk of the machine it ran on, which then failed the next
/// command with a message about space rather than about SWEM. So a run sweeps
/// what earlier runs left, by this file's own naming and by age.
pub fn fresh_data_root(label: &str) -> PathBuf {
    sweep_old_leavings();
    let root = std::env::temp_dir().join(format!(
        "swem-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create an isolated data root");
    root
}

/// Remove the data roots and browser profiles earlier runs of this file left,
/// once they are old enough that no run is reading them. Best effort: a
/// directory that will not go is left alone, because a gate that fails over
/// housekeeping tells the next person nothing about the product.
pub fn sweep_old_leavings() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let mut leavings: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir()
            || !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("swem-"))
        {
            continue;
        }
        let Ok(at) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        leavings.push((at, path));
    }
    // Newest first, so what survives is the evidence someone would look at.
    leavings.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    for (index, (at, path)) in leavings.iter().enumerate() {
        let old = SystemTime::now()
            .duration_since(*at)
            .is_ok_and(|age| age > KEEP_LEAVINGS);
        if old || index >= KEEP_NEWEST_LEAVINGS {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// A second of a tone as PCM16 mono WAV: a real file to import.
///
/// Written here rather than through the music module, because this gate is
/// the product's front door and must depend on nothing the product binary
/// does not already carry.
/// A click track in two halves: `bars` bars of four beats at `bpm`, the
/// second half struck far harder than the first. Nothing about it is told to
/// the product - it arrives as bytes like any file a person has on disk.
///
/// Written at `rate`, because what a walk hands the product should be the
/// size of a file, not of a session: a click track carries nothing above
/// telephone quality, and the same twenty-six seconds at forty-eight
/// kilohertz is two and a half megabytes - twenty-six times what any other
/// import walk hands over, and enough to leave the browser this gate drives
/// no longer answering its driver.
pub fn click_track_wav(rate: u32, bpm: f64, bars: u32) -> Vec<u8> {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a beat is well inside u32, and a tempo is positive by construction"
    )]
    let beat = (60.0 * f64::from(rate) / bpm).round() as u32;
    let frames = beat * bars * 4;
    let burst = rate / 50;
    let data_bytes = frames * 2;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..frames {
        let since = index % beat;
        let loud = if index >= frames / 2 { 16_000 } else { 2_000 };
        // A burst rather than one sample: what this is heard by is the
        // loudness of a hundredth of a second, and a single spike can fall
        // between two of those windows.
        let sample: i16 = if since < burst && (since % 40) < 20 {
            loud
        } else if since < burst {
            -loud
        } else {
            0
        };
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

/// A real WAV a person could own and this product does not take: PCM16 at the
/// composition's rate, but with more channels than the codec accepts. The walk
/// uses it to ask what a person is told when their file is refused.
pub fn many_channel_wav(channels: u16, seconds: u32) -> Vec<u8> {
    const RATE: u32 = 48_000;
    let frames = RATE * seconds;
    let block_align = channels * 2;
    let data_bytes = frames * u32::from(block_align);
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * u32::from(block_align)).to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for _ in 0..frames {
        for _ in 0..channels {
            wav.extend_from_slice(&0_i16.to_le_bytes());
        }
    }
    wav
}

pub fn tone_wav(hertz: f64, seconds: u32) -> Vec<u8> {
    scaled_tone_wav(hertz, seconds, 12_000.0)
}

/// A mono PCM16 tone at 48 kHz, peaking at `amplitude`.
pub fn scaled_tone_wav(hertz: f64, seconds: u32, amplitude: f64) -> Vec<u8> {
    const RATE: u32 = 48_000;
    let frames = RATE * seconds;
    let data_bytes = frames * 2;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..frames {
        let phase = f64::from(index) / f64::from(RATE) * hertz * std::f64::consts::TAU;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the sine is scaled well inside i16"
        )]
        let sample = (phase.sin() * amplitude) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

/// The other half of the tidy walk: the two controls on this surface that
/// destroy something a person made, each pressed here for the first time
///.
///
/// The roll's Clear takes every note out of a block, and its own title says
/// it cannot be taken back from there. What it must not take is the block:
/// somebody asking for the notes to go is not asking for the block to go, and
/// a Clear that took both would be a person losing what they were about to
/// write into. The section's Remove is the same shape one level up - and the
/// panel a person pressed it in has to close, since what it was open on is
/// gone.
pub fn the_two_destroying_controls_of_the_workbench(
    report: &serde_json::Value,
    composition: &serde_json::Value,
) {
    assert_eq!(
        report["clearBefore"], "offered",
        "the roll would not clear a block that holds notes: {report}"
    );
    assert_eq!(
        report["clearAfter"], "refused",
        "the roll still offers to clear a block with nothing in it: {report}"
    );
    assert_eq!(
        report["blockAfterClear"], "0",
        "clearing the notes did not leave an empty block behind: {report}"
    );
    // The copy a person makes of a stretch they have already shaped, and the
    // bar the piece counts in - the picker that had never been asked for
    // anything but the four it starts on.
    assert_eq!(
        (report["beforeCopy"].as_str(), report["afterCopy"].as_str()),
        (Some("1"), Some("2")),
        "copying the block did not leave two of them: {report}"
    );
    assert_eq!(
        (report["beganIn"].as_str(), report["countsIn"].as_str()),
        (Some("4/4"), Some("3/4")),
        "the piece is not counted in three after it was asked to be: {report}"
    );
    assert_eq!(
        composition["tempo"]["beats"],
        serde_json::json!(3),
        "the record does not carry the bar the person set: {}",
        composition["tempo"]
    );
    // The roll says what is missing and offers it where a person is looking:
    // notes written on a role that sounds nothing, and the picker inside that
    // notice. Every other walk reaches an instrument through the role's own
    // header instead, so this way in had never been taken.
    assert!(
        report["owed"]
            .as_str()
            .is_some_and(|said| said.contains("no instrument")),
        "the roll does not say the notes written on it sound nothing: {report}"
    );
    assert_eq!(
        report["playsAfterRoll"], "opendaw.vaporisateur",
        "the instrument picked on the roll did not reach the role: {report}"
    );
    assert_eq!(
        report["owedAfter"], "settled",
        "the roll still says the role has nothing to sound its notes: {report}"
    );
    let lead = role_in(composition, "lead");
    let blocks = lead["clips"]
        .as_array()
        .unwrap_or_else(|| panic!("the role kept no blocks: {lead}"));
    let block = blocks
        .iter()
        .find(|clip| clip["name"] == report["block"])
        .unwrap_or_else(|| panic!("the block the notes were cleared from is gone: {lead}"));
    assert_eq!(
        block["notes"].as_array().map_or(0, Vec::len),
        0,
        "the cleared block still holds notes: {block}"
    );
    assert_eq!(
        report["sectionsAfter"], "0",
        "the section is still drawn on the plan after it was taken off: {report}"
    );
    assert_eq!(
        report["panelAfterSection"], "closed",
        "the panel stayed open on a section that is no longer there: {report}"
    );
    // The ledger a person reads is the last six things done, so these rows
    // are read at the end rather than beside the ones about the roles.
    let after = report["ledgerAfter"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .unwrap_or_default();
    for expected in ["cleared the notes of", "removed the section"] {
        assert!(
            after.contains(expected),
            "the ledger does not say what was destroyed ({expected}): {after}"
        );
    }
    assert!(
        composition["sections"].as_array().is_none_or(Vec::is_empty),
        "the record still carries the section that was taken off: {}",
        composition["sections"]
    );
}

/// One role of a composition, by name.
pub fn role_in(composition: &serde_json::Value, role: &str) -> serde_json::Value {
    composition["tracks"]
        .as_array()
        .and_then(|tracks| tracks.iter().find(|track| track["name"] == role))
        .unwrap_or_else(|| panic!("no role {role}: {composition}"))
        .clone()
}

/// The second half of the voice walk: the lead taken back to audio blocks
/// from the instrument's own panel.
///
/// That is the way back a person who picked an instrument by mistake needs,
/// and a button nobody had pressed. The role's own picker offers the
/// same act as its "audio" option and is walked, so what is untested here is
/// the control: whether the panel closes behind it rather than staying open
/// on an instrument that is no longer there.
pub fn the_lead_is_a_role_of_audio_blocks_again(
    report: &serde_json::Value,
    composition: &serde_json::Value,
) {
    assert_eq!(
        report["panelAfter"], "closed",
        "the instrument's panel stayed open after the instrument came off: {report}"
    );
    assert!(
        report["peakAfter"].as_f64().unwrap_or(0.0) > 0.005,
        "taking one part back to audio blocks silenced the whole piece: {report}"
    );
    assert_eq!(
        report["playsBack"], "audio",
        "the lead's own picker does not say it is a role of audio blocks: {report}"
    );
    assert_eq!(
        report["playsOn"], "opendaw.soundfont",
        "taking the lead off the bank took the bass off it too: {report}"
    );
    let after: Vec<&str> = report["saidAfter"]
        .as_array()
        .map(|rows| rows.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        after
            .iter()
            .any(|row| row.contains("took the instrument off") && row.contains("Lead")),
        "the ledger does not say the instrument came off the lead: {after:?}"
    );
    // And the row about the voice that role played still names it. The bank
    // is read off the role as it stands (`soundfontOf`), and a role with no
    // instrument falls back to the General MIDI bank the package ships - the
    // one this voice came from - so the word survives the instrument coming
    // off. Were it a bank of the person's own it would not, and the row would
    // give way to the General MIDI number the record itself carries, which is
    // still a voice where "gave “Lead” a voice" would be nothing.
    assert!(
        after
            .iter()
            .any(|row| row.contains("Lead") && row.contains("voice") && row.contains("Flute")),
        "the ledger stopped saying which voice the lead played: {after:?}"
    );
    let lead = role_in(composition, "Lead");
    assert!(
        lead["instrument"].is_null(),
        "the lead still carries an instrument after it was taken off: {lead}"
    );
    assert!(
        lead["clips"]
            .as_array()
            .is_some_and(|clips| !clips.is_empty()),
        "taking the instrument off the lead took its blocks with it: {lead}"
    );
}

/// The interchange file a person opens the release with somewhere else.
///
/// A release carries `project.dawproject` so the piece can be opened in
/// another tool, and that document points at files outside the container:
/// every clip's audio as `takes/<node>.wav`, and - once an instrument names
/// the sound it plays - the sound itself. This asserts the only thing that
/// makes the document worth carrying: **every file it points at is in the
/// release beside it**. A reference to a file nobody shipped opens as a
/// project of missing clips, which is worse than no interchange file, because
/// it looks like one.
///
/// It also asserts that an instrument's sound is named at all. `State` is the
/// format's own slot for the file a device loads, a `FileReference` with
/// `path` and `external` exactly like a clip's audio
/// (`@opendaw/lib-dawproject`, `DeviceSchema`).
pub fn the_interchange_file_points_only_at_files_the_release_holds(released: &Path) {
    let render: serde_json::Value = serde_json::from_slice(
        &std::fs::read(released.join("render.json")).expect("read render.json"),
    )
    .expect("parse render.json");
    let nodes = render["nodes"].as_object().cloned().unwrap_or_default();

    let dawproject =
        std::fs::read(released.join("project.dawproject")).expect("read the dawproject");
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(dawproject)).expect("the dawproject is a zip");
    let mut project = String::new();
    std::io::Read::read_to_string(
        &mut archive
            .by_name("project.xml")
            .expect("project.xml in the container"),
        &mut project,
    )
    .expect("read project.xml");

    // Everything the document sends a reader outside the container for.
    let mut pointed: Vec<String> = Vec::new();
    for piece in project.split("path=\"").skip(1) {
        let Some((path, rest)) = piece.split_once('"') else {
            continue;
        };
        if rest.starts_with(" external=\"true\"") {
            pointed.push(path.to_owned());
        }
    }
    // What the render says is outside the container: a take with audio is a
    // file, and so is the sound an instrument was given. Counting them is what
    // keeps this from passing on a document that points at nothing because it
    // forgot to.
    let owed = nodes
        .values()
        .filter(|node| {
            (node["kind"] == "take" && node["audio"] != serde_json::Value::Bool(false))
                || (node["kind"] == "track" && node["instrument"]["material"].is_string())
        })
        .count();
    assert!(
        pointed.len() >= owed,
        "the render holds {owed} file(s) outside the container and the dawproject points at {}: \
         {pointed:?}\n{project}",
        pointed.len()
    );
    let missing: Vec<&String> = pointed
        .iter()
        .filter(|path| !released.join(path).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "the dawproject of this release points at {} file(s) the release does not hold: {missing:?}\n\
         a person opening it in another tool gets a project of missing clips. The release holds: {:?}",
        missing.len(),
        std::fs::read_dir(released)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name())
            .collect::<Vec<_>>()
    );

    // And the sound an instrument was given is named, so the player it opens
    // in is not an empty one.
    for (node_id, node) in nodes
        .iter()
        .filter(|(_, node)| node["kind"] == "track" && node["instrument"]["material"].is_string())
    {
        let material = node["instrument"]["material"].as_str().unwrap_or_default();
        let reference = format!("<State path=\"{material}\" external=\"true\"/>");
        assert!(
            project.contains(&reference),
            "the instrument of {node_id} plays {material} and the dawproject never names it, so a \
             person opening this release elsewhere gets an empty player:\n{project}"
        );
        if let Some(voice) = node["instrument"]["voice"].as_array() {
            let (program, bank) = (
                voice[0].as_i64().unwrap_or(-1),
                voice[1].as_i64().unwrap_or(-1),
            );
            for (name, value) in [
                ("General MIDI program", program),
                ("General MIDI bank", bank),
            ] {
                assert!(
                    project.contains(&format!("name=\"{name}\" value=\"{value}\"")),
                    "the instrument of {node_id} is set to {name} {value} and the dawproject does \
                     not say so, so the release opens on whatever preset comes first:\n{project}"
                );
            }
        }
    }
}

/// What the engine said about its own render, beside the master it wrote.
///
/// A silent stem has been read as a broken product more than once.
/// The account says what the render itself produced - the master's peak and
/// each role's - so a red run separates "the render made silence" from
/// "something after the render lost the sound" without anybody going to look
/// for a file afterwards.
pub fn render_account(released: &Path) -> String {
    let mut pending = vec![released.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name == "render-result.json")
            {
                return std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| format!("unreadable ({error})"));
            }
        }
    }
    "no render-result.json beside the master".to_owned()
}

/// The shape of a PCM16 WAV the gate reads back: rate, channels, samples.
pub fn parse_pcm16_wav(bytes: &[u8]) -> (u32, u16, Vec<i16>) {
    assert!(bytes.len() > 44 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE");
    let mut offset = 12;
    let mut format: Option<(u32, u16)> = None;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        let body = offset + 8;
        if id == b"fmt " {
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            let rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
            format = Some((rate, channels));
        } else if id == b"data" {
            let (rate, channels) = format.expect("fmt before data");
            let end = (body + size).min(bytes.len());
            let samples = bytes[body..end]
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            return (rate, channels, samples);
        }
        offset = body + size + (size % 2);
    }
    panic!("no data chunk");
}

/// The first file named `name` under `root` whose path passes through a
/// directory named `through`.
pub fn find_file(root: &Path, name: &str, through: &str) -> Option<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().is_some_and(|file| file == name)
                && path.components().any(|part| part.as_os_str() == through)
            {
                return Some(path);
            }
        }
    }
    None
}

/// `(sample rate, channels, frames)` of a canonical 16-bit WAV.
pub fn wav_shape(bytes: &[u8]) -> (u32, u16, u32) {
    assert!(bytes.len() >= 44 && &bytes[0..4] == b"RIFF", "a WAV file");
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    let rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let data = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
    (rate, channels, data / (u32::from(channels.max(1)) * 2))
}

/// Where the named stretches fall, in the model and on the page.
///
/// Counting them proved nothing about where they fell: the reading answers in
/// the recording's own frames and the piece counts in its own ticks, and an
/// eight-kilohertz click track ten bars long had both halves drawn across the
/// first bar and a half of the block. Two parts existed, the tempo was right,
/// and the gate was green.
pub fn where_the_stretches_fall(
    composition: &serde_json::Value,
    sections: &[serde_json::Value],
    report: &serde_json::Value,
) {
    let block = composition
        .pointer("/tracks/0/clips/0")
        .unwrap_or_else(|| panic!("the recording is a block of the piece: {composition}"))
        .clone();
    let opens = block["start_ticks"].as_i64().unwrap_or_default();
    let closes = opens + block["length_ticks"].as_i64().unwrap_or_default();
    let ticks =
        |section: &serde_json::Value, field: &str| section[field].as_i64().unwrap_or_default();
    let first = ticks(&sections[0], "start_ticks");
    let last = ticks(&sections[1], "end_ticks");
    assert_eq!(
        first, opens,
        "the first stretch starts at {first}, and the recording at {opens}: {sections:?}"
    );
    // A tolerance of one beat: where a part ends is the reading's judgement,
    // where the block ends is arithmetic, and they need not agree to the tick.
    let beat = (closes - opens) / 40;
    let covered = if closes > opens {
        format!("{}%", (last - opens) * 100 / (closes - opens))
    } else {
        "none".to_owned()
    };
    assert!(
        (last - closes).abs() <= beat,
        "the named stretches end at {last} and the recording at {closes}, which is \
         {covered} of it - the reading answers in the recording's frames and the piece \
         counts in its own ticks: {sections:?}"
    );

    // And where the two of them meet. Both ends can be right while the seam
    // between them is not: this recording is struck far harder from its exact
    // middle, which is the only thing there is to hear a part boundary in, so
    // the edge a person is given to cut on belongs at the middle of the
    // block.
    let seam = ticks(&sections[1], "start_ticks");
    let middle = opens + (closes - opens) / 2;
    let out_by = (seam - middle).abs();
    assert!(
        out_by <= beat,
        "the stretches meet at {seam} and the recording changes at {middle}, out by \
         {out_by} ticks - {}% of the block, and a person cutting on that edge cuts \
         there: {sections:?}",
        out_by * 100 / (closes - opens).max(1)
    );

    // And where the page draws them. The model can be right while the screen
    // is wrong: the ruler, the named stretches and the playhead are laid out
    // from one constant for the width of the role column, and the column
    // itself was styled with a width plus padding, so the grid was drawn a
    // second to the left of every block on it. A person cutting a section
    // edge against the block under it would have been cutting a second early.
    if opens != 0 {
        return;
    }
    let grid = &report["grid"];
    let (Some(ruler), Some(drawn)) = (grid["ruler"].as_i64(), grid["block"].as_i64()) else {
        panic!("the driver did not report where the grid begins: {report}")
    };
    assert!(
        (ruler - drawn).abs() <= 1,
        "the ruler and the named stretches begin at x={ruler} and the block that starts \
         at the piece's first tick at x={drawn}: the grid is drawn {} px beside the music \
         it measures",
        (ruler - drawn).abs()
    );
}

/// A copy of this repository's own package with one more thing declared: a
/// tool, packed here and served on loopback. Returns the staging directory to
/// remove afterwards, the package to install, and the address and digest the
/// person will be shown before they consent.
pub fn a_package_that_brings_a_tool() -> (PathBuf, PathBuf, String, String) {
    let staging = std::env::temp_dir().join(format!(
        "swem-package-tool-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
    ));
    std::fs::create_dir_all(&staging).expect("make a place for the package and its tool");

    // The tool, as a package would publish it: one executable in one archive.
    let tool_home = staging.join("tool");
    std::fs::create_dir_all(&tool_home).expect("make the tool's directory");
    let tool = tool_home.join(TOOL_A_PACKAGE_BRINGS);
    std::fs::write(&tool, "#!/bin/sh\necho 'the tool a package brought'\n")
        .expect("write the tool");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755))
            .expect("make the tool executable");
    }
    let archive = staging.join(format!("{TOOL_A_PACKAGE_BRINGS}.tar.gz"));
    let packed = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&tool_home)
        .arg(TOOL_A_PACKAGE_BRINGS)
        .output()
        .expect("run tar");
    assert!(
        packed.status.success(),
        "could not pack the tool: {}",
        String::from_utf8_lossy(&packed.stderr)
    );
    let digest = sha256_of(&archive);
    let archive_url = serve_one_file(&archive);

    // The package that declares it: this repository's own, with the
    // declaration a package makes when it brings a tool along.
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/plugins/hello-node");
    let package = staging.join("hello-node");
    let copied = Command::new("cp")
        .arg("-r")
        .arg(&source)
        .arg(&package)
        .output()
        .expect("run cp");
    assert!(
        copied.status.success(),
        "could not copy the package: {}",
        String::from_utf8_lossy(&copied.stderr)
    );
    let _ = std::fs::remove_dir_all(package.join("behaviour").join("target"));
    let manifest_path = package.join("plugin.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).expect("the package's manifest"))
            .expect("the manifest parses");
    manifest["tools"] = serde_json::json!([{
        "name": TOOL_A_PACKAGE_BRINGS,
        "version": "1.0.0",
        "artifacts": {
            swem_host::current_platform(): {
                "url": archive_url,
                "sha256": digest,
                "entry": TOOL_A_PACKAGE_BRINGS,
            }
        }
    }]);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).expect("the manifest serializes"),
    )
    .expect("write the manifest that declares the tool");

    (staging, package, archive_url, digest)
}

/// The tool the walk's package declares. Named here because the walk, the
/// manifest it writes and the file the Cycle reads must all say the same word.
pub const TOOL_A_PACKAGE_BRINGS: &str = "swem-stub-tool";

/// One file, served on loopback for as long as this process lives, and the
/// address to ask for it at.
///
/// The product fetches a package archive with `curl`, which is the right
/// thing for it to do and the wrong thing to ask of a gate machine: there is
/// no network there. Serving the archive here keeps the product's own fetch
/// in the walk - the bytes really are pulled over HTTP and really are
/// digested - while the address is one this machine can answer.
pub fn serve_one_file(path: &Path) -> String {
    use std::io::{BufRead, BufReader, Write};

    let bytes = std::fs::read(path).expect("read the file to serve");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("package.tar.gz")
        .to_owned();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind the file server");
    let address = listener.local_addr().expect("the file server's address");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let Ok(peer) = stream.try_clone() else {
                continue;
            };
            // The request head is read and dropped: this server answers one
            // thing, whatever is asked of it.
            let mut reader = BufReader::new(peer);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line == "\r\n" || line == "\n" => break,
                    Ok(_) => {}
                }
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/gzip\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n",
                bytes.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&bytes);
            let _ = stream.flush();
        }
    });
    format!("http://{address}/{name}")
}

/// Where the product put an installed package. The data-root layout differs
/// per platform, so this looks for the directory rather than assuming one
/// spelling of one path.
/// What a registry describing one agent leaves on disk, for a walk to point
/// the product at.
pub struct FixtureRegistry {
    pub index_url: String,
    pub sha256: String,
}

/// A registry index of one agent: the workspace's own ACP fixture, packaged
/// the way a registry packages a native distribution, under `registry_id`.
/// The product checks no advertised name for it, so the fixture agent may
/// honestly stand in for the distribution.
pub fn fixture_agent_registry(
    data_root: &Path,
    registry_id: &str,
    platform: &str,
) -> FixtureRegistry {
    let beside = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .to_path_buf();
    let agent_name = if cfg!(windows) {
        "swem-hands-agent.exe"
    } else {
        "swem-hands-agent"
    };
    assert!(
        beside.join(agent_name).is_file(),
        "build the fixture agent first: cargo build -p swem-host --bin swem-hands-agent"
    );
    let supply = data_root.join("supply");
    std::fs::create_dir_all(&supply).expect("create the registry's directory");
    let archive = supply.join("agent.tar.gz");
    let tarred = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&beside)
        .arg(agent_name)
        .status()
        .expect("run tar");
    assert!(tarred.success(), "packaging the fixture agent failed");
    let sha256 = sha256_of(&archive);
    let index = supply.join("registry.json");
    std::fs::write(
        &index,
        serde_json::to_vec_pretty(&serde_json::json!({
            "agents": [{
                "id": registry_id,
                "name": format!("{registry_id} (this gate's own registry)"),
                "version": "0.1.0-gate",
                "distribution": {
                    "binary": {
                        platform: {
                            "archive": format!("file://{}", archive.display()),
                            "sha256": sha256,
                            "cmd": agent_name,
                        }
                    }
                }
            }]
        }))
        .expect("serialize the registry index"),
    )
    .expect("write the registry index");
    FixtureRegistry {
        index_url: format!("file://{}", index.display()),
        sha256,
    }
}

/// Whether `path` lies under `<data root>/installed/<kind>/<id>/`, wherever
/// the data root itself is spelled on this platform.
pub fn under_install_root(path: &Path, data_root: &Path, kind: &str, id: &str) -> bool {
    let parts: Vec<&str> = path
        .components()
        .filter_map(|part| part.as_os_str().to_str())
        .collect();
    path.starts_with(data_root) && parts.windows(3).any(|run| run == ["installed", kind, id])
}

/// The hex SHA-256 of one file, as the person would paste it beside an
/// archive's address: computed here rather than by `sha256sum`, which macOS
/// does not ship.
pub fn sha256_of(path: &Path) -> String {
    use sha2::Digest as _;
    let bytes = std::fs::read(path).expect("read the archive to digest");
    format!("{:x}", sha2::Sha256::digest(bytes))
}

pub fn find_installed_package(data_root: &Path, id: &str) -> Option<PathBuf> {
    fn walk(directory: &Path, id: &str, depth: usize) -> Option<PathBuf> {
        if depth > 6 {
            return None;
        }
        for entry in std::fs::read_dir(directory).ok()?.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.file_name().is_some_and(|name| name == id) && path.join("plugin.json").is_file()
            {
                return Some(path);
            }
            if let Some(found) = walk(&path, id, depth + 1) {
                return Some(found);
            }
        }
        None
    }
    walk(data_root, id, 0)
}

/// The first directory of this name under `data_root`. The gate knows what
/// the product keeps, not where inside its own data root it keeps it.
pub fn find_directory(data_root: &Path, name: &str) -> Option<PathBuf> {
    let mut pending = vec![data_root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if entry.file_name() == name {
                return Some(path);
            }
            pending.push(path);
        }
    }
    None
}

/// Every container this product has made on this machine, by name. Empty when
/// there is no Podman here, which is also when nothing could have made one.
pub fn swem_containers() -> std::collections::BTreeSet<String> {
    Command::new("podman")
        .args([
            "ps",
            "--all",
            "--filter",
            "label=io.swem.managed=true",
            "--format",
            "{{.Names}}",
        ])
        .output()
        .map(|listed| {
            String::from_utf8_lossy(&listed.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The person's own agent, declared on this machine the way a person declares
/// one: a document in the agents directory naming an executable.
pub fn declare_the_fixture_agent(data_root: &Path) {
    let hands = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .join(if cfg!(windows) {
            "swem-hands-agent.exe"
        } else {
            "swem-hands-agent"
        });
    assert!(
        hands.is_file(),
        "build the fixture agent first: cargo build -p swem-host --bin swem-hands-agent"
    );
    let agents = data_root.join("SWEM").join("workbench").join("agents");
    std::fs::create_dir_all(&agents).expect("create the agent directory");
    std::fs::write(
        agents.join("hands.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "id": "hands",
            "name": "My own agent",
            "command": hands.display().to_string(),
        }))
        .expect("serialize the declaration"),
    )
    .expect("write the agent declaration");
}

/// Any project at all under `data_root`, by the file a project is made of.
/// [`find_project`] answers about one slug; this answers "is there one".
pub fn find_any_project(data_root: &Path) -> Option<PathBuf> {
    let mut pending = vec![data_root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.join("project.json").is_file() {
                return Some(path);
            }
            pending.push(path);
        }
    }
    None
}

/// One HTTP request to the running product, as any other program on this
/// machine can make it: the status, then the body.
///
/// `curl` rather than an HTTP crate on purpose - the product already requires
/// it to fetch an agent's archive, and the point of this walk is that a
/// program which is not the Workbench page is knocking.
pub fn knock(url: &str, arguments: &[&str]) -> (u32, String) {
    let output = Command::new("curl")
        .args(["-s", "-o", "-", "-w", "\n%{http_code}"])
        .args(arguments)
        .arg(url)
        .output()
        .expect("curl runs: the product needs it too");
    let answer = String::from_utf8_lossy(&output.stdout);
    let (body, status) = answer
        .rsplit_once('\n')
        .expect("curl writes the status on its own line");
    (
        status.trim().parse().expect("a status code"),
        body.to_owned(),
    )
}

/// A tone near the top of what the file holds: two seconds at 220 Hz, peaking
/// at 30 000 of 32 767. A person's own recording arrives this hot; what makes
/// the master overflow is turning it up afterwards.
pub fn loud_tone_wav(hertz: f64, seconds: u32) -> Vec<u8> {
    scaled_tone_wav(hertz, seconds, 30_000.0)
}
