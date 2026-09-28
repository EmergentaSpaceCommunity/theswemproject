// Names in what was written: who a message is for.

/// A name after `@`, where a name can stand: at the beginning or after
/// something that is not part of a word or an address.
const NAMED = /(^|[^\w@./-])@([a-z0-9][a-z0-9_-]{0,31})/g;

/// The pieces of a text with the names in it set apart. Pure, so it is
/// tested without a page.
export function withNames(text: string, handles: string[]): (string | { name: string })[] {
  const pieces: (string | { name: string })[] = [];
  let from = 0;
  for (const found of text.matchAll(NAMED)) {
    const handle = found[2] ?? "";
    if (!handles.includes(handle)) continue;
    const at = (found.index ?? 0) + (found[1] ?? "").length;
    if (at > from) pieces.push(text.slice(from, at));
    pieces.push({ name: handle });
    from = at + handle.length + 1;
  }
  if (from < text.length) pieces.push(text.slice(from));
  return pieces;
}
