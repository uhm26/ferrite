//! Render a sample screen to a PNG without a window, to check the renderer by eye.
//! `cargo run --release --example snapshot -- out.png [file-with-vt-bytes]`
//! Also prints how long a full redraw and a no-change redraw take.

use std::io::Write;
use std::time::Instant;

use term::config::Config;
use term::font::Font;
use term::render::draw::Canvas;
use term::render::{Renderer, Selection, Shown, View};
use term::term::Term;

const SAMPLE: &str = "\x1b[1mterm\x1b[0m snapshot  \x1b[2mdim\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munder\x1b[0m \x1b[4:3mcurly\x1b[0m \x1b[9mstrike\x1b[0m\r\n\
\x1b[31mred \x1b[32mgreen \x1b[33myellow \x1b[34mblue \x1b[35mmagenta \x1b[36mcyan \x1b[90mgray\x1b[0m\r\n\
\x1b[38;2;255;128;0mtruecolor\x1b[0m \x1b[48;5;236m 256-bg \x1b[0m \x1b[7mreverse\x1b[0m wide: 漢字 ok\r\n\
╭──────────────╮ ┌─┬─┐ ━━┳━━ ▁▂▃▄▅▆▇█ ░▒▓\r\n\
│ box drawing  │ ├─┼─┤   ┃   ▖▗▘▝▙▚▛▜▞▟\r\n\
╰──────────────╯ └─┴─┘ ━━┻━━ ▏▎▍▌▋▊▉\r\n\
\x1b[30;40m  \x1b[41m  \x1b[42m  \x1b[43m  \x1b[44m  \x1b[45m  \x1b[46m  \x1b[47m  \x1b[100m  \x1b[101m  \x1b[102m  \x1b[103m  \x1b[104m  \x1b[105m  \x1b[106m  \x1b[107m  \x1b[0m selected text here\r\n\
fn main() { println!(\"{}\", 0x2a); } \x1b[90m// == != -> => <=\x1b[0m\r\n\
~/projects/term \x1b[33mmain\x1b[0m ❯ ";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).map_or("snapshot.png", String::as_str);
    let input = match args.get(2) {
        Some(p) => std::fs::read(p).expect("read input"),
        None => SAMPLE.as_bytes().to_vec(),
    };
    term::sys::init_locale();
    let cfg = Config::default();
    let font = Font::new(&cfg.font, cfg.font_size).expect("font");
    let mut r = Renderer::new(font, cfg.padding, cfg.theme);
    r.top = r.bar_height();
    let (w, h) = (900, 260);
    let (cols, rows) = r.grid_for(w, h);
    let mut t = Term::new(cols, rows, 1000, cfg.palette());
    t.feed(&input);
    let mut px = vec![0u32; w * h];
    let mut shown = Shown::default();
    let tabs = ["~/projects/term".to_string(), "build server▏".into(), "claude".into()];
    let view = View {
        focused: true,
        selection: Some(Selection {
            start: (6, 50),
            end: (6, 67),
        }),
        tabs: &tabs,
        active_tab: 1,
    };
    let mut canvas = Canvas { px: &mut px, w, h };
    let t0 = Instant::now();
    r.draw(&t, &view, &mut canvas, &mut shown);
    let full = t0.elapsed();
    let t1 = Instant::now();
    let damage = r.draw(&t, &view, &mut canvas, &mut shown);
    let idle = t1.elapsed();
    shown.invalidate();
    let t2 = Instant::now();
    r.draw(&t, &view, &mut canvas, &mut shown);
    let warm = t2.elapsed();
    println!(
        "cell {}x{}, grid {cols}x{rows}, first draw {full:?} (cold glyphs), warm full draw {warm:?}, no-change draw {idle:?}, damage {}",
        r.font.cell_w,
        r.font.cell_h,
        damage.len()
    );
    write_png(out, w, h, &px).expect("write png");
}

/// Minimal PNG: RGB, one IDAT with stored (uncompressed) deflate blocks.
fn write_png(path: &str, w: usize, h: usize, px: &[u32]) -> std::io::Result<()> {
    let mut raw = Vec::with_capacity(h * (1 + 3 * w));
    for row in px.chunks(w) {
        raw.push(0);
        for p in row {
            raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let n = b.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut f = std::fs::File::create(path)?;
    f.write_all(b"\x89PNG\r\n\x1a\n")?;
    for (kind, data) in [(b"IHDR", &ihdr), (b"IDAT", &z), (b"IEND", &Vec::new())] {
        f.write_all(&(data.len() as u32).to_be_bytes())?;
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        f.write_all(&body)?;
        f.write_all(&crc32(&body).to_be_bytes())?;
    }
    Ok(())
}

fn adler32(d: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in d {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

fn crc32(d: &[u8]) -> u32 {
    let mut c = !0u32;
    for &x in d {
        c ^= u32::from(x);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}
