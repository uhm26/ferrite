//! Line icons, drawn by us: each is a few strokes on a 24-unit grid (polylines,
//! rings, rounded boxes, dots), rasterized once per px size into an 8-bit mask by
//! distance to the strokes, then blended in any color. Strokes are [`STROKE`] px at
//! every size, so a 14 px icon beside text is as heavy as a 20 px one alone.

use super::paint::{Rect, sdf};

/// Stroke width, px.
pub const STROKE: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Folder,
    /// A solid folder: gives folder rows visual weight (owner: the look felt "empty").
    FolderFill,
    File,
    Home,
    Desktop,
    Documents,
    Download,
    Image,
    Music,
    Video,
    Drive,
    Code,
    Back,
    Forward,
    Up,
    Down,
    Search,
    List,
    Grid,
    Check,
    Close,
    Plus,
    Minus,
    Trash,
}

/// One stroke, in grid units.
enum Shape {
    /// Open polyline.
    Line(&'static [(f32, f32)]),
    /// Closed polyline.
    Loop(&'static [(f32, f32)]),
    /// Circle outline: center, radius.
    Ring((f32, f32), f32),
    /// Filled circle: center, radius.
    Dot((f32, f32), f32),
    /// Rounded box outline: x, y, w, h, radius.
    Frame(f32, f32, f32, f32, f32),
    /// Filled polygon (closed).
    Solid(&'static [(f32, f32)]),
}

use Shape::*;

// The drawings, on the 24 grid.
const PAGE: [(f32, f32); 5] = [
    (6.0, 3.0),
    (14.0, 3.0),
    (19.0, 8.0),
    (19.0, 21.0),
    (6.0, 21.0),
];
const FOLD: [(f32, f32); 3] = [(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)];
const FOLDER_PTS: [(f32, f32); 6] = [
    (3.0, 6.5),
    (9.5, 6.5),
    (11.5, 8.5),
    (21.0, 8.5),
    (21.0, 19.0),
    (3.0, 19.0),
];
const FOLDER: &[Shape] = &[Loop(&FOLDER_PTS)];
const FOLDER_FILL: &[Shape] = &[Solid(&FOLDER_PTS)];
const FILE: &[Shape] = &[Loop(&PAGE), Line(&FOLD)];
const HOME: &[Shape] = &[
    Line(&[(3.0, 11.0), (12.0, 3.5), (21.0, 11.0)]),
    Line(&[(5.5, 9.5), (5.5, 20.5), (18.5, 20.5), (18.5, 9.5)]),
    Line(&[(10.0, 20.5), (10.0, 14.5), (14.0, 14.5), (14.0, 20.5)]),
];
const DESKTOP: &[Shape] = &[
    Frame(3.0, 4.0, 18.0, 12.0, 2.0),
    Line(&[(12.0, 16.0), (12.0, 20.0)]),
    Line(&[(8.0, 20.0), (16.0, 20.0)]),
];
const DOCUMENTS: &[Shape] = &[
    Loop(&PAGE),
    Line(&FOLD),
    Line(&[(9.5, 13.0), (15.5, 13.0)]),
    Line(&[(9.5, 17.0), (15.5, 17.0)]),
];
const DOWNLOAD: &[Shape] = &[
    Line(&[(12.0, 3.5), (12.0, 15.0)]),
    Line(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]),
    Line(&[(5.0, 20.0), (19.0, 20.0)]),
];
const IMAGE: &[Shape] = &[
    Frame(3.0, 3.0, 18.0, 18.0, 2.5),
    Ring((9.0, 9.0), 1.75),
    Line(&[(20.5, 15.0), (16.0, 10.5), (5.5, 21.0)]),
];
const MUSIC: &[Shape] = &[
    Line(&[(9.0, 17.5), (9.0, 5.5), (20.0, 3.5), (20.0, 15.5)]),
    Ring((6.5, 17.5), 2.5),
    Ring((17.5, 15.5), 2.5),
];
const VIDEO: &[Shape] = &[
    Frame(2.5, 6.0, 13.5, 12.0, 2.0),
    Loop(&[(16.0, 10.5), (21.5, 7.5), (21.5, 16.5), (16.0, 13.5)]),
];
const DRIVE: &[Shape] = &[
    Frame(2.5, 13.0, 19.0, 7.5, 2.0),
    Line(&[(4.5, 13.0), (7.5, 4.5), (16.5, 4.5), (19.5, 13.0)]),
    Dot((6.5, 16.75), 1.0),
    Dot((10.0, 16.75), 1.0),
];
const CODE: &[Shape] = &[
    Line(&[(8.5, 7.5), (4.0, 12.0), (8.5, 16.5)]),
    Line(&[(15.5, 7.5), (20.0, 12.0), (15.5, 16.5)]),
];
const BACK: &[Shape] = &[Line(&[(15.0, 5.5), (8.5, 12.0), (15.0, 18.5)])];
const FORWARD: &[Shape] = &[Line(&[(9.0, 5.5), (15.5, 12.0), (9.0, 18.5)])];
const UP: &[Shape] = &[Line(&[(5.5, 15.0), (12.0, 8.5), (18.5, 15.0)])];
const DOWN: &[Shape] = &[Line(&[(5.5, 9.0), (12.0, 15.5), (18.5, 9.0)])];
const SEARCH: &[Shape] = &[Ring((10.5, 10.5), 6.5), Line(&[(15.5, 15.5), (20.5, 20.5)])];
const LIST: &[Shape] = &[
    Line(&[(9.0, 6.0), (20.5, 6.0)]),
    Line(&[(9.0, 12.0), (20.5, 12.0)]),
    Line(&[(9.0, 18.0), (20.5, 18.0)]),
    Dot((4.5, 6.0), 1.25),
    Dot((4.5, 12.0), 1.25),
    Dot((4.5, 18.0), 1.25),
];
const GRID: &[Shape] = &[
    Frame(3.5, 3.5, 7.0, 7.0, 1.5),
    Frame(13.5, 3.5, 7.0, 7.0, 1.5),
    Frame(3.5, 13.5, 7.0, 7.0, 1.5),
    Frame(13.5, 13.5, 7.0, 7.0, 1.5),
];
const CHECK: &[Shape] = &[Line(&[(4.5, 12.5), (9.5, 17.5), (19.5, 6.5)])];
const CLOSE: &[Shape] = &[
    Line(&[(6.0, 6.0), (18.0, 18.0)]),
    Line(&[(18.0, 6.0), (6.0, 18.0)]),
];
const PLUS: &[Shape] = &[
    Line(&[(12.0, 5.0), (12.0, 19.0)]),
    Line(&[(5.0, 12.0), (19.0, 12.0)]),
];
const MINUS: &[Shape] = &[Line(&[(5.0, 12.0), (19.0, 12.0)])];
const TRASH: &[Shape] = &[
    Line(&[(4.0, 6.5), (20.0, 6.5)]),
    Line(&[(9.5, 6.5), (9.5, 3.5), (14.5, 3.5), (14.5, 6.5)]),
    Line(&[(6.0, 6.5), (7.0, 20.5), (17.0, 20.5), (18.0, 6.5)]),
    Line(&[(10.0, 10.5), (10.0, 16.5)]),
    Line(&[(14.0, 10.5), (14.0, 16.5)]),
];

impl Icon {
    pub const ALL: [Icon; 24] = [
        Icon::Folder,
        Icon::FolderFill,
        Icon::File,
        Icon::Home,
        Icon::Desktop,
        Icon::Documents,
        Icon::Download,
        Icon::Image,
        Icon::Music,
        Icon::Video,
        Icon::Drive,
        Icon::Code,
        Icon::Back,
        Icon::Forward,
        Icon::Up,
        Icon::Down,
        Icon::Search,
        Icon::List,
        Icon::Grid,
        Icon::Check,
        Icon::Close,
        Icon::Plus,
        Icon::Minus,
        Icon::Trash,
    ];

    fn shapes(self) -> &'static [Shape] {
        match self {
            Icon::Folder => FOLDER,
            Icon::FolderFill => FOLDER_FILL,
            Icon::File => FILE,
            Icon::Home => HOME,
            Icon::Desktop => DESKTOP,
            Icon::Documents => DOCUMENTS,
            Icon::Download => DOWNLOAD,
            Icon::Image => IMAGE,
            Icon::Music => MUSIC,
            Icon::Video => VIDEO,
            Icon::Drive => DRIVE,
            Icon::Code => CODE,
            Icon::Back => BACK,
            Icon::Forward => FORWARD,
            Icon::Up => UP,
            Icon::Down => DOWN,
            Icon::Search => SEARCH,
            Icon::List => LIST,
            Icon::Grid => GRID,
            Icon::Check => CHECK,
            Icon::Close => CLOSE,
            Icon::Plus => PLUS,
            Icon::Minus => MINUS,
            Icon::Trash => TRASH,
        }
    }

    /// The coverage mask, `size` × `size` px, row-major.
    pub fn mask(self, size: usize) -> Box<[u8]> {
        let unit = size as f32 / 24.0; // px per grid unit
        let half = STROKE / 2.0;
        let shapes = self.shapes();
        let mut out = vec![0u8; size * size];
        for (i, cell) in out.iter_mut().enumerate() {
            let p = (
                ((i % size) as f32 + 0.5) / unit,
                ((i / size) as f32 + 0.5) / unit,
            );
            let cov = shapes
                .iter()
                .map(|s| {
                    let (d, filled) = distance(s, p);
                    let d = d * unit; // px
                    let edge = if filled { d } else { d.abs() - half };
                    (0.5 - edge).clamp(0.0, 1.0)
                })
                .fold(0.0f32, f32::max);
            *cell = (cov * 255.0 + 0.5) as u8;
        }
        out.into_boxed_slice()
    }
}

/// Distance in grid units from `p` to the shape's stroke center (or, for a filled
/// shape, signed to its edge), and whether it is filled.
fn distance(s: &Shape, p: (f32, f32)) -> (f32, bool) {
    match *s {
        Line(pts) => (polyline(pts, p, false), false),
        Loop(pts) => (polyline(pts, p, true), false),
        Ring(c, r) => (len(p.0 - c.0, p.1 - c.1) - r, false),
        Dot(c, r) => (len(p.0 - c.0, p.1 - c.1) - r, true),
        Frame(x, y, w, h, r) => (sdf(&Rect::new(x, y, w, h), r, p.0, p.1), false),
        Solid(pts) => {
            let d = polyline(pts, p, true);
            (if inside(pts, p) { -d } else { d }, true)
        }
    }
}

fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

fn polyline(pts: &[(f32, f32)], p: (f32, f32), closed: bool) -> f32 {
    let n = pts.len();
    let segs = if closed { n } else { n - 1 };
    (0..segs)
        .map(|i| segment(pts[i], pts[(i + 1) % n], p))
        .fold(f32::MAX, f32::min)
}

/// Even-odd test: is `p` inside the closed polygon `pts`?
fn inside(pts: &[(f32, f32)], p: (f32, f32)) -> bool {
    let n = pts.len();
    (0..n).fold(false, |odd, i| {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let crosses =
            (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0);
        odd != crosses
    })
}

fn segment(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    len(p.0 - a.0 - t * dx, p.1 - a.1 - t * dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_draws_something_inside_its_box() {
        for icon in Icon::ALL {
            let m = icon.mask(16);
            let ink: u32 = m.iter().map(|&c| u32::from(c)).sum();
            assert!(ink > 255 * 8, "{icon:?} is nearly empty");
            assert!(ink < 255 * 16 * 16 / 2, "{icon:?} is a blob");
        }
    }

    #[test]
    fn stroke_is_solid_on_the_line_and_clear_off_it() {
        let m = Icon::Minus.mask(24); // a line along y = 12, x 5..19
        assert_eq!(m[11 * 24 + 12], 255 / 2 + 64); // pixel row 11 centre 11.5: 0.5 px off-line
        assert_eq!(m[4 * 24 + 12], 0);
    }

    #[test]
    fn solid_folder_is_filled_inside() {
        let m = Icon::FolderFill.mask(24);
        assert_eq!(m[14 * 24 + 12], 255, "middle of the body");
        assert_eq!(m[2 * 24 + 12], 0, "above the folder");
    }

    #[test]
    fn back_mirrors_forward() {
        let (b, f) = (Icon::Back.mask(20), Icon::Forward.mask(20));
        for y in 0..20 {
            for x in 0..20 {
                let (l, r) = (i32::from(b[y * 20 + x]), i32::from(f[y * 20 + 19 - x]));
                assert!((l - r).abs() <= 40, "at {x},{y}: {l} vs {r}");
            }
        }
    }
}
