//! Pixel effects: blur, pixelate and the spotlight's dimming.
//!
//! Effects always read the original screenshot, never what has been drawn on
//! it. The preview and the saved file then agree, whatever order things were
//! added in.

use egui::Rect;

use crate::platform::Image;

/// Integer pixel bounds of `r` inside an image of `w` × `h`, or `None` when
/// nothing is left.
pub fn clamp_rect(r: Rect, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = r.min.x.floor().max(0.0) as i64;
    let y0 = r.min.y.floor().max(0.0) as i64;
    let x1 = (r.max.x.ceil() as i64).min(w as i64);
    let y1 = (r.max.y.ceil() as i64).min(h as i64);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// A blurred copy of part of `src`. Three box blurs in a row come close to a
/// Gaussian and cost the same whatever the radius. Pixels outside the region
/// are sampled too, so the edges do not darken.
pub fn blur(src: &Image, region: Rect, radius: f32) -> Option<(u32, u32, Image)> {
    let (x, y, w, h) = clamp_rect(region, src.width, src.height)?;
    let r = radius.max(1.0).round() as u32;
    // Work on the region plus a margin, then cut the margin away again.
    let margin = r * 3;
    let mx0 = x.saturating_sub(margin);
    let my0 = y.saturating_sub(margin);
    let mx1 = (x + w + margin).min(src.width);
    let my1 = (y + h + margin).min(src.height);
    let (mw, mh) = (mx1 - mx0, my1 - my0);

    let mut buf: Vec<[f32; 4]> = Vec::with_capacity((mw * mh) as usize);
    for yy in my0..my1 {
        for xx in mx0..mx1 {
            let i = ((yy * src.width + xx) * 4) as usize;
            let p = &src.rgba[i..i + 4];
            buf.push([p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]);
        }
    }
    let mut tmp = buf.clone();
    for _ in 0..3 {
        box_pass(&buf, &mut tmp, mw as usize, mh as usize, r as usize, true);
        box_pass(&tmp, &mut buf, mw as usize, mh as usize, r as usize, false);
    }

    let mut out = Image {
        width: w,
        height: h,
        rgba: Vec::with_capacity((w * h * 4) as usize),
    };
    for yy in 0..h {
        for xx in 0..w {
            let p = buf[((yy + y - my0) * mw + (xx + x - mx0)) as usize];
            out.rgba
                .extend(p.iter().map(|v| v.round().clamp(0.0, 255.0) as u8));
        }
    }
    Some((x, y, out))
}

/// One running-sum box blur along rows or columns, clamping at the edges.
fn box_pass(src: &[[f32; 4]], dst: &mut [[f32; 4]], w: usize, h: usize, r: usize, rows: bool) {
    let (lines, len) = if rows { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if rows { line * w + i } else { i * w + line };
    let window = (2 * r + 1) as f32;
    for line in 0..lines {
        let mut sum = [0.0f32; 4];
        // Start with the window centred on the first pixel.
        for k in 0..=2 * r {
            let i = k.saturating_sub(r).min(len - 1);
            let p = src[at(line, i)];
            for c in 0..4 {
                sum[c] += p[c];
            }
        }
        for i in 0..len {
            let out = &mut dst[at(line, i)];
            for c in 0..4 {
                out[c] = sum[c] / window;
            }
            let leaving = src[at(line, i.saturating_sub(r))];
            let entering = src[at(line, (i + r + 1).min(len - 1))];
            for c in 0..4 {
                sum[c] += entering[c] - leaving[c];
            }
        }
    }
}

/// Part of `src` in blocks of one average colour each. The blocks line up
/// with the image rather than the region, so neighbouring regions match.
pub fn pixelate(src: &Image, region: Rect, block: f32) -> Option<(u32, u32, Image)> {
    let (x, y, w, h) = clamp_rect(region, src.width, src.height)?;
    let b = block.max(2.0).round() as u32;
    let mut out = Image {
        width: w,
        height: h,
        rgba: vec![0; (w * h * 4) as usize],
    };
    let bx0 = x / b * b;
    let by0 = y / b * b;
    let mut by = by0;
    while by < y + h {
        let mut bx = bx0;
        while bx < x + w {
            // Average over the block where it lies inside the region.
            let (cx0, cy0) = (bx.max(x), by.max(y));
            let (cx1, cy1) = ((bx + b).min(x + w), (by + b).min(y + h));
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for yy in cy0..cy1 {
                for xx in cx0..cx1 {
                    let i = ((yy * src.width + xx) * 4) as usize;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += src.rgba[i + c] as u64;
                    }
                    n += 1;
                }
            }
            if n > 0 {
                let avg: Vec<u8> = sum.iter().map(|s| (s / n) as u8).collect();
                for yy in cy0..cy1 {
                    for xx in cx0..cx1 {
                        let o = (((yy - y) * w + (xx - x)) * 4) as usize;
                        out.rgba[o..o + 4].copy_from_slice(&avg);
                    }
                }
            }
            bx += b;
        }
        by += b;
    }
    Some((x, y, out))
}

/// Copy `patch` into `dst` with its top left corner at `x`, `y`.
pub fn paste(dst: &mut Image, patch: &Image, x: u32, y: u32) {
    for row in 0..patch.height {
        let dy = y + row;
        if dy >= dst.height {
            break;
        }
        let w = patch.width.min(dst.width.saturating_sub(x));
        if w == 0 {
            return;
        }
        let s = (row * patch.width * 4) as usize;
        let d = ((dy * dst.width + x) * 4) as usize;
        dst.rgba[d..d + (w * 4) as usize].copy_from_slice(&patch.rgba[s..s + (w * 4) as usize]);
    }
}

/// How much of a pixel is outside every spot, 0 to 1, with a soft one pixel
/// edge. `spots` are in the same coordinates as `(px, py)`.
fn outside(spots: &[(Rect, bool)], px: f32, py: f32) -> f32 {
    let mut coverage: f32 = 0.0;
    for (r, round) in spots {
        let inside = if *round {
            let c = r.center();
            let (a, b) = (r.width() / 2.0, r.height() / 2.0);
            if a <= 0.0 || b <= 0.0 {
                0.0
            } else {
                // Distance to the ellipse edge, roughly in pixels.
                let d = ((px - c.x) / a).hypot((py - c.y) / b);
                ((1.0 - d) * a.min(b) + 0.5).clamp(0.0, 1.0)
            }
        } else {
            let dx = (px - r.min.x).min(r.max.x - px);
            let dy = (py - r.min.y).min(r.max.y - py);
            (dx.min(dy) + 0.5).clamp(0.0, 1.0)
        };
        coverage = coverage.max(inside);
        if coverage >= 1.0 {
            break;
        }
    }
    1.0 - coverage
}

/// Darken everything outside the spots by `dim` (0 to 1).
pub fn spotlight(dst: &mut Image, spots: &[(Rect, bool)], dim: f32) {
    if spots.is_empty() {
        return;
    }
    let keep = 1.0 - dim.clamp(0.0, 1.0);
    for y in 0..dst.height {
        for x in 0..dst.width {
            let o = outside(spots, x as f32 + 0.5, y as f32 + 0.5);
            if o <= 0.0 {
                continue;
            }
            let f = 1.0 - o * (1.0 - keep);
            let i = ((y * dst.width + x) * 4) as usize;
            for c in 0..3 {
                dst.rgba[i + c] = (dst.rgba[i + c] as f32 * f).round() as u8;
            }
        }
    }
}

/// The spotlight as a black overlay with transparent holes, at `scale` of the
/// image size, for the editor preview.
pub fn spotlight_overlay(
    width: u32,
    height: u32,
    spots: &[(Rect, bool)],
    dim: f32,
    scale: f32,
) -> egui::ColorImage {
    let w = ((width as f32 * scale).round() as usize).max(1);
    let h = ((height as f32 * scale).round() as usize).max(1);
    let scaled: Vec<(Rect, bool)> = spots
        .iter()
        .map(|(r, round)| {
            (
                Rect::from_min_max(
                    (r.min.to_vec2() * scale).to_pos2(),
                    (r.max.to_vec2() * scale).to_pos2(),
                ),
                *round,
            )
        })
        .collect();
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let o = outside(&scaled, x as f32 + 0.5, y as f32 + 0.5);
            let a = (o * dim.clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels.push(egui::Color32::from_black_alpha(a));
        }
    }
    egui::ColorImage::new([w, h], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn gradient(w: u32, h: u32) -> Image {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[(x * 10) as u8, (y * 10) as u8, 100, 255]);
            }
        }
        Image {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn a_flat_area_stays_flat_under_blur() {
        let flat = Image {
            width: 20,
            height: 20,
            rgba: [50u8, 60, 70, 255].repeat(400),
        };
        let (x, y, out) = blur(
            &flat,
            Rect::from_min_size(pos2(5.0, 5.0), vec2(10.0, 10.0)),
            3.0,
        )
        .unwrap();
        assert_eq!((x, y, out.width, out.height), (5, 5, 10, 10));
        assert!(out.rgba.chunks(4).all(|p| p == [50, 60, 70, 255]));
    }

    #[test]
    fn blur_smooths_an_edge() {
        let mut img = Image {
            width: 20,
            height: 1,
            rgba: vec![0; 80],
        };
        for x in 10..20 {
            img.rgba[x * 4..x * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        let (_, _, out) = blur(
            &img,
            Rect::from_min_size(pos2(0.0, 0.0), vec2(20.0, 1.0)),
            2.0,
        )
        .unwrap();
        let v = out.rgba[10 * 4];
        assert!(v > 60 && v < 200, "edge value {v}");
    }

    #[test]
    fn pixelate_gives_one_colour_per_block() {
        let img = gradient(8, 8);
        let (_, _, out) = pixelate(
            &img,
            Rect::from_min_size(pos2(0.0, 0.0), vec2(8.0, 8.0)),
            4.0,
        )
        .unwrap();
        let first = &out.rgba[0..4];
        for y in 0..4 {
            for x in 0..4 {
                let i = (y * 8 + x) * 4;
                assert_eq!(&out.rgba[i..i + 4], first);
            }
        }
        assert_ne!(&out.rgba[4 * 4..4 * 4 + 4], first);
    }

    #[test]
    fn spotlight_leaves_the_spot_and_dims_the_rest() {
        let mut img = Image {
            width: 10,
            height: 10,
            rgba: [200u8, 200, 200, 255].repeat(100),
        };
        let spot = Rect::from_min_size(pos2(2.0, 2.0), vec2(6.0, 6.0));
        spotlight(&mut img, &[(spot, false)], 0.5);
        assert_eq!(
            &img.rgba[(5 * 10 + 5) * 4..(5 * 10 + 5) * 4 + 4],
            &[200, 200, 200, 255]
        );
        assert_eq!(&img.rgba[0..4], &[100, 100, 100, 255]);
    }

    #[test]
    fn regions_outside_the_image_are_ignored() {
        let img = gradient(4, 4);
        assert!(
            blur(
                &img,
                Rect::from_min_size(pos2(10.0, 10.0), vec2(5.0, 5.0)),
                2.0
            )
            .is_none()
        );
        assert_eq!(
            clamp_rect(Rect::from_min_size(pos2(-3.0, 1.0), vec2(5.0, 10.0)), 4, 4),
            Some((0, 1, 2, 3))
        );
    }
}
