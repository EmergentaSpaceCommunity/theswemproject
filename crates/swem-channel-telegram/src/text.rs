//! What an agent says is Markdown; Telegram shows HTML of a small kind.
//!
//! The subset Telegram renders: `b`, `i`, `s`, `u`, `code`, `pre`,
//! `a href`, `blockquote`, `tg-spoiler`. Headings become bold lines, lists
//! become lines with a bullet or a number, tables are left as code so their
//! columns survive, and everything else is text with `<`, `>` and `&`
//! escaped. A message longer than the limit is cut at a paragraph.

use std::fmt::Write as _;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Escape what HTML would otherwise read as markup.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What the renderer carries from one event to the next.
#[derive(Default)]
struct Rendering {
    out: String,
    /// The lists being written, innermost last; a number for an ordered one.
    list_stack: Vec<Option<u64>>,
    in_code_block: bool,
    in_table: bool,
    table_row: String,
    table: String,
}

impl Rendering {
    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { .. } | Tag::Strong => self.out.push_str("<b>"),
            Tag::BlockQuote(_) => self.out.push_str("<blockquote>"),
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                match kind {
                    CodeBlockKind::Fenced(language) if !language.is_empty() => {
                        let _ = write!(
                            self.out,
                            "<pre><code class=\"language-{}\">",
                            escaped(&language)
                        );
                    }
                    _ => self.out.push_str("<pre>"),
                }
            }
            Tag::List(first) => self.list_stack.push(first),
            Tag::Item => {
                let depth = self.list_stack.len().saturating_sub(1);
                self.out.push_str(&"  ".repeat(depth));
                match self.list_stack.last_mut() {
                    Some(Some(number)) => {
                        let _ = write!(self.out, "{number}. ");
                        *number += 1;
                    }
                    _ => self.out.push_str("• "),
                }
            }
            Tag::Emphasis => self.out.push_str("<i>"),
            Tag::Strikethrough => self.out.push_str("<s>"),
            Tag::Link { dest_url, .. } => {
                let _ = write!(self.out, "<a href=\"{}\">", escaped(&dest_url));
            }
            Tag::Table(_) => {
                self.in_table = true;
                self.table.clear();
            }
            Tag::TableHead | Tag::TableRow => self.table_row.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.out.push_str("\n\n"),
            TagEnd::Heading(_) => self.out.push_str("</b>\n\n"),
            TagEnd::BlockQuote(_) => self.out.push_str("</blockquote>\n\n"),
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                if self.out.ends_with("<pre>") {
                    self.out.push_str("</pre>\n\n");
                } else if self.out.contains("<code class=\"language-")
                    && !self.out.ends_with("</code></pre>\n\n")
                {
                    self.out.push_str("</code></pre>\n\n");
                } else {
                    self.out.push_str("</pre>\n\n");
                }
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.out.push('\n');
                }
            }
            TagEnd::Item => self.out.push('\n'),
            TagEnd::Emphasis => self.out.push_str("</i>"),
            TagEnd::Strong => self.out.push_str("</b>"),
            TagEnd::Strikethrough => self.out.push_str("</s>"),
            TagEnd::Link => self.out.push_str("</a>"),
            TagEnd::Table => {
                self.in_table = false;
                self.out.push_str("<pre>");
                self.out.push_str(&escaped(self.table.trim_end()));
                self.out.push_str("</pre>\n\n");
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                self.table.push_str(self.table_row.trim_end_matches(" | "));
                self.table.push('\n');
            }
            TagEnd::TableCell => self.table_row.push_str(" | "),
            _ => {}
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if self.in_table {
                    self.table_row.push_str(&text);
                } else {
                    self.out.push_str(&escaped(&text));
                }
            }
            Event::Code(code) => {
                if self.in_table {
                    self.table_row.push_str(&code);
                } else {
                    let _ = write!(self.out, "<code>{}</code>", escaped(&code));
                }
            }
            Event::SoftBreak => self.out.push(if self.in_code_block { '\n' } else { ' ' }),
            Event::HardBreak => self.out.push('\n'),
            Event::Rule => self.out.push_str("———\n\n"),
            Event::Html(raw) | Event::InlineHtml(raw) => self.out.push_str(&escaped(&raw)),
            Event::TaskListMarker(done) => self.out.push_str(if done { "☑ " } else { "☐ " }),
            _ => {}
        }
    }
}

/// Markdown to the HTML Telegram renders.
#[must_use]
pub fn html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    let mut rendering = Rendering::default();
    for event in Parser::new_ext(markdown, options) {
        rendering.event(event);
    }
    rendering.out.trim_end().to_owned()
}

/// Cut text into pieces no longer than `limit`, at paragraph, then line,
/// then word boundaries, never inside a tag's name.
#[must_use]
pub fn chunks(text: &str, limit: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while rest.chars().count() > limit {
        let window: String = rest.chars().take(limit).collect();
        let cut = window
            .rfind("\n\n")
            .or_else(|| window.rfind('\n'))
            .or_else(|| window.rfind(' '))
            .filter(|&at| at > limit / 4)
            .unwrap_or(window.len());
        let (head, tail) = rest.split_at(cut);
        pieces.push(head.trim_end().to_owned());
        rest = tail.trim_start();
    }
    if !rest.is_empty() || pieces.is_empty() {
        pieces.push(rest.to_owned());
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_becomes_telegrams_html() {
        let out = html(
            "# Title\n\nSome **bold** and `code` with <tags> & more.\n\n- one\n- two\n\n```rust\nlet x = 1;\n```",
        );
        assert!(out.starts_with("<b>Title</b>"));
        assert!(out.contains("<b>bold</b>"));
        assert!(out.contains("<code>code</code>"));
        assert!(out.contains("&lt;tags&gt; &amp; more"));
        assert!(out.contains("• one\n• two"));
        assert!(out.contains("<pre><code class=\"language-rust\">let x = 1;\n</code></pre>"));
    }

    #[test]
    fn a_table_survives_as_code() {
        let out = html("| a | b |\n|---|---|\n| 1 | 2 |");
        assert!(out.contains("<pre>a | b\n1 | 2</pre>"), "{out}");
    }

    #[test]
    fn long_text_is_cut_at_paragraphs() {
        let text = format!("{}\n\n{}", "a".repeat(30), "b".repeat(30));
        let pieces = chunks(&text, 40);
        assert_eq!(pieces, vec!["a".repeat(30), "b".repeat(30)]);
        assert_eq!(chunks("short", 40), vec!["short"]);
    }
}
