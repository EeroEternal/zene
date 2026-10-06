//! Harness tree: data-ized composition, rendering, and mutation admission
//! (docs/harness-evolution.md §3.1/§3.2).
//!
//! Four node kinds map 1:1 onto existing filesystem conventions, so the
//! loading path stays untouched: `rules` -> `AGENTS.md` (`agent_instructions`),
//! `skill` -> `.agents/skills/<name>/SKILL.md` (`discover_skills`), `config` ->
//! `.zene/config.toml` (deep-merged), `prompt` -> the `system_prompt` config
//! key. Endpoints, credentials, permissions, sandbox, hooks and the model are
//! never mutable: [`DENIED_CONFIG_KEYS`] is the admission boundary.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Config keys that mutations may never set: endpoints, credentials,
/// permissions, sandbox, hooks, and the model (evaluation determinism).
pub const DENIED_CONFIG_KEYS: &[&str] = &[
    "provider",
    "model",
    "api_key",
    "base_url",
    "anthropic_base_url",
    "anthropic_api_key",
    "permission_mode",
    "permission_rules",
    "sandbox",
    "hooks",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessKind {
    Config,
    Rules,
    Skill,
    Prompt,
}

/// One harness node. `config` fields per kind:
/// `rules`/`prompt` = `{"body": String}`, `skill` = `{"name", "body"}`,
/// `config` = `{"data": Object}` merged into `.zene/config.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessEntry {
    pub id: String,
    pub kind: HarnessKind,
    pub config: Value,
}

/// Flat, ordered harness composition; render order is tree order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessTree {
    pub entries: Vec<HarnessEntry>,
}

impl HarnessTree {
    pub fn get(&self, id: &str) -> Option<&HarnessEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Admission for the tree shape: unique ids and per-kind validation.
    pub fn validate(&self) -> Result<()> {
        let mut seen = BTreeMap::new();
        for entry in &self.entries {
            if entry.id.trim().is_empty() {
                bail!("entry id must not be empty");
            }
            if seen.insert(&entry.id, ()).is_some() {
                bail!("duplicate entry id `{}`", entry.id);
            }
            validate_entry(entry)?;
        }
        Ok(())
    }

    /// Render the tree into an episode workdir. Loading-side consumers read
    /// filesystem conventions, so this writes the files those consumers expect.
    pub fn render(&self, workdir: &Path) -> Result<()> {
        self.validate()?;
        let mut rules: Vec<String> = Vec::new();
        let mut config = Map::new();
        let mut want_config = false;
        for entry in &self.entries {
            match entry.kind {
                HarnessKind::Rules => rules.push(text_field(entry, "body")?.to_string()),
                HarnessKind::Prompt => {
                    want_config = true;
                    config.insert(
                        "system_prompt".to_string(),
                        Value::String(text_field(entry, "body")?.to_string()),
                    );
                }
                HarnessKind::Skill => {
                    let name = text_field(entry, "name")?;
                    let dir = workdir.join(".agents/skills").join(name);
                    fs::create_dir_all(&dir)
                        .with_context(|| format!("create skill dir {}", dir.display()))?;
                    fs::write(dir.join("SKILL.md"), text_field(entry, "body")?.as_bytes())
                        .with_context(|| format!("write skill {name}"))?;
                }
                HarnessKind::Config => {
                    want_config = true;
                    let data = entry
                        .config
                        .get("data")
                        .and_then(Value::as_object)
                        .context("config entry needs a `data` object")?;
                    deep_merge(&mut config, data);
                }
            }
        }
        if !rules.is_empty() {
            fs::create_dir_all(workdir).with_context(|| create_workdir(workdir))?;
            fs::write(workdir.join("AGENTS.md"), rules.join("\n\n")).context("write AGENTS.md")?;
        }
        if want_config {
            let dir = workdir.join(".zene");
            fs::create_dir_all(&dir).with_context(|| create_workdir(&dir))?;
            let rendered = toml::to_string_pretty(&Value::Object(config))
                .context("render config entries to TOML")?;
            fs::write(dir.join("config.toml"), rendered).context("write .zene/config.toml")?;
        }
        Ok(())
    }
}

fn create_workdir(path: &Path) -> String {
    format!("create {}", path.display())
}

fn text_field<'a>(entry: &'a HarnessEntry, key: &str) -> Result<&'a str> {
    entry
        .config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .with_context(|| format!("entry `{}` needs a non-empty `{key}`", entry.id))
}

fn validate_entry(entry: &HarnessEntry) -> Result<()> {
    if let Some(key) = first_denied_key(&entry.config) {
        bail!(
            "entry `{}` touches denied config key `{key}` (endpoints/credentials/permissions/model stay outside the tree)",
            entry.id
        );
    }
    match entry.kind {
        HarnessKind::Rules | HarnessKind::Prompt => {
            text_field(entry, "body")?;
        }
        HarnessKind::Skill => {
            let name = text_field(entry, "name")?;
            if !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                bail!(
                    "entry `{}`: skill name `{name}` must be [A-Za-z0-9_-] (no path separators)",
                    entry.id
                );
            }
            let body = text_field(entry, "body")?;
            if !body.trim_start().starts_with("---") {
                bail!(
                    "entry `{}`: SKILL.md body must start with frontmatter (`---`)",
                    entry.id
                );
            }
        }
        HarnessKind::Config => {
            let data = entry
                .config
                .get("data")
                .and_then(Value::as_object)
                .context("config entry needs a `data` object")?;
            toml::to_string(&Value::Object(data.clone()))
                .context("config `data` must be TOML-representable")?;
        }
    }
    Ok(())
}

fn first_denied_key(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                if DENIED_CONFIG_KEYS.contains(&key.as_str()) {
                    return Some(key.clone());
                }
                if let Some(found) = first_denied_key(nested) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(first_denied_key),
        _ => None,
    }
}

fn deep_merge(target: &mut Map<String, Value>, patch: &Map<String, Value>) {
    for (key, value) in patch {
        match (target.get_mut(key), value) {
            (Some(Value::Object(existing)), Value::Object(nested)) => deep_merge(existing, nested),
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

/// A controlled change against a [`HarnessTree`] (docs/harness-evolution.md §3.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub op: MutationOp,
    pub id: String,
    /// Required for `create` (node kind) and forbidden to differ on `update`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<HarnessKind>,
    /// Full replacement node config. Required for `create` and `update`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationOp {
    Create,
    Update,
    Remove,
}

/// Apply mutations with admission: `create` rejects existing ids, `update`
/// rejects missing ids or kind changes, `remove` rejects missing ids, and the
/// resulting tree must pass [`HarnessTree::validate`]. Returns the new tree;
/// the input is untouched (reject = nothing moves).
pub fn apply_mutations(tree: &HarnessTree, mutations: &[Mutation]) -> Result<HarnessTree> {
    let mut entries = tree.entries.clone();
    for mutation in mutations {
        let existing = entries.iter().position(|entry| entry.id == mutation.id);
        match mutation.op {
            MutationOp::Create => {
                if existing.is_some() {
                    bail!("create rejected: id `{}` already exists", mutation.id);
                }
                let kind = mutation.kind.context("create requires `kind`")?.to_owned();
                let config = mutation
                    .config
                    .clone()
                    .context("create requires `config`")?;
                entries.push(HarnessEntry {
                    id: mutation.id.clone(),
                    kind,
                    config,
                });
            }
            MutationOp::Update => {
                let index = existing
                    .with_context(|| format!("update rejected: id `{}` missing", mutation.id))?;
                if let Some(kind) = mutation.kind {
                    if kind != entries[index].kind {
                        bail!("update rejected: `{}` cannot change kind", mutation.id);
                    }
                }
                let config = mutation
                    .config
                    .clone()
                    .context("update requires `config`")?;
                entries[index].config = config;
            }
            MutationOp::Remove => {
                let index = existing
                    .with_context(|| format!("remove rejected: id `{}` missing", mutation.id))?;
                entries.remove(index);
            }
        }
    }
    let candidate = HarnessTree { entries };
    candidate.validate()?;
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, kind: HarnessKind, config: Value) -> HarnessEntry {
        HarnessEntry {
            id: id.to_string(),
            kind,
            config,
        }
    }

    fn tree() -> HarnessTree {
        HarnessTree {
            entries: vec![
                entry(
                    "r1",
                    HarnessKind::Rules,
                    json!({"body": "# Rules\n\nbe brief"}),
                ),
                entry(
                    "s1",
                    HarnessKind::Skill,
                    json!({"name": "demo", "body": "---\nname: demo\n---\n\nbody"}),
                ),
                entry("c1", HarnessKind::Config, json!({"data": {"max_turns": 7}})),
                entry("p1", HarnessKind::Prompt, json!({"body": "You are terse."})),
            ],
        }
    }

    #[test]
    fn render_writes_every_kind_to_its_convention() {
        let base = tempfile::tempdir().unwrap();
        let workdir = base.path().join("episode");
        tree().render(&workdir).unwrap();

        let agents = fs::read_to_string(workdir.join("AGENTS.md")).unwrap();
        assert!(agents.contains("# Rules\n\nbe brief"));
        let skill = fs::read_to_string(workdir.join(".agents/skills/demo/SKILL.md")).unwrap();
        assert!(skill.starts_with("---"));
        let config = fs::read_to_string(workdir.join(".zene/config.toml")).unwrap();
        assert!(config.contains("max_turns = 7"));
        assert!(config.contains("system_prompt = \"You are terse.\""));
    }

    #[test]
    fn admission_rejects_contract_violations() {
        let base = tree();
        let err = apply_mutations(
            &base,
            &[Mutation {
                op: MutationOp::Create,
                id: "r1".into(),
                kind: Some(HarnessKind::Rules),
                config: Some(json!({"body": "x"})),
            }],
        )
        .unwrap_err();
        assert!(err.to_string().contains("already exists"));

        let err = apply_mutations(
            &base,
            &[Mutation {
                op: MutationOp::Update,
                id: "missing".into(),
                kind: None,
                config: Some(json!({"body": "x"})),
            }],
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing"));

        let err = apply_mutations(
            &base,
            &[Mutation {
                op: MutationOp::Update,
                id: "r1".into(),
                kind: Some(HarnessKind::Prompt),
                config: Some(json!({"body": "x"})),
            }],
        )
        .unwrap_err();
        assert!(err.to_string().contains("cannot change kind"));

        let err = apply_mutations(
            &base,
            &[Mutation {
                op: MutationOp::Remove,
                id: "missing".into(),
                kind: None,
                config: None,
            }],
        )
        .unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn admission_rejects_denied_and_hostile_inputs() {
        let base = tree();
        // credentials/endpoints/permissions/model never enter the tree
        let err = apply_mutations(
            &base,
            &[Mutation {
                op: MutationOp::Create,
                id: "evil".into(),
                kind: Some(HarnessKind::Config),
                config: Some(json!({"data": {"api_key": "sk-leak"}})),
            }],
        )
        .unwrap_err();
        assert!(err.to_string().contains("denied config key"));

        // skill names cannot escape the skills directory
        let err = apply_mutations(
            &base,
            &[Mutation {
                op: MutationOp::Update,
                id: "s1".into(),
                kind: None,
                config: Some(json!({"name": "../../etc", "body": "---\n"})),
            }],
        )
        .unwrap_err();
        assert!(err.to_string().contains("path separators"));

        // skills must keep frontmatter
        let err = apply_mutations(
            &base,
            &[Mutation {
                op: MutationOp::Update,
                id: "s1".into(),
                kind: None,
                config: Some(json!({"name": "demo", "body": "no frontmatter"})),
            }],
        )
        .unwrap_err();
        assert!(err.to_string().contains("frontmatter"));
    }

    #[test]
    fn apply_produces_candidate_tree_and_leaves_input_untouched() {
        let base = tree();
        let candidate = apply_mutations(
            &base,
            &[
                Mutation {
                    op: MutationOp::Update,
                    id: "r1".into(),
                    kind: None,
                    config: Some(json!({"body": "# Rules v2"})),
                },
                Mutation {
                    op: MutationOp::Create,
                    id: "p2".into(),
                    kind: Some(HarnessKind::Prompt),
                    config: Some(json!({"body": "extra"})),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            base.get("r1").unwrap().config["body"],
            "# Rules\n\nbe brief"
        );
        assert_eq!(candidate.get("r1").unwrap().config["body"], "# Rules v2");
        assert_eq!(candidate.entries.len(), 5);
    }
}
