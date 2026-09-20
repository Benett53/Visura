//! Thumbnails for the history grid.
//!
//! Decoding happens on worker threads and the result is kept on disk, because
//! a library of a few thousand screenshots is several gigabytes and decoding
//! it again on every start would be the slowest thing the program does.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::SystemTime;

use crate::platform;

/// Longest edge of a cached thumbnail. Large enough that the biggest tile size
/// still looks sharp on a high DPI screen, small enough to stay cheap.
const THUMB_EDGE: u32 = 320;
const WORKERS: usize = 2;
/// How many decoded images stay in video memory before the oldest are dropped.
const TEXTURE_BUDGET: usize = 500;

struct Job {
    path: PathBuf,
    key: u64,
}

struct Done {
    path: PathBuf,
    image: Option<egui::ColorImage>,
}

pub struct Thumbs {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    ready: HashMap<PathBuf, egui::TextureHandle>,
    /// Files that failed to decode, so they are not retried on every frame.
    broken: HashSet<PathBuf>,
    pending: HashSet<PathBuf>,
    order: VecDeque<PathBuf>,
}

impl Thumbs {
    pub fn new() -> Self {
        let (job_tx, job_rx) = channel::<Job>();
        let (done_tx, done_rx) = channel::<Done>();
        let job_rx = std::sync::Arc::new(std::sync::Mutex::new(job_rx));

        for _ in 0..WORKERS {
            let job_rx = job_rx.clone();
            let done_tx = done_tx.clone();
            std::thread::Builder::new()
                .name("visura-thumbs".into())
                .spawn(move || {
                    loop {
                        let job = {
                            let Ok(guard) = job_rx.lock() else {
                                return;
                            };
                            match guard.recv() {
                                Ok(job) => job,
                                Err(_) => return,
                            }
                        };
                        let image = build(&job.path, job.key);
                        if done_tx
                            .send(Done {
                                path: job.path,
                                image,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                })
                .ok();
        }

        Self {
            jobs: job_tx,
            done: done_rx,
            ready: HashMap::new(),
            broken: HashSet::new(),
            pending: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    /// Take everything the workers finished. Returns true when something new
    /// arrived and the grid should be redrawn.
    pub fn collect(&mut self, ctx: &egui::Context) -> bool {
        let mut changed = false;
        while let Ok(done) = self.done.try_recv() {
            self.pending.remove(&done.path);
            match done.image {
                Some(image) => {
                    let handle = ctx.load_texture(
                        done.path.to_string_lossy(),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.order.push_back(done.path.clone());
                    self.ready.insert(done.path, handle);
                    changed = true;
                }
                None => {
                    self.broken.insert(done.path);
                }
            }
        }
        while self.order.len() > TEXTURE_BUDGET {
            if let Some(old) = self.order.pop_front() {
                self.ready.remove(&old);
            }
        }
        changed
    }

    /// The texture for a shot, queueing the work when it is not there yet.
    /// Only call this for tiles that are actually on screen.
    pub fn get(
        &mut self,
        path: &Path,
        modified: SystemTime,
        bytes: u64,
    ) -> Option<&egui::TextureHandle> {
        if self.broken.contains(path) {
            return None;
        }
        if !self.ready.contains_key(path) && self.pending.insert(path.to_path_buf()) {
            let _ = self.jobs.send(Job {
                path: path.to_path_buf(),
                key: cache_key(path, modified, bytes),
            });
        }
        self.ready.get(path)
    }

    /// Forget a file, so a replaced or deleted shot does not linger.
    pub fn forget(&mut self, path: &Path) {
        self.ready.remove(path);
        self.broken.remove(path);
        self.order.retain(|p| p != path);
    }

    pub fn is_busy(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// Identifies the exact bytes on disk. Editing a file in place changes its
/// size or its time, so the thumbnail is rebuilt without a stale hit.
fn cache_key(path: &Path, modified: SystemTime, bytes: u64) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |data: &[u8]| {
        for byte in data {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    };
    eat(path.to_string_lossy().as_bytes());
    let secs = modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    eat(&secs.to_le_bytes());
    eat(&bytes.to_le_bytes());
    hash
}

fn cache_path(key: u64) -> PathBuf {
    platform::cache_dir()
        .join("thumbs")
        .join(format!("{key:016x}.jpg"))
}

fn build(path: &Path, key: u64) -> Option<egui::ColorImage> {
    let cached = cache_path(key);
    if let Ok(bytes) = std::fs::read(&cached)
        && let Some(image) = decode(&bytes)
    {
        return Some(image);
    }

    let source = image::open(path).ok()?;
    let thumb = source.thumbnail(THUMB_EDGE, THUMB_EDGE).to_rgb8();

    // Best effort: a cache that cannot be written is not worth an error.
    if let Some(parent) = cached.parent()
        && std::fs::create_dir_all(parent).is_ok()
    {
        let mut encoded = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut encoded);
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 82);
        let written = image::ImageEncoder::write_image(
            encoder,
            thumb.as_raw(),
            thumb.width(),
            thumb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .is_ok();
        if written {
            let _ = std::fs::write(&cached, &encoded);
        }
    }

    Some(egui::ColorImage::from_rgb(
        [thumb.width() as usize, thumb.height() as usize],
        thumb.as_raw(),
    ))
}

fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory(bytes).ok()?.to_rgb8();
    Some(egui::ColorImage::from_rgb(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

/// Drop cached thumbnails that no longer belong to a file in the library.
pub fn sweep_cache(known: &HashSet<u64>) {
    let dir = platform::cache_dir().join("thumbs");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(key) = u64::from_str_radix(stem, 16) else {
            continue;
        };
        if !known.contains(&key) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// The cache key for a shot, so the caller can build the set for `sweep_cache`.
pub fn key_for(path: &Path, modified: SystemTime, bytes: u64) -> u64 {
    cache_key(path, modified, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_changes_with_every_input() {
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
        let later = now + std::time::Duration::from_secs(1);
        let a = cache_key(Path::new("/a.png"), now, 10);
        assert_ne!(a, cache_key(Path::new("/b.png"), now, 10));
        assert_ne!(a, cache_key(Path::new("/a.png"), later, 10));
        assert_ne!(a, cache_key(Path::new("/a.png"), now, 11));
        assert_eq!(a, cache_key(Path::new("/a.png"), now, 10));
    }

    #[test]
    fn cache_file_names_are_reversible() {
        let key = 0x1234_5678_9abc_def0u64;
        let path = cache_path(key);
        let stem = path.file_stem().unwrap().to_str().unwrap();
        assert_eq!(u64::from_str_radix(stem, 16).unwrap(), key);
    }
}
