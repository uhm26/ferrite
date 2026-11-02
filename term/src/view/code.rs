//! Source files to styled lines: a dim line-number gutter and light syntax colors
//! (comments, strings, numbers, keywords, Capitalized types). A word-level scanner,
//! not a parser: good enough to read code, wrong now and then on odd syntax.

use super::{Line, Span, Style};

const DIM: u8 = 8;
const STRING: u8 = 2;
const NUMBER: u8 = 3;
const KEYWORD: u8 = 5;
const TYPE: u8 = 6;

/// Keywords across the languages Claude shows most (Rust, Python, JS/TS, Go, C, shell).
const KEYWORDS: &[&str] = &[
    "fn",
    "let",
    "mut",
    "pub",
    "use",
    "mod",
    "struct",
    "enum",
    "impl",
    "trait",
    "for",
    "while",
    "loop",
    "if",
    "else",
    "match",
    "return",
    "const",
    "static",
    "self",
    "crate",
    "super",
    "where",
    "as",
    "in",
    "break",
    "continue",
    "async",
    "await",
    "move",
    "ref",
    "type",
    "unsafe",
    "extern",
    "dyn",
    "true",
    "false",
    "def",
    "class",
    "import",
    "from",
    "lambda",
    "function",
    "var",
    "new",
    "this",
    "null",
    "nil",
    "and",
    "or",
    "not",
    "elif",
    "pass",
    "raise",
    "try",
    "except",
    "finally",
    "with",
    "yield",
    "is",
    "func",
    "package",
    "go",
    "defer",
    "switch",
    "case",
    "default",
    "export",
    "interface",
    "then",
    "fi",
    "do",
    "done",
    "end",
];

/// The line-comment marker for a file extension.
pub(super) fn comment_mark(ext: &str) -> &'static str {
    match ext {
        "py" | "sh" | "bash" | "fish" | "toml" | "yaml" | "yml" | "conf" | "rb" | "pl" | "r"
        | "cfg" | "ini" | "nix" | "mk" => "#",
        "lua" | "sql" | "hs" => "--",
        _ => "//",
    }
}

pub fn format(text: &str, ext: &str) -> Vec<Line> {
    let count = text.lines().count().max(1);
    let gutter = count.to_string().len();
    let mark = comment_mark(ext);
    text.lines()
        .enumerate()
        .map(|(i, raw)| {
            let num = Span::new(
                Style {
                    fg: Some(DIM),
                    ..Style::default()
                },
                format!("{:>gutter$} ", i + 1),
            );
            let mut spans = vec![num];
            spans.extend(highlight(raw, mark));
            Line {
                source: i,
                spans,
                indent: gutter + 1,
                ..Line::default()
            }
        })
        .collect()
}

/// One line split into colored pieces.
pub(super) fn highlight(line: &str, mark: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        if rest.starts_with(mark) {
            let comment = Style {
                fg: Some(DIM),
                italic: true,
                ..Style::default()
            };
            spans.push(Span::new(comment, rest));
            break;
        }
        let (piece, style) = next_piece(rest);
        spans.push(Span::new(style, piece));
        rest = &rest[piece.len()..];
    }
    spans
}

/// The next token of `s` and its style.
fn next_piece(s: &str) -> (&str, Style) {
    let c = s.chars().next().unwrap_or(' ');
    let plain = Style::default();
    if c == '"' {
        let end = string_end(s);
        return (
            &s[..end],
            Style {
                fg: Some(STRING),
                ..plain
            },
        );
    }
    if c.is_alphanumeric() || c == '_' {
        let end = s
            .find(|x: char| !(x.is_alphanumeric() || x == '_'))
            .unwrap_or(s.len());
        let word = &s[..end];
        let fg = if c.is_ascii_digit() {
            Some(NUMBER)
        } else if KEYWORDS.contains(&word) {
            Some(KEYWORD)
        } else if c.is_uppercase() {
            Some(TYPE)
        } else {
            None
        };
        return (word, Style { fg, ..plain });
    }
    (&s[..c.len_utf8()], plain)
}

/// Byte index just past the closing quote of the string starting `s` (escapes skipped).
fn string_end(s: &str) -> usize {
    let mut escaped = false;
    for (i, c) in s.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return i + 1,
            _ => {}
        }
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(line: &str) -> Vec<(String, Option<u8>)> {
        highlight(line, "//")
            .into_iter()
            .map(|s| (s.text, s.style.fg))
            .filter(|(t, _)| t.trim() != "")
            .collect()
    }

    #[test]
    fn colors_tokens() {
        let c = colors(r#"let x: Vec<u8> = "a\"b"; // note"#);
        assert_eq!(c[0], ("let".into(), Some(KEYWORD)));
        assert!(c.contains(&("Vec".into(), Some(TYPE))));
        assert!(c.contains(&(r#""a\"b""#.into(), Some(STRING))));
        assert_eq!(c.last().unwrap(), &("// note".into(), Some(DIM)));
        assert_eq!(colors("42")[0], ("42".into(), Some(NUMBER)));
    }

    #[test]
    fn gutter_width_fits_the_line_count() {
        let text = "a\n".repeat(120);
        let lines = format(&text, "rs");
        assert_eq!(lines[0].spans[0].text, "  1 ");
        assert_eq!(lines[0].indent, 4);
    }
}
