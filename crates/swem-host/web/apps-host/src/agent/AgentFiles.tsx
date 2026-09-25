// What a person handed the agent, and what the agent handed back.
//
// Two directories inside the agent's own workspace, shown as two lists. A
// file attached in the composer lands in the inbox; anything the agent writes
// into the outbox appears here, with no resource link, no tool and no
// capability - which is the point, because a directory is the one interface
// every agent already has.

import { sessionStore, useSession } from "./store.ts";

function bytesWords(count: number): string {
  if (count < 1024) return `${count} B`;
  if (count < 1024 * 1024) return `${Math.round(count / 1024)} KB`;
  return `${(count / (1024 * 1024)).toFixed(1)} MB`;
}

function Area({ area, blurb }: { area: string; blurb: string }) {
  const { files, filesStatus, profileId } = useSession();
  const here = files.filter((file) => file.area === area);
  return (
    <section className="files-area" data-files-area={area}>
      <div className="k-eyebrow">{area}</div>
      <p className="k-muted">{blurb}</p>
      {here.length === 0 && filesStatus ? (
        <p className="k-caption" data-files-unread={area}>
          {filesStatus}
        </p>
      ) : here.length === 0 ? (
        <p className="k-caption" data-files-empty={area}>
          Nothing here yet.
        </p>
      ) : (
        <ul className="files-list">
          {here.map((file) => (
            <li key={file.name} data-file-name={file.name}>
              <a
                href={`/api/profiles/${encodeURIComponent(profileId)}/files/${area}/${encodeURIComponent(file.name)}`}
                rel="noopener noreferrer"
                target="_blank"
              >
                {file.name}
              </a>
              <span className="k-caption"> {bytesWords(file.byte_length)}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function AgentFiles({ hidden }: { hidden: boolean }) {
  return (
    <div className="panel files-panel" data-agent-panel="files" hidden={hidden}>
      <header className="panel-header">
        <strong>Files</strong>
        <button id="files-refresh" onClick={() => void sessionStore.loadFiles()}>
          Refresh
        </button>
      </header>
      <Area area="inbox" blurb="What you hand over. Attaching a file in the conversation puts it here, and the turn tells the agent where it is." />
      <Area area="outbox" blurb="What the agent hands back. Anything it writes here shows up for you to open." />
    </div>
  );
}
