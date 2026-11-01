use super::*;

/// Records everything as text, one entry per callback.
#[derive(Default)]
struct Log(Vec<String>);

impl Perform for Log {
    fn print_ascii(&mut self, s: &[u8]) {
        self.0.push(format!("a:{}", String::from_utf8_lossy(s)));
    }
    fn print(&mut self, c: char) {
        self.0.push(format!("c:{c}"));
    }
    fn execute(&mut self, b: u8) {
        self.0.push(format!("x:{b:02x}"));
    }
    fn csi(&mut self, p: &Params, inter: &[u8], action: u8) {
        let groups: Vec<String> = p.groups().map(|g| format!("{g:?}")).collect();
        let inter = String::from_utf8_lossy(inter);
        self.0
            .push(format!("csi:{inter}{}{}", groups.join(""), action as char));
    }
    fn esc(&mut self, inter: &[u8], action: u8) {
        let inter = String::from_utf8_lossy(inter);
        self.0.push(format!("esc:{inter}{}", action as char));
    }
    fn osc(&mut self, data: &[u8], bel: bool) {
        let data = String::from_utf8_lossy(data);
        self.0.push(format!("osc:{data}:{bel}"));
    }
}

fn run(bytes: &[u8]) -> Vec<String> {
    let mut log = Log::default();
    Parser::new().feed(&mut log, bytes);
    log.0
}

#[test]
fn ascii_runs_are_one_call() {
    assert_eq!(run(b"hello\r\n"), ["a:hello", "x:0d", "x:0a"]);
}

#[test]
fn utf8_split_across_feeds() {
    let mut log = Log::default();
    let mut p = Parser::new();
    let bytes = "é漢".as_bytes();
    for b in bytes {
        p.feed(&mut log, std::slice::from_ref(b));
    }
    assert_eq!(log.0, ["c:é", "c:漢"]);
}

#[test]
fn broken_utf8_gives_replacement() {
    assert_eq!(run(b"\xe6a"), ["c:\u{fffd}", "a:a"]);
    assert_eq!(run(b"\xff"), ["c:\u{fffd}"]);
}

#[test]
fn csi_params_and_subparams() {
    assert_eq!(run(b"\x1b[1;31m"), ["csi:[1][31]m"]);
    assert_eq!(run(b"\x1b[4:3m"), ["csi:[4, 3]m"]);
    assert_eq!(run(b"\x1b[38:2::1:2:3m"), ["csi:[38, 2, 0, 1, 2, 3]m"]);
    assert_eq!(run(b"\x1b[;5H"), ["csi:[0][5]H"]);
}

#[test]
fn private_and_intermediates() {
    assert_eq!(run(b"\x1b[?1049h"), ["csi:?[1049]h"]);
    assert_eq!(run(b"\x1b[?2026$p"), ["csi:?$[2026]p"]);
    assert_eq!(run(b"\x1b[2 q"), ["csi: [2]q"]);
}

#[test]
fn controls_inside_csi_run_immediately() {
    assert_eq!(run(b"\x1b[1\n2A"), ["x:0a", "csi:[12]A"]);
}

#[test]
fn osc_bel_and_st() {
    assert_eq!(run(b"\x1b]0;hi\x07"), ["osc:0;hi:true"]);
    assert_eq!(run(b"\x1b]2;t\x1b\\x"), ["osc:2;t:false", "esc:\\", "a:x"]);
}

#[test]
fn esc_with_intermediate() {
    assert_eq!(run(b"\x1b(0\x1b7"), ["esc:(0", "esc:7"]);
}

#[test]
fn dcs_is_swallowed() {
    assert_eq!(run(b"\x1bP+q544e\x1b\\ok"), ["esc:\\", "a:ok"]);
}

#[test]
fn can_aborts() {
    assert_eq!(run(b"\x1b[12\x18x"), ["a:x"]);
}
