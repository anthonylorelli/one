pub mod convert;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use onenote_parser::Parser;
use typed_path::TypedPathBuf;

use crate::ir::{ExportWarning, Notebook, SectionEntry, SourceBundle};
use crate::sources::OneNoteSource;

/// Local binary file source supporting `.onepkg` archives, `.onetoc2` notebooks,
/// single `.one` section files, and notebook directories.
#[derive(Debug, Clone)]
pub struct LocalFileSource {
    input_path: PathBuf,
}

impl LocalFileSource {
    pub fn new(input_path: impl Into<PathBuf>) -> Self {
        Self {
            input_path: input_path.into(),
        }
    }

    pub fn input_path(&self) -> &Path {
        &self.input_path
    }
}

impl OneNoteSource for LocalFileSource {
    fn source_name(&self) -> &'static str {
        "local-file"
    }

    fn load(&self) -> Result<SourceBundle> {
        let path = &self.input_path;
        if !path.exists() {
            bail!("Input path does not exist: {}", path.display());
        }

        let parser = Parser::new();
        let mut warnings = Vec::new();

        if path.is_dir() {
            let notebook = load_directory(&parser, path, &mut warnings)?;
            return Ok(SourceBundle {
                notebooks: vec![notebook],
                warnings,
            });
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();

        let typed = to_typed_path(path)?;

        let notebook = match ext.as_str() {
            "onepkg" => {
                let nb_name = file_stem_or(path, "Notebook");
                let parsed = parser
                    .parse_package(typed.to_path())
                    .with_context(|| format!("Failed to parse .onepkg archive: {}", path.display()))?;
                convert::convert_notebook(&parsed, nb_name, &mut warnings)
            }
            "onetoc2" => {
                let nb_name = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| file_stem_or(path, "Notebook"));
                let parsed = parser
                    .parse_notebook(typed.to_path())
                    .with_context(|| format!("Failed to parse .onetoc2 notebook: {}", path.display()))?;
                convert::convert_notebook(&parsed, nb_name, &mut warnings)
            }
            "one" => {
                let parsed = parser
                    .parse_section(typed.to_path())
                    .with_context(|| format!("Failed to parse .one section: {}", path.display()))?;
                let nb_name = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Notebook".to_string());
                let section = convert::convert_section(&parsed, &mut warnings);
                Notebook {
                    id: None,
                    name: nb_name,
                    entries: vec![SectionEntry::Section(section)],
                }
            }
            other => {
                bail!(
                    "Unsupported file extension '.{other}' for {}. Expected .one, .onetoc2, .onepkg, or a notebook directory.",
                    path.display()
                );
            }
        };

        Ok(SourceBundle {
            notebooks: vec![notebook],
            warnings,
        })
    }
}

fn load_directory(
    parser: &Parser,
    dir: &Path,
    warnings: &mut Vec<ExportWarning>,
) -> Result<Notebook> {
    let mut toc_files: Vec<PathBuf> = Vec::new();
    let mut one_files: Vec<PathBuf> = Vec::new();

    for entry in fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory: {}", dir.display()))?
    {
        let entry = entry?;
        let entry_path = entry.path();
        if !entry_path.is_file() {
            continue;
        }
        let ext = entry_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("onetoc2") => toc_files.push(entry_path),
            Some("one") => one_files.push(entry_path),
            _ => {}
        }
    }

    toc_files.sort();
    one_files.sort();

    let nb_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Notebook".to_string());

    if let Some(toc_path) = toc_files.first() {
        let typed = to_typed_path(toc_path)?;
        let parsed = parser
            .parse_notebook(typed.to_path())
            .with_context(|| format!("Failed to parse notebook TOC: {}", toc_path.display()))?;
        return Ok(convert::convert_notebook(&parsed, nb_name, warnings));
    }

    if one_files.is_empty() {
        bail!(
            "No .onetoc2 or .one files found in directory: {}",
            dir.display()
        );
    }

    let mut entries = Vec::new();
    for one_path in one_files {
        let typed = to_typed_path(&one_path)?;
        match parser.parse_section(typed.to_path()) {
            Ok(parsed) => {
                entries.push(SectionEntry::Section(convert::convert_section(
                    &parsed, warnings,
                )));
            }
            Err(err) => {
                warnings.push(ExportWarning::new(
                    one_path.display().to_string(),
                    format!("Failed to parse section file: {err}"),
                ));
            }
        }
    }

    Ok(Notebook {
        id: None,
        name: nb_name,
        entries,
    })
}

fn to_typed_path(path: &Path) -> Result<TypedPathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(TypedPathBuf::from_unix(path.as_os_str().as_bytes()))
    }
    #[cfg(windows)]
    {
        let s = path
            .to_str()
            .with_context(|| format!("Non-UTF-8 Windows path: {}", path.display()))?;
        Ok(TypedPathBuf::from_windows(s.as_bytes()))
    }
}

fn file_stem_or(path: &Path, fallback: &str) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}
