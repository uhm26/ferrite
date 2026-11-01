//! The freeze overlay of one output: a layer surface showing that output's captured
//! buffer, dimmed everywhere except a "hole" (selection or hovered window) that gets a
//! 1 px white hairline. Dim and lines are single-pixel buffers stretched by
//! wp_viewporter on subsurfaces, so moving the hole redraws no pixels on our side.

use crate::capture::Shot;
use crate::wl::{Arg, Conn, Error, proto};

/// Globals the overlay needs, plus shared objects made once.
pub struct Kit {
    pub compositor: u32,
    pub subcompositor: u32,
    pub layer_shell: u32,
    pub viewporter: u32,
    /// The two single-pixel buffers: black at 45 % (ui-skill shadow), white.
    pub dim: u32,
    pub line: u32,
    /// An empty wl_region: subsurfaces take no input, the parent gets it all.
    pub no_input: u32,
}

/// Black 45 % (premultiplied, so only alpha is set) and opaque white.
pub const DIM_RGBA: [u32; 4] = [0, 0, 0, (u32::MAX / 100) * 45];
pub const LINE_RGBA: [u32; 4] = [u32::MAX; 4];

struct Piece {
    surf: u32,
    sub: u32,
    vp: u32,
    buf: u32,
    shown: bool,
    /// What is on screen now, to skip pieces that did not move.
    rect: [i32; 4],
}

pub struct Overlay {
    pub surf: u32,
    pub layer: u32,
    /// The output's rectangle in the layout.
    pub rect: [i32; 4],
    shot_buf: u32,
    y_invert: bool,
    configured: bool,
    /// 4 dim pieces, then 4 hairlines (stacked above).
    pieces: Vec<Piece>,
    /// Wanted hole in layout coordinates; what is on screen; pending frame callback.
    hole: Option<[i32; 4]>,
    shown: Option<Option<[i32; 4]>>,
    frame_cb: Option<u32>,
}

impl Overlay {
    pub fn new(c: &mut Conn, k: &Kit, shot: &Shot) -> Result<Self, Error> {
        let surf = c.new_id();
        c.send(
            k.compositor,
            proto::compositor::CREATE_SURFACE,
            &[Arg::U(surf)],
        )?;
        let layer = c.new_id();
        let args = [
            Arg::U(layer),
            Arg::U(surf),
            Arg::U(shot.output),
            Arg::U(proto::layer_shell::LAYER_OVERLAY),
            Arg::S("snap"),
        ];
        c.send(k.layer_shell, proto::layer_shell::GET_LAYER_SURFACE, &args)?;
        use proto::layer_surface as ls;
        c.send(layer, ls::SET_ANCHOR, &[Arg::U(ls::ANCHOR_ALL)])?;
        c.send(layer, ls::SET_EXCLUSIVE_ZONE, &[Arg::I(-1)])?;
        c.send(
            layer,
            ls::SET_KEYBOARD_INTERACTIVITY,
            &[Arg::U(ls::KEYBOARD_EXCLUSIVE)],
        )?;
        c.send(surf, proto::surface::COMMIT, &[])?;
        let mut pieces = Vec::new();
        for i in 0..8 {
            pieces.push(piece(c, k, surf, if i < 4 { k.dim } else { k.line })?);
        }
        let rect = [shot.x, shot.y, shot.w as i32, shot.h as i32];
        let (shot_buf, y_invert) = (shot.buf, shot.y_invert);
        let (configured, hole, shown, frame_cb) = (false, None, None, None);
        Ok(Self {
            surf,
            layer,
            rect,
            shot_buf,
            y_invert,
            configured,
            pieces,
            hole,
            shown,
            frame_cb,
        })
    }

    /// layer_surface.configure: ack, show the frozen image, then the dim.
    pub fn configure(&mut self, c: &mut Conn, serial: u32) -> Result<(), Error> {
        c.send(
            self.layer,
            proto::layer_surface::ACK_CONFIGURE,
            &[Arg::U(serial)],
        )?;
        if self.configured {
            return Ok(());
        }
        self.configured = true;
        c.send(
            self.surf,
            proto::surface::ATTACH,
            &[Arg::U(self.shot_buf), Arg::I(0), Arg::I(0)],
        )?;
        if self.y_invert {
            let t = [Arg::I(proto::surface::FLIPPED_180)];
            c.send(self.surf, proto::surface::SET_BUFFER_TRANSFORM, &t)?;
        }
        let all = [Arg::I(0), Arg::I(0), Arg::I(i32::MAX), Arg::I(i32::MAX)];
        c.send(self.surf, proto::surface::DAMAGE, &all)?;
        self.apply(c)
    }

    /// Ask for a new hole (layout coordinates); drawn at the next frame at the latest.
    pub fn set_hole(&mut self, c: &mut Conn, hole: Option<[i32; 4]>) -> Result<(), Error> {
        self.hole = hole;
        match self.configured && self.frame_cb.is_none() && self.shown != Some(hole) {
            true => self.apply(c),
            false => Ok(()),
        }
    }

    /// The frame callback fired: the compositor showed our last change.
    pub fn frame_done(&mut self, c: &mut Conn, cb: u32) -> Result<bool, Error> {
        if self.frame_cb != Some(cb) {
            return Ok(false);
        }
        self.frame_cb = None;
        if self.shown != Some(self.hole) {
            self.apply(c)?;
        }
        Ok(true)
    }

    pub fn destroy(&self, c: &mut Conn) -> Result<(), Error> {
        c.send(self.layer, proto::layer_surface::DESTROY, &[])?;
        c.send(self.surf, proto::surface::DESTROY, &[])
    }

    /// Move the 8 pieces for `self.hole`, then one parent commit with a frame callback.
    fn apply(&mut self, c: &mut Conn) -> Result<(), Error> {
        let rects = layout(self.rect, self.hole);
        for (p, r) in self.pieces.iter_mut().zip(rects) {
            place(c, p, r)?;
        }
        let cb = c.new_id();
        c.send(self.surf, proto::surface::FRAME, &[Arg::U(cb)])?;
        c.send(self.surf, proto::surface::COMMIT, &[])?;
        (self.frame_cb, self.shown) = (Some(cb), Some(self.hole));
        Ok(())
    }
}

/// A subsurface of `parent` showing `buf`, with a viewport and no input.
fn piece(c: &mut Conn, k: &Kit, parent: u32, buf: u32) -> Result<Piece, Error> {
    let (surf, sub, vp) = (c.new_id(), c.new_id(), c.new_id());
    c.send(
        k.compositor,
        proto::compositor::CREATE_SURFACE,
        &[Arg::U(surf)],
    )?;
    let args = [Arg::U(sub), Arg::U(surf), Arg::U(parent)];
    c.send(k.subcompositor, proto::subcompositor::GET_SUBSURFACE, &args)?;
    c.send(
        k.viewporter,
        proto::viewporter::GET_VIEWPORT,
        &[Arg::U(vp), Arg::U(surf)],
    )?;
    c.send(
        surf,
        proto::surface::SET_INPUT_REGION,
        &[Arg::U(k.no_input)],
    )?;
    Ok(Piece {
        surf,
        sub,
        vp,
        buf,
        shown: false,
        rect: [0; 4],
    })
}

/// Show `p` stretched over `[x, y, w, h]` (output-local), or hide it if empty.
fn place(c: &mut Conn, p: &mut Piece, [x, y, w, h]: [i32; 4]) -> Result<(), Error> {
    if p.rect == [x, y, w, h] {
        return Ok(()); // a hover or drag usually moves only some of the 8 pieces
    }
    p.rect = [x, y, w, h];
    if w <= 0 || h <= 0 {
        if p.shown {
            c.send(
                p.surf,
                proto::surface::ATTACH,
                &[Arg::U(0), Arg::I(0), Arg::I(0)],
            )?;
            c.send(p.surf, proto::surface::COMMIT, &[])?;
            p.shown = false;
        }
        return Ok(());
    }
    // Attach on every change: Hyprland 0.57 applied a new viewport size only in a
    // commit that also attached a buffer (pieces froze at their first size).
    let buf = [Arg::U(p.buf), Arg::I(0), Arg::I(0)];
    c.send(p.surf, proto::surface::ATTACH, &buf)?;
    p.shown = true;
    c.send(
        p.sub,
        proto::subsurface::SET_POSITION,
        &[Arg::I(x), Arg::I(y)],
    )?;
    c.send(
        p.vp,
        proto::viewport::SET_DESTINATION,
        &[Arg::I(w), Arg::I(h)],
    )?;
    let all = [Arg::I(0), Arg::I(0), Arg::I(i32::MAX), Arg::I(i32::MAX)];
    c.send(p.surf, proto::surface::DAMAGE, &all)?;
    c.send(p.surf, proto::surface::COMMIT, &[])
}

/// Output-local rectangles of the 4 dim pieces (around the hole, clipped to the
/// output) and the 4 hairlines (just outside the unclipped hole, so a selection
/// spanning two outputs has no line on the seam).
fn layout(out: [i32; 4], hole: Option<[i32; 4]>) -> [[i32; 4]; 8] {
    let [ox, oy, w, h] = out;
    let none = [0; 4];
    let Some([gx, gy, gw, gh]) = hole else {
        return [[0, 0, w, h], none, none, none, none, none, none, none];
    };
    let (lx, ly) = (gx - ox, gy - oy);
    let (x0, y0) = (lx.clamp(0, w), ly.clamp(0, h));
    let (x1, y1) = ((lx + gw).clamp(0, w), (ly + gh).clamp(0, h));
    if x0 >= x1 || y0 >= y1 {
        return [[0, 0, w, h], none, none, none, none, none, none, none];
    }
    [
        [0, 0, w, y0],
        [0, y1, w, h - y1],
        [0, y0, x0, y1 - y0],
        [x1, y0, w - x1, y1 - y0],
        [lx - 1, ly - 1, gw + 2, 1],
        [lx - 1, ly + gh, gw + 2, 1],
        [lx - 1, ly, 1, gh],
        [lx + gw, ly, 1, gh],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(r: &[i32; 4]) -> i64 {
        i64::from(r[2].max(0)) * i64::from(r[3].max(0))
    }

    #[test]
    fn dim_pieces_cover_all_but_the_hole() {
        let out = [1920, 0, 1920, 1080];
        let r = layout(out, Some([2000, 100, 300, 200]));
        let dim: i64 = r[..4].iter().map(area).sum();
        assert_eq!(dim, 1920 * 1080 - 300 * 200);
        assert_eq!(r[4], [79, 99, 302, 1]);
    }

    #[test]
    fn hole_elsewhere_dims_everything() {
        let r = layout([0, 0, 1920, 1080], Some([1920, 0, 50, 50]));
        assert_eq!(r[0], [0, 0, 1920, 1080]);
        assert!(r[1..].iter().all(|p| area(p) == 0));
    }

    #[test]
    fn hole_across_the_seam_is_clipped() {
        let r = layout([0, 0, 1920, 1080], Some([1900, 10, 100, 10]));
        assert_eq!(r[3], [1920, 10, 0, 10]); // nothing right of the hole on this output
        assert_eq!(r[7], [2000, 10, 1, 10]); // its right line lies on the other output
    }
}
