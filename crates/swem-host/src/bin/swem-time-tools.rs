//! An agent's own schedules, served to its session over standard streams.
//!
//! The Workbench hands this to an engine with a session in a chat. It is
//! given the ledger, the agent it serves and the chat the agent is in, and
//! none of the three can be changed by a call.

use std::path::PathBuf;

const USAGE: &str = "usage: swem-time-tools --ledger <path> --agent <id> --chat <id>";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let given = |name: &str| {
        arguments
            .windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].clone())
            .ok_or(USAGE)
    };
    swem_host::serve_time_tools(
        PathBuf::from(given("--ledger")?),
        given("--agent")?,
        given("--chat")?,
    )
    .await?;
    Ok(())
}
