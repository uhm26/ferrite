//! Window rectangles from Hyprland's IPC socket: the text replies of `clients` and
//! `monitors` (no JSON parser needed: one `key: value` per line).

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// A window someone can see right now, in layout coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Win {
    pub addr: u64,
    /// x, y, w, h
    pub rect: [i32; 4],
    workspace: i64,
    floating: bool,
    fullscreen: bool,
    /// 0 = focused last; lower is more recent, so more likely on top.
    focus: u32,
}

/// Windows on the workspaces shown now, topmost-first: fullscreen, floating, tiled,
/// each by focus recency. `hit` takes the first that contains a point.
pub fn windows() -> io::Result<Vec<Win>> {
    let shown = shown_workspaces(&request("monitors")?);
    let mut wins: Vec<Win> = parse_clients(&request("clients")?)
        .into_iter()
        .filter(|w| shown.contains(&w.workspace))
        .collect();
    wins.sort_by_key(|w| (!w.fullscreen, !w.floating, w.focus));
    Ok(wins)
}

pub fn hit(wins: &[Win], x: i32, y: i32) -> Option<&Win> {
    wins.iter().find(|w| {
        let [wx, wy, ww, wh] = w.rect;
        (wx..wx + ww).contains(&x) && (wy..wy + wh).contains(&y)
    })
}

fn request(cmd: &str) -> io::Result<String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_default();
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .ok_or_else(|| io::Error::other("not running under Hyprland"))?;
    let path = PathBuf::from(dir)
        .join("hypr")
        .join(sig)
        .join(".socket.sock");
    let mut s = UnixStream::connect(path)?;
    // A stalled Hyprland must not hang screenshot: no windows is better than no screenshot.
    s.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
    s.write_all(cmd.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

/// Active and special workspace ids of every monitor.
fn shown_workspaces(text: &str) -> Vec<i64> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let v = l
                .strip_prefix("active workspace: ")
                .or(l.strip_prefix("special workspace: "))?;
            first_int(v)
        })
        .collect()
}

/// `Window <hex addr> -> title:` starts a block; then tab-indented `key: value` lines.
fn parse_clients(text: &str) -> Vec<Win> {
    let mut wins = Vec::new();
    let mut cur: Option<(Win, bool)> = None; // (window, still visible)
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Window ") {
            wins.extend(cur.take().filter(|(_, ok)| *ok).map(|(w, _)| w));
            let hex = rest.split(' ').next().unwrap_or_default();
            let Ok(addr) = u64::from_str_radix(hex, 16) else {
                continue;
            };
            let w = Win {
                addr,
                rect: [0; 4],
                workspace: 0,
                floating: false,
                fullscreen: false,
                focus: u32::MAX,
            };
            cur = Some((w, true));
            continue;
        }
        let (Some((w, ok)), Some((k, v))) = (cur.as_mut(), line.trim_start().split_once(": "))
        else {
            continue;
        };
        match k {
            "mapped" => *ok &= v == "1",
            "hidden" => *ok &= v == "0",
            "at" => (w.rect[0], w.rect[1]) = pair(v),
            "size" => (w.rect[2], w.rect[3]) = pair(v),
            "workspace" => w.workspace = first_int(v).unwrap_or(0),
            "floating" => w.floating = v == "1",
            "fullscreen" => w.fullscreen = v != "0",
            "focusHistoryID" => w.focus = v.parse().unwrap_or(u32::MAX),
            _ => {}
        }
    }
    wins.extend(cur.filter(|(_, ok)| *ok).map(|(w, _)| w));
    wins
}

fn pair(v: &str) -> (i32, i32) {
    let (a, b) = v.split_once(',').unwrap_or_default();
    (a.trim().parse().unwrap_or(0), b.trim().parse().unwrap_or(0))
}

fn first_int(v: &str) -> Option<i64> {
    v.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENTS: &str = "Window 55cc41dd76e0 -> Firefox:\n\tmapped: 1\n\thidden: 0\n\tat: 21,61\n\tsize: 1878,998\n\tworkspace: 3 (3)\n\tfloating: 0\n\tfullscreen: 0\n\tfocusHistoryID: 2\n\nWindow abc -> popup:\n\tmapped: 1\n\thidden: 0\n\tat: 100,100\n\tsize: 50,50\n\tworkspace: 3 (3)\n\tfloating: 1\n\tfullscreen: 0\n\tfocusHistoryID: 5\n\nWindow def -> gone:\n\tmapped: 1\n\thidden: 1\n\tat: 0,0\n\tsize: 9,9\n\tworkspace: 3 (3)\n";

    #[test]
    fn parses_clients_and_skips_hidden() {
        let w = parse_clients(CLIENTS);
        assert_eq!(w.len(), 2);
        assert_eq!(
            (w[0].addr, w[0].rect),
            (0x55cc_41dd_76e0, [21, 61, 1878, 998])
        );
        assert!(w[1].floating);
    }

    #[test]
    fn floating_wins_the_hit() {
        let mut w = parse_clients(CLIENTS);
        w.sort_by_key(|w| (!w.fullscreen, !w.floating, w.focus));
        assert_eq!(hit(&w, 120, 120).map(|w| w.addr), Some(0xabc));
        assert_eq!(hit(&w, 30, 70).map(|w| w.addr), Some(0x55cc_41dd_76e0));
        assert_eq!(hit(&w, 5, 5), None);
    }

    #[test]
    fn shown_workspaces_from_monitors() {
        let m = "Monitor DP-2 (ID 0):\n\tactive workspace: 2 (2)\n\tspecial workspace:  ()\nMonitor DP-3 (ID 1):\n\tactive workspace: 1 (1)\n\tspecial workspace: -98 (special:magic)\n";
        assert_eq!(shown_workspaces(m), vec![2, 1, -98]);
    }
}
