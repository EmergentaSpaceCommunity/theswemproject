//! Hermetic Podman CLI double for the environment transport contract test.
//! It proves argv/lifecycle/stdio composition only; it is never isolation
//! evidence and is not used by production discovery.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(125)
        }
    }
}

fn run() -> Result<u8, String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("create") => create(&args[1..]),
        Some("start") => start(&args[1..]),
        Some("rm") => remove(&args[1..]),
        Some("container") if args.get(1).map(String::as_str) == Some("inspect") => {
            inspect(&args[2..])
        }
        Some("container") if args.get(1).map(String::as_str) == Some("exists") => {
            exists(&args[2..])
        }
        command => Err(format!("unsupported fixture Podman command {command:?}")),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the hermetic CLI double mirrors one linear create-to-inspect record and is not production policy code"
)]
fn create(args: &[String]) -> Result<u8, String> {
    require_flag(args, "--pull=never")?;
    require_flag(args, "--network=none")?;
    require_flag(args, "--unsetenv-all")?;
    let lease_id = option_value(args, "--label", "io.swem.lease-id=")?;
    let container_name = option(args, "--name")?;
    let instance_id = format!("{:x}", Sha256::digest(lease_id.as_bytes()));
    let image_index = args
        .iter()
        .position(|argument| argument.starts_with("sha256:") || argument.contains("@sha256:"))
        .ok_or_else(|| "pinned image argument is missing".to_owned())?;
    let image = &args[image_index];
    let image_digest = image
        .rsplit_once('@')
        .map_or(image.as_str(), |(_, digest)| digest);
    let inspected_agent_args = args[image_index + 1..].to_vec();
    let fixture_agent_index = inspected_agent_args
        .iter()
        .position(|argument| argument == "--fixture-host-agent")
        .ok_or_else(|| "fixture host agent marker is missing".to_owned())?;
    let fixture_agent = inspected_agent_args
        .get(fixture_agent_index + 1)
        .cloned()
        .ok_or_else(|| "fixture host agent path is missing".to_owned())?;
    let fixture_host_workspace = inspected_agent_args
        .iter()
        .position(|argument| argument == "--fixture-host-workspace")
        .and_then(|index| inspected_agent_args.get(index + 1))
        .cloned();
    let entrypoint: Vec<String> = serde_json::from_str(&option(args, "--entrypoint")?)
        .map_err(|error| format!("entrypoint JSON: {error}"))?;
    let agent_executable = entrypoint
        .first()
        .cloned()
        .ok_or_else(|| "entrypoint is empty".to_owned())?;
    let user = option(args, "--user")?;
    let workdir = option(args, "--workdir")?;
    let mount = option(args, "--mount")?;
    let mount_source = mount_component(&mount, "src")?;
    let mount_target = mount_component(&mount, "dst")?;
    let memory = option(args, "--memory")?
        .trim_end_matches('m')
        .parse::<u64>()
        .map_err(|error| format!("memory: {error}"))?
        * 1_048_576;
    let nano_cpus = option(args, "--cpus")?
        .parse::<u64>()
        .map_err(|error| format!("cpus: {error}"))?
        * 1_000_000_000;
    let pids = option(args, "--pids-limit")?
        .parse::<u64>()
        .map_err(|error| format!("pids: {error}"))?;
    let inspection = json!([{
        "Id": instance_id,
        "Name": container_name,
        "Image": image_digest,
        "ImageDigest": image_digest,
        "State": {"Status": "created", "Running": false},
        "EffectiveCaps": [],
        "Config": {
            "User": user,
            "AttachStdin": true,
            "AttachStdout": true,
            "AttachStderr": true,
            "Tty": false,
            "OpenStdin": true,
            "Env": [
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                "HOME=/home/swem",
                "LANG=C.UTF-8",
                "container=podman"
            ],
            "Cmd": inspected_agent_args,
            "WorkingDir": workdir,
            "Entrypoint": [agent_executable],
            "Labels": {
                "io.swem.lease-id": lease_id,
                "io.swem.managed": "true"
            }
        },
        "HostConfig": {
            "NetworkMode": "none",
            "IpcMode": "private",
            "PidMode": "private",
            "CgroupMode": "private",
            "ReadonlyRootfs": true,
            "Memory": memory,
            "NanoCpus": nano_cpus,
            "PidsLimit": pids,
            "CapDrop": ["CAP_ALL"],
            "SecurityOpt": ["no-new-privileges"]
        },
        "Mounts": [{
            "Type": "bind",
            "Source": mount_source,
            "Destination": mount_target,
            "RW": true,
            "Options": ["nodev", "nosuid"]
        }]
    }]);
    let state = json!({
        "fixture_agent": fixture_agent,
        "fixture_host_workspace": fixture_host_workspace,
        "inspection": inspection
    });
    fs::write(
        state_path(&instance_id),
        serde_json::to_vec(&state).unwrap(),
    )
    .map_err(|error| format!("write fixture state: {error}"))?;
    println!("{instance_id}");
    Ok(0)
}

fn inspect(args: &[String]) -> Result<u8, String> {
    let instance_id = args
        .last()
        .ok_or_else(|| "container id is missing".to_owned())?;
    let state = read_state(instance_id)?;
    println!("{}", state["inspection"]);
    Ok(0)
}

fn start(args: &[String]) -> Result<u8, String> {
    require_flag(args, "--interactive")?;
    require_flag(args, "--attach")?;
    let instance_id = args
        .last()
        .ok_or_else(|| "container id is missing".to_owned())?;
    let state = read_state(instance_id)?;
    let executable = state["fixture_agent"]
        .as_str()
        .ok_or_else(|| "fixture agent path is missing from state".to_owned())?;
    let mut command = Command::new(executable);
    if let Some(workspace) = state["fixture_host_workspace"].as_str() {
        command.current_dir(workspace);
    }
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("launch fixture agent: {error}"))?;
    Ok(status.code().unwrap_or(1).try_into().unwrap_or(1))
}

fn remove(args: &[String]) -> Result<u8, String> {
    require_flag(args, "--force")?;
    require_flag(args, "--ignore")?;
    let instance_id = args
        .last()
        .ok_or_else(|| "container id is missing".to_owned())?;
    match fs::remove_file(state_path(instance_id)) {
        Ok(()) => Ok(0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(format!("remove fixture state: {error}")),
    }
}

fn exists(args: &[String]) -> Result<u8, String> {
    let instance_id = args
        .last()
        .ok_or_else(|| "container id is missing".to_owned())?;
    Ok(u8::from(!state_path(instance_id).is_file()))
}

fn read_state(instance_id: &str) -> Result<Value, String> {
    let bytes = fs::read(state_path(instance_id))
        .map_err(|error| format!("read fixture state: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse fixture state: {error}"))
}

fn state_path(instance_id: &str) -> PathBuf {
    env::temp_dir().join(format!("swem-podman-fixture-{instance_id}.json"))
}

fn option(args: &[String], name: &str) -> Result<String, String> {
    let index = args
        .iter()
        .position(|argument| argument == name)
        .ok_or_else(|| format!("{name} is missing"))?;
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| format!("{name} has no value"))
}

fn option_value(args: &[String], name: &str, prefix: &str) -> Result<String, String> {
    args.windows(2)
        .find(|pair| pair[0] == name && pair[1].starts_with(prefix))
        .map(|pair| pair[1][prefix.len()..].to_owned())
        .ok_or_else(|| format!("{name} {prefix}… is missing"))
}

fn require_flag(args: &[String], flag: &str) -> Result<(), String> {
    args.iter()
        .any(|argument| argument == flag)
        .then_some(())
        .ok_or_else(|| format!("required flag {flag} is missing"))
}

fn mount_component(mount: &str, name: &str) -> Result<String, String> {
    mount
        .split(',')
        .filter_map(|component| component.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
        .ok_or_else(|| format!("mount {name} is missing"))
}
