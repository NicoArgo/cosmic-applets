// SPDX-License-Identifier: GPL-3.0-only

//! The triangle's geometry and color, with no Wayland in sight.
//!
//! Coordinates are logical pixels in surface space: origin top-left, y down,
//! the corner of the screen at (0, SIZE).

/// Side of the (square) surface. It is also the hit area: the whole corner
/// triangle of this size answers the pointer, however small the drawing is —
/// a corner is easy to throw the mouse at, as long as the target is generous.
pub const SIZE: u32 = 28;

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
pub fn inside(x: f64, y: f64, legs: f64) -> bool {
    x >= 0.0 && y <= SIZE as f64 && x + (SIZE as f64 - y) <= legs
}

/// Premultiplied ARGB8888 (little-endian bytes: B, G, R, A) for a surface of
/// `SIZE * scale` pixels a side. Edges are 4×4 supersampled so the diagonal
/// isn't a staircase.
pub fn render(look: Look, rgb: [f32; 3], scale: u32) -> Vec<u8> {
    let side = SIZE * scale;
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
                    if inside(x, y, look.legs as f64) {
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
/// whatever is underneath.
pub fn input_staircase(step: u32) -> Vec<(i32, i32, i32, i32)> {
    (0..SIZE.div_ceil(step))
        .map(|k| {
            let bottom = SIZE - k * step;
            let h = step.min(bottom);
            (0, (bottom - h) as i32, (SIZE - k * step) as i32, h as i32)
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

    #[test]
    fn corner_is_inside_and_far_side_is_not() {
        let s = SIZE as f64;
        assert!(inside(0.5, s - 0.5, 12.0));
        assert!(!inside(s - 1.0, 1.0, 12.0));
        // On the diagonal's outer side.
        assert!(!inside(10.0, s - 10.0, 12.0));
        assert!(inside(5.0, s - 5.0, 12.0));
    }

    #[test]
    fn render_is_opaque_at_the_corner_and_clear_opposite() {
        for scale in [1, 2] {
            let side = (SIZE * scale) as usize;
            let px = render(HOVER, [1.0, 0.0, 0.0], scale);
            assert_eq!(px.len(), side * side * 4);
            let at = |x: usize, y: usize| &px[(y * side + x) * 4..][..4];
            // bottom-left pixel: full coverage, premultiplied red.
            assert_eq!(at(0, side - 1), &[0, 0, 230, 230]);
            // top-right pixel: nothing.
            assert_eq!(at(side - 1, 0), &[0, 0, 0, 0]);
        }
    }

    #[test]
    fn staircase_covers_the_hit_triangle_and_stays_in_the_surface() {
        let rects = input_staircase(4);
        let covered = |x: f64, y: f64| {
            rects.iter().any(|&(rx, ry, rw, rh)| {
                x >= rx as f64 && x < (rx + rw) as f64 && y >= ry as f64 && y < (ry + rh) as f64
            })
        };
        for py in 0..SIZE {
            for px in 0..SIZE {
                let (x, y) = (px as f64 + 0.5, py as f64 + 0.5);
                if inside(x, y, SIZE as f64) {
                    assert!(covered(x, y), "({px}, {py}) not clickable");
                }
            }
        }
        for (x, y, w, h) in rects {
            assert!(x >= 0 && y >= 0 && x + w <= SIZE as i32 && y + h <= SIZE as i32);
        }
    }

    #[test]
    fn reads_accent_base() {
        let ron = "(\n    base: (\n        red: 0.3470661,\n        green: 0.5795144,\n        blue: 0.88330287,\n        alpha: 1.0,\n    ),\n    hover: (\n        red: 0.1,\n";
        assert_eq!(parse_accent(ron), Some([0.3470661, 0.5795144, 0.88330287]));
        assert_eq!(parse_accent("garbage"), None);
    }
}
