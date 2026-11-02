//! PNG row filters (spec 9.2) for RGB, 3 bytes per pixel. Each row tries None, Sub,
//! Up and Paeth and keeps the one with the smallest sum of |byte as i8|, the usual
//! heuristic (libpng's): small values compress best.

const BPP: usize = 3;

pub struct Filter {
    prev: Vec<u8>,
    cur: Vec<u8>,
    cand: [Vec<u8>; 3],
    out: Vec<u8>,
}

impl Filter {
    pub fn new(row_len: usize) -> Self {
        let z = vec![0; row_len];
        Self {
            prev: z.clone(),
            cur: z.clone(),
            cand: [z.clone(), z.clone(), z],
            out: Vec::new(),
        }
    }

    /// Begin a stripe at row `y0`: the row above it is the Up/Paeth reference.
    pub fn start(&mut self, y0: usize, fill: impl FnOnce(usize, &mut [u8])) {
        self.out.clear();
        match y0 {
            0 => self.prev.fill(0),
            _ => fill(y0 - 1, &mut self.prev),
        }
    }

    /// Filter the next row, which `fill` writes.
    pub fn row(&mut self, fill: impl FnOnce(&mut [u8])) {
        fill(&mut self.cur);
        if self.cur == self.prev {
            // Same as the row above (common on desktops): Up gives all zeros, the best
            // any filter can do, so skip trying the others.
            self.out.push(2);
            self.out.resize(self.out.len() + self.cur.len(), 0);
            std::mem::swap(&mut self.cur, &mut self.prev);
            return;
        }
        let (cur, prev) = (&self.cur, &self.prev);
        let [s, u, p] = &mut self.cand;
        sub(cur, s);
        up(cur, prev, u);
        paeth(cur, prev, p);
        let picks: [&[u8]; 4] = [cur, s, u, p];
        let best = (0..4).min_by_key(|&i| score(picks[i])).unwrap_or(0);
        self.out.push([0, 1, 2, 4][best]); // PNG filter types; 3 is Average, unused
        self.out.extend_from_slice(picks[best]);
        std::mem::swap(&mut self.cur, &mut self.prev);
    }

    /// The filtered bytes of the stripe so far.
    pub fn data(&self) -> &[u8] {
        &self.out
    }
}

fn score(b: &[u8]) -> u32 {
    b.iter().map(|&x| u32::from((x as i8).unsigned_abs())).sum()
}

fn sub(cur: &[u8], out: &mut [u8]) {
    out[..BPP].copy_from_slice(&cur[..BPP]);
    let left = &cur[..cur.len() - BPP];
    for (o, (&x, &a)) in out[BPP..].iter_mut().zip(cur[BPP..].iter().zip(left)) {
        *o = x.wrapping_sub(a);
    }
}

fn up(cur: &[u8], prev: &[u8], out: &mut [u8]) {
    for (o, (&x, &b)) in out.iter_mut().zip(cur.iter().zip(prev)) {
        *o = x.wrapping_sub(b);
    }
}

fn paeth(cur: &[u8], prev: &[u8], out: &mut [u8]) {
    // First pixel: left and up-left are 0, so Paeth predicts "up".
    up(&cur[..BPP], &prev[..BPP], &mut out[..BPP]);
    let n = cur.len() - BPP;
    let (left, upleft) = (&cur[..n], &prev[..n]);
    let rest = cur[BPP..]
        .iter()
        .zip(&prev[BPP..])
        .zip(left.iter().zip(upleft));
    for (o, ((&x, &b), (&a, &c))) in out[BPP..].iter_mut().zip(rest) {
        *o = x.wrapping_sub(predict(a, b, c));
    }
}

#[inline(always)]
fn predict(a: u8, b: u8, c: u8) -> u8 {
    let (a16, b16, c16) = (i16::from(a), i16::from(b), i16::from(c));
    let pa = (b16 - c16).abs();
    let pb = (a16 - c16).abs();
    let pc = (a16 + b16 - 2 * c16).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Undo a filtered row the way a decoder does (spec 9.2).
    pub fn unfilter(kind: u8, f: &[u8], prev: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; f.len()];
        for i in 0..f.len() {
            let a = if i >= BPP { r[i - BPP] } else { 0 };
            let c = if i >= BPP { prev[i - BPP] } else { 0 };
            let pred = match kind {
                0 => 0,
                1 => a,
                2 => prev[i],
                4 => predict(a, prev[i], c),
                _ => panic!("filter type {kind} is not one we write"),
            };
            r[i] = f[i].wrapping_add(pred);
        }
        r
    }

    #[test]
    fn rows_roundtrip() {
        let mut rows: Vec<Vec<u8>> = (0..5u32)
            .map(|y| {
                (0..30u32)
                    .map(|x| (x * 37 + y * y * 11 + (x ^ y)) as u8)
                    .collect()
            })
            .collect();
        rows.insert(2, rows[1].clone()); // an identical row takes the Up shortcut
        let mut f = Filter::new(30);
        f.start(0, |_, _| unreachable!());
        for r in &rows {
            f.row(|out| out.copy_from_slice(r));
        }
        let mut prev = vec![0u8; 30];
        for (r, line) in rows.iter().zip(f.data().chunks(31)) {
            let got = unfilter(line[0], &line[1..], &prev);
            assert_eq!(&got, r);
            prev = got;
        }
    }
}
