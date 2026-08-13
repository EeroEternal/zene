use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use crate::language::{skip_dir, SourceLanguage};
use crate::parse::parse_file;

const INDEX_VERSION: u32 = 1;
const MAX_FILES: usize = 4000;
const MAX_FILE_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub kind: String,
    pub name: String,
    pub line: u32,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSymbols {
    pub hash: String,
    pub language: String,
    pub defs: Vec<Symbol>,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SymbolIndex {
    pub version: u32,
    pub files: BTreeMap<String, FileSymbols>,
}

#[derive(Debug, Clone, Default)]
pub struct RefreshStats {
    pub parsed: usize,
    pub cached: usize,
    pub removed: usize,
}

pub fn refresh(workdir: &Path) -> Result<(SymbolIndex, RefreshStats)> {
    let path = index_path(workdir);
    let mut index = load_index(&path).unwrap_or_else(|_| SymbolIndex {
        version: INDEX_VERSION,
        files: BTreeMap::new(),
    });
    if index.version != INDEX_VERSION {
        index = SymbolIndex {
            version: INDEX_VERSION,
            files: BTreeMap::new(),
        };
    }

    let source_files = collect_source_files(workdir)?;
    let mut live = BTreeMap::new();
    let mut stats = RefreshStats::default();

    for abs in source_files {
        let rel = relative_path(workdir, &abs);
        let meta = match fs::metadata(&abs) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let bytes = match fs::read(&abs) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(source) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let hash = content_hash(&bytes);
        if let Some(existing) = index.files.get(&rel) {
            if existing.hash == hash {
                live.insert(rel, existing.clone());
                stats.cached += 1;
                continue;
            }
        }
        let Some(language) = SourceLanguage::from_path(&abs) else {
            continue;
        };
        match parse_file(language, source) {
            Ok((defs, refs)) => {
                live.insert(
                    rel,
                    FileSymbols {
                        hash,
                        language: language.as_str().to_string(),
                        defs,
                        refs,
                    },
                );
                stats.parsed += 1;
            }
            Err(_) => continue,
        }
    }

    stats.removed = index
        .files
        .keys()
        .filter(|k| !live.contains_key(*k))
        .count();
    index.files = live;
    save_index(&path, &index)?;
    Ok((index, stats))
}

fn index_path(workdir: &Path) -> PathBuf {
    workdir.join(".zene").join("index").join("v1.json")
}

fn load_index(path: &Path) -> Result<SymbolIndex> {
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn save_index(path: &Path, index: &SymbolIndex) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string(index)?;
    fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("rename {}", path.display()))?;
    Ok(())
}

fn collect_source_files(workdir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(workdir)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !(entry.file_type().is_some_and(|t| t.is_dir()) && skip_dir(&name))
        })
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if SourceLanguage::from_path(path).is_none() {
            continue;
        }
        files.push(path.to_path_buf());
        if files.len() >= MAX_FILES {
            break;
        }
    }
    Ok(files)
}

fn relative_path(workdir: &Path, path: &Path) -> String {
    path.strip_prefix(workdir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
