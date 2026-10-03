//! A tunnel: an address from outside, for a while.
//!
//! A Workbench on a laptop has no address the outside can reach, and some
//! of what a channel does needs one: the page a bot opens inside the
//! messenger talks to the Workbench from the person's phone. A tunnel is a
//! package of kind [`TUNNEL_KIND`]: an MCP server program the harness starts
//! and keeps, with the tools below, which stands at a public address for a
//! local URL the harness gives it and holds that address while it runs. The
//! harness names no tunnel vendor; a package carries one vendor's way of
//! getting an address, and answers in these terms.
//!
//! What the harness gives a tunnel when it starts it is in its environment:
//! [`HOME_VARIABLE`] (a directory of the tunnel's own), and for every tool
//! its catalog entry `requires`, the path of that tool's program under
//! [`tool_variable`] - so a package that drives a vendor's program never
//! fetches it itself; the Store does, under one consent.

use serde::{Deserialize, Serialize};

use crate::shape::Shape;

/// The kind a tunnel package is.
pub const TUNNEL_KIND: &str = "swem/tunnel@1";

/// The variable naming the directory a tunnel may write into.
pub const HOME_VARIABLE: &str = "SWEM_TUNNEL_HOME";

/// The variable under which a required tool's program is handed to a
/// package: `SWEM_TOOL_<ID>`, the id in upper case with `-` as `_`.
#[must_use]
pub fn tool_variable(tool_id: &str) -> String {
    let id: String = tool_id
        .chars()
        .map(|letter| {
            if letter.is_ascii_alphanumeric() {
                letter.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("SWEM_TOOL_{id}")
}

/// Stand at an address from outside for a local URL: `open`.
pub const OPEN: &str = "open";
/// Leave the address: `close`.
pub const CLOSE: &str = "close";
/// Where the tunnel stands, if anywhere: `look`.
pub const LOOK: &str = "look";

/// The shape every tunnel answers to, as a document.
pub const TUNNEL_SHAPE: &str = r#"{
  "schema": "swem:shape@0.1",
  "id": "swem/tunnel",
  "tools": [
    {"name": "open", "inputSchema": {"type": "object",
      "properties": {"url": {"type": "string"}}, "required": ["url"]}},
    {"name": "close", "inputSchema": {"type": "object", "properties": {}}},
    {"name": "look", "inputSchema": {"type": "object", "properties": {}}}
  ]
}"#;

/// The shape every tunnel answers to.
///
/// # Panics
///
/// Never: the document is this crate's own and is checked by its tests.
#[must_use]
pub fn shape() -> Shape {
    Shape::parse(TUNNEL_SHAPE.as_bytes()).expect("the tunnel shape is a shape")
}

/// What `open` and `look` answer: where the tunnel stands.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Standing {
    /// The origin from outside (`https://...`), empty when the tunnel is
    /// not open.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
    /// The local URL the address stands for, when open.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// A word on the vendor's side, for the page: what the address is, or
    /// why there is none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub said: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_is_a_shape_and_names_every_tool() {
        let names: Vec<String> = shape().tools.iter().map(|tool| tool.name.clone()).collect();
        for tool in [OPEN, CLOSE, LOOK] {
            assert!(names.iter().any(|name| name == tool), "{tool}");
        }
    }

    #[test]
    fn a_tool_is_handed_under_its_own_variable() {
        assert_eq!(tool_variable("cloudflared"), "SWEM_TOOL_CLOUDFLARED");
        assert_eq!(tool_variable("my-tool.v2"), "SWEM_TOOL_MY_TOOL_V2");
    }
}
