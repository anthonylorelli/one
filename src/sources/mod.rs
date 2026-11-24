pub mod graph;
pub mod local;

use anyhow::Result;

use crate::ir::SourceBundle;

/// Pluggable acquisition backend that loads OneNote notebooks into the canonical IR.
///
/// Both local binary file parsers (`.one` / `.onetoc2` / `.onepkg`) and cloud backends
/// (Microsoft Graph API) implement this trait so the two-pass path planner, cross-page
/// link resolver, and Markdown renderer remain completely source-agnostic.
pub trait OneNoteSource {
    /// Human-readable identifier for this source (used in logs and export reports).
    fn source_name(&self) -> &'static str;

    /// Load notebooks, sections, pages, and any non-fatal warnings into a [`SourceBundle`].
    fn load(&self) -> Result<SourceBundle>;
}
