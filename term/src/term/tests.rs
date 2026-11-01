use super::cell::{Color, Flags};
use super::*;

fn palette() -> Palette {
    Palette::new(&crate::config::Config::default().theme)
}

fn term(cols: usize, rows: usize) -> Term {
    crate::sys::init_locale();
    Term::new(cols, rows, 100, palette())
}

/// Screen row `y` as text, trailing blanks cut.
fn line(t: &Term, y: usize) -> String {
    let s: String = t
        .visible_row(y)
        .iter()
        .filter(|c| !c.flags.has(Flags::SPACER))
        .map(|c| c.ch)
        .collect();
    s.trim_end().to_string()
}

#[test]
fn prints_and_wraps() {
    let mut t = term(5, 3);
    t.feed(b"helloworld");
    assert_eq!(line(&t, 0), "hello");
    assert_eq!(line(&t, 1), "world");
    assert!(t.grid.row(0).wrapped);
    assert_eq!(
        (t.cursor.row, t.cursor.col, t.cursor.pending_wrap),
        (1, 4, true)
    );
}

#[test]
fn scroll_into_history_and_view() {
    let mut t = term(4, 2);
    t.feed(b"a\r\nb\r\nc");
    assert_eq!(t.history_len(), 1);
    assert_eq!(line(&t, 0), "b");
    t.scroll_view(1);
    assert_eq!(line(&t, 0), "a");
    assert_eq!(line(&t, 1), "b");
    t.feed(b"\r\nd"); // view stays on the same text
    assert_eq!(line(&t, 0), "a");
}

#[test]
fn cursor_moves_and_erase() {
    let mut t = term(10, 3);
    t.feed(b"abcdefghij\x1b[1;3H\x1b[K");
    assert_eq!(line(&t, 0), "ab");
    t.feed(b"\x1b[2;5HX\x1b[1K");
    assert_eq!(line(&t, 1), "");
    t.feed(b"\x1b[3;1Hxyz\x1b[3;2H\x1b[P");
    assert_eq!(line(&t, 2), "xz");
    t.feed(b"\x1b[2@");
    assert_eq!(line(&t, 2), "x  z");
}

#[test]
fn sgr_colors() {
    let mut t = term(10, 2);
    t.feed(b"\x1b[1;31mA\x1b[38;2;1;2;3mB\x1b[38:5:200mC\x1b[4:3;48;5;7mD\x1b[0mE");
    let row = t.visible_row(0);
    assert_eq!(row[0].fg, Color::index(1));
    assert!(row[0].flags.has(Flags::BOLD));
    assert_eq!(row[1].fg, Color::rgb(1, 2, 3));
    assert_eq!(row[2].fg, Color::index(200));
    assert!(row[3].flags.has(Flags::CURLY_UL));
    assert_eq!(row[3].bg, Color::index(7));
    assert_eq!(row[4].fg, Color::DEFAULT);
    assert_eq!(row[4].flags, Flags(0));
}

#[test]
fn wide_chars() {
    let mut t = term(4, 2);
    t.feed("漢字x".as_bytes());
    assert_eq!(line(&t, 0), "漢字");
    assert_eq!(line(&t, 1), "x");
    t.feed(b"\x1b[1;2Ha"); // overwrite the spacer: the wide char goes
    assert_eq!(line(&t, 0), " a字");
}

#[test]
fn alt_screen_keeps_primary() {
    let mut t = term(5, 2);
    t.feed(b"main\x1b[?1049h");
    assert_eq!(line(&t, 0), "");
    t.feed(b"alt\x1b[?1049l");
    assert_eq!(line(&t, 0), "main");
    assert_eq!(t.cursor.col, 4);
}

#[test]
fn scroll_region() {
    let mut t = term(3, 4);
    t.feed(b"1\r\n2\r\n3\r\n4\x1b[2;3r\x1b[3;1H\n");
    assert_eq!(
        [line(&t, 0), line(&t, 1), line(&t, 2), line(&t, 3)],
        ["1", "3", "", "4"]
    );
    assert_eq!(t.history_len(), 0);
}

#[test]
fn replies() {
    let mut t = term(10, 5);
    t.feed(b"\x1b[3;4H\x1b[6n\x1b[c\x1b]11;?\x07\x1b[?2026$p");
    let r = String::from_utf8(t.replies.clone()).unwrap();
    assert_eq!(
        r,
        "\x1b[3;4R\x1b[?62;22c\x1b]11;rgb:0d0d/0d0d/0d0d\x07\x1b[?2026;2$y"
    );
}

#[test]
fn title_and_line_drawing() {
    let mut t = term(5, 2);
    t.feed(b"\x1b]2;hi there\x1b\\\x1b(0qx\x1b(Bq");
    assert_eq!(t.title.as_deref(), Some("hi there"));
    assert_eq!(line(&t, 0), "─│q");
}

#[test]
fn resize_keeps_cursor_line() {
    let mut t = term(5, 4);
    t.feed(b"a\r\nb\r\nc\r\nd");
    t.resize(5, 2);
    assert_eq!([line(&t, 0), line(&t, 1)], ["c", "d"]);
    assert_eq!(t.cursor.row, 1);
    t.resize(3, 4);
    assert_eq!(line(&t, 0), "a");
    assert_eq!(t.cursor.row, 3);
}

#[test]
fn no_autowrap_overwrites_last_column() {
    let mut t = term(3, 2);
    t.feed(b"\x1b[?7labcdef");
    assert_eq!(line(&t, 0), "abf");
}
