//! MCP fixture for empirical agent/model tool-argument characterisation.
//!
//! This is deliberately a test peer, not a product compatibility registry.
//! It publishes four controlled JSON Schema 2020-12 shapes and appends one
//! external receipt for every valid invocation. A byte-transparent observer
//! can be placed in front of it to retain invalid attempts as well.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorData, Implementation, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt as _};
use serde_json::{Value, json};

const FLAT: &str = "shape_flat_closed";
const NESTED: &str = "shape_nested_closed";
const UNION: &str = "shape_discriminated_union";
const VALUE: &str = "shape_unconstrained_value";

#[derive(Debug)]
struct ShapeServer {
    receipts: PathBuf,
    receipt_lock: Mutex<()>,
}

impl ShapeServer {
    fn new(receipts: PathBuf) -> Self {
        Self {
            receipts,
            receipt_lock: Mutex::new(()),
        }
    }

    fn tools() -> Vec<Tool> {
        vec![flat_tool(), nested_tool(), union_tool(), value_tool()]
    }

    fn validate(name: &str, arguments: &Value) -> Result<(), &'static str> {
        let object = arguments.as_object().ok_or("arguments must be an object")?;
        if object
            .get("nonce")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err("nonce must be a non-empty string");
        }
        match name {
            FLAT => {
                if object.len() != 3
                    || object.get("enabled").and_then(Value::as_bool).is_none()
                    || !object
                        .get("count")
                        .and_then(Value::as_i64)
                        .is_some_and(|count| (1..=9).contains(&count))
                {
                    return Err("flat arguments do not match the advertised closed shape");
                }
            }
            NESTED => validate_nested(object)?,
            UNION => validate_union(object)?,
            VALUE => {
                if object.len() != 2 || !object.contains_key("value") {
                    return Err("value arguments do not match the advertised closed envelope");
                }
            }
            _ => return Err("unknown tool"),
        }
        Ok(())
    }

    fn append_receipt(&self, name: &str, arguments: &Value) -> Result<(), ErrorData> {
        let _guard = self
            .receipt_lock
            .lock()
            .map_err(|_| ErrorData::internal_error("receipt lock poisoned", None))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.receipts)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let receipt = json!({"tool": name, "arguments": arguments});
        serde_json::to_writer(&mut file, &receipt)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        file.write_all(b"\n")
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        file.flush()
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))
    }
}

impl ServerHandler for ShapeServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "swem-mcp-tool-shape-fixture",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Controlled MCP tool shapes for empirical characterisation")
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(Self::tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.into_owned();
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        Self::validate(&name, &arguments)
            .map_err(|message| ErrorData::invalid_params(message, Some(arguments.clone())))?;
        self.append_receipt(&name, &arguments)?;
        Ok(CallToolResponse::Complete(CallToolResult::structured(
            json!({
                "accepted": true,
                "tool": name,
                "nonce": arguments["nonce"]
            }),
        )))
    }
}

fn tool(name: &'static str, description: &'static str, schema: Value) -> Tool {
    let Value::Object(schema) = schema else {
        panic!("fixture schema is an object")
    };
    Tool::new(name, description, Arc::new(JsonObject::from_iter(schema)))
}

fn flat_tool() -> Tool {
    tool(
        FLAT,
        "Flat, closed object baseline",
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "nonce": {"type": "string", "minLength": 1},
                "enabled": {"type": "boolean"},
                "count": {"type": "integer", "minimum": 1, "maximum": 9}
            },
            "required": ["nonce", "enabled", "count"],
            "additionalProperties": false
        }),
    )
}

fn nested_tool() -> Tool {
    tool(
        NESTED,
        "Concrete nested object with constrained arrays and enums",
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "nonce": {"type": "string", "minLength": 1},
                "timeline": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "minLength": 1},
                        "clips": {
                            "type": "array",
                            "minItems": 2,
                            "maxItems": 2,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "asset": {"type": "string", "pattern": "^[a-z][a-z0-9-]+$"},
                                    "start_ms": {"type": "integer", "minimum": 0},
                                    "duration_ms": {"type": "integer", "minimum": 1},
                                    "role": {"type": "string", "enum": ["opening", "closing"]}
                                },
                                "required": ["asset", "start_ms", "duration_ms", "role"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["title", "clips"],
                    "additionalProperties": false
                }
            },
            "required": ["nonce", "timeline"],
            "additionalProperties": false
        }),
    )
}

fn union_tool() -> Tool {
    tool(
        UNION,
        "Nested discriminated union; choose exactly one operation",
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "nonce": {"type": "string", "minLength": 1},
                "operation": {
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "kind": {"const": "trim"},
                                "start_ms": {"type": "integer", "minimum": 0},
                                "end_ms": {"type": "integer", "minimum": 1}
                            },
                            "required": ["kind", "start_ms", "end_ms"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "properties": {
                                "kind": {"const": "normalize"},
                                "target_lufs": {"type": "number", "minimum": -24, "maximum": -6}
                            },
                            "required": ["kind", "target_lufs"],
                            "additionalProperties": false
                        }
                    ]
                }
            },
            "required": ["nonce", "operation"],
            "additionalProperties": false
        }),
    )
}

fn value_tool() -> Tool {
    tool(
        VALUE,
        "Nested value intentionally unconstrained by its domain",
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "nonce": {"type": "string", "minLength": 1},
                "value": {}
            },
            "required": ["nonce", "value"],
            "additionalProperties": false
        }),
    )
}

fn validate_nested(object: &serde_json::Map<String, Value>) -> Result<(), &'static str> {
    if object.len() != 2 {
        return Err("nested envelope has unknown or missing properties");
    }
    let timeline = object
        .get("timeline")
        .and_then(Value::as_object)
        .ok_or("timeline must be an object")?;
    if timeline.len() != 2
        || timeline
            .get("title")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err("timeline does not match the advertised closed shape");
    }
    let clips = timeline
        .get("clips")
        .and_then(Value::as_array)
        .filter(|clips| clips.len() == 2)
        .ok_or("timeline must contain exactly two clips")?;
    for clip in clips {
        let clip = clip.as_object().ok_or("clip must be an object")?;
        if clip.len() != 4
            || !clip
                .get("asset")
                .and_then(Value::as_str)
                .is_some_and(valid_asset)
            || clip.get("start_ms").and_then(Value::as_u64).is_none()
            || clip
                .get("duration_ms")
                .and_then(Value::as_u64)
                .is_none_or(|duration| duration == 0)
            || !matches!(
                clip.get("role").and_then(Value::as_str),
                Some("opening" | "closing")
            )
        {
            return Err("clip does not match the advertised closed shape");
        }
    }
    Ok(())
}

fn valid_asset(asset: &str) -> bool {
    let mut characters = asset.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn validate_union(object: &serde_json::Map<String, Value>) -> Result<(), &'static str> {
    if object.len() != 2 {
        return Err("union envelope has unknown or missing properties");
    }
    let operation = object
        .get("operation")
        .and_then(Value::as_object)
        .ok_or("operation must be an object")?;
    match operation.get("kind").and_then(Value::as_str) {
        Some("trim") => {
            let start = operation.get("start_ms").and_then(Value::as_u64);
            let end = operation.get("end_ms").and_then(Value::as_u64);
            if operation.len() != 3
                || !matches!((start, end), (Some(start), Some(end)) if end > start)
            {
                return Err("trim operation is invalid");
            }
        }
        Some("normalize") => {
            let target = operation.get("target_lufs").and_then(Value::as_f64);
            if operation.len() != 2
                || !target.is_some_and(|target| (-24.0..=-6.0).contains(&target))
            {
                return Err("normalize operation is invalid");
            }
        }
        _ => return Err("operation discriminator is invalid"),
    }
    Ok(())
}

fn arguments() -> Result<PathBuf, String> {
    let values = std::env::args().skip(1).collect::<Vec<_>>();
    let receipts = values
        .windows(2)
        .find(|pair| pair[0] == "--receipts")
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or_else(|| {
            "usage: swem-mcp-tool-shape-fixture --receipts <absolute-jsonl>".to_owned()
        })?;
    if !receipts.is_absolute() {
        return Err("receipt path must be absolute".to_owned());
    }
    if let Some(parent) = receipts.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    Ok(receipts)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ShapeServer::new(arguments()?)
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
