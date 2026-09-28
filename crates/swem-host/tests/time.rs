//! A schedule is a message that arrives on time: when it is next due, how
//! it is claimed, what is said late and what is passed over.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use swem_host::{NewTimedMessage, RoutingLedger, RunState, TimedMessageChange, When};

const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;

/// A ledger of its own for a test, named for it: two tests that begin in
/// the same instant must not be given one ledger.
fn fixture(name: &str) -> (PathBuf, RoutingLedger, String, String, String) {
    let root = std::env::temp_dir().join(format!(
        "swem-time-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create a root");
    let mut ledger = RoutingLedger::open(&root.join("routes.sqlite3")).expect("open the ledger");
    let owner = ledger.owner().expect("the owner").participant_id;
    let agent = ledger
        .agent_of_profile("ada")
        .expect("an agent")
        .participant_id;
    let chat = ledger
        .start_chat("", &owner, std::slice::from_ref(&agent))
        .expect("a chat")
        .chat_id;
    (root, ledger, owner, agent, chat)
}

fn now() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis(),
    )
    .expect("a time that fits")
}

#[test]
fn when_a_schedule_is_next_due_and_how_a_person_says_it() {
    // 2026-09-28 is a Monday. 10:00:00 in Kyiv is 07:00:00 universal.
    let monday_ten_kyiv: u64 = 1_790_578_800_000;
    let every = When::Every { minutes: 30 };
    assert_eq!(every.next_after(1_000, 1_000), Some(1_000 + 30 * MINUTE));
    assert_eq!(
        every.next_after(1_000 + 95 * MINUTE, 1_000),
        Some(1_000 + 120 * MINUTE),
        "an interval counts from when the schedule was made"
    );
    assert_eq!(every.in_words(), "Every 30 minutes");
    assert_eq!(When::Every { minutes: 120 }.in_words(), "Every 2 hours");
    assert_eq!(When::Every { minutes: 1440 }.in_words(), "Every day");

    let daily = When::Cron {
        line: "0 6 * * *".into(),
        zone: "Europe/Kyiv".into(),
    };
    daily.check().expect("a day's time is kept");
    assert_eq!(
        daily.next_after(monday_ten_kyiv, 0),
        Some(monday_ten_kyiv + 20 * HOUR),
        "six in the morning there, the day after"
    );
    assert!(daily.in_words().starts_with("Every day at 06:00"));
    let weekly = When::Cron {
        line: "0 9 * * 1".into(),
        zone: "Europe/Kyiv".into(),
    };
    assert_eq!(
        weekly.next_after(monday_ten_kyiv, 0),
        Some(monday_ten_kyiv + 7 * 24 * HOUR - HOUR)
    );
    assert!(weekly.in_words().starts_with("Every Monday at 09:00"));
    assert_eq!(daily.least_apart(monday_ten_kyiv), Some(24 * 60));

    let once = When::Once { at_ms: 5_000 };
    assert_eq!(once.next_after(4_999, 0), Some(5_000));
    assert_eq!(once.next_after(5_000, 0), None);

    for unkept in [
        When::Every { minutes: 0 },
        When::Cron {
            line: "every morning".into(),
            zone: "Europe/Kyiv".into(),
        },
        When::Cron {
            line: "0 6 * * * *".into(),
            zone: "Europe/Kyiv".into(),
        },
        When::Cron {
            line: "0 6 * * *".into(),
            zone: "Atlantis/Centre".into(),
        },
    ] {
        assert!(unkept.check().is_err(), "{unkept:?} was kept");
    }
}

#[test]
fn what_is_due_is_claimed_once_said_late_or_passed_over() {
    let (root, mut ledger, owner, agent, chat) = fixture("claimed");
    let made = now();
    let schedule = ledger
        .keep_schedule(&NewTimedMessage {
            agent_id: agent.clone(),
            made_by: owner.clone(),
            chat_id: Some(chat.clone()),
            say: "  Check the changelog against what was merged  ".into(),
            when: When::Every { minutes: 10 },
        })
        .expect("keep a schedule");
    assert_eq!(schedule.say, "Check the changelog against what was merged");
    let first = schedule.next_due_ms.expect("it is due some time");
    assert!(first >= made + 10 * MINUTE && first <= now() + 10 * MINUTE);
    let speaker = ledger.participant(&schedule.speaker_id).expect("its voice");
    assert_eq!(speaker.name, "Every 10 minutes");
    assert_eq!(speaker.made_by.as_deref(), Some(owner.as_str()));

    // Not yet.
    assert!(ledger.claim_due(first - 1).expect("look").is_empty());

    // On time, and once whoever looks.
    let due = ledger.claim_due(first + 1_000).expect("look");
    assert_eq!(due.len(), 1);
    assert_eq!((due[0].due_ms, due[0].late), (first, false));
    assert!(
        RoutingLedger::open(&root.join("routes.sqlite3"))
            .expect("another keeper")
            .claim_due(first + 2_000)
            .expect("look")
            .is_empty(),
        "what one keeper took up was taken up by another"
    );
    let next = ledger
        .schedule(&schedule.schedule_id)
        .expect("read")
        .next_due_ms;
    assert_eq!(next, Some(first + 10 * MINUTE));

    // The run before it has not ended: the next one is passed over, in words.
    assert!(
        ledger
            .claim_due(first + 10 * MINUTE)
            .expect("look")
            .is_empty()
    );
    ledger
        .run_said(&schedule.schedule_id, first, &chat, "d_first")
        .expect("said");
    ledger
        .run_ended(&schedule.schedule_id, first, RunState::Answered, None)
        .expect("ended");

    // Nobody kept time for three hours: said once, late, and the moments
    // in between are not made up for.
    let back = first + 3 * HOUR + 4 * MINUTE;
    let due = ledger.claim_due(back).expect("look");
    assert_eq!(due.len(), 1);
    assert!(due[0].late);
    assert_eq!(due[0].due_ms, first + 20 * MINUTE);
    assert_eq!(
        ledger
            .schedule(&schedule.schedule_id)
            .expect("read")
            .next_due_ms,
        Some(first + 3 * HOUR + 10 * MINUTE)
    );
    ledger
        .run_ended(
            &schedule.schedule_id,
            first + 20 * MINUTE,
            RunState::Failed,
            Some("the agent is not signed in"),
        )
        .expect("ended");

    // Nobody kept time for two days: not said at all, and said so.
    let days_later = first + 50 * HOUR;
    assert!(ledger.claim_due(days_later).expect("look").is_empty());

    let runs = ledger.runs(Some(&agent), 10).expect("runs");
    assert_eq!(
        runs.iter()
            .map(|run| (run.state, run.late, run.note.as_deref().unwrap_or("")))
            .collect::<Vec<_>>(),
        [
            (
                RunState::Skipped,
                true,
                "nobody kept time for more than a day after it was due"
            ),
            (RunState::Failed, true, "the agent is not signed in"),
            (RunState::Skipped, false, "the run before it had not ended"),
            (RunState::Answered, false, ""),
        ]
    );
    assert!(ledger.runs_going().expect("going").is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn a_schedule_is_paused_changed_due_once_and_forgotten() {
    let (root, mut ledger, owner, agent, chat) = fixture("paused");
    let new = |when: When| NewTimedMessage {
        agent_id: agent.clone(),
        made_by: owner.clone(),
        chat_id: Some(chat.clone()),
        say: "Look for a new release branch and say so".into(),
        when,
    };
    let schedule = ledger
        .keep_schedule(&new(When::Every { minutes: 5 }))
        .expect("keep");
    let due = schedule.next_due_ms.expect("due");

    // Off: nothing is due, whatever the time.
    let off = ledger
        .change_schedule(
            &schedule.schedule_id,
            &TimedMessageChange {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .expect("turn it off");
    assert!(!off.enabled && off.next_due_ms.is_none());
    assert!(ledger.claim_due(due + HOUR).expect("look").is_empty());
    // On again: due at its next moment after now, not at the one it missed.
    let on = ledger
        .change_schedule(
            &schedule.schedule_id,
            &TimedMessageChange {
                enabled: Some(true),
                say: Some("Look for a release branch".into()),
                ..Default::default()
            },
        )
        .expect("turn it on");
    assert!(on.enabled && on.next_due_ms.is_some_and(|next| next > now()));
    assert_eq!(on.say, "Look for a release branch");

    // Once: said, and then it is over.
    let once = ledger
        .keep_schedule(&new(When::Once {
            at_ms: now() + MINUTE,
        }))
        .expect("keep");
    let at = once.next_due_ms.expect("due");
    assert_eq!(ledger.claim_due(at).expect("look").len(), 1);
    let after = ledger.schedule(&once.schedule_id).expect("read");
    assert!(!after.enabled && after.next_due_ms.is_none());
    assert!(
        ledger
            .keep_schedule(&new(When::Once { at_ms: 1_000 }))
            .is_err(),
        "a moment that has passed was kept"
    );

    // What cannot be kept is refused in words.
    for (say, chat_id, agent_id) in [
        ("   ", Some(chat.clone()), agent.clone()),
        ("Check", Some("c_nowhere".to_owned()), agent.clone()),
        ("Check", Some(chat.clone()), owner.clone()),
    ] {
        let refused = ledger.keep_schedule(&NewTimedMessage {
            agent_id,
            made_by: owner.clone(),
            chat_id,
            say: say.into(),
            when: When::Every { minutes: 5 },
        });
        assert!(refused.is_err(), "{say:?} was kept");
    }

    ledger
        .forget_schedule(&schedule.schedule_id)
        .expect("forget");
    assert_eq!(
        ledger
            .schedules(Some(&agent))
            .expect("list")
            .iter()
            .map(|one| one.schedule_id.as_str())
            .collect::<Vec<_>>(),
        [once.schedule_id.as_str()]
    );
    assert!(ledger.schedule(&schedule.schedule_id).is_err());
    std::fs::remove_dir_all(root).ok();
}
