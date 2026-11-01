//! Box-drawing (U+2500 block) and block elements (U+2580 block), drawn to the exact
//! cell size so lines meet with no gaps between rows (TUI borders, Claude Code's
//! boxes). Double lines and diagonals still come from the font.

/// Coverage box for `c`, or None if the font should draw it.
pub fn draw(c: char, w: usize, h: usize, t: usize) -> Option<Box<[u8]>> {
    let block = (0x2580..=0x259f).contains(&(c as u32));
    if !block && lines(c).is_none() && rounded(c).is_none() {
        return None; // the common case: no buffer to allocate
    }
    let mut b = Canvas {
        px: vec![0; w * h],
        w,
        h,
    };
    if let Some((l, r, u, d)) = lines(c) {
        b.lines([l, r, u, d], t);
    } else if let Some(corner) = rounded(c) {
        b.arc(corner, t);
    } else if !b.block(c) {
        return None;
    }
    Some(b.px.into())
}

/// Weights (0 none, 1 light, 2 heavy) of the arms: left, right, up, down.
fn lines(c: char) -> Option<(u8, u8, u8, u8)> {
    Some(match c {
        '─' | '┄' | '┈' | '╌' => (1, 1, 0, 0),
        '━' | '┅' | '┉' | '╍' => (2, 2, 0, 0),
        '│' | '┆' | '┊' | '╎' => (0, 0, 1, 1),
        '┃' | '┇' | '┋' | '╏' => (0, 0, 2, 2),
        '┌' => (0, 1, 0, 1),
        '┐' => (1, 0, 0, 1),
        '└' => (0, 1, 1, 0),
        '┘' => (1, 0, 1, 0),
        '┏' => (0, 2, 0, 2),
        '┓' => (2, 0, 0, 2),
        '┗' => (0, 2, 2, 0),
        '┛' => (2, 0, 2, 0),
        '├' => (0, 1, 1, 1),
        '┤' => (1, 0, 1, 1),
        '┬' => (1, 1, 0, 1),
        '┴' => (1, 1, 1, 0),
        '┼' => (1, 1, 1, 1),
        '┣' => (0, 2, 2, 2),
        '┫' => (2, 0, 2, 2),
        '┳' => (2, 2, 0, 2),
        '┻' => (2, 2, 2, 0),
        '╋' => (2, 2, 2, 2),
        '╴' => (1, 0, 0, 0),
        '╵' => (0, 0, 1, 0),
        '╶' => (0, 1, 0, 0),
        '╷' => (0, 0, 0, 1),
        '╸' => (2, 0, 0, 0),
        '╹' => (0, 0, 2, 0),
        '╺' => (0, 2, 0, 0),
        '╻' => (0, 0, 0, 2),
        _ => return None,
    })
}

/// Rounded corners as (goes right, goes down).
fn rounded(c: char) -> Option<(bool, bool)> {
    match c {
        '╭' => Some((true, true)),
        '╮' => Some((false, true)),
        '╯' => Some((false, false)),
        '╰' => Some((true, false)),
        _ => None,
    }
}

struct Canvas {
    px: Vec<u8>,
    w: usize,
    h: usize,
}

impl Canvas {
    fn rect(&mut self, (x0, y0, x1, y1): (usize, usize, usize, usize), a: u8) {
        for y in y0.min(self.h)..y1.min(self.h) {
            let row = &mut self.px[y * self.w..(y + 1) * self.w];
            row[x0.min(self.w)..x1.min(self.w)].fill(a);
        }
    }

    /// Arms from the cell edges to the center. Each arm runs past the center by
    /// half the thickest crossing arm, so joins are solid.
    fn lines(&mut self, [l, r, u, d]: [u8; 4], t: usize) {
        let (cx, cy) = (self.w / 2, self.h / 2);
        let th = |wt: u8| usize::from(wt) * t;
        let vmax = th(u.max(d));
        let hmax = th(l.max(r));
        let (x_lo, x_hi) = (cx.saturating_sub(vmax / 2), cx - vmax / 2 + vmax.max(1));
        let (y_lo, y_hi) = (cy.saturating_sub(hmax / 2), cy - hmax / 2 + hmax.max(1));
        let hband = |wt: u8| (cy - th(wt) / 2, cy - th(wt) / 2 + th(wt));
        let vband = |wt: u8| (cx - th(wt) / 2, cx - th(wt) / 2 + th(wt));
        if l > 0 {
            let (y0, y1) = hband(l);
            self.rect((0, y0, x_hi, y1), 255);
        }
        if r > 0 {
            let (y0, y1) = hband(r);
            self.rect((x_lo, y0, self.w, y1), 255);
        }
        if u > 0 {
            let (x0, x1) = vband(u);
            self.rect((x0, 0, x1, y_hi), 255);
        }
        if d > 0 {
            let (x0, x1) = vband(d);
            self.rect((x0, y_lo, x1, self.h), 255);
        }
    }

    /// A quarter circle joining the vertical arm (up or down) and the horizontal
    /// arm (left or right), antialiased, with straight runs to the edges.
    fn arc(&mut self, (right, down): (bool, bool), t: usize) {
        let (cx, cy) = (self.w / 2, self.h / 2);
        let half = t as f32 / 2.0;
        // Centers of the straight lines, in pixel coordinates.
        let (lx, ly) = (cx as f32 + half, cy as f32 + half);
        let r = (self.w as f32 / 2.0).min(self.h as f32 / 2.0).max(1.0);
        let ox = if right { lx + r } else { lx - r };
        let oy = if down { ly + r } else { ly - r };
        for y in 0..self.h {
            for x in 0..self.w {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let in_x = if right { px <= ox } else { px >= ox };
                let in_y = if down { py <= oy } else { py >= oy };
                if !(in_x && in_y) {
                    continue;
                }
                let d = ((px - ox).powi(2) + (py - oy).powi(2)).sqrt();
                let cov = (half + 0.5 - (d - r).abs()).clamp(0.0, 1.0);
                let i = y * self.w + x;
                self.px[i] = self.px[i].max((cov * 255.0) as u8);
            }
        }
        // Straight parts beyond the arc.
        let (x0, x1) = (cx - t / 2, cx - t / 2 + t);
        let (y0, y1) = (cy - t / 2, cy - t / 2 + t);
        let (oxi, oyi) = (ox.max(0.0) as usize, oy.max(0.0) as usize);
        if down {
            self.rect((x0, oyi, x1, self.h), 255);
        } else {
            self.rect((x0, 0, x1, oyi), 255);
        }
        if right {
            self.rect((oxi, y0, self.w, y1), 255);
        } else {
            self.rect((0, y0, oxi, y1), 255);
        }
    }

    /// Block elements U+2580..=U+259F. Returns false for other chars.
    fn block(&mut self, c: char) -> bool {
        let (w, h) = (self.w, self.h);
        let eighth_h = |n: usize| (h * n + 4) / 8;
        let eighth_w = |n: usize| (w * n + 4) / 8;
        match c as u32 {
            0x2580 => self.rect((0, 0, w, h / 2), 255),
            0x2581..=0x2588 => {
                let n = (c as usize) - 0x2580;
                self.rect((0, h - eighth_h(n), w, h), 255);
            }
            0x2589..=0x258f => {
                let n = 0x2590 - (c as usize);
                self.rect((0, 0, eighth_w(n), h), 255);
            }
            0x2590 => self.rect((w / 2, 0, w, h), 255),
            0x2591 => self.rect((0, 0, w, h), 64),
            0x2592 => self.rect((0, 0, w, h), 128),
            0x2593 => self.rect((0, 0, w, h), 192),
            0x2594 => self.rect((0, 0, w, eighth_h(1)), 255),
            0x2595 => self.rect((w - eighth_w(1), 0, w, h), 255),
            0x2596..=0x259f => self.quadrants(c as u32 - 0x2596),
            _ => return false,
        }
        true
    }

    fn quadrants(&mut self, i: u32) {
        // Bits: upper-left, upper-right, lower-left, lower-right.
        const Q: [u8; 10] = [
            0b0010, 0b0001, 0b1000, 0b1011, 0b1001, 0b1110, 0b1101, 0b0100, 0b0110, 0b0111,
        ];
        let bits = Q[i as usize];
        let (w, h, mx, my) = (self.w, self.h, self.w / 2, self.h / 2);
        let parts = [
            (0, 0, mx, my),
            (mx, 0, w, my),
            (0, my, mx, h),
            (mx, my, w, h),
        ];
        for (k, &(x0, y0, x1, y1)) in parts.iter().enumerate() {
            if bits & (0b1000 >> k) != 0 {
                self.rect((x0, y0, x1, y1), 255);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_line_reaches_both_edges() {
        let b = draw('│', 9, 18, 1).unwrap();
        assert_eq!(b[4], 255); // top row, center column
        assert_eq!(b[17 * 9 + 4], 255); // bottom row
        assert_eq!(b[17 * 9 + 3], 0);
    }

    #[test]
    fn corner_meets_lines() {
        let b = draw('┌', 9, 18, 1).unwrap();
        assert_eq!(b[9 * 9 + 8], 255); // right edge on the center row
        assert_eq!(b[17 * 9 + 4], 255); // bottom edge on the center column
        assert_eq!(b[0], 0);
        let r = draw('╭', 9, 18, 1).unwrap();
        assert_eq!(r[17 * 9 + 4], 255);
        assert!(r[9 * 9 + 8] > 230); // antialiased arc end
    }

    #[test]
    fn blocks() {
        let b = draw('▄', 4, 8, 1).unwrap();
        assert_eq!((b[0], b[7 * 4]), (0, 255));
        assert!(draw('a', 4, 8, 1).is_none());
    }
}
