/// The Cycle's own sentences, as a person reads them.
///
/// The kernel writes these rows, and it writes them out of records: a ref, a
/// predicate contract, a logical id. None of that is wrong to record and all
/// of it is wrong to show, so this is where the shape a person did not write
/// comes off. Nothing here interprets a domain - a module's word is kept and
/// only its packaging is dropped.
///
/// They live in their own module, rather than beside the rail that renders
/// them, so that a node suite can ask them what a person would read: a .tsx
/// file is not importable from a test that runs without a bundler.

/// One sentence of the Cycle: a record ref in it becomes "this revision", a
/// predicate contract ref becomes the words the module named it with, and
/// everything else stays. The exact sentence remains the title of the line,
/// so nothing is hidden - only moved out of the way.
export function readable(sentence: string): string {
  return sentence
    .replace(/`[0-9a-f]{12,64}`/g, "this revision")
    .replace(/\b[a-z][a-z0-9.]*\.predicate:([a-z0-9-]+)@[0-9][0-9.]*/gi, (_whole, name: string) =>
      `"${name.replace(/-+/g, " ")}"`,
    );
}

/// One Cycle sentence split into the parts a person reads plainly and the
/// parts it marked as names.
///
/// The kernel writes a logical id in backticks - "the delivery of `music`
/// was refused" - which is how it says "this word is a thing, not prose".
/// Printed as text the marks show, and a person who never wrote a backtick
/// reads punctuation the sentence does not have. So the marks come off and
/// what they marked is rendered as a name.
///
/// A lone backtick is not a mark: it stays in the text, because a sentence
/// with one is a sentence that meant it.
export function marked(sentence: string): { text: string; name: boolean }[] {
  const parts: { text: string; name: boolean }[] = [];
  let rest = sentence;
  for (;;) {
    const open = rest.indexOf("`");
    const close = open < 0 ? -1 : rest.indexOf("`", open + 1);
    if (open < 0 || close < 0) break;
    if (open > 0) parts.push({ text: rest.slice(0, open), name: false });
    parts.push({ text: rest.slice(open + 1, close), name: true });
    rest = rest.slice(close + 1);
  }
  if (rest.length > 0) parts.push({ text: rest, name: false });
  return parts;
}

/// A domain's own name for one of its outputs, as a person reads it:
/// `swem.output:audio-master` is "audio master". The module's word is kept -
/// nothing here knows what a master is - and only its shape is dropped, which
/// is the part a person did not write and cannot use.
export function outputName(root: string): string {
  const local = root.includes(":") ? root.slice(root.lastIndexOf(":") + 1) : root;
  return local.replace(/[-_]+/g, " ").trim() || root;
}

/// The same sentence where markup cannot be drawn: a `title` attribute, which
/// a browser renders as plain text and never as markup. `marked` exists for
/// the places that can set a name as code; this is for the places that cannot,
/// and printing the backticks there shows a person punctuation that meant
/// "this word is a thing" and now means nothing.
export function unmarked(sentence: string): string {
  return marked(sentence)
    .map((part) => part.text)
    .join("");
}
