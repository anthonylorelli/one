# one

[![CI](https://github.com/anthonylorelli/one/actions/workflows/ci.yml/badge.svg)](https://github.com/anthonylorelli/one/actions/workflows/ci.yml)

A fast, single-binary Rust CLI tool to export Microsoft OneNote notebooks to clean **GitHub Flavored Markdown (GFM)** with YAML frontmatter, extracted media, preserved hierarchy, checkboxes/tags, and resolved internal cross-page links.

Designed for migrating notes out of OneNote into Git repositories, VS Code, Obsidian, Notion, Logseq, or any Markdown-based workflow — completely offline for local files, with a pluggable architecture built to add Microsoft Graph cloud support next.

---

## Features

- **Offline local OneNote export** — Reads `.one` section files, `.onetoc2` table-of-contents files, `.onepkg` package archives, and notebook folders (including nested section-group subdirectories) via [`onenote_parser`](https://crates.io/crates/onenote_parser). Supports both OneNote Desktop and OneDrive-downloaded files.
- **Full notebook hierarchy & subpage nesting**:
  - `--subpages nested` *(default)*: Parent pages with subpages become folder notes (`Parent/Parent.md`, `Parent/Child.md`).
  - `--subpages flat`: All pages stay in the section directory with hierarchical numeric prefixes (`01 Parent.md`, `01.1 Child.md`).
- **Portable GitHub Flavored Markdown + YAML frontmatter**:
  - Preserves `title`, `created`, `updated`, `author`, `notebook`, `section_group`, `section`, `onenote_id`, and aggregated `tags`.
  - Converts OneNote To-Do tags to GFM task lists (`- [ ]` / `- [x]`).
  - Maps non-checkbox OneNote tags (`Important`, `Question`, `Idea`, `Critical`, custom tags) using `--tag-style inline | emoji | frontmatter`.
  - Preserves headings, bold/italic/strikethrough, inline & fenced monospace code, nested lists, pipe tables, images, embedded file attachments, and Windows handwriting OCR text.
- **Two-pass internal link resolution**:
  - Indexes every exported page/section GUID, title, and URL before rendering.
  - Rewrites internal `onenote:` and OneNote web (`wd=target(...)`) links into portable relative Markdown links (`--links relative`) or Obsidian-style wikilinks (`--links wikilink`).
- **Deduplicated asset extraction & migration reports**:
  - Writes images and file attachments to `<section>/_assets/` with SHA-256 content deduplication and collision-safe filenames.
  - Emits an optional JSON report (`--report`) summarizing counts, unresolved internal links, and non-fatal parser warnings.

---

## Installation

### Prebuilt binaries

Download prebuilt **x64** and **arm64** archives for Linux, macOS, and Windows (plus `SHA256SUMS.txt`) from the [GitHub Releases](https://github.com/anthonylorelli/one/releases) page, or grab the latest build artifacts from the [CI workflow](https://github.com/anthonylorelli/one/actions/workflows/ci.yml).

To cut a new release, push a version tag (e.g., `git tag v0.1.0 && git push origin v0.1.0`).

### Build from source

Requires Rust **1.85+** (Edition 2024).

```bash
git clone https://github.com/anthonylorelli/one.git
cd one
cargo install --path .
```

Or build a release binary without installing:

```bash
cargo build --release
./target/release/one --help
```

---

## Getting Your OneNote Files

`one export local` accepts any of the following local inputs:

| Source | How to obtain | What to pass to `one` |
| :--- | :--- | :--- |
| **OneNote Desktop (Windows)** | **File → Export → Notebook → OneNote Package (`*.onepkg`)** (or export a single section as `*.one`) | `./MyNotebook.onepkg` or `./Section.one` |
| **OneDrive (Personal account)** | Open [OneNote for the web](https://www.onenote.com), right-click a notebook → **Export notebook**, and unzip the downloaded archive | `./MyNotebook/` or `./MyNotebook/Open Notebook.onetoc2` |
| **Local notebook / backup folder** | Copy an existing notebook directory containing `.onetoc2` and `.one` files (or a plain folder of `.one` files) | `./MyNotebook/` |

---

## Quick Start

### 1. Inspect a notebook before exporting

```bash
# List notebooks, section groups, and sections
one list local ./MyNotebook.onepkg

# Include the full page & subpage tree
one list local ./MyNotebook.onepkg --pages
```

### 2. Export to portable GFM Markdown

```bash
one export local ./MyNotebook.onepkg -o ./exported
```

### 3. Common presets

```bash
# Obsidian-friendly export (wikilinks + folder notes + preserved timestamps)
one export local ./MyNotebook -o ./vault \
  --subpages nested \
  --links wikilink \
  --tag-style inline \
  --preserve-mtime

# Flat numbered pages with emoji tags and a JSON migration report
one export local ./MyNotebook.onepkg -o ./exported \
  --subpages flat \
  --tag-style emoji \
  --report ./export-report.json

# Filter to a single section and dry-run without writing files
one export local ./MyNotebook --section "Architecture" --dry-run
```

---

## CLI Reference

### Commands

| Command | Description |
| :--- | :--- |
| `one export local <INPUT> [OPTIONS]` | Export a `.one`, `.onetoc2`, `.onepkg` file, or notebook directory to Markdown. |
| `one list local <INPUT> [--pages]` | Print the notebook/section/page hierarchy without writing files. |
| `one export graph [OPTIONS]` | *(Planned)* Export cloud notebooks via Microsoft Graph API. |
| `one list graph` | *(Planned)* List cloud notebooks via Microsoft Graph API. |

### Export Options

| Flag | Default | Description |
| :--- | :--- | :--- |
| `-o, --output <DIR>` | `./exported` | Destination directory for exported Markdown files and `_assets/`. |
| `--subpages <nested\|flat>` | `nested` | `nested` creates folder notes (`Parent/Parent.md`); `flat` prefixes filenames (`01`, `01.1`). |
| `--links <relative\|wikilink>` | `relative` | Format for resolved internal OneNote links. |
| `--tag-style <inline\|emoji\|frontmatter>` | `inline` | How non-checkbox OneNote tags appear in body text (`#important`, `⭐`, or frontmatter-only). |
| `--preserve-mtime` | `false` | Set each `.md` file's modification timestamp to the page's last-modified date. |
| `--no-frontmatter` | `false` | Omit YAML frontmatter blocks from exported pages. |
| `--dry-run` | `false` | Run discovery, link resolution, and rendering without writing Markdown or asset files. |
| `--report <PATH>` | *(none)* | Write a JSON migration report (counts, unresolved links, parser warnings). |
| `-n, --notebook <SUBSTR>` | *(all)* | Export only notebooks whose name contains `<SUBSTR>` (case-insensitive). |
| `-s, --section <SUBSTR>` | *(all)* | Export only sections whose name contains `<SUBSTR>` (case-insensitive). |
| `-v, --verbose` | `warn` | Increase log verbosity (`-v` for `info`, `-vv` for `debug`; also honors `RUST_LOG`). |

---

## Output Format

### Directory layout (`--subpages nested`)

```text
exported/
└── Knowledge Base/
    ├── Engineering/
    │   └── Architecture/
    │       ├── _assets/
    │       │   └── diagram.png
    │       └── System Overview/
    │           ├── System Overview.md
    │           └── Storage Layer.md
    └── Ops/
        └── Runbook.md
```

### Example exported Markdown

```markdown
---
title: "System Overview"
created: 2025-04-01T10:00:00+00:00
updated: 2026-02-15T14:30:00+00:00
author: "Anthony"
notebook: "Knowledge Base"
section_group: "Engineering"
section: "Architecture"
onenote_id: "11111111-1111-1111-1111-111111111111"
tags:
  - "important"
---

# System Overview

Check [Storage Layer](Storage%20Layer.md) and [Runbook](../../../Ops/Runbook.md) #important

- [ ] Ship v1
  - [x] Build parser

![Diagram](../_assets/diagram.png)
```

### Migration report (`--report report.json`)

```json
{
  "source": "local-file",
  "output_dir": "./exported",
  "dry_run": false,
  "notebooks_exported": 1,
  "sections_exported": 2,
  "pages_exported": 3,
  "assets_written": 1,
  "links_resolved": 2,
  "unresolved_links": [
    {
      "source_page": "Knowledge Base/Engineering/Architecture/System Overview/System Overview.md",
      "link_text": "Missing Note",
      "target_raw": "onenote:#Missing%20Note&page-id={99999999-9999-9999-9999-999999999999}&end"
    }
  ],
  "warnings": []
}
```

---

## Architecture

`one` separates **data acquisition** from **Markdown generation** using a shared Canonical Intermediate Representation (IR) and a two-pass pipeline:

```text
OneNoteSource (local | graph)
        │
        ▼
  Canonical IR (src/ir/)
        │
        ├── Pass 1: PathPlanner + LinkRegistry (src/ir/link_registry.rs)
        │     • Cross-platform path sanitization & deduplication
        │     • Nested/flat subpage layout & `_assets` reservation
        │     • Global GUID / URL / (section, title) link index
        │
        └── Pass 2: Pipeline + GFM Renderer (src/pipeline.rs, src/render/)
              • SHA-256 deduplicated asset extraction
              • Internal link rewriting & unresolved-link tracking
              • YAML frontmatter + GFM emission + ExportReport
```

---

## Known Limitations & Roadmap

- **Handwritten ink**: Freeform ink strokes are not rendered to SVG/PNG. If OneNote for Windows stored handwriting OCR (`ink_recognition`), the recognized text is exported as a blockquote; otherwise a non-fatal warning is logged.
- **Freeform canvas positioning**: OneNote's 2D freeform outline containers are linearized into sequential Markdown blocks.
- **Password-protected sections**: Encrypted `.one` sections cannot be decrypted offline and will emit a non-fatal warning.
- **Microsoft Graph API (planned)**: Delegated OAuth 2.0 (`Notes.Read`) cloud export for OneDrive and Microsoft 365 notebooks is stubbed in `src/sources/graph.rs` and planned for the next phase.

---

## Development

```bash
# Run unit and integration tests
cargo test

# Lint with Clippy (warnings as errors)
cargo clippy --all-targets -- -D warnings

# Format code
cargo fmt
```

## License

This project is licensed under the **MIT License**. Note that it depends on [`onenote_parser`](https://crates.io/crates/onenote_parser) (licensed under MPL-2.0).