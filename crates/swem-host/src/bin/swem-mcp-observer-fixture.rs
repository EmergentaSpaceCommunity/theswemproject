//! Test entrypoint for the shared byte-transparent MCP observer.

use std::path::PathBuf;

use swem_host::mcp_observer::{
    StdioObserverConfig, run_ephemeral_stdio_observer, run_evidence_stdio_observer,
};

struct Arguments {
    server_name: String,
    evidence: Option<PathBuf>,
    command: PathBuf,
    command_args: Vec<String>,
}

fn arguments() -> Result<Arguments, String> {
    let values = std::env::args().skip(1).collect::<Vec<_>>();
    let separator = values
        .iter()
        .position(|value| value == "--")
        .ok_or_else(|| {
            "usage: --server-name NAME --evidence PATH -- COMMAND [ARG...]".to_owned()
        })?;
    let control = &values[..separator];
    let command = values
        .get(separator + 1)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "observer needs a child command after --".to_owned())?;
    let mut server_name = None;
    let mut evidence = None;
    let mut index = 0;
    while index < control.len() {
        let value = control
            .get(index + 1)
            .ok_or_else(|| format!("{} needs a value", control[index]))?;
        match control[index].as_str() {
            "--server-name" => server_name = Some(value.clone()),
            "--evidence" => evidence = Some(PathBuf::from(value)),
            other => return Err(format!("unknown observer option {other}")),
        }
        index += 2;
    }
    Ok(Arguments {
        server_name: server_name
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "observer needs a non-empty --server-name".to_owned())?,
        evidence,
        command,
        command_args: values[(separator + 2)..].to_vec(),
    })
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let arguments = arguments()?;
    let config = StdioObserverConfig {
        server_name: arguments.server_name,
        command: arguments.command,
        command_args: arguments.command_args,
    };
    if let Some(evidence) = arguments.evidence {
        run_evidence_stdio_observer(config, &evidence).await
    } else {
        run_ephemeral_stdio_observer(config).await
    }
}
