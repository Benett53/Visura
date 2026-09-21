//! Turning a pattern plus the facts of a capture into a path.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::platform::LocalTime;

/// What the pattern tokens can refer to.
pub struct Context<'a> {
    pub time: LocalTime,
    pub window_title: &'a str,
    pub app: &'a str,
    pub width: u32,
    pub height: u32,
}

/// The tokens, in the order the settings screen lists them.
pub const TOKENS: &[(&str, &str)] = &[
    ("%app", "program the window belongs to"),
    ("%win", "window title"),
    ("%Y", "year, four digits"),
    ("%y", "year, two digits"),
    ("%m", "month"),
    ("%d", "day"),
    ("%H", "hour"),
    ("%M", "minute"),
    ("%S", "second"),
    ("%ms", "millisecond"),
    ("%w", "width in pixels"),
    ("%h", "height in pixels"),
];

/// Expand a pattern. Unknown tokens are left as they are rather than silently
/// dropped, so a typo shows up in the file name instead of disappearing.
pub fn expand(pattern: &str, ctx: &Context<'_>) -> String {
    let mut out = String::with_capacity(pattern.len() + 16);
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let rest: String = chars[i + 1..].iter().collect();
        // Longest match first, otherwise %ms would be read as %m followed by s.
        let matched = [
            ("ms", format!("{:03}", ctx.time.millis)),
            ("app", sanitise(ctx.app)),
            ("win", sanitise(ctx.window_title)),
            ("Y", format!("{:04}", ctx.time.year)),
            ("y", format!("{:02}", ctx.time.year.rem_euclid(100))),
            ("m", format!("{:02}", ctx.time.month)),
            ("d", format!("{:02}", ctx.time.day)),
            ("H", format!("{:02}", ctx.time.hour)),
            ("M", format!("{:02}", ctx.time.minute)),
            ("S", format!("{:02}", ctx.time.second)),
            ("w", ctx.width.to_string()),
            ("h", ctx.height.to_string()),
            ("%", "%".to_string()),
        ]
        .into_iter()
        .find(|(token, _)| rest.starts_with(token));

        match matched {
            Some((token, value)) => {
                out.push_str(&value);
                i += 1 + token.chars().count();
            }
            None => {
                out.push('%');
                i += 1;
            }
        }
    }
    out
}

/// Make a piece of text safe for a file name on both platforms.
///
/// Windows is the stricter of the two, so its rules are applied everywhere and
/// the same shot gets the same name on either system.
pub fn sanitise(text: &str) -> String {
    const FORBIDDEN: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = false;
    for c in text.chars() {
        let c = if FORBIDDEN.contains(&c) || c.is_control() {
            ' '
        } else {
            c
        };
        if c == ' ' {
            if last_was_space {
                continue;
            }
            last_was_space = true;
        } else {
            last_was_space = false;
        }
        out.push(c);
        if out.chars().count() >= 60 {
            break;
        }
    }
    // Trailing dots and spaces are legal to create but a nuisance on Windows.
    out.trim().trim_end_matches('.').trim().to_string()
}

/// Tidy up the separators an empty token leaves behind.
///
/// The default pattern is `%app_%Y-%m-%d`. A full screen shot has no program
/// to name, and `_2026-09-21.png` would be the result. Collapsing runs of
/// separators and trimming the ends keeps that readable.
fn tidy_separators(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last: Option<char> = None;
    for c in name.chars() {
        let is_sep = c == '_' || c == '-';
        if is_sep && last == Some(c) {
            continue;
        }
        if is_sep && out.is_empty() {
            continue;
        }
        out.push(c);
        last = Some(c);
    }
    out.trim_matches(['_', '-', ' ']).to_string()
}

/// A name that says nothing: twelve characters of lowercase letters and
/// digits, no date, no program, no window title.
pub fn anonymous_name() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    // A screenshot name does not need cryptographic randomness, it needs to
    // not collide and not leak. The clock plus a counter gives both without
    // pulling in a random number generator.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut state = nanos
        ^ (COUNTER
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15));
    if state == 0 {
        state = 0x2545_f491_4f6c_dd1d;
    }

    let mut name = String::with_capacity(12);
    for _ in 0..12 {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        name.push(ALPHABET[(state % ALPHABET.len() as u64) as usize] as char);
    }
    name
}

/// Build the full target path and make sure nothing is overwritten.
///
/// A counter is appended rather than a random suffix: two shots of the same
/// program on the same day stay next to each other and stay in order.
pub fn target_path(
    root: &Path,
    subfolder_pattern: &str,
    filename_pattern: &str,
    extension: &str,
    anonymous: bool,
    ctx: &Context<'_>,
) -> PathBuf {
    // The folder structure stays either way. Anonymous is about what a
    // file name gives away to whoever ends up looking at the file, not
    // about hiding the library from its owner, who still wants to find
    // last month's shots.
    let mut dir = root.to_path_buf();
    let sub = expand(subfolder_pattern, ctx);
    for part in sub.split(['/', '\\']) {
        let part = sanitise(part);
        if !part.is_empty() {
            dir.push(part);
        }
    }

    let stem = if anonymous {
        anonymous_name()
    } else {
        let cleaned = tidy_separators(&sanitise(&expand(filename_pattern, ctx)));
        if cleaned.is_empty() {
            "screenshot".to_string()
        } else {
            cleaned
        }
    };

    let mut candidate = dir.join(format!("{stem}.{extension}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}_{n}.{extension}"));
        n += 1;
        if n > 9999 {
            break;
        }
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context<'static> {
        Context {
            time: LocalTime {
                year: 2026,
                month: 3,
                day: 7,
                hour: 9,
                minute: 4,
                second: 5,
                millis: 42,
            },
            window_title: "Notepad: a/b*c",
            app: "notepad",
            width: 1920,
            height: 1080,
        }
    }

    fn ctx_without_app() -> Context<'static> {
        Context {
            app: "",
            window_title: "",
            ..ctx()
        }
    }

    #[test]
    fn the_default_pattern_names_the_program_then_the_date() {
        assert_eq!(expand("%app_%Y-%m-%d", &ctx()), "notepad_2026-03-07");
    }

    #[test]
    fn ms_wins_over_m_followed_by_s() {
        assert_eq!(expand("%ms", &ctx()), "042");
        assert_eq!(expand("%m%s", &ctx()), "03%s");
    }

    #[test]
    fn window_titles_lose_forbidden_characters() {
        assert_eq!(expand("%win", &ctx()), "Notepad a b c");
    }

    #[test]
    fn unknown_tokens_survive_visibly() {
        assert_eq!(expand("%Q-%Y", &ctx()), "%Q-2026");
    }

    #[test]
    fn a_double_percent_is_a_literal_percent() {
        assert_eq!(expand("100%%", &ctx()), "100%");
    }

    #[test]
    fn a_missing_program_does_not_leave_a_dangling_separator() {
        let expanded = expand("%app_%Y-%m-%d", &ctx_without_app());
        assert_eq!(expanded, "_2026-03-07");
        assert_eq!(tidy_separators(&expanded), "2026-03-07");
    }

    #[test]
    fn separators_are_never_doubled() {
        assert_eq!(tidy_separators("a__b---c_"), "a_b-c");
        assert_eq!(tidy_separators("___"), "");
    }

    #[test]
    fn sanitise_collapses_whitespace_and_trims() {
        assert_eq!(sanitise("  a \t b.. "), "a b");
    }

    #[test]
    fn sanitise_caps_the_length() {
        let long = "x".repeat(200);
        assert_eq!(sanitise(&long).chars().count(), 60);
    }

    #[test]
    fn anonymous_names_are_twelve_safe_characters() {
        let name = anonymous_name();
        assert_eq!(name.chars().count(), 12);
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn anonymous_names_do_not_repeat() {
        let names: std::collections::HashSet<String> = (0..500).map(|_| anonymous_name()).collect();
        assert_eq!(names.len(), 500);
    }

    #[test]
    fn anonymous_names_keep_the_dated_folder() {
        let root = std::env::temp_dir().join("visura-anon-test");
        let anon = target_path(&root, "%Y-%m", "%app_%Y-%m-%d", "png", true, &ctx());
        assert_eq!(anon.parent().unwrap(), root.join("2026-03"));

        let stem = anon.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(stem.chars().count(), 12);
        assert!(!stem.contains("notepad"));
        assert!(!stem.contains("2026"));
    }
}
