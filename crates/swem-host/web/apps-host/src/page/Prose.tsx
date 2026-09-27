// What an agent wrote, read as it meant it: paragraphs, lists, tables and
// code, with the code in the colours of what it is written in.
//
// While the agent is still writing, what it has written is often cut in the
// middle of a mark; it is read as if the mark were closed, so the page does
// not flicker between stars and bold. Code is coloured once it is whole.

import { memo, useEffect, useState, type ReactElement, type ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import remend from "remend";
import { createCssVariablesTheme, createHighlighterCore, type HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import bash from "shiki/langs/bash.mjs";
import css from "shiki/langs/css.mjs";
import diff from "shiki/langs/diff.mjs";
import go from "shiki/langs/go.mjs";
import html from "shiki/langs/html.mjs";
import java from "shiki/langs/java.mjs";
import javascript from "shiki/langs/javascript.mjs";
import json from "shiki/langs/json.mjs";
import jsx from "shiki/langs/jsx.mjs";
import markdown from "shiki/langs/markdown.mjs";
import python from "shiki/langs/python.mjs";
import rust from "shiki/langs/rust.mjs";
import sql from "shiki/langs/sql.mjs";
import toml from "shiki/langs/toml.mjs";
import tsx from "shiki/langs/tsx.mjs";
import typescript from "shiki/langs/typescript.mjs";
import yaml from "shiki/langs/yaml.mjs";

import { Check, Copy } from "./icons.tsx";

const THEME = "swem";
const theme = createCssVariablesTheme({ name: THEME, variablePrefix: "--k-code-", fontStyle: true });

let ready: Promise<HighlighterCore> | null = null;
const highlighter = (): Promise<HighlighterCore> =>
  (ready ??= createHighlighterCore({
    themes: [theme],
    langs: [bash, css, diff, go, html, java, javascript, json, jsx, markdown, python, rust, sql, toml, tsx, typescript, yaml],
    engine: createJavaScriptRegexEngine({ forgiving: true }),
  }));

const coloured = new Map<string, string>();

function Code({ code, language }: { code: string; language: string }) {
  const key = `${language}\n${code}`;
  const [markup, setMarkup] = useState<string | null>(coloured.get(key) ?? null);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    let left = false;
    const known = coloured.get(key);
    if (known !== undefined) {
      setMarkup(known);
      return;
    }
    setMarkup(null);
    if (!language) return;
    void highlighter().then((shiki) => {
      if (left) return;
      const named = shiki.getLoadedLanguages().includes(language);
      if (!named) return;
      const made = shiki.codeToHtml(code, { lang: language, theme: THEME });
      coloured.set(key, made);
      setMarkup(made);
    });
    return () => {
      left = true;
    };
  }, [key, code, language]);
  const copy = () => {
    void navigator.clipboard?.writeText(code).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  };
  return (
    <figure className="k-code">
      <header>
        <span>{language || "text"}</span>
        <button type="button" className="k-btn k-quiet" onClick={copy} aria-label="Copy the code">
          {copied ? <Check size={14} /> : <Copy size={14} />}
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </header>
      {markup ? <div dangerouslySetInnerHTML={{ __html: markup }} /> : <pre><code>{code}</code></pre>}
    </figure>
  );
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (node && typeof node === "object" && "props" in node) return textOf((node as ReactElement<{ children?: ReactNode }>).props.children);
  return "";
}

const parts: Components = {
  pre({ children }) {
    const inner = (Array.isArray(children) ? children[0] : children) as ReactElement<{ className?: string; children?: ReactNode }> | undefined;
    const language = /language-([\w+-]+)/.exec(inner?.props?.className ?? "")?.[1] ?? "";
    return <Code code={textOf(inner?.props?.children).replace(/\n$/, "")} language={language} />;
  },
  a({ href, children }) {
    // What an agent links to opens beside the chat, never in place of it.
    return <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>;
  },
};

export const Prose = memo(function Prose({ text, settled }: { text: string; settled: boolean }) {
  return (
    <div className="k-prose">
      <Markdown remarkPlugins={[remarkGfm]} components={parts}>
        {settled ? text : remend(text)}
      </Markdown>
    </div>
  );
});
