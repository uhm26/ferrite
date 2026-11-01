//! Huffman code lengths with a length limit, and canonical codes (RFC 1951 3.2.2).

/// Code lengths for `freq` into `out`, none longer than `max`; unused symbols get 0.
/// At least two symbols get a code, so the code is always complete.
pub fn lengths(freq: &[u32], max: u8, out: &mut [u8]) {
    out.fill(0);
    let mut syms: Vec<(u32, u16)> = (0..freq.len())
        .filter(|&s| freq[s] > 0)
        .map(|s| (freq[s], s as u16))
        .collect();
    for (s, _) in freq.iter().enumerate().filter(|&(_, &f)| f == 0) {
        if syms.len() >= 2 {
            break;
        }
        syms.push((1, s as u16));
    }
    syms.sort_unstable();
    let mut count = depth_histogram(&syms);
    limit(&mut count, usize::from(max));
    // Least frequent symbols take the longest codes.
    let mut it = syms.iter();
    for len in (1..=usize::from(max)).rev() {
        for &(_, s) in it.by_ref().take(count[len] as usize) {
            out[usize::from(s)] = len as u8;
        }
    }
}

/// How many leaves sit at each depth of an optimal Huffman tree over `syms` (sorted
/// by frequency). Two-queue construction: leaves and new nodes both come out sorted.
fn depth_histogram(syms: &[(u32, u16)]) -> Vec<u32> {
    let n = syms.len();
    let mut weight: Vec<u64> = syms.iter().map(|&(f, _)| u64::from(f)).collect();
    weight.resize(2 * n - 1, 0);
    let mut parent = vec![0usize; 2 * n - 1];
    let (mut leaf, mut node) = (0, n);
    for k in n..2 * n - 1 {
        for _ in 0..2 {
            let take_leaf = leaf < n && (node >= k || weight[leaf] <= weight[node]);
            let pick = if take_leaf { &mut leaf } else { &mut node };
            weight[k] += weight[*pick];
            parent[*pick] = k;
            *pick += 1;
        }
    }
    // Parents have higher indices, so walking down from the root sees them first.
    let mut depth = vec![0u32; 2 * n - 1];
    let mut count = vec![0u32; n + 1];
    for k in (0..2 * n - 2).rev() {
        depth[k] = depth[parent[k]] + 1;
        if k < n {
            count[depth[k] as usize] += 1;
        }
    }
    count
}

/// Clamp depths to `max`, then fix the Kraft sum (miniz's enforce_max_code_size).
fn limit(count: &mut Vec<u32>, max: usize) {
    count.resize(count.len().max(max + 1), 0);
    let over: u32 = count[max + 1..].iter().sum();
    count[max] += over;
    count.truncate(max + 1);
    let mut total: u32 = (1..=max).map(|i| count[i] << (max - i)).sum();
    while total != 1 << max {
        count[max] -= 1;
        if let Some(i) = (1..max).rev().find(|&i| count[i] != 0) {
            count[i] -= 1;
            count[i + 1] += 2;
        }
        total -= 1;
    }
}

/// Canonical codes for `lens`, bit-reversed because deflate writes them LSB first.
pub fn codes(lens: &[u8], out: &mut [u16]) {
    let mut bl_count = [0u16; 16];
    for &l in lens {
        bl_count[usize::from(l)] += 1;
    }
    bl_count[0] = 0;
    let mut next = [0u16; 16];
    let mut code = 0u16;
    for bits in 1..16 {
        code = (code + bl_count[bits - 1]) << 1;
        next[bits] = code;
    }
    for (s, &l) in lens.iter().enumerate() {
        if l > 0 {
            let c = next[usize::from(l)];
            next[usize::from(l)] += 1;
            out[s] = c.reverse_bits() >> (16 - l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kraft(lens: &[u8], max: u8) -> u32 {
        lens.iter()
            .filter(|&&l| l > 0)
            .map(|&l| 1 << (max - l))
            .sum()
    }

    #[test]
    fn limited_and_complete() {
        // Fibonacci frequencies give the deepest trees.
        let mut f = vec![1u32, 1];
        while f.len() < 30 {
            f.push(f[f.len() - 1] + f[f.len() - 2]);
        }
        let mut lens = [0u8; 30];
        lengths(&f, 7, &mut lens);
        assert!(lens.iter().all(|&l| (1..=7).contains(&l)));
        assert_eq!(kraft(&lens, 7), 1 << 7);
    }

    #[test]
    fn one_symbol_still_gets_two_codes() {
        let mut lens = [0u8; 4];
        lengths(&[0, 0, 5, 0], 15, &mut lens);
        assert_eq!(lens, [1, 0, 1, 0]);
    }

    #[test]
    fn canonical_example_from_rfc() {
        // RFC 1951 3.2.2: lengths (3,3,3,3,3,2,4,4) give 010,011,100,101,110,00,1110,1111.
        let lens = [3, 3, 3, 3, 3, 2, 4, 4];
        let mut c = [0u16; 8];
        codes(&lens, &mut c);
        let want = [0b010, 0b011, 0b100, 0b101, 0b110, 0b00, 0b1110, 0b1111];
        for i in 0..8 {
            assert_eq!(c[i].reverse_bits() >> (16 - lens[i]), want[i]);
        }
    }
}
