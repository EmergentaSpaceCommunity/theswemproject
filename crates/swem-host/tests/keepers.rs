//! Who keeps an agent's time is what was chosen, and the default otherwise.

use std::time::{SystemTime, UNIX_EPOCH};

use swem_host::{KEEPER_LOOK_SCHEMA, KEPT_BY_SWEM, KEPT_BY_THE_SYSTEM, KeeperLook, Keepers};

fn keepers(label: &str) -> Keepers {
    Keepers::at(std::env::temp_dir().join(format!(
        "swem-keepers-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    )))
}

#[test]
fn an_agent_follows_the_default_until_somebody_chooses() {
    let keepers = keepers("chosen");
    // Nothing was ever chosen: the running product keeps everybody's time.
    assert_eq!(keepers.default_keeper(), KEPT_BY_SWEM);
    assert_eq!(keepers.keeper_of("ada"), KEPT_BY_SWEM);
    assert_eq!(keepers.chosen_for("ada"), None);

    keepers
        .choose_for("ada", Some(KEPT_BY_THE_SYSTEM))
        .expect("choose for ada");
    assert_eq!(keepers.keeper_of("ada"), KEPT_BY_THE_SYSTEM);
    assert_eq!(keepers.keeper_of("bo"), KEPT_BY_SWEM);

    keepers
        .make_default(KEPT_BY_THE_SYSTEM)
        .expect("make the default");
    assert_eq!(keepers.keeper_of("bo"), KEPT_BY_THE_SYSTEM);
    keepers
        .choose_for("bo", Some(KEPT_BY_SWEM))
        .expect("choose for bo");
    assert_eq!(keepers.keeper_of("bo"), KEPT_BY_SWEM);
    keepers.choose_for("bo", None).expect("let bo follow");
    assert_eq!(keepers.keeper_of("bo"), KEPT_BY_THE_SYSTEM);

    assert!(keepers.make_default("a-clock-in-the-machine").is_err());
    assert!(keepers.choose_for("ada", Some("cron")).is_err());
    assert_eq!(keepers.default_keeper(), KEPT_BY_THE_SYSTEM);
}

#[test]
fn a_look_is_written_down_and_read() {
    let keepers = keepers("look");
    assert_eq!(keepers.last_look(KEPT_BY_THE_SYSTEM), None);
    let look = KeeperLook {
        schema: KEEPER_LOOK_SCHEMA.into(),
        looked_ms: 1_790_000_000_000,
        said: 2,
        kept_elsewhere: false,
        said_last_ms: None,
    };
    keepers
        .looked(KEPT_BY_THE_SYSTEM, &look)
        .expect("write the look down");
    let kept = keepers.last_look(KEPT_BY_THE_SYSTEM).expect("the look");
    assert_eq!(kept.said, 2);
    assert_eq!(kept.said_last_ms, Some(1_790_000_000_000));

    // A look that found nothing does not forget the one that said something.
    keepers
        .looked(
            KEPT_BY_THE_SYSTEM,
            &KeeperLook {
                looked_ms: 1_790_000_060_000,
                said: 0,
                kept_elsewhere: true,
                ..look
            },
        )
        .expect("write the next look down");
    let kept = keepers.last_look(KEPT_BY_THE_SYSTEM).expect("the look");
    assert_eq!(kept.looked_ms, 1_790_000_060_000);
    assert!(kept.kept_elsewhere);
    assert_eq!(kept.said_last_ms, Some(1_790_000_000_000));
}
