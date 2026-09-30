//! The shape of a server: the tools a kind of package has to answer with.
//!
//! No standard lets a server declare "I implement tool set X". A host that
//! takes a kind of MCP server writes down the shape it calls - the tool
//! names and what it sends them - starts a candidate once, lists its tools
//! and compares. The comparison is here, so every host compares the same
//! way; starting the server and listing is the host's, since only it has a
//! client.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The document a shape is.
pub const SHAPE_SCHEMA: &str = "swem:shape@0.1";

/// What a host calls of a server of one kind: the tools, and for each the
/// input the host sends.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Shape {
    #[serde(default = "shape_schema")]
    pub schema: String,
    /// The kind of server this is the shape of, in the host's words.
    pub id: String,
    pub tools: Vec<ToolShape>,
}

fn shape_schema() -> String {
    SHAPE_SCHEMA.to_owned()
}

/// One tool the host calls, with the JSON Schema of what it sends: the
/// properties the host fills and which of them it always fills.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolShape {
    pub name: String,
    #[serde(rename = "inputSchema", default = "empty_object")]
    pub input_schema: Value,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// One tool a server listed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolListed {
    pub name: String,
    pub input_schema: Value,
}

impl Shape {
    /// Read a shape document.
    ///
    /// # Errors
    ///
    /// Not a shape, or a shape of another schema.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let shape: Self =
            serde_json::from_slice(bytes).map_err(|error| format!("not a shape: {error}"))?;
        if shape.schema != SHAPE_SCHEMA {
            return Err(format!(
                "a shape of {}, and this reads {SHAPE_SCHEMA}",
                shape.schema
            ));
        }
        if shape.id.trim().is_empty() {
            return Err("a shape names what it is the shape of".into());
        }
        if shape.tools.is_empty() {
            return Err("a shape names at least one tool".into());
        }
        Ok(shape)
    }

    /// Whether a server that listed these tools answers every call the
    /// host makes: each tool of the shape is there, takes every property
    /// the host sends with the type the host sends, and requires nothing
    /// the host would not send.
    ///
    /// # Errors
    ///
    /// Every way the server falls short, in words.
    pub fn check(&self, listed: &[ToolListed]) -> Result<(), String> {
        let mut short = Vec::new();
        for tool in &self.tools {
            let Some(found) = listed.iter().find(|one| one.name == tool.name) else {
                short.push(format!("does not answer `{}`", tool.name));
                continue;
            };
            for reason in tool_short(&tool.name, &tool.input_schema, &found.input_schema) {
                short.push(reason);
            }
        }
        if short.is_empty() {
            Ok(())
        } else {
            Err(short.join("; "))
        }
    }
}

fn tool_short(name: &str, sent: &Value, takes: &Value) -> Vec<String> {
    let mut short = Vec::new();
    let sent_properties = properties_of(sent);
    let taken_properties = properties_of(takes);
    for (property, schema) in &sent_properties {
        let Some(found) = taken_properties.get(property) else {
            short.push(format!("`{name}` does not take `{property}`"));
            continue;
        };
        let (sent_type, taken_type) = (types_of(schema), types_of(found));
        if !sent_type.is_empty() && !taken_type.is_empty() && !sent_type.is_subset(&taken_type) {
            short.push(format!(
                "`{name}` takes `{property}` as {}, and the host sends {}",
                taken_type.iter().cloned().collect::<Vec<_>>().join(" or "),
                sent_type.iter().cloned().collect::<Vec<_>>().join(" or ")
            ));
        }
    }
    let always_sent = required_of(sent);
    for property in required_of(takes) {
        // What the host always sends satisfies it; what the host may send
        // does not: a call without it would be refused.
        if !always_sent.contains(&property) {
            short.push(format!(
                "`{name}` requires `{property}`, which the host does not always send"
            ));
        }
    }
    short
}

fn properties_of(schema: &Value) -> serde_json::Map<String, Value> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn required_of(schema: &Value) -> BTreeSet<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// The JSON types a schema admits: `type` as one word or several, and
/// `null` when it is nullable. Empty means the schema does not say.
fn types_of(schema: &Value) -> BTreeSet<String> {
    match schema.get("type") {
        Some(Value::String(one)) => BTreeSet::from([one.clone()]),
        Some(Value::Array(several)) => several
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => {
            // `anyOf` of typed alternatives, as generated schemas write an
            // optional value.
            schema
                .get("anyOf")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(types_of)
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shape() -> Shape {
        Shape::parse(
            json!({
                "schema": SHAPE_SCHEMA,
                "id": "example/helper",
                "tools": [{
                    "name": "echo",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"nonce": {"type": "string"}},
                        "required": ["nonce"]
                    }
                }]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    fn listed(input_schema: Value) -> Vec<ToolListed> {
        vec![ToolListed {
            name: "echo".into(),
            input_schema,
        }]
    }

    #[test]
    fn a_server_with_the_tools_of_the_shape_fits() {
        let fits = listed(json!({
            "type": "object",
            "properties": {"nonce": {"type": "string"}, "loud": {"type": "boolean"}},
            "required": ["nonce"],
            "$schema": "http://json-schema.org/draft-07/schema#",
            "title": "EchoRequest"
        }));
        shape().check(&fits).unwrap();
    }

    #[test]
    fn a_server_without_the_tool_is_said_so() {
        let short = shape().check(&[]).unwrap_err();
        assert_eq!(short, "does not answer `echo`");
    }

    #[test]
    fn a_tool_that_takes_a_property_as_another_type_is_said_so() {
        let short = shape()
            .check(&listed(json!({
                "type": "object",
                "properties": {"nonce": {"type": "integer"}},
                "required": ["nonce"]
            })))
            .unwrap_err();
        assert_eq!(
            short,
            "`echo` takes `nonce` as integer, and the host sends string"
        );
    }

    #[test]
    fn a_tool_that_requires_what_the_host_does_not_always_send_is_said_so() {
        let short = shape()
            .check(&listed(json!({
                "type": "object",
                "properties": {"nonce": {"type": "string"}, "key": {"type": "string"}},
                "required": ["nonce", "key"]
            })))
            .unwrap_err();
        assert_eq!(
            short,
            "`echo` requires `key`, which the host does not always send"
        );
    }

    #[test]
    fn a_tool_that_does_not_take_what_the_host_sends_is_said_so() {
        let short = shape()
            .check(&listed(json!({"type": "object", "properties": {}})))
            .unwrap_err();
        assert_eq!(short, "`echo` does not take `nonce`");
    }

    #[test]
    fn a_nullable_property_admits_the_type_the_host_sends() {
        let fits = listed(json!({
            "type": "object",
            "properties": {"nonce": {"anyOf": [{"type": "string"}, {"type": "null"}]}},
            "required": ["nonce"]
        }));
        shape().check(&fits).unwrap();
    }

    #[test]
    fn a_shape_of_another_schema_or_without_tools_is_refused() {
        assert!(
            Shape::parse(br#"{"schema":"swem:shape@9","id":"x","tools":[{"name":"a"}]}"#)
                .unwrap_err()
                .contains("a shape of swem:shape@9")
        );
        assert!(
            Shape::parse(br#"{"id":"x","tools":[]}"#)
                .unwrap_err()
                .contains("at least one tool")
        );
    }
}
