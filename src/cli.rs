use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::{ExportOptions, LinkStyle, SubpageStrategy, TagStyle};

/// Export Microsoft OneNote notebooks to GitHub Flavored Markdown.
#[derive(Debug, Parser)]
#[command(name = "one", version, about)]
pub struct Cli {
    /// Increase logging verbosity (-v info, -vv debug).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Export OneNote notebooks to Markdown.
    Export {
        #[command(subcommand)]
        source: ExportSourceCommand,
    },
    /// Inspect notebooks, section groups, sections, and pages without writing files.
    List {
        #[command(subcommand)]
        source: ListSourceCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ExportSourceCommand {
    /// Export from a local .one file, .onetoc2 file, .onepkg archive, or notebook directory.
    Local {
        /// Path to a .one, .onetoc2, .onepkg file, or notebook directory.
        input: PathBuf,

        #[command(flatten)]
        opts: CommonExportArgs,
    },
    /// Export from Microsoft Graph cloud notebooks (planned).
    Graph {
        #[command(flatten)]
        opts: CommonExportArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum ListSourceCommand {
    /// List sections and pages in a local .one, .onetoc2, .onepkg file, or notebook directory.
    Local {
        /// Path to a .one, .onetoc2, .onepkg file, or notebook directory.
        input: PathBuf,

        /// Include individual page titles in the tree output.
        #[arg(long)]
        pages: bool,
    },
    /// List notebooks via Microsoft Graph API (planned).
    Graph,
}

#[derive(Debug, Args)]
pub struct CommonExportArgs {
    /// Destination directory for exported Markdown files and assets.
    #[arg(short, long, default_value = "./exported")]
    pub output: PathBuf,

    /// Subpage layout strategy.
    #[arg(long, value_enum, default_value_t = SubpageStrategy::Nested)]
    pub subpages: SubpageStrategy,

    /// Internal cross-page link format.
    #[arg(long, value_enum, default_value_t = LinkStyle::Relative)]
    pub links: LinkStyle,

    /// Rendering style for non-checkbox OneNote tags.
    #[arg(long, value_enum, default_value_t = TagStyle::Inline)]
    pub tag_style: TagStyle,

    /// Apply each page's last-modified timestamp to its exported .md file on disk.
    #[arg(long)]
    pub preserve_mtime: bool,

    /// Omit YAML frontmatter from exported Markdown files.
    #[arg(long)]
    pub no_frontmatter: bool,

    /// Plan and render without writing Markdown or asset files to disk.
    #[arg(long)]
    pub dry_run: bool,

    /// Write a JSON migration report (counts, warnings, unresolved links) to this path.
    #[arg(long)]
    pub report: Option<PathBuf>,

    /// Only export notebooks whose name contains this substring (case-insensitive).
    #[arg(short, long)]
    pub notebook: Option<String>,

    /// Only export sections whose name contains this substring (case-insensitive).
    #[arg(short, long)]
    pub section: Option<String>,
}

impl CommonExportArgs {
    pub fn into_export_options(self) -> ExportOptions {
        ExportOptions {
            output_dir: self.output,
            subpages: self.subpages,
            links: self.links,
            tag_style: self.tag_style,
            preserve_mtime: self.preserve_mtime,
            dry_run: self.dry_run,
            include_frontmatter: !self.no_frontmatter,
            report_path: self.report,
            notebook_filter: self.notebook,
            section_filter: self.section,
        }
    }
}
