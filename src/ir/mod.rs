pub mod link_registry;

use chrono::{DateTime, Utc};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};

/// Top-level result produced by any [`crate::sources::OneNoteSource`].
#[derive(Debug, Clone, Default)]
pub struct SourceBundle {
    pub notebooks: Vec<Notebook>,
    pub warnings: Vec<ExportWarning>,
}

/// A non-fatal issue encountered during loading, link resolution, or rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportWarning {
    pub scope: String,
    pub message: String,
}

impl ExportWarning {
    pub fn new(scope: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            message: message.into(),
        }
    }
}

/// A OneNote notebook containing sections and nested section groups.
#[derive(Debug, Clone)]
pub struct Notebook {
    pub id: Option<String>,
    pub name: String,
    pub entries: Vec<SectionEntry>,
}

/// An entry inside a notebook or section group.
#[derive(Debug, Clone)]
pub enum SectionEntry {
    Section(Section),
    SectionGroup(SectionGroup),
}

/// A folder of sections and nested section groups.
#[derive(Debug, Clone)]
pub struct SectionGroup {
    pub id: Option<String>,
    pub name: String,
    pub entries: Vec<SectionEntry>,
}

/// A OneNote section containing an ordered list of pages.
#[derive(Debug, Clone)]
pub struct Section {
    pub id: Option<String>,
    pub name: String,
    pub pages: Vec<Page>,
}

/// A single OneNote page normalized into the canonical IR.
#[derive(Debug, Clone)]
pub struct Page {
    /// Page GUID (normalized lowercase hyphenated UUID when available).
    pub id: Option<String>,
    pub title: String,
    /// Subpage indentation depth (`0` = top-level, `1` = subpage, `2` = sub-subpage).
    pub level: u8,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub author: Option<String>,
    pub web_url: Option<String>,
    pub client_url: Option<String>,
    pub blocks: Vec<Block>,
    pub assets: Vec<Asset>,
}

/// An embedded binary asset (image or file attachment) owned by a page.
#[derive(Debug, Clone)]
pub struct Asset {
    /// Local identifier referenced by [`Block::Image`] or [`Block::Attachment`].
    pub id: String,
    /// Suggested filename (will be sanitized before writing).
    pub filename: String,
    pub data: Vec<u8>,
}

/// A semantic OneNote note tag attached to a paragraph, heading, list item, or table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteTag {
    pub label: String,
    pub is_checkbox: bool,
    pub completed: bool,
    pub kind: NoteTagKind,
}

/// Coarse classification for built-in OneNote tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteTagKind {
    ToDo,
    Important,
    Question,
    Idea,
    Critical,
    RememberForLater,
    Contact,
    Custom,
}

impl NoteTagKind {
    /// Infer a built-in kind from a tag label (case-insensitive).
    pub fn from_label(label: &str) -> Self {
        let lower = label.trim().to_ascii_lowercase();
        match lower.as_str() {
            "to do" | "to-do" | "todo" | "to do priority 1" | "to do priority 2" => Self::ToDo,
            "important" => Self::Important,
            "question" => Self::Question,
            "idea" => Self::Idea,
            "critical" => Self::Critical,
            "remember for later" | "remember-for-later" => Self::RememberForLater,
            "contact" | "phone number" | "address" => Self::Contact,
            _ => Self::Custom,
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            Self::ToDo => "☑️",
            Self::Important => "⭐",
            Self::Question => "❓",
            Self::Idea => "💡",
            Self::Critical => "❗",
            Self::RememberForLater => "📌",
            Self::Contact => "👤",
            Self::Custom => "🏷️",
        }
    }
}

/// A block-level element in a OneNote page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        inlines: Vec<Inline>,
        tags: Vec<NoteTag>,
    },
    Paragraph {
        inlines: Vec<Inline>,
        tags: Vec<NoteTag>,
    },
    List {
        ordered: bool,
        start: u32,
        items: Vec<ListItem>,
    },
    Table {
        headers: Option<Vec<Vec<Block>>>,
        rows: Vec<Vec<Vec<Block>>>,
        tags: Vec<NoteTag>,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    Image {
        asset_id: String,
        alt: String,
        hyperlink: Option<LinkTarget>,
        tags: Vec<NoteTag>,
    },
    Attachment {
        asset_id: String,
        filename: String,
        tags: Vec<NoteTag>,
    },
    /// Recognized handwriting text extracted from OneNote ink OCR.
    InkText {
        text: String,
    },
    /// Placeholder for content that cannot be faithfully represented in GFM.
    Unsupported {
        kind: String,
        detail: String,
    },
}

/// An item in an ordered, unordered, or task list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// `Some(false)` for `- [ ]`, `Some(true)` for `- [x]`, `None` for regular list items.
    pub checkbox: Option<bool>,
    pub tags: Vec<NoteTag>,
    pub content: Vec<Block>,
    pub children: Vec<Block>,
}

/// An inline run within a paragraph, heading, or list item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Bold(Vec<Inline>),
    Italic(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Link {
        text: Vec<Inline>,
        target: LinkTarget,
    },
    LineBreak,
}

/// Destination of a hyperlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    External(String),
    OneNote(OneNoteLink),
}

impl LinkTarget {
    /// Parse a raw URL string into either an internal [`OneNoteLink`] or an external URL.
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if let Some(link) = OneNoteLink::parse(trimmed) {
            Self::OneNote(link)
        } else {
            Self::External(trimmed.to_string())
        }
    }
}

/// Parsed fields from an internal OneNote hyperlink (`onenote:...` or OneNote web/client URL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneNoteLink {
    pub raw: String,
    pub page_id: Option<String>,
    pub section_id: Option<String>,
    pub page_title: Option<String>,
    pub section_name: Option<String>,
}

impl OneNoteLink {
    /// Parse `onenote:` protocol URIs and known OneNote internal link patterns.
    ///
    /// Example desktop/local URI:
    /// `onenote:#Target%20Page&section-id={A1B2C3D4-...}&page-id={11223344-...}&end`
    /// or cross-section URI:
    /// `onenote:../OtherSection.one#Target%20Page&section-id={...}&page-id={...}&end`
    pub fn parse(raw: &str) -> Option<Self> {
        let lower = raw.to_ascii_lowercase();
        let is_onenote_scheme = lower.starts_with("onenote:");
        let is_web_onenote = (lower.contains("onedrive.live.com")
            || lower.contains("sharepoint.com")
            || lower.contains("onenote.com"))
            && (lower.contains("page-id=") || lower.contains("section-id=") || lower.contains("wd=target"));

        if !is_onenote_scheme && !is_web_onenote {
            return None;
        }

        let mut page_id = None;
        let mut section_id = None;
        let mut page_title = None;
        let mut section_name = None;

        // Extract optional section filename before `#` for `onenote:` URIs
        if is_onenote_scheme {
            let without_scheme = &raw["onenote:".len()..];
            let (before_hash, after_hash) = match without_scheme.split_once('#') {
                Some((before, after)) => (before, Some(after)),
                None => (without_scheme, None),
            };

            let decoded_before = percent_decode_str(before_hash)
                .decode_utf8_lossy()
                .trim()
                .to_string();
            if !decoded_before.is_empty() {
                // Take final path component and strip `.one`
                let leaf = decoded_before
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&decoded_before);
                let cleaned = leaf
                    .strip_suffix(".one")
                    .or_else(|| leaf.strip_suffix(".ONE"))
                    .unwrap_or(leaf)
                    .trim();
                if !cleaned.is_empty() && !cleaned.contains('=') {
                    section_name = Some(cleaned.to_string());
                }
            }

            if let Some(fragment) = after_hash {
                parse_onenote_fragment(
                    fragment,
                    true,
                    &mut page_title,
                    &mut section_id,
                    &mut page_id,
                );
            } else if without_scheme.contains('=') {
                parse_onenote_fragment(
                    without_scheme,
                    false,
                    &mut page_title,
                    &mut section_id,
                    &mut page_id,
                );
            }
        } else {
            // OneNote web URL with query/fragment parameters (including `wd=target(...)`)
            let search_part = raw.split_once('?').map(|(_, q)| q).unwrap_or(raw);
            for part in search_part.split(['&', '#']) {
                if let Some((k, v)) = part.split_once('=') {
                    let key = k.trim().to_ascii_lowercase();
                    let decoded_v = percent_decode_str(v.trim()).decode_utf8_lossy().to_string();
                    match key.as_str() {
                        "page-id" => page_id = normalize_guid(&decoded_v),
                        "section-id" => section_id = normalize_guid(&decoded_v),
                        "wd" => parse_wd_target(
                            &decoded_v,
                            &mut section_name,
                            &mut section_id,
                            &mut page_title,
                            &mut page_id,
                        ),
                        _ => {}
                    }
                }
            }
        }

        Some(Self {
            raw: raw.to_string(),
            page_id,
            section_id,
            page_title,
            section_name,
        })
    }
}

fn parse_onenote_fragment(
    fragment: &str,
    allow_leading_title: bool,
    page_title: &mut Option<String>,
    section_id: &mut Option<String>,
    page_id: &mut Option<String>,
) {
    for (idx, part) in fragment.split('&').enumerate() {
        if let Some((k, v)) = part.split_once('=') {
            let key = k.trim().to_ascii_lowercase();
            let decoded_v = percent_decode_str(v.trim()).decode_utf8_lossy().to_string();
            match key.as_str() {
                "page-id" => *page_id = normalize_guid(&decoded_v),
                "section-id" => *section_id = normalize_guid(&decoded_v),
                _ => {}
            }
        } else if allow_leading_title && idx == 0 {
            let decoded = percent_decode_str(part.trim())
                .decode_utf8_lossy()
                .trim()
                .to_string();
            if !decoded.is_empty() && !decoded.eq_ignore_ascii_case("end") {
                *page_title = Some(decoded);
            }
        }
    }
}

/// Parse OneNote web `wd=target(Section.one|sec-guid/Page Title|page-guid/)` parameters.
fn parse_wd_target(
    value: &str,
    section_name: &mut Option<String>,
    section_id: &mut Option<String>,
    page_title: &mut Option<String>,
    page_id: &mut Option<String>,
) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let Some(start) = lower.find("target(") else {
        return;
    };
    let inner = &trimmed[start + "target(".len()..];
    let inner = inner.rsplit_once(')').map(|(before, _)| before).unwrap_or(inner);

    for segment in inner.split('/').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((raw_name, raw_guid)) = segment.split_once('|') {
            let name = raw_name.trim();
            let guid = normalize_guid(raw_guid);
            if let Some(sec) = name
                .strip_suffix(".one")
                .or_else(|| name.strip_suffix(".ONE"))
            {
                if !sec.trim().is_empty() {
                    *section_name = Some(sec.trim().to_string());
                }
                if guid.is_some() {
                    *section_id = guid;
                }
            } else {
                if !name.is_empty() {
                    *page_title = Some(name.to_string());
                }
                if guid.is_some() {
                    *page_id = guid;
                }
            }
        }
    }
}

/// Normalize a GUID string (with or without braces, upper/lower case) to lowercase hyphenated form.
///
/// OneNote GUIDs occasionally carry a trailing sequence suffix like `{GUID}{1}{B0}`;
/// this extracts the primary 36-char UUID cleanly.
pub fn normalize_guid(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // If wrapped in `{...}`, take the first braced segment.
    let candidate = if let Some(rest) = trimmed.strip_prefix('{') {
        rest.split('}').next().unwrap_or(rest)
    } else {
        trimmed
    };

    let cleaned = candidate.trim().to_ascii_lowercase();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_desktop_onenote_link() {
        let raw = "onenote:#Project%20Plan&section-id={AF6DE84B-D817-4F07-8BAF-FB6629CB7C2C}&page-id={11223344-5566-7788-99AA-BBCCDDEEFF00}&end";
        let link = OneNoteLink::parse(raw).expect("should parse onenote link");
        assert_eq!(link.page_title.as_deref(), Some("Project Plan"));
        assert_eq!(
            link.section_id.as_deref(),
            Some("af6de84b-d817-4f07-8baf-fb6629cb7c2c")
        );
        assert_eq!(
            link.page_id.as_deref(),
            Some("11223344-5566-7788-99aa-bbccddeeff00")
        );
        assert_eq!(link.section_name, None);
    }

    #[test]
    fn parses_cross_section_onenote_link() {
        let raw = "onenote:Design%20Docs.one#API%20Spec&section-id={A1B2C3D4-0000-0000-0000-000000000001}&page-id={A1B2C3D4-0000-0000-0000-000000000002}&end";
        let link = OneNoteLink::parse(raw).expect("should parse cross-section link");
        assert_eq!(link.section_name.as_deref(), Some("Design Docs"));
        assert_eq!(link.page_title.as_deref(), Some("API Spec"));
        assert_eq!(
            link.page_id.as_deref(),
            Some("a1b2c3d4-0000-0000-0000-000000000002")
        );
    }

    #[test]
    fn keeps_regular_urls_external() {
        assert!(matches!(
            LinkTarget::parse("https://example.com/docs"),
            LinkTarget::External(url) if url == "https://example.com/docs"
        ));
    }

    #[test]
    fn normalizes_guid_with_suffix() {
        assert_eq!(
            normalize_guid("{AF6DE84B-D817-4F07-8BAF-FB6629CB7C2C}{1}{B0}").as_deref(),
            Some("af6de84b-d817-4f07-8baf-fb6629cb7c2c")
        );
    }

    #[test]
    fn parses_web_wd_target_link() {
        let raw = "https://contoso.sharepoint.com/sites/eng/_layouts/OneNote.aspx?id=%2Fnb&wd=target%28Design%20Docs.one%7C11111111-1111-1111-1111-111111111111%2FAPI%20Spec%7C22222222-2222-2222-2222-222222222222%2F%29";
        let link = OneNoteLink::parse(raw).expect("should parse wd=target link");
        assert_eq!(link.section_name.as_deref(), Some("Design Docs"));
        assert_eq!(
            link.section_id.as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(link.page_title.as_deref(), Some("API Spec"));
        assert_eq!(
            link.page_id.as_deref(),
            Some("22222222-2222-2222-2222-222222222222")
        );
    }

    #[test]
    fn parses_section_only_onenote_link_without_fake_page_title() {
        let raw = "onenote:../Ops.one";
        let link = OneNoteLink::parse(raw).expect("should parse section-only link");
        assert_eq!(link.section_name.as_deref(), Some("Ops"));
        assert_eq!(link.page_title, None);
        assert_eq!(link.page_id, None);
    }
}
