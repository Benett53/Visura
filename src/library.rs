//! The screenshot library.
//!
//! There is no database and no index file. The folder is the truth: whatever
//! the user does to it with a file manager is what Visura shows on the next
//! refresh. That is the whole reason tidying up stays simple.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::platform::{self, LocalTime};

/// Upper bound on entries read in one scan. A folder with more shots than this
/// is almost certainly the wrong folder, and the list stays responsive.
const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 6;

pub const EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

#[derive(Clone, Debug)]
pub struct Shot {
    pub path: PathBuf,
    pub name: String,
    pub modified: SystemTime,
    pub taken: LocalTime,
    pub bytes: u64,
}

impl Shot {
    pub fn human_size(&self) -> String {
        let bytes = self.bytes as f64;
        if bytes >= 1024.0 * 1024.0 {
            format!("{:.1} MB", bytes / (1024.0 * 1024.0))
        } else {
            format!("{:.0} KB", bytes / 1024.0)
        }
    }

    pub fn clock(&self) -> String {
        format!("{:02}:{:02}", self.taken.hour, self.taken.minute)
    }
}

/// Read the whole library, newest first.
pub fn scan(root: &Path) -> Vec<Shot> {
    let mut shots = Vec::new();
    collect(root, 0, &mut shots);
    // Sorting by the file time rather than the name keeps the order right even
    // when the name pattern has been changed at some point.
    shots.sort_by_key(|s| std::cmp::Reverse(s.modified));
    shots
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<Shot>) {
    if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            subdirs.push(path);
            continue;
        }
        if !has_image_extension(&path) {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        out.push(Shot {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            taken: platform::local_from_system_time(modified),
            modified,
            bytes: meta.len(),
            path,
        });
        if out.len() >= MAX_ENTRIES {
            return;
        }
    }
    // Newest folders first, so the recent months are read before the archive.
    subdirs.sort();
    for sub in subdirs.into_iter().rev() {
        collect(&sub, depth + 1, out);
    }
}

pub fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Heading for a group of shots taken on the same day.
pub fn day_label(day: LocalTime, today: LocalTime) -> String {
    match today.day_number() - day.day_number() {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        n @ 2..=6 => format!("{n} days ago"),
        _ => format!("{:04}-{:02}-{:02}", day.year, day.month, day.day),
    }
}

/// Delete through the recycle bin or trash, never by unlinking.
pub fn delete(paths: &[PathBuf]) -> Result<(), String> {
    platform::move_to_trash(paths)
}

/// Remove empty folders left behind after a clean-up, so the library does not
/// slowly fill up with empty month directories.
pub fn prune_empty_dirs(root: &Path) {
    fn walk(dir: &Path, depth: usize, root: &Path) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        let mut empty = true;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !walk(&path, depth + 1, root) {
                    empty = false;
                }
            } else {
                empty = false;
            }
        }
        if empty && dir != root {
            return std::fs::remove_dir(dir).is_ok();
        }
        empty
    }
    walk(root, 0, root);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(year: i32, month: u32, day: u32) -> LocalTime {
        LocalTime {
            year,
            month,
            day,
            hour: 12,
            minute: 0,
            second: 0,
            millis: 0,
        }
    }

    #[test]
    fn extensions_are_matched_case_insensitively() {
        assert!(has_image_extension(Path::new("a.PNG")));
        assert!(has_image_extension(Path::new("a.jpeg")));
        assert!(!has_image_extension(Path::new("a.txt")));
        assert!(!has_image_extension(Path::new("a")));
    }

    #[test]
    fn day_labels_read_like_a_person_wrote_them() {
        let today = time(2026, 3, 10);
        assert_eq!(day_label(today, today), "Today");
        assert_eq!(day_label(time(2026, 3, 9), today), "Yesterday");
        assert_eq!(day_label(time(2026, 3, 7), today), "3 days ago");
        assert_eq!(day_label(time(2026, 2, 1), today), "2026-02-01");
    }

    #[test]
    fn day_numbers_survive_month_and_year_ends() {
        assert_eq!(
            time(2026, 3, 1).day_number() - time(2026, 2, 28).day_number(),
            1
        );
        assert_eq!(
            time(2027, 1, 1).day_number() - time(2026, 12, 31).day_number(),
            1
        );
        // 2028 is a leap year.
        assert_eq!(
            time(2028, 3, 1).day_number() - time(2028, 2, 28).day_number(),
            2
        );
    }

    #[test]
    fn scanning_finds_images_in_subfolders_newest_first() {
        let dir = std::env::temp_dir().join(format!("visura-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("2026-01")).unwrap();
        std::fs::write(dir.join("2026-01/a.png"), b"x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("b.jpg"), b"y").unwrap();
        std::fs::write(dir.join("notes.txt"), b"z").unwrap();

        let shots = scan(&dir);
        assert_eq!(shots.len(), 2);
        assert_eq!(shots[0].name, "b.jpg");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
