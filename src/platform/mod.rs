//! Everything that differs between Windows and Linux lives behind this module.
//!
//! The rest of the program only sees plain data: rectangles, RGBA buffers and
//! a list of windows sorted front to back. Adding a third platform means
//! adding a file here and nothing else.

use std::path::PathBuf;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(all(unix, not(target_os = "macos")))]
pub use self::linux::*;

/// A rectangle in physical screen pixels. The origin is the top left of the
/// virtual desktop, which can be negative on multi monitor setups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Build a rectangle from two corners in any order.
    pub fn from_corners(a: (i32, i32), b: (i32, i32)) -> Self {
        let x = a.0.min(b.0);
        let y = a.1.min(b.1);
        Self {
            x,
            y,
            w: (a.0 - b.0).abs(),
            h: (a.1 - b.1).abs(),
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// Clip to `other`, returning an empty rectangle when they do not overlap.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        Rect {
            x,
            y,
            w: (r - x).max(0),
            h: (b - y).max(0),
        }
    }
}

/// An RGBA8 image, row major, no padding.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 255];
        }
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// Copy a sub rectangle out of the image. The rectangle is in image
    /// coordinates and is clipped to the image bounds.
    pub fn crop(&self, r: Rect) -> Image {
        let r = r.intersect(&Rect::new(0, 0, self.width as i32, self.height as i32));
        let mut rgba = Vec::with_capacity((r.w * r.h * 4).max(0) as usize);
        for row in 0..r.h {
            let start = (((r.y + row) as u32 * self.width + r.x as u32) * 4) as usize;
            rgba.extend_from_slice(&self.rgba[start..start + (r.w as usize) * 4]);
        }
        Image {
            width: r.w.max(0) as u32,
            height: r.h.max(0) as u32,
            rgba,
        }
    }
}

/// A top level window as the overlay sees it.
#[derive(Clone, Debug)]
pub struct WindowInfo {
    /// Visible bounds, shadows already removed on Windows.
    pub rect: Rect,
    pub title: String,
    /// Executable name without extension, used for the `%app` token.
    pub app: String,
}

/// Wall clock time in the machine's own time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

impl LocalTime {
    /// Days since the epoch, used to group the history by day.
    pub fn day_number(&self) -> i64 {
        // Howard Hinnant's days_from_civil.
        let y = if self.month <= 2 {
            self.year - 1
        } else {
            self.year
        } as i64;
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let m = self.month as i64;
        let d = self.day as i64;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
}

/// Fall back to a sensible picture folder when the configuration is empty.
pub fn default_screenshot_dir() -> PathBuf {
    pictures_dir().join("Visura")
}
