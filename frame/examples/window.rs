//! Smallest frame app: a dark window that shows the last key typed. Esc closes.
//! `cargo run --example window`

use frame::font::{self, Font};
use frame::sys::{self, POLLIN, PollFd};
use frame::window::{Canvas, Event, Rect, Window};
use frame::wl::Error;
use std::time::Instant;

const BG: u32 = 0xff_10_10_12;
const FG: u32 = 0xff_e6_e6_e6;
const ESCAPE: u32 = 0xff1b; // XKB_KEY_Escape

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut win: Window = Window::new("frame", "frame-example", true)?;
    let mut font = Font::new("sans", 16.0)?;
    let (w, h) = win.configured_size();
    win.set_size(if w == 0 { 480 } else { w }, if h == 0 { 160 } else { h });
    let mut text = String::from("type something (Esc quits)");
    let mut dirty = true;
    loop {
        while let Some(ev) = win.next_event() {
            match ev {
                Event::Close => return Ok(()),
                Event::Key(k) if k.sym == ESCAPE => return Ok(()),
                Event::Key(k) => text = format!("{} {:#x} {:?}", k.text, k.sym, k.mods),
                Event::Resize(w, h) if w > 0 && h > 0 => win.set_size(w, h),
                _ => continue,
            }
            dirty = true;
        }
        if dirty && !win.frame_pending() && draw(&mut win, &mut font, &text)? {
            dirty = false;
        }
        wait(&mut win)?;
    }
}

/// Block until the compositor sends something or key repeat is due.
fn wait(win: &mut Window) -> Result<(), Error> {
    win.flush()?;
    let timeout = win.next_deadline().map_or(-1, |at| {
        at.saturating_duration_since(Instant::now()).as_millis() as i32
    });
    let mut fds = [PollFd::new(win.fd(), POLLIN)];
    sys::wait(&mut fds, timeout)?;
    win.dispatch()?;
    win.tick(Instant::now());
    Ok(())
}

/// Full redraw into a free buffer. False if none is free yet.
fn draw(win: &mut Window, font: &mut Font, text: &str) -> Result<bool, Error> {
    let Some(i) = win.acquire()? else {
        return Ok(false);
    };
    let (mut c, ()) = win.canvas(i);
    c.px.fill(BG);
    let (asc, _) = font.face(font::REGULAR).metrics();
    let mut x = 24i32;
    for ch in text.chars() {
        let face = font.face_for(ch, font::REGULAR);
        if let Some((g, advance)) = face.render(ch) {
            blit(&mut c, &g, x, 24 + (asc / 64) as i32);
            x += ((advance + 32) / 64) as i32;
        }
    }
    let all = Rect {
        x: 0,
        y: 0,
        w: c.w,
        h: c.h,
    };
    win.present(i, &[all])?;
    Ok(true)
}

/// Blend a coverage bitmap in FG onto the canvas, its baseline at `y`.
fn blit(c: &mut Canvas, g: &font::Rendered, x: i32, y: i32) {
    for row in 0..g.rows {
        let dy = y - g.top + row as i32;
        for col in 0..g.width {
            let dx = x + g.left + col as i32;
            let a = u32::from(g.bitmap[row * g.pitch + col]);
            if a == 0 || dx < 0 || dy < 0 || dx as usize >= c.w || dy as usize >= c.h {
                continue;
            }
            let p = &mut c.px[dy as usize * c.w + dx as usize];
            *p = mix(*p, a);
        }
    }
}

/// FG over the pixel at coverage `a` (0-255), per channel.
fn mix(bg: u32, a: u32) -> u32 {
    let ch = |s: u32| {
        let (f, b) = ((FG >> s) & 0xff, (bg >> s) & 0xff);
        ((f * a + b * (255 - a) + 127) / 255) << s
    };
    0xff00_0000 | ch(16) | ch(8) | ch(0)
}
