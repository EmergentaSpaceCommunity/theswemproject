//! What an App said a person is looking at, given to the agent with the
//! next turn.
//!
//! An MCP App may tell its host what the model should know
//! (`ui/update-model-context`): content blocks, each update replacing the
//! one before, sent with the next turn a person writes. The host interprets
//! none of it. It keeps the last update per connection, refuses context
//! from a server the session does not attach - the agent could not follow a
//! link into it - and hands the blocks to the agent as baseline content, so
//! no capability has to be negotiated and the route keeps them verbatim.

use agent_client_protocol::schema::v1::{ContentBlock, ResourceLink, TextContent};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Ordering, WorkbenchShellError, WorkbenchShellState};

/// How many blocks one update may carry.
const MAX_BLOCKS: usize = 16;
/// How much text one block may carry.
const MAX_TEXT_BYTES: usize = 16 * 1024;

/// One block of context, in the shape the Apps specification sends it: the
/// two kinds this host declares it takes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelContextBlock {
    Text {
        text: String,
    },
    ResourceLink {
        uri: String,
        name: String,
        #[serde(
            default,
            rename = "mimeType",
            alias = "mime_type",
            skip_serializing_if = "Option::is_none"
        )]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

/// What the agent is given with every next turn until it is let go of.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelContext {
    /// The server whose App said it.
    pub server_name: String,
    pub content: Vec<ModelContextBlock>,
}

/// The body of a bind: the same two fields.
#[derive(Clone, Debug, Deserialize)]
pub struct BindModelContextBody {
    pub server_name: String,
    pub content: Vec<ModelContextBlock>,
}

/// The context as ACP baseline content, block for block.
pub(crate) fn context_content(context: &ModelContext) -> Vec<ContentBlock> {
    context
        .content
        .iter()
        .map(|block| match block {
            ModelContextBlock::Text { text } => ContentBlock::Text(TextContent::new(text.clone())),
            ModelContextBlock::ResourceLink {
                uri,
                name,
                mime_type,
                description,
            } => {
                let mut link = ResourceLink::new(name.clone(), uri.clone());
                if let Some(mime_type) = mime_type {
                    link = link.mime_type(mime_type.clone());
                }
                if let Some(description) = description {
                    link = link.description(description.clone());
                }
                ContentBlock::ResourceLink(link)
            }
        })
        .collect()
}

pub(super) fn validate(body: &BindModelContextBody) -> Result<(), WorkbenchShellError> {
    if body.content.is_empty() {
        return Err(WorkbenchShellError::Invalid(
            "a context says something: at least one block".into(),
        ));
    }
    if body.content.len() > MAX_BLOCKS {
        return Err(WorkbenchShellError::Invalid(format!(
            "a context carries at most {MAX_BLOCKS} blocks"
        )));
    }
    for block in &body.content {
        match block {
            ModelContextBlock::Text { text } => {
                if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
                    return Err(WorkbenchShellError::Invalid(format!(
                        "a text block is between one character and {MAX_TEXT_BYTES} bytes"
                    )));
                }
            }
            ModelContextBlock::ResourceLink { uri, name, .. } => {
                if uri.trim().is_empty() || name.trim().is_empty() {
                    return Err(WorkbenchShellError::Invalid(
                        "a resource link names what it links and where".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

impl WorkbenchShellState {
    /// Keep what an App of `server_name` said as the context of this
    /// connection's next turns. Requires an open connection whose profile
    /// attaches the same server, so the agent can follow what it is given.
    /// Nothing is sent to the agent here; the next prompt carries it.
    ///
    /// # Errors
    ///
    /// Not found for an unknown connection; conflict for a server the
    /// connection does not attach; invalid for an empty or oversized context.
    pub async fn bind_model_context(
        &self,
        connection_id: &str,
        body: BindModelContextBody,
    ) -> Result<ModelContext, WorkbenchShellError> {
        validate(&body)?;
        let connection = self.connection(connection_id).await?;
        if !connection
            .attachments
            .iter()
            .any(|attachment| attachment.binding.server_name == body.server_name)
        {
            return Err(WorkbenchShellError::Conflict(format!(
                "connection {connection_id} does not attach {}; the agent could not follow what \
                 it is given",
                body.server_name
            )));
        }
        let context = ModelContext {
            server_name: body.server_name,
            content: body.content,
        };
        *connection.model_context.lock().await = Some(context.clone());
        let event = connection.context_events.fetch_add(1, Ordering::Relaxed);
        self.append_context_event(
            &connection,
            event,
            "host/agent_context_bound",
            serde_json::to_value(&context)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?,
        )
        .await?;
        Ok(context)
    }

    /// Let go of the context; later prompts carry none.
    ///
    /// # Errors
    ///
    /// Not found for an unknown connection.
    pub async fn clear_model_context(
        &self,
        connection_id: &str,
    ) -> Result<serde_json::Value, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let previous = connection.model_context.lock().await.take();
        if let Some(previous) = previous {
            let event = connection.context_events.fetch_add(1, Ordering::Relaxed);
            self.append_context_event(
                &connection,
                event,
                "host/agent_context_cleared",
                json!({ "server_name": previous.server_name }),
            )
            .await?;
        }
        Ok(json!({ "cleared": true }))
    }

    /// The context of one open connection, if any.
    ///
    /// # Errors
    ///
    /// Not found for an unknown connection.
    pub async fn model_context(
        &self,
        connection_id: &str,
    ) -> Result<Option<ModelContext>, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let context = connection.model_context.lock().await.clone();
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_is_baseline_content_block_for_block() {
        let context = ModelContext {
            server_name: "notes".into(),
            content: vec![
                ModelContextBlock::Text {
                    text: "the board, as a person sees it".into(),
                },
                ModelContextBlock::ResourceLink {
                    uri: "notes://board".into(),
                    name: "the board".into(),
                    mime_type: Some("application/json".into()),
                    description: None,
                },
            ],
        };
        let blocks = serde_json::to_value(context_content(&context)).unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "the board, as a person sees it");
        assert_eq!(blocks[1]["type"], "resource_link");
        assert_eq!(blocks[1]["uri"], "notes://board");
        assert_eq!(blocks[1]["mimeType"], "application/json");
    }

    #[test]
    fn a_block_arrives_in_the_shape_an_app_sends_it() {
        let body: BindModelContextBody = serde_json::from_value(json!({
            "server_name": "notes",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "resource_link", "uri": "notes://board", "name": "the board", "mimeType": "application/json"}
            ]
        }))
        .unwrap();
        assert!(validate(&body).is_ok());
        assert_eq!(body.content.len(), 2);
        let empty = BindModelContextBody {
            server_name: "notes".into(),
            content: Vec::new(),
        };
        assert!(validate(&empty).is_err());
        let blank = BindModelContextBody {
            server_name: "notes".into(),
            content: vec![ModelContextBlock::Text { text: "  ".into() }],
        };
        assert!(validate(&blank).is_err());
    }
}
