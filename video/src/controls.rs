//! The controls over the video: one flat bar floating at the bottom (frame's
//! panel, boltsnap look): play/pause, time, seek track, total, volume, fullscreen.
//! It glides down out of the window when the pointer rests, and back on motion.
//! Drawn on the window surface, which is see-through everywhere else.

use frame::ui::theme::{CONTROL_H, GLIDE, KNOB, S4, S8, S12, S24, SNAP};
use frame::ui::{Align, Icon, Ink, Rect, Ui};

use crate::player::State;

/// The bar: one control row with S8 around it (frame's toolbar band, 48).
pub const BAR_H: f32 = CONTROL_H + 2.0 * S8;
/// Gap between the bar and the window edges.
const MARGIN: f32 = S12;
/// Track thickness at rest; it grows by 2 px under the pointer.
const TRACK: f32 = S4;
const VOLUME_W: f32 = 2.0 * 32.0;
/// The time pill above the seek track.
const TIP_H: f32 = S24;
/// Letterbox black: the video's own background, not a ui surface.
pub const VIDEO_BG: u32 = 0xff00_0000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Toggle,
    /// Seek to seconds: exact (a click, the end of a drag) or to the keyframe
    /// before it (fast: each step of a drag).
    Seek(f64, bool),
    /// Seek by seconds, to a keyframe in that direction (fast, like mpv's arrows).
    Skip(f64),
    /// 0-100.
    Volume(f64),
    /// Volume up (+) or down (-) by this much.
    Louder(f64),
    Mute,
    Fullscreen,
}

/// What the bar shows this frame.
pub struct Look<'a> {
    pub state: &'a State,
    pub visible: bool,
    pub fullscreen: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Seek,
    Volume,
}

#[derive(Default)]
pub struct Controls {
    grab: Option<Grab>,
    /// Where the bar was last drawn (clicks there are not clicks on the video).
    pub bar: Rect,
    /// Last position sent while dragging, so a still pointer sends nothing.
    sent: Option<f64>,
    /// The seek grab moved since the press (its last seek was to a keyframe).
    moved: bool,
}

struct Layout {
    bar: Rect,
    play: Rect,
    cur: Rect,
    seek: Rect,
    total: Rect,
    mute: Rect,
    volume: Rect,
    full: Rect,
}

/// The bar `drop` px below its resting place in a `w` × `h` window, times `slot` wide.
fn layout(w: f32, h: f32, slot: f32, drop: f32) -> Layout {
    let bar = Rect::new(
        MARGIN,
        h - MARGIN - BAR_H + drop,
        (w - 2.0 * MARGIN).max(0.0),
        BAR_H,
    );
    let y = bar.y + S8;
    let button = |x: f32| Rect::new(x, y, CONTROL_H, CONTROL_H);
    let play = button(bar.x + S8);
    let full = button(bar.x + bar.w - S8 - CONTROL_H);
    let volume = Rect::new(full.x - S8 - VOLUME_W, y, VOLUME_W, CONTROL_H);
    let mute = button(volume.x - S4 - CONTROL_H);
    let cur = Rect::new(play.x + CONTROL_H + S8, y, slot, CONTROL_H);
    let total = Rect::new(mute.x - S12 - slot, y, slot, CONTROL_H);
    let sx = cur.x + slot + S12;
    let seek = Rect::new(sx, y, (total.x - S12 - sx).max(0.0), CONTROL_H);
    Layout {
        bar,
        play,
        cur,
        seek,
        total,
        mute,
        volume,
        full,
    }
}

/// The first window row the bar or its time pill can touch: rows above it never
/// change once the video shows, so only these are cleared and sent.
pub fn zone_top(h: usize) -> usize {
    let top = h as f32 - MARGIN - BAR_H - S8 - TIP_H - S4;
    top.max(0.0).floor() as usize
}

/// `t` seconds as M:SS, or H:MM:SS when `long`.
pub fn clock(t: f64, long: bool) -> String {
    let s = if t.is_finite() { t.max(0.0) as u64 } else { 0 };
    if long {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

impl Controls {
    /// Draw the bar; returns what the owner clicked or dragged.
    pub fn draw(&mut self, ui: &mut Ui, look: &Look) -> Vec<Action> {
        let st = look.state;
        let (w, h) = ui.p.size();
        let long = st.duration >= 3600.0;
        let ink = ui.small();
        let slot = clock_width(ui, &clock(st.duration, long), ink);
        let hidden = BAR_H + MARGIN;
        let drop = ui.spring("bar/drop", if look.visible { 0.0 } else { hidden }, GLIDE);
        let l = layout(w, h, slot, drop);
        self.bar = l.bar;
        let mut out = Vec::new();
        if drop >= hidden - 0.5 {
            self.grab = None;
            return out;
        }
        ui.panel(l.bar);
        let playing = !st.paused && !st.ended;
        if ui.icon_button(
            "play",
            l.play,
            if playing { Icon::Pause } else { Icon::Play },
            true,
        ) {
            out.push(Action::Toggle);
        }
        let at = self.seek(ui, &l, st, &mut out);
        let text = ui.pal.text;
        draw_clock(ui, l.cur, &clock(at, long), ink.color(text), Align::Right);
        draw_clock(ui, l.total, &clock(st.duration, long), ink, Align::Left);
        let quiet = st.muted || st.volume <= 0.0;
        if ui.icon_button(
            "mute",
            l.mute,
            if quiet { Icon::Mute } else { Icon::Volume },
            true,
        ) {
            out.push(Action::Mute);
        }
        self.volume(ui, l.volume, st, &mut out);
        let icon = if look.fullscreen {
            Icon::Shrink
        } else {
            Icon::Expand
        };
        if ui.icon_button("full", l.full, icon, true) {
            out.push(Action::Fullscreen);
        }
        out
    }

    /// Start, follow or end a drag of `grab` over `track`; the pointer's 0-1 spot
    /// on it while grabbed.
    fn drag(&mut self, ui: &mut Ui, grab: Grab, hit: Rect, track: Rect) -> Option<f64> {
        let i = &ui.s.input;
        let at = if track.w > 0.0 {
            f64::from((i.pos.0 - track.x) / track.w).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if i.pressed && ui.over(hit) && self.grab.is_none() {
            self.grab = Some(grab);
            self.sent = None;
        }
        (self.grab == Some(grab)).then_some(at)
    }

    /// The seek track: returns the time to show (the drag spot while dragging).
    fn seek(&mut self, ui: &mut Ui, l: &Layout, st: &State, out: &mut Vec<Action>) -> f64 {
        let hit = Rect::new(l.seek.x - S4, l.bar.y, l.seek.w + S8, l.bar.h);
        let can = st.duration > 0.0;
        if !can && self.grab == Some(Grab::Seek) {
            self.grab = None; // the file went away mid-drag
        }
        let grabbed = if can {
            self.drag(ui, Grab::Seek, hit, l.seek)
        } else {
            None
        };
        let released = ui.s.input.released || !ui.s.input.down;
        if let Some(at) = grabbed {
            // The press seeks exactly right away (a click needs nothing more), drag
            // steps go to keyframes, and the release lands exactly if it moved.
            let t = at * st.duration;
            let first = self.sent.is_none();
            if released {
                self.grab = None;
                if first || self.moved {
                    out.push(Action::Seek(t, true));
                }
                self.moved = false;
            } else if self.sent != Some(t) {
                self.moved |= !first;
                self.sent = Some(t);
                out.push(Action::Seek(t, first));
            }
        }
        let played = grabbed.unwrap_or(if can { st.time / st.duration } else { 0.0 });
        let over = can && ui.over(hit);
        ui.s.pointer |= over;
        let hot = over || grabbed.is_some();
        track(ui, "seek/grow", l.seek, (played as f32, hot), ui.pal.text);
        if over || grabbed.is_some() {
            let spot = grabbed
                .unwrap_or(f64::from((ui.s.input.pos.0 - l.seek.x) / l.seek.w).clamp(0.0, 1.0));
            let x = l.seek.x + l.seek.w * spot as f32;
            tip(
                ui,
                x,
                l.bar.y,
                &clock(spot * st.duration, st.duration >= 3600.0),
            );
        }
        played * st.duration
    }

    fn volume(&mut self, ui: &mut Ui, r: Rect, st: &State, out: &mut Vec<Action>) {
        let hit = Rect::new(r.x - S4, r.y, r.w + S8, r.h);
        let grabbed = self.drag(ui, Grab::Volume, hit, r);
        if let Some(at) = grabbed {
            if self.sent != Some(at) {
                self.sent = Some(at);
                out.push(Action::Volume(at * 100.0));
            }
            if ui.s.input.released || !ui.s.input.down {
                self.grab = None;
            }
        }
        let level = grabbed.unwrap_or(st.volume / 100.0) as f32;
        let over = ui.over(hit);
        ui.s.pointer |= over;
        let fill = if st.muted { ui.pal.dim } else { ui.pal.text };
        track(
            ui,
            "volume/grow",
            r,
            (level, over || grabbed.is_some()),
            fill,
        );
    }
}

/// A thin capsule across the middle of `r`, `frac` of it filled, a knob on the
/// fill's end while `hot`.
fn track(ui: &mut Ui, name: &str, r: Rect, (frac, hot): (f32, bool), fill: u32) {
    let grow = ui.spring(name, f32::from(hot), SNAP);
    let th = TRACK + 2.0 * grow;
    let cy = r.y + r.h / 2.0;
    let line = Rect::new(r.x, cy - th / 2.0, r.w, th);
    ui.p.rect(line, th / 2.0, ui.pal.selected);
    let end = r.w * frac.clamp(0.0, 1.0);
    ui.p.rect(
        Rect {
            w: end.max(th),
            ..line
        },
        th / 2.0,
        fill,
    );
    if grow > 0.01 {
        let k = KNOB / 2.0 * grow;
        ui.p.rect(Rect::new(r.x + end - k, cy - k, 2.0 * k, 2.0 * k), k, fill);
    }
}

/// The time under the pointer, in a pill above the bar.
fn tip(ui: &mut Ui, x: f32, bar_top: f32, s: &str) {
    let ink = ui.small().color(ui.pal.text);
    let w = clock_width(ui, s, ink) + 2.0 * S8;
    let max_x = ui.p.size().0 - MARGIN - w;
    let r = Rect::new(
        (x - w / 2.0).clamp(MARGIN, max_x.max(MARGIN)),
        bar_top - S8 - TIP_H,
        w,
        TIP_H,
    )
    .round();
    let panel = ui.pal.panel; // the panel color as a capsule
    ui.p.rect(r, TIP_H / 2.0, panel);
    draw_clock(ui, r, s, ink, Align::Center);
}

/// Width of a clock string with every digit in a cell as wide as "0", so the
/// numbers don't shift as they change (ui skill §3: fixed-width slots).
fn clock_width(ui: &mut Ui, s: &str, ink: Ink) -> f32 {
    let cell = ui.s.text.width("0", ink);
    let mut buf = [0u8; 4];
    s.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                cell
            } else {
                ui.s.text.width(c.encode_utf8(&mut buf), ink)
            }
        })
        .sum()
}

fn draw_clock(ui: &mut Ui, r: Rect, s: &str, ink: Ink, align: Align) {
    let cell = ui.s.text.width("0", ink);
    let w = clock_width(ui, s, ink);
    let mut x = match align {
        Align::Left => r.x,
        Align::Center => r.x + (r.w - w) / 2.0,
        Align::Right => r.x + r.w - w,
    };
    let y = ui.s.text.center(r.y, r.h, ink);
    let mut buf = [0u8; 4];
    for c in s.chars() {
        let g = c.encode_utf8(&mut buf);
        let gw = ui.s.text.width(g, ink);
        if c.is_ascii_digit() {
            ui.s.text
                .draw(&mut ui.p, ((x + (cell - gw) / 2.0).round(), y), g, ink);
            x += cell;
        } else {
            ui.s.text.draw(&mut ui.p, (x.round(), y), g, ink);
            x += gw;
        }
    }
}

/// The whole window when there's no picture: black, and why if the file failed.
pub fn blank(ui: &mut Ui, failed: Option<&str>) {
    ui.p.clear(VIDEO_BG);
    let Some(why) = failed else {
        return;
    };
    let (w, h) = ui.p.size();
    let body = ui.body();
    let small = ui.small();
    ui.text(
        Rect::new(0.0, h / 2.0 - CONTROL_H, w, CONTROL_H),
        "Can't play this file",
        body,
        Align::Center,
    );
    ui.text(
        Rect::new(0.0, h / 2.0, w, CONTROL_H),
        why,
        small,
        Align::Center,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats() {
        assert_eq!(clock(0.0, false), "0:00");
        assert_eq!(clock(65.9, false), "1:05");
        assert_eq!(clock(3725.0, true), "1:02:05");
        assert_eq!(clock(f64::NAN, false), "0:00");
        assert_eq!(clock(-3.0, true), "0:00:00");
    }

    #[test]
    fn layout_fits_inside_the_bar_in_order() {
        for w in [480.0, 1280.0, 3840.0] {
            let l = layout(w, 720.0, 40.0, 0.0);
            let row = [l.play, l.cur, l.seek, l.total, l.mute, l.volume, l.full];
            for pair in row.windows(2) {
                assert!(pair[0].x + pair[0].w <= pair[1].x, "overlap at width {w}");
            }
            assert!(l.play.x >= l.bar.x && l.full.x + l.full.w <= l.bar.x + l.bar.w);
            assert!(l.seek.w > 0.0, "no room to seek at {w}");
        }
    }

    #[test]
    fn a_click_seeks_exactly_once_and_a_drag_lands_exactly() {
        let mut ui = frame::ui::State::new(frame::ui::Text::new(frame::ui::theme::FAMILY).unwrap());
        let state = State {
            duration: 100.0,
            ..State::default()
        };
        let look = Look {
            state: &state,
            visible: true,
            fullscreen: false,
        };
        let (w, h) = (1280, 720);
        let mut px = vec![0u32; w * h];
        let mut c = Controls::default();
        let mut frame = |ui: &mut frame::ui::State, c: &mut Controls| {
            let mut f = ui.begin(frame::window::Canvas { px: &mut px, w, h }, 0.0);
            let acts = c.draw(&mut f, &look);
            f.end();
            acts
        };
        let y = h as f32 - MARGIN - BAR_H / 2.0; // the middle of the bar: on the seek track
        ui.input.pos = (640.0, y);
        frame(&mut ui, &mut c);
        // Press and release: one exact seek, at the press.
        ui.input.button(true);
        assert!(matches!(
            frame(&mut ui, &mut c)[..],
            [Action::Seek(_, true)]
        ));
        ui.input.button(false);
        assert!(frame(&mut ui, &mut c).is_empty());
        // Both in one frame: still one exact seek.
        ui.input.button(true);
        ui.input.button(false);
        assert!(matches!(
            frame(&mut ui, &mut c)[..],
            [Action::Seek(_, true)]
        ));
        // Drag: exact press, keyframe steps, exact landing where it let go.
        ui.input.button(true);
        frame(&mut ui, &mut c);
        ui.input.pos.0 = 700.0;
        let [Action::Seek(t, false)] = frame(&mut ui, &mut c)[..] else {
            panic!("a drag step seeks to a keyframe");
        };
        ui.input.button(false);
        assert_eq!(frame(&mut ui, &mut c), [Action::Seek(t, true)]);
    }

    #[test]
    fn zone_covers_the_bar_and_its_pill() {
        let l = layout(1280.0, 720.0, 40.0, 0.0);
        assert!((zone_top(720) as f32) < l.bar.y - S8 - TIP_H);
    }
}
