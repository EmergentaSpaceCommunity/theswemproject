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
    let missing: Vec<&str> = [
        (
            "swem-hands-agent",
            "cargo build -p swem-host --bin swem-hands-agent",
        ),
        (
            "swem-telegram-api-fixture",
            "cargo build -p swem-host --bin swem-telegram-api-fixture",
        ),
        (
            "swem-channel-telegram",
            "cargo build -p swem-channel-telegram",
        ),
    ]
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
    let deadline = Instant::now() + Duration::from_mins(1);
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

/// Two ports beside each other that nothing listens on: the Workbench's and
/// the one Apps are drawn at.
fn two_ports_beside_each_other() -> u16 {
    for _ in 0..50 {
        let Ok(first) = std::net::TcpListener::bind(("127.0.0.1", 0)) else {
            continue;
        };
        let Ok(port) = first.local_addr().map(|address| address.port()) else {
            continue;
        };
        if port < u16::MAX && std::net::TcpListener::bind(("127.0.0.1", port + 1)).is_ok() {
            return port;
        }
    }
    panic!("this machine gave no two ports beside each other");
}

/// The product, started at an address: whoever comes signs in. The address
/// is `localhost`, which a browser takes for a safe place, so the passkey
/// ceremony is the one a server's is.
///
/// Returns the running product, its address, and the word of a first start
/// it printed.
pub fn start_product_at_an_address(data_root: &Path) -> (Product, String, String) {
    install_the_distributions_packages();
    let port = two_ports_beside_each_other();
    let mut child = Command::new(env!("CARGO_BIN_EXE_swem"))
        .args([
            "workbench",
            "serve",
            "--no-open",
            "--at",
            &format!("http://localhost:{port}"),
        ])
        .env("XDG_DATA_HOME", data_root)
        .env("LOCALAPPDATA", data_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start the swem binary");
    let stdout = child.stdout.take().expect("product stdout");
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + Duration::from_mins(1);
    let mut address = None;
    while Instant::now() < deadline {
        let Some(Ok(line)) = lines.next() else { break };
        println!("product: {line}");
        if let Some(url) = line.strip_prefix("SWEM Workbench: ") {
            address = Some(url.trim().to_owned());
        }
        // The word stands on a line of its own: four groups of four.
        let word = line.trim();
        if let Some(address) = &address
            && word.len() == 19
            && word.split('-').count() == 4
            && word
                .chars()
                .all(|letter| letter == '-' || letter.is_ascii_alphanumeric())
        {
            let address = address.clone();
            let word = word.to_owned();
            std::thread::spawn(move || {
                for line in lines.map_while(Result::ok) {
                    println!("product: {line}");
                }
            });
            return (Product { child }, address, word);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("the product never printed its address and the word of a first start");
}

/// How long a gate's leavings are kept before the next run sweeps them: long
/// enough that a run under way, or one whose failure someone is still looking
/// at, is never touched.
pub const KEEP_LEAVINGS: Duration = Duration::from_hours(6);

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

/// An agent that says back what it was told, declared the same way. What
/// it says names whoever it was told to name, which is what makes two of
/// them in one chat go on answering each other.
pub fn declare_an_agent_that_says_it_back(data_root: &Path) {
    let echo = PathBuf::from(env!("CARGO_BIN_EXE_swem"))
        .parent()
        .expect("the product binary has a directory")
        .join(if cfg!(windows) {
            "swem-echo-agent.exe"
        } else {
            "swem-echo-agent"
        });
    assert!(
        echo.is_file(),
        "build the fixture agent first: cargo build -p swem-host --bin swem-echo-agent"
    );
    let agents = data_root.join("SWEM").join("workbench").join("agents");
    std::fs::create_dir_all(&agents).expect("create the agent directory");
    std::fs::write(
        agents.join("echo.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "id": "echo",
            "name": "Says it back",
            "command": echo.display().to_string(),
        }))
        .expect("serialize the declaration"),
    )
    .expect("write the agent declaration");
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
