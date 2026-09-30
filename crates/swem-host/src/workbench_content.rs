//! Domain-neutral byte transport for the Workbench.
//!
//! This is deliberately a Harness boundary, not a Cycle artifact store. It
//! owns immutable bytes and transport metadata, then projects those bytes into
//! standard ACP content blocks. Semantic artifact types, acceptance and
//! staleness remain outside this module.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agent_client_protocol::schema::v1::{
    AudioContent, BlobResourceContents, ContentBlock, EmbeddedResource, EmbeddedResourceResource,
    ImageContent, ResourceLink, TextResourceContents,
};
use base64::Engine as _;
use http_body_util::BodyExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::WorkbenchShellError;

const CONTENT_SCHEMA: &str = "swem.workbench-content.v1";
const MAX_CONTENT_BYTES: u64 = 8 * 1024 * 1024 * 1024;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbenchContentSource {
    UserUpload,
    AgentOutput,
    /// Bytes a declared server published as a resource, brought in by the
    /// host itself. Nothing writes it since 2026-09-27; descriptors recorded
    /// before keep reading.
    ProjectArtifact,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkbenchContentLifecycle {
    Available,
}

/// Durable, domain-neutral metadata for one immutable byte representation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkbenchContentDescriptor {
    pub schema: String,
    pub descriptor_id: String,
    pub content_digest: String,
    pub byte_length: u64,
    pub media_type: String,
    pub name: String,
    pub source: WorkbenchContentSource,
    pub producer: String,
    pub lifecycle: WorkbenchContentLifecycle,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_reference: Option<String>,
    /// RFC 6920 named-information URI for the immutable bytes.
    pub uri: String,
}

#[derive(Clone, Debug)]
pub struct WorkbenchContentStore {
    root: PathBuf,
}

struct IngestMetadata {
    name: String,
    media_type: String,
    source: WorkbenchContentSource,
    producer: String,
    semantic_reference: Option<String>,
}

impl WorkbenchContentStore {
    pub fn open(root: &Path) -> Result<Self, WorkbenchShellError> {
        for directory in [
            root.join("blobs"),
            root.join("descriptors"),
            root.join("tmp"),
        ] {
            std::fs::create_dir_all(&directory)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        }
        let root = std::fs::canonicalize(root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        Ok(Self { root })
    }

    /// Stream an HTTP request body to an immutable content-addressed blob.
    pub async fn ingest_http(
        &self,
        name: String,
        media_type: String,
        mut body: crate::workbench_shell::AskedBody,
    ) -> Result<WorkbenchContentDescriptor, WorkbenchShellError> {
        validate_metadata(&name, &media_type)?;
        let temporary = self.root.join("tmp").join(format!(
            "upload-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let mut hasher = Sha256::new();
        let mut byte_length = 0_u64;
        let stream_result = async {
            while let Some(frame) = body.frame().await {
                let frame =
                    frame.map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?;
                let Ok(data) = frame.into_data() else {
                    continue;
                };
                byte_length = byte_length
                    .checked_add(
                        u64::try_from(data.len())
                            .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?,
                    )
                    .ok_or_else(|| {
                        WorkbenchShellError::Invalid("content length overflow".into())
                    })?;
                if byte_length > MAX_CONTENT_BYTES {
                    return Err(WorkbenchShellError::Invalid(format!(
                        "content exceeds the {MAX_CONTENT_BYTES} byte Workbench limit"
                    )));
                }
                hasher.update(&data);
                file.write_all(&data)
                    .await
                    .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            }
            file.flush()
                .await
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            file.sync_all()
                .await
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            Ok::<_, WorkbenchShellError>(())
        }
        .await;
        drop(file);
        if let Err(error) = stream_result {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
        self.finish_ingest(
            temporary,
            hasher.finalize().into(),
            byte_length,
            IngestMetadata {
                name,
                media_type,
                source: WorkbenchContentSource::UserUpload,
                producer: "workbench-surface".into(),
                semantic_reference: None,
            },
        )
        .await
    }

    pub async fn ingest_bytes(
        &self,
        bytes: &[u8],
        name: String,
        media_type: String,
        source: WorkbenchContentSource,
        producer: String,
    ) -> Result<WorkbenchContentDescriptor, WorkbenchShellError> {
        self.ingest_bytes_with_reference(bytes, name, media_type, source, producer, None)
            .await
    }

    /// Ingest bytes that belong to an exact semantic record: the descriptor
    /// carries the reference so the bytes can be traced back to it.
    pub async fn ingest_bytes_with_reference(
        &self,
        bytes: &[u8],
        name: String,
        media_type: String,
        source: WorkbenchContentSource,
        producer: String,
        semantic_reference: Option<String>,
    ) -> Result<WorkbenchContentDescriptor, WorkbenchShellError> {
        validate_metadata(&name, &media_type)?;
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let blob = self.blob_path(&hex_digest(&digest));
        if let Ok(existing) = tokio::fs::metadata(&blob).await {
            if existing.len()
                != u64::try_from(bytes.len())
                    .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
            {
                return Err(WorkbenchShellError::Failed(
                    "content-address collision has a different byte length".into(),
                ));
            }
        } else {
            let temporary = self.root.join("tmp").join(format!(
                "bytes-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            tokio::fs::write(&temporary, bytes)
                .await
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            return self
                .finish_ingest(
                    temporary,
                    digest,
                    u64::try_from(bytes.len())
                        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?,
                    IngestMetadata {
                        name,
                        media_type,
                        source,
                        producer,
                        semantic_reference,
                    },
                )
                .await;
        }
        self.persist_descriptor(
            digest,
            u64::try_from(bytes.len())
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?,
            IngestMetadata {
                name,
                media_type,
                source,
                producer,
                semantic_reference,
            },
        )
        .await
    }

    /// Import a standard ACP file resource after the active environment lease
    /// has resolved its agent-visible URI to a host workspace candidate. This
    /// is intentionally narrower than a generic URI fetcher: only an existing
    /// regular target whose canonical path is inside the connection's exact
    /// host workspace can enter the content store.
    pub async fn ingest_workspace_link(
        &self,
        resource: &ResourceLink,
        workspace: &Path,
        requested: &Path,
        producer: String,
    ) -> Result<WorkbenchContentDescriptor, WorkbenchShellError> {
        let resolved = resolve_workspace_link(resource, workspace, requested).await?;
        let copied = copy_workspace_file(&self.root, &resolved.path, resolved.byte_length).await?;
        self.finish_ingest(
            copied.temporary,
            copied.digest,
            resolved.byte_length,
            IngestMetadata {
                name: resource.name.clone(),
                media_type: resolved.media_type,
                source: WorkbenchContentSource::AgentOutput,
                producer,
                semantic_reference: None,
            },
        )
        .await
    }

    async fn finish_ingest(
        &self,
        temporary: PathBuf,
        digest: [u8; 32],
        byte_length: u64,
        metadata: IngestMetadata,
    ) -> Result<WorkbenchContentDescriptor, WorkbenchShellError> {
        let hex = hex_digest(&digest);
        let blob = self.blob_path(&hex);
        if let Ok(actual) = tokio::fs::metadata(&blob).await {
            tokio::fs::remove_file(&temporary)
                .await
                .map_err(|remove| WorkbenchShellError::Failed(remove.to_string()))?;
            if actual.len() != byte_length {
                return Err(WorkbenchShellError::Failed(
                    "content-address collision has a different byte length".into(),
                ));
            }
            return self.persist_descriptor(digest, byte_length, metadata).await;
        }
        match tokio::fs::rename(&temporary, &blob).await {
            Ok(()) => {}
            Err(error) => {
                if let Ok(actual) = tokio::fs::metadata(&blob).await {
                    tokio::fs::remove_file(&temporary)
                        .await
                        .map_err(|remove| WorkbenchShellError::Failed(remove.to_string()))?;
                    if actual.len() != byte_length {
                        return Err(WorkbenchShellError::Failed(
                            "content-address collision has a different byte length".into(),
                        ));
                    }
                } else {
                    let _ = tokio::fs::remove_file(&temporary).await;
                    return Err(WorkbenchShellError::Failed(error.to_string()));
                }
            }
        }
        self.persist_descriptor(digest, byte_length, metadata).await
    }

    async fn persist_descriptor(
        &self,
        digest: [u8; 32],
        byte_length: u64,
        metadata: IngestMetadata,
    ) -> Result<WorkbenchContentDescriptor, WorkbenchShellError> {
        let content_hex = hex_digest(&digest);
        let uri_digest = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
        let mut descriptor = WorkbenchContentDescriptor {
            schema: CONTENT_SCHEMA.into(),
            descriptor_id: String::new(),
            content_digest: format!("sha256:{content_hex}"),
            byte_length,
            media_type: metadata.media_type,
            name: metadata.name,
            source: metadata.source,
            producer: metadata.producer,
            lifecycle: WorkbenchContentLifecycle::Available,
            semantic_reference: metadata.semantic_reference,
            uri: format!("ni:///sha-256;{uri_digest}"),
        };
        let canonical = serde_json::to_vec(&descriptor)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        descriptor.descriptor_id = hex_digest(&Sha256::digest(canonical).into());
        let bytes = serde_json::to_vec_pretty(&descriptor)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let path = self.descriptor_path(&descriptor.descriptor_id);
        if let Ok(existing) = tokio::fs::read(&path).await {
            if existing != bytes {
                return Err(WorkbenchShellError::Failed(
                    "content descriptor identity collision".into(),
                ));
            }
            return Ok(descriptor);
        }
        let temporary = self.root.join("tmp").join(format!(
            "descriptor-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        file.write_all(&bytes)
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        file.sync_all()
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        drop(file);
        if let Err(rename_error) = tokio::fs::rename(&temporary, &path).await {
            if tokio::fs::metadata(&path).await.is_ok() {
                let _ = tokio::fs::remove_file(&temporary).await;
                let existing = tokio::fs::read(&path)
                    .await
                    .map_err(|read| WorkbenchShellError::Failed(read.to_string()))?;
                if existing != bytes {
                    return Err(WorkbenchShellError::Failed(
                        "content descriptor identity collision".into(),
                    ));
                }
            } else {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(WorkbenchShellError::Failed(rename_error.to_string()));
            }
        }
        Ok(descriptor)
    }

    pub async fn load(
        &self,
        descriptor_id: &str,
    ) -> Result<WorkbenchContentDescriptor, WorkbenchShellError> {
        validate_descriptor_id(descriptor_id)?;
        let bytes = tokio::fs::read(self.descriptor_path(descriptor_id))
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => {
                    WorkbenchShellError::NotFound(format!("unknown content {descriptor_id}"))
                }
                _ => WorkbenchShellError::Failed(error.to_string()),
            })?;
        let descriptor: WorkbenchContentDescriptor = serde_json::from_slice(&bytes)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        validate_metadata(&descriptor.name, &descriptor.media_type)?;
        let digest = descriptor
            .content_digest
            .strip_prefix("sha256:")
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| WorkbenchShellError::Failed("invalid stored content digest".into()))?;
        let digest_bytes = decode_hex_digest(digest)?;
        let expected_uri = format!(
            "ni:///sha-256;{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest_bytes)
        );
        let mut identity_input = descriptor.clone();
        identity_input.descriptor_id.clear();
        let canonical = serde_json::to_vec(&identity_input)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let expected_id = hex_digest(&Sha256::digest(canonical).into());
        if descriptor.descriptor_id != descriptor_id
            || descriptor.schema != CONTENT_SCHEMA
            || descriptor.uri != expected_uri
            || expected_id != descriptor_id
        {
            return Err(WorkbenchShellError::Failed(
                "stored content descriptor failed identity validation".into(),
            ));
        }
        Ok(descriptor)
    }

    pub async fn bytes(
        &self,
        descriptor: &WorkbenchContentDescriptor,
    ) -> Result<Vec<u8>, WorkbenchShellError> {
        let digest = descriptor
            .content_digest
            .strip_prefix("sha256:")
            .ok_or_else(|| WorkbenchShellError::Failed("unsupported content digest".into()))?;
        let bytes = tokio::fs::read(self.blob_path(digest))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        verify_bytes(descriptor, &bytes)?;
        Ok(bytes)
    }

    pub async fn blob_file(
        &self,
        descriptor: &WorkbenchContentDescriptor,
    ) -> Result<tokio::fs::File, WorkbenchShellError> {
        let digest = descriptor
            .content_digest
            .strip_prefix("sha256:")
            .ok_or_else(|| WorkbenchShellError::Failed("unsupported content digest".into()))?;
        let path = self.blob_path(digest);
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        if metadata.len() != descriptor.byte_length {
            return Err(WorkbenchShellError::Failed(format!(
                "stored bytes failed length validation for {}",
                descriptor.descriptor_id
            )));
        }
        tokio::fs::File::open(path)
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    pub async fn content_block(
        &self,
        descriptor_id: &str,
    ) -> Result<ContentBlock, WorkbenchShellError> {
        let descriptor = self.load(descriptor_id).await?;
        let bytes = self.bytes(&descriptor).await?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        if descriptor.media_type.starts_with("image/") {
            return Ok(ContentBlock::Image(
                ImageContent::new(encoded, descriptor.media_type).uri(descriptor.uri),
            ));
        }
        if descriptor.media_type.starts_with("audio/") {
            return Ok(ContentBlock::Audio(AudioContent::new(
                encoded,
                descriptor.media_type,
            )));
        }
        if is_text_media_type(&descriptor.media_type) {
            let text = String::from_utf8(bytes).map_err(|_| {
                WorkbenchShellError::Invalid(format!(
                    "content {} declares a textual MIME type but is not UTF-8",
                    descriptor.descriptor_id
                ))
            })?;
            return Ok(ContentBlock::Resource(EmbeddedResource::new(
                EmbeddedResourceResource::TextResourceContents(
                    TextResourceContents::new(text, descriptor.uri)
                        .mime_type(descriptor.media_type),
                ),
            )));
        }
        Ok(ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::BlobResourceContents(
                BlobResourceContents::new(encoded, descriptor.uri).mime_type(descriptor.media_type),
            ),
        )))
    }

    fn blob_path(&self, digest: &str) -> PathBuf {
        self.root.join("blobs").join(digest)
    }

    fn descriptor_path(&self, descriptor_id: &str) -> PathBuf {
        self.root
            .join("descriptors")
            .join(format!("{descriptor_id}.json"))
    }
}

struct ResolvedWorkspaceLink {
    path: PathBuf,
    byte_length: u64,
    media_type: String,
}

async fn resolve_workspace_link(
    resource: &ResourceLink,
    workspace: &Path,
    requested: &Path,
) -> Result<ResolvedWorkspaceLink, WorkbenchShellError> {
    let media_type = resource
        .mime_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    validate_metadata(&resource.name, &media_type)?;
    let workspace = tokio::fs::canonicalize(workspace)
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    let path = tokio::fs::canonicalize(requested)
        .await
        .map_err(|_| WorkbenchShellError::Invalid("linked workspace file is unavailable".into()))?;
    if !path.starts_with(&workspace) {
        return Err(WorkbenchShellError::Invalid(
            "linked file is outside the connection workspace".into(),
        ));
    }
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|_| WorkbenchShellError::Invalid("linked workspace file is unavailable".into()))?;
    if !metadata.is_file() {
        return Err(WorkbenchShellError::Invalid(
            "linked workspace resource is not a regular file".into(),
        ));
    }
    if metadata.len() > MAX_CONTENT_BYTES {
        return Err(content_too_large());
    }
    if let Some(declared) = resource.size {
        let declared = u64::try_from(declared).map_err(|_| {
            WorkbenchShellError::Invalid("resource link declares a negative byte size".into())
        })?;
        if declared != metadata.len() {
            return Err(WorkbenchShellError::Invalid(
                "resource link byte size differs from the workspace file".into(),
            ));
        }
    }
    Ok(ResolvedWorkspaceLink {
        path,
        byte_length: metadata.len(),
        media_type,
    })
}

struct CopiedWorkspaceFile {
    temporary: PathBuf,
    digest: [u8; 32],
}

async fn copy_workspace_file(
    root: &Path,
    source: &Path,
    expected_length: u64,
) -> Result<CopiedWorkspaceFile, WorkbenchShellError> {
    let temporary = root.join("tmp").join(format!(
        "workspace-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = copy_workspace_file_inner(source, &temporary, expected_length).await;
    match result {
        Ok(digest) => Ok(CopiedWorkspaceFile { temporary, digest }),
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary).await;
            Err(error)
        }
    }
}

async fn copy_workspace_file_inner(
    source: &Path,
    temporary: &Path,
    expected_length: u64,
) -> Result<[u8; 32], WorkbenchShellError> {
    let mut input = tokio::fs::File::open(source)
        .await
        .map_err(|_| WorkbenchShellError::Invalid("linked workspace file is unavailable".into()))?;
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary)
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut byte_length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).await.map_err(|_| {
            WorkbenchShellError::Invalid("linked workspace file became unreadable".into())
        })?;
        if read == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(
                u64::try_from(read)
                    .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?,
            )
            .ok_or_else(|| WorkbenchShellError::Invalid("content length overflow".into()))?;
        if byte_length > MAX_CONTENT_BYTES {
            return Err(content_too_large());
        }
        hasher.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    }
    if byte_length != expected_length {
        return Err(WorkbenchShellError::Invalid(
            "linked workspace file changed during import".into(),
        ));
    }
    output
        .flush()
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    output
        .sync_all()
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    Ok(hasher.finalize().into())
}

fn content_too_large() -> WorkbenchShellError {
    WorkbenchShellError::Invalid(format!(
        "content exceeds the {MAX_CONTENT_BYTES} byte Workbench limit"
    ))
}

fn validate_metadata(name: &str, media_type: &str) -> Result<(), WorkbenchShellError> {
    if name.trim().is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
        return Err(WorkbenchShellError::Invalid(
            "content name must contain 1-512 non-control characters".into(),
        ));
    }
    if media_type.trim().is_empty()
        || media_type.len() > 255
        || media_type.chars().any(char::is_control)
    {
        return Err(WorkbenchShellError::Invalid(
            "content type must contain 1-255 non-control characters".into(),
        ));
    }
    Ok(())
}

fn validate_descriptor_id(value: &str) -> Result<(), WorkbenchShellError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(WorkbenchShellError::Invalid(
            "content descriptor id must be a SHA-256 hex value".into(),
        ))
    }
}

fn verify_bytes(
    descriptor: &WorkbenchContentDescriptor,
    bytes: &[u8],
) -> Result<(), WorkbenchShellError> {
    let expected = descriptor
        .content_digest
        .strip_prefix("sha256:")
        .ok_or_else(|| WorkbenchShellError::Failed("unsupported content digest".into()))?;
    let actual = hex_digest(&Sha256::digest(bytes).into());
    if expected != actual || descriptor.byte_length != bytes.len() as u64 {
        Err(WorkbenchShellError::Failed(format!(
            "stored bytes failed descriptor verification for {}",
            descriptor.descriptor_id
        )))
    } else {
        Ok(())
    }
}

fn hex_digest(digest: &[u8; 32]) -> String {
    let mut result = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(result, "{byte:02x}");
    }
    result
}

fn decode_hex_digest(value: &str) -> Result<[u8; 32], WorkbenchShellError> {
    let mut result = [0_u8; 32];
    if value.len() != 64 {
        return Err(WorkbenchShellError::Failed(
            "invalid SHA-256 hex length".into(),
        ));
    }
    for (target, pair) in result.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let pair = std::str::from_utf8(pair)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        *target = u8::from_str_radix(pair, 16)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    }
    Ok(result)
}

fn is_text_media_type(media_type: &str) -> bool {
    let essence = media_type.split(';').next().unwrap_or(media_type).trim();
    essence.starts_with("text/")
        || matches!(
            essence,
            "application/json"
                | "application/ld+json"
                | "application/xml"
                | "application/javascript"
                | "application/x-yaml"
                | "application/toml"
        )
        || essence.ends_with("+json")
        || essence.ends_with("+xml")
}
