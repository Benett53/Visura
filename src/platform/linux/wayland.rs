//! Window outlines on Wayland.
//!
//! Wayland keeps window positions to the compositor, and there is no common
//! protocol for asking. Two compositors answer on the command line, and those
//! are used: Hyprland (`hyprctl`) and Sway (`swaymsg`). Everywhere else the
//! list stays empty and the overlay simply offers no outlines, as before.
//!
//! Both report logical coordinates, while the frozen frame from the screenshot
//! helper is in pixels. The layout is scaled onto the frame by comparing the
//! two sizes, which is exact for a single scale factor across all monitors.

use serde_json::Value;

use super::super::{Rect, WindowInfo};

/// A window as the compositor reports it, in logical coordinates.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    rect: Rect,
    title: String,
    class: String,
    pid: Option<u32>,
}

/// Windows front to back plus the logical bounds of all monitors together.
#[derive(Debug, Clone, PartialEq)]
struct Layout {
    windows: Vec<Entry>,
    bounds: Rect,
}

/// The windows on screen, front most first, in the pixels of a frame of
/// `frame` size taken by the screenshot helper.
pub fn windows(frame: (u32, u32)) -> Vec<WindowInfo> {
    let layout = if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        hyprland()
    } else if std::env::var_os("SWAYSOCK").is_some() {
        sway()
    } else {
        None
    };
    layout
        .map(|layout| to_screen(&layout, frame))
        .unwrap_or_default()
}

fn json(program: &str, args: &[&str]) -> Option<Value> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

fn hyprland() -> Option<Layout> {
    let clients = json("hyprctl", &["clients", "-j"])?;
    let monitors = json("hyprctl", &["monitors", "-j"])?;
    parse_hyprland(&clients, &monitors)
}

fn sway() -> Option<Layout> {
    parse_sway(&json("swaymsg", &["-t", "get_tree", "-r"])?)
}

fn int(value: &Value) -> i32 {
    value.as_f64().unwrap_or(0.0).round() as i32
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

/// Hyprland: `hyprctl clients -j` and `hyprctl monitors -j`.
fn parse_hyprland(clients: &Value, monitors: &Value) -> Option<Layout> {
    let mut bounds: Option<Rect> = None;
    let mut shown_workspaces = Vec::new();
    for monitor in monitors.as_array()? {
        let scale = monitor["scale"]
            .as_f64()
            .filter(|s| *s > 0.0)
            .unwrap_or(1.0);
        let (mut w, mut h) = (monitor["width"].as_f64()?, monitor["height"].as_f64()?);
        // Odd transforms are the rotated ones.
        if monitor["transform"].as_i64().unwrap_or(0) % 2 == 1 {
            std::mem::swap(&mut w, &mut h);
        }
        let rect = Rect::new(
            int(&monitor["x"]),
            int(&monitor["y"]),
            (w / scale).round() as i32,
            (h / scale).round() as i32,
        );
        bounds = Some(bounds.map_or(rect, |b| union(b, rect)));
        for key in ["activeWorkspace", "specialWorkspace"] {
            if let Some(id) = monitor[key]["id"].as_i64().filter(|id| *id != 0) {
                shown_workspaces.push(id);
            }
        }
    }

    let mut ranked = Vec::new();
    for client in clients.as_array()? {
        let visible = client["mapped"].as_bool().unwrap_or(true)
            && !client["hidden"].as_bool().unwrap_or(false)
            && client["workspace"]["id"]
                .as_i64()
                .is_some_and(|id| shown_workspaces.contains(&id));
        if !visible {
            continue;
        }
        // Newer versions report the fullscreen state as a number.
        let fullscreen = client["fullscreen"].as_bool().unwrap_or(false)
            || client["fullscreen"].as_i64().unwrap_or(0) > 0;
        let group = if fullscreen {
            0
        } else if client["floating"].as_bool().unwrap_or(false) {
            1
        } else {
            2
        };
        let recency = client["focusHistoryID"].as_i64().unwrap_or(i64::MAX);
        let entry = Entry {
            rect: Rect::new(
                int(&client["at"][0]),
                int(&client["at"][1]),
                int(&client["size"][0]),
                int(&client["size"][1]),
            ),
            title: text(&client["title"]),
            class: text(&client["class"]),
            pid: client["pid"].as_u64().map(|p| p as u32),
        };
        ranked.push(((group, recency), entry));
    }
    ranked.sort_by_key(|(rank, _)| *rank);
    Some(Layout {
        windows: ranked.into_iter().map(|(_, entry)| entry).collect(),
        bounds: bounds?,
    })
}

/// Sway: `swaymsg -t get_tree -r`. The root's rectangle spans all outputs.
fn parse_sway(tree: &Value) -> Option<Layout> {
    fn walk(node: &Value, focus_rank: usize, out: &mut Vec<((u8, usize), Entry)>) {
        let is_view = node["pid"].is_u64()
            && node["nodes"].as_array().is_none_or(|n| n.is_empty())
            && node["floating_nodes"]
                .as_array()
                .is_none_or(|n| n.is_empty());
        if is_view {
            if node["visible"].as_bool().unwrap_or(false) {
                let group = if node["fullscreen_mode"].as_i64().unwrap_or(0) > 0 {
                    0
                } else if node["type"].as_str() == Some("floating_con") {
                    1
                } else {
                    2
                };
                let class = node["app_id"]
                    .as_str()
                    .or_else(|| node["window_properties"]["class"].as_str())
                    .unwrap_or_default()
                    .to_string();
                let rect = &node["rect"];
                out.push((
                    (group, focus_rank),
                    Entry {
                        rect: Rect::new(
                            int(&rect["x"]),
                            int(&rect["y"]),
                            int(&rect["width"]),
                            int(&rect["height"]),
                        ),
                        title: text(&node["name"]),
                        class,
                        pid: node["pid"].as_u64().map(|p| p as u32),
                    },
                ));
            }
            return;
        }
        // `focus` lists the children's ids, most recently focused first.
        let focus: Vec<i64> = node["focus"]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        for key in ["nodes", "floating_nodes"] {
            for child in node[key].as_array().into_iter().flatten() {
                let rank = child["id"]
                    .as_i64()
                    .and_then(|id| focus.iter().position(|f| *f == id))
                    .unwrap_or(usize::MAX);
                walk(child, rank, out);
            }
        }
    }

    let root = &tree["rect"];
    let bounds = Rect::new(
        int(&root["x"]),
        int(&root["y"]),
        int(&root["width"]),
        int(&root["height"]),
    );
    if bounds.is_empty() {
        return None;
    }
    let mut ranked = Vec::new();
    walk(tree, 0, &mut ranked);
    ranked.sort_by_key(|(rank, _)| *rank);
    Some(Layout {
        windows: ranked.into_iter().map(|(_, entry)| entry).collect(),
        bounds,
    })
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect::new(
        x,
        y,
        a.right().max(b.right()) - x,
        a.bottom().max(b.bottom()) - y,
    )
}

/// Scale the logical layout onto the helper's frame.
fn to_screen(layout: &Layout, (width, height): (u32, u32)) -> Vec<WindowInfo> {
    let bounds = layout.bounds;
    if bounds.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }
    let sx = width as f64 / bounds.w as f64;
    let sy = height as f64 / bounds.h as f64;
    let frame = Rect::new(0, 0, width as i32, height as i32);
    layout
        .windows
        .iter()
        .filter_map(|entry| {
            let r = entry.rect;
            let x0 = ((r.x - bounds.x) as f64 * sx).round() as i32;
            let y0 = ((r.y - bounds.y) as f64 * sy).round() as i32;
            let x1 = ((r.x + r.w - bounds.x) as f64 * sx).round() as i32;
            let y1 = ((r.y + r.h - bounds.y) as f64 * sy).round() as i32;
            let rect = Rect::new(x0, y0, x1 - x0, y1 - y0).intersect(&frame);
            if rect.w < 8 || rect.h < 8 {
                return None;
            }
            let app = entry
                .pid
                .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| entry.class.clone());
            Some(WindowInfo {
                rect,
                title: entry.title.clone(),
                app,
                areas: Vec::new(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_orders_floating_above_tiled_and_skips_hidden_workspaces() {
        let monitors = serde_json::json!([
            {"x": 0, "y": 0, "width": 3840, "height": 2160, "scale": 2.0, "transform": 0,
             "activeWorkspace": {"id": 1}, "specialWorkspace": {"id": 0}}
        ]);
        let clients = serde_json::json!([
            {"at": [0, 0], "size": [960, 1080], "mapped": true, "hidden": false,
             "workspace": {"id": 1}, "floating": false, "fullscreen": 0,
             "title": "tiled", "class": "foot", "focusHistoryID": 0},
            {"at": [100, 100], "size": [400, 300], "mapped": true, "hidden": false,
             "workspace": {"id": 1}, "floating": true, "fullscreen": false,
             "title": "float", "class": "pavucontrol", "focusHistoryID": 3},
            {"at": [0, 0], "size": [1920, 1080], "mapped": true, "hidden": false,
             "workspace": {"id": 2}, "floating": false, "fullscreen": 0,
             "title": "elsewhere", "class": "firefox", "focusHistoryID": 1}
        ]);
        let layout = parse_hyprland(&clients, &monitors).unwrap();
        assert_eq!(layout.bounds, Rect::new(0, 0, 1920, 1080));
        let titles: Vec<&str> = layout.windows.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, ["float", "tiled"]);

        // A scale of 2: the frame has twice the logical size.
        let shown = to_screen(&layout, (3840, 2160));
        assert_eq!(shown[0].rect, Rect::new(200, 200, 800, 600));
        assert_eq!(shown[0].app, "pavucontrol");
    }

    #[test]
    fn sway_takes_visible_views_with_floating_first() {
        let tree = serde_json::json!({
            "id": 1, "type": "root", "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440},
            "focus": [2],
            "nodes": [{
                "id": 2, "type": "output", "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440},
                "focus": [3],
                "nodes": [{
                    "id": 3, "type": "workspace", "focus": [5, 4],
                    "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440},
                    "nodes": [{
                        "id": 4, "type": "con", "pid": 10, "visible": true, "name": "editor",
                        "app_id": "code", "fullscreen_mode": 0,
                        "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440},
                        "nodes": [], "floating_nodes": []
                    }],
                    "floating_nodes": [{
                        "id": 5, "type": "floating_con", "pid": 11, "visible": true,
                        "name": "calc", "app_id": null,
                        "window_properties": {"class": "Galculator"}, "fullscreen_mode": 0,
                        "rect": {"x": 1000, "y": 500, "width": 300, "height": 400},
                        "nodes": [], "floating_nodes": []
                    }]
                }]
            }]
        });
        let layout = parse_sway(&tree).unwrap();
        let names: Vec<(&str, &str)> = layout
            .windows
            .iter()
            .map(|w| (w.title.as_str(), w.class.as_str()))
            .collect();
        assert_eq!(names, [("calc", "Galculator"), ("editor", "code")]);
    }
}
