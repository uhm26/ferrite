//! A fast deflate (RFC 1951) for filtered PNG rows: greedy LZ77 with a one-entry hash
//! table, then one dynamic-Huffman block per 64 Ki tokens. A stripe ends with a sync
//! flush (empty stored block), so stripes from different threads simply concatenate.

use super::huffman;

const WINDOW: usize = 32 * 1024;
const MAX_MATCH: usize = 258;
const HASH_BITS: u32 = 15;
const BLOCK_TOKENS: usize = 1 << 16;
/// Token: `L << 16 | dist`. L is the literal byte (dist 0) or 256 + match length, so
/// literals and matches take the same branch-free path through the encoder.
const LIT_END: u32 = 256;

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Symbol 30 is ours, not deflate's: "no distance" for literals, written as 0 bits.
const DIST_BASE: [u16; 31] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 0,
];
const DIST_EXTRA: [u8; 31] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13, 0,
];
/// Order the code-length code lengths are sent in (RFC 1951 3.2.7).
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Match length → length symbol index (0..29).
static LEN_SYM: [u8; 259] = {
    let mut t = [0u8; 259];
    let mut s = 0;
    while s < 29 {
        let mut l = LEN_BASE[s] as usize;
        while l < LEN_BASE[s] as usize + (1 << LEN_EXTRA[s]) && l < 259 {
            t[l] = s as u8; // 258 is overwritten by symbol 28, as the RFC wants
            l += 1;
        }
        s += 1;
    }
    t
};

/// Distance symbol: `DIST_LO[d]` for d ≤ 256 (0 → 30), else `DIST_HI[(d - 1) >> 7]`.
static DIST_LO: [u8; 257] = dist_table(false);
static DIST_HI: [u8; 257] = dist_table(true);

const fn dist_table(hi: bool) -> [u8; 257] {
    let mut t = [0u8; 257];
    t[0] = 30;
    let mut s = 0;
    while s < 30 {
        let mut d = DIST_BASE[s] as usize;
        while d < DIST_BASE[s] as usize + (1 << DIST_EXTRA[s]) {
            match (hi, d <= 256) {
                (false, true) => t[d] = s as u8,
                (true, false) => t[(d - 1) >> 7] = s as u8,
                _ => {}
            }
            d += 1;
        }
        s += 1;
    }
    t
}

#[inline(always)]
fn dist_sym(d: usize) -> usize {
    // Both lookups, then a select: no branch to mispredict.
    let lo = DIST_LO[d.min(256)];
    let hi = DIST_HI[(d.saturating_sub(1) >> 7).min(256)];
    usize::from(if d <= 256 { lo } else { hi })
}

/// LSB-first bit writer without branches: every `put` stores 8 bytes at `pos` and
/// advances by the bytes completed. `room` must make space before a run of puts.
struct Bits<'a> {
    out: &'a mut Vec<u8>,
    pos: usize,
    acc: u64,
    n: u32,
}

impl<'a> Bits<'a> {
    fn new(out: &'a mut Vec<u8>) -> Self {
        let pos = out.len();
        Self {
            out,
            pos,
            acc: 0,
            n: 0,
        }
    }

    /// Make space for `bytes` more output (plus the 8-byte store).
    fn room(&mut self, bytes: usize) {
        let need = self.pos + bytes + 8;
        if self.out.len() < need {
            self.out.resize(need.max(self.out.len() * 2), 0);
        }
    }

    /// `len` ≤ 56 bits of `v` (no bits above `len` set).
    #[inline(always)]
    fn put(&mut self, v: u64, len: u32) {
        self.acc |= v << self.n;
        self.n += len;
        self.out[self.pos..self.pos + 8].copy_from_slice(&self.acc.to_le_bytes());
        let done = self.n / 8;
        self.pos += done as usize;
        self.acc >>= done * 8;
        self.n -= done * 8;
    }

    /// Pad to a byte boundary; the output ends there.
    fn finish(self) {
        let end = self.pos + usize::from(self.n > 0);
        self.out.truncate(end);
    }
}

/// Per-thread compressor state, reused across stripes.
pub struct Deflater {
    table: Vec<u32>,
    tokens: Vec<u32>,
}

impl Deflater {
    pub fn new() -> Self {
        Self {
            table: vec![0; 1 << HASH_BITS],
            tokens: Vec::with_capacity(BLOCK_TOKENS + 256), // a skip step can overshoot ~70
        }
    }

    /// Append `data` as non-final blocks plus a sync flush.
    pub fn stripe(&mut self, data: &[u8], out: &mut Vec<u8>) {
        self.table.fill(0);
        let mut bits = Bits::new(out);
        let mut i = 0;
        while i < data.len() {
            self.tokens.clear();
            i = self.lz77(data, i);
            // Worst case 48 bits per token, plus the block header.
            bits.room(self.tokens.len() * 6 + 512);
            write_block(&self.tokens, &mut bits);
        }
        bits.room(1);
        bits.put(0, 3); // empty stored block: BFINAL 0, BTYPE 00, then LEN/NLEN
        bits.finish();
        out.extend_from_slice(&[0, 0, 0xff, 0xff]);
    }

    /// Tokenize from `i` until the block is full or the data ends; returns the new `i`.
    fn lz77(&mut self, data: &[u8], mut i: usize) -> usize {
        let n = data.len();
        // Misses in a row: in noise (photos) we hash ever fewer positions, like LZ4.
        let mut misses = 0usize;
        while i + 8 <= n && self.tokens.len() < BLOCK_TOKENS {
            let v = read32(data, i);
            let h = (v.wrapping_mul(0x1e35_a7bd) >> (32 - HASH_BITS)) as usize;
            let cand = self.table[h] as usize;
            self.table[h] = i as u32;
            if cand < i && i - cand <= WINDOW && read32(data, cand) == v {
                let max = (n - i).min(MAX_MATCH);
                let len = 4 + common(data, cand + 4, i + 4, max - 4);
                self.tokens
                    .push((LIT_END + len as u32) << 16 | (i - cand) as u32);
                i += len;
                misses = 0;
            } else {
                let step = (1 + (misses >> 5)).min(n - i);
                self.tokens
                    .extend(data[i..i + step].iter().map(|&b| u32::from(b) << 16));
                i += step;
                misses += 1;
            }
        }
        if i + 8 > n {
            self.tokens
                .extend(data[i..].iter().map(|&b| u32::from(b) << 16));
            i = n;
        }
        i
    }
}

#[inline(always)]
fn read32(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

/// How many bytes at `a` and `b` agree, up to `max` (`b` > `a`, `b + max` ≤ len).
#[inline(always)]
fn common(d: &[u8], a: usize, b: usize, max: usize) -> usize {
    let mut k = 0;
    while k + 8 <= max {
        let x = u64::from_le_bytes(d[a + k..a + k + 8].try_into().unwrap_or_default());
        let y = u64::from_le_bytes(d[b + k..b + k + 8].try_into().unwrap_or_default());
        if x != y {
            return k + ((x ^ y).trailing_zeros() / 8) as usize;
        }
        k += 8;
    }
    while k < max && d[a + k] == d[b + k] {
        k += 1;
    }
    k
}

/// The two codes of one block: literal/length (286) and distance (30, plus our 30).
struct Codes {
    ll: [u8; 286],
    lc: [u16; 286],
    dl: [u8; 31],
    dc: [u16; 31],
}

/// Token L (literal or 256 + length) → literal/length symbol.
#[inline(always)]
fn ll_sym(l: usize) -> usize {
    if l < 256 {
        l
    } else {
        257 + usize::from(LEN_SYM[(l - 256).min(258)])
    }
}

fn write_block(tokens: &[u32], bits: &mut Bits) {
    // Four histograms, summed after: runs of one symbol (zeros) would otherwise wait
    // on the same counter every step.
    let mut lf4 = [[0u32; 286]; 4];
    let mut df4 = [[0u32; 31]; 4];
    for (k, &t) in tokens.iter().enumerate() {
        lf4[k & 3][ll_sym((t >> 16) as usize)] += 1;
        df4[k & 3][dist_sym((t & 0xffff) as usize)] += 1;
    }
    let lf: [u32; 286] = std::array::from_fn(|s| lf4.iter().map(|h| h[s]).sum());
    let df: [u32; 31] = std::array::from_fn(|s| df4.iter().map(|h| h[s]).sum());
    let mut lf = lf;
    lf[256] = 1;
    let mut c = Codes {
        ll: [0; 286],
        lc: [0; 286],
        dl: [0; 31],
        dc: [0; 31],
    };
    huffman::lengths(&lf, 15, &mut c.ll);
    huffman::lengths(&df[..30], 15, &mut c.dl[..30]);
    huffman::codes(&c.ll, &mut c.lc);
    huffman::codes(&c.dl, &mut c.dc);
    write_header(&c, bits);
    // Per L: the code with its extra bits already appended (≤ 15 + 5 bits).
    let mut lt = [(0u32, 0u32); 256 + 259];
    for (l, e) in lt
        .iter_mut()
        .enumerate()
        .filter(|&(l, _)| !(256..259).contains(&l))
    {
        let s = ll_sym(l);
        let (code, n) = (u32::from(c.lc[s]), u32::from(c.ll[s]));
        *e = match l.checked_sub(256) {
            Some(len) => {
                let k = s - 257;
                let extra = (len - usize::from(LEN_BASE[k])) as u32;
                (code | extra << n, n + u32::from(LEN_EXTRA[k]))
            }
            None => (code, n),
        };
    }
    // One write per token: length part ≤ 20 bits, distance part ≤ 28 bits.
    for &t in tokens {
        let (lcode, ln) = lt[(t >> 16) as usize];
        let d = (t & 0xffff) as usize;
        let s = dist_sym(d);
        let (dcode, dn) = (u32::from(c.dc[s]), u32::from(c.dl[s]));
        let dpart = dcode | (d as u32 - u32::from(DIST_BASE[s])) << dn;
        let dlen = dn + u32::from(DIST_EXTRA[s]);
        bits.put(u64::from(lcode) | u64::from(dpart) << ln, ln + dlen);
    }
    bits.put(u64::from(c.lc[256]), u32::from(c.ll[256]));
}

/// Block header: BFINAL 0, BTYPE 10, then both code length tables, run-length coded.
fn write_header(c: &Codes, bits: &mut Bits) {
    let hlit = 257
        + c.ll[257..]
            .iter()
            .rposition(|&l| l > 0)
            .map_or(0, |p| p + 1);
    let hdist = 1 + c.dl.iter().rposition(|&l| l > 0).unwrap_or(0);
    let lens: Vec<u8> = c.ll[..hlit].iter().chain(&c.dl[..hdist]).copied().collect();
    let runs = rle(&lens);
    let mut cf = [0u32; 19];
    runs.iter().for_each(|&(s, _, _)| cf[usize::from(s)] += 1);
    let (mut cl, mut cc) = ([0u8; 19], [0u16; 19]);
    huffman::lengths(&cf, 7, &mut cl);
    huffman::codes(&cl, &mut cc);
    let hclen = 4.max(1 + CL_ORDER.iter().rposition(|&s| cl[s] > 0).unwrap_or(0));
    bits.put(0b100, 3);
    bits.put((hlit - 257) as u64, 5);
    bits.put((hdist - 1) as u64, 5);
    bits.put((hclen - 4) as u64, 4);
    for &s in &CL_ORDER[..hclen] {
        bits.put(u64::from(cl[s]), 3);
    }
    for (s, extra, n) in runs {
        bits.put(u64::from(cc[usize::from(s)]), u32::from(cl[usize::from(s)]));
        bits.put(u64::from(extra), u32::from(n));
    }
}

/// Code lengths as (symbol, extra value, extra bits): 16 repeats the previous length
/// 3–6 times, 17 and 18 are runs of 3–10 and 11–138 zeros.
fn rle(lens: &[u8]) -> Vec<(u8, u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lens.len() {
        let l = lens[i];
        let run = lens[i..].iter().take_while(|&&x| x == l).count();
        let repeat = i > 0 && lens[i - 1] == l;
        let (sym, n, base, nbits) = match (l, run) {
            (0, 11..) => (18, run.min(138), 11, 7),
            (0, 3..) => (17, run.min(10), 3, 3),
            (_, 3..) if repeat => (16, run.min(6), 3, 2),
            _ => (l, 1, 1, 0),
        };
        out.push((sym, (n - base) as u8, nbits));
        i += n;
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Inflate a complete raw deflate stream (ending in a final block) with the
    /// system's gzip. Returns the data and whether gzip's CRC/size check passed;
    /// `orig` (if given) supplies the CRC and size for that check.
    pub fn gunzip(deflate: &[u8], orig: Option<&[u8]>) -> (Vec<u8>, bool) {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
        gz.extend_from_slice(deflate);
        let o = orig.unwrap_or_default();
        gz.extend_from_slice(&crate::png::check::crc32(o).to_le_bytes());
        gz.extend_from_slice(&(o.len() as u32).to_le_bytes());
        let mut p = Command::new("gzip")
            .arg("-dc")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("gzip");
        let mut stdin = p.stdin.take().unwrap();
        // Write from another thread: gzip blocks on a full stdout pipe otherwise.
        let writer = std::thread::spawn(move || stdin.write_all(&gz));
        let out = p.wait_with_output().unwrap();
        writer.join().unwrap().unwrap();
        (out.stdout, out.status.success())
    }

    fn roundtrip(data: &[u8]) {
        let mut d = Deflater::new();
        let mut out = Vec::new();
        // Two stripes back to back, like two threads' output concatenated.
        let (a, b) = data.split_at(data.len() / 3);
        d.stripe(a, &mut out);
        d.stripe(b, &mut out);
        out.extend_from_slice(&[0x03, 0x00]); // final empty fixed block
        let (got, ok) = gunzip(&out, Some(data));
        assert!(ok, "gzip rejected the stream");
        assert!(got == data, "inflated data differs");
    }

    #[test]
    fn roundtrips() {
        roundtrip(b"");
        roundtrip(b"abc");
        roundtrip(&[0u8; 100_000]);
        let mut x = 1u32;
        let noise: Vec<u8> = (0..300_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect();
        roundtrip(&noise);
        // Text-like: few symbols, many repeats at all distances.
        let text: Vec<u8> = (0..400_000u32)
            .map(|i| b"snap  fast\n"[(i.wrapping_mul(i) % 11) as usize])
            .collect();
        roundtrip(&text);
        // Screenshot-like: mostly zeros, rare other bytes, deep Huffman trees.
        let skew: Vec<u8> = (0..900_000u32)
            .map(|i| {
                if i.wrapping_mul(2_654_435_761) % 97 < 3 {
                    (i % 251) as u8
                } else {
                    0
                }
            })
            .collect();
        roundtrip(&skew);
    }

    #[test]
    fn rle_runs() {
        let mut lens = vec![0u8; 140];
        lens.extend([5, 5, 5, 5, 5, 5, 5, 5, 0, 0, 0, 7]);
        let r = rle(&lens);
        let expand: usize = r
            .iter()
            .map(|&(s, e, _)| match s {
                16 => usize::from(e) + 3,
                17 => usize::from(e) + 3,
                18 => usize::from(e) + 11,
                _ => 1,
            })
            .sum();
        assert_eq!(expand, lens.len());
        assert_eq!(r[0], (18, 127, 7));
    }

    #[test]
    fn symbol_tables() {
        assert_eq!(LEN_SYM[3], 0);
        assert_eq!(LEN_SYM[258], 28);
        assert_eq!(LEN_SYM[257], 27);
        assert_eq!(dist_sym(0), 30);
        assert_eq!(dist_sym(1), 0);
        assert_eq!(dist_sym(256), 15);
        assert_eq!(dist_sym(257), 16);
        assert_eq!(dist_sym(32768), 29);
    }
}
