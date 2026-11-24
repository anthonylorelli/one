use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::config::{ExportOptions, LinkStyle, TagStyle};
use crate::ir::link_registry::{LinkRegistry, PlannedPage};
use crate::ir::{Block, Inline, LinkTarget, ListItem, NoteTag};
use crate::render::report::UnresolvedLink;
use crate::render::sanitize::{encode_markdown_path, relative_path_from_file};

/// Result of rendering a single page to GFM Markdown.
#[derive(Debug)]
pub struct RenderedPage {
    pub markdown: String,
    pub links_resolved: usize,
    pub unresolved_links: Vec<UnresolvedLink>,
}

/// Render a [`PlannedPage`] into GitHub Flavored Markdown with optional YAML frontmatter.
pub fn render_page(
    planned: &PlannedPage<'_>,
    asset_paths: &HashMap<String, PathBuf>,
    registry: &LinkRegistry,
    options: &ExportOptions,
) -> RenderedPage {
    let mut ctx = RenderCtx {
        planned,
        asset_paths,
        registry,
        options,
        links_resolved: 0,
        unresolved_links: Vec::new(),
    };

    let mut out = String::new();

    if options.include_frontmatter {
        let page_tags = collect_page_tags(&planned.page.blocks);
        write_frontmatter(&mut out, planned, &page_tags);
    }

    // Emit page title as top-level H1 unless the first block is already an identical H1.
    let first_is_same_h1 = matches!(
        planned.page.blocks.first(),
        Some(Block::Heading { level: 1, inlines, .. })
            if plain_text_inlines(inlines).trim().eq_ignore_ascii_case(planned.page.title.trim())
    );
    if !first_is_same_h1 && !planned.page.title.trim().is_empty() {
        let single_line_title = planned.page.title.trim().replace(['\r', '\n'], " ");
        out.push_str("# ");
        out.push_str(&single_line_title);
        out.push_str("\n\n");
    }

    let body = ctx.render_blocks(&planned.page.blocks, 0);
    out.push_str(body.trim_end());
    if !out.ends_with('\n') {
        out.push('\n');
    }

    RenderedPage {
        markdown: out,
        links_resolved: ctx.links_resolved,
        unresolved_links: ctx.unresolved_links,
    }
}

struct RenderCtx<'a> {
    planned: &'a PlannedPage<'a>,
    asset_paths: &'a HashMap<String, PathBuf>,
    registry: &'a LinkRegistry,
    options: &'a ExportOptions,
    links_resolved: usize,
    unresolved_links: Vec<UnresolvedLink>,
}

impl<'a> RenderCtx<'a> {
    fn render_blocks(&mut self, blocks: &[Block], indent: usize) -> String {
        let mut rendered = Vec::new();
        for block in blocks {
            let chunk = self.render_block(block, indent);
            if !chunk.trim().is_empty() {
                rendered.push(chunk);
            }
        }
        rendered.join("\n\n")
    }

    fn render_block(&mut self, block: &Block, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match block {
            Block::Heading {
                level,
                inlines,
                tags,
            } => {
                let hashes = "#".repeat((*level).clamp(1, 6) as usize);
                let text = self.render_inlines(inlines);
                let (prefix, suffix) = format_non_checkbox_tags(tags, self.options.tag_style);
                format!("{pad}{hashes} {prefix}{}{suffix}", text.trim())
            }
            Block::Paragraph { inlines, tags } => {
                let text = self.render_inlines(inlines);
                let trimmed = text.trim();
                if trimmed.is_empty() && tags.is_empty() {
                    return String::new();
                }

                let checkbox = tags.iter().find(|t| t.is_checkbox).map(|t| t.completed);
                let (prefix, suffix) = format_non_checkbox_tags(tags, self.options.tag_style);

                if let Some(done) = checkbox {
                    let box_str = if done { "[x]" } else { "[ ]" };
                    let cont_pad = format!("{pad}  ");
                    let body = indent_continuation_lines(trimmed, &cont_pad);
                    format!("{pad}- {box_str} {prefix}{body}{suffix}")
                } else {
                    let body = indent_continuation_lines(trimmed, &pad);
                    format!("{pad}{prefix}{body}{suffix}")
                }
            }
            Block::List {
                ordered,
                start,
                items,
            } => self.render_list(*ordered, *start, items, indent),
            Block::Table {
                headers,
                rows,
                tags,
            } => self.render_table(headers.as_deref(), rows, tags, indent),
            Block::CodeBlock { language, code } => {
                let lang = language.as_deref().unwrap_or("");
                let fence = choose_code_fence(code);
                let indented_code = code
                    .lines()
                    .map(|l| format!("{pad}{l}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{pad}{fence}{lang}\n{indented_code}\n{pad}{fence}")
            }
            Block::Image {
                asset_id,
                alt,
                hyperlink,
                tags,
            } => {
                let (prefix, suffix) = format_non_checkbox_tags(tags, self.options.tag_style);
                let clean_alt = escape_bracket_text(alt);
                let dest = self.relative_asset_link(asset_id);
                let img_md = format!("![{clean_alt}]({dest})");
                let wrapped = if let Some(target) = hyperlink {
                    let link_dest = self.resolve_link_target(target, alt);
                    format!("[{img_md}]({link_dest})")
                } else {
                    img_md
                };
                format!("{pad}{prefix}{wrapped}{suffix}")
            }
            Block::Attachment {
                asset_id,
                filename,
                tags,
            } => {
                let (prefix, suffix) = format_non_checkbox_tags(tags, self.options.tag_style);
                let dest = self.relative_asset_link(asset_id);
                let label = escape_bracket_text(filename);
                format!("{pad}{prefix}[{label}]({dest}){suffix}")
            }
            Block::InkText { text } => {
                let lines = text
                    .lines()
                    .map(|l| format!("{pad}> {}", l.trim_end()))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{pad}> *[Recognized handwriting]*\n{lines}")
            }
            Block::Unsupported { kind, detail } => {
                format!("{pad}<!-- Unsupported OneNote {kind}: {detail} -->")
            }
        }
    }

    fn render_list(
        &mut self,
        ordered: bool,
        start: u32,
        items: &[ListItem],
        indent: usize,
    ) -> String {
        let mut lines = Vec::new();
        let pad = " ".repeat(indent);

        for (i, item) in items.iter().enumerate() {
            let checkbox = item
                .checkbox
                .or_else(|| item.tags.iter().find(|t| t.is_checkbox).map(|t| t.completed));

            let marker = if let Some(done) = checkbox {
                if done {
                    "- [x] ".to_string()
                } else {
                    "- [ ] ".to_string()
                }
            } else if ordered {
                format!("{}. ", start + i as u32)
            } else {
                "- ".to_string()
            };

            // In CommonMark/GFM, child indentation under ordered markers (`1. `, `10. `) must
            // align with the content column (`marker.len()`); task items still use `- ` bullets.
            let child_indent = if checkbox.is_some() {
                indent + 2
            } else {
                indent + marker.len()
            };
            let cont_pad = " ".repeat(child_indent);

            let (prefix, suffix) = format_non_checkbox_tags(&item.tags, self.options.tag_style);
            let mut first_line_text: Option<String> = None;
            let mut extra_blocks: Vec<String> = Vec::new();

            for block in &item.content {
                match block {
                    Block::Paragraph { inlines, tags } if first_line_text.is_none() => {
                        let text = self.render_inlines(inlines);
                        let (p, s) = format_non_checkbox_tags(tags, self.options.tag_style);
                        let combined = format!("{p}{}{s}", text.trim());
                        if !combined.trim().is_empty() {
                            first_line_text = Some(indent_continuation_lines(&combined, &cont_pad));
                        }
                    }
                    Block::Image { .. } | Block::Attachment { .. } if first_line_text.is_none() => {
                        let rendered = self.render_block(block, 0);
                        if !rendered.trim().is_empty() {
                            first_line_text = Some(rendered);
                        }
                    }
                    other => {
                        let rendered = self.render_block(other, child_indent);
                        if !rendered.trim().is_empty() {
                            extra_blocks.push(rendered);
                        }
                    }
                }
            }

            let lead = first_line_text.unwrap_or_default();
            lines.push(format!("{pad}{marker}{prefix}{}{suffix}", lead.trim()));

            for extra in extra_blocks {
                lines.push(extra);
            }

            if !item.children.is_empty() {
                let mut child_chunks = Vec::new();
                for child in &item.children {
                    let rendered_child = self.render_block(child, child_indent);
                    if !rendered_child.trim().is_empty() {
                        child_chunks.push(rendered_child);
                    }
                }
                if !child_chunks.is_empty() {
                    lines.push(child_chunks.join("\n"));
                }
            }
        }

        lines.join("\n")
    }

    fn render_table(
        &mut self,
        headers: Option<&[Vec<Block>]>,
        rows: &[Vec<Vec<Block>>],
        tags: &[NoteTag],
        indent: usize,
    ) -> String {
        let (header_row, body_rows): (Vec<String>, Vec<Vec<String>>) = match headers {
            Some(hdr) => {
                let h = hdr.iter().map(|cell| self.render_table_cell(cell)).collect();
                let b = rows
                    .iter()
                    .map(|r| r.iter().map(|cell| self.render_table_cell(cell)).collect())
                    .collect();
                (h, b)
            }
            None => {
                if let Some((first, rest)) = rows.split_first() {
                    let h = first
                        .iter()
                        .map(|cell| self.render_table_cell(cell))
                        .collect();
                    let b = rest
                        .iter()
                        .map(|r| r.iter().map(|cell| self.render_table_cell(cell)).collect())
                        .collect();
                    (h, b)
                } else {
                    return String::new();
                }
            }
        };

        let col_count = std::iter::once(header_row.len())
            .chain(body_rows.iter().map(Vec::len))
            .max()
            .unwrap_or(0);
        if col_count == 0 {
            return String::new();
        }

        let pad = " ".repeat(indent);
        let mut lines = Vec::new();

        let (prefix, suffix) = format_non_checkbox_tags(tags, self.options.tag_style);
        if !prefix.is_empty() || !suffix.is_empty() {
            lines.push(format!("{pad}{}{}", prefix.trim(), suffix.trim()));
        }

        lines.push(format!("{pad}{}", format_pipe_row(&header_row, col_count)));
        let sep = vec!["---".to_string(); col_count];
        lines.push(format!("{pad}{}", format_pipe_row(&sep, col_count)));
        for row in &body_rows {
            lines.push(format!("{pad}{}", format_pipe_row(row, col_count)));
        }

        lines.join("\n")
    }

    fn render_table_cell(&mut self, blocks: &[Block]) -> String {
        let mut parts = Vec::new();
        for block in blocks {
            let rendered = self.render_block(block, 0);
            for line in rendered.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.replace('|', "\\|"));
                }
            }
        }
        parts.join("<br>")
    }

    fn render_inlines(&mut self, inlines: &[Inline]) -> String {
        let mut out = String::new();
        for inline in inlines {
            out.push_str(&self.render_inline(inline));
        }
        out
    }

    fn render_inline(&mut self, inline: &Inline) -> String {
        match inline {
            Inline::Text(s) => escape_markdown_text(s),
            Inline::Code(s) => format_inline_code(s),
            Inline::Bold(children) => wrap_emphasis(&self.render_inlines(children), "**"),
            Inline::Italic(children) => wrap_emphasis(&self.render_inlines(children), "*"),
            Inline::Strikethrough(children) => wrap_emphasis(&self.render_inlines(children), "~~"),
            Inline::LineBreak => "  \n".to_string(),
            Inline::Link { text, target } => self.render_link(text, target),
        }
    }

    fn render_link(&mut self, text_inlines: &[Inline], target: &LinkTarget) -> String {
        let label = self.render_inlines(text_inlines);
        let plain_label = plain_text_inlines(text_inlines);

        match target {
            LinkTarget::External(url) => {
                let display = if label.trim().is_empty() {
                    url.clone()
                } else {
                    label
                };
                format!("[{display}]({})", encode_external_url(url))
            }
            LinkTarget::OneNote(link) => {
                if let Some(target_path) = self.registry.resolve(link, &self.planned.section_name) {
                    self.links_resolved += 1;
                    match self.options.links {
                        LinkStyle::Relative => {
                            let rel =
                                relative_path_from_file(&self.planned.relative_md_path, target_path);
                            let encoded = encode_markdown_path(&rel);
                            let display = if label.trim().is_empty() {
                                target_path
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| encoded.clone())
                            } else {
                                label
                            };
                            format!("[{display}]({encoded})")
                        }
                        LinkStyle::Wikilink => {
                            let target_stem = target_path
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if plain_label.trim().is_empty()
                                || plain_label.trim().eq_ignore_ascii_case(&target_stem)
                            {
                                format!("[[{target_stem}]]")
                            } else {
                                format!("[[{target_stem}|{}]]", plain_label.trim())
                            }
                        }
                    }
                } else {
                    self.unresolved_links.push(UnresolvedLink {
                        source_page: self.planned.relative_md_path.clone(),
                        link_text: plain_label.clone(),
                        target_raw: link.raw.clone(),
                    });
                    let display = if label.trim().is_empty() {
                        link.page_title
                            .clone()
                            .unwrap_or_else(|| link.raw.clone())
                    } else {
                        label
                    };
                    format!("[{display}](<{}>)", link.raw)
                }
            }
        }
    }

    fn resolve_link_target(&mut self, target: &LinkTarget, label: &str) -> String {
        match target {
            LinkTarget::External(url) => encode_external_url(url),
            LinkTarget::OneNote(link) => {
                if let Some(target_path) = self.registry.resolve(link, &self.planned.section_name) {
                    self.links_resolved += 1;
                    let rel = relative_path_from_file(&self.planned.relative_md_path, target_path);
                    encode_markdown_path(&rel)
                } else {
                    self.unresolved_links.push(UnresolvedLink {
                        source_page: self.planned.relative_md_path.clone(),
                        link_text: label.to_string(),
                        target_raw: link.raw.clone(),
                    });
                    format!("<{}>", link.raw)
                }
            }
        }
    }

    fn relative_asset_link(&self, asset_id: &str) -> String {
        if let Some(asset_rel_to_root) = self.asset_paths.get(asset_id) {
            let rel = relative_path_from_file(&self.planned.relative_md_path, asset_rel_to_root);
            encode_markdown_path(&rel)
        } else {
            encode_markdown_path(Path::new(asset_id))
        }
    }
}

fn write_frontmatter(out: &mut String, planned: &PlannedPage<'_>, tags: &[String]) {
    out.push_str("---\n");
    push_yaml_str(out, "title", &planned.page.title);
    if let Some(created) = planned.page.created_at {
        push_yaml_raw(out, "created", &created.to_rfc3339());
    }
    if let Some(updated) = planned.page.updated_at {
        push_yaml_raw(out, "updated", &updated.to_rfc3339());
    }
    if let Some(author) = planned.page.author.as_deref().filter(|a| !a.trim().is_empty()) {
        push_yaml_str(out, "author", author);
    }
    push_yaml_str(out, "notebook", &planned.notebook_name);
    if !planned.section_group_path.is_empty() {
        push_yaml_str(out, "section_group", &planned.section_group_path.join("/"));
    }
    push_yaml_str(out, "section", &planned.section_name);
    if let Some(id) = planned.page.id.as_deref() {
        push_yaml_str(out, "onenote_id", id);
    }
    if !tags.is_empty() {
        out.push_str("tags:\n");
        for tag in tags {
            out.push_str("  - ");
            out.push_str(&yaml_quote(tag));
            out.push('\n');
        }
    }
    out.push_str("---\n\n");
}

fn push_yaml_str(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(&yaml_quote(value));
    out.push('\n');
}

fn push_yaml_raw(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

fn yaml_quote(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => {}
            '\t' => escaped.push_str("\\t"),
            c if (c as u32) < 0x20 => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    format!("\"{escaped}\"")
}

fn indent_continuation_lines(text: &str, pad: &str) -> String {
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut out = first.to_string();
    for line in lines {
        out.push('\n');
        out.push_str(pad);
        out.push_str(line);
    }
    out
}

fn collect_page_tags(blocks: &[Block]) -> Vec<String> {
    let mut set = BTreeSet::new();
    collect_tags_recursive(blocks, &mut set);
    set.into_iter().collect()
}

fn push_non_checkbox_tags(tags: &[NoteTag], out: &mut BTreeSet<String>) {
    for tag in tags {
        if !tag.is_checkbox {
            let slug = normalize_tag_slug(&tag.label);
            if !slug.is_empty() {
                out.insert(slug);
            }
        }
    }
}

fn collect_tags_recursive(blocks: &[Block], out: &mut BTreeSet<String>) {
    for block in blocks {
        match block {
            Block::Heading { tags, .. }
            | Block::Paragraph { tags, .. }
            | Block::Image { tags, .. }
            | Block::Attachment { tags, .. } => push_non_checkbox_tags(tags, out),
            Block::List { items, .. } => {
                for item in items {
                    push_non_checkbox_tags(&item.tags, out);
                    collect_tags_recursive(&item.content, out);
                    collect_tags_recursive(&item.children, out);
                }
            }
            Block::Table {
                headers,
                rows,
                tags,
            } => {
                push_non_checkbox_tags(tags, out);
                if let Some(hdr) = headers {
                    for cell in hdr {
                        collect_tags_recursive(cell, out);
                    }
                }
                for row in rows {
                    for cell in row {
                        collect_tags_recursive(cell, out);
                    }
                }
            }
            Block::CodeBlock { .. } | Block::InkText { .. } | Block::Unsupported { .. } => {}
        }
    }
}

fn format_non_checkbox_tags(tags: &[NoteTag], style: TagStyle) -> (String, String) {
    let non_cb: Vec<&NoteTag> = tags.iter().filter(|t| !t.is_checkbox).collect();
    if non_cb.is_empty() {
        return (String::new(), String::new());
    }

    match style {
        TagStyle::Frontmatter => (String::new(), String::new()),
        TagStyle::Inline => {
            let rendered = non_cb
                .iter()
                .map(|t| normalize_tag_slug(&t.label))
                .filter(|s| !s.is_empty())
                .map(|s| format!("#{s}"))
                .collect::<Vec<_>>()
                .join(" ");
            if rendered.is_empty() {
                (String::new(), String::new())
            } else {
                (String::new(), format!(" {rendered}"))
            }
        }
        TagStyle::Emoji => {
            let emojis = non_cb
                .iter()
                .map(|t| t.kind.emoji())
                .collect::<Vec<_>>()
                .join("");
            (format!("{emojis} "), String::new())
        }
    }
}

/// Normalize a OneNote tag label into a clean kebab-case Markdown tag slug.
pub fn normalize_tag_slug(label: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in label.trim().chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                slug.push(lc);
            }
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn plain_text_inlines(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(s) | Inline::Code(s) => out.push_str(s),
            Inline::Bold(c) | Inline::Italic(c) | Inline::Strikethrough(c) => {
                out.push_str(&plain_text_inlines(c));
            }
            Inline::Link { text, .. } => out.push_str(&plain_text_inlines(text)),
            Inline::LineBreak => out.push(' '),
        }
    }
    out
}

/// Keep leading/trailing whitespace outside Markdown emphasis delimiters so GFM parses cleanly.
fn wrap_emphasis(content: &str, delim: &str) -> String {
    if content.trim().is_empty() {
        return content.to_string();
    }
    let trimmed_start = content.trim_start();
    let leading = &content[..content.len() - trimmed_start.len()];
    let inner = trimmed_start.trim_end();
    let trailing = &trimmed_start[inner.len()..];
    format!("{leading}{delim}{inner}{delim}{trailing}")
}

fn format_inline_code(code: &str) -> String {
    let max_backticks = code
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let ticks = "`".repeat(max_backticks + 1);
    if code.starts_with('`') || code.ends_with('`') || (code.starts_with(' ') && code.ends_with(' '))
    {
        format!("{ticks} {code} {ticks}")
    } else {
        format!("{ticks}{code}{ticks}")
    }
}

fn choose_code_fence(code: &str) -> String {
    let max_ticks = code
        .lines()
        .map(|l| l.chars().take_while(|&c| c == '`').count())
        .max()
        .unwrap_or(0);
    "`".repeat(max_ticks.max(2) + 1)
}

fn format_pipe_row(cells: &[String], col_count: usize) -> String {
    let mut row = String::from("|");
    for idx in 0..col_count {
        row.push(' ');
        if let Some(cell) = cells.get(idx) {
            row.push_str(cell);
        }
        row.push_str(" |");
    }
    row
}

fn escape_markdown_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' | '*' | '_' | '[' | ']' | '`' | '~' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn escape_bracket_text(text: &str) -> String {
    text.replace('[', "\\[").replace(']', "\\]").replace('\n', " ")
}

fn encode_external_url(url: &str) -> String {
    if url.contains([' ', '(', ')']) {
        format!("<{url}>")
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::ir::link_registry::build_plan;
    use crate::ir::{NoteTagKind, Notebook, Page, Section, SectionEntry, SourceBundle};

    #[test]
    fn renders_full_gfm_page_with_frontmatter_tags_tasks_tables_and_links() {
        let created = chrono::Utc.with_ymd_and_hms(2025, 3, 14, 18, 22, 0).unwrap();
        let updated = chrono::Utc.with_ymd_and_hms(2026, 1, 9, 9, 15, 42).unwrap();

        let bundle = SourceBundle {
            notebooks: vec![Notebook {
                id: None,
                name: "Engineering".into(),
                entries: vec![
                    SectionEntry::Section(Section {
                        id: Some("sec-a".into()),
                        name: "Specs".into(),
                        pages: vec![Page {
                            id: Some("11111111-1111-1111-1111-111111111111".into()),
                            title: "Overview".into(),
                            level: 0,
                            created_at: Some(created),
                            updated_at: Some(updated),
                            author: Some("Alice".into()),
                            web_url: None,
                            client_url: None,
                            blocks: vec![
                                Block::Paragraph {
                                    inlines: vec![
                                        Inline::Text("See ".into()),
                                        Inline::Link {
                                            text: vec![Inline::Text("API Spec".into())],
                                            target: LinkTarget::parse(
                                                "onenote:#API%20Spec&page-id={22222222-2222-2222-2222-222222222222}&end",
                                            ),
                                        },
                                        Inline::Text(" and ".into()),
                                        Inline::Bold(vec![Inline::Text("deploy ".into())]),
                                        Inline::Code("cargo build".into()),
                                    ],
                                    tags: vec![NoteTag {
                                        label: "Important".into(),
                                        is_checkbox: false,
                                        completed: false,
                                        kind: NoteTagKind::Important,
                                    }],
                                },
                                Block::List {
                                    ordered: false,
                                    start: 1,
                                    items: vec![
                                        ListItem {
                                            checkbox: Some(false),
                                            tags: vec![],
                                            content: vec![Block::Paragraph {
                                                inlines: vec![Inline::Text("Write tests".into())],
                                                tags: vec![],
                                            }],
                                            children: vec![],
                                        },
                                        ListItem {
                                            checkbox: Some(true),
                                            tags: vec![],
                                            content: vec![Block::Paragraph {
                                                inlines: vec![Inline::Text("Design IR".into())],
                                                tags: vec![],
                                            }],
                                            children: vec![],
                                        },
                                    ],
                                },
                                Block::Table {
                                    headers: None,
                                    rows: vec![
                                        vec![
                                            vec![Block::Paragraph {
                                                inlines: vec![Inline::Text("Key".into())],
                                                tags: vec![],
                                            }],
                                            vec![Block::Paragraph {
                                                inlines: vec![Inline::Text("Value".into())],
                                                tags: vec![],
                                            }],
                                        ],
                                        vec![
                                            vec![Block::Paragraph {
                                                inlines: vec![Inline::Text("Format".into())],
                                                tags: vec![],
                                            }],
                                            vec![Block::Paragraph {
                                                inlines: vec![Inline::Text("GFM | Pipe".into())],
                                                tags: vec![],
                                            }],
                                        ],
                                    ],
                                    tags: vec![],
                                },
                            ],
                            assets: vec![],
                        }],
                    }),
                    SectionEntry::Section(Section {
                        id: Some("sec-b".into()),
                        name: "Reference".into(),
                        pages: vec![Page {
                            id: Some("22222222-2222-2222-2222-222222222222".into()),
                            title: "API Spec".into(),
                            level: 0,
                            created_at: None,
                            updated_at: None,
                            author: None,
                            web_url: None,
                            client_url: None,
                            blocks: vec![],
                            assets: vec![],
                        }],
                    }),
                ],
            }],
            warnings: vec![],
        };

        let opts = ExportOptions::default();
        let plan = build_plan(&bundle, &opts);
        let rendered = render_page(&plan.pages[0], &HashMap::new(), &plan.registry, &opts);

        assert_eq!(rendered.links_resolved, 1);
        assert!(rendered.unresolved_links.is_empty());
        assert!(rendered.markdown.contains("title: \"Overview\""));
        assert!(rendered.markdown.contains("created: 2025-03-14T18:22:00+00:00"));
        assert!(rendered.markdown.contains("  - \"important\""));
        assert!(rendered.markdown.contains("[API Spec](../Reference/API%20Spec.md)"));
        assert!(rendered.markdown.contains("**deploy** `cargo build` #important"));
        assert!(rendered.markdown.contains("- [ ] Write tests\n- [x] Design IR"));
        assert!(rendered.markdown.contains("| Key | Value |\n| --- | --- |\n| Format | GFM \\| Pipe |"));
    }

    #[test]
    fn indents_ordered_sublists_and_multiline_items_per_gfm() {
        let bundle = SourceBundle {
            notebooks: vec![Notebook {
                id: None,
                name: "Nb".into(),
                entries: vec![SectionEntry::Section(Section {
                    id: None,
                    name: "Sec".into(),
                    pages: vec![Page {
                        id: None,
                        title: "Lists".into(),
                        level: 0,
                        created_at: None,
                        updated_at: None,
                        author: None,
                        web_url: None,
                        client_url: None,
                        blocks: vec![Block::List {
                            ordered: true,
                            start: 1,
                            items: vec![ListItem {
                                checkbox: None,
                                tags: vec![],
                                content: vec![
                                    Block::Paragraph {
                                        inlines: vec![
                                            Inline::Text("First line".into()),
                                            Inline::LineBreak,
                                            Inline::Text("Second line".into()),
                                        ],
                                        tags: vec![],
                                    },
                                    Block::CodeBlock {
                                        language: Some("rust".into()),
                                        code: "fn main() {}\nlet x = 1;".into(),
                                    },
                                ],
                                children: vec![Block::List {
                                    ordered: false,
                                    start: 1,
                                    items: vec![ListItem {
                                        checkbox: None,
                                        tags: vec![],
                                        content: vec![Block::Paragraph {
                                            inlines: vec![Inline::Text("Nested child".into())],
                                            tags: vec![],
                                        }],
                                        children: vec![],
                                    }],
                                }],
                            }],
                        }],
                        assets: vec![],
                    }],
                })],
            }],
            warnings: vec![],
        };

        let opts = ExportOptions {
            include_frontmatter: false,
            ..Default::default()
        };
        let plan = build_plan(&bundle, &opts);
        let rendered = render_page(&plan.pages[0], &HashMap::new(), &plan.registry, &opts);

        assert!(
            rendered.markdown.contains(
                "1. First line  \n   Second line\n   ```rust\n   fn main() {}\n   let x = 1;\n   ```\n   - Nested child"
            ),
            "Unexpected markdown:\n{}",
            rendered.markdown
        );
    }

    #[test]
    fn yaml_quote_escapes_tabs_and_control_chars() {
        assert_eq!(yaml_quote("a\tb\nc\x07d"), "\"a\\tb\\nc d\"");
    }

    #[test]
    fn renders_image_only_list_item_inline_and_indents_child_paragraph_breaks() {
        let bundle = SourceBundle {
            notebooks: vec![Notebook {
                id: None,
                name: "Nb".into(),
                entries: vec![SectionEntry::Section(Section {
                    id: None,
                    name: "Sec".into(),
                    pages: vec![Page {
                        id: None,
                        title: "Line1\nLine2".into(),
                        level: 0,
                        created_at: None,
                        updated_at: None,
                        author: None,
                        web_url: None,
                        client_url: None,
                        blocks: vec![Block::List {
                            ordered: false,
                            start: 1,
                            items: vec![ListItem {
                                checkbox: None,
                                tags: vec![],
                                content: vec![Block::Image {
                                    asset_id: "img-1".into(),
                                    alt: "Logo".into(),
                                    hyperlink: None,
                                    tags: vec![],
                                }],
                                children: vec![Block::Paragraph {
                                    inlines: vec![
                                        Inline::Text("Child line 1".into()),
                                        Inline::LineBreak,
                                        Inline::Text("Child line 2".into()),
                                    ],
                                    tags: vec![],
                                }],
                            }],
                        }],
                        assets: vec![],
                    }],
                })],
            }],
            warnings: vec![],
        };

        let opts = ExportOptions {
            include_frontmatter: false,
            ..Default::default()
        };
        let plan = build_plan(&bundle, &opts);
        let mut assets = HashMap::new();
        assets.insert("img-1".into(), PathBuf::from("Nb/Sec/_assets/logo.png"));
        let rendered = render_page(&plan.pages[0], &assets, &plan.registry, &opts);

        assert!(rendered.markdown.starts_with("# Line1 Line2\n\n"));
        assert!(
            rendered
                .markdown
                .contains("- ![Logo](_assets/logo.png)\n  Child line 1  \n  Child line 2"),
            "Unexpected markdown:\n{}",
            rendered.markdown
        );
    }
}
