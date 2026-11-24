use std::fs;
use std::path::Path;

use chrono::{TimeZone, Utc};
use tempfile::tempdir;

use one::config::{ExportOptions, LinkStyle, SubpageStrategy, TagStyle};
use one::ir::{
    Asset, Block, Inline, LinkTarget, ListItem, NoteTag, NoteTagKind, Notebook, Page, Section,
    SectionEntry, SectionGroup, SourceBundle,
};
use one::pipeline::export_bundle;
use one::sources::OneNoteSource;
use one::sources::local::LocalFileSource;

fn sample_bundle() -> SourceBundle {
    let created = Utc.with_ymd_and_hms(2025, 4, 1, 10, 0, 0).unwrap();
    let updated = Utc.with_ymd_and_hms(2026, 2, 15, 14, 30, 0).unwrap();

    let shared_png = b"\x89PNG\r\n\x1a\nSHARED-IMAGE-BYTES".to_vec();
    let other_png = b"\x89PNG\r\n\x1a\nDIFFERENT-IMAGE-BYTES".to_vec();

    SourceBundle {
        notebooks: vec![Notebook {
            id: Some("nb-1".into()),
            name: "Knowledge Base".into(),
            entries: vec![
                SectionEntry::SectionGroup(SectionGroup {
                    id: Some("sg-1".into()),
                    name: "Engineering".into(),
                    entries: vec![SectionEntry::Section(Section {
                        id: Some("aaaa0000-0000-0000-0000-000000000001".into()),
                        name: "Architecture".into(),
                        pages: vec![
                            Page {
                                id: Some("11111111-1111-1111-1111-111111111111".into()),
                                title: "System Overview".into(),
                                level: 0,
                                created_at: Some(created),
                                updated_at: Some(updated),
                                author: Some("Anthony".into()),
                                web_url: None,
                                client_url: None,
                                blocks: vec![
                                    Block::Paragraph {
                                        inlines: vec![
                                            Inline::Text("Check ".into()),
                                            Inline::Link {
                                                text: vec![Inline::Text("Storage Layer".into())],
                                                target: LinkTarget::parse(
                                                    "onenote:#Storage%20Layer&page-id={22222222-2222-2222-2222-222222222222}&end",
                                                ),
                                            },
                                            Inline::Text(" and ".into()),
                                            Inline::Link {
                                                text: vec![Inline::Text("Runbook".into())],
                                                target: LinkTarget::parse(
                                                    "onenote:Ops.one#Runbook&page-id={33333333-3333-3333-3333-333333333333}&end",
                                                ),
                                            },
                                            Inline::Text(" and ".into()),
                                            Inline::Link {
                                                text: vec![Inline::Text("Missing Note".into())],
                                                target: LinkTarget::parse(
                                                    "onenote:#Missing%20Note&page-id={99999999-9999-9999-9999-999999999999}&end",
                                                ),
                                            },
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
                                                    inlines: vec![Inline::Text("Ship v1".into())],
                                                    tags: vec![],
                                                }],
                                                children: vec![Block::List {
                                                    ordered: false,
                                                    start: 1,
                                                    items: vec![ListItem {
                                                        checkbox: Some(true),
                                                        tags: vec![],
                                                        content: vec![Block::Paragraph {
                                                            inlines: vec![Inline::Text(
                                                                "Build parser".into(),
                                                            )],
                                                            tags: vec![],
                                                        }],
                                                        children: vec![],
                                                    }],
                                                }],
                                            },
                                        ],
                                    },
                                    Block::Image {
                                        asset_id: "img-1".into(),
                                        alt: "Diagram".into(),
                                        hyperlink: None,
                                        tags: vec![],
                                    },
                                ],
                                assets: vec![Asset {
                                    id: "img-1".into(),
                                    filename: "diagram.png".into(),
                                    data: shared_png.clone(),
                                }],
                            },
                            Page {
                                id: Some("22222222-2222-2222-2222-222222222222".into()),
                                title: "Storage Layer".into(),
                                level: 1,
                                created_at: Some(created),
                                updated_at: Some(updated),
                                author: None,
                                web_url: None,
                                client_url: None,
                                blocks: vec![
                                    Block::Image {
                                        asset_id: "img-dup".into(),
                                        alt: "Shared".into(),
                                        hyperlink: None,
                                        tags: vec![],
                                    },
                                    Block::Image {
                                        asset_id: "img-collide".into(),
                                        alt: "Colliding Name".into(),
                                        hyperlink: None,
                                        tags: vec![],
                                    },
                                ],
                                assets: vec![
                                    Asset {
                                        id: "img-dup".into(),
                                        filename: "diagram.png".into(),
                                        data: shared_png,
                                    },
                                    Asset {
                                        id: "img-collide".into(),
                                        filename: "diagram.png".into(),
                                        data: other_png,
                                    },
                                ],
                            },
                        ],
                    })],
                }),
                SectionEntry::Section(Section {
                    id: Some("bbbb0000-0000-0000-0000-000000000002".into()),
                    name: "Ops".into(),
                    pages: vec![Page {
                        id: Some("33333333-3333-3333-3333-333333333333".into()),
                        title: "Runbook".into(),
                        level: 0,
                        created_at: None,
                        updated_at: None,
                        author: None,
                        web_url: None,
                        client_url: None,
                        blocks: vec![Block::Paragraph {
                            inlines: vec![Inline::Text("Restart service".into())],
                            tags: vec![NoteTag {
                                label: "Question".into(),
                                is_checkbox: false,
                                completed: false,
                                kind: NoteTagKind::Question,
                            }],
                        }],
                        assets: vec![],
                    }],
                }),
            ],
        }],
        warnings: vec![],
    }
}

#[test]
fn exports_hierarchy_links_deduped_assets_and_report() {
    let tmp = tempdir().unwrap();
    let out_dir = tmp.path().join("out");
    let report_path = tmp.path().join("report.json");

    let opts = ExportOptions {
        output_dir: out_dir.clone(),
        subpages: SubpageStrategy::Nested,
        links: LinkStyle::Relative,
        tag_style: TagStyle::Inline,
        preserve_mtime: true,
        dry_run: false,
        include_frontmatter: true,
        report_path: Some(report_path.clone()),
        notebook_filter: None,
        section_filter: None,
    };

    let bundle = sample_bundle();
    let report = export_bundle("test", &bundle, &opts).unwrap();

    assert_eq!(report.notebooks_exported, 1);
    assert_eq!(report.sections_exported, 2);
    assert_eq!(report.pages_exported, 3);
    // 3 image references across 2 pages, but 2 share identical bytes -> only 2 unique assets written
    assert_eq!(report.assets_written, 2);
    assert_eq!(report.links_resolved, 2);
    assert_eq!(report.unresolved_links.len(), 1);
    assert_eq!(report.unresolved_links[0].link_text, "Missing Note");

    let overview_path =
        out_dir.join("Knowledge Base/Engineering/Architecture/System Overview/System Overview.md");
    let storage_path =
        out_dir.join("Knowledge Base/Engineering/Architecture/System Overview/Storage Layer.md");
    let runbook_path = out_dir.join("Knowledge Base/Ops/Runbook.md");

    assert!(overview_path.exists());
    assert!(storage_path.exists());
    assert!(runbook_path.exists());
    assert!(report_path.exists());

    let overview = fs::read_to_string(&overview_path).unwrap();
    assert!(overview.contains("[Storage Layer](Storage%20Layer.md)"));
    assert!(overview.contains("[Runbook](../../../Ops/Runbook.md)"));
    assert!(overview.contains("![Diagram](../_assets/diagram.png)"));
    assert!(overview.contains("- [ ] Ship v1\n  - [x] Build parser"));
    assert!(overview.contains("section_group: \"Engineering\""));

    let storage = fs::read_to_string(&storage_path).unwrap();
    // Shared image reuses diagram.png; different bytes with same filename get hash suffix
    assert!(storage.contains("![Shared](../_assets/diagram.png)"));
    assert!(storage.contains("![Colliding Name](../_assets/diagram-"));

    // Verify mtime preservation
    let meta = fs::metadata(&overview_path).unwrap();
    let modified = meta.modified().unwrap();
    let expected = Utc.with_ymd_and_hms(2026, 2, 15, 14, 30, 0).unwrap();
    let actual_secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert_eq!(actual_secs, expected.timestamp());
}

#[test]
fn supports_wikilinks_emoji_tags_and_dry_run() {
    let tmp = tempdir().unwrap();
    let out_dir = tmp.path().join("dry");

    let opts = ExportOptions {
        output_dir: out_dir.clone(),
        subpages: SubpageStrategy::Flat,
        links: LinkStyle::Wikilink,
        tag_style: TagStyle::Emoji,
        dry_run: true,
        ..Default::default()
    };

    let bundle = sample_bundle();
    let report = export_bundle("test", &bundle, &opts).unwrap();

    assert_eq!(report.pages_exported, 3);
    assert_eq!(report.links_resolved, 2);
    assert!(!out_dir.exists(), "dry-run must not create output files");
}

#[test]
fn supports_section_filtering() {
    let tmp = tempdir().unwrap();
    let out_dir = tmp.path().join("filtered");

    let opts = ExportOptions {
        output_dir: out_dir.clone(),
        section_filter: Some("ops".into()),
        ..Default::default()
    };

    let bundle = sample_bundle();
    let report = export_bundle("test", &bundle, &opts).unwrap();

    assert_eq!(report.sections_exported, 1);
    assert_eq!(report.pages_exported, 1);
    assert!(out_dir.join("Knowledge Base/Ops/Runbook.md").exists());
    assert!(!out_dir.join("Knowledge Base/Engineering").exists());
}

#[test]
fn local_source_reports_clear_errors_for_invalid_inputs() {
    let missing = LocalFileSource::new(Path::new("/nonexistent/notebook.onepkg"));
    assert!(missing.load().is_err());

    let tmp = tempdir().unwrap();
    let empty_dir = LocalFileSource::new(tmp.path());
    let err = empty_dir.load().unwrap_err().to_string();
    assert!(err.contains("No .onetoc2 or .one files found"));

    let bad_file = tmp.path().join("notes.txt");
    fs::write(&bad_file, "hello").unwrap();
    let bad_ext = LocalFileSource::new(&bad_file);
    let err = bad_ext.load().unwrap_err().to_string();
    assert!(err.contains("Unsupported file extension"));
}
