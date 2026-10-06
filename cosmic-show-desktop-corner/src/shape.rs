// SPDX-License-Identifier: GPL-3.0-only

//! The triangle's geometry and color, with no Wayland in sight.
//!
//! Coordinates are logical pixels in surface space: origin top-left, y down.
//! The surface sits in one of the screen's corners, and the triangle hugs
//! that corner of the surface. The geometry is worked out once, for the
//! bottom-right corner, where the screen's corner is at (SIZE, SIZE); the
//! other corners are mirror images of it ([`ScreenCorner::to_bottom_right`]).

/// Side of the (square) surface. It is also the hit area: the whole corner
/// triangle of this size answers the pointer, however small the drawing is —
/// a corner is easy to throw the mouse at, as long as the target is generous.
pub const SIZE: u32 = 28;

/// Which corner of the screen the surface sits in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ScreenCorner {
    pub const ALL: [ScreenCorner; 4] = [
        ScreenCorner::TopLeft,
        ScreenCorner::TopRight,
        ScreenCorner::BottomLeft,
        ScreenCorner::BottomRight,
    ];

    /// The name on the command line: `top-left`, `bottom-right`…
    pub fn name(self) -> &'static str {
        match self {
            ScreenCorner::TopLeft => "top-left",
            ScreenCorner::TopRight => "top-right",
            ScreenCorner::BottomLeft => "bottom-left",
            ScreenCorner::BottomRight => "bottom-right",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    fn left(self) -> bool {
        matches!(self, ScreenCorner::TopLeft | ScreenCorner::BottomLeft)
    }

    fn top(self) -> bool {
        matches!(self, ScreenCorner::TopLeft | ScreenCorner::TopRight)
    }

    /// The same point in the bottom-right frame — a mirror across the
    /// surface's middle for each side that differs. Its own inverse.
    pub fn to_bottom_right(self, x: f64, y: f64) -> (f64, f64) {
        let s = SIZE as f64;
        (
            if self.left() { s - x } else { x },
            if self.top() { s - y } else { y },
        )
    }
}

/// How the triangle looks in each state: legs (logical px) and opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub legs: f32,
    pub alpha: f32,
    /// How far the accent is mixed toward white. The accent often comes from
    /// the wallpaper itself (cosmic-wallsync), so pure accent would vanish
    /// into the very image it was taken from; a lighter tint stays visible.
    pub lighten: f32,
}

/// Idle: a small, faint mark — there, if you look for it.
pub const IDLE: Look = Look { legs: 12.0, alpha: 0.60, lighten: 0.55 };
/// Pointer over it: grows to announce it is clickable.
pub const HOVER: Look = Look { legs: 22.0, alpha: 0.90, lighten: 0.0 };
/// Button held.
pub const PRESSED: Look = Look { legs: 19.0, alpha: 1.0, lighten: 0.0 };

/// Whether a logical point is inside the corner triangle with the given legs.
pub fn inside(corner: ScreenCorner, x: f64, y: f64, legs: f64) -> bool {
    let (x, y) = corner.to_bottom_right(x, y);
    let s = SIZE as f64;
    (0.0..=s).contains(&x) && (0.0..=s).contains(&y) && (s - x) + (s - y) <= legs
}

/// Premultiplied ARGB8888 (little-endian bytes: B, G, R, A) for a surface of
/// `SIZE * scale` pixels a side. Edges are 4×4 supersampled so the diagonal
/// isn't a staircase.
#[cfg(test)]
pub fn render(corner: ScreenCorner, look: Look, rgb: [f32; 3], scale: u32) -> Vec<u8> {
    render_mode(corner, look, rgb, scale, false)
}

/// Where the "all screens" notch sits, as a fraction of the legs, and how
/// wide it is in logical pixels.
const NOTCH_AT: f64 = 0.5;
const NOTCH_WIDTH: f64 = 1.5;

/// [`render`], with `all_screens` cutting a thin gap parallel to the diagonal:
/// two stacked layers instead of one, for "shows the desktop on every screen".
/// The difference is meant to be noticed when looked for, not to shout.
pub fn render_mode(
    corner: ScreenCorner,
    look: Look,
    rgb: [f32; 3],
    scale: u32,
    all_screens: bool,
) -> Vec<u8> {
    let side = SIZE * scale;
    let notch = all_screens.then(|| {
        let from = look.legs as f64 * NOTCH_AT;
        from..from + NOTCH_WIDTH
    });
    let rgb = rgb.map(|c| c.clamp(0.0, 1.0) * (1.0 - look.lighten) + look.lighten);
    let mut out = Vec::with_capacity((side * side * 4) as usize);
    const N: u32 = 4;
    for py in 0..side {
        for px in 0..side {
            let mut hits = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let x = (px as f64 + (sx as f64 + 0.5) / N as f64) / scale as f64;
                    let y = (py as f64 + (sy as f64 + 0.5) / N as f64) / scale as f64;
                    if inside(corner, x, y, look.legs as f64)
                        && !notch.as_ref().is_some_and(|notch| {
                            let (bx, by) = corner.to_bottom_right(x, y);
                            let s = SIZE as f64;
                            notch.contains(&((s - bx) + (s - by)))
                        })
                    {
                        hits += 1;
                    }
                }
            }
            let a = look.alpha * hits as f32 / (N * N) as f32;
            let [r, g, b] = rgb.map(|c| (c * a * 255.0).round() as u8);
            out.extend_from_slice(&[b, g, r, (a * 255.0).round() as u8]);
        }
    }
    out
}

/// Rectangles (x, y, w, h) whose union covers the full-size hit triangle, for
/// the input region: Wayland regions are rectangles, so the diagonal is a
/// staircase of `step`-pixel bands. Outside it, clicks fall through to
/// whatever is underneath. Built for bottom-right and mirrored like the rest.
pub fn input_staircase(corner: ScreenCorner, step: u32) -> Vec<(i32, i32, i32, i32)> {
    let s = SIZE as i32;
    (0..SIZE.div_ceil(step))
        .map(|k| {
            let bottom = SIZE - k * step;
            let h = step.min(bottom);
            let (x, y, w, h) =
                ((k * step) as i32, (bottom - h) as i32, (SIZE - k * step) as i32, h as i32);
            (
                if corner.left() { s - x - w } else { x },
                if corner.top() { s - y - h } else { y },
                w,
                h,
            )
        })
        .collect()
}

/// The accent color out of a COSMIC theme `accent` entry: the first
/// `red`/`green`/`blue` triple, which is the component's `base`.
pub fn parse_accent(ron: &str) -> Option<[f32; 3]> {
    let field = |name: &str| -> Option<f32> {
        let start = ron.find(&format!("{name}:"))? + name.len() + 1;
        let rest = ron[start..].trim_start();
        let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e'))?;
        rest[..end].parse().ok()
    };
    Some([field("red")?, field("green")?, field("blue")?])
}

#[cfg(test)]
mod tests {
    use super::*;

    use ScreenCorner::*;

    /// The surface's own corner pixel (where the screen's corner is) and the
    /// opposite one, as (x, y) in a `side`-pixel square.
    fn near_and_far(corner: ScreenCorner, side: usize) -> ((usize, usize), (usize, usize)) {
        let (l, t) = (corner.left(), corner.top());
        let pick = |low: bool| if low { 0 } else { side - 1 };
        ((pick(l), pick(t)), (pick(!l), pick(!t)))
    }

    #[test]
    fn corner_is_inside_and_far_side_is_not() {
        let s = SIZE as f64;
        assert!(inside(BottomRight, s - 0.5, s - 0.5, 12.0));
        assert!(!inside(BottomRight, 1.0, 1.0, 12.0));
        // On the diagonal's outer side.
        assert!(!inside(BottomRight, s - 10.0, s - 10.0, 12.0));
        assert!(inside(BottomRight, s - 5.0, s - 5.0, 12.0));
    }

    #[test]
    fn each_corner_hugs_its_own_corner() {
        let s = SIZE as f64;
        for (corner, x, y) in [
            (TopLeft, 0.5, 0.5),
            (TopRight, s - 0.5, 0.5),
            (BottomLeft, 0.5, s - 0.5),
            (BottomRight, s - 0.5, s - 0.5),
        ] {
            assert!(inside(corner, x, y, 12.0), "{corner:?}");
            assert!(!inside(corner, s - x, s - y, 12.0), "{corner:?} opposite");
            for other in ScreenCorner::ALL.into_iter().filter(|&c| c != corner) {
                assert!(!inside(other, x, y, 12.0), "{other:?} at {corner:?}'s spot");
            }
        }
    }

    #[test]
    fn mirroring_is_its_own_inverse() {
        for corner in ScreenCorner::ALL {
            let (x, y) = corner.to_bottom_right(3.0, 20.0);
            assert_eq!(corner.to_bottom_right(x, y), (3.0, 20.0));
        }
    }

    #[test]
    fn names_round_trip() {
        for corner in ScreenCorner::ALL {
            assert_eq!(ScreenCorner::from_name(corner.name()), Some(corner));
        }
        assert_eq!(ScreenCorner::from_name("middle"), None);
    }

    #[test]
    fn render_is_opaque_at_the_corner_and_clear_opposite() {
        for corner in ScreenCorner::ALL {
            for scale in [1, 2] {
                let side = (SIZE * scale) as usize;
                let px = render(corner, HOVER, [1.0, 0.0, 0.0], scale);
                assert_eq!(px.len(), side * side * 4);
                let at = |(x, y): (usize, usize)| &px[(y * side + x) * 4..][..4];
                let (near, far) = near_and_far(corner, side);
                // The screen's corner: full coverage, premultiplied red.
                assert_eq!(at(near), &[0, 0, 230, 230], "{corner:?}");
                // The opposite pixel: nothing.
                assert_eq!(at(far), &[0, 0, 0, 0], "{corner:?}");
            }
        }
    }

    #[test]
    fn staircase_covers_the_hit_triangle_and_stays_in_the_surface() {
        for corner in ScreenCorner::ALL {
            let rects = input_staircase(corner, 4);
            let covered = |x: f64, y: f64| {
                rects.iter().any(|&(rx, ry, rw, rh)| {
                    x >= rx as f64
                        && x < (rx + rw) as f64
                        && y >= ry as f64
                        && y < (ry + rh) as f64
                })
            };
            for py in 0..SIZE {
                for px in 0..SIZE {
                    let (x, y) = (px as f64 + 0.5, py as f64 + 0.5);
                    if inside(corner, x, y, SIZE as f64) {
                        assert!(covered(x, y), "{corner:?}: ({px}, {py}) not clickable");
                    }
                }
            }
            // And nothing near the opposite corner takes clicks.
            let (_, (fx, fy)) = near_and_far(corner, SIZE as usize);
            assert!(!covered(fx as f64 + 0.5, fy as f64 + 0.5), "{corner:?}");
            for (x, y, w, h) in rects {
                assert!(x >= 0 && y >= 0 && x + w <= SIZE as i32 && y + h <= SIZE as i32);
            }
        }
    }

    #[test]
    fn all_screens_cuts_a_notch_and_keeps_the_corner() {
        for corner in ScreenCorner::ALL {
            let one = render_mode(corner, HOVER, [1.0, 0.0, 0.0], 1, false);
            let all = render_mode(corner, HOVER, [1.0, 0.0, 0.0], 1, true);
            assert_ne!(one, all, "{corner:?}: the modes must look different");
            let ((nx, ny), _) = near_and_far(corner, SIZE as usize);
            let alpha = |px: &[u8], x: usize, y: usize| px[(y * SIZE as usize + x) * 4 + 3];
            assert_eq!(alpha(&one, nx, ny), alpha(&all, nx, ny), "the very corner stays");
            // Fewer lit pixels, never more.
            let lit = |px: &[u8]| px.chunks(4).map(|p| p[3] as u32).sum::<u32>();
            assert!(lit(&all) < lit(&one));
        }
    }

    #[test]
    fn reads_accent_base() {
        let ron = "(\n    base: (\n        red: 0.3470661,\n        green: 0.5795144,\n        blue: 0.88330287,\n        alpha: 1.0,\n    ),\n    hover: (\n        red: 0.1,\n";
        assert_eq!(parse_accent(ron), Some([0.3470661, 0.5795144, 0.88330287]));
        assert_eq!(parse_accent("garbage"), None);
    }
}
