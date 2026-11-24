use std::io::Read;

use chrono::{DateTime, Utc};
use onenote_parser::contents::{
    Content, EmbeddedFile, FileDataStatus, Image, List, NoteTag as RawNoteTag, Outline,
    OutlineElement, OutlineItem, ParagraphStyling, RichText, Table, TextHyperlink,
};
use onenote_parser::notebook::Notebook as RawNotebook;
use onenote_parser::page::{Page as RawPage, PageContent};
use onenote_parser::property::note_tag::NoteTagShape;
use onenote_parser::section::{
    Section as RawSection, SectionEntry as RawSectionEntry, SectionGroup as RawSectionGroup,
};
use onenote_parser::warn::Report as ParserReport;

use crate::ir::{
    Asset, Block, ExportWarning, Inline, LinkTarget, ListItem, NoteTag, NoteTagKind, Notebook,
    Page, Section, SectionEntry, SectionGroup, normalize_guid,
};
use crate::render::sanitize::sanitize_filename;

/// Convert a parsed [`onenote_parser::notebook::Notebook`] into our canonical [`Notebook`].
pub fn convert_notebook(
    raw: &RawNotebook,
    name: String,
    warnings: &mut Vec<ExportWarning>,
) -> Notebook {
    collect_parser_warnings(&name, raw.report(), warnings);

    let entries = raw
        .entries()
        .iter()
        .map(|entry| convert_section_entry(entry, warnings))
        .collect();

    Notebook {
        id: Some(raw.file_identity().to_string()),
        name,
        entries,
    }
}

fn convert_section_entry(
    raw: &RawSectionEntry,
    warnings: &mut Vec<ExportWarning>,
) -> SectionEntry {
    match raw {
        RawSectionEntry::Section(sec) => SectionEntry::Section(convert_section(sec, warnings)),
        RawSectionEntry::SectionGroup(group) => {
            SectionEntry::SectionGroup(convert_section_group(group, warnings))
        }
    }
}

fn convert_section_group(
    raw: &RawSectionGroup,
    warnings: &mut Vec<ExportWarning>,
) -> SectionGroup {
    let entries = raw
        .entries()
        .iter()
        .map(|entry| convert_section_entry(entry, warnings))
        .collect();

    SectionGroup {
        id: Some(raw.file_identity().to_string()),
        name: raw.display_name().to_string(),
        entries,
    }
}

/// Convert a parsed [`onenote_parser::section::Section`] into our canonical [`Section`].
pub fn convert_section(raw: &RawSection, warnings: &mut Vec<ExportWarning>) -> Section {
    let sec_name = raw.display_name().to_string();
    collect_parser_warnings(&sec_name, raw.report(), warnings);

    let mut pages = Vec::new();
    for series in raw.page_series() {
        for raw_page in series.pages() {
            pages.push(convert_page(raw_page, &sec_name, warnings));
        }
    }

    Section {
        id: Some(raw.file_identity().to_string()),
        name: sec_name,
        pages,
    }
}

fn convert_page(raw: &RawPage, section_name: &str, warnings: &mut Vec<ExportWarning>) -> Page {
    let title = raw
        .title_text()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Untitled Page")
        .to_string();

    let scope = format!("{section_name}/{title}");
    let mut page_ctx = PageBuildCtx {
        scope: &scope,
        assets: Vec::new(),
        asset_seq: 0,
        has_ink: false,
        warnings,
    };

    let mut blocks = Vec::new();
    for content in raw.contents() {
        match content {
            PageContent::Outline(outline) => {
                blocks.extend(page_ctx.convert_outline(outline));
            }
            PageContent::Image(image) => {
                if let Some(block) = page_ctx.convert_image(image) {
                    blocks.push(block);
                }
            }
            PageContent::EmbeddedFile(file) => {
                if let Some(block) = page_ctx.convert_embedded_file(file) {
                    blocks.push(block);
                }
            }
            PageContent::Ink(_) => {
                page_ctx.has_ink = true;
            }
            PageContent::Unknown => {
                page_ctx.warnings.push(ExportWarning::new(
                    page_ctx.scope,
                    "Skipped unknown OneNote page content node",
                ));
            }
        }
    }

    // If the page has handwriting recognition OCR, emit it as a clean blockquote;
    // otherwise warn when unrecognized ink strokes were skipped.
    if let Some(ink_ocr) = raw.ink_recognition() {
        let recognized = ink_ocr.text();
        if !recognized.trim().is_empty() {
            blocks.push(Block::InkText { text: recognized });
        }
    } else if page_ctx.has_ink {
        page_ctx.warnings.push(ExportWarning::new(
            page_ctx.scope,
            "Page contains handwritten ink without OCR text; ink strokes were not exported to Markdown",
        ));
    }

    let level = raw.level().clamp(0, u8::MAX as i32) as u8;
    let created_at = convert_utc_time(raw.created_time());
    let updated_at = convert_utc_time(raw.updated_time());

    Page {
        id: normalize_guid(raw.link_target_id()),
        title,
        level,
        created_at,
        updated_at,
        author: raw.author().map(str::to_string),
        web_url: None,
        client_url: None,
        blocks,
        assets: page_ctx.assets,
    }
}

struct PageBuildCtx<'a> {
    scope: &'a str,
    assets: Vec<Asset>,
    asset_seq: usize,
    has_ink: bool,
    warnings: &'a mut Vec<ExportWarning>,
}

impl<'a> PageBuildCtx<'a> {
    fn next_asset_id(&mut self) -> String {
        self.asset_seq += 1;
        format!("asset-{}", self.asset_seq)
    }

    fn convert_outline(&mut self, outline: &Outline) -> Vec<Block> {
        self.convert_outline_items(outline.items())
    }

    fn convert_outline_items(&mut self, items: &[OutlineItem]) -> Vec<Block> {
        let mut blocks: Vec<Block> = Vec::new();
        let mut idx = 0;

        while idx < items.len() {
            match &items[idx] {
                OutlineItem::Group(group) => {
                    blocks.extend(self.convert_outline_items(group.outlines()));
                    idx += 1;
                }
                OutlineItem::Element(elem) => {
                    if let Some(list_kind) = detect_element_list_kind(elem) {
                        // Coalesce consecutive sibling list items of the same list kind.
                        let start = elem
                            .list_contents()
                            .first()
                            .and_then(List::list_restart)
                            .filter(|&n| n > 0)
                            .map(|n| n as u32)
                            .unwrap_or(1);

                        let mut list_items = Vec::new();
                        while idx < items.len() {
                            if let OutlineItem::Element(next_elem) = &items[idx]
                                && detect_element_list_kind(next_elem) == Some(list_kind)
                            {
                                list_items.push(self.convert_list_item(next_elem));
                                idx += 1;
                                continue;
                            }
                            break;
                        }

                        blocks.push(Block::List {
                            ordered: list_kind == ElementListKind::Ordered,
                            start,
                            items: list_items,
                        });
                    } else {
                        blocks.extend(self.convert_plain_outline_element(elem));
                        idx += 1;
                    }
                }
            }
        }

        blocks
    }

    fn convert_list_item(&mut self, elem: &OutlineElement) -> ListItem {
        let mut content = Vec::new();
        let mut item_tags = Vec::new();

        for c in elem.contents() {
            if let Content::RichText(rt) = c {
                let tags = convert_note_tags(rt.note_tags());
                item_tags.extend(tags);
                let inlines = convert_rich_text_inlines(rt);
                if !inlines.is_empty() {
                    content.push(Block::Paragraph {
                        inlines,
                        tags: Vec::new(),
                    });
                }
            } else if let Some(b) = self.convert_content(c) {
                content.push(b);
            }
        }

        let checkbox = item_tags
            .iter()
            .find(|t| t.is_checkbox)
            .map(|t| t.completed);

        let children = self.convert_outline_items(elem.children());

        ListItem {
            checkbox,
            tags: item_tags,
            content,
            children,
        }
    }

    fn convert_plain_outline_element(&mut self, elem: &OutlineElement) -> Vec<Block> {
        let mut out = Vec::new();
        for c in elem.contents() {
            if let Some(block) = self.convert_content(c) {
                out.push(block);
            }
        }
        if !elem.children().is_empty() {
            out.extend(self.convert_outline_items(elem.children()));
        }
        out
    }

    fn convert_content(&mut self, content: &Content) -> Option<Block> {
        match content {
            Content::RichText(rt) => convert_rich_text_block(rt),
            Content::Table(table) => Some(self.convert_table(table)),
            Content::Image(image) => self.convert_image(image),
            Content::EmbeddedFile(file) => self.convert_embedded_file(file),
            Content::Ink(_) => {
                self.has_ink = true;
                None
            }
            Content::Unknown => {
                self.warnings.push(ExportWarning::new(
                    self.scope,
                    "Skipped unknown outline content block",
                ));
                None
            }
        }
    }

    fn convert_table(&mut self, table: &Table) -> Block {
        let mut rows = Vec::new();
        for row in table.contents() {
            let mut cells = Vec::new();
            for cell in row.contents() {
                cells.push(self.convert_outline_elements(cell.contents()));
            }
            rows.push(cells);
        }

        Block::Table {
            headers: None,
            rows,
            tags: convert_note_tags(table.note_tags()),
        }
    }

    fn convert_outline_elements(&mut self, elements: &[OutlineElement]) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut idx = 0;

        while idx < elements.len() {
            let elem = &elements[idx];
            if let Some(list_kind) = detect_element_list_kind(elem) {
                let start = elem
                    .list_contents()
                    .first()
                    .and_then(List::list_restart)
                    .filter(|&n| n > 0)
                    .map(|n| n as u32)
                    .unwrap_or(1);

                let mut list_items = Vec::new();
                while idx < elements.len()
                    && detect_element_list_kind(&elements[idx]) == Some(list_kind)
                {
                    list_items.push(self.convert_list_item(&elements[idx]));
                    idx += 1;
                }

                blocks.push(Block::List {
                    ordered: list_kind == ElementListKind::Ordered,
                    start,
                    items: list_items,
                });
            } else {
                blocks.extend(self.convert_plain_outline_element(elem));
                idx += 1;
            }
        }

        blocks
    }

    fn convert_image(&mut self, image: &Image) -> Option<Block> {
        let (bytes, raw_ext) = if image.data_status() == FileDataStatus::Available {
            if let Some(mut reader) = image.read() {
                let mut buf = Vec::new();
                match reader.read_to_end(&mut buf) {
                    Ok(_) if !buf.is_empty() => (Some(buf), image.extension().map(str::to_string)),
                    Ok(_) => (None, None),
                    Err(err) => {
                        self.warnings.push(ExportWarning::new(
                            self.scope,
                            format!("Failed reading image payload: {err}"),
                        ));
                        (None, None)
                    }
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        // Fallback to browser-compatible web_picture if primary image data is unavailable
        let (bytes, raw_ext) = match bytes {
            Some(b) => (Some(b), raw_ext),
            None => {
                if let Some(web_pic) = image.web_picture()
                    && web_pic.data_status() == FileDataStatus::Available
                {
                    let mut buf = Vec::new();
                    if web_pic.read().read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                        (Some(buf), web_pic.extension().map(str::to_string))
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            }
        };

        let Some(data) = bytes else {
            self.warnings.push(ExportWarning::new(
                self.scope,
                "Image has no available binary payload; skipped",
            ));
            return Some(Block::Unsupported {
                kind: "image".to_string(),
                detail: image
                    .image_filename()
                    .unwrap_or("missing image payload")
                    .to_string(),
            });
        };

        let asset_id = self.next_asset_id();
        let sniffed_ext = sniff_extension(&data);
        let fallback_ext = raw_ext
            .as_deref()
            .map(|e| e.trim_start_matches('.'))
            .filter(|e| !e.is_empty())
            .or(sniffed_ext)
            .or(Some("png"));

        let raw_name = image.image_filename().unwrap_or(&asset_id);
        let filename = sanitize_filename(raw_name, &asset_id, fallback_ext);
        let alt = image
            .alt_text()
            .or_else(|| image.image_filename())
            .unwrap_or("")
            .trim()
            .to_string();

        self.assets.push(Asset {
            id: asset_id.clone(),
            filename,
            data,
        });

        Some(Block::Image {
            asset_id,
            alt,
            hyperlink: image.hyperlink_url().map(LinkTarget::parse),
            tags: convert_note_tags(image.note_tags()),
        })
    }

    fn convert_embedded_file(&mut self, file: &EmbeddedFile) -> Option<Block> {
        if file.data_status() != FileDataStatus::Available {
            self.warnings.push(ExportWarning::new(
                self.scope,
                format!(
                    "Embedded file '{}' has unavailable payload ({:?}); skipped",
                    file.filename(),
                    file.data_status()
                ),
            ));
            return Some(Block::Unsupported {
                kind: "attachment".to_string(),
                detail: file.filename().to_string(),
            });
        }

        let mut data = Vec::new();
        if let Err(err) = file.read().read_to_end(&mut data) {
            self.warnings.push(ExportWarning::new(
                self.scope,
                format!("Failed reading embedded file '{}': {err}", file.filename()),
            ));
            return None;
        }

        let asset_id = self.next_asset_id();
        let filename = sanitize_filename(file.filename(), &asset_id, sniff_extension(&data));
        let display_name = if file.filename().trim().is_empty() {
            filename.clone()
        } else {
            file.filename().trim().to_string()
        };

        self.assets.push(Asset {
            id: asset_id.clone(),
            filename,
            data,
        });

        Some(Block::Attachment {
            asset_id,
            filename: display_name,
            tags: convert_note_tags(file.note_tags()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementListKind {
    Ordered,
    Unordered,
    Task,
}

fn detect_element_list_kind(elem: &OutlineElement) -> Option<ElementListKind> {
    if let Some(list) = elem.list_contents().first() {
        return Some(if is_ordered_list(list) {
            ElementListKind::Ordered
        } else {
            ElementListKind::Unordered
        });
    }

    // Treat standalone paragraphs with checkbox NoteTags as task-list items so consecutive
    // checkboxes form a clean GFM task list without extra blank lines between items.
    let has_checkbox = elem.contents().iter().any(|c| {
        matches!(c, Content::RichText(rt) if rt.note_tags().iter().any(is_checkbox_tag))
    });
    if has_checkbox {
        return Some(ElementListKind::Task);
    }

    None
}

fn is_ordered_list(list: &List) -> bool {
    is_ordered_list_format(list.list_format())
}

fn is_ordered_list_format(fmt: &[char]) -> bool {
    if fmt.is_empty() {
        return false;
    }
    // Per [MS-ONE] 2.3.20, numbered list formats embed low control-range numbering specifiers
    // (0x0000..=0x001F), whereas bulleted lists contain a single Unicode bullet glyph.
    fmt.iter().any(|&c| (c as u32) <= 0x1F)
}

fn convert_rich_text_block(rt: &RichText) -> Option<Block> {
    let inlines = convert_rich_text_inlines(rt);
    let tags = convert_note_tags(rt.note_tags());

    if inlines.is_empty() && tags.is_empty() {
        return None;
    }

    if let Some(level) = detect_heading_level(rt.paragraph_style()) {
        return Some(Block::Heading {
            level,
            inlines,
            tags,
        });
    }

    // If the entire paragraph is styled with a monospace font and contains multiple lines,
    // emit a fenced code block.
    if is_monospace_style(rt.paragraph_style())
        && rt.text().contains(['\n', '\r'])
        && tags.is_empty()
    {
        let code = visible_plain_text(rt);
        if !code.trim().is_empty() {
            return Some(Block::CodeBlock {
                language: None,
                code,
            });
        }
    }

    Some(Block::Paragraph { inlines, tags })
}

fn detect_heading_level(style: &ParagraphStyling) -> Option<u8> {
    parse_heading_level_id(style.style_id()?)
}

fn parse_heading_level_id(style_id: &str) -> Option<u8> {
    let id = style_id.trim().to_ascii_lowercase();
    match id.as_str() {
        "h1" | "heading 1" | "heading1" => Some(1),
        "h2" | "heading 2" | "heading2" => Some(2),
        "h3" | "heading 3" | "heading3" => Some(3),
        "h4" | "heading 4" | "heading4" => Some(4),
        "h5" | "heading 5" | "heading5" => Some(5),
        "h6" | "heading 6" | "heading6" => Some(6),
        _ => None,
    }
}

fn convert_rich_text_inlines(rt: &RichText) -> Vec<Inline> {
    let utf16: Vec<u16> = rt.text().encode_utf16().collect();
    if utf16.is_empty() {
        return Vec::new();
    }

    let base_style = rt.paragraph_style();
    let formatting = rt.text_run_formatting();
    let indices = rt.text_run_indices();
    let hyperlinks: Vec<TextHyperlink> = rt.hyperlinks();

    // Build run spans: (start_u16, end_u16, &ParagraphStyling)
    let mut runs: Vec<(usize, usize, &ParagraphStyling)> = Vec::new();
    if formatting.is_empty() {
        runs.push((0, utf16.len(), base_style));
    } else {
        let mut bounds: Vec<usize> = std::iter::once(0)
            .chain(indices.iter().map(|&i| (i as usize).min(utf16.len())))
            .collect();
        if bounds.len() == formatting.len() {
            bounds.push(utf16.len());
        }
        for (i, style) in formatting.iter().enumerate() {
            let start = bounds.get(i).copied().unwrap_or(utf16.len());
            let end = bounds.get(i + 1).copied().unwrap_or(utf16.len());
            if start < end {
                runs.push((start, end, style));
            }
        }
    }

    // Collect all boundary points (run boundaries + hyperlink boundaries)
    let mut cuts: Vec<usize> = Vec::with_capacity(runs.len() * 2 + hyperlinks.len() * 2);
    for &(s, e, _) in &runs {
        cuts.push(s);
        cuts.push(e);
    }
    for hl in &hyperlinks {
        cuts.push((hl.start() as usize).min(utf16.len()));
        cuts.push((hl.end() as usize).min(utf16.len()));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut result: Vec<Inline> = Vec::new();
    let mut active_link: Option<(usize, Vec<Inline>)> = None;

    for pair in cuts.windows(2) {
        let (seg_start, seg_end) = (pair[0], pair[1]);
        if seg_start >= seg_end {
            continue;
        }

        let style = runs
            .iter()
            .find(|&&(s, e, _)| seg_start >= s && seg_end <= e)
            .map(|&(_, _, st)| st)
            .unwrap_or(base_style);

        if style.hidden() {
            continue;
        }

        let raw_text = String::from_utf16_lossy(&utf16[seg_start..seg_end]);
        let clean_text = strip_onenote_control_chars(&raw_text);
        if clean_text.is_empty() {
            continue;
        }

        let hl_index = hyperlinks
            .iter()
            .position(|hl| seg_start >= hl.start() as usize && seg_end <= hl.end() as usize);

        // Flush previous hyperlink if we moved out of it or into a different hyperlink
        if let Some((prev_idx, _)) = &active_link
            && Some(*prev_idx) != hl_index
            && let Some((idx, link_inlines)) = active_link.take()
        {
            result.push(Inline::Link {
                text: coalesce_inlines(link_inlines),
                target: LinkTarget::parse(hyperlinks[idx].target()),
            });
        }

        let styled_inlines = build_styled_inlines(&clean_text, style, base_style);

        if let Some(idx) = hl_index {
            match &mut active_link {
                Some((_, buf)) => buf.extend(styled_inlines),
                None => active_link = Some((idx, styled_inlines)),
            }
        } else {
            result.extend(styled_inlines);
        }
    }

    if let Some((idx, link_inlines)) = active_link.take() {
        result.push(Inline::Link {
            text: coalesce_inlines(link_inlines),
            target: LinkTarget::parse(hyperlinks[idx].target()),
        });
    }

    coalesce_inlines(result)
}

/// Merge adjacent inline runs with identical formatting wrappers so split OneNote text runs
/// emit clean `**foobar**` instead of broken `**foo****bar**` Markdown.
fn coalesce_inlines(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut merged: Vec<Inline> = Vec::with_capacity(inlines.len());

    for item in inlines {
        match (merged.last_mut(), item) {
            (Some(Inline::Text(prev)), Inline::Text(next)) => prev.push_str(&next),
            (Some(Inline::Code(prev)), Inline::Code(next)) => prev.push_str(&next),
            (Some(Inline::Bold(prev)), Inline::Bold(next)) => {
                let mut combined = std::mem::take(prev);
                combined.extend(next);
                *prev = coalesce_inlines(combined);
            }
            (Some(Inline::Italic(prev)), Inline::Italic(next)) => {
                let mut combined = std::mem::take(prev);
                combined.extend(next);
                *prev = coalesce_inlines(combined);
            }
            (Some(Inline::Strikethrough(prev)), Inline::Strikethrough(next)) => {
                let mut combined = std::mem::take(prev);
                combined.extend(next);
                *prev = coalesce_inlines(combined);
            }
            (_, other) => merged.push(other),
        }
    }

    merged
}

fn build_styled_inlines(
    text: &str,
    run_style: &ParagraphStyling,
    base_style: &ParagraphStyling,
) -> Vec<Inline> {
    let bold = run_style.bold() || base_style.bold();
    let italic = run_style.italic() || base_style.italic();
    let strike = run_style.strikethrough() || base_style.strikethrough();
    let is_code = is_monospace_style(run_style)
        || (run_style.font().is_none() && is_monospace_style(base_style));

    let mut out = Vec::new();
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let parts: Vec<&str> = normalized.split('\n').collect();

    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(Inline::LineBreak);
        }
        if part.is_empty() {
            continue;
        }
        let mut node = if is_code {
            Inline::Code((*part).to_string())
        } else {
            Inline::Text((*part).to_string())
        };
        if !is_code {
            if strike {
                node = Inline::Strikethrough(vec![node]);
            }
            if italic {
                node = Inline::Italic(vec![node]);
            }
            if bold {
                node = Inline::Bold(vec![node]);
            }
        }
        out.push(node);
    }

    out
}

fn visible_plain_text(rt: &RichText) -> String {
    let utf16: Vec<u16> = rt.text().encode_utf16().collect();
    let formatting = rt.text_run_formatting();
    if formatting.is_empty() {
        return normalize_newlines(&strip_onenote_control_chars(rt.text()));
    }
    let mut bounds: Vec<usize> = std::iter::once(0)
        .chain(rt.text_run_indices().iter().map(|&i| (i as usize).min(utf16.len())))
        .collect();
    if bounds.len() == formatting.len() {
        bounds.push(utf16.len());
    }
    let mut out = Vec::new();
    for (i, style) in formatting.iter().enumerate() {
        if style.hidden() {
            continue;
        }
        let start = bounds.get(i).copied().unwrap_or(utf16.len());
        let end = bounds.get(i + 1).copied().unwrap_or(utf16.len());
        if start < end {
            out.extend_from_slice(&utf16[start..end]);
        }
    }
    normalize_newlines(&strip_onenote_control_chars(&String::from_utf16_lossy(&out)))
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn strip_onenote_control_chars(s: &str) -> String {
    s.chars()
        .filter(|&c| c != '\u{fddf}' && c != '\u{fffc}' && c != '\u{000b}')
        .collect()
}

fn is_monospace_style(style: &ParagraphStyling) -> bool {
    style.font().is_some_and(is_monospace_font_name)
}

fn is_monospace_font_name(font: &str) -> bool {
    let lower = font.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "consolas"
            | "courier"
            | "courier new"
            | "cascadia code"
            | "cascadia mono"
            | "menlo"
            | "monaco"
            | "fira code"
            | "fira mono"
            | "jetbrains mono"
            | "source code pro"
            | "dejavu sans mono"
            | "liberation mono"
            | "ubuntu mono"
            | "sf mono"
            | "lucida console"
            | "roboto mono"
            | "inconsolata"
            | "monospace"
    )
}

fn convert_note_tags(raw_tags: &[RawNoteTag]) -> Vec<NoteTag> {
    raw_tags
        .iter()
        .map(|raw| {
            let is_checkbox = is_checkbox_tag(raw);
            let completed = raw.item_status().completed();
            let shape = raw.definition().map(|d| d.shape());
            let label = raw
                .definition()
                .map(|d| d.label().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if is_checkbox {
                        "To Do".to_string()
                    } else {
                        "Tag".to_string()
                    }
                });

            let kind = classify_tag(&label, shape, is_checkbox);

            NoteTag {
                label,
                is_checkbox,
                completed,
                kind,
            }
        })
        .collect()
}

fn is_checkbox_tag(raw: &RawNoteTag) -> bool {
    if raw.item_status().task_tag() {
        return true;
    }
    matches!(
        raw.definition().map(|d| d.shape()),
        Some(
            NoteTagShape::GreenCheckBox
                | NoteTagShape::YellowCheckBox
                | NoteTagShape::BlueCheckBox
                | NoteTagShape::GreenStarCheckBox
                | NoteTagShape::YellowStarCheckBox
                | NoteTagShape::BlueStarCheckBox
                | NoteTagShape::GreenExclamationCheckBox
                | NoteTagShape::YellowExclamationCheckBox
                | NoteTagShape::BlueExclamationCheckBox
                | NoteTagShape::GreenRightArrowCheckBox
                | NoteTagShape::YellowRightArrowCheckBox
                | NoteTagShape::BlueRightArrowCheckBox
                | NoteTagShape::BlueCheckBox1
                | NoteTagShape::BlueCheckBox2
                | NoteTagShape::BlueCheckBox3
                | NoteTagShape::GreenCheckBox1
                | NoteTagShape::GreenCheckBox2
                | NoteTagShape::GreenCheckBox3
                | NoteTagShape::YellowCheckBox1
                | NoteTagShape::YellowCheckBox2
                | NoteTagShape::YellowCheckBox3
                | NoteTagShape::BluePersonCheckBox
                | NoteTagShape::YellowPersonCheckBox
                | NoteTagShape::GreenPersonCheckBox
                | NoteTagShape::BlueFlagCheckBox
                | NoteTagShape::RedFlagCheckBox
                | NoteTagShape::GreenFlagCheckBox
        )
    )
}

fn classify_tag(label: &str, shape: Option<NoteTagShape>, is_checkbox: bool) -> NoteTagKind {
    if is_checkbox {
        return NoteTagKind::ToDo;
    }
    let from_label = NoteTagKind::from_label(label);
    if from_label != NoteTagKind::Custom {
        return from_label;
    }
    match shape {
        Some(NoteTagShape::YellowStar | NoteTagShape::BlueStar | NoteTagShape::GreenStar) => {
            NoteTagKind::Important
        }
        Some(NoteTagShape::QuestionMark) => NoteTagKind::Question,
        Some(NoteTagShape::LightBulb) => NoteTagKind::Idea,
        Some(NoteTagShape::HighPriority) => NoteTagKind::Critical,
        Some(NoteTagShape::Pushpin) => NoteTagKind::RememberForLater,
        Some(NoteTagShape::ContactInformation) => NoteTagKind::Contact,
        _ => NoteTagKind::Custom,
    }
}

fn convert_utc_time(dt: time::UtcDateTime) -> Option<DateTime<Utc>> {
    let ts = dt.unix_timestamp();
    if ts <= 0 {
        return None;
    }
    DateTime::from_timestamp(ts, dt.nanosecond())
}

fn collect_parser_warnings(
    scope: &str,
    report: &ParserReport,
    out: &mut Vec<ExportWarning>,
) {
    for warning in report.warnings() {
        let warn_scope = match warning.page() {
            Some((_, page_title)) => format!("{scope}/{page_title}"),
            None => scope.to_string(),
        };
        out.push(ExportWarning::new(warn_scope, warning.message()));
    }
}

fn sniff_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"%PDF-") {
        Some("pdf")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_adjacent_styled_inlines_recursively() {
        let input = vec![
            Inline::Bold(vec![Inline::Italic(vec![Inline::Text("foo".into())])]),
            Inline::Bold(vec![Inline::Italic(vec![Inline::Text("bar".into())])]),
            Inline::Text(" ".into()),
            Inline::Text("baz".into()),
            Inline::Code("a".into()),
            Inline::Code("b".into()),
        ];

        assert_eq!(
            coalesce_inlines(input),
            vec![
                Inline::Bold(vec![Inline::Italic(vec![Inline::Text("foobar".into())])]),
                Inline::Text(" baz".into()),
                Inline::Code("ab".into()),
            ]
        );
    }

    #[test]
    fn detects_ordered_vs_bulleted_list_formats() {
        assert!(!is_ordered_list_format(&[]));
        assert!(!is_ordered_list_format(&['\u{2022}']));
        assert!(!is_ordered_list_format(&['-']));
        assert!(is_ordered_list_format(&['\u{0000}', '.']));
        assert!(is_ordered_list_format(&['\u{0001}', ')']));
    }

    #[test]
    fn parses_heading_styles_and_monospace_fonts() {
        assert_eq!(parse_heading_level_id("h1"), Some(1));
        assert_eq!(parse_heading_level_id("Heading 3"), Some(3));
        assert_eq!(parse_heading_level_id("Heading6"), Some(6));
        assert_eq!(parse_heading_level_id("Normal"), None);

        assert!(is_monospace_font_name("Consolas"));
        assert!(is_monospace_font_name("JetBrains Mono"));
        assert!(!is_monospace_font_name("Calibri"));
    }

    #[test]
    fn classifies_tags_by_checkbox_label_and_shape() {
        assert_eq!(
            classify_tag("Custom", Some(NoteTagShape::GreenCheckBox), true),
            NoteTagKind::ToDo
        );
        assert_eq!(
            classify_tag("Important", None, false),
            NoteTagKind::Important
        );
        assert_eq!(
            classify_tag("My Star", Some(NoteTagShape::YellowStar), false),
            NoteTagKind::Important
        );
        assert_eq!(
            classify_tag("Brainstorm", Some(NoteTagShape::LightBulb), false),
            NoteTagKind::Idea
        );
        assert_eq!(classify_tag("Misc", None, false), NoteTagKind::Custom);
    }

    #[test]
    fn strips_onenote_control_chars_and_sniffs_magic_bytes() {
        assert_eq!(
            strip_onenote_control_chars("a\u{fddf}b\u{fffc}c\u{000b}d"),
            "abcd"
        );
        assert_eq!(sniff_extension(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(sniff_extension(b"\xFF\xD8\xFF\xE0"), Some("jpg"));
        assert_eq!(sniff_extension(b"GIF89a..."), Some("gif"));
        assert_eq!(sniff_extension(b"RIFF1234WEBP"), Some("webp"));
        assert_eq!(sniff_extension(b"%PDF-1.7"), Some("pdf"));
        assert_eq!(sniff_extension(b"plain text"), None);
    }

    #[test]
    fn ignores_epoch_zero_timestamps() {
        assert_eq!(convert_utc_time(time::UtcDateTime::UNIX_EPOCH), None);
        let valid = time::UtcDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        assert_eq!(
            convert_utc_time(valid).map(|dt| dt.timestamp()),
            Some(1_700_000_000)
        );
    }
}
