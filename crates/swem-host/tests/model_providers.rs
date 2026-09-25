//! The model providers a person sets up once: the shipped ones are listed,
//! a declared one is kept on disk and overrides a shipped id, a kind of key
//! the product does not know is refused, and a shipped provider cannot be
//! forgotten - only corrected.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use swem_host::workbench_shell::{
    DeclareModelProviderBody, ModelChoice, ModelProviderBook, ModelProviderOrigin,
    WorkbenchShellError,
};

fn scratch(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("swem-model-providers-{name}-{stamp}"));
    std::fs::create_dir_all(&root).expect("scratch root");
    root
}

#[test]
fn the_shipped_providers_are_listed_with_the_key_each_one_takes() {
    let book = ModelProviderBook::open(&scratch("shipped")).expect("open");
    let listed = book.list().expect("list");
    let anthropic = listed
        .iter()
        .find(|view| view.provider.id == "anthropic")
        .expect("the product ships Anthropic");
    assert_eq!(anthropic.provider.origin, ModelProviderOrigin::BuiltIn);
    assert_eq!(anthropic.key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    assert!(
        !anthropic.provider.models.is_empty(),
        "a shipped vendor lists models"
    );
    let ollama = listed
        .iter()
        .find(|view| view.provider.id == "ollama")
        .expect("the product ships Ollama");
    assert_eq!(ollama.key_env.as_deref(), Some("OLLAMA_HOST"));
    assert!(ollama.provider.base_url.is_some());
}

#[test]
fn a_declared_provider_is_kept_and_overrides_a_shipped_id() {
    let root = scratch("declared");
    let book = ModelProviderBook::open(&root).expect("open");
    let declared = book
        .declare(&DeclareModelProviderBody {
            id: "local".into(),
            name: "The box under the desk".into(),
            base_url: "http://127.0.0.1:8000/v1".into(),
            key_type: "generic_env_var".into(),
            models: vec![ModelChoice {
                id: "quality".into(),
                name: "Quality".into(),
            }],
        })
        .expect("declare");
    assert_eq!(declared.provider.origin, ModelProviderOrigin::Declared);
    assert!(root.join("local.json").is_file(), "kept as a document");
    assert_eq!(book.get("local").expect("get").models[0].id, "quality");

    // A second process sees it, and a shipped id can be corrected.
    let again = ModelProviderBook::open(&root).expect("reopen");
    assert!(
        again
            .list()
            .expect("list")
            .iter()
            .any(|view| view.provider.id == "local")
    );
    again
        .declare(&DeclareModelProviderBody {
            id: "openrouter".into(),
            name: "OpenRouter, my way".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            key_type: "openrouter_api_key".into(),
            models: vec![ModelChoice {
                id: "some/model".into(),
                name: String::new(),
            }],
        })
        .expect("override a shipped provider");
    let overridden = again.get("openrouter").expect("get");
    assert_eq!(overridden.origin, ModelProviderOrigin::Declared);
    assert_eq!(overridden.name, "OpenRouter, my way");
    // Forgetting the override brings the shipped one back.
    again.forget("openrouter").expect("forget the override");
    assert_eq!(
        again.get("openrouter").expect("shipped again").origin,
        ModelProviderOrigin::BuiltIn
    );
}

#[test]
fn a_kind_of_key_the_product_does_not_know_is_refused() {
    let book = ModelProviderBook::open(&scratch("unknown-key")).expect("open");
    let refused = book
        .declare(&DeclareModelProviderBody {
            id: "odd".into(),
            name: "Odd".into(),
            base_url: String::new(),
            key_type: "magic_token".into(),
            models: Vec::new(),
        })
        .expect_err("an unknown kind of key is refused");
    assert!(
        matches!(refused, WorkbenchShellError::Invalid(_)),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("magic_token"));
}

#[test]
fn a_shipped_provider_cannot_be_forgotten_and_a_stranger_is_not_found() {
    let book = ModelProviderBook::open(&scratch("forget")).expect("open");
    let shipped = book.forget("anthropic").expect_err("shipped");
    assert!(
        matches!(shipped, WorkbenchShellError::Invalid(_)),
        "{shipped:?}"
    );
    let stranger = book.forget("nobody").expect_err("unknown");
    assert!(
        matches!(stranger, WorkbenchShellError::NotFound(_)),
        "{stranger:?}"
    );
    let missing = book.get("nobody").expect_err("unknown");
    assert!(
        matches!(missing, WorkbenchShellError::NotFound(_)),
        "{missing:?}"
    );
}
