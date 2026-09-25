// The project's assets: the bytes it is made of, with the names a person
// gave them.
//
// Nothing here is a new kind of thing. The bytes are the typed artifacts the
// ontology has always held, their type is declared by the module that owns
// the media type, and binding one to a place in a model is that domain's own
// command. What was missing was a name a person chose and somewhere to see
// them - so a project's assets read as digests and could not be found again,
// played, kept or reused in a second slot.

import { useRef, useState } from "react";

import { callProjectTool, materializeArtifact } from "./api";
import type { AssetView, AssetsView, ContentDescriptor } from "./types";

interface Props {
  serverName: string;
  assets: AssetsView;
  onChanged: () => void;
}

/// The media type of a file the browser did not name, from what it is called.
const BY_EXTENSION: Record<string, string> = {
  wav: "audio/wav",
  txt: "text/plain;charset=utf-8",
  json: "application/json",
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
};

export function mediaTypeOf(file: { name: string; type?: string }): string {
  if (file.type) return file.type === "audio/x-wav" ? "audio/wav" : file.type;
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  return BY_EXTENSION[extension] ?? "application/octet-stream";
}

/// Standard base64 of exact bytes, in chunks so a long file does not blow the
/// argument list of `String.fromCharCode`.
export function toBase64(bytes: ArrayBuffer): string {
  const view = new Uint8Array(bytes);
  let binary = "";
  const chunk = 0x8000;
  for (let index = 0; index < view.length; index += chunk) {
    binary += String.fromCharCode(...view.subarray(index, index + chunk));
  }
  return btoa(binary);
}

/// One asset: what it is, what it is called, and what a person does with it.
function AssetRow({ serverName, asset, onChanged }: { serverName: string; asset: AssetView; onChanged: () => void }) {
  const [descriptor, setDescriptor] = useState<ContentDescriptor | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const name = useRef<HTMLInputElement>(null);
  const open = async () => {
    setBusy(true);
    setProblem("");
    try {
      const result = await materializeArtifact<ContentDescriptor>(serverName, asset.digest);
      setDescriptor(result.descriptor);
    } catch (failure) {
      setProblem(String(failure));
    } finally {
      setBusy(false);
    }
  };
  const rename = async () => {
    // What the person typed goes to the project, empty included. The rule for
    // a name lives there ("a name is 1-80 characters") and the refusal is in
    // words; a page that swallowed the empty case here did neither - Save
    // cleared nothing, said nothing, and left the field open.
    const wanted = name.current?.value.trim() ?? "";
    setBusy(true);
    setProblem("");
    try {
      await callProjectTool(serverName, "name_asset", { digest: asset.digest, name: wanted });
      setRenaming(false);
      onChanged();
    } catch (failure) {
      setProblem((failure as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const served = descriptor ? `/api/content/${descriptor.descriptor_id}` : null;
  const usedBy = asset.used_by ?? [];
  return (
    <li className="asset-row" data-digest={asset.digest} data-name={asset.name ?? ""} data-media-type={asset.media_type}>
      <div className="asset-head">
        <strong className="asset-name">{asset.name ?? `unnamed · ${asset.digest.slice(0, 12)}`}</strong>
        <span className="k-chip">{asset.produced ? "produced" : "brought in"}</span>
        <span className="k-caption k-muted">
          {asset.media_type} · {asset.byte_length} bytes
          {usedBy.length > 0 ? ` · used in ${usedBy.length} record${usedBy.length === 1 ? "" : "s"}` : " · not used yet"}
        </span>
      </div>
      <div className="asset-actions">
        {served === null ? (
          <button className="asset-open" data-digest={asset.digest} disabled={busy} onClick={() => void open()}>
            Open
          </button>
        ) : (
          <>
            <a className="asset-download" data-digest={asset.digest} href={`${served}?download=1`} download={descriptor?.name}>
              Download
            </a>
            {asset.media_type.startsWith("audio/") ? (
              <audio className="asset-audio" data-digest={asset.digest} controls src={served} />
            ) : null}
            {asset.media_type.startsWith("image/") ? (
              <img className="asset-image" data-digest={asset.digest} src={served} alt={asset.name ?? "asset"} />
            ) : null}
          </>
        )}
        {renaming ? (
          <>
            <input className="asset-name-input" data-digest={asset.digest} defaultValue={asset.name ?? ""} placeholder="what you call it" ref={name} />
            <button className="asset-name-save" data-digest={asset.digest} disabled={busy} onClick={() => void rename()}>
              Save
            </button>
          </>
        ) : (
          <button className="asset-rename" data-digest={asset.digest} onClick={() => setRenaming(true)}>
            {asset.name ? "Rename" : "Name it"}
          </button>
        )}
      </div>
      {problem ? <span className="bad asset-problem">{problem}</span> : null}
    </li>
  );
}

export function Assets({ serverName, assets, onChanged }: Props) {
  // One line per file that did not come in. A person who chooses several at
  // once is told about every one of them: a single slot held the last
  // refusal only, so the first file was refused in silence and its absence
  // from the list was the only sign.
  const [refusals, setRefusals] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const file = useRef<HTMLInputElement>(null);
  const bringIn = async (chosen: FileList | null) => {
    if (!chosen || chosen.length === 0) return;
    setBusy(true);
    setRefusals([]);
    const refused: string[] = [];
    let landed = 0;
    try {
      for (const one of Array.from(chosen)) {
        // A file's refusal is about that file. The rest of the choice still
        // comes in, and what came in before it is still shown: a throw out of
        // this loop used to abandon both, so a person saw one complaint and
        // an empty space where their earlier files already were.
        try {
          const bytes = await one.arrayBuffer();
          if (bytes.byteLength > 4 * 1024 * 1024) {
            refused.push(
              `${one.name} is ${Math.ceil(bytes.byteLength / (1024 * 1024))} MiB, more than one message carries; put it in the project workspace and bring it in from there`,
            );
            continue;
          }
          const stored = (await callProjectTool(serverName, "put_binary_asset", {
            media_type: mediaTypeOf(one),
            base64: toBase64(bytes),
          })) as { structuredContent?: { artifact?: { content_digest?: string } } };
          const digest = stored?.structuredContent?.artifact?.content_digest;
          // The file's own name is the first name it has here; a person renames
          // it from the row.
          if (digest) {
            await callProjectTool(serverName, "name_asset", {
              digest,
              name: one.name.replace(/\.[^.]+$/, ""),
            }).catch(() => undefined);
          }
          landed += 1;
        } catch (failure) {
          refused.push(`${one.name}: ${(failure as Error).message}`);
        }
      }
    } finally {
      if (landed > 0) onChanged();
      setRefusals(refused);
      setBusy(false);
      if (file.current) file.current.value = "";
    }
  };
  return (
    <section id="project-assets" aria-label="Assets">
      <h3>Assets</h3>
      {assets.assets.length === 0 ? (
        <p id="assets-empty" className="k-muted">
          Nothing yet. Bring a file in and it belongs to the project: any slot can use it, and it
          keeps the name you give it.
        </p>
      ) : null}
      <ul className="asset-list">
        {assets.assets.map((asset) => (
          <AssetRow key={asset.digest} serverName={serverName} asset={asset} onChanged={onChanged} />
        ))}
      </ul>
      <label className="bring-asset">
        <span className="k-caption">Bring a file into this project</span>
        <input id="bring-asset" type="file" multiple ref={file} disabled={busy} onChange={(event) => void bringIn(event.target.files)} />
      </label>
      <div id="assets-status" className="k-caption">
        {busy ? (
          "Bringing it in…"
        ) : refusals.length > 0 ? (
          <ul className="asset-refusals">
            {refusals.map((refusal) => (
              <li key={refusal} className="asset-refusal">
                {refusal}
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    </section>
  );
}
