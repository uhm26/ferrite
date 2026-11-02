//! Markdown to styled lines: headings (h1 in the sand accent, all bold, h1/h2 with a
//! rule under them), lists as `•`, quotes with a bar, fenced code on a shaded block,
//! rules, tables with box lines, and inline `code`, **bold**, *italic* and links
//! (`[text](url)` shows `text (url)`, so the url can be Ctrl+clicked). A small subset:
//! what READMEs, plans and Claude's notes use.

use super::{Line, Span, Style};

const ACCENT: u8 = 3; // sand (the accent)
const DIM: u8 = 8;
const BRIGHT: u8 = 15;
const CODE_BG: u8 = 0;

pub fn format(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut fence = false;
    // The previous line was paragraph or list text: a plain line right after it
    // continues it (Markdown joins hard-wrapped lines into one paragraph).
    let mut flowing = false;
    for (i, raw) in text.lines().enumerate() {
        let continues = flowing && starts_plain(raw);
        flowing = false;
        let line = |spans: Vec<Span>| Line {
            source: i,
            spans,
            ..Line::default()
        };
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            fence = !fence;
            let label = Span::new(dim(), trimmed.trim_start_matches('`'));
            out.push(Line {
                fill: Some(CODE_BG),
                ..line(vec![label])
            });
        } else if fence {
            let code = Style {
                bg: Some(CODE_BG),
                ..Style::default()
            };
            out.push(Line {
                fill: Some(CODE_BG),
                ..line(vec![Span::new(code, raw)])
            });
        } else if is_rule(trimmed) {
            out.push(Line {
                rule: true,
                ..line(Vec::new())
            });
        } else if let Some((level, title)) = heading(trimmed) {
            out.push(line(inline(title, heading_style(level))));
            if level <= 2 {
                out.push(Line {
                    rule: true,
                    ..line(Vec::new())
                });
            }
        } else if continues && let Some(last) = out.last_mut() {
            last.spans.push(Span::new(Style::default(), " "));
            last.spans.extend(inline(raw.trim(), Style::default()));
            flowing = true;
        } else {
            out.push(block(i, raw));
            flowing = !raw.trim().is_empty() && !raw.trim_start().starts_with(['>', '|']);
        }
    }
    out
}

/// Plain text: not empty, not a list item, quote, table row, heading or fence.
fn starts_plain(raw: &str) -> bool {
    let t = raw.trim_start();
    let numbered = t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    !t.is_empty()
        && !numbered
        && ![">", "|", "#", "```", "- ", "* ", "+ "].iter().any(|p| t.starts_with(p))
        && !is_rule(t)
}

/// Quotes, list items, table rows and paragraphs.
fn block(i: usize, raw: &str) -> Line {
    let trimmed = raw.trim_start();
    let pad = raw.len() - trimmed.len();
    let (prefix, rest, style) = if let Some(rest) = trimmed.strip_prefix('>') {
        (
            "│ ".to_string(),
            rest.trim_start(),
            Style {
                italic: true,
                ..Style::default()
            },
        )
    } else if let Some(rest) = ["- ", "* ", "+ "]
        .iter()
        .find_map(|m| trimmed.strip_prefix(m))
    {
        (format!("{}• ", " ".repeat(pad)), rest, Style::default())
    } else if trimmed.starts_with('|') {
        return Line {
            source: i,
            spans: vec![table_row(raw)],
            ..Line::default()
        };
    } else {
        (" ".repeat(pad), trimmed, Style::default())
    };
    let mut spans = vec![Span::new(
        Style {
            fg: Some(DIM),
            ..Style::default()
        },
        prefix.clone(),
    )];
    spans.extend(inline(rest, style));
    Line {
        source: i,
        spans,
        indent: prefix.chars().count(),
        ..Line::default()
    }
}

fn is_rule(t: &str) -> bool {
    let t = t.trim_end();
    t.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|c| t.chars().all(|x| x.to_string() == *c))
}

fn heading(t: &str) -> Option<(usize, &str)> {
    let level = t.chars().take_while(|&c| c == '#').count();
    let rest = t.get(level..)?;
    ((1..=6).contains(&level) && rest.starts_with(' ')).then(|| (level, rest.trim()))
}

fn heading_style(level: usize) -> Style {
    let fg = match level {
        1 => Some(ACCENT),
        2 => Some(BRIGHT),
        _ => None,
    };
    Style {
        fg,
        bold: true,
        ..Style::default()
    }
}

fn dim() -> Style {
    Style {
        fg: Some(DIM),
        ..Style::default()
    }
}

/// `| a | b |` with box-drawing bars; the `|---|` separator becomes a line.
fn table_row(raw: &str) -> Span {
    let sep = raw.chars().all(|c| "|-: ".contains(c));
    let text: String = raw
        .chars()
        .map(|c| match c {
            '|' => '│',
            '-' | ':' if sep => '─',
            c => c,
        })
        .collect();
    Span::new(if sep { dim() } else { Style::default() }, text)
}

/// Inline markup on top of `base`: `code`, **bold**, *italic* / _italic_, links.
fn inline(text: &str, base: Style) -> Vec<Span> {
    let mut spans = Vec::new();
    let (mut cur, mut style) = (String::new(), base);
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let flush = |cur: &mut String, spans: &mut Vec<Span>, style| {
            if !cur.is_empty() {
                spans.push(Span::new(style, std::mem::take(cur)));
            }
        };
        if c == '`'
            && let Some(end) = rest[1..].find('`')
        {
            flush(&mut cur, &mut spans, style);
            let code = Style {
                bg: Some(CODE_BG),
                ..base
            };
            spans.push(Span::new(code, &rest[1..=end]));
            rest = &rest[end + 2..];
        } else if let Some(r) = rest.strip_prefix("**").or_else(|| rest.strip_prefix("__")) {
            flush(&mut cur, &mut spans, style);
            style.bold = !style.bold;
            rest = r;
        } else if (c == '*' || (c == '_' && word_edge(text, rest))) && rest.len() > 1 {
            flush(&mut cur, &mut spans, style);
            style.italic = !style.italic;
            rest = &rest[1..];
        } else if let Some((label, url, after)) = link(rest) {
            flush(&mut cur, &mut spans, style);
            spans.push(Span::new(
                Style {
                    underline: true,
                    ..style
                },
                label,
            ));
            spans.push(Span::new(dim(), format!(" ({url})")));
            rest = after;
        } else {
            cur.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    if !cur.is_empty() {
        spans.push(Span::new(style, cur));
    }
    spans
}

/// `_` only marks italics at a word edge (not inside snake_case names).
fn word_edge(text: &str, rest: &str) -> bool {
    let at = text.len() - rest.len();
    let before = text[..at].chars().next_back();
    let after = rest[1..].chars().next();
    !before.is_some_and(char::is_alphanumeric) || !after.is_some_and(char::is_alphanumeric)
}

/// `[label](url)` at the start of `s`: label, url, and what follows.
fn link(s: &str) -> Option<(&str, &str, &str)> {
    let s = s.strip_prefix('[')?;
    let close = s.find("](")?;
    let end = s[close + 2..].find(')')? + close + 2;
    Some((&s[..close], &s[close + 2..end], &s[end + 1..]))
}

#[cfg(test)]
mod tests {
    use super::super::emit;
    use super::super::tests::screen;
    use super::*;

    fn shown(md: &str) -> Vec<String> {
        screen(&emit(&format(md), 60, 0).0, 60)
    }

    #[test]
    fn blocks_and_inline() {
        let md = "# Title\nSome **bold** and `code`, see [docs](https://x.y).\n- item one\n> quote\n```rust\nlet a = 1;\n```\nsnake_case_name stays";
        let rows = shown(md);
        assert_eq!(rows[0], "Title");
        assert!(rows[1].starts_with("────"));
        assert_eq!(rows[2], "Some bold and code, see docs (https://x.y).");
        assert_eq!(rows[3], "• item one");
        assert_eq!(rows[4], "│ quote");
        assert_eq!(rows[5], "rust");
        assert_eq!(rows[6], "let a = 1;");
        assert_eq!(rows[7], "snake_case_name stays");
    }

    #[test]
    fn hard_wrapped_lines_join() {
        let rows = shown("one two\nthree\n\n- item\n  more\n1. first\nnext para");
        assert_eq!(rows, ["one two three", "• item more", "1. first next para"]);
    }

    #[test]
    fn styles() {
        let spans = inline("a **b** *c* `d`", Style::default());
        let bold = spans.iter().find(|s| s.text == "b").unwrap();
        assert!(bold.style.bold);
        assert!(spans.iter().find(|s| s.text == "c").unwrap().style.italic);
        assert_eq!(
            spans.iter().find(|s| s.text == "d").unwrap().style.bg,
            Some(CODE_BG)
        );
        assert_eq!(heading("## Hi"), Some((2, "Hi")));
        assert_eq!(heading("#no"), None);
        assert!(is_rule("---") && !is_rule("--"));
    }
}
