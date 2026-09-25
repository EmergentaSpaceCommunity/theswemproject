//! The Workbench's Cycle client integration layer (the paper's second UI
//! layer): a first-party client of an ordinary MCP server that publishes a
//! project envelope resource. It knows only envelope semantics - a server
//! name, exact record refs, the resource URIs the server itself declares -
//! and never a domain schema. It reads and never writes: the client it opens
//! can issue `resources/list` and `resources/read` only, so read-only holds by
//! construction, and it dials the declared server without any agent session
//! or environment resolution. It shows its scope honestly: an independent
//! host-side connection whose only shared truth with the agent is the
//! server's own persistent state.

use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{ContentBlock, McpServer, McpServerStdio, ResourceLink};
use base64::Engine as _;
use rmcp::model::{ReadResourceRequestParams, ResourceContents};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tokio::sync::OnceCell;

use crate::WorkbenchShellError;
use crate::workbench_apps::{HostClient, ManagedStdioExit, spawn_host_client};

/// MIME profile a server marks its project envelope resource with; the
/// discovery key, the same way MCP Apps are found by `text/html;profile=mcp-app`.
pub const PROJECT_ENVELOPE_MIME: &str = "application/json;profile=swem-project-envelope@0.1";

const CONNECTION_SCOPE: &str = "independent_host_connection";

/// One declared stdio server that exposes a project envelope, dialled.
pub(crate) struct ProjectSourceEntry {
    pub server_name: String,
    client: HostClient,
    exit: ManagedStdioExit,
    /// The listed envelope resource; every other project URI is declared by
    /// the envelope itself.
    pub root_uri: String,
    pub description: Option<String>,
    /// The server's own statement of where its records live, read once when
    /// the server was first dialled. It is a property of the server, not of
    /// a moment, so one read is the truth and the list never asks again.
    persistence: String,
}

/// What one declaration turned out to be, once dialled.
pub(crate) enum Probe {
    /// A server that lists an envelope resource: a project, kept open.
    Project(Arc<ProjectSourceEntry>),
    /// A server that lists none. It was terminated on the spot and is
    /// remembered as such, so it is neither dialled again nor listed.
    NotProject,
}

/// One stdio declaration and, once somebody needed it, what it is.
///
/// The cell is what makes dialling lazy and shared: the first request for
/// this name runs the probe, every concurrent request for the same name
/// awaits that one probe, and a request for another name never waits here at
/// all. A probe that could not even start is not stored, so the next open
/// tries again.
pub(crate) struct SourceSlot {
    declaration: McpServerStdio,
    probe: OnceCell<Probe>,
}

/// Every project source the Workbench may dial, in declaration order.
///
/// Nothing is dialled by listing. Until 2026-09-21 this type dialled every
/// declaration the moment anything asked for the list - one child process
/// each, one WebAssembly compile each - and held the shell's lock across all
/// of it, so a person with two projects waited 48.8 s to see two rows and a
/// request for either project queued behind both dials. The slots below are
/// looked up under a lock that is never held across an `.await`; the dial
/// happens on the slot's own cell after the lock is put down.
pub(crate) struct ProjectSources {
    slots: Mutex<Vec<Arc<SourceSlot>>>,
    /// Declarations with a transport this host cannot dial (fail closed).
    unsupported: Mutex<Vec<String>>,
}

/// What the shell page shows per project source.
///
/// Most of it is known-later: a row exists from the declaration alone, and
/// what the server says about itself arrives once the server has been asked.
/// The host does not guess in the meantime - `persistence` is the server's
/// word, and a caption is not worth inventing a fact for.
#[derive(Clone, Debug, Serialize)]
pub struct ProjectSourceView {
    pub server_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The server's own statement of where its records live; absent until
    /// the server has been dialled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistence: Option<String>,
    /// `Some(true)` only for a journal-backed project: an agent over the same
    /// declaration then sees the same records, so binding is meaningful.
    /// `Some(false)` for a transport this host cannot dial or a server that
    /// keeps nothing; `None` until the server has been asked.
    pub available: Option<bool>,
    pub connection_scope: &'static str,
}

impl ProjectSourceView {
    pub(crate) fn unsupported(server_name: &str) -> Self {
        Self {
            server_name: server_name.to_owned(),
            root_uri: None,
            description: None,
            persistence: Some("unsupported_transport".to_owned()),
            available: Some(false),
            connection_scope: CONNECTION_SCOPE,
        }
    }

    /// A declaration nobody has opened yet: its name, and nothing the host
    /// would have to ask the server for.
    pub(crate) fn unprobed(server_name: &str) -> Self {
        Self {
            server_name: server_name.to_owned(),
            root_uri: None,
            description: None,
            persistence: None,
            available: None,
            connection_scope: CONNECTION_SCOPE,
        }
    }
}

/// One verbatim `resources/read`: the text is never re-serialized.
#[derive(Clone, Debug, Serialize)]
pub struct EnvelopeRead {
    pub uri: String,
    pub mime: String,
    pub text: String,
}

/// One verbatim blob read: the exact bytes and the MIME the server states.
#[derive(Clone, Debug)]
pub struct BlobRead {
    pub uri: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// An artifact as the envelope declares it: the exact record ref that
/// listed it, the URI of its bytes, its media type and a name for the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeclaredArtifact {
    /// The record these bytes belong to, when the envelope answers that for the
    /// selection being read. `None` when it does not: bytes are
    /// content-addressed and several executions may produce the same ones.
    pub reference: Option<String>,
    pub uri: String,
    pub media_type: String,
    pub name: String,
}

/// Exact refs the operator bound as the next agent turn's context.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentContextBinding {
    pub server_name: String,
    pub revision_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_ref: Option<String>,
    /// The declared resource URI the agent can read for this context.
    pub uri: String,
}

/// The selection a surface is reading when it asks for an artifact's bytes.
/// Absent when the caller is not reading one, which only costs the provenance
/// label - never the bytes.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MaterializeArtifactBody {
    #[serde(default)]
    pub selection_ref: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BindAgentContextBody {
    pub server_name: String,
    pub revision_ref: String,
    #[serde(default)]
    pub selection_ref: Option<String>,
}

impl ProjectSources {
    pub fn new() -> Self {
        Self {
            slots: Mutex::new(Vec::new()),
            unsupported: Mutex::new(Vec::new()),
        }
    }

    /// Take in every declaration not yet known, dialling none. Only adds:
    /// a name already here keeps whatever it has, its live child included,
    /// which is what lets a project be declared while another is open
    /// without the open one being torn down and dialled again.
    pub fn refresh(&self, declarations: &[McpServer]) {
        let mut slots = lock(&self.slots);
        let mut unsupported = lock(&self.unsupported);
        for declaration in declarations {
            let name = declaration_name(declaration);
            let known = slots.iter().any(|slot| slot.declaration.name == name)
                || unsupported.contains(&name);
            if known {
                continue;
            }
            match declaration {
                McpServer::Stdio(stdio) => slots.push(Arc::new(SourceSlot {
                    declaration: stdio.clone(),
                    probe: OnceCell::new(),
                })),
                _ => unsupported.push(name),
            }
        }
    }

    /// The dialled source of one name: this call's dial when it is the first
    /// to ask, the shared one when another is under way, the kept one after.
    ///
    /// # Errors
    ///
    /// Not found for a name nobody declared or a server that publishes no
    /// envelope; failed when the process could not be started, which is not
    /// remembered.
    pub async fn source(
        &self,
        server_name: &str,
    ) -> Result<Arc<ProjectSourceEntry>, WorkbenchShellError> {
        let slot = lock(&self.slots)
            .iter()
            .find(|slot| slot.declaration.name == server_name)
            .cloned();
        let Some(slot) = slot else {
            let declared: Vec<String> = lock(&self.slots)
                .iter()
                .map(|slot| slot.declaration.name.clone())
                .collect();
            return Err(WorkbenchShellError::NotFound(format!(
                "no project source named {server_name} (declared: {})",
                declared.join(", ")
            )));
        };
        let probe = slot
            .probe
            .get_or_try_init(|| probe(&slot.declaration))
            .await?;
        match probe {
            Probe::Project(entry) => Ok(Arc::clone(entry)),
            Probe::NotProject => Err(WorkbenchShellError::NotFound(format!(
                "{server_name} publishes no project envelope"
            ))),
        }
    }

    /// One row per declaration, from what listing knows: the dialled ones
    /// from their servers' own words, the rest by name alone. Dials nothing.
    pub fn views(&self) -> Vec<ProjectSourceView> {
        let mut views: Vec<ProjectSourceView> = lock(&self.slots)
            .iter()
            .filter_map(|slot| match slot.probe.get() {
                Some(Probe::Project(entry)) => Some(entry.view()),
                Some(Probe::NotProject) => None,
                None => Some(ProjectSourceView::unprobed(&slot.declaration.name)),
            })
            .collect();
        // A declaration this host cannot dial is shown, never hidden.
        views.extend(
            lock(&self.unsupported)
                .iter()
                .map(|name| ProjectSourceView::unsupported(name)),
        );
        views
    }

    /// Cancel every dialled client and await the real child-process boundary.
    /// A slot nobody opened has no child and costs nothing here.
    pub async fn shutdown(&self) -> Result<(), String> {
        let slots = std::mem::take(&mut *lock(&self.slots));
        let mut first_error = None;
        for slot in slots {
            // Only the last holder can shut down: a read still in flight owns
            // the entry too. When that happens the child is killed on the
            // reader's own last drop, which is the boundary the transport
            // already enforces.
            let Some(slot) = Arc::into_inner(slot) else {
                continue;
            };
            let Some(Probe::Project(entry)) = slot.probe.into_inner() else {
                continue;
            };
            let Some(entry) = Arc::into_inner(entry) else {
                continue;
            };
            if let Err(error) = entry.shutdown().await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// A poisoned lock here means a thread panicked between two reads of a
/// list; the list is still a list.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Dial one declaration and say what it is. No profile, resolver, environment
/// or agent session takes part.
///
/// A server that lists no envelope resource is terminated here and answered
/// as `NotProject`, which the caller keeps; a process that cannot be started
/// is an error the caller does not keep.
async fn probe(stdio: &McpServerStdio) -> Result<Probe, WorkbenchShellError> {
    let Some((client, exit)) = spawn_host_client(stdio).await else {
        return Err(WorkbenchShellError::Failed(format!(
            "the project server {} could not be started",
            stdio.name
        )));
    };
    let listed = client.list_all_resources().await.unwrap_or_default();
    let Some(root) = listed
        .iter()
        .find(|resource| resource.mime_type.as_deref() == Some(PROJECT_ENVELOPE_MIME))
    else {
        let _ = client.cancel().await;
        let _ = exit.wait().await;
        return Ok(Probe::NotProject);
    };
    let mut entry = ProjectSourceEntry {
        server_name: stdio.name.clone(),
        client,
        exit,
        root_uri: root.uri.clone(),
        description: root.description.clone(),
        persistence: String::new(),
    };
    entry.persistence = match entry.read_root().await {
        Ok(read) => serde_json::from_str::<Value>(&read.text)
            .ok()
            .and_then(|envelope| {
                envelope
                    .get("persistence")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "unreadable".to_owned()),
        Err(_) => "unavailable".to_owned(),
    };
    Ok(Probe::Project(Arc::new(entry)))
}

fn declaration_name(declaration: &McpServer) -> String {
    match declaration {
        McpServer::Stdio(stdio) => stdio.name.clone(),
        McpServer::Http(http) => http.name.clone(),
        McpServer::Sse(sse) => sse.name.clone(),
        _ => "<unknown transport>".to_owned(),
    }
}

impl ProjectSourceEntry {
    /// The only read primitive: one `resources/read` of a URI the server
    /// declared, verified to come back as the requested URI with the
    /// envelope MIME.
    pub async fn read_declared(&self, uri: &str) -> Result<EnvelopeRead, WorkbenchShellError> {
        if uri != self.root_uri {
            let under_root = uri.strip_prefix(&self.root_uri).is_some_and(|rest| {
                rest.starts_with('/') && !rest.chars().any(|c| c.is_whitespace() || c.is_control())
            });
            if !under_root {
                return Err(WorkbenchShellError::NotFound(format!(
                    "{uri} is not a resource declared under {}",
                    self.root_uri
                )));
            }
        }
        let read = self
            .client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let Some(ResourceContents::TextResourceContents {
            uri: returned,
            mime_type,
            text,
            ..
        }) = read.contents.first()
        else {
            return Err(WorkbenchShellError::Failed(format!(
                "project resource {uri} returned no text content"
            )));
        };
        if returned != uri {
            return Err(WorkbenchShellError::Failed(format!(
                "resources/read returned {returned} for requested {uri}"
            )));
        }
        if mime_type.as_deref() != Some(PROJECT_ENVELOPE_MIME) {
            return Err(WorkbenchShellError::Failed(format!(
                "project resource {uri} has MIME {mime_type:?}, expected {PROJECT_ENVELOPE_MIME}"
            )));
        }
        Ok(EnvelopeRead {
            uri: returned.clone(),
            mime: PROJECT_ENVELOPE_MIME.to_owned(),
            text: text.clone(),
        })
    }

    pub async fn read_root(&self) -> Result<EnvelopeRead, WorkbenchShellError> {
        self.read_declared(&self.root_uri).await
    }

    /// One `resources/read` of a blob URI the envelope declared for an
    /// artifact: the bytes come back base64-encoded, are decoded here and
    /// verified against the digest the envelope named. The MIME is whatever
    /// the server states for those bytes.
    pub async fn read_declared_blob(
        &self,
        uri: &str,
        expected_digest: &str,
    ) -> Result<BlobRead, WorkbenchShellError> {
        let under_root = uri.strip_prefix(&self.root_uri).is_some_and(|rest| {
            rest.starts_with('/') && !rest.chars().any(|c| c.is_whitespace() || c.is_control())
        });
        if !under_root {
            return Err(WorkbenchShellError::NotFound(format!(
                "{uri} is not a resource declared under {}",
                self.root_uri
            )));
        }
        let read = self
            .client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let Some(ResourceContents::BlobResourceContents {
            uri: returned,
            mime_type,
            blob,
            ..
        }) = read.contents.first()
        else {
            return Err(WorkbenchShellError::Failed(format!(
                "project resource {uri} returned no blob content"
            )));
        };
        if returned != uri {
            return Err(WorkbenchShellError::Failed(format!(
                "resources/read returned {returned} for requested {uri}"
            )));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(blob.as_bytes())
            .map_err(|error| WorkbenchShellError::Failed(format!("blob of {uri}: {error}")))?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != expected_digest {
            return Err(WorkbenchShellError::Failed(format!(
                "blob of {uri} hashes to {digest}, the envelope declares {expected_digest}"
            )));
        }
        let Some(mime) = mime_type.clone() else {
            return Err(WorkbenchShellError::Failed(format!(
                "project resource {uri} states no media type for its bytes"
            )));
        };
        Ok(BlobRead {
            uri: returned.clone(),
            mime,
            bytes,
        })
    }

    /// The listing view, from what the server said of itself when it was
    /// dialled. No read: the list is a list.
    pub fn view(&self) -> ProjectSourceView {
        ProjectSourceView {
            server_name: self.server_name.clone(),
            root_uri: Some(self.root_uri.clone()),
            description: self.description.clone(),
            available: Some(self.persistence == "journal"),
            persistence: Some(self.persistence.clone()),
            connection_scope: CONNECTION_SCOPE,
        }
    }

    /// Cancel the client and await the real child-process boundary.
    pub async fn shutdown(self) -> Result<(), String> {
        let cancelled = self
            .client
            .cancel()
            .await
            .map_err(|error| error.to_string());
        let exited = self.exit.wait().await;
        cancelled.map(|_| ()).and(exited)
    }
}

/// The URI the envelope declares for one of its views (`selections` or
/// `revisions`) by exact ref. The host never constructs a project URI.
pub(crate) fn declared_view_uri(envelope: &Value, kind: &str, reference: &str) -> Option<String> {
    envelope
        .get("logical_ids")?
        .as_array()?
        .iter()
        .filter_map(|line| line.get(kind).and_then(Value::as_array))
        .flatten()
        .find(|view| view.get("ref").and_then(Value::as_str) == Some(reference))
        .and_then(|view| view.get("uri").and_then(Value::as_str))
        .map(str::to_owned)
}

/// The artifact with `digest` as the envelope declares it, wherever it lists
/// it: among the intent sources (keyed by record ref) or the produced
/// artifacts of an execution (keyed by output port). The host never
/// constructs the URI; it copies the one the envelope carries.
///
/// Where to read the bytes and which execution produced them are two different
/// questions. The first is answered by any declaration, because the bytes are
/// content-addressed. The second is a statement about one selection, so it is
/// answered inside `scope` - the selection being read - and nowhere else: the
/// same master can be produced by several branches, and naming a foreign
/// execution would explain one branch by another branch's work. With no scope
/// the record is named only when exactly one execution produced these bytes.
pub(crate) fn declared_artifact(
    envelope: &Value,
    digest: &str,
    scope: Option<&str>,
) -> Option<DeclaredArtifact> {
    let matches = |view: &Value| view.get("content_digest").and_then(Value::as_str) == Some(digest);
    let short = digest.get(..12).unwrap_or(digest);
    let declared = |view: &Value, reference: Option<String>, name: String| {
        Some(DeclaredArtifact {
            reference,
            uri: view.get("uri")?.as_str()?.to_owned(),
            media_type: view.get("media_type")?.as_str()?.to_owned(),
            name,
        })
    };

    // An intent source is a project-level record: its ref means the same thing
    // in every selection, so it carries no scope.
    if let Some(sources) = envelope
        .pointer("/sources/artifacts")
        .and_then(Value::as_object)
    {
        for (reference, view) in sources {
            if matches(view) {
                return declared(view, Some(reference.clone()), format!("source-{short}"));
            }
        }
    }

    let mut produced: Vec<(&str, &str, &str, &Value)> = Vec::new();
    for selection in envelope
        .get("logical_ids")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|line| line.get("selections").and_then(Value::as_array))
        .flatten()
    {
        let selection_ref = selection
            .get("ref")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let executions = selection.get("executions").and_then(Value::as_object);
        for (execution_ref, execution) in executions.into_iter().flatten() {
            let outputs = execution
                .get("produced_artifacts")
                .and_then(Value::as_object);
            for (port, view) in outputs.into_iter().flatten() {
                if matches(view) {
                    produced.push((selection_ref, execution_ref, port, view));
                }
            }
        }
    }

    let scoped = scope.and_then(|wanted| {
        produced
            .iter()
            .find(|(selection, ..)| *selection == wanted)
            .copied()
    });
    if let Some((_, execution_ref, port, view)) = scoped {
        return declared(
            view,
            Some(execution_ref.to_owned()),
            format!("{port}-{short}"),
        );
    }
    let (_, execution_ref, port, view) = *produced.first()?;
    let one_execution = produced
        .iter()
        .all(|(_, other, _, _)| *other == execution_ref);
    let reference = (scope.is_none() && one_execution).then(|| execution_ref.to_owned());
    declared(view, reference, format!("{port}-{short}"))
}

/// Validate a bind request against a freshly read envelope and return the
/// declared URI the context points at.
pub(crate) fn validate_binding(
    envelope: &Value,
    body: &BindAgentContextBody,
) -> Result<String, WorkbenchShellError> {
    if envelope.get("persistence").and_then(Value::as_str) != Some("journal") {
        return Err(WorkbenchShellError::Conflict(format!(
            "project source {} has no persistent journal; an agent over the same declaration would not see these records",
            body.server_name
        )));
    }
    let revision_uri =
        declared_view_uri(envelope, "revisions", &body.revision_ref).ok_or_else(|| {
            WorkbenchShellError::Conflict(format!(
                "revision {} is not in the project envelope",
                body.revision_ref
            ))
        })?;
    let Some(selection_ref) = &body.selection_ref else {
        return Ok(revision_uri);
    };
    let selection = envelope
        .get("logical_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|line| line.get("selections").and_then(Value::as_array))
        .flatten()
        .find(|view| view.get("ref").and_then(Value::as_str) == Some(selection_ref.as_str()))
        .ok_or_else(|| {
            WorkbenchShellError::Conflict(format!(
                "delivery selection {selection_ref} is not in the project envelope"
            ))
        })?;
    let over = selection
        .pointer("/selection/model_revision_ref")
        .and_then(Value::as_str);
    if over != Some(body.revision_ref.as_str()) {
        return Err(WorkbenchShellError::Conflict(format!(
            "delivery selection {selection_ref} is over revision {}, not {}",
            over.unwrap_or("<unknown>"),
            body.revision_ref
        )));
    }
    selection
        .get("uri")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            WorkbenchShellError::Failed(format!(
                "delivery selection {selection_ref} declares no resource uri"
            ))
        })
}

/// The bound context as standard ACP baseline content: a resource link the
/// agent's own attachment can read. No capability negotiation is needed and
/// the durable route keeps the link verbatim.
pub(crate) fn context_link(binding: &AgentContextBinding) -> ContentBlock {
    let name = match &binding.selection_ref {
        Some(selection) => format!("project selection {selection}"),
        None => format!("project revision {}", binding.revision_ref),
    };
    ContentBlock::ResourceLink(
        ResourceLink::new(name, binding.uri.clone()).mime_type(PROJECT_ENVELOPE_MIME),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn envelope() -> Value {
        json!({
            "persistence": "journal",
            "logical_ids": [{
                "logical_id": "a",
                "revisions": [
                    {"ref": "sha256:r1", "uri": "swem://project/revisions/r1"},
                    {"ref": "sha256:r2", "uri": "swem://project/revisions/r2"}
                ],
                "selections": [
                    {"ref": "sha256:s1", "uri": "swem://project/selections/s1", "selection": {"model_revision_ref": "sha256:r1"}}
                ]
            }]
        })
    }

    fn body(revision: &str, selection: Option<&str>) -> BindAgentContextBody {
        BindAgentContextBody {
            server_name: "cycle".into(),
            revision_ref: revision.into(),
            selection_ref: selection.map(str::to_owned),
        }
    }

    #[test]
    fn binding_follows_declared_uris_and_refuses_foreign_refs() {
        let envelope = envelope();
        assert_eq!(
            validate_binding(&envelope, &body("sha256:r1", None)).unwrap(),
            "swem://project/revisions/r1"
        );
        assert_eq!(
            validate_binding(&envelope, &body("sha256:r1", Some("sha256:s1"))).unwrap(),
            "swem://project/selections/s1"
        );
        assert!(matches!(
            validate_binding(&envelope, &body("sha256:zz", None)),
            Err(WorkbenchShellError::Conflict(_))
        ));
        assert!(matches!(
            validate_binding(&envelope, &body("sha256:r2", Some("sha256:s1"))),
            Err(WorkbenchShellError::Conflict(_))
        ));
        let mut ephemeral = envelope;
        ephemeral["persistence"] = json!("process_memory");
        assert!(matches!(
            validate_binding(&ephemeral, &body("sha256:r1", None)),
            Err(WorkbenchShellError::Conflict(_))
        ));
    }

    #[test]
    fn artifacts_are_found_where_the_envelope_lists_them() {
        let digest = "a".repeat(64);
        let envelope = json!({
            "sources": {"artifacts": {
                "sha256:src": {"content_digest": digest, "media_type": "text/plain", "uri": "swem://project/artifacts/src"}
            }},
            "logical_ids": [{
                "selections": [{
                    "ref": "sha256:one",
                    "executions": {
                        "sha256:exec": {"produced_artifacts": {
                            "out": {"content_digest": "b".repeat(64), "media_type": "audio/wav", "uri": "swem://project/artifacts/out"}
                        }}
                    }
                }]
            }]
        });
        let source = declared_artifact(&envelope, &digest, None).unwrap();
        assert_eq!(source.reference.as_deref(), Some("sha256:src"));
        assert_eq!(source.uri, "swem://project/artifacts/src");
        assert_eq!(source.name, format!("source-{}", "a".repeat(12)));
        let produced = declared_artifact(&envelope, &"b".repeat(64), None).unwrap();
        assert_eq!(produced.reference.as_deref(), Some("sha256:exec"));
        assert_eq!(produced.media_type, "audio/wav");
        assert_eq!(produced.name, format!("out-{}", "b".repeat(12)));
        assert!(declared_artifact(&envelope, &"c".repeat(64), None).is_none());
    }

    /// The same bytes can be produced by more than one branch, so the record
    /// that produced them is answered inside the selection being read. Naming
    /// any other branch's execution would explain one line by another's work.
    #[test]
    fn a_producing_execution_is_named_only_inside_the_selection_being_read() {
        let shared = "d".repeat(64);
        let view =
            |uri: &str| json!({"content_digest": shared, "media_type": "audio/wav", "uri": uri});
        let envelope = json!({
            "logical_ids": [{
                "selections": [
                    {
                        "ref": "sha256:superseded",
                        "executions": {"sha256:old": {"produced_artifacts": {
                            "master": view("swem://project/artifacts/shared")
                        }}}
                    },
                    {
                        "ref": "sha256:accepted",
                        "executions": {"sha256:new": {"produced_artifacts": {
                            "master": view("swem://project/artifacts/shared")
                        }}}
                    },
                    {
                        "ref": "sha256:unrelated",
                        "executions": {"sha256:other": {"produced_artifacts": {
                            "report": {"content_digest": "e".repeat(64), "media_type": "text/plain", "uri": "swem://project/artifacts/report"}
                        }}}
                    }
                ]
            }]
        });

        // Each branch is explained by its own execution.
        let accepted = declared_artifact(&envelope, &shared, Some("sha256:accepted")).unwrap();
        assert_eq!(accepted.reference.as_deref(), Some("sha256:new"));
        let superseded = declared_artifact(&envelope, &shared, Some("sha256:superseded")).unwrap();
        assert_eq!(superseded.reference.as_deref(), Some("sha256:old"));

        // A branch that did not produce these bytes still serves them - they are
        // content-addressed - but claims no producing execution.
        let foreign = declared_artifact(&envelope, &shared, Some("sha256:unrelated")).unwrap();
        assert_eq!(foreign.reference, None);
        assert_eq!(foreign.uri, "swem://project/artifacts/shared");

        // With no selection in hand the label is withheld rather than guessed.
        let unscoped = declared_artifact(&envelope, &shared, None).unwrap();
        assert_eq!(unscoped.reference, None);
        assert_eq!(unscoped.media_type, "audio/wav");

        // One producer, so the answer is unambiguous without a scope.
        let single = declared_artifact(&envelope, &"e".repeat(64), None).unwrap();
        assert_eq!(single.reference.as_deref(), Some("sha256:other"));
    }

    #[test]
    fn the_context_is_a_baseline_resource_link() {
        let block = context_link(&AgentContextBinding {
            server_name: "cycle".into(),
            revision_ref: "sha256:r1".into(),
            selection_ref: Some("sha256:s1".into()),
            uri: "swem://project/selections/s1".into(),
        });
        let value = serde_json::to_value(block).unwrap();
        assert_eq!(value["type"], "resource_link");
        assert_eq!(value["uri"], "swem://project/selections/s1");
        assert_eq!(value["mimeType"], PROJECT_ENVELOPE_MIME);
    }
}
