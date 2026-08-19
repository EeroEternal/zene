use crate::rank::score_files;
use crate::store::{RefreshStats, SymbolIndex};

pub const DEFAULT_TOKEN_BUDGET: u32 = 2500;
const MAX_DEFS_PER_FILE: usize = 40;

pub fn render_map(
    index: &SymbolIndex,
    stats: &RefreshStats,
    query: Option<&str>,
    token_budget: u32,
    path_prefix: Option<&str>,
) -> String {
    let prefix = path_prefix.map(normalize_prefix).filter(|p| !p.is_empty());
    let scores = score_files(index, query);
    let mut ranked: Vec<(&String, f64)> = scores
        .iter()
        .filter(|(path, _)| prefix.as_ref().is_none_or(|p| path_in_prefix(path, p)))
        .map(|(path, score)| (path, *score))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let budget_chars = (token_budget.max(64) as usize).saturating_mul(4);
    let mut body = String::new();
    let mut shown_files = 0usize;
    let mut shown_defs = 0usize;

    for (path, _) in &ranked {
        let Some(file) = index.files.get(*path) else {
            continue;
        };
        if file.defs.is_empty() {
            continue;
        }
        let mut block = format!("{path}\n");
        for symbol in file.defs.iter().take(MAX_DEFS_PER_FILE) {
            block.push_str(&format!("  {} {}\n", symbol.kind, symbol.name));
        }
        if body.len() + block.len() > budget_chars && !body.is_empty() {
            break;
        }
        body.push_str(&block);
        shown_files += 1;
        shown_defs += file.defs.len().min(MAX_DEFS_PER_FILE);
    }

    let mut out = String::new();
    out.push_str("Repository map (signatures only; use Read for implementations).\n");
    if let Some(q) = query.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!("Personalized to: {q}\n"));
    }
    out.push_str(&format!(
        "Indexed {} files ({} parsed, {} cached, {} removed). Showing {shown_files} files / {shown_defs} symbols.\n\n",
        index.files.len(),
        stats.parsed,
        stats.cached,
        stats.removed
    ));
    if body.is_empty() {
        out.push_str("No indexable definitions found.\n");
    } else {
        out.push_str(&body);
    }
    out
}

fn normalize_prefix(path: &str) -> String {
    path.replace('\\', "/").trim_matches('/').to_string()
}

fn path_in_prefix(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}
