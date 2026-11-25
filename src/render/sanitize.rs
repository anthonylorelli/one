use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

const MAX_SEGMENT_CHARS: usize = 100;

/// Characters percent-encoded when building Markdown relative link destinations.
const LINK_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'#')
    .add(b'%')
    .add(b'(')
    .add(b')')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b']');

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Sanitize a single directory or file stem for cross-platform safety (macOS, Linux, Windows).
pub fn sanitize_segment(raw: &str, fallback: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    let mut last_was_space = false;

    for ch in raw.chars() {
        let mapped = match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        };

        if mapped.is_whitespace() {
            if !last_was_space {
                cleaned.push(' ');
                last_was_space = true;
            }
        } else {
            cleaned.push(mapped);
            last_was_space = false;
        }
    }

    let trimmed = cleaned.trim().trim_matches('.').trim();
    let mut result = if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.chars().take(MAX_SEGMENT_CHARS).collect::<String>()
    };

    // Trim trailing spaces/dots again after truncation
    while result.ends_with([' ', '.']) {
        result.pop();
    }
    if result.is_empty() {
        result = fallback.to_string();
    }

    // Neutralize Windows reserved device names (case-insensitive, ignoring extension)
    let stem_upper = result
        .split('.')
        .next()
        .unwrap_or(&result)
        .to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem_upper.as_str()) {
        result = format!("_{result}");
    }

    result
}

/// Sanitize an asset filename while preserving a clean file extension if present.
pub fn sanitize_filename(raw: &str, fallback_stem: &str, fallback_ext: Option<&str>) -> String {
    let leaf = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let (stem, ext) = match leaf.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.is_empty() && e.len() <= 16 => (s, Some(e)),
        _ => (leaf, fallback_ext),
    };

    let clean_stem = sanitize_segment(stem, fallback_stem);
    match ext {
        Some(e) => {
            let clean_ext: String = e
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .take(16)
                .collect();
            if clean_ext.is_empty() {
                clean_stem
            } else {
                format!("{clean_stem}.{clean_ext}")
            }
        }
        None => clean_stem,
    }
}

/// Allocate a unique case-insensitive name within a sibling set (`Notes`, `Notes (2)`, ...).
pub fn deduplicate_name(base: &str, used_lower: &mut HashSet<String>) -> String {
    let lower = base.to_ascii_lowercase();
    if used_lower.insert(lower) {
        return base.to_string();
    }

    let mut counter = 2usize;
    loop {
        let candidate = format!("{base} ({counter})");
        if used_lower.insert(candidate.to_ascii_lowercase()) {
            return candidate;
        }
        counter += 1;
    }
}

/// Compute a relative path from the directory containing `from_file` to `to_path`.
pub fn relative_path_from_file(from_file: &Path, to_path: &Path) -> PathBuf {
    let from_dir = from_file.parent().unwrap_or_else(|| Path::new(""));
    relative_path(from_dir, to_path)
}

/// Compute a relative path from directory `from_dir` to `to_path`.
pub fn relative_path(from_dir: &Path, to_path: &Path) -> PathBuf {
    let from_comps: Vec<Component<'_>> = from_dir.components().collect();
    let to_comps: Vec<Component<'_>> = to_path.components().collect();

    let mut common = 0usize;
    while common < from_comps.len()
        && common < to_comps.len()
        && from_comps[common] == to_comps[common]
    {
        common += 1;
    }

    let mut rel = PathBuf::new();
    for _ in common..from_comps.len() {
        rel.push("..");
    }
    for comp in &to_comps[common..] {
        rel.push(comp.as_os_str());
    }

    if rel.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        rel
    }
}

/// Format a relative path as a POSIX forward-slash URL-encoded Markdown link destination.
pub fn encode_markdown_path(path: &Path) -> String {
    path.components()
        .map(|comp| {
            let s = comp.as_os_str().to_string_lossy();
            utf8_percent_encode(&s, LINK_ENCODE_SET).to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_illegal_chars_and_reserved_names() {
        assert_eq!(
            sanitize_segment("Q1: Budget / Plan?", "Untitled"),
            "Q1- Budget - Plan-"
        );
        assert_eq!(sanitize_segment("   ...  ", "Untitled"), "Untitled");
        assert_eq!(sanitize_segment("CON", "Untitled"), "_CON");
        assert_eq!(sanitize_segment("nul.txt", "Untitled"), "_nul.txt");
    }

    #[test]
    fn sanitizes_asset_filenames_against_traversal() {
        assert_eq!(
            sanitize_filename("../../evil/CON.png", "asset", None),
            "_CON.png"
        );
        assert_eq!(
            sanitize_filename("photo.jpg", "image", Some("png")),
            "photo.jpg"
        );
        assert_eq!(sanitize_filename("", "image", Some("png")), "image.png");
    }

    #[test]
    fn computes_and_encodes_relative_paths() {
        let from = Path::new("Notebook/Section A/Page 1.md");
        let to = Path::new("Notebook/Section B/Target (v2).md");
        let rel = relative_path_from_file(from, to);
        assert_eq!(
            encode_markdown_path(&rel),
            "../Section%20B/Target%20%28v2%29.md"
        );
    }

    #[test]
    fn deduplicates_case_insensitively() {
        let mut used = HashSet::new();
        assert_eq!(deduplicate_name("Notes", &mut used), "Notes");
        assert_eq!(deduplicate_name("notes", &mut used), "notes (2)");
        assert_eq!(deduplicate_name("NOTES", &mut used), "NOTES (3)");
    }
}
