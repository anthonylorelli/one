use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// How OneNote subpages (`level > 0`) are mapped to the output filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubpageStrategy {
    /// Pages with children become directories (`Parent/Parent.md`, `Parent/Child.md`).
    #[default]
    Nested,
    /// All pages live in the section directory with hierarchical numeric prefixes (`01 Parent.md`, `01.1 Child.md`).
    Flat,
}

/// How resolved internal OneNote cross-page links are rendered in Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkStyle {
    /// Standard portable GFM relative Markdown links (`[Page](../Section/Page.md)`).
    #[default]
    Relative,
    /// Obsidian-style wikilinks (`[[Page]]` or `[[Page|Alias]]`).
    Wikilink,
}

/// How non-checkbox OneNote tags (`Important`, `Question`, custom tags, etc.) are emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TagStyle {
    /// Emit inline Markdown tags (e.g. `#important`) and aggregate into YAML frontmatter `tags`.
    #[default]
    Inline,
    /// Prefix tagged blocks with recognizable emoji markers (e.g. `⭐`, `❓`) and aggregate into frontmatter `tags`.
    Emoji,
    /// Only record tags in YAML frontmatter `tags` without adding inline markers to body text.
    Frontmatter,
}

/// Configuration controlling how a OneNote bundle is exported to Markdown.
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub output_dir: PathBuf,
    pub subpages: SubpageStrategy,
    pub links: LinkStyle,
    pub tag_style: TagStyle,
    pub preserve_mtime: bool,
    pub dry_run: bool,
    pub include_frontmatter: bool,
    pub report_path: Option<PathBuf>,
    pub notebook_filter: Option<String>,
    pub section_filter: Option<String>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("./exported"),
            subpages: SubpageStrategy::Nested,
            links: LinkStyle::Relative,
            tag_style: TagStyle::Inline,
            preserve_mtime: false,
            dry_run: false,
            include_frontmatter: true,
            report_path: None,
            notebook_filter: None,
            section_filter: None,
        }
    }
}
