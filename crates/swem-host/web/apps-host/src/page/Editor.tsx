// A file's text, edited. CodeMirror does the editing; what it looks like is
// the kit's palette, so it follows the theme like everything else.

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { yaml } from "@codemirror/lang-yaml";
import { HighlightStyle, bracketMatching, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { useEffect, useRef } from "react";

import { languageOf } from "./files.ts";

function written(place: string): Extension {
  switch (languageOf(place)) {
    case "javascript":
      return javascript();
    case "jsx":
      return javascript({ jsx: true });
    case "typescript":
      return javascript({ typescript: true });
    case "tsx":
      return javascript({ typescript: true, jsx: true });
    case "json":
      return json();
    case "markdown":
      return markdown();
    case "python":
      return python();
    case "rust":
      return rust();
    case "html":
      return html();
    case "css":
      return css();
    case "yaml":
      return yaml();
    default:
      return [];
  }
}

const look = EditorView.theme({
  "&": { height: "100%", color: "var(--color-text-primary)", backgroundColor: "transparent", fontSize: "var(--font-text-sm-size)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "var(--font-text-md-line-height)" },
  ".cm-content": { caretColor: "var(--color-text-primary)", padding: "10px 0" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--color-text-primary)" },
  ".cm-gutters": { backgroundColor: "transparent", color: "var(--color-text-tertiary)", border: "none" },
  ".cm-activeLine": { backgroundColor: "var(--color-background-ghost)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--color-text-secondary)" },
  ".cm-selectionBackground, &.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": { backgroundColor: "var(--color-background-tertiary)" },
  ".cm-selectionMatch": { backgroundColor: "var(--color-background-tertiary)" },
  ".cm-matchingBracket, &.cm-focused .cm-matchingBracket": { backgroundColor: "transparent", outline: "var(--border-width-regular) solid var(--color-border-info)" },
  ".cm-searchMatch": { backgroundColor: "transparent", outline: "var(--border-width-regular) solid var(--color-border-warning)" },
  ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--color-background-tertiary)" },
  ".cm-panels": { backgroundColor: "var(--color-background-secondary)", color: "var(--color-text-primary)" },
  ".cm-panels.cm-panels-top": { borderBottom: "var(--border-width-regular) solid var(--color-border-secondary)" },
  ".cm-panel.cm-search": { padding: "8px 12px", display: "flex", flexWrap: "wrap", alignItems: "center", gap: "6px" },
  ".cm-panel.cm-search label": { display: "inline-flex", alignItems: "center", gap: "4px", fontSize: "var(--font-text-xs-size)" },
  ".cm-textfield": {
    backgroundColor: "var(--color-background-primary)",
    color: "var(--color-text-primary)",
    border: "var(--border-width-regular) solid var(--color-border-secondary)",
    borderRadius: "var(--border-radius-sm)",
    fontSize: "var(--font-text-sm-size)",
  },
  ".cm-button": {
    backgroundImage: "none",
    backgroundColor: "var(--color-background-ghost)",
    color: "var(--color-text-primary)",
    border: "var(--border-width-regular) solid var(--color-border-secondary)",
    borderRadius: "var(--border-radius-sm)",
    fontSize: "var(--font-text-xs-size)",
  },
  ".cm-panel.cm-search [name=close]": { color: "var(--color-text-secondary)", cursor: "pointer" },
});

// The same colours code has in a chat.
const colours = HighlightStyle.define([
  { tag: [tags.keyword, tags.modifier, tags.operatorKeyword, tags.controlKeyword, tags.definitionKeyword, tags.moduleKeyword], color: "var(--color-text-danger)" },
  { tag: [tags.string, tags.special(tags.string), tags.regexp, tags.inserted], color: "var(--color-text-success)" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment, tags.docComment, tags.meta], color: "var(--color-text-tertiary)" },
  { tag: [tags.number, tags.bool, tags.null, tags.atom, tags.constant(tags.variableName)], color: "var(--color-text-info)" },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName), tags.macroName], color: "var(--color-text-info)" },
  { tag: [tags.typeName, tags.className, tags.namespace, tags.tagName], color: "var(--color-text-warning)" },
  { tag: [tags.punctuation, tags.bracket, tags.operator], color: "var(--color-text-secondary)" },
  { tag: [tags.link, tags.url], color: "var(--color-text-info)", textDecoration: "underline" },
  { tag: tags.heading, fontWeight: "var(--font-weight-semibold)" },
  { tag: tags.strong, fontWeight: "var(--font-weight-semibold)" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.deleted, color: "var(--color-text-danger)" },
]);

/// The editor of one file as it was opened. A file opened anew is a new
/// editor: the parent gives it a key.
export function Editor({ place, text, onChange, onSave }: { place: string; text: string; onChange: (text: string) => void; onSave: () => void }) {
  const at = useRef<HTMLDivElement | null>(null);
  const changed = useRef(onChange);
  const saved = useRef(onSave);
  changed.current = onChange;
  saved.current = onSave;
  useEffect(() => {
    if (!at.current) return undefined;
    const view = new EditorView({
      parent: at.current,
      state: EditorState.create({
        doc: text,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          highlightActiveLine(),
          history(),
          drawSelection(),
          indentOnInput(),
          bracketMatching(),
          highlightSelectionMatches(),
          search({ top: true }),
          keymap.of([
            {
              key: "Mod-s",
              preventDefault: true,
              run: () => {
                saved.current();
                return true;
              },
            },
            indentWithTab,
            ...defaultKeymap,
            ...historyKeymap,
            ...searchKeymap,
          ]),
          written(place),
          syntaxHighlighting(colours),
          look,
          EditorView.contentAttributes.of({ "aria-label": `What ${place} says`, spellcheck: "false" }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) changed.current(update.state.doc.toString());
          }),
        ],
      }),
    });
    return () => view.destroy();
    // The text is what the file said when it was opened; what is written
    // afterwards lives in the editor.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [place]);
  return <div className="w-editor-body" ref={at} />;
}
