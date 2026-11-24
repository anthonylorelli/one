use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ir::ExportWarning;

/// Record of an internal OneNote link that could not be resolved to an exported page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedLink {
    pub source_page: PathBuf,
    pub link_text: String,
    pub target_raw: String,
}

/// Summary report produced after an export run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportReport {
    pub source: String,
    pub output_dir: PathBuf,
    pub dry_run: bool,
    pub notebooks_exported: usize,
    pub sections_exported: usize,
    pub pages_exported: usize,
    pub assets_written: usize,
    pub links_resolved: usize,
    pub unresolved_links: Vec<UnresolvedLink>,
    pub warnings: Vec<ExportWarning>,
}

impl ExportReport {
    pub fn print_summary(&self) {
        let mode = if self.dry_run { " (dry-run)" } else { "" };
        println!("Export complete{mode}:");
        println!("  Source:     {}", self.source);
        println!("  Output:     {}", self.output_dir.display());
        println!("  Notebooks:  {}", self.notebooks_exported);
        println!("  Sections:   {}", self.sections_exported);
        println!("  Pages:      {}", self.pages_exported);
        println!("  Assets:     {}", self.assets_written);
        println!("  Links:      {} resolved, {} unresolved", self.links_resolved, self.unresolved_links.len());
        if !self.warnings.is_empty() {
            println!("  Warnings:   {}", self.warnings.len());
        }
    }
}
