use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::{ExportOptions, SubpageStrategy};
use crate::ir::{Notebook, OneNoteLink, Page, Section, SectionEntry, SourceBundle, normalize_guid};
use crate::render::sanitize::{deduplicate_name, sanitize_segment};

/// A page scheduled for export during Pass 1, paired with its planned output paths.
#[derive(Debug)]
pub struct PlannedPage<'a> {
    pub notebook_name: String,
    pub section_group_path: Vec<String>,
    pub section_name: String,
    pub section_id: Option<String>,
    /// Section directory path relative to `options.output_dir`.
    pub section_dir: PathBuf,
    /// Markdown file path relative to `options.output_dir`.
    pub relative_md_path: PathBuf,
    pub page: &'a Page,
}

/// Output of Pass 1: planned pages plus a lookup table for cross-page links.
#[derive(Debug)]
pub struct ExportPlan<'a> {
    pub pages: Vec<PlannedPage<'a>>,
    pub registry: LinkRegistry,
    pub notebook_count: usize,
    pub section_count: usize,
}

/// Index built in Pass 1 to resolve internal OneNote links to exported Markdown paths.
#[derive(Debug, Default)]
pub struct LinkRegistry {
    by_page_id: HashMap<String, PathBuf>,
    by_section_id: HashMap<String, PathBuf>,
    by_url: HashMap<String, PathBuf>,
    by_section_and_title: HashMap<(String, String), PathBuf>,
    /// Maps normalized title -> `Some(path)` if unique across export, or `None` if ambiguous.
    by_unique_title: HashMap<String, Option<PathBuf>>,
}

impl LinkRegistry {
    fn register_page(
        &mut self,
        section_name: &str,
        section_id: Option<&str>,
        page: &Page,
        rel_md_path: &Path,
    ) {
        let path_buf = rel_md_path.to_path_buf();

        if let Some(id) = page.id.as_deref().and_then(normalize_guid) {
            self.by_page_id.entry(id).or_insert_with(|| path_buf.clone());
        }

        // First page of a section serves as the target for section-only links.
        if let Some(sec_id) = section_id.and_then(normalize_guid) {
            self.by_section_id
                .entry(sec_id)
                .or_insert_with(|| path_buf.clone());
        }

        if let Some(url) = &page.web_url {
            self.by_url
                .entry(url.trim().to_ascii_lowercase())
                .or_insert_with(|| path_buf.clone());
        }
        if let Some(url) = &page.client_url {
            self.by_url
                .entry(url.trim().to_ascii_lowercase())
                .or_insert_with(|| path_buf.clone());
        }

        let norm_sec = normalize_key(section_name);
        let norm_title = normalize_key(&page.title);
        if !norm_title.is_empty() {
            self.by_section_and_title
                .entry((norm_sec, norm_title.clone()))
                .or_insert_with(|| path_buf.clone());

            self.by_unique_title
                .entry(norm_title)
                .and_modify(|existing| *existing = None)
                .or_insert_with(|| Some(path_buf));
        }
    }

    /// Resolve a parsed internal [`OneNoteLink`] to a target Markdown path relative to output root.
    pub fn resolve(&self, link: &OneNoteLink, current_section: &str) -> Option<&Path> {
        // 1. Exact page GUID match
        if let Some(page_id) = link.page_id.as_deref().and_then(normalize_guid)
            && let Some(path) = self.by_page_id.get(&page_id)
        {
            return Some(path.as_path());
        }

        // 2. Exact raw URL match (e.g., Graph oneNoteWebUrl / oneNoteClientUrl)
        if let Some(path) = self.by_url.get(&link.raw.trim().to_ascii_lowercase()) {
            return Some(path.as_path());
        }

        // 3. Explicit `(section_name, page_title)` match
        if let Some(title) = link.page_title.as_deref() {
            let norm_title = normalize_key(title);
            if let Some(sec) = link.section_name.as_deref()
                && let Some(path) = self
                    .by_section_and_title
                    .get(&(normalize_key(sec), norm_title.clone()))
            {
                return Some(path.as_path());
            }

            // 4. Same-section `(current_section, page_title)` match
            if let Some(path) = self
                .by_section_and_title
                .get(&(normalize_key(current_section), norm_title.clone()))
            {
                return Some(path.as_path());
            }

            // 5. Globally unique page title match
            if let Some(Some(path)) = self.by_unique_title.get(&norm_title) {
                return Some(path.as_path());
            }
        }

        // 6. Section-only link (no page-id specified)
        if link.page_id.is_none()
            && let Some(sec_id) = link.section_id.as_deref().and_then(normalize_guid)
            && let Some(path) = self.by_section_id.get(&sec_id)
        {
            return Some(path.as_path());
        }

        None
    }
}

/// Run Pass 1: discover all pages, apply filters, compute deterministic output paths,
/// and populate the [`LinkRegistry`].
pub fn build_plan<'a>(bundle: &'a SourceBundle, options: &ExportOptions) -> ExportPlan<'a> {
    let mut plan = ExportPlan {
        pages: Vec::new(),
        registry: LinkRegistry::default(),
        notebook_count: 0,
        section_count: 0,
    };

    let mut used_notebooks = HashSet::new();

    for notebook in &bundle.notebooks {
        if let Some(filter) = &options.notebook_filter
            && !matches_filter(&notebook.name, filter)
        {
            continue;
        }

        let nb_dir_name = deduplicate_name(
            &sanitize_segment(&notebook.name, "Untitled Notebook"),
            &mut used_notebooks,
        );
        let nb_dir = PathBuf::from(&nb_dir_name);
        let before_sections = plan.section_count;

        plan_entries(
            notebook,
            &notebook.entries,
            &nb_dir,
            &[],
            options,
            &mut plan,
        );

        if plan.section_count > before_sections || options.section_filter.is_none() {
            plan.notebook_count += 1;
        }
    }

    plan
}

fn plan_entries<'a>(
    notebook: &'a Notebook,
    entries: &'a [SectionEntry],
    parent_dir: &Path,
    group_path: &[String],
    options: &ExportOptions,
    plan: &mut ExportPlan<'a>,
) {
    let mut used_siblings = HashSet::new();

    for entry in entries {
        match entry {
            SectionEntry::SectionGroup(group) => {
                let dir_name = deduplicate_name(
                    &sanitize_segment(&group.name, "Untitled Group"),
                    &mut used_siblings,
                );
                let mut next_groups = group_path.to_vec();
                next_groups.push(group.name.clone());
                plan_entries(
                    notebook,
                    &group.entries,
                    &parent_dir.join(dir_name),
                    &next_groups,
                    options,
                    plan,
                );
            }
            SectionEntry::Section(section) => {
                if let Some(filter) = &options.section_filter
                    && !matches_filter(&section.name, filter)
                {
                    continue;
                }

                let sec_dir_name = deduplicate_name(
                    &sanitize_segment(&section.name, "Untitled Section"),
                    &mut used_siblings,
                );
                let sec_dir = parent_dir.join(sec_dir_name);
                plan.section_count += 1;

                plan_section_pages(notebook, group_path, section, &sec_dir, options, plan);
            }
        }
    }
}

fn plan_section_pages<'a>(
    notebook: &'a Notebook,
    group_path: &[String],
    section: &'a Section,
    section_dir: &Path,
    options: &ExportOptions,
    plan: &mut ExportPlan<'a>,
) {
    match options.subpages {
        SubpageStrategy::Nested => {
            plan_pages_nested(notebook, group_path, section, section_dir, plan)
        }
        SubpageStrategy::Flat => plan_pages_flat(notebook, group_path, section, section_dir, plan),
    }
}

fn plan_pages_nested<'a>(
    notebook: &'a Notebook,
    group_path: &[String],
    section: &'a Section,
    section_dir: &Path,
    plan: &mut ExportPlan<'a>,
) {
    struct StackFrame {
        level: u8,
        dir: PathBuf,
        used_children: HashSet<String>,
    }

    let mut root_used = HashSet::new();
    // Reserve `_assets` so a parent page titled `_assets` cannot collide with `<section>/_assets/`.
    root_used.insert("_assets".to_string());
    let mut stack: Vec<StackFrame> = Vec::new();

    for (idx, page) in section.pages.iter().enumerate() {
        while stack.last().is_some_and(|frame| frame.level >= page.level) {
            stack.pop();
        }

        let has_children = section
            .pages
            .get(idx + 1)
            .is_some_and(|next| next.level > page.level);

        let (parent_dir, used_set) = match stack.last_mut() {
            Some(frame) => (frame.dir.clone(), &mut frame.used_children),
            None => (section_dir.to_path_buf(), &mut root_used),
        };

        let stem = deduplicate_name(&sanitize_segment(&page.title, "Untitled Page"), used_set);

        let rel_md_path = if has_children {
            let page_dir = parent_dir.join(&stem);
            let md_path = page_dir.join(format!("{stem}.md"));
            let mut child_used = HashSet::new();
            child_used.insert(stem.to_ascii_lowercase());
            stack.push(StackFrame {
                level: page.level,
                dir: page_dir,
                used_children: child_used,
            });
            md_path
        } else {
            parent_dir.join(format!("{stem}.md"))
        };

        plan.registry
            .register_page(&section.name, section.id.as_deref(), page, &rel_md_path);

        plan.pages.push(PlannedPage {
            notebook_name: notebook.name.clone(),
            section_group_path: group_path.to_vec(),
            section_name: section.name.clone(),
            section_id: section.id.clone(),
            section_dir: section_dir.to_path_buf(),
            relative_md_path: rel_md_path,
            page,
        });
    }
}

fn plan_pages_flat<'a>(
    notebook: &'a Notebook,
    group_path: &[String],
    section: &'a Section,
    section_dir: &Path,
    plan: &mut ExportPlan<'a>,
) {
    let mut counters: Vec<usize> = Vec::new();
    let mut used = HashSet::new();

    for page in &section.pages {
        let depth = page.level as usize;
        if counters.len() <= depth {
            while counters.len() <= depth {
                counters.push(0);
            }
        } else {
            counters.truncate(depth + 1);
        }

        // Ensure parent counters are at least 1 if a section starts at level > 0
        for parent in counters.iter_mut().take(depth) {
            if *parent == 0 {
                *parent = 1;
            }
        }
        counters[depth] += 1;

        let prefix = counters
            .iter()
            .enumerate()
            .map(|(i, n)| {
                if i == 0 {
                    format!("{n:02}")
                } else {
                    n.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(".");

        let raw_stem = format!("{prefix} {}", sanitize_segment(&page.title, "Untitled Page"));
        let stem = deduplicate_name(&raw_stem, &mut used);
        let rel_md_path = section_dir.join(format!("{stem}.md"));

        plan.registry
            .register_page(&section.name, section.id.as_deref(), page, &rel_md_path);

        plan.pages.push(PlannedPage {
            notebook_name: notebook.name.clone(),
            section_group_path: group_path.to_vec(),
            section_name: section.name.clone(),
            section_id: section.id.clone(),
            section_dir: section_dir.to_path_buf(),
            relative_md_path: rel_md_path,
            page,
        });
    }
}

fn normalize_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn matches_filter(name: &str, filter: &str) -> bool {
    name.to_ascii_lowercase()
        .contains(&filter.trim().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: &str, title: &str, level: u8) -> Page {
        Page {
            id: Some(id.to_string()),
            title: title.to_string(),
            level,
            created_at: None,
            updated_at: None,
            author: None,
            web_url: None,
            client_url: None,
            blocks: Vec::new(),
            assets: Vec::new(),
        }
    }

    fn sample_bundle() -> SourceBundle {
        SourceBundle {
            notebooks: vec![Notebook {
                id: None,
                name: "Work".to_string(),
                entries: vec![SectionEntry::Section(Section {
                    id: Some("sec-1".to_string()),
                    name: "Design".to_string(),
                    pages: vec![
                        page("11111111-1111-1111-1111-111111111111", "Architecture", 0),
                        page("22222222-2222-2222-2222-222222222222", "Storage", 1),
                        page("33333333-3333-3333-3333-333333333333", "Schema", 2),
                        page("44444444-4444-4444-4444-444444444444", "API", 1),
                        page("55555555-5555-5555-5555-555555555555", "Standalone", 0),
                    ],
                })],
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn nested_subpages_create_folder_notes() {
        let bundle = sample_bundle();
        let plan = build_plan(&bundle, &ExportOptions::default());
        let paths: Vec<String> = plan
            .pages
            .iter()
            .map(|p| p.relative_md_path.to_string_lossy().replace('\\', "/"))
            .collect();

        assert_eq!(
            paths,
            vec![
                "Work/Design/Architecture/Architecture.md",
                "Work/Design/Architecture/Storage/Storage.md",
                "Work/Design/Architecture/Storage/Schema.md",
                "Work/Design/Architecture/API.md",
                "Work/Design/Standalone.md",
            ]
        );
    }

    #[test]
    fn flat_subpages_use_hierarchical_prefixes() {
        let bundle = sample_bundle();
        let options = ExportOptions {
            subpages: SubpageStrategy::Flat,
            ..Default::default()
        };
        let plan = build_plan(&bundle, &options);
        let paths: Vec<String> = plan
            .pages
            .iter()
            .map(|p| p.relative_md_path.to_string_lossy().replace('\\', "/"))
            .collect();

        assert_eq!(
            paths,
            vec![
                "Work/Design/01 Architecture.md",
                "Work/Design/01.1 Storage.md",
                "Work/Design/01.1.1 Schema.md",
                "Work/Design/01.2 API.md",
                "Work/Design/02 Standalone.md",
            ]
        );
    }

    #[test]
    fn resolves_links_by_guid_and_title_fallback() {
        let bundle = sample_bundle();
        let plan = build_plan(&bundle, &ExportOptions::default());

        let by_guid = OneNoteLink::parse(
            "onenote:#WrongTitle&page-id={33333333-3333-3333-3333-333333333333}&end",
        )
        .unwrap();
        assert_eq!(
            plan.registry.resolve(&by_guid, "Design"),
            Some(Path::new("Work/Design/Architecture/Storage/Schema.md"))
        );

        let by_title = OneNoteLink::parse("onenote:#Standalone&end").unwrap();
        assert_eq!(
            plan.registry.resolve(&by_title, "Design"),
            Some(Path::new("Work/Design/Standalone.md"))
        );
    }

    #[test]
    fn avoids_assets_dir_collision_for_nested_parent_page() {
        let bundle = SourceBundle {
            notebooks: vec![Notebook {
                id: None,
                name: "Work".into(),
                entries: vec![SectionEntry::Section(Section {
                    id: None,
                    name: "Design".into(),
                    pages: vec![
                        page("11111111-1111-1111-1111-111111111111", "_assets", 0),
                        page("22222222-2222-2222-2222-222222222222", "Child", 1),
                    ],
                })],
            }],
            warnings: vec![],
        };

        let plan = build_plan(&bundle, &ExportOptions::default());
        assert_eq!(
            plan.pages[0].relative_md_path,
            Path::new("Work/Design/_assets (2)/_assets (2).md")
        );
        assert_eq!(
            plan.pages[1].relative_md_path,
            Path::new("Work/Design/_assets (2)/Child.md")
        );
    }
}
