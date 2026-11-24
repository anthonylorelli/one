use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use one::cli::{Cli, Command, ExportSourceCommand, ListSourceCommand};
use one::pipeline::{print_bundle_tree, run_export};
use one::sources::OneNoteSource;
use one::sources::graph::GraphSource;
use one::sources::local::LocalFileSource;

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Command::Export { source } => match source {
            ExportSourceCommand::Local { input, opts } => {
                let src = LocalFileSource::new(input);
                let report = run_export(&src, &opts.into_export_options())?;
                report.print_summary();
            }
            ExportSourceCommand::Graph { opts } => {
                let src = GraphSource;
                let report = run_export(&src, &opts.into_export_options())?;
                report.print_summary();
            }
        },
        Command::List { source } => match source {
            ListSourceCommand::Local { input, pages } => {
                let src = LocalFileSource::new(input);
                let bundle = src.load()?;
                print_bundle_tree(&bundle, pages);
            }
            ListSourceCommand::Graph => {
                let src = GraphSource;
                let bundle = src.load()?;
                print_bundle_tree(&bundle, false);
            }
        },
    }

    Ok(())
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
