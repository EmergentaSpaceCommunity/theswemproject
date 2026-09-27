//! What an App said a person is looking at, given to the agent with what
//! the person says.
//!
//! An MCP App may tell its host what the model should know
//! (`ui/update-model-context`): content blocks, each update replacing the
//! one before. The page keeps the last one and sends it with the person's
//! next message; it is part of that message in the chat's record. The host
//! interprets none of it. An agent is given it when it attaches the App's
//! server - otherwise it could not follow a link into it - as baseline
//! content, so no capability has to be negotiated.

use agent_client_protocol::schema::v1::{ContentBlock, ResourceLink, TextContent};
use serde::{Deserialize, Serialize};

use super::WorkbenchShellError;

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

#[cfg(test)]
mod tests {
    use serde_json::json;

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
