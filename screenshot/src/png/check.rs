//! The two checksums PNG needs: CRC-32 per chunk (over compressed bytes, so small)
//! and zlib's Adler-32 over all filtered bytes. Adler-32 of stripes is combined, so
//! every thread sums only its own stripe.

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC: [u32; 256] = crc_table();

pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |c, &b| {
        CRC[((c ^ u32::from(b)) & 0xff) as usize] ^ (c >> 8)
    })
}

const BASE: u32 = 65521;
/// Most bytes summed before `b` could overflow u32 (zlib's NMAX).
const NMAX: usize = 5552;

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(NMAX / 32 * 32) {
        // 32 bytes at a time: b gains 32·a plus a weighted sum, which vectorizes.
        let (blocks, rest) = chunk.as_chunks::<32>();
        for blk in blocks {
            let (mut s1, mut s2) = (0u32, 0u32);
            for (k, &x) in blk.iter().enumerate() {
                s1 += u32::from(x);
                s2 += (32 - k as u32) * u32::from(x);
            }
            b += 32 * a + s2;
            a += s1;
        }
        for &x in rest {
            a += u32::from(x);
            b += a;
        }
        a %= BASE;
        b %= BASE;
    }
    (b << 16) | a
}

/// Adler-32 of `A ++ B` from adler(A), adler(B) and len(B) (zlib's adler32_combine).
pub fn adler32_combine(a1: u32, a2: u32, len2: usize) -> u32 {
    let rem = (len2 % BASE as usize) as u32;
    let s1 = a1 & 0xffff;
    let s2 = (rem * s1) % BASE;
    let s1 = (s1 + (a2 & 0xffff) + BASE - 1) % BASE;
    let s2 = (s2 + (a1 >> 16) + (a2 >> 16) + BASE - rem) % BASE;
    (s2 << 16) | s1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn combine_matches_whole() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i * 7 + i / 13) as u8).collect();
        for cut in [0, 1, 5552, 9999, 20_000] {
            let (x, y) = data.split_at(cut);
            assert_eq!(
                adler32_combine(adler32(x), adler32(y), y.len()),
                adler32(&data)
            );
        }
    }
}
