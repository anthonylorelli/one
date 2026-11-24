# one

A fast, single-binary Rust CLI tool to export Microsoft OneNote notebooks to clean **GitHub Flavored Markdown (GFM)** with YAML frontmatter, extracted media, preserved hierarchy, checkboxes/tags, and resolved internal cross-page links.

## Features

- **Local OneNote export** (`.one` sections, `.onetoc2` notebooks, `.onepkg` package archives, and notebook folders) powered by [`onenote_parser`](https://crates.io/crates/onenote_parser) — supports both OneNote Desktop and OneDrive-downloaded files offline.
- **Pluggable source architecture** (`OneNoteSource` -> Canonical IR -> two-pass Markdown renderer), ready to layer Microsoft Graph API cloud export on top of the same pipeline.
- **Full hierarchy & subpage nesting**:
  - `--subpages nested` *(default)*: parent pages with subpages become folder notes (`Parent/Parent.md` + `Parent/Child.md`).
  - `--subpages flat`: hierarchical numeric prefixes (`01 Parent.md`, `01.1 Child.md`).
- **GFM Markdown + YAML frontmatter**:
  - Preserves `title`, `created`, `updated`, `author`, `notebook`, `section_group`, `section`, `onenote_id`, and aggregated `tags`.
  - Converts OneNote To-Do tags to GFM task lists (`- [ ]` / `- [x]`) and supports `--tag-style inline | emoji | frontmatter`.
  - Converts tables, bold/italic/strikethrough, inline & fenced monospace code, images, file attachments (SHA-256 deduplicated in `_assets/`), and recognized handwriting text.
- **Two-pass internal link resolution**:
  - Rewrites `onenote:` cross-page links into portable relative Markdown links (`--links relative`) or Obsidian-style wikilinks (`--links wikilink`).
  - Logs any unresolved links and non-fatal parser warnings to an optional JSON report (`--report report.json`).

## Usage

```bash
# Build
cargo build --release

# Inspect a local .one, .onetoc2, .onepkg file, or notebook directory
cargo run -- list local ./path/to/Notebook.onepkg --pages

# Export to portable GFM Markdown (default)
cargo run -- export local ./path/to/Notebook.onepkg -o ./exported

# Export with flat subpages, wikilinks, emoji tags, preserved mtime, and a JSON report
cargo run -- export local ./path/to/Notebook \
  -o ./exported \
  --subpages flat \
  --links wikilink \
  --tag-style emoji \
  --preserve-mtime \
  --report ./export-report.json

# Dry run (plan & report without writing Markdown or assets)
cargo run -- export local ./path/to/Section.one --dry-run
```

## Architecture

1. **Sources (`src/sources/`)**: Implementations of `OneNoteSource` normalize input formats into the shared Canonical IR (`src/ir/`).
   - `local`: Reads `.one`, `.onetoc2`, `.onepkg`, and notebook directories.
   - `graph`: Stub ready for delegated Microsoft Graph API cloud export.
2. **Pass 1 (`src/ir/link_registry.rs`)**: Traverses the IR, sanitizes and deduplicates cross-platform file paths, computes subpage nesting, and indexes every page/section GUID and title in `LinkRegistry`.
3. **Pass 2 (`src/pipeline.rs`, `src/render/markdown.rs`)**: Deduplicates and writes binary assets, resolves internal links, emits GFM Markdown + YAML frontmatter, and produces `ExportReport`.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```