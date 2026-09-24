//! Turning annotations into pictures, on screen and into the saved file.
//!
//! Both go through the same egui shapes. On screen egui draws them; for the
//! file they are tessellated by egui's own tessellator and filled into the
//! image by a small triangle rasteriser here. What is saved therefore looks
//! like what was on screen, down to the anti-aliasing, without pulling in a
//! second 2D library.

use std::sync::Arc;

use egui::epaint::{
    Color32, FontId, Fonts, Galley, Mesh, Shape as Paint, Stroke, StrokeKind, TessellationOptions,
    Tessellator, TextOptions, text::FontDefinitions,
};
use egui::{Pos2, Rect, Vec2, pos2, vec2};

use super::effects;
use super::model::{Annotation, Doc, HIGHLIGHTER_ALPHA, Shape};
use crate::platform::Image;

/// From image pixels to wherever the shapes are drawn.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    /// Where the image's top left corner lands.
    pub origin: Pos2,
    /// Target units per image pixel.
    pub scale: f32,
}

impl Mapping {
    pub const IDENTITY: Mapping = Mapping {
        origin: Pos2::ZERO,
        scale: 1.0,
    };

    pub fn pos(&self, p: Pos2) -> Pos2 {
        self.origin + p.to_vec2() * self.scale
    }

    pub fn rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.pos(r.min), self.pos(r.max))
    }
}

/// Lays out text at a size in target units.
pub type Layout<'a> = dyn FnMut(String, f32, Color32) -> Arc<Galley> + 'a;

/// Black or white, whichever reads better on `background`.
pub fn contrast(background: Color32) -> Color32 {
    let [r, g, b, _] = background.to_array();
    let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    if luma > 150.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Padding around text with a background, relative to the font size.
const TEXT_PAD: f32 = 0.25;

/// The shapes for one annotation, plus the size of its text in image pixels
/// where it has any. Effects and the spotlight have no shapes; they are
/// pixels and handled separately.
pub fn shapes(item: &Annotation, map: &Mapping, layout: &mut Layout) -> (Vec<Paint>, Option<Vec2>) {
    let s = map.scale;
    let style = &item.style;
    let mut out = Vec::new();
    let mut text_size = None;
    let stroke = Stroke::new(style.width * s, style.color);

    match &item.shape {
        Shape::Rectangle(r) => {
            let r = map.rect(*r);
            if let Some(fill) = style.fill {
                out.push(Paint::rect_filled(r, 0.0, fill));
            }
            out.push(Paint::rect_stroke(r, 0.0, stroke, StrokeKind::Middle));
        }
        Shape::Ellipse(r) => {
            let r = map.rect(*r);
            if let Some(fill) = style.fill {
                out.push(Paint::ellipse_filled(r.center(), r.size() / 2.0, fill));
            }
            out.push(Paint::ellipse_stroke(r.center(), r.size() / 2.0, stroke));
        }
        Shape::Line(a, b) => {
            let (a, b) = (map.pos(*a), map.pos(*b));
            out.push(Paint::line_segment([a, b], stroke));
            out.push(Paint::circle_filled(a, stroke.width / 2.0, style.color));
            out.push(Paint::circle_filled(b, stroke.width / 2.0, style.color));
        }
        Shape::Arrow(a, b) => {
            let (a, b) = (map.pos(*a), map.pos(*b));
            let length = (b - a).length();
            if length > 0.5 {
                let dir = (b - a) / length;
                let normal = vec2(-dir.y, dir.x);
                let head = (item.head_length() * s).min(length);
                let half = head * 0.55;
                let base = b - dir * head;
                // The shaft stops inside the head, so its flat end never
                // pokes out beside the tip.
                let shaft_end = b - dir * (head * 0.6);
                if (shaft_end - a).dot(dir) > 0.0 {
                    out.push(Paint::line_segment([a, shaft_end], stroke));
                    out.push(Paint::circle_filled(a, stroke.width / 2.0, style.color));
                }
                out.push(Paint::convex_polygon(
                    vec![b, base + normal * half, base - normal * half],
                    style.color,
                    Stroke::NONE,
                ));
            }
        }
        Shape::Pen(points) => {
            let points: Vec<Pos2> = points.iter().map(|p| map.pos(*p)).collect();
            freehand(&mut out, points, stroke, true);
        }
        Shape::Highlighter(points) => {
            let points: Vec<Pos2> = points.iter().map(|p| map.pos(*p)).collect();
            let [r, g, b, _] = style.color.to_array();
            let color = Color32::from_rgba_unmultiplied(r, g, b, HIGHLIGHTER_ALPHA);
            // No round joins here: overlapping see-through discs would leave
            // darker spots along the stroke.
            freehand(
                &mut out,
                points,
                Stroke::new(item.stroke_width() * s, color),
                false,
            );
        }
        Shape::Text(at, text) => {
            let shown = if text.is_empty() { " " } else { text.as_str() };
            let galley = layout(shown.to_string(), style.font_size * s, style.color);
            let pos = map.pos(*at);
            let size = galley.size();
            if let Some(fill) = style.fill {
                let pad = style.font_size * TEXT_PAD * s;
                out.push(Paint::rect_filled(
                    Rect::from_min_size(pos, size).expand(pad),
                    pad,
                    fill,
                ));
            }
            text_size = Some(size / s);
            out.push(Paint::galley(pos, galley, style.color));
        }
        Shape::Counter(center, number) => {
            let c = map.pos(*center);
            let radius = item.counter_radius() * s;
            out.push(Paint::circle_filled(c, radius, style.color));
            let galley = layout(
                number.to_string(),
                style.font_size * 0.9 * s,
                contrast(style.color),
            );
            let size = galley.size();
            out.push(Paint::galley(c - size / 2.0, galley, Color32::WHITE));
        }
        Shape::Redact(r) => {
            out.push(Paint::rect_filled(map.rect(*r), 0.0, Color32::BLACK));
        }
        Shape::Spotlight(_) | Shape::Blur(_) | Shape::Pixelate(_) => {}
    }
    (out, text_size)
}

fn freehand(out: &mut Vec<Paint>, points: Vec<Pos2>, stroke: Stroke, round_joins: bool) {
    match points.len() {
        0 => {}
        1 => out.push(Paint::circle_filled(
            points[0],
            stroke.width / 2.0,
            stroke.color,
        )),
        _ => {
            let caps: Vec<Pos2> = if round_joins { points.clone() } else { vec![] };
            out.push(Paint::line(points, stroke));
            for p in caps {
                out.push(Paint::circle_filled(p, stroke.width / 2.0, stroke.color));
            }
        }
    }
}

/// Text rendering for export, independent of the screen's scaling.
pub fn export_fonts() -> Fonts {
    Fonts::new(TextOptions::default(), FontDefinitions::default())
}

/// Burn the edit into a copy of `base` and crop it.
pub fn export(base: &Image, doc: &Doc, fonts: &mut Fonts) -> Image {
    let mut out = Image {
        width: base.width,
        height: base.height,
        rgba: base.rgba.clone(),
    };
    let mut spots = Vec::new();
    let mut dim: f32 = 0.0;

    for item in &doc.items {
        match &item.shape {
            Shape::Blur(r) => {
                if let Some((x, y, patch)) = effects::blur(base, *r, item.style.strength) {
                    effects::paste(&mut out, &patch, x, y);
                }
            }
            Shape::Pixelate(r) => {
                if let Some((x, y, patch)) = effects::pixelate(base, *r, item.style.strength) {
                    effects::paste(&mut out, &patch, x, y);
                }
            }
            Shape::Spotlight(r) => {
                spots.push((*r, item.style.round));
                dim = dim.max(item.style.strength);
            }
            _ => {
                let (paints, _) = shapes(item, &Mapping::IDENTITY, &mut |text, size, color| {
                    fonts.with_pixels_per_point(1.0).layout_no_wrap(
                        text,
                        FontId::proportional(size),
                        color,
                    )
                });
                rasterize(&mut out, paints, fonts);
            }
        }
    }
    // The spotlight dims everything else, drawings included.
    effects::spotlight(&mut out, &spots, dim);

    match doc
        .crop
        .and_then(|c| effects::clamp_rect(c, out.width, out.height))
    {
        Some((x, y, w, h)) if (w, h) != (out.width, out.height) => crop(&out, x, y, w, h),
        _ => out,
    }
}

fn crop(src: &Image, x: u32, y: u32, w: u32, h: u32) -> Image {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * src.width + x) * 4) as usize;
        rgba.extend_from_slice(&src.rgba[start..start + (w * 4) as usize]);
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

/// Tessellate shapes at one pixel per point and fill them into `dst`.
pub fn rasterize(dst: &mut Image, paints: Vec<Paint>, fonts: &Fonts) {
    let atlas = fonts.texture_atlas();
    let options = TessellationOptions {
        // Discs would otherwise be looked up in a part of the atlas this
        // font set never prepared.
        prerasterized_discs: false,
        ..TessellationOptions::default()
    };
    let mut tessellator = Tessellator::new(1.0, options, atlas.size(), Vec::new());
    let mut mesh = Mesh::default();
    for paint in paints {
        tessellator.tessellate_shape(paint, &mut mesh);
    }
    fill_mesh(dst, &mesh, atlas.image());
}

fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Whether a pixel exactly on an edge belongs to this triangle. Two triangles
/// sharing an edge see it in opposite directions, so exactly one of them
/// takes such a pixel and nothing is blended twice.
fn owns_edge(a: Pos2, b: Pos2) -> bool {
    let e = b - a;
    e.y > 0.0 || (e.y == 0.0 && e.x < 0.0)
}

/// Fill a triangle mesh into `dst`. Vertex colours are premultiplied, the
/// atlas supplies coverage for text; blending is ordinary "over".
pub fn fill_mesh(dst: &mut Image, mesh: &Mesh, atlas: &egui::ColorImage) {
    let (tw, th) = (atlas.size[0], atlas.size[1]);
    let texel = |uv: Pos2| -> [f32; 4] {
        let x = ((uv.x * tw as f32) as usize).min(tw.saturating_sub(1));
        let y = ((uv.y * th as f32) as usize).min(th.saturating_sub(1));
        let c = atlas.pixels[y * tw + x];
        [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32].map(|v| v / 255.0)
    };

    for tri in mesh.indices.chunks_exact(3) {
        let mut v = [
            mesh.vertices[tri[0] as usize],
            mesh.vertices[tri[1] as usize],
            mesh.vertices[tri[2] as usize],
        ];
        let mut area = edge(v[0].pos, v[1].pos, v[2].pos);
        if area.abs() < 1e-9 {
            continue;
        }
        if area < 0.0 {
            v.swap(1, 2);
            area = -area;
        }
        let (a, b, c) = (v[0].pos, v[1].pos, v[2].pos);
        let x0 = a.x.min(b.x).min(c.x).floor().max(0.0) as i64;
        let y0 = a.y.min(b.y).min(c.y).floor().max(0.0) as i64;
        let x1 = (a.x.max(b.x).max(c.x).ceil() as i64).min(dst.width as i64);
        let y1 = (a.y.max(b.y).max(c.y).ceil() as i64).min(dst.height as i64);
        let own = [owns_edge(b, c), owns_edge(c, a), owns_edge(a, b)];
        let colors = v.map(|vert| {
            let [r, g, b, a] = vert.color.to_array();
            [r as f32, g as f32, b as f32, a as f32].map(|x| x / 255.0)
        });

        for py in y0..y1 {
            for px in x0..x1 {
                let p = pos2(px as f32 + 0.5, py as f32 + 0.5);
                let w = [edge(b, c, p), edge(c, a, p), edge(a, b, p)];
                let inside = w
                    .iter()
                    .zip(own)
                    .all(|(w, own)| *w > 0.0 || (*w == 0.0 && own));
                if !inside {
                    continue;
                }
                let l = w.map(|x| x / area);
                let uv = pos2(
                    v[0].uv.x * l[0] + v[1].uv.x * l[1] + v[2].uv.x * l[2],
                    v[0].uv.y * l[0] + v[1].uv.y * l[1] + v[2].uv.y * l[2],
                );
                let t = texel(uv);
                let mut src = [0.0f32; 4];
                for (k, value) in src.iter_mut().enumerate() {
                    *value =
                        (colors[0][k] * l[0] + colors[1][k] * l[1] + colors[2][k] * l[2]) * t[k];
                }
                blend(dst, px as u32, py as u32, src);
            }
        }
    }
}

/// Premultiplied `src` over the straight RGBA pixel at `x`, `y`.
fn blend(dst: &mut Image, x: u32, y: u32, src: [f32; 4]) {
    let sa = src[3].clamp(0.0, 1.0);
    if sa <= 0.0 && src[..3].iter().all(|v| *v <= 0.0) {
        return;
    }
    let i = ((y * dst.width + x) * 4) as usize;
    let d = &mut dst.rgba[i..i + 4];
    let da = d[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    for k in 0..3 {
        let dc = d[k] as f32 / 255.0 * da;
        let premult = src[k] + dc * (1.0 - sa);
        let straight = if out_a > 0.0 { premult / out_a } else { 0.0 };
        d[k] = (straight * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    d[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::model::Style;

    fn canvas(w: u32, h: u32) -> Image {
        Image {
            width: w,
            height: h,
            rgba: [255u8, 255, 255, 255].repeat((w * h) as usize),
        }
    }

    fn px(img: &Image, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * img.width + x) * 4) as usize;
        [
            img.rgba[i],
            img.rgba[i + 1],
            img.rgba[i + 2],
            img.rgba[i + 3],
        ]
    }

    #[test]
    fn a_see_through_fill_is_even_across_its_diagonal() {
        let mut img = canvas(20, 20);
        let fonts = export_fonts();
        let fill = Color32::from_rgba_unmultiplied(0, 0, 0, 128);
        rasterize(
            &mut img,
            vec![Paint::rect_filled(
                Rect::from_min_size(pos2(2.0, 2.0), vec2(16.0, 16.0)),
                0.0,
                fill,
            )],
            &fonts,
        );
        let centre = px(&img, 10, 10);
        let diagonal = px(&img, 7, 7);
        let off = px(&img, 12, 5);
        assert_eq!(centre, diagonal);
        assert_eq!(centre, off);
        assert!(centre[0] > 110 && centre[0] < 140, "{centre:?}");
        assert_eq!(px(&img, 0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn exported_text_and_shapes_leave_marks() {
        let base = canvas(200, 80);
        let mut doc = Doc::default();
        let style = Style {
            color: Color32::from_rgb(200, 0, 0),
            ..Style::default()
        };
        doc.items.push(Annotation {
            shape: Shape::Text(pos2(10.0, 10.0), "Hi".into()),
            style,
        });
        doc.items.push(Annotation {
            shape: Shape::Rectangle(Rect::from_min_size(pos2(120.0, 10.0), vec2(60.0, 40.0))),
            style,
        });
        let mut fonts = export_fonts();
        let out = export(&base, &doc, &mut fonts);
        let red_in = |x0: u32, x1: u32| {
            (10..60).any(|y| {
                (x0..x1).any(|x| {
                    let p = px(&out, x, y);
                    p[0] > 150 && p[1] < 80
                })
            })
        };
        assert!(red_in(10, 60), "text left no pixels");
        assert!(red_in(118, 124), "rectangle outline missing");
        // The rectangle has no fill.
        assert_eq!(px(&out, 150, 30), [255, 255, 255, 255]);
    }

    #[test]
    fn crop_and_redaction_end_up_in_the_file() {
        let base = canvas(50, 40);
        let doc = Doc {
            items: vec![Annotation {
                shape: Shape::Redact(Rect::from_min_size(pos2(0.0, 0.0), vec2(20.0, 20.0))),
                style: Style::default(),
            }],
            crop: Some(Rect::from_min_size(pos2(10.0, 10.0), vec2(30.0, 20.0))),
        };
        let out = export(&base, &doc, &mut export_fonts());
        assert_eq!((out.width, out.height), (30, 20));
        assert_eq!(px(&out, 2, 2), [0, 0, 0, 255]);
        assert_eq!(px(&out, 20, 15), [255, 255, 255, 255]);
    }
}
