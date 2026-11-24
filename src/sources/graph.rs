use anyhow::{Result, bail};

use crate::ir::SourceBundle;
use crate::sources::OneNoteSource;

/// Placeholder for the Microsoft Graph API cloud source (planned for Phase 3).
#[derive(Debug, Default)]
pub struct GraphSource;

impl OneNoteSource for GraphSource {
    fn source_name(&self) -> &'static str {
        "microsoft-graph"
    }

    fn load(&self) -> Result<SourceBundle> {
        bail!(
            "Microsoft Graph API export is not implemented yet. \
             Use `one export local <path>` to export .one, .onetoc2, or .onepkg files."
        )
    }
}
