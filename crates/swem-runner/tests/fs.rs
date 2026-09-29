//! An agent's files are reached inside the folder it was given and nowhere
//! else, and a file is written only while it is what was read.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use swem_runner::{EntryKind, FsAnswer, FsRequest, Why, answer, digest_of};

/// A folder an agent works in, and beside it what it must not reach.
fn a_folder(label: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "swem-runner-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let root = base.join("works-in");
    fs::create_dir_all(root.join("src")).expect("make the folder");
    fs::write(root.join("src").join("main.rs"), "fn main() {}\n").expect("a file");
    fs::write(root.join("README.md"), "# Notes\n").expect("a file");
    fs::write(root.join(".hidden"), "x").expect("a file");
    fs::write(base.join("secret.txt"), "not the agent's").expect("what is outside");
    (root, base)
}

fn refused(answer: &FsAnswer) -> Why {
    match answer {
        FsAnswer::Refused { why, .. } => *why,
        other => panic!("it was not refused: {other:?}"),
    }
}

fn read(root: &Path, path: &str) -> FsAnswer {
    answer(&FsRequest::Read {
        root: root.to_owned(),
        path: path.to_owned(),
        at_most: 1024,
    })
}

fn write(root: &Path, path: &str, text: &str, was: Option<String>, over: bool) -> FsAnswer {
    answer(&FsRequest::Write {
        root: root.to_owned(),
        path: path.to_owned(),
        bytes: STANDARD.encode(text),
        was,
        over,
    })
}

#[test]
fn a_folder_is_listed_folders_first_and_a_file_is_read() {
    let (root, base) = a_folder("listed");
    let FsAnswer::Listed { entries } = answer(&FsRequest::List {
        root: root.clone(),
        dir: String::new(),
    }) else {
        panic!("the folder was not listed");
    };
    let names: Vec<(&str, EntryKind)> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.kind))
        .collect();
    assert_eq!(
        names,
        [
            ("src", EntryKind::Folder),
            (".hidden", EntryKind::File),
            ("README.md", EntryKind::File),
        ]
    );
    assert_eq!(entries[2].byte_length, 8);
    assert!(entries[2].modified_ms.is_some());

    let FsAnswer::Read {
        bytes,
        sha256,
        byte_length,
    } = read(&root, "src/main.rs")
    else {
        panic!("the file was not read");
    };
    assert_eq!(STANDARD.decode(bytes).expect("bytes"), b"fn main() {}\n");
    assert_eq!(sha256, digest_of(b"fn main() {}\n"));
    assert_eq!(byte_length, 13);

    assert_eq!(refused(&read(&root, "src")), Why::NotAFile);
    assert_eq!(refused(&read(&root, "nothing.txt")), Why::NotFound);
    fs::write(root.join("big.bin"), vec![0_u8; 2048]).expect("a big file");
    assert_eq!(refused(&read(&root, "big.bin")), Why::TooBig);
    fs::remove_dir_all(base).expect("remove the fixture");
}

#[test]
fn nothing_outside_its_folder_is_reached() {
    let (root, base) = a_folder("outside");
    for climbing in [
        "../secret.txt",
        "src/../../secret.txt",
        "/etc/passwd",
        "src//main.rs",
        "src\\main.rs",
        "./README.md",
    ] {
        assert_eq!(refused(&read(&root, climbing)), Why::Outside, "{climbing}");
        assert_eq!(
            refused(&write(&root, climbing, "x", None, true)),
            Why::Outside,
            "{climbing}"
        );
    }
    assert_eq!(
        refused(&answer(&FsRequest::Remove {
            root: root.clone(),
            path: String::new(),
            with_all: true,
        })),
        Why::Outside,
        "its folder itself is not removed"
    );

    #[cfg(unix)]
    {
        // A link planted in the folder serves nothing outside it, and
        // nothing is written through it.
        std::os::unix::fs::symlink(base.join("secret.txt"), root.join("way-out")).expect("a link");
        std::os::unix::fs::symlink(&base, root.join("up")).expect("a link to a folder");
        std::os::unix::fs::symlink(root.join("README.md"), root.join("notes")).expect("a link");
        assert_eq!(refused(&read(&root, "way-out")), Why::Outside);
        assert_eq!(refused(&read(&root, "up/secret.txt")), Why::Outside);
        assert_eq!(
            refused(&write(&root, "way-out", "taken", None, true)),
            Why::Outside
        );
        assert_eq!(
            refused(&write(&root, "up/planted.txt", "x", None, false)),
            Why::Outside
        );
        assert_eq!(
            fs::read_to_string(base.join("secret.txt")).expect("still there"),
            "not the agent's"
        );
        // A link that stays inside is what it leads to.
        assert!(matches!(read(&root, "notes"), FsAnswer::Read { .. }));
        let FsAnswer::Listed { entries } = answer(&FsRequest::List {
            root: root.clone(),
            dir: String::new(),
        }) else {
            panic!("the folder was not listed");
        };
        let kind_of = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.name == name)
                .expect("listed")
                .kind
        };
        assert_eq!(kind_of("way-out"), EntryKind::Link);
        assert_eq!(kind_of("up"), EntryKind::Link);
        assert_eq!(kind_of("notes"), EntryKind::File);
        // Removing a link removes the link.
        assert_eq!(
            answer(&FsRequest::Remove {
                root: root.clone(),
                path: "notes".into(),
                with_all: false,
            }),
            FsAnswer::Done
        );
        assert!(root.join("README.md").is_file());
    }
    fs::remove_dir_all(base).expect("remove the fixture");
}

#[test]
fn a_file_is_written_only_while_it_is_what_was_read() {
    let (root, base) = a_folder("written");
    let FsAnswer::Read { sha256: was, .. } = read(&root, "README.md") else {
        panic!("the file was not read");
    };
    let FsAnswer::Written { sha256, .. } = write(
        &root,
        "README.md",
        "# Notes\n\nMine.\n",
        Some(was.clone()),
        false,
    ) else {
        panic!("it was not written");
    };
    assert_eq!(sha256, digest_of(b"# Notes\n\nMine.\n"));

    // Somebody else wrote it meanwhile: what was read is not what is there.
    let changed = write(
        &root,
        "README.md",
        "# Mine alone\n",
        Some(was.clone()),
        false,
    );
    let FsAnswer::Refused { why, now, .. } = &changed else {
        panic!("it was written over what somebody else wrote");
    };
    assert_eq!(*why, Why::Changed);
    assert_eq!(now.as_deref(), Some(sha256.as_str()));
    assert_eq!(
        fs::read_to_string(root.join("README.md")).expect("read"),
        "# Notes\n\nMine.\n"
    );
    // The person chose theirs.
    assert!(matches!(
        write(&root, "README.md", "# Mine alone\n", Some(was), true),
        FsAnswer::Written { .. }
    ));

    // A new file is new: what is there is not written over by it.
    assert!(matches!(
        write(&root, "src/lib.rs", "pub fn f() {}\n", None, false),
        FsAnswer::Written { .. }
    ));
    assert_eq!(
        refused(&write(&root, "src/lib.rs", "other", None, false)),
        Why::Exists
    );
    assert_eq!(
        refused(&write(&root, "src", "x", None, true)),
        Why::NotAFile
    );
    assert_eq!(
        refused(&write(&root, "no-such/f.txt", "x", None, false)),
        Why::NotFound
    );
    // Nothing of the writing is left beside what was written.
    assert!(fs::read_dir(&root).expect("list").flatten().all(|entry| {
        !entry
            .file_name()
            .to_string_lossy()
            .ends_with(".swem-writing")
    }));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("src/lib.rs"), fs::Permissions::from_mode(0o755))
            .expect("make it runnable");
        assert!(matches!(
            write(&root, "src/lib.rs", "changed", None, true),
            FsAnswer::Written { .. }
        ));
        let mode = fs::metadata(root.join("src/lib.rs"))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755, "what it allowed stays what it allows");
    }
    fs::remove_dir_all(base).expect("remove the fixture");
}

#[test]
fn folders_and_files_are_made_renamed_and_removed() {
    let (root, base) = a_folder("moved");
    let ask = |request: FsRequest| answer(&request);
    assert_eq!(
        ask(FsRequest::MakeDir {
            root: root.clone(),
            path: "docs".into(),
        }),
        FsAnswer::Done
    );
    assert_eq!(
        refused(&ask(FsRequest::MakeDir {
            root: root.clone(),
            path: "docs".into(),
        })),
        Why::Exists
    );
    assert_eq!(
        ask(FsRequest::Rename {
            root: root.clone(),
            from: "README.md".into(),
            to: "docs/README.md".into(),
        }),
        FsAnswer::Done
    );
    assert!(root.join("docs/README.md").is_file());
    assert_eq!(
        refused(&ask(FsRequest::Rename {
            root: root.clone(),
            from: "src/main.rs".into(),
            to: "docs/README.md".into(),
        })),
        Why::Exists
    );
    assert_eq!(
        refused(&ask(FsRequest::Rename {
            root: root.clone(),
            from: "src/main.rs".into(),
            to: "../main.rs".into(),
        })),
        Why::Outside
    );
    // A folder that holds something is not removed by mistake.
    assert_eq!(
        refused(&ask(FsRequest::Remove {
            root: root.clone(),
            path: "docs".into(),
            with_all: false,
        })),
        Why::NotEmpty
    );
    assert_eq!(
        ask(FsRequest::Remove {
            root: root.clone(),
            path: "docs".into(),
            with_all: true,
        }),
        FsAnswer::Done
    );
    assert!(!root.join("docs").exists());
    assert_eq!(
        ask(FsRequest::Remove {
            root: root.clone(),
            path: "src/main.rs".into(),
            with_all: false,
        }),
        FsAnswer::Done
    );
    fs::remove_dir_all(base).expect("remove the fixture");
}

#[test]
fn the_program_answers_one_request_on_its_streams() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let (root, base) = a_folder("program");
    let mut runner = Command::new(env!("CARGO_BIN_EXE_swem-runner"))
        .arg("fs")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the runner");
    let asked = serde_json::to_vec(&FsRequest::Read {
        root: root.clone(),
        path: "README.md".into(),
        at_most: 1024,
    })
    .expect("a request");
    runner
        .stdin
        .take()
        .expect("its input")
        .write_all(&asked)
        .expect("ask");
    let output = runner.wait_with_output().expect("its answer");
    assert!(output.status.success());
    let answered: FsAnswer = serde_json::from_slice(&output.stdout).expect("an answer");
    assert_eq!(answered, read(&root, "README.md"));
    fs::remove_dir_all(base).expect("remove the fixture");
}
