//! Minimal PNG writer for headless screenshots (`--png` in apps): checking a look
//! without a screen. Copied from ../term/examples/snapshot.rs at term 7be66f5.

use std::io::Write;

/// Write `px` (`w` × `h`, ARGB) as an RGB PNG: one IDAT of stored (uncompressed)
/// deflate blocks. Big files, zero dependencies.
pub fn write(path: &str, w: usize, h: usize, px: &[u32]) -> std::io::Result<()> {
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
