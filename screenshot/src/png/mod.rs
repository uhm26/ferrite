//! Our own PNG encoder, built for speed: 8-bit RGB, rows cut into stripes that all
//! cores filter and compress at once. Each stripe is one IDAT chunk holding its own
//! run of deflate blocks; one zlib header goes before them, one Adler-32 after.

mod check;
mod deflate;
mod filter;
mod huffman;

use std::io::{self, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Rows per stripe aim for ~8 stripes per thread: rows differ a lot in cost, so small
/// stripes balance better (measured 5.5 vs 6.8 ms for 4 per thread). But ≥16 rows,
/// since every stripe restarts the 32 KiB LZ77 window.
const MIN_ROWS: usize = 16;

/// How the image is cut: `count` stripes of `rows` rows (the last may be shorter).
#[derive(Clone, Copy)]
struct Cut {
    w: u32,
    h: u32,
    rows: usize,
    count: usize,
}

struct Stripe {
    index: usize,
    /// A whole IDAT chunk: length, type, data, CRC.
    chunk: Vec<u8>,
    adler: u32,
    len: usize,
}

/// Encode a `w`×`h` RGB image to `out`. `fill(y, row)` writes row `y` (3·w bytes);
/// it is called from many threads at once.
pub fn encode<F>(w: u32, h: u32, fill: F, out: &mut impl Write) -> io::Result<()>
where
    F: Fn(u32, &mut [u8]) + Sync,
{
    if w == 0 || h == 0 {
        return Err(io::Error::other("empty image"));
    }
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = (h as usize / (threads * 8)).max(MIN_ROWS);
    let count = (h as usize).div_ceil(rows);
    let cut = Cut { w, h, rows, count };
    let next = AtomicUsize::new(0);
    let mut stripes: Vec<Stripe> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.min(count))
            .map(|_| s.spawn(|| work(cut, &next, &fill)))
            .collect();
        workers
            .into_iter()
            .flat_map(|t| t.join().unwrap_or_default())
            .collect()
    });
    if stripes.len() != count {
        return Err(io::Error::other("an encoder thread panicked"));
    }
    stripes.sort_unstable_by_key(|s| s.index);
    let adler = stripes
        .iter()
        .fold(1, |a, s| check::adler32_combine(a, s.adler, s.len));

    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&w.to_be_bytes());
    ihdr[4..8].copy_from_slice(&h.to_be_bytes());
    ihdr[8..10].copy_from_slice(&[8, 2]); // 8 bits, RGB; deflate, filter 0, no interlace
    let mut head = b"\x89PNG\r\n\x1a\n".to_vec();
    head.extend(chunk(b"IHDR", &ihdr));
    head.extend(chunk(b"IDAT", &[0x78, 0x01])); // zlib header: deflate, 32 KiB window
    out.write_all(&head)?;
    for s in &stripes {
        out.write_all(&s.chunk)?;
    }
    let mut tail = [0x03, 0x00, 0, 0, 0, 0]; // final empty fixed block, then Adler-32
    tail[2..].copy_from_slice(&adler.to_be_bytes());
    let mut end = chunk(b"IDAT", &tail);
    end.extend(chunk(b"IEND", &[]));
    out.write_all(&end)?;
    Ok(())
}

/// One thread: take stripes until none are left.
fn work<F>(cut: Cut, next: &AtomicUsize, fill: &F) -> Vec<Stripe>
where
    F: Fn(u32, &mut [u8]),
{
    let Cut { w, h, rows, count } = cut;
    let row_len = w as usize * 3;
    let mut f = filter::Filter::new(row_len);
    let mut z = deflate::Deflater::new();
    let mut done = Vec::new();
    loop {
        let index = next.fetch_add(1, Ordering::Relaxed);
        if index >= count {
            return done;
        }
        let y0 = index * rows;
        let y1 = (y0 + rows).min(h as usize);
        f.start(y0, |y, row| fill(y as u32, row));
        for y in y0..y1 {
            f.row(|row| fill(y as u32, row));
        }
        let data = f.data();
        let mut chunk = vec![0; 8];
        z.stripe(data, &mut chunk);
        seal(&mut chunk, b"IDAT");
        let (adler, len) = (check::adler32(data), data.len());
        done.push(Stripe {
            index,
            chunk,
            adler,
            len,
        });
    }
}

/// Fill in the length and type at the front of `c` and append the CRC.
fn seal(c: &mut Vec<u8>, kind: &[u8; 4]) {
    let len = (c.len() - 8) as u32;
    c[..4].copy_from_slice(&len.to_be_bytes());
    c[4..8].copy_from_slice(kind);
    let crc = check::crc32(&c[4..]);
    c.extend_from_slice(&crc.to_be_bytes());
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = vec![0; 8];
    c.extend_from_slice(data);
    seal(&mut c, kind);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inflate the concatenated IDAT data with gzip, unfilter, and compare every row.
    #[test]
    fn whole_image_roundtrips() {
        let (w, h) = (700u32, 300u32);
        let px = |x: u32, y: u32| -> [u8; 3] {
            let n = x.wrapping_mul(2_654_435_761u32.wrapping_mul(y + 1)) >> 13;
            match (x / 50 + y / 40) % 3 {
                0 => [10, 20, 30],                 // flat
                1 => [x as u8, y as u8, 7],        // gradient
                _ => [n as u8, (n >> 8) as u8, 1], // noise
            }
        };
        let fill = |y: u32, row: &mut [u8]| {
            for x in 0..w {
                row[x as usize * 3..][..3].copy_from_slice(&px(x, y));
            }
        };
        let mut png = Vec::new();
        encode(w, h, fill, &mut png).unwrap();
        let mut z = Vec::new();
        let mut at = 8;
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            if &png[at + 4..at + 8] == b"IDAT" {
                z.extend_from_slice(&png[at + 8..at + 8 + len]);
            }
            at += 12 + len;
        }
        let (raw, _) = deflate::tests::gunzip(&z[2..z.len() - 4], None);
        assert_eq!(check::adler32(&raw).to_be_bytes(), z[z.len() - 4..]);
        let stride = w as usize * 3 + 1;
        assert_eq!(raw.len(), stride * h as usize);
        let mut prev = vec![0u8; stride - 1];
        for y in 0..h {
            let line = &raw[y as usize * stride..][..stride];
            let got = filter::tests::unfilter(line[0], &line[1..], &prev);
            let mut want = vec![0u8; stride - 1];
            fill(y, &mut want);
            assert!(got == want, "row {y} differs (filter {})", line[0]);
            prev = got;
        }
    }
}

#[cfg(test)]
mod bench {
    /// Encode speed on a real screen: `SNAP_RAW` is a 3840x1080 rgb24 dump, made with
    /// `ffmpeg -i shot.png -f rawvideo -pix_fmt rgb24 shot.raw`. Run with
    /// `cargo test --release bench_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_real() {
        let real = std::fs::read(std::env::var("SNAP_RAW").unwrap()).unwrap();
        let fill =
            |y: u32, row: &mut [u8]| row.copy_from_slice(&real[y as usize * 11520..][..11520]);
        let mut ts = Vec::new();
        for _ in 0..40 {
            let t = std::time::Instant::now();
            let mut out = Vec::with_capacity(4 << 20);
            super::encode(3840, 1080, fill, &mut out).unwrap();
            ts.push(t.elapsed().as_micros());
        }
        ts.sort_unstable();
        eprintln!("median {} us, p25 {} us", ts[20], ts[10]);
    }
}
