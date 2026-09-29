//! Who may come in to a Workbench served at an address.
//!
//! The ceremonies are done with a passkey kept in software: what a browser
//! and a device would answer is answered here, and checked by the same code
//! that checks a real one.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use swem_host::{Access, AccessError, CODES_MADE, CameBy, CameIn, May, Principal};
use url::Url;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;

const FROM: &str = "192.0.2.14";

fn root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "swem-access-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ))
}

fn address() -> Url {
    Url::parse("https://workbench.example.org").expect("an address")
}

fn access(label: &str) -> Access {
    Access::open(&root(label), &address(), "ada").expect("the book opens")
}

type Device = WebauthnAuthenticator<SoftPasskey>;

fn a_device() -> Device {
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

fn register(access: &Access, device: &mut Device, word: &str, name: &str) -> CameIn {
    let begun = access
        .begin_registering(word, name, FROM)
        .expect("registering begins");
    let asked = serde_json::from_value(begun.asked).expect("what a browser is asked");
    let answered = device
        .do_registration(address(), asked)
        .expect("the device registers");
    access
        .finish_registering(
            &begun.ceremony,
            &serde_json::to_value(answered).expect("an answer"),
            FROM,
        )
        .expect("registering finishes")
}

fn sign_in(access: &Access, device: &mut Device) -> Result<CameIn, AccessError> {
    let begun = access.begin_signing_in()?;
    let asked = serde_json::from_value(begun.asked).expect("what a browser is asked");
    let answered = device
        .do_authentication(address(), asked)
        .map_err(|error| AccessError::Refused(format!("{error:?}")))?;
    access.finish_signing_in(
        &begun.ceremony,
        &serde_json::to_value(answered).expect("an answer"),
        FROM,
    )
}

#[test]
fn a_workbench_is_made_somebodys_own_once_and_they_come_back_with_their_device() {
    let access = access("claim");
    assert!(!access.claimed().expect("read"));
    let word = access
        .word_of_a_first_start()
        .expect("a word")
        .expect("it belongs to nobody");

    let mut laptop = a_device();
    // Typed as a person types: in capitals, with spaces.
    let typed = word.to_uppercase().replace('-', " ");
    let came = register(&access, &mut laptop, &typed, "MacBook");
    assert!(matches!(&came.principal.by, CameBy::Device { name, .. } if name == "MacBook"));
    let codes = came.codes.expect("the codes are said at the first start");
    assert_eq!(codes.len(), CODES_MADE);
    assert!(access.claimed().expect("read"));
    assert_eq!(
        access.of_session(&came.session).expect("read"),
        Some(came.principal.clone())
    );

    // It belongs to somebody: no word of a first start is made again, and
    // the one that was used lets nobody else in.
    assert_eq!(access.word_of_a_first_start().expect("read"), None);
    assert!(matches!(
        access.begin_registering(&word, "Somebody else", "203.0.113.7"),
        Err(AccessError::Refused(_))
    ));

    let again = sign_in(&access, &mut laptop).expect("the laptop signs in");
    assert_ne!(again.session, came.session);
    assert!(again.codes.is_none());
    let devices = access.devices().expect("devices");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].last_from.as_deref(), Some(FROM));

    access.end_session(&again.session, FROM).expect("sign out");
    assert_eq!(access.of_session(&again.session).expect("read"), None);
    assert!(access.of_session(&came.session).expect("read").is_some());
}

#[test]
fn a_device_that_was_never_registered_does_not_come_in() {
    let access = access("stranger");
    let word = access
        .word_of_a_first_start()
        .expect("a word")
        .expect("one");
    let mut laptop = a_device();
    register(&access, &mut laptop, &word, "MacBook");

    let mut stranger = a_device();
    assert!(sign_in(&access, &mut stranger).is_err());
    assert_eq!(access.of_session("not-a-session").expect("read"), None);
}

#[test]
fn a_second_device_is_added_with_a_word_and_one_taken_away_comes_in_no_more() {
    let access = access("second");
    let word = access
        .word_of_a_first_start()
        .expect("a word")
        .expect("one");
    let mut laptop = a_device();
    let came = register(&access, &mut laptop, &word, "MacBook");

    let word = access
        .word_for_a_device(&came.principal, FROM)
        .expect("a word for the phone");
    let mut phone = a_device();
    let on_the_phone = register(&access, &mut phone, &word, "iPhone");
    assert!(
        on_the_phone.codes.is_none(),
        "codes are said once, at the first start"
    );
    assert_eq!(access.devices().expect("devices").len(), 2);

    let CameBy::Device { device_id, .. } = &on_the_phone.principal.by else {
        panic!("the phone came in as a device");
    };
    access
        .take_away(device_id, &came.principal, FROM)
        .expect("the phone is taken away");
    // What it had open is closed, and it signs in no more.
    assert_eq!(
        access.of_session(&on_the_phone.session).expect("read"),
        None
    );
    assert!(sign_in(&access, &mut phone).is_err());
    assert!(sign_in(&access, &mut laptop).is_ok());
}

#[test]
fn a_code_brings_a_person_back_once_to_register_a_device() {
    let access = access("codes");
    let word = access
        .word_of_a_first_start()
        .expect("a word")
        .expect("one");
    let mut laptop = a_device();
    let came = register(&access, &mut laptop, &word, "MacBook");
    let codes = came.codes.expect("codes");

    let back = access.come_back(&codes[0], FROM).expect("a code lets in");
    assert_eq!(back.principal.by, CameBy::Code);
    // With a code a person may register a device and do nothing else.
    assert!(!back.principal.may(May::Everything));
    assert!(!back.principal.may(May::SayWhatIsDue));
    let word = access
        .word_for_a_device(&back.principal, FROM)
        .expect("a word for a new device");
    let mut new_laptop = a_device();
    register(&access, &mut new_laptop, &word, "New MacBook");

    assert!(matches!(
        access.come_back(&codes[0], FROM),
        Err(AccessError::Refused(_))
    ));
    assert_eq!(access.codes_left().expect("codes").left, CODES_MADE - 1);

    // Made anew, the old ones are good no more.
    let new = access.new_codes(&came.principal, FROM).expect("new codes");
    assert!(access.come_back(&codes[1], FROM).is_err());
    assert!(access.come_back(&new[0], FROM).is_ok());
}

#[test]
fn a_token_opens_what_it_says_and_nothing_once_withdrawn() {
    let access = access("tokens");
    let owner = Principal::of_this_run();
    let (token, opens) = access
        .make_token("Morning scheduler", &[May::SayWhatIsDue], &owner, FROM)
        .expect("a token");
    assert!(opens.starts_with("swem_"));

    let who = access
        .of_token(&opens, "198.51.100.4")
        .expect("read")
        .expect("the token is known");
    assert!(who.may(May::SayWhatIsDue));
    assert!(!who.may(May::Everything));
    // A program that holds a token makes no tokens and lets no device in.
    assert!(
        access
            .make_token("Another", &[May::Everything], &who, FROM)
            .is_err()
    );
    assert!(access.word_for_a_device(&who, FROM).is_err());

    let listed = access.tokens().expect("tokens");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].last_from.as_deref(), Some("198.51.100.4"));
    assert!(!format!("{listed:?}").contains(&opens));

    access
        .withdraw(&token.token_id, &owner, FROM)
        .expect("withdrawn");
    assert_eq!(access.of_token(&opens, "198.51.100.4").expect("read"), None);
    assert!(access.tokens().expect("tokens").is_empty());
}

#[test]
fn tries_that_fail_are_limited_and_written_down() {
    let access = access("tries");
    access.word_of_a_first_start().expect("a word");
    let mut refused = 0;
    let mut limited = false;
    for _ in 0..8 {
        match access.begin_registering("aaaa-bbbb-cccc-dddd", "Guess", "203.0.113.7") {
            Err(AccessError::Refused(_)) => refused += 1,
            Err(AccessError::TooManyTries { minutes }) => {
                assert!(minutes >= 1);
                limited = true;
            }
            other => panic!("a guess was answered with {other:?}"),
        }
    }
    assert_eq!(refused, 5);
    assert!(limited);
    // Somebody elsewhere is not held for it.
    assert!(matches!(
        access.begin_registering("aaaa-bbbb-cccc-dddd", "Guess", "198.51.100.23"),
        Err(AccessError::Refused(_))
    ));

    let happened = access.happened(20).expect("what was done");
    assert!(happened.iter().all(|one| one.refused));
    assert!(happened.iter().any(|one| one.from == "203.0.113.7"));
    assert!(
        happened.iter().all(|one| !one.what.contains("aaaa")),
        "what somebody came with is not written down"
    );
}

#[test]
fn nothing_that_opens_is_kept_as_it_was_said() {
    let root = root("kept");
    let access = Access::open(&root, &address(), "ada").expect("the book opens");
    let word = access
        .word_of_a_first_start()
        .expect("a word")
        .expect("one");
    let mut laptop = a_device();
    let came = register(&access, &mut laptop, &word, "MacBook");
    let (_, opens) = access
        .make_token("Script", &[May::Everything], &came.principal, FROM)
        .expect("a token");
    let codes = came.codes.expect("codes");
    drop(access);

    let mut kept = Vec::new();
    for entry in std::fs::read_dir(&root).expect("the book's folder") {
        kept.extend(std::fs::read(entry.expect("an entry").path()).expect("a file"));
    }
    let kept = String::from_utf8_lossy(&kept);
    for said in [&word, &came.session, &opens, &codes[0]] {
        assert!(
            !kept.contains(said.as_str()),
            "{said} is kept as it was said"
        );
        assert!(!kept.contains(&said.replace('-', "")));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |path: &std::path::Path| {
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(&root.join("access.db")), 0o600);
    }
}
