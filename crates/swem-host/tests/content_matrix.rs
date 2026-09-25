use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    AudioContent, BlobResourceContents, ContentBlock, EmbeddedResource, EmbeddedResourceResource,
    ImageContent, ResourceLink, TextContent, TextResourceContents,
};
use base64::Engine as _;
use serde_json::Value;
use swem_host::{
    IntegrationKind, LaunchCommand, NativeSessionOptions, Readiness, SupplyError, discover_agents,
    run_native_session_with_content, verify_discovered_agent,
};

const OBSERVATION_FILE: &str = ".swem-content-observation.json";

fn fixture_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "swem-content-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ))
}

fn fixture_launch(arguments: Vec<String>) -> (LaunchCommand, PathBuf) {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    (
        LaunchCommand {
            executable: executable.display().to_string(),
            args: arguments,
            integration: IntegrationKind::DirectAcp,
        },
        executable,
    )
}

fn encoded(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[tokio::test]
async fn all_stable_acp_content_variants_cross_the_native_seam_without_transformation() {
    let root = fixture_root("matrix");
    fs::create_dir_all(&root).expect("create content workspace");
    let transcript = root.join("transcript.jsonl");
    let image_bytes = [0_u8, 1, 2, 250, 251, 252, 10];
    let audio_bytes = [82_u8, 73, 70, 70, 0, 255, 16, 32];
    let blob_bytes = [0_u8, 159, 146, 150, 255, 65, 66, 67];
    let embedded_text = "line one\nстрока two\n";
    let prompt = vec![
        ContentBlock::Text(TextContent::new("SWEM_CONTENT_MATRIX")),
        ContentBlock::ResourceLink(
            ResourceLink::new("linked-source", "file:///workspace/linked-source.txt")
                .mime_type("text/plain")
                .size(321),
        ),
        ContentBlock::Image(
            ImageContent::new(encoded(&image_bytes), "image/png")
                .uri("file:///workspace/image.png"),
        ),
        ContentBlock::Audio(AudioContent::new(encoded(&audio_bytes), "audio/wav")),
        ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(
                TextResourceContents::new(embedded_text, "file:///workspace/context.txt")
                    .mime_type("text/plain; charset=utf-8"),
            ),
        )),
        ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::BlobResourceContents(
                BlobResourceContents::new(encoded(&blob_bytes), "file:///workspace/payload.bin")
                    .mime_type("application/octet-stream"),
            ),
        )),
    ];
    let (launch, executable) = fixture_launch(Vec::new());
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.transcript_path = Some(transcript.clone());

    let outcome = run_native_session_with_content(
        &launch,
        &executable,
        &root,
        std::slice::from_ref(&prompt),
        &options,
    )
    .await
    .expect("run ACP content matrix");

    assert_eq!(outcome.turns.len(), 1);
    assert_eq!(outcome.turns[0].prompt, "SWEM_CONTENT_MATRIX");
    assert_eq!(outcome.turns[0].prompt_content, prompt);
    assert_eq!(outcome.turns[0].reply_content, prompt);
    assert_eq!(outcome.turns[0].reply_text, "SWEM_CONTENT_MATRIX");

    let observations: Value = serde_json::from_slice(
        &fs::read(root.join(OBSERVATION_FILE)).expect("read fixture observation"),
    )
    .expect("parse fixture observation");
    let observations = observations.as_array().expect("observation array");
    assert_eq!(observations.len(), 6);
    assert_eq!(observations[1]["type"], "resource_link");
    assert_eq!(
        observations[1]["uri"],
        "file:///workspace/linked-source.txt"
    );
    assert_eq!(
        fs::read(root.join("content-2-image.bin")).expect("read image oracle"),
        image_bytes
    );
    assert_eq!(
        fs::read(root.join("content-3-audio.bin")).expect("read audio oracle"),
        audio_bytes
    );
    assert_eq!(
        fs::read(root.join("content-4-resource.txt")).expect("read text resource oracle"),
        embedded_text.as_bytes()
    );
    assert_eq!(
        fs::read(root.join("content-5-resource.bin")).expect("read blob oracle"),
        blob_bytes
    );

    let transcript = fs::read_to_string(transcript).expect("read content transcript");
    assert!(transcript.contains("sha256:"));
    assert!(!transcript.contains(&encoded(&image_bytes)));
    assert!(!transcript.contains(&encoded(&audio_bytes)));
    assert!(!transcript.contains(&encoded(&blob_bytes)));
    fs::remove_dir_all(root).expect("remove content fixture");
}

#[tokio::test]
async fn unsupported_rich_content_fails_before_session_creation_without_path_fallback() {
    let root = fixture_root("unsupported");
    fs::create_dir_all(&root).expect("create unsupported workspace");
    let transcript = root.join("transcript.jsonl");
    let prompt = vec![
        ContentBlock::Text(TextContent::new("SWEM_CONTENT_MATRIX")),
        ContentBlock::Image(ImageContent::new(encoded(b"not-a-path"), "image/png")),
    ];
    let (launch, executable) = fixture_launch(vec!["--baseline-content-only".into()]);
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.transcript_path = Some(transcript.clone());

    let error = run_native_session_with_content(&launch, &executable, &root, &[prompt], &options)
        .await
        .expect_err("image must require advertised prompt capability");
    assert!(matches!(error, SupplyError::Protocol(_)));
    assert!(error.to_string().contains("image"));
    let transcript = fs::read_to_string(transcript).expect("read unsupported transcript");
    assert!(!transcript.contains("session/new"));
    assert!(!transcript.contains("not-a-path"));
    assert!(!root.join(OBSERVATION_FILE).exists());
    fs::remove_dir_all(root).expect("remove unsupported fixture");
}

#[tokio::test]
async fn text_and_resource_link_remain_baseline_content_without_capability_flags() {
    let root = fixture_root("baseline");
    fs::create_dir_all(&root).expect("create baseline workspace");
    let prompt = vec![
        ContentBlock::Text(TextContent::new("SWEM_CONTENT_MATRIX")),
        ContentBlock::ResourceLink(ResourceLink::new(
            "baseline-link",
            "file:///workspace/baseline.txt",
        )),
    ];
    let (launch, executable) = fixture_launch(vec!["--baseline-content-only".into()]);
    let options = NativeSessionOptions::new(Duration::from_secs(10));

    let outcome = run_native_session_with_content(
        &launch,
        &executable,
        &root,
        std::slice::from_ref(&prompt),
        &options,
    )
    .await
    .expect("baseline ACP content");
    assert_eq!(outcome.turns[0].reply_content, prompt);
    fs::remove_dir_all(root).expect("remove baseline fixture");
}

#[tokio::test]
async fn malformed_binary_content_is_rejected_before_it_reaches_the_agent() {
    let root = fixture_root("malformed");
    fs::create_dir_all(&root).expect("create malformed workspace");
    let prompt = vec![
        ContentBlock::Text(TextContent::new("SWEM_CONTENT_MATRIX")),
        ContentBlock::Audio(AudioContent::new("%%%not-base64%%%", "audio/wav")),
    ];
    let (launch, executable) = fixture_launch(Vec::new());
    let error = run_native_session_with_content(
        &launch,
        &executable,
        &root,
        &[prompt],
        &NativeSessionOptions::new(Duration::from_secs(10)),
    )
    .await
    .expect_err("malformed base64 must fail closed");
    assert!(error.to_string().contains("valid standard base64"));
    assert!(!root.join(OBSERVATION_FILE).exists());
    fs::remove_dir_all(root).expect("remove malformed fixture");
}

#[tokio::test]
#[ignore = "live ACP content: requires SWEM_NATIVE_AGENT and agent credentials"]
async fn live_agent_receives_baseline_and_every_rich_content_kind_it_advertises() {
    let agent_id = std::env::var("SWEM_NATIVE_AGENT")
        .expect("set SWEM_NATIVE_AGENT=<catalog id> for this explicit live test");
    let discovery = discover_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .unwrap_or_else(|| panic!("unknown catalog agent {agent_id}"));
    assert_ne!(discovery.readiness, Readiness::Absent, "agent is absent");
    let handshake = verify_discovered_agent(&discovery, Duration::from_secs(30))
        .await
        .expect("verify live ACP agent");
    assert_eq!(handshake.readiness, Readiness::HandshakeReady);
    let launch = discovery.launch.expect("live agent launch command");
    let executable = discovery
        .executable_path
        .expect("live agent executable path");
    let capabilities = &handshake.agent_capabilities["promptCapabilities"];
    let image = capabilities["image"].as_bool().unwrap_or(false);
    let audio = capabilities["audio"].as_bool().unwrap_or(false);
    let embedded = capabilities["embeddedContext"].as_bool().unwrap_or(false);

    let root = fixture_root(&format!("live-{agent_id}"));
    fs::create_dir_all(&root).expect("create live content workspace");
    fs::write(root.join("linked.txt"), b"SWEM_LINKED_RESOURCE_7F31")
        .expect("write linked resource");
    let linked_uri = format!(
        "file:///{}",
        root.join("linked.txt")
            .display()
            .to_string()
            .replace('\\', "/")
    );
    let transcript = root.join("transcript.jsonl");
    let mut prompt = vec![
        ContentBlock::Text(TextContent::new(
            "Inspect the structured ACP content in this message. Reply with the exact marker SWEM_LIVE_CONTENT_OK. Do not use shell commands.",
        )),
        ContentBlock::ResourceLink(
            ResourceLink::new("linked.txt", linked_uri)
                .mime_type("text/plain")
                .size(25),
        ),
    ];
    if image {
        prompt.push(ContentBlock::Image(ImageContent::new(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
            "image/png",
        )));
    }
    if audio {
        prompt.push(ContentBlock::Audio(AudioContent::new(
            encoded(b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x40\x1f\0\0\x80\x3e\0\0\x02\0\x10\0data\0\0\0\0"),
            "audio/wav",
        )));
    }
    if embedded {
        prompt.push(ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(
                TextResourceContents::new(
                    "SWEM_EMBEDDED_CONTEXT_C924",
                    "swem://live/content-context",
                )
                .mime_type("text/plain"),
            ),
        )));
    }
    let mut options = NativeSessionOptions::new(Duration::from_secs(180));
    options.transcript_path = Some(transcript.clone());
    let outcome = run_native_session_with_content(&launch, &executable, &root, &[prompt], &options)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "live ACP content failed: {error}; transcript: {}",
                transcript.display()
            )
        });
    assert!(
        outcome.turns[0].reply_text.contains("SWEM_LIVE_CONTENT_OK"),
        "live agent did not return marker; reply: {:?}; transcript: {}",
        outcome.turns[0].reply_text,
        transcript.display()
    );
    eprintln!(
        "live ACP content evidence: agent={agent_id}, image={image}, audio={audio}, embedded_context={embedded}, transcript={}",
        transcript.display()
    );
}
