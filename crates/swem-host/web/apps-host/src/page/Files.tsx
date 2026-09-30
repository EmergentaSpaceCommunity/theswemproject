// An agent's files: the folder it works in as a tree, a file read and
// changed beside it, and what was handed over and back.
//
// What is chosen in the tree is what the other side shows: a file is
// read and edited there, a folder says what can be done in it.

import { at } from "../base.ts";
import { Dialog } from "@base-ui/react/dialog";
import { asyncDataLoaderFeature, hotkeysCoreFeature, selectionFeature } from "@headless-tree/core";
import { useTree } from "@headless-tree/react";
import { useEffect, useRef, useState } from "react";

import { sessionStore, useSession } from "../agent/store.ts";
import { Editor } from "./Editor.tsx";
import { ago, asItIs, files, folderOf, isPicture, nameOf, sized, whyNotNamed, within, type Entry, type Opened } from "./files.ts";
import { Again, Chevron, Folder } from "./icons.tsx";
import { when } from "./providers.ts";
import type { Participant } from "./types.ts";

interface Node {
  name: string;
  kind: Entry["kind"];
  byte_length: number;
  modified_ms?: number | null;
}

// What the tree knows a place by. The folder the agent works in is "/".
const ROOT = "/";
const idOf = (place: string): string => `/${place}`;
const placeOf = (id: string): string => id.slice(1);

/// What a person handed the agent, or the agent handed back.
function Handed({ area, title, about }: { area: string; title: string; about: string }) {
  const { files: handed, filesStatus, profileId } = useSession();
  const here = handed.filter((file) => file.area === area);
  return (
    <section className="k-stack w-close" data-files-area={area}>
      <span className="k-eyebrow">{title}</span>
      {here.length === 0 && filesStatus ? (
        <span className="k-caption" data-files-unread={area}>
          {filesStatus}
        </span>
      ) : here.length === 0 ? (
        <span className="k-caption" data-files-empty={area}>
          {about}
        </span>
      ) : (
        <ul className="w-handed">
          {here.map((file) => (
            <li key={file.name} data-file-name={file.name}>
              <a href={at(`/api/profiles/${encodeURIComponent(profileId)}/files/${area}/${encodeURIComponent(file.name)}`)} rel="noopener noreferrer" target="_blank">
                {file.name}
              </a>
              <span className="k-caption"> {sized(file.byte_length)}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/// A name asked for: of something new, or a new one for what is there.
function Naming({ asked, onClose, onNamed }: { asked: { title: string; about: string; name: string; does: string } | null; onClose: () => void; onNamed: (name: string) => Promise<void> }) {
  const [name, setName] = useState("");
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setName(asked?.name ?? "");
    setProblem("");
  }, [asked]);
  const done = async () => {
    const why = whyNotNamed(name);
    if (why) {
      setProblem(why);
      return;
    }
    setBusy(true);
    try {
      await onNamed(name.trim());
      onClose();
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog.Root open={asked !== null} onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">{asked?.title}</Dialog.Title>
          <Dialog.Description className="k-caption">{asked?.about}</Dialog.Description>
          <form
            className="k-stack"
            onSubmit={(event) => {
              event.preventDefault();
              void done();
            }}
          >
            <label className="k-stack w-close">
              <span className="k-caption">Name</span>
              <input className="k-field k-mono" value={name} spellCheck={false} autoFocus onChange={(event) => setName(event.target.value)} />
            </label>
            {problem ? <div className="k-notice k-danger">{problem}</div> : null}
            <div className="k-inline w-end">
              <Dialog.Close className="k-btn k-quiet" type="button">
                Not now
              </Dialog.Close>
              <button type="submit" className="k-btn k-primary" disabled={busy}>
                {asked?.does}
              </button>
            </div>
          </form>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/// Something a person is asked before it is done.
function Asking({ asked, onClose }: { asked: { title: string; about: string; choices: { says: string; primary?: boolean; does: () => Promise<void> | void }[] } | null; onClose: () => void }) {
  const [problem, setProblem] = useState("");
  useEffect(() => setProblem(""), [asked]);
  return (
    <Dialog.Root open={asked !== null} onOpenChange={(next) => (next ? undefined : onClose())}>
      <Dialog.Portal>
        <Dialog.Backdrop className="w-scrim" />
        <Dialog.Popup className="k-dialog w-dialog">
          <Dialog.Title className="k-heading">{asked?.title}</Dialog.Title>
          <Dialog.Description className="k-caption">{asked?.about}</Dialog.Description>
          {problem ? <div className="k-notice k-danger">{problem}</div> : null}
          <div className="k-inline w-end">
            <Dialog.Close className="k-btn k-quiet" type="button">
              Not now
            </Dialog.Close>
            {(asked?.choices ?? []).map((choice) => (
              <button
                type="button"
                className={`k-btn${choice.primary ? " k-primary" : ""}`}
                key={choice.says}
                onClick={() => {
                  Promise.resolve(choice.does())
                    .then(onClose)
                    .catch((error: Error) => setProblem(error.message));
                }}
              >
                {choice.says}
              </button>
            ))}
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

type Chosen = { place: string; kind: "file" } | { place: string; kind: "folder" } | null;

export function Files({ agent, hidden }: { agent: Participant; hidden: boolean }) {
  const profile = agent.profile_id ?? "";
  const [chosen, setChosen] = useState<Chosen>(null);
  const [opened, setOpened] = useState<Opened | null>(null);
  const [text, setText] = useState("");
  const [problem, setProblem] = useState("");
  const [said, setSaid] = useState("");
  const [changedSince, setChangedSince] = useState("");
  const [saving, setSaving] = useState(false);
  const [editing, setEditing] = useState(false);
  const [changedAt, setChangedAt] = useState<number | null>(null);
  const { profiles } = useSession();
  const inAContainer = profiles.find((one) => one.profile_id === profile)?.environment_profile_id === "podman-container-environment";
  const [naming, setNaming] = useState<Parameters<typeof Naming>[0]["asked"]>(null);
  const [named, setNamed] = useState<(name: string) => Promise<void>>(() => async () => {});
  const [asking, setAsking] = useState<Parameters<typeof Asking>[0]["asked"]>(null);
  const changed = opened !== null && opened.text != null && text !== opened.text;
  // What is written is read where a callback made earlier looks for it.
  // What waits for a person to answer a question before it goes on.
  const answeredRef = useRef<(() => void) | null>(null);
  const now = useRef({ opened, text, changed });
  now.current = { opened, text, changed };

  const tree = useTree<Node>({
    rootItemId: ROOT,
    initialState: { expandedItems: [ROOT] },
    getItemName: (item) => item.getItemData()?.name ?? "",
    isItemFolder: (item) => item.getItemData()?.kind === "folder",
    createLoadingItemData: () => ({ name: "…", kind: "other", byte_length: 0 }),
    dataLoader: {
      getItem: async (id) => {
        if (id === ROOT) return { name: "", kind: "folder", byte_length: 0 };
        const place = placeOf(id);
        const found = (await files.list(profile, folderOf(place))).find((entry) => entry.name === nameOf(place));
        return found ?? { name: nameOf(place), kind: "other", byte_length: 0 };
      },
      getChildrenWithData: async (id) => (await files.list(profile, placeOf(id))).map((entry) => ({ id: idOf(within(placeOf(id), entry.name)), data: entry })),
    },
    onPrimaryAction: (item) => choose(placeOf(item.getId()), item.isFolder() ? "folder" : item.getItemData()?.kind === "file" ? "file" : null),
    features: [asyncDataLoaderFeature, selectionFeature, hotkeysCoreFeature],
  });

  /// Read again what the open folders hold, without the tree blinking.
  const readAgain = (folders?: string[]) => {
    const ids = folders?.map(idOf) ?? tree.getItems().filter((item) => item.isFolder() && item.isExpanded()).map((item) => item.getId());
    for (const id of new Set([ROOT, ...ids])) {
      try {
        void tree.getItemInstance(id).invalidateChildrenIds(true);
      } catch {
        // A folder that is gone has nothing to read again.
      }
    }
  };
  useEffect(() => {
    if (hidden || !profile) return undefined;
    void sessionStore.loadFiles(profile);
    readAgain();
    // An agent writes while a person looks.
    const again = window.setInterval(() => readAgain(), 6000);
    return () => window.clearInterval(again);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hidden, profile]);

  const open = async (place: string) => {
    setProblem("");
    setSaid("");
    setChangedSince("");
    setChosen({ place, kind: "file" });
    setOpened(null);
    setEditing(false);
    try {
      const [file, beside] = await Promise.all([files.open(profile, place), files.list(profile, folderOf(place)).catch(() => [])]);
      setOpened(file);
      setText(file.text ?? "");
      setChangedAt(beside.find((entry) => entry.name === nameOf(place))?.modified_ms ?? null);
    } catch (error) {
      setProblem((error as Error).message);
    }
  };

  const save = async (over = false) => {
    const { opened: file, text: written } = now.current;
    if (!file || file.text == null) return;
    setSaving(true);
    setProblem("");
    try {
      const answer = await files.save(profile, file.path, written, file.sha256, over);
      if (answer.saved) {
        setOpened({ ...file, text: written, sha256: answer.sha256 ?? file.sha256, byte_length: new TextEncoder().encode(written).length });
        setChangedSince("");
        setChangedAt(Date.now());
        setSaid(`Saved at ${when(Date.now())}.`);
        readAgain([folderOf(file.path)]);
      } else {
        setChangedSince(answer.sha256 ? "It changed since you opened it" : "It is gone since you opened it");
      }
    } catch (error) {
      setProblem((error as Error).message);
    } finally {
      setSaving(false);
    }
  };

  /// Choose what the other side shows. What was written and not saved is
  /// not left behind without a word.
  function choose(place: string, kind: "file" | "folder" | null) {
    if (kind === null) return;
    const go = () => (kind === "file" ? void open(place) : (setChosen({ place, kind }), setOpened(null), setProblem(""), setSaid(""), setChangedSince("")));
    const { opened: file, changed: unsaved } = now.current;
    if (!unsaved || !file || (kind === "file" && file.path === place)) {
      if (!(kind === "file" && file?.path === place)) go();
      return;
    }
    setAsking({
      title: `${nameOf(file.path)} was changed and not saved`,
      about: "What you wrote is lost when you leave it.",
      choices: [
        { says: "Leave it", does: go },
        {
          says: "Save, then go",
          primary: true,
          does: async () => {
            await save();
            go();
          },
        },
      ],
    });
  }

  const makeIn = (folder: string, what: "file" | "folder") => {
    setNamed(() => async (name: string) => {
      const place = within(folder, name);
      if (what === "folder") await files.makeFolder(profile, place);
      else {
        const made = await files.save(profile, place, "", null);
        if (!made.saved) throw new Error(made.said ?? "It could not be made.");
      }
      readAgain([folder]);
      try {
        tree.getItemInstance(idOf(folder)).expand();
      } catch {
        // The folder it was made in is not shown.
      }
      if (what === "file") void open(place).then(() => setEditing(true));
      else setChosen({ place, kind: "folder" });
    });
    setNaming({
      title: what === "file" ? "A new file" : "A new folder",
      about: folder === "" ? "In the folder it works in." : `In ${folder}.`,
      name: "",
      does: "Make it",
    });
  };

  /// Bring files from the person's own machine into a folder. One that
  /// is there under the same name is replaced only when the person says.
  const bringInto = async (folder: string, brought: File[]) => {
    setProblem("");
    for (const file of brought) {
      const place = within(folder, file.name);
      try {
        await files.bring(profile, place, file);
      } catch (error) {
        if (!(error as Error).message.includes("is there already")) {
          setProblem((error as Error).message);
          continue;
        }
        await new Promise<void>((answered) => {
          setAsking({
            title: `${file.name} is there already`,
            about: folder === "" ? "In the folder it works in." : `In ${folder}.`,
            choices: [
              {
                says: "Replace it",
                primary: true,
                does: async () => {
                  await files.bring(profile, place, file, true);
                },
              },
            ],
          });
          answeredRef.current = answered;
        });
      }
    }
    readAgain([folder]);
    try {
      tree.getItemInstance(idOf(folder)).expand();
    } catch {
      // The folder is not shown.
    }
  };
  const bringing = useRef<HTMLInputElement | null>(null);
  const bringTo = useRef("");
  const bring = (folder: string) => {
    bringTo.current = folder;
    bringing.current?.click();
  };

  const rename = (place: string) => {
    setNamed(() => async (name: string) => {
      const to = within(folderOf(place), name);
      if (to === place) return;
      await files.rename(profile, place, to);
      readAgain([folderOf(place)]);
      if (chosen?.kind === "file") void open(to);
      else setChosen({ place: to, kind: "folder" });
    });
    setNaming({ title: `Rename ${nameOf(place)}`, about: "It stays in the folder it is in.", name: nameOf(place), does: "Rename" });
  };

  const remove = (place: string, kind: "file" | "folder") => {
    setAsking({
      title: `Remove ${nameOf(place)}?`,
      about: kind === "folder" ? "The folder and everything in it is removed from the agent's machine. It cannot be taken back." : "It is removed from the agent's machine. It cannot be taken back.",
      choices: [
        {
          says: "Remove",
          primary: true,
          does: async () => {
            await files.remove(profile, place, kind === "folder");
            readAgain([folderOf(place)]);
            setChosen(null);
            setOpened(null);
          },
        },
      ],
    });
  };

  const folderChosen = chosen?.kind === "folder" ? chosen.place : chosen ? folderOf(chosen.place) : "";

  return (
    <section className="w-files" data-agent-panel="files" hidden={hidden} aria-label={`Files of ${agent.name}`}>
      <Naming asked={naming} onClose={() => setNaming(null)} onNamed={named} />
      <Asking
        asked={asking}
        onClose={() => {
          setAsking(null);
          answeredRef.current?.();
          answeredRef.current = null;
        }}
      />
      <input
        type="file"
        multiple
        hidden
        ref={bringing}
        aria-label="Files to bring"
        onChange={(event) => {
          const brought = [...(event.target.files ?? [])];
          event.target.value = "";
          if (brought.length > 0) void bringInto(bringTo.current, brought);
        }}
      />
      <aside className="w-tree">
        <div className="k-spread w-tree-head">
          <span className="k-inline w-tight w-nowrap">
            <button type="button" className="k-btn" onClick={() => bring(folderChosen)}>
              Upload
            </button>
            <button type="button" className="k-btn" onClick={() => makeIn(folderChosen, "file")}>
              New file
            </button>
          </span>
          <span className="k-inline w-tight w-nowrap">
            <button type="button" className="k-btn k-quiet" title="A new folder" aria-label="New folder" onClick={() => makeIn(folderChosen, "folder")}>
              <Folder size={15} />
            </button>
            <button
              type="button"
              id="files-refresh"
              className="k-btn k-quiet"
              title="Read again"
              aria-label="Read again"
              onClick={() => {
                readAgain();
                void sessionStore.loadFiles(profile);
              }}
            >
              <Again size={15} />
            </button>
          </span>
        </div>
        <div className="k-stack w-tree-handed">
          <Handed area="inbox" title="From you" about="What you attach in a chat lands here." />
          <Handed area="outbox" title={`From ${agent.name}`} about="What it writes into its outbox is here to open." />
        </div>
        <div {...tree.getContainerProps(`Files of ${agent.name}`)} className="w-tree-items">
          {tree.getItems().map((item) => {
            const node = item.getItemData();
            const place = placeOf(item.getId());
            const shown = chosen?.place === place;
            return (
              <button
                {...item.getProps()}
                type="button"
                key={item.getId()}
                className={`w-tree-item${shown ? " k-active" : ""}${node?.kind === "link" || node?.kind === "other" ? " w-faint" : ""}`}
                style={{ paddingLeft: `${8 + item.getItemMeta().level * 14}px` }}
                title={node?.kind === "link" ? "A link that leads out of its folder" : place}
              >
                <span className={`w-twist${item.isFolder() && item.isExpanded() ? " w-open" : ""}`}>{item.isFolder() ? <Chevron size={13} /> : null}</span>
                <span className="w-tree-name">{item.getItemName()}</span>
              </button>
            );
          })}
          {tree.getItems().length === 0 ? <span className="k-caption w-tree-empty">Nothing is in it yet.</span> : null}
        </div>
        <div className="k-caption w-tree-foot">
          {agent.name}'s workspace, {inAContainer ? "shared with its container on this machine" : "on this machine"}.
        </div>
      </aside>
      <div className="w-editor">
        {chosen === null ? (
          <div className="w-editor-none k-caption">Choose a file to read it.</div>
        ) : chosen.kind === "folder" ? (
          <div className="w-editor-none">
            <div className="k-card k-stack">
              <div className="k-inline w-nowrap">
                <Folder size={18} />
                <span className="k-title k-mono">{chosen.place}</span>
              </div>
              <div className="k-inline w-tight">
                <button type="button" className="k-btn" onClick={() => makeIn(chosen.place, "file")}>
                  New file here
                </button>
                <button type="button" className="k-btn" onClick={() => makeIn(chosen.place, "folder")}>
                  New folder here
                </button>
                <button type="button" className="k-btn" onClick={() => bring(chosen.place)}>
                  Bring files here
                </button>
                <button type="button" className="k-btn k-quiet" onClick={() => rename(chosen.place)}>
                  Rename
                </button>
                <button type="button" className="k-btn k-quiet k-is-danger" onClick={() => remove(chosen.place, "folder")}>
                  Remove
                </button>
              </div>
            </div>
          </div>
        ) : (
          <>
            <header className="w-editor-head">
              <span className="k-two">
                <span className="k-name k-mono">
                  {chosen.place.split("/").join(" / ")}
                  {changed ? <span className="k-caption"> · changed</span> : null}
                </span>
                <span className="k-caption" role="status">
                  {opened ? [changedAt ? `Changed ${ago(changedAt)}` : "", sized(opened.byte_length), said].filter(Boolean).join(" · ") : problem ? "" : "Reading…"}
                </span>
              </span>
              <span className="k-inline w-tight w-nowrap">
                <button type="button" className="k-btn k-quiet" onClick={() => rename(chosen.place)}>
                  Rename
                </button>
                <button type="button" className="k-btn k-quiet k-is-danger" onClick={() => remove(chosen.place, "file")}>
                  Remove
                </button>
                <a className="k-btn" href={asItIs(profile, chosen.place)} download={nameOf(chosen.place)}>
                  Download
                </a>
                {opened?.text == null ? (
                  <a className="k-btn" href={asItIs(profile, chosen.place)} target="_blank" rel="noopener noreferrer">
                    Open as it is
                  </a>
                ) : editing ? (
                  <button type="button" className="k-btn k-primary" disabled={!changed || saving} onClick={() => void save()}>
                    {saving ? "Saving…" : "Save"}
                  </button>
                ) : (
                  <button type="button" className="k-btn k-primary" onClick={() => setEditing(true)}>
                    Edit
                  </button>
                )}
              </span>
            </header>
            {problem ? <div className="k-notice k-danger w-editor-says">{problem}</div> : null}
            {changedSince ? (
              <div className="k-notice k-warning w-editor-says k-spread">
                <span>{changedSince}. Nothing was saved.</span>
                <span className="k-inline w-tight w-nowrap">
                  <button type="button" className="k-btn k-quiet" onClick={() => void open(chosen.place)}>
                    Read it again
                  </button>
                  <button type="button" className="k-btn" onClick={() => void save(true)}>
                    Save mine over it
                  </button>
                </span>
              </div>
            ) : null}
            {opened === null ? null : opened.text == null ? (
              isPicture(opened.path) ? (
                <div className="w-editor-none w-scroll">
                  <img className="w-picture" src={asItIs(profile, opened.path)} alt={nameOf(opened.path)} />
                </div>
              ) : (
                <div className="w-editor-none k-caption">This is not text. Open it as it is to look at it or keep it.</div>
              )
            ) : (
              <Editor place={opened.path} text={opened.text} editing={editing} onChange={setText} onSave={() => void save()} key={opened.path} />
            )}
          </>
        )}
      </div>
    </section>
  );
}
