//! A person acts on a project without an agent, through the tool's own
//! schema.
//!
//! The Project space is a projection of `swem://project`; when it offers a
//! step (a rung's action, adding a slot) the step is one MCP tool call on
//! the project's own server, over the same independent client and behind
//! the same gate the App relay uses: only a tool the server declares as
//! App-visible, only `tools/call`. The host never names a tool itself - the
//! Cycle publishes which tool a rung offers, and the form is built from the
//! tool's input schema.
use super::*;

impl WorkbenchShellState {
    /// Call one App-visible tool on a project's server with the person's
    /// arguments; the tool's structured result comes back as is.
    ///
    /// # Errors
    ///
    /// Refuses an unavailable project (`Conflict`), a tool the server does
    /// not declare or does not expose to Apps (`NotFound`), and returns the
    /// tool's own refusal as a `Conflict` sentence.
    pub async fn project_tool_call(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, WorkbenchShellError> {
        if !arguments.is_object() {
            return Err(WorkbenchShellError::Invalid(
                "tool arguments must be a JSON object".into(),
            ));
        }
        self.project_apps_list(server).await?;
        let entry = self.project_app_entry(server).await?;
        workbench_apps::allow_relay(&entry, "tools/call", Some(tool))
            .map_err(|refusal| WorkbenchShellError::NotFound(refusal.message()))?;
        // A rung's action is the long call of the three paths. The attachment
        // is shared and no lock is held over it, so an App of the same
        // server - the domain workbench the person is looking at - reads
        // while the action runs.
        let client = entry.relay_client().map_err(WorkbenchShellError::Failed)?;
        let params = json!({"name": tool, "arguments": arguments});
        let result = workbench_apps::execute_relay(&client, "tools/call", &params)
            .await
            .map_err(WorkbenchShellError::Failed)?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            // The tool's refusal is a sentence for the person, not a payload.
            let text = result
                .get("content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|block| block.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "the tool refused the call".to_owned());
            return Err(WorkbenchShellError::Conflict(text));
        }
        Ok(result)
    }
}
