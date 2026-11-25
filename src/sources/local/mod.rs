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
                let parsed = parser.parse_package(typed.to_path()).with_context(|| {
                    format!("Failed to parse .onepkg archive: {}", path.display())
                })?;
                convert::convert_notebook(&parsed, nb_name, &mut warnings)
            }
            "onetoc2" => {
                let nb_name = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| file_stem_or(path, "Notebook"));
                let parsed = parser.parse_notebook(typed.to_path()).with_context(|| {
                    format!("Failed to parse .onetoc2 notebook: {}", path.display())
                })?;
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

struct DirectoryScan {
    id: Option<String>,
    entries: Vec<SectionEntry>,
    found_any_files: bool,
}

fn load_directory(
    parser: &Parser,
    dir: &Path,
    warnings: &mut Vec<ExportWarning>,
) -> Result<Notebook> {
    let nb_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| {
            fs::canonicalize(dir)
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Notebook".to_string());

    let scan = load_directory_entries(parser, dir, warnings)?;
    if !scan.found_any_files {
        bail!(
            "No .onetoc2 or .one files found in directory: {}",
            dir.display()
        );
    }

    Ok(Notebook {
        id: scan.id,
        name: nb_name,
        entries: scan.entries,
    })
}

fn load_directory_entries(
    parser: &Parser,
    dir: &Path,
    warnings: &mut Vec<ExportWarning>,
) -> Result<DirectoryScan> {
    let mut toc_files: Vec<PathBuf> = Vec::new();
    let mut one_files: Vec<PathBuf> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();

    for entry in
        fs::read_dir(dir).with_context(|| format!("Failed to read directory: {}", dir.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let entry_path = entry.path();
        // Use `DirEntry::file_type` (lstat) so symlinks are not followed recursively.
        if file_type.is_dir() {
            let name = entry_path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            if !name.starts_with('.') && name != "OneNote_RecycleBin" {
                subdirs.push(entry_path);
            }
            continue;
        }
        if !file_type.is_file() {
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
    subdirs.sort();

    if let Some(toc_path) = toc_files.first() {
        let typed = to_typed_path(toc_path)?;
        let parsed = parser
            .parse_notebook(typed.to_path())
            .with_context(|| format!("Failed to parse notebook TOC: {}", toc_path.display()))?;
        let group_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Notebook".to_string());
        let nb = convert::convert_notebook(&parsed, group_name, warnings);
        return Ok(DirectoryScan {
            id: nb.id,
            entries: nb.entries,
            found_any_files: true,
        });
    }

    let mut found_any_files = !one_files.is_empty();
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

    for subdir in subdirs {
        match load_directory_entries(parser, &subdir, warnings) {
            Ok(sub_scan) => {
                found_any_files |= sub_scan.found_any_files;
                if !sub_scan.entries.is_empty() {
                    let group_name = subdir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Section Group".to_string());
                    entries.push(SectionEntry::SectionGroup(crate::ir::SectionGroup {
                        id: sub_scan.id,
                        name: group_name,
                        entries: sub_scan.entries,
                    }));
                }
            }
            Err(err) => {
                warnings.push(ExportWarning::new(
                    subdir.display().to_string(),
                    format!("Failed to scan subdirectory: {err}"),
                ));
            }
        }
    }

    Ok(DirectoryScan {
        id: None,
        entries,
        found_any_files,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scans_subdirectories_and_skips_recycle_bin_and_hidden_dirs() {
        let tmp = tempdir().unwrap();
        let nested = tmp.path().join("Group A");
        let recycle = tmp.path().join("OneNote_RecycleBin");
        let hidden = tmp.path().join(".hidden");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&recycle).unwrap();
        fs::create_dir_all(&hidden).unwrap();

        // Invalid .one bytes trigger a scoped parse warning iff the directory was scanned
        fs::write(nested.join("Notes.one"), b"not-a-valid-one-file").unwrap();
        fs::write(recycle.join("Deleted.one"), b"not-a-valid-one-file").unwrap();
        fs::write(hidden.join("Secret.one"), b"not-a-valid-one-file").unwrap();

        let parser = Parser::new();
        let mut warnings = Vec::new();
        let nb = load_directory(&parser, tmp.path(), &mut warnings).unwrap();
        assert!(nb.entries.is_empty());

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].scope.contains("Group A"));
        assert!(!warnings[0].scope.contains("OneNote_RecycleBin"));
        assert!(!warnings[0].scope.contains(".hidden"));
    }
}
