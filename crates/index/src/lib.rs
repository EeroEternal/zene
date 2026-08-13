//! Workspace symbol index and on-demand repo map.
//!
//! This crate is Select, not ContextEngine: it never injects into the prompt
//! prefix. Callers expose it as a tool so hits land in the conversation Body.

mod language;
mod parse;
mod rank;
mod render;
mod store;

pub use render::DEFAULT_TOKEN_BUDGET;
pub use store::{refresh as refresh_index, RefreshStats, Symbol, SymbolIndex};

use std::path::Path;

use anyhow::Result;

/// Refresh the sidecar index under `{workdir}/.zene/index/` and render a
/// budgeted repo map. `query` personalizes ranking; `path_prefix` limits
/// which files are shown (the index itself still covers the workspace).
pub fn build_repo_map(
    workdir: &Path,
    query: Option<&str>,
    token_budget: u32,
    path_prefix: Option<&str>,
) -> Result<String> {
    let (index, stats) = store::refresh(workdir)?;
    let budget = if token_budget == 0 {
        render::DEFAULT_TOKEN_BUDGET
    } else {
        token_budget
    };
    Ok(render::render_map(
        &index,
        &stats,
        query,
        budget,
        path_prefix,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn indexes_rust_and_writes_sidecar() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("lib.rs"),
            "pub struct Engine {}\npub fn prepare_step() {}\nfn helper() { prepare_step(); }\n",
        )
        .unwrap();

        let map = build_repo_map(dir.path(), None, 800, None).unwrap();
        assert!(map.contains("lib.rs"));
        assert!(map.contains("prepare_step"));
        assert!(map.contains("Engine"));
        assert!(dir.path().join(".zene/index/v1.json").is_file());
        assert!(map.contains("signatures only"));
    }

    #[test]
    fn incremental_refresh_reuses_unchanged_files() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        fs::write(dir.path().join("b.rs"), "pub fn beta() {}\n").unwrap();

        let (_, first) = refresh_index(dir.path()).unwrap();
        assert_eq!(first.parsed, 2);
        assert_eq!(first.cached, 0);

        fs::write(dir.path().join("b.rs"), "pub fn beta() {}\npub fn gamma() {}\n").unwrap();
        let (index, second) = refresh_index(dir.path()).unwrap();
        assert_eq!(second.parsed, 1);
        assert_eq!(second.cached, 1);
        assert!(index.files["b.rs"].defs.iter().any(|s| s.name == "gamma"));
    }

    #[test]
    fn query_personalizes_ranking() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("auth.rs"), "pub fn process_refund() {}\n").unwrap();
        fs::write(dir.path().join("util.rs"), "pub fn helper() {}\n").unwrap();

        let map = build_repo_map(dir.path(), Some("process_refund"), 800, None).unwrap();
        let auth = map.find("auth.rs").expect("auth.rs in map");
        let util = map.find("util.rs");
        assert!(
            util.is_none_or(|u| auth < u),
            "matching file should rank first:\n{map}"
        );
        assert!(map.contains("Personalized to: process_refund"));
    }
}

