use std::collections::{HashMap, HashSet};

use crate::store::SymbolIndex;

const DAMPING: f64 = 0.85;
const ITERATIONS: usize = 20;
const MAX_EDGES_PER_NAME: usize = 8;

pub fn score_files(index: &SymbolIndex, query: Option<&str>) -> HashMap<String, f64> {
    let files: Vec<&String> = index.files.keys().collect();
    if files.is_empty() {
        return HashMap::new();
    }

    let mut defs: HashMap<&str, Vec<&String>> = HashMap::new();
    for (path, file) in &index.files {
        for symbol in &file.defs {
            defs.entry(symbol.name.as_str()).or_default().push(path);
        }
    }

    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for path in index.files.keys() {
        edges.entry(path.clone()).or_default();
    }
    for (src, file) in &index.files {
        let mut seen = HashSet::new();
        for name in &file.refs {
            let Some(targets) = defs.get(name.as_str()) else {
                continue;
            };
            for dst in targets.iter().take(MAX_EDGES_PER_NAME) {
                if *dst == src {
                    continue;
                }
                if seen.insert((*dst).clone()) {
                    edges.entry(src.clone()).or_default().push((*dst).clone());
                }
            }
        }
    }

    let personal = personalization(index, query);
    pagerank(&files, &edges, &personal)
}

fn personalization(index: &SymbolIndex, query: Option<&str>) -> HashMap<String, f64> {
    let mut weights: HashMap<String, f64> = HashMap::new();
    let terms: Vec<String> = query
        .unwrap_or("")
        .split_whitespace()
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_ascii_lowercase())
        .collect();

    for (path, file) in &index.files {
        let mut w = 1.0;
        if !terms.is_empty() {
            let path_l = path.to_ascii_lowercase();
            w = 0.15;
            for term in &terms {
                if path_l.contains(term) {
                    w += 3.0;
                }
                for symbol in &file.defs {
                    if symbol.name.eq_ignore_ascii_case(term) {
                        w += 8.0;
                    } else if symbol.name.to_ascii_lowercase().contains(term) {
                        w += 2.0;
                    }
                }
            }
        }
        weights.insert(path.clone(), w);
    }

    let sum: f64 = weights.values().sum();
    if sum <= 0.0 {
        let n = index.files.len() as f64;
        return index.files.keys().map(|p| (p.clone(), 1.0 / n)).collect();
    }
    for v in weights.values_mut() {
        *v /= sum;
    }
    weights
}

fn pagerank(
    files: &[&String],
    edges: &HashMap<String, Vec<String>>,
    personal: &HashMap<String, f64>,
) -> HashMap<String, f64> {
    let n = files.len() as f64;
    let mut scores: HashMap<String, f64> = files
        .iter()
        .map(|p| (*p).clone())
        .zip(std::iter::repeat(1.0 / n))
        .collect();

    for _ in 0..ITERATIONS {
        let mut next: HashMap<String, f64> = files
            .iter()
            .map(|p| {
                (
                    (*p).clone(),
                    (1.0 - DAMPING) * personal.get(*p).copied().unwrap_or(0.0),
                )
            })
            .collect();
        for (src, dsts) in edges {
            if dsts.is_empty() {
                continue;
            }
            let share = scores.get(src).copied().unwrap_or(0.0) * DAMPING / dsts.len() as f64;
            for dst in dsts {
                *next.entry(dst.clone()).or_insert(0.0) += share;
            }
        }
        scores = next;
    }
    scores
}
