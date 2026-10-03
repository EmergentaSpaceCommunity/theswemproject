//! The product gate of the harness, the Workbench and the Store: the journey
//! a person actually walks, started from the binary they actually run.
//!
//! Every other browser gate in this workspace builds host state in-process,
//! which is why the product could stay unusable while every component was
//! independently proven. This one starts `swem` itself on an empty data
//! directory and drives the browser through what a person does with an
//! agent and nothing else: install it, set it up, talk to it, run it in a
//! container, work with it from an editor, install more from the Store.
//! Nothing calls a host API directly and no declaration file is written.
//!
//! The walks over projects, packages and the domains are the other gate,
//! `product_front_door_cycle.rs`, along the line the product splits on.
//!
//! Ignored by default because it needs a browser; run it with
//! `cargo test -p swem-cli --test product_front_door -- --ignored --nocapture`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod gate;
use gate::*;

/// The registry lists many more agents than this build describes. A person
/// installs one of those by its registry id from the command line - the plan
/// first, then the install against the plan's exact id - and the product then
/// offers it as an agent this computer has: they make it theirs, start a
/// session and get an answer, with no catalogue entry for it anywhere.
#[test]
#[ignore = "product gate: starts the real binary and a real browser"]
fn a_person_installs_an_agent_the_product_never_heard_of() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let Some(platform) = swem_host::registry_platform() else {
        eprintln!("skipped: no registry platform for this machine");
        return;
    };
    let data_root = fresh_data_root("agent-from-registry");
    // An id no catalogue entry of this build names.
    let registry = fixture_agent_registry(&data_root, "hands-acp", platform);
    let swem = |arguments: &[&str]| -> serde_json::Value {
        let output = Command::new(env!("CARGO_BIN_EXE_swem"))
            .args(arguments)
            .env("XDG_DATA_HOME", &data_root)
            .env("LOCALAPPDATA", &data_root)
            .env("SWEM_ACP_REGISTRY_INDEX", &registry.index_url)
            .output()
            .expect("run the product's command line");
        assert!(
            output.status.success(),
            "swem {} failed:\n{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "swem {} did not print JSON ({error}):\n{}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stdout)
            )
        })
    };

    // The plan names what would be fetched, and is what the install consents to.
    let plan = swem(&["agents", "plan-install", "hands-acp"]);
    assert_eq!(plan["registry_id"], "hands-acp");
    assert_eq!(plan["kind"], "agent");
    assert_eq!(plan["distribution"]["sha256"], registry.sha256);
    let plan_id = plan["plan_id"]
        .as_str()
        .expect("the plan has an id")
        .to_owned();
    let receipt = swem(&[
        "agents",
        "install",
        "hands-acp",
        "--consent",
        "--confirm-plan",
        &plan_id,
    ]);
    assert_eq!(receipt["schema"], "swem:install-receipt@0.1");
    assert_eq!(receipt["kind"], "agent");
    assert_eq!(receipt["plan_id"], plan_id);
    let executable = PathBuf::from(receipt["executable"].as_str().expect("what to launch"));
    assert!(
        under_install_root(&executable, &data_root, "agents", "hands-acp"),
        "installed outside the install root: {}",
        executable.display()
    );
    assert!(executable.is_file());
    // The same command line now lists it as an agent this machine has.
    let listed = swem(&["agents", "list", "--json"]);
    let hands = listed
        .as_array()
        .expect("a list")
        .iter()
        .find(|agent| agent["id"] == "hands-acp")
        .expect("the installed agent is listed by its registry id");
    assert_eq!(hands["readiness"], "installed_unverified");

    // And the product offers it: not to install, to make theirs.
    let (product, url) = start_product_against(&data_root, Some(&registry.index_url));
    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/agent_from_registry_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg("hands-acp")
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the registry-agent driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("registry agent OK"),
        "the walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The profile the product made is of that agent, by its registry id.
    let profile =
        find_file(&data_root, "profile.json", "profiles").expect("the product wrote a profile");
    let profile: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile).expect("read the profile"))
            .expect("parse the profile");
    assert_eq!(profile["agent_id"], "hands-acp");
}

/// The Store: the registry's agents and the catalogs a person adds by
/// address, in one list, each installed against its own plan. A person adds a
/// catalog, installs an agent, an MCP server and a skill from the Store,
/// makes the agent theirs, gives it the server and the skill in Setup, and
/// the agent calls the server's tool and reads the skill where its own
/// loader looks - nothing composed by hand, nothing restarted.
#[test]
#[ignore = "product gate: starts the real binary and a real browser"]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: a catalog, three installs, the agent using both"
)]
fn a_person_installs_from_the_store_and_the_agent_uses_it() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let Some(platform) = swem_host::registry_platform() else {
        eprintln!("skipped: no registry platform for this machine");
        return;
    };
    let data_root = fresh_data_root("store");
    // The registry offers the fixture agent under `claude-acp`, which the
    // catalogue knows as `claude-code`: the layout whose skills folder the
    // agent reads (`.claude/skills`).
    let registry = fixture_agent_registry(&data_root, "claude-acp", platform);
    let supply = data_root.join("supply");

    // The catalog: the workspace's own MCP fixture as a native archive, and a
    // skill as a folder in a tar.gz.
    let beside = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .to_path_buf();
    let echo_name = if cfg!(windows) {
        "swem-mcp-echo.exe"
    } else {
        "swem-mcp-echo"
    };
    assert!(
        beside.join(echo_name).is_file(),
        "build the MCP fixture first: cargo build -p swem-host --bin swem-mcp-echo"
    );
    let echo_archive = supply.join("echo.tar.gz");
    let tarred = Command::new("tar")
        .arg("-czf")
        .arg(&echo_archive)
        .arg("-C")
        .arg(&beside)
        .arg(echo_name)
        .status()
        .expect("run tar");
    assert!(tarred.success(), "packaging the MCP fixture failed");
    let skill_dir = supply.join("shout");
    std::fs::create_dir_all(&skill_dir).expect("create the skill's folder");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: Shout\ndescription: when asked to be loud\n---\n\nAnswer in capitals.\n",
    )
    .expect("write the skill");
    let skill_archive = supply.join("shout.tar.gz");
    let tarred = Command::new("tar")
        .arg("-czf")
        .arg(&skill_archive)
        .arg("-C")
        .arg(&supply)
        .arg("shout")
        .status()
        .expect("run tar");
    assert!(tarred.success(), "packaging the skill failed");
    // A second skill that requires the first, and a kind nobody here takes.
    let polite_dir = supply.join("polite");
    std::fs::create_dir_all(&polite_dir).expect("create the second skill's folder");
    std::fs::write(
        polite_dir.join("SKILL.md"),
        "---\nname: Polite\ndescription: shout, but say please\n---\n\nSay please.\n",
    )
    .expect("write the second skill");
    let polite_archive = supply.join("polite.tar.gz");
    let tarred = Command::new("tar")
        .arg("-czf")
        .arg(&polite_archive)
        .arg("-C")
        .arg(&supply)
        .arg("polite")
        .status()
        .expect("run tar");
    assert!(tarred.success(), "packaging the second skill failed");
    let echo_receipt = supply.join("echo-receipt.json");
    let catalog = supply.join("catalog.json");
    std::fs::write(
        &catalog,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "swem:catalog@0.2",
            "name": "This gate's catalog",
            "entries": [
                {"kind": "skill", "id": "polite", "name": "Polite", "version": "1.0.0-gate",
                 "description": "shout, but say please",
                 "requires": [{"kind": "skill", "id": "shout"}],
                 "distribution": {"archive": {
                     "url": format!("file://{}", polite_archive.display()),
                     "sha256": sha256_of(&polite_archive)}}},
                {"kind": "example.other/thing@1", "id": "nothing", "name": "For another host",
                 "version": "1.0.0", "description": "a kind this Workbench does not take",
                 "distribution": {"archive": {
                     "url": format!("file://{}", polite_archive.display()),
                     "sha256": sha256_of(&polite_archive)}}},
                {"kind": "server", "id": "echo", "name": "Echo", "version": "0.1.0-gate",
                 "description": "echoes a nonce and leaves a receipt",
                 "distribution": {"binary": {platform: {
                     "archive": format!("file://{}", echo_archive.display()),
                     "sha256": sha256_of(&echo_archive),
                     "cmd": echo_name,
                     "args": ["--receipt", echo_receipt.display().to_string()]}}}},
                {"kind": "skill", "id": "shout", "name": "Shout", "version": "1.0.0-gate",
                 "description": "answer in capitals",
                 "distribution": {"archive": {
                     "url": format!("file://{}", skill_archive.display()),
                     "sha256": sha256_of(&skill_archive)}}}
            ]
        }))
        .expect("serialize the catalog"),
    )
    .expect("write the catalog");
    // The same catalog, published again with a newer skill: what an update
    // is. Added by its new address it takes the place of the old one, as a
    // catalog of one name does.
    let later = supply.join("catalog-later.json");
    let mut republished: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&catalog).expect("read the catalog"))
            .expect("parse the catalog");
    for entry in republished["entries"]
        .as_array_mut()
        .expect("entries")
        .iter_mut()
    {
        if entry["id"] == "shout" {
            entry["version"] = serde_json::json!("1.1.0");
            entry["description"] = serde_json::json!("answer in capitals, louder");
        }
    }
    std::fs::write(
        &later,
        serde_json::to_vec_pretty(&republished).expect("serialize the later catalog"),
    )
    .expect("write the later catalog");

    let (product, url) = start_product_against(&data_root, Some(&registry.index_url));
    let nonce = format!("store-walk-{}", std::process::id());
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/store_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(format!("file://{}", catalog.display()))
        .arg("claude-code")
        .arg("claude-acp")
        .arg(&nonce)
        .arg(format!("file://{}", later.display()))
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the store driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("store OK"),
        "the walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // What the product wrote, not what the page said: one receipt per kind
    // under one root, the server's declaration, the skill on the profile,
    // and the server's own receipt of having been called.
    for (kind, id) in [
        ("agents", "claude-acp"),
        ("servers", "echo"),
        ("skills", "shout"),
    ] {
        let receipt = find_file(&data_root, "installation.json", id)
            .unwrap_or_else(|| panic!("no receipt for {id}"));
        assert!(
            under_install_root(&receipt, &data_root, kind, id),
            "{id}'s receipt is not under the install root: {}",
            receipt.display()
        );
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipt).expect("read the receipt"))
                .expect("parse the receipt");
        assert_eq!(receipt["schema"], "swem:install-receipt@0.1");
    }
    // What was removed is gone from the install root; what was updated has
    // both versions and the newer one is the one read.
    assert!(
        find_file(&data_root, "installation.json", "polite").is_none(),
        "the removed skill left its receipt"
    );
    let installed = find_directory(&data_root, "installed").expect("the install root");
    let shout = swem_host::load_receipts(&installed, &swem_host::Kind::SKILL);
    assert_eq!(
        shout["shout"].version, "1.1.0",
        "the newest is not the one read"
    );
    assert!(
        find_file(&data_root, "echo.json", "mcp-servers").is_some(),
        "the installed server is not declared in the MCP catalogue"
    );
    let profile = find_file(&data_root, "profile.json", "profiles").expect("a profile");
    let profile: serde_json::Value =
        serde_json::from_slice(&std::fs::read(profile).expect("read the profile"))
            .expect("parse the profile");
    assert_eq!(profile["agent_id"], "claude-code");
    assert!(
        profile["agent_skills"]
            .as_array()
            .is_some_and(|skills| skills.iter().any(|skill| skill["name"] == "shout")),
        "the skill is not on the profile: {}",
        profile["agent_skills"]
    );
    let called: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&echo_receipt).expect("the server left its receipt"))
            .expect("parse the server's receipt");
    assert_eq!(called["nonce"], nonce);
}

/// A first run with nothing in it: no project, no agent, no profile.
///
/// The product must be usable before there is anything to use it on. This
/// walks the door a person with an empty machine actually has: open the
/// product, see that no project exists, go to the Agent space, install an
/// agent from the plan the product shows, answer the consent question, and
/// end with an agent the product offers to start - without restarting it.
///
/// The registry is the product's own setting rather than the public index:
/// the gate stands one up on disk, with a digest-verified archive it builds
/// from the fixture agent in this workspace. That keeps the walk hermetic
/// and keeps every step of the supply path real - the index is fetched, the
/// plan is resolved for this platform, the archive is downloaded, its sha256
/// is checked against what the index promised, and what comes out is
/// launched. Nothing here is a fixture path inside the product.
///
/// This is the walk that caught the product telling a person an agent it had
/// just installed for them "is not available on this host": the list the
/// shell answered from was made when the product started.
#[test]
#[ignore = "product gate: needs a browser and node"]
#[allow(
    clippy::too_many_lines,
    reason = "one first run, in order: the registry is stood up, the agent installed, then used"
)]
fn a_person_with_nothing_installs_an_agent_and_can_start_it() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let Some(platform) = swem_host::registry_platform() else {
        eprintln!("skipped: no registry platform for this machine");
        return;
    };
    let data_root = fresh_data_root("agent-first-run");

    // `codex` is the catalog entry this index answers for. Its registry id is
    // `codex-acp`, and the product checks no advertised name for it, so the
    // fixture agent may honestly stand in for the distribution.
    let registry = fixture_agent_registry(&data_root, "codex-acp", platform);
    let sha256 = registry.sha256.clone();

    let (product, url) = start_product_against(&data_root, Some(&registry.index_url));
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/agent_first_run_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg("codex")
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the first-run driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("first run OK"),
        "the first run did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = walked
        .lines()
        .find(|line| line.starts_with("{\"profiles\""))
        .map(|line| serde_json::from_str(line).expect("parse the driver report"))
        .expect("the driver reported what it saw");

    // What the product wrote, not what the page said about it: the receipt of
    // the install, and the profile it made.
    let installed = find_file(&data_root, "installation.json", "codex-acp")
        .expect("the product wrote an installation receipt");
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&installed).expect("read the receipt"))
            .expect("parse the receipt");
    assert_eq!(receipt["registry_id"], "codex-acp");
    assert_eq!(receipt["version"], "0.1.0-gate");
    // One receipt schema for everything the product installs, under one
    // root inside the data root - not a directory of its own beside it.
    assert_eq!(receipt["schema"], "swem:install-receipt@0.1");
    assert_eq!(receipt["kind"], "agent");
    assert!(
        under_install_root(&installed, &data_root, "agents", "codex-acp"),
        "the receipt is not under the install root: {}",
        installed.display()
    );
    assert_eq!(
        receipt["sha256"].as_str().unwrap_or_default(),
        sha256,
        "the receipt does not record the digest the registry promised"
    );
    let executable = receipt["executable"]
        .as_str()
        .expect("the receipt says what to launch");
    assert!(
        Path::new(executable).is_file(),
        "the receipt points at no file: {executable}"
    );

    assert!(
        !report["terminal"].as_str().unwrap_or_default().is_empty(),
        "no terminal ran in the agent's environment: {report}"
    );

    // One installed agent, two people. The page showed both profiles and
    // showed the second one's session list empty; this is the same claim on
    // disk, where it matters: separate working directories and separate
    // native homes, so neither person's agent writes into the other's files.
    let profiles: Vec<String> = report["profiles"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        profiles.contains(&"ada".to_owned()) && profiles.len() == 2,
        "the same agent did not end with two profiles: {profiles:?}"
    );
    for (root, what) in [("workspaces", "workspace"), ("agent-homes", "native home")] {
        let holder = find_directory(&data_root, root)
            .unwrap_or_else(|| panic!("the product made no {root} directory"));
        let mut named: Vec<String> = std::fs::read_dir(&holder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        named.sort();
        assert!(
            named.contains(&"ada".to_owned()) && named.len() == 2,
            "the two profiles do not have a {what} each, {root} holds {named:?}"
        );
    }

    // The page said where the agent runs, and said it from the product's own
    // list rather than naming a place itself.
    let runs_in = report["runs_in"].as_str().unwrap_or_default();
    assert_eq!(
        runs_in,
        swem_host::THIS_MACHINE,
        "the page does not say the agent runs on this machine: {report}"
    );
    assert!(
        swem_host::environment_profiles()
            .iter()
            .any(|place| place.environment_profile_id == runs_in),
        "the page named an environment this product does not offer: {runs_in}"
    );

    // And the claim the catalogue exists for. The environment was a string
    // nothing read: a profile could say its agent runs elsewhere and the
    // product would start it here anyway. So take a profile that says exactly
    // that - the way one arrives is a profile written by another build, or by
    // hand - and check the product refuses rather than substitutes.
    let inventory = find_directory(&data_root, "profiles")
        .or_else(|| find_directory(&data_root, "inventory"))
        .expect("the product keeps its profiles somewhere");
    let ada = find_file(&inventory, "profile.json", "ada")
        .or_else(|| find_file(&data_root, "ada.json", ""))
        .expect("the second person's profile is on disk");
    let mut written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ada).expect("read the profile"))
            .expect("parse the profile");
    let elsewhere = "a-container-on-another-machine";
    written["environment_profile_id"] = serde_json::Value::String(elsewhere.to_owned());
    std::fs::write(
        &ada,
        serde_json::to_vec_pretty(&written).expect("serialize the profile"),
    )
    .expect("write the profile back");

    let (again, url) = start_product_against(&data_root, None);
    let refusal_driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/agent_unknown_environment_driver.mjs");
    let refused = Command::new(&node)
        .arg(&refusal_driver)
        .arg(&url)
        .arg("ada")
        .arg(elsewhere)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the unknown-environment driver");
    again.stop();
    let saw = String::from_utf8_lossy(&refused.stdout).into_owned();
    println!("{saw}");
    assert!(
        refused.status.success() && saw.contains("unknown environment OK"),
        "a profile naming an environment this product does not have was not refused:\n{}",
        String::from_utf8_lossy(&refused.stderr)
    );
}

/// A person hands their agent a file and the agent hands one back, with no
/// project in existence and no protocol for files on either side.
///
/// This is the claim that a directory is enough. The attachment a person makes
/// in the composer lands in `inbox` inside the agent's own workspace and the
/// turn names the path; the agent opens that path with nothing but `read`.
/// What it writes into `outbox` appears in the page for the person to open,
/// with no resource link, no tool call and no capability negotiated. The walk
/// ends by aiming a write out of the workspace and having it refused, because
/// a directory is only enough if it is also a boundary.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_hands_their_agent_a_file_and_gets_one_back() {
    const SECRET: &str = "the-thing-in-the-file-7f3a";
    const ANSWER: &str = "what the agent wrote back";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("agent-files");
    let handed = data_root.join("notes.txt");
    std::fs::write(&handed, format!("a note: {SECRET}\n")).expect("write the file a person has");
    declare_the_fixture_agent(&data_root);

    let (product, url) = start_product(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/agent_files_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(&handed)
        .arg(SECRET)
        .arg(ANSWER)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the files driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("files OK"),
        "the file round trip did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // What the page showed, checked where it lives: two directories in the
    // agent's workspace, with one file each and nothing beside them.
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let workspace = std::fs::read_dir(&workspaces)
        .expect("read the workspaces directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .expect("the profile has a workspace");
    let handed_back = std::fs::read_to_string(workspace.join("inbox").join("notes.txt"))
        .expect("the file a person handed over is in the inbox");
    assert!(
        handed_back.contains(SECRET),
        "the inbox holds different bytes: {handed_back}"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("outbox").join("reply.txt"))
            .expect("what the agent wrote is in the outbox"),
        ANSWER
    );
    assert!(
        !workspace.join("..").join("escaped.txt").exists()
            && !workspaces.join("escaped.txt").exists(),
        "the refused write landed anyway"
    );
}

/// A person's agent runs a command, and the person watches it happen.
///
/// The profile that works alone inside its workspace used to let an agent
/// write code it could never compile: every shell effect was rejected, and
/// ACP's own way for an agent to ask its client to run a command was refused.
/// Now the harness carries those out, in the environment it already keeps for
/// that profile - so the command runs where the agent's work is, and the
/// terminal it runs in is one of the person's own, listed in the panel they
/// open their own terminals in. The walk ends with the person clicking Watch
/// and reading what their agent's command said.
///
/// The walk then changes the person's mind: set to *asks every time*, the same
/// command reaches them as a question with the command line in it, refuses
/// when they say no, and runs when they say yes.
///
/// What this walk does not prove: that a real agent adapter asks for
/// `terminal/*` rather than running commands inside its own process. The
/// fixture asks because the protocol allows it; which of the installable
/// adapters do is theirs to decide, and is not asserted here.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_watches_their_agent_run_a_command() {
    const MARKER: &str = "what-the-agent-ran-4c81";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("agent-terminal");
    declare_the_fixture_agent(&data_root);

    let (product, url) = start_product(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/agent_terminal_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(MARKER)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the terminal driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("agent terminal OK"),
        "the agent never ran a command a person could watch:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Where it ran, checked on the disk rather than in the page: the file the
    // command wrote is in the profile's own workspace.
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let workspace = std::fs::read_dir(&workspaces)
        .expect("read the workspaces directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .expect("the profile has a workspace");
    let wrote = std::fs::read_to_string(workspace.join("ran.txt"))
        .expect("the command the agent ran wrote in the profile's workspace");
    assert!(
        wrote.contains(MARKER),
        "the command ran somewhere else: {wrote}"
    );

    // And the pair from the asking mode, where it lands rather than where it
    // was reported: the refused command left nothing at all.
    assert!(
        !workspace.join("refused.txt").exists(),
        "the command the person said no to ran anyway"
    );
    assert!(
        workspace.join("allowed.txt").is_file(),
        "the command the person said yes to left nothing behind"
    );
}

/// A person leaves a standing instruction and the product carries it out
/// without them.
///
/// Every other walk in this file is a person doing something. This one is the
/// product doing something because time passed: the person sets a schedule
/// and touches nothing else - no session started by hand, no prompt typed -
/// and a file appears in the agent's outbox that only the agent could have
/// written. The record says the clock wrote the turn, which is what makes a
/// scheduled turn accountable rather than anonymous.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_leaves_a_standing_instruction_and_the_product_carries_it_out() {
    const ANSWER: &str = "written while nobody was looking";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("agent-schedule");
    declare_the_fixture_agent(&data_root);

    let (product, url) = start_product(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/agent_schedule_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(ANSWER)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the schedule driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("schedule OK"),
        "the standing instruction was not carried out:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The walk read the chat through the product's own door and found the
    // schedule named there. It also read the schedule as the product keeps
    // it and the run it was said in: due at a moment, taken up, said in a
    // chat and answered.
    let report: serde_json::Value = serde_json::from_str(
        walked
            .lines()
            .last()
            .expect("the walk reported what it saw"),
    )
    .expect("parse the walk's report");
    assert_eq!(
        report["schedule"]["when"],
        serde_json::json!({ "kind": "every", "minutes": 1 })
    );
    let run = &report["run"];
    assert_eq!(run["state"], "answered", "{run}");
    assert!(
        run["claimed_ms"].as_u64().unwrap_or(0) >= run["due_ms"].as_u64().unwrap_or(u64::MAX),
        "it was said before it was due: {run}"
    );
    assert!(
        run["chat_id"].as_str().is_some_and(|chat| !chat.is_empty()),
        "the run was said nowhere: {run}"
    );
}

/// A person puts two agents in one chat.
///
/// What is said there is for who it names. The one named answers and the
/// other does not; two that go on naming each other are counted and, at the
/// chat's limit, wait for a person; one that is taken out is gone from the
/// chat. The engines here say back what they are told, so whether they
/// answer each other is decided by the product and by nothing they think.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_puts_two_agents_in_one_chat() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("chat-of-several");
    gate::declare_an_agent_that_says_it_back(&data_root);

    let (product, url) = start_product(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/chat_of_several_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the driver of a chat of several");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("chat of several OK"),
        "two agents could not be put in one chat:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(
        walked
            .lines()
            .last()
            .expect("the walk reported what it saw"),
    )
    .expect("parse the walk's report");
    assert_eq!(report["answered_the_one_named"], serde_json::json!(["ada"]));
    assert_eq!(report["held_at"], 2, "{report}");
    assert_eq!(
        report["in_the_chat_at_the_end"].as_array().map(Vec::len),
        Some(1),
        "{report}"
    );
}

/// A person works with their agent from the editor they write code in.
///
/// Every other agent walk in this file goes through the page. This one does
/// not open the page at all for the work itself: an editor starts the product
/// as its own child process over stdio, speaks ACP to it, and gets the agent
/// that person's profile runs - in that profile's directory, under that
/// profile's rules. The page is used only to set the agent up beforehand and
/// to read afterwards, which is the claim: two doors, one product, one record.
///
/// The client is the ACP project's own TypeScript SDK rather than a fixture
/// SWEM wrote, so what passes here is the protocol as a third party
/// implements it. It is still not Zed - no editor is installed on a gate
/// machine - and the walk claims no more than it does.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: the page, then the editor, then the page again"
)]
fn a_person_works_with_their_agent_from_their_editor() {
    const WROTE: &str = "from-the-editor.txt";
    const ANSWER: &str = "written while the person was in their editor";
    const EDITOR: &str = "the-gates-editor";
    const ASKED: &str = "write a file into your outbox";
    const REFUSED: &str = "not-allowed.txt";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let editor_client = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/acp-client");
    if !editor_client.join("node_modules").is_dir() {
        eprintln!(
            "skipped: the editor's own SDK is not installed. \
             scripts/gate.sh installs it; by hand it is \
             `npm install --prefix {}`",
            editor_client.display()
        );
        return;
    }
    let data_root = fresh_data_root("agent-editor");
    declare_the_fixture_agent(&data_root);

    // First the person sets their agent up, where a person sets one up.
    let (product, url) = start_product(&data_root);
    let setup = Command::new(&node)
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/editor_door_setup_driver.mjs"))
        .arg(&url)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the editor setup driver");
    product.stop();
    let walked = String::from_utf8_lossy(&setup.stdout).into_owned();
    println!("{walked}");
    assert!(
        setup.status.success() && walked.contains("editor setup OK"),
        "the person could not set their agent up:\n{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let profile = walked
        .lines()
        .find(|line| line.starts_with("{\"profile\""))
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|report| report["profile"].as_str().map(str::to_owned))
        .expect("the driver said which profile it made");

    // Then they close the page and open their editor. The directory the
    // editor has open is a real one, and it is not the agent's: an editor
    // sends its own `cwd` with every session, and taking it would let any
    // editor point a profile at any directory on the machine.
    let opened_in_the_editor = data_root.join("some-other-project");
    std::fs::create_dir_all(&opened_in_the_editor).expect("create the directory the editor opened");
    // The person at the editor is who answers the agent's questions, so the
    // walk plays them: once saying yes, once saying no.
    let open_the_editor = |say: serde_json::Value, answer: &str| -> serde_json::Value {
        let editor = Command::new(&node)
            .arg(editor_client.join("editor.mjs"))
            .arg(env!("CARGO_BIN_EXE_swem"))
            .arg(&profile)
            .arg(&opened_in_the_editor)
            .arg(say.to_string())
            .arg(answer)
            .env("XDG_DATA_HOME", &data_root)
            .env("LOCALAPPDATA", &data_root)
            .output()
            .expect("run the editor");
        let seen = String::from_utf8_lossy(&editor.stdout).into_owned();
        println!("editor: {seen}");
        assert!(
            editor.status.success(),
            "the editor could not work with SWEM:\n{}",
            String::from_utf8_lossy(&editor.stderr)
        );
        seen.lines()
            .find(|line| line.starts_with('{'))
            .map(|line| serde_json::from_str(line).expect("parse what the editor saw"))
            .expect("the editor reported what it saw")
    };
    let seen = open_the_editor(
        serde_json::json!({
            "write": format!("outbox/{WROTE}"),
            "text": ANSWER,
            "ask": ASKED,
        }),
        "allow",
    );

    // What an editor gets: an agent that names itself, on the version of the
    // protocol it asked for, ending its turn.
    assert_eq!(
        seen["agent"], "swem",
        "the editor met something else: {seen}"
    );
    assert_eq!(seen["protocol"], 1, "a protocol the editor did not ask for");
    assert_eq!(seen["stop"], "end_turn", "the turn did not end: {seen}");
    assert!(
        seen["updates"].as_u64().unwrap_or(0) > 0,
        "the editor was told nothing while the turn ran: {seen}"
    );

    // The boundary, in the editor's own window: the person is told where
    // their agent actually is rather than the harness quietly widening it to
    // wherever the editor happened to be pointed.
    let said = seen["text"].as_str().unwrap_or_default();
    assert!(
        said.contains(&opened_in_the_editor.display().to_string())
            && said.contains("is not what it can reach"),
        "the editor was not told its directory is not the agent's: {said}"
    );

    // And the boundary on disk, which is the part that matters: the file the
    // turn produced is in the profile's workspace, and the directory the
    // editor claimed has nothing in it.
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let workspace = std::fs::read_dir(&workspaces)
        .expect("read the workspaces directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .expect("the profile has a workspace");
    assert_eq!(
        std::fs::read_to_string(workspace.join("outbox").join(WROTE))
            .expect("the editor's turn wrote into the profile's workspace"),
        ANSWER
    );
    assert!(
        !opened_in_the_editor.join("outbox").exists(),
        "the agent worked in the directory the editor claimed: {}",
        opened_in_the_editor.display()
    );

    // The question reached the person who was there. This profile asks every
    // time - it is what one click in the page makes - so without the editor
    // being asked, the agent would have stopped at its first question and the
    // only surface that could release it is the page nobody has open.
    let asked = seen["asked"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        asked.len(),
        1,
        "the editor was asked {} times, not once: {seen}",
        asked.len()
    );
    assert_eq!(asked[0]["title"], ASKED, "the question lost its words");
    assert_eq!(
        asked[0]["answered"], "allow-once",
        "the editor answered something the agent did not offer: {seen}"
    );

    // And no is a real answer, not a slower yes: the same turn with the
    // person refusing leaves nothing on disk.
    let refused = open_the_editor(
        serde_json::json!({
            "write": format!("outbox/{REFUSED}"),
            "text": "should not exist",
            "ask": ASKED,
        }),
        "reject",
    );
    assert_eq!(
        refused["asked"][0]["answered"], "reject-once",
        "the editor did not refuse: {refused}"
    );
    assert!(
        !workspace.join("outbox").join(REFUSED).exists(),
        "the agent did it anyway after the person said no"
    );
    assert!(
        refused["text"]
            .as_str()
            .unwrap_or_default()
            .contains("not done"),
        "the agent did not say it had not done it: {refused}"
    );

    // Back in the page: the same product, the same record. The session the
    // editor opened is this person's session, the turn says an editor wrote
    // it, and the file is in their Files.
    let (again, url) = start_product(&data_root);
    let read = Command::new(&node)
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/editor_door_record_driver.mjs"))
        .arg(&url)
        .arg(&profile)
        .arg(WROTE)
        .arg(EDITOR)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the editor record driver");
    again.stop();
    let saw = String::from_utf8_lossy(&read.stdout).into_owned();
    println!("{saw}");
    assert!(
        read.status.success() && saw.contains("editor record OK"),
        "the editor's work is not in the record the page reads:\n{}",
        String::from_utf8_lossy(&read.stderr)
    );
}

/// A person has their agent read the file they have open, unsaved.
///
/// This is the one thing an editor can give an agent that the page never
/// could. ACP puts `fs/read_text_file` and `fs/write_text_file` on the client
/// because they are operations in the client's environment, and the
/// Workbench's client is a browser tab that has no files at all - so this host
/// refused both, everywhere, and said so in the code. At the editor door the
/// client is an editor: it has exactly the environment those methods name, and
/// the version of the file the person is actually looking at is in its buffer
/// and nowhere else.
///
/// So the walk makes the two versions differ on purpose. The file on disk says
/// one thing, the editor's buffer says another, and what the agent gets back
/// is the buffer. Then it writes, and the buffer changes while the disk does
/// not - which is what an unsaved editor is. Then it names a path outside the
/// profile's workspace, and the harness refuses it before the editor is asked
/// at all: a boundary that depended on the editor saying no would not be one.
///
/// The client is the ACP project's own TypeScript SDK, as in the other editor
/// walks, so the capability handshake is read by a third party's code.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: the page, then four launches of the editor, then the disk"
)]
fn a_person_has_their_agent_read_the_file_they_have_open() {
    const FILE: &str = "notes.txt";
    const ON_DISK: &str = "what was saved last time";
    const ON_SCREEN: &str = "what the person has typed and not saved";
    const TYPED_BY_THE_AGENT: &str = "what the agent put in their window";
    const NOT_THEIRS: &str = "a file this agent has no business with";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let editor_client = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/acp-client");
    if !editor_client.join("node_modules").is_dir() {
        eprintln!(
            "skipped: the editor's own SDK is not installed. \
             scripts/gate.sh installs it; by hand it is \
             `npm install --prefix {}`",
            editor_client.display()
        );
        return;
    }
    let data_root = fresh_data_root("agent-editor-files");
    declare_the_fixture_agent(&data_root);

    let (product, url) = start_product(&data_root);
    let setup = Command::new(&node)
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/editor_door_setup_driver.mjs"))
        .arg(&url)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the editor setup driver");
    product.stop();
    let walked = String::from_utf8_lossy(&setup.stdout).into_owned();
    println!("{walked}");
    assert!(
        setup.status.success() && walked.contains("editor setup OK"),
        "the person could not set their agent up:\n{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let profile = walked
        .lines()
        .find(|line| line.starts_with("{\"profile\""))
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|report| report["profile"].as_str().map(str::to_owned))
        .expect("the driver said which profile it made");

    let opened_in_the_editor = data_root.join("some-other-project");
    std::fs::create_dir_all(&opened_in_the_editor).expect("create the directory the editor opened");
    // `on_screen` is what this launch of the editor has in its window. Left
    // out, the editor claims no filesystem at all - which is what it did
    // before any of this and what the page still does.
    let open_the_editor = |say: serde_json::Value, on_screen: Option<&str>| -> serde_json::Value {
        let mut editor = Command::new(&node);
        editor
            .arg(editor_client.join("editor.mjs"))
            .arg(env!("CARGO_BIN_EXE_swem"))
            .arg(&profile)
            .arg(&opened_in_the_editor)
            .arg(say.to_string())
            .arg("allow")
            .arg("");
        if let Some(on_screen) = on_screen {
            editor.arg(on_screen);
        }
        let editor = editor
            .env("XDG_DATA_HOME", &data_root)
            .env("LOCALAPPDATA", &data_root)
            .output()
            .expect("run the editor");
        let seen = String::from_utf8_lossy(&editor.stdout).into_owned();
        println!("editor: {seen}");
        assert!(
            editor.status.success(),
            "the editor could not work with SWEM:\n{}",
            String::from_utf8_lossy(&editor.stderr)
        );
        seen.lines()
            .find(|line| line.starts_with('{'))
            .map(|line| serde_json::from_str(line).expect("parse what the editor saw"))
            .expect("the editor reported what it saw")
    };

    // First the saved version, put on the disk by an agent with hands and no
    // protocol - which is also this editor with nothing open, claiming no
    // filesystem and being refused if it asked.
    let saved = open_the_editor(serde_json::json!({ "write": FILE, "text": ON_DISK }), None);
    assert_eq!(saved["stop"], "end_turn", "the turn did not end: {saved}");
    assert_eq!(
        saved["files"].as_array().map(Vec::len),
        Some(0),
        "an editor that claimed no files was asked for one: {saved}"
    );
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let workspace = std::fs::read_dir(&workspaces)
        .expect("read the workspaces directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .expect("the profile has a workspace");
    let on_disk = workspace.join(FILE);
    assert_eq!(
        std::fs::read_to_string(&on_disk).expect("the saved version is on the disk"),
        ON_DISK
    );

    // Then the person opens it and types. What the agent asks for is the same
    // path, and what it gets is the window.
    let read = open_the_editor(serde_json::json!({ "client_read": FILE }), Some(ON_SCREEN));
    let said = read["text"].as_str().unwrap_or_default();
    assert!(
        said.contains(ON_SCREEN),
        "the agent did not get what is on the person's screen: {read}"
    );
    assert!(
        !said.contains(ON_DISK),
        "the agent was handed the saved file instead of the open one: {read}"
    );
    assert_eq!(
        read["files"],
        serde_json::json!([{
            "method": "fs/read_text_file",
            // The host asks for the path as the workspace resolves it, which on
            // macOS spells a temp directory with its `/private` prefix.
            "path": std::fs::canonicalize(&on_disk)
                .unwrap_or(on_disk.clone())
                .display()
                .to_string(),
        }]),
        "the editor was asked for something other than the file, once: {read}"
    );

    // And writing goes to the window too. The disk is what the person's own
    // save would change, and nothing here saved.
    let wrote = open_the_editor(
        serde_json::json!({ "client_write": FILE, "text": TYPED_BY_THE_AGENT }),
        Some(ON_SCREEN),
    );
    assert_eq!(
        wrote["onScreen"], TYPED_BY_THE_AGENT,
        "the agent's writing did not reach the person's window: {wrote}"
    );
    assert_eq!(
        wrote["files"][0]["method"], "fs/write_text_file",
        "the editor was not asked to write: {wrote}"
    );
    assert_eq!(
        std::fs::read_to_string(&on_disk).expect("the file is still on the disk"),
        ON_DISK,
        "the write went round the editor and onto the disk"
    );

    // The boundary is the profile's, and the harness holds it. An editor that
    // was asked and said no would be a different claim: this one is never
    // asked at all.
    let outside = data_root.join("not-theirs.txt");
    std::fs::write(&outside, NOT_THEIRS).expect("write the file outside the workspace");
    let refused = open_the_editor(
        serde_json::json!({ "client_read_anywhere": outside.display().to_string() }),
        Some(ON_SCREEN),
    );
    let said = refused["text"].as_str().unwrap_or_default();
    assert!(
        said.contains("the client did not read")
            && said.contains("outside the workspace this session works in"),
        "the agent was not told why it cannot have that file: {refused}"
    );
    assert!(
        !said.contains(NOT_THEIRS),
        "the agent was handed a file outside the workspace: {refused}"
    );
    assert_eq!(
        refused["files"].as_array().map(Vec::len),
        Some(0),
        "the editor was asked about a path the harness should have stopped: {refused}"
    );
}

/// A person has two windows of their editor open on the same SWEM.
///
/// ACP lets a client hold as many sessions as it likes on one connection, and
/// an editor is one process with several windows in it. This door kept one:
/// the second `session/new` replaced the first, and a prompt in the first
/// window came back "this prompt names no session this door opened" - a
/// person's window going dead because they opened another one.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: set the agent up, then two windows of one editor"
)]
fn a_person_has_two_windows_of_their_editor_open() {
    const ONE: &str = "first-window.txt";
    const TWO: &str = "second-window.txt";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let editor_client = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/acp-client");
    if !editor_client.join("node_modules").is_dir() {
        eprintln!(
            "skipped: the editor's own SDK is not installed. \
             scripts/gate.sh installs it; by hand it is \
             `npm install --prefix {}`",
            editor_client.display()
        );
        return;
    }
    let data_root = fresh_data_root("agent-editor-windows");
    declare_the_fixture_agent(&data_root);

    let (product, url) = start_product(&data_root);
    let setup = Command::new(&node)
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/editor_door_setup_driver.mjs"))
        .arg(&url)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the editor setup driver");
    product.stop();
    let walked = String::from_utf8_lossy(&setup.stdout).into_owned();
    println!("{walked}");
    assert!(
        setup.status.success() && walked.contains("editor setup OK"),
        "the person could not set their agent up:\n{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let profile = walked
        .lines()
        .find(|line| line.starts_with("{\"profile\""))
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|report| report["profile"].as_str().map(str::to_owned))
        .expect("the driver said which profile it made");

    let opened_in_the_editor = data_root.join("some-other-project");
    std::fs::create_dir_all(&opened_in_the_editor).expect("create the directory the editor opened");
    let editor = Command::new(&node)
        .arg(editor_client.join("two-windows.mjs"))
        .arg(env!("CARGO_BIN_EXE_swem"))
        .arg(&profile)
        .arg(&opened_in_the_editor)
        .arg(
            serde_json::json!({ "write": format!("outbox/{ONE}"), "text": "the first window" })
                .to_string(),
        )
        .arg(
            serde_json::json!({ "write": format!("outbox/{TWO}"), "text": "the second window" })
                .to_string(),
        )
        .env("XDG_DATA_HOME", &data_root)
        .env("LOCALAPPDATA", &data_root)
        .output()
        .expect("run the editor");
    let seen = String::from_utf8_lossy(&editor.stdout).into_owned();
    println!("editor: {seen}");
    assert!(
        editor.status.success(),
        "the editor could not hold two windows:\n{}",
        String::from_utf8_lossy(&editor.stderr)
    );
    let report: serde_json::Value = seen
        .lines()
        .find(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).expect("parse what the editor saw"))
        .expect("the editor reported what it saw");
    let windows = report["windows"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        windows.len(),
        2,
        "the editor did not open two windows: {report}"
    );

    // Two conversations, not one twice.
    assert_ne!(
        windows[0]["session"], windows[1]["session"],
        "both windows were given the same conversation: {report}"
    );
    // And each window was answered about its own work, which is the part the
    // person sees: a window whose session the door forgot is a window that
    // says nothing back.
    for (window, wrote) in windows.iter().zip([ONE, TWO]) {
        assert_eq!(
            window["stop"], "end_turn",
            "a window's turn did not end: {report}"
        );
        assert!(
            window["text"].as_str().unwrap_or_default().contains(wrote),
            "the window was not answered about its own work ({wrote}): {report}"
        );
    }

    // Closing one window closes one window. The conversation stays in the
    // record - a close is not a delete - but this door stops holding it, and
    // the agent it was running stops with it rather than living on until the
    // whole editor exits.
    let closed = &report["closed"];
    assert_eq!(
        closed["ok"], true,
        "the door would not close a window: {report}"
    );
    assert_eq!(
        closed["stillAnswers"], false,
        "the closed window went on answering: {report}"
    );
    assert_eq!(
        report["after"], "end_turn",
        "closing one window stopped the other: {report}"
    );

    // Both turns did their work, in the profile's own directory.
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let workspace = std::fs::read_dir(&workspaces)
        .expect("read the workspaces directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .expect("the profile has a workspace");
    for wrote in [ONE, TWO] {
        assert!(
            workspace.join("outbox").join(wrote).exists(),
            "the window that wrote {wrote} left nothing behind"
        );
    }
}

/// A person closes their editor and comes back to the same conversation.
///
/// Before this, every launch of the editor was a new conversation: the id the
/// door handed out died with the process, so a person who worked from their
/// editor on Monday and again on Tuesday had two lanes in the record and no
/// way to join them. An editor keeps the id its agent gave it - that is what
/// ACP's `session/load` is for - so the id has to be the lane's own, and the
/// door has to be able to hand the conversation back.
///
/// What this proves is the harness's half of it: one lane, one id, and what
/// was said replayed to the editor that reopened it. Whether the agent itself
/// remembers is the agent's business - a real one that advertises this does,
/// and the fixture here keeps no memory and says so.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: set up, work in the editor, close it, come back, read the page"
)]
fn a_person_closes_their_editor_and_comes_back_to_the_same_conversation() {
    const BEFORE: &str = "before-closing.txt";
    const AFTER: &str = "after-coming-back.txt";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let editor_client = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/acp-client");
    if !editor_client.join("node_modules").is_dir() {
        eprintln!(
            "skipped: the editor's own SDK is not installed. \
             scripts/gate.sh installs it; by hand it is \
             `npm install --prefix {}`",
            editor_client.display()
        );
        return;
    }
    let data_root = fresh_data_root("agent-editor-again");
    declare_the_fixture_agent(&data_root);

    let (product, url) = start_product(&data_root);
    let setup = Command::new(&node)
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/editor_door_setup_driver.mjs"))
        .arg(&url)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the editor setup driver");
    product.stop();
    let walked = String::from_utf8_lossy(&setup.stdout).into_owned();
    println!("{walked}");
    assert!(
        setup.status.success() && walked.contains("editor setup OK"),
        "the person could not set their agent up:\n{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let profile = walked
        .lines()
        .find(|line| line.starts_with("{\"profile\""))
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|report| report["profile"].as_str().map(str::to_owned))
        .expect("the driver said which profile it made");

    let opened_in_the_editor = data_root.join("some-other-project");
    std::fs::create_dir_all(&opened_in_the_editor).expect("create the directory the editor opened");
    // Each call is a launch of the editor: its own process, its own
    // connection, exactly as a person closing their editor and opening it
    // again tomorrow. `load` is the id this editor kept from last time.
    let launch_the_editor = |say: serde_json::Value, load: &str| -> serde_json::Value {
        let editor = Command::new(&node)
            .arg(editor_client.join("editor.mjs"))
            .arg(env!("CARGO_BIN_EXE_swem"))
            .arg(&profile)
            .arg(&opened_in_the_editor)
            .arg(say.to_string())
            .arg("allow")
            .arg(load)
            .env("XDG_DATA_HOME", &data_root)
            .env("LOCALAPPDATA", &data_root)
            .output()
            .expect("run the editor");
        let seen = String::from_utf8_lossy(&editor.stdout).into_owned();
        println!("editor: {seen}");
        assert!(
            editor.status.success(),
            "the editor could not work with SWEM:\n{}",
            String::from_utf8_lossy(&editor.stderr)
        );
        seen.lines()
            .find(|line| line.starts_with('{'))
            .map(|line| serde_json::from_str(line).expect("parse what the editor saw"))
            .expect("the editor reported what it saw")
    };

    // Monday: a new conversation, and the editor keeps what it was given.
    let first = launch_the_editor(
        serde_json::json!({ "write": format!("outbox/{BEFORE}"), "text": "said before closing" }),
        "",
    );
    assert_eq!(
        first["loadSession"], true,
        "the door did not tell the editor it can hand a session back: {first}"
    );
    assert_eq!(first["stop"], "end_turn", "the turn did not end: {first}");
    let kept = first["session"]
        .as_str()
        .expect("the editor was given a session id")
        .to_owned();

    // On Monday there was nothing to offer yet.
    assert_eq!(
        first["listed"].as_array().map(Vec::len),
        Some(0),
        "the door offered a conversation before there was one: {first}"
    );

    // Tuesday: the same id, handed back. The editor is given the conversation
    // before its load is answered, which is how it draws the history a person
    // left behind.
    let again = launch_the_editor(
        serde_json::json!({ "write": format!("outbox/{AFTER}"), "text": "said after coming back" }),
        &kept,
    );
    assert_eq!(
        again["session"].as_str(),
        Some(kept.as_str()),
        "the editor was moved to another conversation: {again}"
    );
    // And before it handed anything back, the editor asked what conversations
    // there are - which is what an editor with nothing kept has to do - and a
    // person can recognise this one, because it is offered by what was said in
    // it rather than by the id it has inside.
    let listed = again["listed"].as_array().cloned().unwrap_or_default();
    assert_eq!(
        listed.len(),
        1,
        "the door offered {} conversations, not the one there is: {again}",
        listed.len()
    );
    assert_eq!(
        listed[0]["session"].as_str(),
        Some(kept.as_str()),
        "the conversation offered is not the one that exists: {again}"
    );
    assert!(
        listed[0]["title"]
            .as_str()
            .unwrap_or_default()
            .contains(BEFORE),
        "the conversation is offered by its id rather than by what was said: {again}"
    );

    let replayed = again["replayed"].as_array().cloned().unwrap_or_default();
    assert!(
        !replayed.is_empty(),
        "the editor loaded the session and was told nothing that was said in it: {again}"
    );
    assert!(
        replayed
            .iter()
            .any(|update| update["text"].as_str().unwrap_or_default().contains(BEFORE)),
        "the replay does not carry what was said before the editor closed: {again}"
    );
    assert_eq!(
        again["stop"], "end_turn",
        "the second turn did not end: {again}"
    );

    // Both turns did their work, in the profile's own directory.
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let workspace = std::fs::read_dir(&workspaces)
        .expect("read the workspaces directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.is_dir())
        .expect("the profile has a workspace");
    for wrote in [BEFORE, AFTER] {
        assert!(
            workspace.join("outbox").join(wrote).exists(),
            "the turn that wrote {wrote} left nothing behind"
        );
    }

    // And in the page it is one conversation, not one per launch.
    let (page, url) = start_product(&data_root);
    let read = Command::new(&node)
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/editor_door_resume_driver.mjs"))
        .arg(&url)
        .arg(&profile)
        .arg(&kept)
        .arg(BEFORE)
        .arg(AFTER)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the editor resume driver");
    page.stop();
    let saw = String::from_utf8_lossy(&read.stdout).into_owned();
    println!("{saw}");
    assert!(
        read.status.success() && saw.contains("editor resume OK"),
        "the page does not show one conversation:\n{}",
        String::from_utf8_lossy(&read.stderr)
    );
}

/// A person puts their agent somewhere it cannot read their machine, and goes
/// on working with it.
///
/// The product has had exactly one way of running an agent: a process on this
/// machine, with the person's own files around it. This is the second row of
/// that catalogue - a container holding the profile's directory and nothing
/// else, with no network - chosen in the page the way the permission choice
/// beside it is chosen, and then used without anything else about the page
/// changing.
///
/// What makes the claim real rather than configured: the agent reports the
/// path it sees, which is the container's `/workspace`, and the bytes come
/// out into the person's own directory on this machine.
#[test]
#[ignore = "product gate: starts the real binary, drives a real browser and runs a container"]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: the image, the directories a person would already have, the page, the disk"
)]
fn a_person_puts_their_agent_in_a_container_and_it_still_works() {
    const WROTE: &str = "in-the-box.txt";
    const ANSWER: &str = "written from inside a container";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    // The image a container environment runs an agent in. A machine that can
    // reach a registry pulls one; this builds the same thing from what is
    // already here, which is what makes the walk runnable where the proxy
    // refuses every registry.
    let built = Command::new("sh")
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../scripts/build-agent-container-image.sh"),
        )
        .output()
        .expect("run the image build script");
    if !built.status.success() {
        eprintln!(
            "skipped: no image to run an agent in: {}",
            String::from_utf8_lossy(&built.stderr).trim()
        );
        return;
    }
    let image = String::from_utf8_lossy(&built.stdout).trim().to_owned();
    assert!(
        image.starts_with("sha256:"),
        "the image build did not report a pinned image: {image}"
    );

    let before = swem_containers();
    let data_root = fresh_data_root("agent-container");
    declare_the_fixture_agent(&data_root);

    // A container will not run an agent as the machine's root, and this gate
    // runs as root. So the directories the profile will be given are made
    // first, owned by an ordinary person - which is what they would be on a
    // machine somebody works on. The product reuses them rather than making
    // its own.
    let workbench = data_root.join("SWEM").join("workbench");
    let workspace = workbench.join("workspaces").join("hands");
    let agent_home = workbench.join("agent-homes").join("hands");
    for directory in [&workspace, &agent_home] {
        std::fs::create_dir_all(directory).expect("make the profile's directory");
        let chowned = Command::new("chown")
            .arg("-R")
            .arg("1000:1000")
            .arg(directory)
            .status();
        if !chowned.is_ok_and(|status| status.success()) {
            eprintln!("skipped: cannot give a profile's directory to an ordinary user");
            return;
        }
    }

    let mut arguments = vec![
        "workbench".to_owned(),
        "serve".to_owned(),
        "--no-open".to_owned(),
        "--port".to_owned(),
        "0".to_owned(),
        "--container-image".to_owned(),
        image.clone(),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_swem"))
        .args(&mut arguments)
        .env("XDG_DATA_HOME", &data_root)
        .env("LOCALAPPDATA", &data_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start the swem binary");
    let stdout = child.stdout.take().expect("product stdout");
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + Duration::from_mins(1);
    let mut url = None;
    while Instant::now() < deadline {
        let Some(Ok(line)) = lines.next() else { break };
        println!("product: {line}");
        if let Some(found) = line.strip_prefix("SWEM Workbench: ") {
            std::thread::spawn(move || {
                for line in lines.map_while(Result::ok) {
                    println!("product: {line}");
                }
            });
            url = Some(found.trim().to_owned());
            break;
        }
    }
    let product = Product { child };
    let url = url.expect("the product never printed its Workbench URL");

    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/agent_in_a_container_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(swem_host::IN_A_CONTAINER)
        .arg(WROTE)
        .arg(ANSWER)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the container driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("in a container OK"),
        "the agent did not work from inside a container:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The bytes on this machine. The agent never saw this path - it wrote to
    // `/workspace/outbox` - so finding them here is the bind, the container
    // and the profile's own directory all being the one thing.
    assert_eq!(
        std::fs::read_to_string(workspace.join("outbox").join(WROTE))
            .expect("the container's file is in the profile's workspace"),
        ANSWER
    );

    // And the container is gone: it exists while the agent does and not a
    // moment longer. Compared against what was there before this walk rather
    // than against nothing, because another run on this machine may be
    // holding its own.
    let after = swem_containers();
    let left: Vec<&String> = after.difference(&before).collect();
    assert!(
        left.is_empty(),
        "closing the session left containers behind: {left:?}"
    );
}

/// A page on another site cannot work the Workbench.
///
/// The Workbench listens on loopback and asks nothing of whoever calls it: no
/// token, no `Origin`. Loopback is not a boundary a browser respects - any
/// page a person visits can post to `http://127.0.0.1:<port>/api/...`, and the
/// body is parsed whatever the content type says, so a plain cross-site form
/// needs no preflight to get through. Behind that door are a live terminal and
/// a profile's secrets.
#[test]
#[ignore = "live product test: starts the real binary"]
fn a_page_on_another_site_cannot_work_the_workbench() {
    require_the_binaries_cargo_does_not_build();
    let data_root = fresh_data_root("front-door-lock");
    let (product, url) = start_product(&data_root);
    // The address the product prints carries this run's secret; the door
    // itself is the origin under it.
    let api = url.split_once("/?").map_or_else(
        || url.trim_end_matches('/').to_owned(),
        |(origin, _)| origin.to_owned(),
    );

    // Reading. A foreign page must not learn which agents this person has.
    let (status, body) = knock(
        &format!("{api}/api/profiles"),
        &["-H", "Origin: https://not-the-workbench.example"],
    );
    assert_eq!(
        status, 403,
        "a foreign page read the profile inventory: {body}"
    );

    // Writing, shaped the way a cross-site form is: a content type that needs
    // no preflight, carrying the JSON the route parses anyway.
    let (status, body) = knock(
        &format!("{api}/api/terminals"),
        &[
            "-X",
            "POST",
            "-H",
            "Origin: https://not-the-workbench.example",
            "-H",
            "Content-Type: text/plain;charset=UTF-8",
            "--data",
            r#"{"profile_id":"any"}"#,
        ],
    );
    assert_eq!(
        status, 403,
        "a foreign page reached the terminal route: {body}"
    );
    assert!(
        !body.contains("profile"),
        "the refusal told a foreign page what the route wanted: {body}"
    );

    // Another program on this machine, stating no origin at all: not a
    // browser cross-site request, and still not this Workbench's caller.
    let (status, body) = knock(&format!("{api}/api/profiles"), &[]);
    assert_eq!(
        status, 403,
        "a program that holds no secret read the profile inventory: {body}"
    );

    // The address the product printed carries the run's secret, and what a
    // person opens is what works.
    let (status, body) = knock(&url, &["-i"]);
    assert_eq!(status, 200, "the printed address did not open: {body}");
    let secret = url
        .split_once("?token=")
        .expect("the product prints an address carrying this run's secret")
        .1
        .to_owned();
    assert!(
        body.contains("set-cookie: swem_session_"),
        "the page was not handed the secret to keep: {body}"
    );
    let (status, body) = knock(
        &format!("{api}/api/profiles"),
        &["-H", &format!("X-Swem-Session: {secret}")],
    );
    assert_eq!(status, 200, "the secret did not open the door: {body}");

    // The same address without it does not, so the secret is the thing that
    // opens the door and not the address.
    let (status, _) = knock(&format!("{api}/?token=not-the-secret"), &[]);
    assert_eq!(status, 403, "any token opened the page");

    product.stop();
    let _ = std::fs::remove_dir_all(&data_root);
}

/// Before anybody came in: the page is there for anybody, nothing behind it
/// is, whatever a caller states of itself.
fn nothing_behind_the_door_answers(address: &str) {
    let (status, _) = knock(&format!("{address}/"), &[]);
    assert_eq!(status, 200, "the door itself did not open");
    for route in [
        "api/profiles",
        "api/stream",
        "api/terminals",
        "api/access/standing",
    ] {
        let (status, body) = knock(&format!("{address}/{route}"), &[]);
        assert_eq!(
            status, 401,
            "{route} answered somebody who never came in: {body}"
        );
        assert!(
            !body.contains("profile"),
            "the refusal said what the route holds: {body}"
        );
    }
    let (status, body) = knock(
        &format!("{address}/api/profiles"),
        &["-H", "X-Swem-Session: whatever-a-run-would-have-minted"],
    );
    assert_eq!(status, 401, "the secret of a run opened an address: {body}");
    let (status, body) = knock(
        &format!("{address}/api/access"),
        &["-H", "Origin: https://not-the-workbench.example"],
    );
    assert_eq!(status, 403, "another site's page was answered: {body}");
    let (status, body) = knock(
        &format!("{address}/api/access/register/begin"),
        &[
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "--data",
            r#"{"word":"aaaa-bbbb-cccc-dddd","name":"A guess"}"#,
        ],
    );
    assert_eq!(
        status, 403,
        "a word nobody was given began a ceremony: {body}"
    );
}

/// A person puts the Workbench at an address and comes to it with a
/// passkey, and nobody else comes in.
///
/// The first start prints a word. With it a person registers the device
/// they sit at and is shown codes to come back with; they make a token for
/// a program, sign out, and sign in with the passkey. From a second browser,
/// which holds nothing, they are refused, come back with a code and register
/// that device. Everything behind the door is tried from outside, as a
/// program would: with nothing, with the secret of a run, as another site's
/// page, and with a token that may do one thing.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_comes_to_their_workbench_from_elsewhere_with_a_passkey() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("door");
    let (product, address, word) = start_product_at_an_address(&data_root);

    nothing_behind_the_door_answers(&address);

    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/door_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&address)
        .arg(&word)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the door driver");
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("door OK"),
        "a person did not come in with a passkey:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The token the person made may say that something is due and nothing
    // else. It was shown once, on the page, and is what the driver read.
    let token = walked
        .lines()
        .find_map(|line| line.strip_prefix("token: "))
        .expect("the driver read the token the page showed")
        .trim()
        .to_owned();
    let bearer = format!("Authorization: Bearer {token}");
    for route in ["api/profiles", "api/access/standing", "api/terminals"] {
        let (status, body) = knock(&format!("{address}/{route}"), &["-H", &bearer]);
        assert_eq!(status, 403, "a token that may knock opened {route}: {body}");
    }

    // What it may do, it does: it knocks, the Workbench looks, and with
    // nothing due nothing is said. Without the token a knock is nobody's.
    let (status, body) = knock(
        &format!("{address}/api/time/due"),
        &["-X", "POST", "-H", &bearer],
    );
    assert_eq!(
        status, 200,
        "the token that may knock was not let knock: {body}"
    );
    assert!(
        body.contains(r#""said":0"#),
        "a knock with nothing due was not answered as one: {body}"
    );
    let (status, body) = knock(&format!("{address}/api/time/due"), &["-X", "POST"]);
    assert_eq!(status, 401, "a knock by nobody was answered: {body}");

    // Nothing that opens is kept as it was said.
    let access = find_directory(&data_root, "access").expect("the product kept who may come in");
    let mut kept = Vec::new();
    for entry in std::fs::read_dir(&access).expect("read it").flatten() {
        kept.extend(std::fs::read(entry.path()).unwrap_or_default());
    }
    let kept = String::from_utf8_lossy(&kept);
    for said in [&word, &token] {
        assert!(
            !kept.contains(said.as_str()),
            "what opens the Workbench is kept as it was said"
        );
    }

    product.stop();
    let _ = std::fs::remove_dir_all(&data_root);
}

/// The example built beside this crate's binaries by `scripts/gate.sh`: a
/// product that is not SWEM with the harness built in.
fn the_example_product() -> PathBuf {
    let example = Path::new(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .join("examples")
        .join(if cfg!(windows) {
            "built_in.exe"
        } else {
            "built_in"
        });
    assert!(
        example.is_file(),
        "this walk needs the example product, which cargo test does not build. Run:\n  \
         cargo build -p swem-host --example built_in\nscripts/gate.sh does this and the rest."
    );
    example
}

/// Somebody builds the harness into a product of their own: their server,
/// their people, their name. Each person finds a Workbench of their own
/// under a path of that server, and nobody finds another's.
#[test]
#[ignore = "product gate: starts the example product and drives a real browser"]
#[allow(
    clippy::too_many_lines,
    reason = "one walk over one product: its people, its server, and the Store's kind of its own"
)]
fn a_product_of_ones_own_has_the_harness_built_in() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let example = the_example_product();
    let root = fresh_data_root("built-in");
    // Each person's data root has the agent they may make theirs.
    for person in ["ada", "bo"] {
        let agents = root.join(person).join("agents");
        std::fs::create_dir_all(&agents).expect("the agents directory");
        let hands = Path::new(env!("CARGO_BIN_EXE_swem"))
            .parent()
            .expect("a directory")
            .join(if cfg!(windows) {
                "swem-hands-agent.exe"
            } else {
                "swem-hands-agent"
            });
        std::fs::write(
            agents.join("hands.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "id": "hands",
                "name": "My own agent",
                "command": hands.display().to_string(),
            }))
            .expect("a declaration"),
        )
        .expect("write the declaration");
    }
    let server = Path::new(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("a directory")
        .join(if cfg!(windows) {
            "swem-mcp-echo.exe"
        } else {
            "swem-mcp-echo"
        });
    let mut child = Command::new(&example)
        .env("SWEM_EXAMPLE_ROOT", &root)
        .env("SWEM_EXAMPLE_SERVER", &server)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start the example product");
    let stdout = child.stdout.take().expect("the example's stdout");
    let product = Product { child };
    let mut lines = BufReader::new(stdout).lines();
    let (mut address, mut ada, mut bo) = (None, None, None);
    let deadline = Instant::now() + Duration::from_mins(1);
    while Instant::now() < deadline && bo.is_none() {
        let Some(Ok(line)) = lines.next() else { break };
        println!("example: {line}");
        if let Some(found) = line.strip_prefix("Example: ") {
            address = Some(found.trim().to_owned());
        } else if let Some(found) = line.strip_prefix("Ada comes in at: ") {
            ada = Some(found.trim().to_owned());
        } else if let Some(found) = line.strip_prefix("Bo comes in at: ") {
            bo = Some(found.trim().to_owned());
        }
    }
    // Keep reading: a pipe dropped here would kill the example mid-walk.
    std::thread::spawn(move || {
        for line in lines.map_while(Result::ok) {
            println!("example: {line}");
        }
    });
    let (address, ada, bo) = (
        address.expect("the example printed its address"),
        ada.expect("the example printed Ada's way in"),
        bo.expect("the example printed Bo's way in"),
    );
    let adas_agents = format!("{address}people/ada/agents/");

    // Two helpers for the product's Store: one that answers the shape the
    // product calls (the echo fixture), one that does not (the taker
    // fixture, whose tools are other tools). Each is an archive whose tree
    // has a `helper.json` naming the program, as the example reads them.
    let supply = root.join("supply");
    std::fs::create_dir_all(&supply).expect("the supply");
    let beside = Path::new(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("a directory")
        .to_path_buf();
    let fixture = |name: &str| {
        beside.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        })
    };
    let mut entries = Vec::new();
    for (id, program, args) in [
        (
            "echoes",
            fixture("swem-mcp-echo"),
            vec![
                "--receipt".to_owned(),
                supply.join("echoes-receipt.json").display().to_string(),
            ],
        ),
        (
            "takes",
            fixture("swem-mcp-taker-fixture"),
            vec![
                "--home".to_owned(),
                supply.join("takes-home").display().to_string(),
            ],
        ),
    ] {
        assert!(
            program.is_file(),
            "build the fixtures first: cargo build -p swem-host --bins"
        );
        let tree = supply.join(id);
        std::fs::create_dir_all(&tree).expect("the helper's tree");
        std::fs::write(
            tree.join("helper.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"program": program.display().to_string(), "args": args}),
            )
            .expect("a manifest"),
        )
        .expect("write the manifest");
        let archive = supply.join(format!("{id}.tar.gz"));
        let tarred = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&supply)
            .arg(id)
            .status()
            .expect("run tar");
        assert!(tarred.success(), "packaging the helper failed");
        entries.push(serde_json::json!({
            "kind": "example/helper@1", "id": id, "name": format!("The helper that {id}"), "version": "0.1.0",
            "distribution": {"archive": {"url": format!("file://{}", archive.display()), "sha256": sha256_of(&archive)}}
        }));
    }
    let catalog = supply.join("catalog.json");
    std::fs::write(
        &catalog,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "swem:catalog@0.2", "name": "Helpers for the example", "entries": entries
        }))
        .expect("serialize the catalog"),
    )
    .expect("write the catalog");

    // From outside, as a program: nobody the product did not let in gets
    // anything of the harness, at the page or behind it.
    for route in ["", "api/profiles", "api/access", "workbench.js"] {
        let (status, _) = knock(&format!("{adas_agents}{route}"), &[]);
        assert_eq!(
            status, 403,
            "{route} answered somebody the product never let in"
        );
    }

    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/built_in_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&ada)
        .arg(&bo)
        .arg(&adas_agents)
        .arg(format!("file://{}", catalog.display()))
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("built in OK"),
        "the product's people did not find their own Workbenches:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A person gives their own agent a role and a model, and the agent gets
/// them. The role is a file in the agent's working directory - the only
/// place every agent reads - and the fixture agent reads that file back when
/// asked; the model is asked for through a provider the person declared
/// first, and the profile keeps both on disk under a revision.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_gives_an_agent_a_role_and_a_model() {
    const ROLE: &str = "You are Ada, the hands of this studio. Be brief.";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("agent-setup");
    declare_the_fixture_agent(&data_root);

    let (product, url) = start_product(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/agent_setup_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(ROLE)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the setup driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("setup OK"),
        "the setup walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The profile keeps what the person gave it, under a revision.
    let profiles = find_directory(&data_root, "profiles").expect("the product made profiles");
    let profile: serde_json::Value = serde_json::from_slice(
        &std::fs::read(profiles.join("hands").join("profile.json"))
            .expect("the profile is on disk"),
    )
    .expect("the profile is JSON");
    assert_eq!(profile["model_provider"], "desk", "{profile}");
    assert_eq!(profile["model"], "quality", "{profile}");
    assert_eq!(profile["role"], ROLE, "{profile}");
    assert!(profile["revision"].as_u64().unwrap_or(0) >= 3, "{profile}");

    // The provider is a document beside the other things a person declares.
    let providers =
        find_directory(&data_root, "model-providers").expect("the product keeps providers");
    let provider: serde_json::Value = serde_json::from_slice(
        &std::fs::read(providers.join("desk.json")).expect("the provider is on disk"),
    )
    .expect("the provider is JSON");
    assert_eq!(provider["base_url"], "http://127.0.0.1:8000/v1");

    // And the role is where the agent read it from: AGENTS.md in its
    // working directory, in SWEM's span, with the person's words in it.
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let instructions = std::fs::read_to_string(workspaces.join("hands").join("AGENTS.md"))
        .expect("the role was written for the agent");
    assert!(instructions.contains(ROLE), "{instructions}");
    assert!(
        instructions.starts_with("<!-- swem:system -->"),
        "{instructions}"
    );
}

/// A person installs a server from the Store that declares a home App, and
/// the Workbench offers it as a space beside its own: choosing it mounts the
/// server's App in the main area. The server is the Apps fixture with
/// `--home`; the catalog is one this gate writes; nothing of the server is
/// compiled into the product, and the host keeps no registry of spaces.
#[test]
#[ignore = "product gate: needs a browser and node"]
fn a_person_installs_a_server_with_a_home_app_and_opens_its_space() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let Some(platform) = swem_host::registry_platform() else {
        eprintln!("skipped: no registry platform for this machine");
        return;
    };
    let data_root = fresh_data_root("space");
    let registry = fixture_agent_registry(&data_root, "claude-acp", platform);
    let supply = data_root.join("supply");
    let beside = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .to_path_buf();
    let fixture_name = if cfg!(windows) {
        "swem-mcp-apps-fixture.exe"
    } else {
        "swem-mcp-apps-fixture"
    };
    assert!(
        beside.join(fixture_name).is_file(),
        "build the Apps fixture first: cargo build -p swem-host --bin swem-mcp-apps-fixture"
    );
    let archive = supply.join("notes.tar.gz");
    std::fs::create_dir_all(&supply).expect("create the supply");
    let tarred = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&beside)
        .arg(fixture_name)
        .status()
        .expect("run tar");
    assert!(tarred.success(), "packaging the Apps fixture failed");
    let catalog = supply.join("catalog.json");
    std::fs::write(
        &catalog,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "swem:catalog@0.1",
            "name": "This gate's catalog",
            "entries": [
                {"kind": "server", "id": "notes", "name": "Notes", "version": "0.1.0-gate",
                 "description": "a note board with a home App",
                 "distribution": {"binary": {platform: {
                     "archive": format!("file://{}", archive.display()),
                     "sha256": sha256_of(&archive),
                     "cmd": fixture_name,
                     "args": ["--receipt", supply.join("notes-receipt.json").display().to_string(),
                              "--poison", supply.join("notes-poison.json").display().to_string(),
                              "--home"]}}}}
            ]
        }))
        .expect("serialize the catalog"),
    )
    .expect("write the catalog");
    let (product, url) = start_product_against(&data_root, Some(&registry.index_url));
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/space_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(format!("file://{}", catalog.display()))
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the space driver");
    product.stop();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("space OK"),
        "the walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The server was installed under the install root, once, and declared
    // from there: nothing beside the product was written by hand.
    assert!(
        find_directory(&data_root, "notes")
            .is_some_and(|dir| under_install_root(&dir, &data_root, "servers", "notes")),
        "the server did not land under the install root"
    );
}

/// Every file under a data root but the keys, which are the one place a
/// secret belongs.
fn files_outside_the_keys(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "keys") {
                continue;
            }
            files_outside_the_keys(&path, into);
        } else {
            into.push(path);
        }
    }
}

/// A person reaches their agent from a messenger: they add a bot on
/// Providers → Channels with the token the messenger gave them, say the
/// code shown there to the bot once, and from then on what they write to
/// the bot is a chat with their agent - on the page as well, marked as from
/// the messenger - and the agent's answer is drafted while it is written and
/// sent when done. A stranger gets one line and no chat.
///
/// The messenger is a Bot API that is a fixture, started here: the Telegram
/// channel that ships with the product talks to it at the "Bot API address"
/// the walk types on the page, which is the setting a local Bot API server
/// needs too. A real bot is tried by hand, by whoever has one.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_reaches_their_agent_from_a_messenger() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("messenger");
    declare_the_fixture_agent(&data_root);

    // The messenger, as far as a bot can tell: on loopback, printing its
    // address on one line.
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .join(if cfg!(windows) {
            "swem-telegram-api-fixture.exe"
        } else {
            "swem-telegram-api-fixture"
        });
    let mut api = Command::new(&fixture)
        .arg("--files")
        .arg(data_root.join("messenger-files"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let api_address = {
        let stdout = api.stdout.take().expect("the fixture's stdout");
        BufReader::new(stdout)
            .lines()
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };

    let (product, url) = start_product(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/messenger_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(&api_address)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the messenger driver");
    product.stop();
    let _ = api.kill();
    let _ = api.wait();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("messenger OK"),
        "the messenger walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The bot's token went to the keys and nowhere else: not into the
    // channel's document, the ledger, a log or the chat.
    let mut files = Vec::new();
    files_outside_the_keys(&data_root, &mut files);
    assert!(
        !files.is_empty(),
        "the product wrote nothing under its root"
    );
    for path in files {
        let bytes = std::fs::read(&path).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("walk-not-a-token"),
            "the token is in {}",
            path.display()
        );
    }
}

/// The messenger beyond direct messages: the bot in a group is a chat of
/// several, and whoever writes in it is in it; a forum topic is a chat of
/// its own, answered in the topic; somebody alone with the bot waits until
/// the person lets them into a chat from the page, and is then reached
/// where they wrote from; the agent's question arrives as buttons and the
/// answer pressed there lets the agent go on; a file that came with a
/// message is read by the agent, and what the agent puts in its outbox
/// goes back through the bot.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_messenger_carries_a_group_a_guest_a_question_and_files() {
    const SECRET: &str = "what-was-in-the-file-2b9c";
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("messenger-more");
    declare_the_fixture_agent(&data_root);

    // The messenger holds a file somebody sent the bot, under its id.
    let files = data_root.join("messenger-files");
    std::fs::create_dir_all(&files).expect("the messenger's files");
    std::fs::write(files.join("doc1"), SECRET).expect("the file somebody sent");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .join(if cfg!(windows) {
            "swem-telegram-api-fixture.exe"
        } else {
            "swem-telegram-api-fixture"
        });
    let mut api = Command::new(&fixture)
        .arg("--files")
        .arg(&files)
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let api_address = {
        let stdout = api.stdout.take().expect("the fixture's stdout");
        BufReader::new(stdout)
            .lines()
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };

    let (product, url) = start_product(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/messenger_more_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(&api_address)
        .arg(SECRET)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the messenger driver");
    product.stop();
    let _ = api.kill();
    let _ = api.wait();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("messenger more OK"),
        "the messenger walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The file that came with the message is in the agent's inbox, and what
    // the agent handed back is in its outbox - the same two places the page
    // shows.
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let inbox = std::fs::read_to_string(workspaces.join("hands").join("inbox").join("notes.txt"))
        .expect("the file reached the agent's inbox");
    assert_eq!(inbox, SECRET);
    assert!(
        workspaces
            .join("hands")
            .join("outbox")
            .join("reply.txt")
            .is_file(),
        "what the agent wrote is not in its outbox"
    );
}

/// A Workbench served at an address has a door the messenger delivers to,
/// instead of being asked. The person switches the bot to it on the
/// Channels page; the messenger is told the door and a secret; a delivery
/// at the door reaches the chat, one without the secret is refused, and
/// nothing is asked for meanwhile; the bot is switched back. On a Workbench
/// not served at an address the page says why there is no door (the first
/// messenger walk checks that).
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_messenger_delivers_to_the_door_of_a_served_workbench() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("messenger-door");
    declare_the_fixture_agent(&data_root);
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .join(if cfg!(windows) {
            "swem-telegram-api-fixture.exe"
        } else {
            "swem-telegram-api-fixture"
        });
    let mut api = Command::new(&fixture)
        .arg("--files")
        .arg(data_root.join("messenger-files"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let api_address = {
        let stdout = api.stdout.take().expect("the fixture's stdout");
        BufReader::new(stdout)
            .lines()
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };

    let (product, address, word) = start_product_at_an_address(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/messenger_door_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&address)
        .arg(&word)
        .arg(&api_address)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the messenger door driver");
    product.stop();
    let _ = api.kill();
    let _ = api.wait();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("messenger door OK"),
        "the messenger door walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The Workbench's page inside the messenger: the bot of a served Workbench
/// offers it with one button; opened from the bot, it knows who you are by
/// the messenger's signature, takes a file bigger than the messenger does
/// to the agent, shows what the agent put out and the chat as it stands.
/// Opened by nobody it says so; a signature that is not the messenger's is
/// refused.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_opens_the_workbench_inside_the_messenger() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("messenger-app");
    declare_the_fixture_agent(&data_root);
    // A file bigger than the messenger carries: 60 MB, where Telegram's bots
    // take 20 in and give 50 out.
    let big = data_root.join("from-the-phone").join("notes-big.bin");
    std::fs::create_dir_all(big.parent().expect("a parent")).expect("the phone's directory");
    let bytes: Vec<u8> = (0..60u32 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(&big, &bytes).expect("the big file");

    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .join(if cfg!(windows) {
            "swem-telegram-api-fixture.exe"
        } else {
            "swem-telegram-api-fixture"
        });
    let mut api = Command::new(&fixture)
        .arg("--files")
        .arg(data_root.join("messenger-files"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let api_address = {
        let stdout = api.stdout.take().expect("the fixture's stdout");
        BufReader::new(stdout)
            .lines()
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };

    // The page as it is hosted anywhere: the committed copy, served from
    // an origin of its own, as a static host would.
    let hosted = serve_one_file_as(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/mini-app/dist/index.html"),
        "text/html;charset=utf-8",
    );
    let (product, address, word) = start_product_at_an_address(&data_root);
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/messenger_app_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&address)
        .arg(&word)
        .arg(&api_address)
        .arg(&big)
        .arg(&hosted)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the messenger app driver");
    product.stop();
    let _ = api.kill();
    let _ = api.wait();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("messenger app OK"),
        "the messenger app walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The big file reached the agent's inbox whole.
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let arrived = std::fs::read(workspaces.join("hands").join("inbox").join("notes-big.bin"))
        .expect("the big file reached the agent's inbox");
    assert_eq!(arrived.len(), bytes.len());
    assert_eq!(arrived, bytes, "the file arrived changed");
}

/// A Workbench on a laptop has no address from outside, and the page a bot
/// opens inside the messenger needs one. The person installs a tunnel from
/// the Store (here a fixture that stands at the Workbench's own gate, so
/// the walk really goes through the gate), sends /app to the bot, and the
/// bot's button opens the page through the tunnel: who you are, 60 MB in.
/// The gate answers the page and nothing else. Before the tunnel, the page
/// and the bot say what to do; closed from the Channels page, the page says
/// to send /app again.
#[test]
#[ignore = "product gate: starts the real binary and drives a real browser"]
fn a_person_opens_the_workbench_inside_the_messenger_from_a_laptop() {
    let _serial = one_at_a_time();
    let (Some(browser), Some(node)) = (browser(), node()) else {
        eprintln!("skipped: no browser or node on this machine");
        return;
    };
    let data_root = fresh_data_root("messenger-tunnel");
    declare_the_fixture_agent(&data_root);
    let big = data_root.join("from-the-phone").join("notes-big.bin");
    std::fs::create_dir_all(big.parent().expect("a parent")).expect("the phone's directory");
    let bytes: Vec<u8> = (0..60u32 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    std::fs::write(&big, &bytes).expect("the big file");

    // The tunnel fixture, published as one bare executable in a catalog of
    // the test's own: the road a tool takes.
    let beside = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .to_path_buf();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let fixture = beside.join(format!("swem-tunnel-fixture{suffix}"));
    assert!(
        fixture.is_file(),
        "build the tunnel fixture first: cargo build -p swem-host --bin swem-tunnel-fixture"
    );
    let platform = swem_host::registry_platform().expect("a platform this gate runs on");
    let catalog = data_root.join("supply").join("catalog.json");
    std::fs::create_dir_all(catalog.parent().expect("a parent")).expect("the supply");
    std::fs::write(
        &catalog,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "swem:catalog@0.2",
            "name": "Tunnels for this gate",
            "entries": [{
                "kind": "swem/tunnel@1", "id": "nowhere", "name": "A tunnel to nowhere",
                "version": "0.1.0",
                "distribution": {"binary": {platform: {
                    "archive": format!("file://{}", fixture.display()),
                    "sha256": sha256_of(&fixture),
                    "cmd": format!("./swem-tunnel-nowhere{suffix}")}}}
            }]
        }))
        .expect("serialize the catalog"),
    )
    .expect("write the catalog");

    let hosted = serve_one_file_as(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/mini-app/dist/index.html"),
        "text/html;charset=utf-8",
    );
    let bot_api = beside.join(format!("swem-telegram-api-fixture{suffix}"));
    let mut api = Command::new(&bot_api)
        .arg("--files")
        .arg(data_root.join("messenger-files"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let api_address = {
        let stdout = api.stdout.take().expect("the fixture's stdout");
        BufReader::new(stdout)
            .lines()
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };

    let (product, url) = start_product(&data_root);
    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/messenger_tunnel_driver.mjs");
    let output = Command::new(&node)
        .arg(&driver)
        .arg(&url)
        .arg(&api_address)
        .arg(format!("file://{}", catalog.display()))
        .arg(&big)
        .arg(&hosted)
        .env("SWEM_BROWSER", &browser)
        .env("SWEM_BROWSER_NO_SANDBOX", "1")
        .output()
        .expect("run the messenger tunnel driver");
    product.stop();
    let _ = api.kill();
    let _ = api.wait();
    let walked = String::from_utf8_lossy(&output.stdout).into_owned();
    println!("{walked}");
    assert!(
        output.status.success() && walked.contains("messenger tunnel OK"),
        "the messenger tunnel walk did not finish:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let workspaces = find_directory(&data_root, "workspaces").expect("the product made workspaces");
    let arrived = std::fs::read(workspaces.join("hands").join("inbox").join("notes-big.bin"))
        .expect("the big file reached the agent's inbox through the gate");
    assert_eq!(arrived, bytes, "the file arrived changed");
}
