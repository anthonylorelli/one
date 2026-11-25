use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use tracing::{debug, info};

use crate::config::ExportOptions;
use crate::ir::link_registry::build_plan;
use crate::ir::{SectionEntry, SourceBundle};
use crate::render::markdown::render_page;
use crate::render::report::ExportReport;
use crate::render::sanitize::sanitize_filename;
use crate::sources::OneNoteSource;

/// Load from any [`OneNoteSource`] and run the two-pass export pipeline.
pub fn run_export(source: &dyn OneNoteSource, options: &ExportOptions) -> Result<ExportReport> {
    info!(source = source.source_name(), "Loading OneNote data");
    let bundle = source.load()?;
    export_bundle(source.source_name(), &bundle, options)
}

/// Execute Pass 1 (plan + link index) and Pass 2 (asset extraction + Markdown emission)
/// on an already-loaded [`SourceBundle`].
pub fn export_bundle(
    source_name: &str,
    bundle: &SourceBundle,
    options: &ExportOptions,
) -> Result<ExportReport> {
    let plan = build_plan(bundle, options);
    info!(
        notebooks = plan.notebook_count,
        sections = plan.section_count,
        pages = plan.pages.len(),
        "Planned export hierarchy"
    );

    let mut report = ExportReport {
        source: source_name.to_string(),
        output_dir: options.output_dir.clone(),
        dry_run: options.dry_run,
        notebooks_exported: plan.notebook_count,
        sections_exported: plan.section_count,
        pages_exported: 0,
        assets_written: 0,
        links_resolved: 0,
        unresolved_links: Vec::new(),
        warnings: bundle.warnings.clone(),
    };

    // Content-hash dedup per section asset directory: (section_dir, sha256) -> relative asset path
    let mut deduped_assets: HashMap<(PathBuf, [u8; 32]), PathBuf> = HashMap::new();
    // Tracks every actually allocated lowercase filename per asset directory
    let mut used_asset_names: HashMap<PathBuf, HashSet<String>> = HashMap::new();

    for planned in &plan.pages {
        let mut page_asset_paths: HashMap<String, PathBuf> = HashMap::new();

        for asset in &planned.page.assets {
            let hash: [u8; 32] = Sha256::digest(&asset.data).into();
            let dedup_key = (planned.section_dir.clone(), hash);

            let rel_asset_path = if let Some(existing) = deduped_assets.get(&dedup_key) {
                existing.clone()
            } else {
                let assets_dir = planned.section_dir.join("_assets");
                let clean_name = sanitize_filename(&asset.filename, "asset", None);
                let unique_name =
                    allocate_asset_filename(&assets_dir, &clean_name, &hash, &mut used_asset_names);
                let rel_path = assets_dir.join(unique_name);

                if !options.dry_run {
                    let full_asset_path = options.output_dir.join(&rel_path);
                    if let Some(parent) = full_asset_path.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!("Failed to create assets directory: {}", parent.display())
                        })?;
                    }
                    fs::write(&full_asset_path, &asset.data).with_context(|| {
                        format!("Failed to write asset: {}", full_asset_path.display())
                    })?;
                }

                report.assets_written += 1;
                deduped_assets.insert(dedup_key, rel_path.clone());
                rel_path
            };

            page_asset_paths.insert(asset.id.clone(), rel_asset_path);
        }

        let rendered = render_page(planned, &page_asset_paths, &plan.registry, options);
        report.links_resolved += rendered.links_resolved;
        report.unresolved_links.extend(rendered.unresolved_links);

        if !options.dry_run {
            let full_md_path = options.output_dir.join(&planned.relative_md_path);
            debug!(path = %full_md_path.display(), "Writing Markdown page");
            if let Some(parent) = full_md_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create output directory: {}", parent.display())
                })?;
            }
            fs::write(&full_md_path, rendered.markdown.as_bytes()).with_context(|| {
                format!("Failed to write Markdown file: {}", full_md_path.display())
            })?;

            if options.preserve_mtime
                && let Some(ts) = planned.page.updated_at.or(planned.page.created_at)
                && let Some(sys_time) = datetime_to_system_time(ts)
                && let Ok(file) = fs::OpenOptions::new().write(true).open(&full_md_path)
            {
                let _ = file.set_modified(sys_time);
            }
        }

        report.pages_exported += 1;
    }

    if let Some(report_path) = &options.report_path {
        if let Some(parent) = report_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create report parent directory: {}",
                    parent.display()
                )
            })?;
        }
        let json = serde_json::to_string_pretty(&report)?;
        fs::write(report_path, json)
            .with_context(|| format!("Failed to write export report: {}", report_path.display()))?;
    }

    Ok(report)
}

/// Print a human-readable tree of notebooks, section groups, sections, and pages.
pub fn print_bundle_tree(bundle: &SourceBundle, show_pages: bool) {
    for notebook in &bundle.notebooks {
        println!("Notebook: {}", notebook.name);
        print_entries(&notebook.entries, 1, show_pages);
    }
    if !bundle.warnings.is_empty() {
        eprintln!("\nWarnings ({}):", bundle.warnings.len());
        for w in &bundle.warnings {
            eprintln!("  [{}] {}", w.scope, w.message);
        }
    }
}

fn print_entries(entries: &[SectionEntry], depth: usize, show_pages: bool) {
    let indent = "  ".repeat(depth);
    for entry in entries {
        match entry {
            SectionEntry::SectionGroup(group) => {
                println!("{indent}[Group] {}", group.name);
                print_entries(&group.entries, depth + 1, show_pages);
            }
            SectionEntry::Section(section) => {
                println!("{indent}- {} ({} pages)", section.name, section.pages.len());
                if show_pages {
                    for page in &section.pages {
                        let page_indent = "  ".repeat(depth + 1 + page.level as usize);
                        println!("{page_indent}* {}", page.title);
                    }
                }
            }
        }
    }
}

fn allocate_asset_filename(
    assets_dir: &Path,
    clean_name: &str,
    hash: &[u8; 32],
    used: &mut HashMap<PathBuf, HashSet<String>>,
) -> String {
    let dir_set = used.entry(assets_dir.to_path_buf()).or_default();
    if dir_set.insert(clean_name.to_ascii_lowercase()) {
        return clean_name.to_string();
    }

    // Disambiguate colliding filenames with an 8-hex-char content hash suffix, falling back
    // to a numeric counter if the disambiguated name itself is already allocated.
    let short_hash: String = hash[..4].iter().map(|b| format!("{b:02x}")).collect();
    let candidate = append_suffix(clean_name, &short_hash);
    if dir_set.insert(candidate.to_ascii_lowercase()) {
        return candidate;
    }

    let mut counter = 2usize;
    loop {
        let fallback = append_suffix(clean_name, &format!("{short_hash}-{counter}"));
        if dir_set.insert(fallback.to_ascii_lowercase()) {
            return fallback;
        }
        counter += 1;
    }
}

fn append_suffix(clean_name: &str, suffix: &str) -> String {
    match clean_name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}-{suffix}.{ext}"),
        None => format!("{clean_name}-{suffix}"),
    }
}

fn datetime_to_system_time(ts: chrono::DateTime<chrono::Utc>) -> Option<SystemTime> {
    let secs = u64::try_from(ts.timestamp()).ok()?;
    UNIX_EPOCH.checked_add(Duration::new(secs, ts.timestamp_subsec_nanos()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_asset_filename_never_collides_with_disambiguated_names() {
        let dir = Path::new("Section/_assets");
        let mut used = HashMap::new();

        let hash_a = [0x11u8; 32];
        let mut hash_b = [0x22u8; 32];
        hash_b[0..4].copy_from_slice(&[0xab, 0xcd, 0x12, 0x34]);

        // Pre-allocate literal name matching hash_b's disambiguated form
        let first = allocate_asset_filename(dir, "diagram-abcd1234.png", &hash_a, &mut used);
        assert_eq!(first, "diagram-abcd1234.png");

        let second = allocate_asset_filename(dir, "diagram.png", &hash_a, &mut used);
        assert_eq!(second, "diagram.png");

        // Second `diagram.png` has short_hash `abcd1234`, which collides with `first` -> gets `-2`
        let third = allocate_asset_filename(dir, "diagram.png", &hash_b, &mut used);
        assert_eq!(third, "diagram-abcd1234-2.png");
    }
}
