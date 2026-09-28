//! A provider's key: given once, kept for its owner alone, never shown.

use swem_host::{KeyError, KeyStore};

fn root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-keys-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create a root");
    root
}

#[test]
fn a_key_is_given_once_read_by_whoever_starts_an_engine_and_taken_away() {
    let root = root("given");
    let keys = KeyStore::open(&root.join("keys")).expect("open the keys");
    assert_eq!(keys.held("anthropic").expect("read"), None);
    assert!(keys.key("anthropic").expect("read").is_none());

    let held = keys
        .give("anthropic", "ANTHROPIC_API_KEY", "sk-first")
        .expect("give the key");
    assert_eq!(held.variable, "ANTHROPIC_API_KEY");
    assert_eq!(keys.held("anthropic").expect("read"), Some(held));

    // Given again, it is the new one; another process sees the same.
    keys.give("anthropic", "ANTHROPIC_API_KEY", "sk-second")
        .expect("give it again");
    let reopened = KeyStore::open(&root.join("keys")).expect("open again");
    let key = reopened
        .key("anthropic")
        .expect("read")
        .expect("the key is there");
    assert_eq!(
        (key.variable.as_str(), key.value.as_str()),
        ("ANTHROPIC_API_KEY", "sk-second")
    );
    assert!(
        !format!("{key:?}").contains("sk-second"),
        "a key printed for a log shows its value"
    );

    reopened.take("anthropic").expect("take it away");
    reopened
        .take("anthropic")
        .expect("taking nothing away is done");
    assert_eq!(keys.held("anthropic").expect("read"), None);
    std::fs::remove_dir_all(root).ok();
}

#[cfg(unix)]
#[test]
fn keys_are_their_owners_alone_even_after_somebody_opened_the_directory_up() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = root("closed");
    let directory = root.join("keys");
    let keys = KeyStore::open(&directory).expect("open the keys");
    keys.give("openai", "OPENAI_API_KEY", "sk-one")
        .expect("give the key");
    let mode = |path: &std::path::Path| {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(&directory), 0o700);
    assert_eq!(mode(&directory.join("openai.json")), 0o600);

    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
        .expect("open the directory up");
    KeyStore::open(&directory).expect("open the keys again");
    assert_eq!(mode(&directory), 0o700);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn what_is_not_a_key_is_refused_in_words_and_never_repeated() {
    let root = root("refused");
    let keys = KeyStore::open(&root.join("keys")).expect("open the keys");
    for (provider, variable, value) in [
        ("anthropic", "ANTHROPIC_API_KEY", "   "),
        ("anthropic", "1KEY", "sk"),
        ("anthropic", "A KEY", "sk"),
        ("../elsewhere", "ANTHROPIC_API_KEY", "sk"),
    ] {
        let refused = keys
            .give(provider, variable, value)
            .expect_err("it is refused");
        assert!(matches!(refused, KeyError::Invalid(_)), "{refused}");
    }
    assert_eq!(
        std::fs::read_dir(root.join("keys")).expect("list").count(),
        0,
        "a refusal left something behind"
    );

    // A file that is not a key says where it is wrong, not what it holds.
    std::fs::write(
        root.join("keys").join("groq.json"),
        br#"{"value": "sk-leaked""#,
    )
    .expect("write a broken file");
    let unreadable = keys.held("groq").expect_err("it cannot be read");
    assert!(
        !unreadable.to_string().contains("sk-leaked"),
        "{unreadable}"
    );
    std::fs::remove_dir_all(root).ok();
}
