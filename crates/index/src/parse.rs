use anyhow::{Context, Result};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::language::SourceLanguage;
use crate::store::Symbol;

pub fn parse_file(language: SourceLanguage, source: &str) -> Result<(Vec<Symbol>, Vec<String>)> {
    let lang = ts_language(language);
    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .with_context(|| format!("set tree-sitter language {}", language.as_str()))?;
    let tree = parser
        .parse(source, None)
        .with_context(|| format!("parse {} source", language.as_str()))?;
    let query = Query::new(&lang, query_src(language))
        .with_context(|| format!("compile {} query", language.as_str()))?;

    let mut defs = Vec::new();
    let mut refs = Vec::new();
    let mut cursor = QueryCursor::new();
    let source_bytes = source.as_bytes();
    let mut matches = cursor.matches(&query, tree.root_node(), source_bytes);

    while let Some(m) = matches.next() {
        let mut name: Option<String> = None;
        let mut def_node = None;
        let mut is_ref = false;
        for capture in m.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let node = capture.node;
            let text = node.utf8_text(source_bytes).unwrap_or("").trim();
            if text.is_empty() {
                continue;
            }
            match capture_name {
                "name" => {
                    name = Some(text.to_string());
                    if def_node.is_none() {
                        def_node = Some(node);
                    }
                }
                "def" => def_node = Some(node),
                "ref" => {
                    is_ref = true;
                    refs.push(text.to_string());
                }
                _ => {}
            }
        }
        if is_ref {
            continue;
        }
        let Some(name) = name else { continue };
        let Some(node) = def_node else { continue };
        let line = node.start_position().row as u32 + 1;
        let signature = first_line(source, node.start_byte());
        let kind = classify_kind(language, &signature);
        defs.push(Symbol {
            kind,
            name,
            line,
            signature,
        });
    }

    defs.sort_by(|a, b| a.line.cmp(&b.line).then(a.name.cmp(&b.name)));
    defs.dedup_by(|a, b| a.name == b.name && a.line == b.line);

    refs.sort();
    refs.dedup();
    refs.retain(|name| name.len() >= 2);

    Ok((defs, refs))
}

fn ts_language(language: SourceLanguage) -> tree_sitter::Language {
    match language {
        SourceLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        SourceLanguage::Python => tree_sitter_python::LANGUAGE.into(),
        SourceLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        SourceLanguage::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        SourceLanguage::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        SourceLanguage::Go => tree_sitter_go::LANGUAGE.into(),
    }
}

fn query_src(language: SourceLanguage) -> &'static str {
    match language {
        SourceLanguage::Rust => RUST_QUERY,
        SourceLanguage::Python => PYTHON_QUERY,
        SourceLanguage::TypeScript | SourceLanguage::Tsx => TYPESCRIPT_QUERY,
        SourceLanguage::JavaScript => JAVASCRIPT_QUERY,
        SourceLanguage::Go => GO_QUERY,
    }
}

fn classify_kind(language: SourceLanguage, signature: &str) -> String {
    let s = signature.trim_start();
    match language {
        SourceLanguage::Rust => {
            if s.contains("fn ") {
                "function".into()
            } else if s.contains("struct ") {
                "struct".into()
            } else if s.contains("enum ") {
                "enum".into()
            } else if s.contains("trait ") {
                "trait".into()
            } else if s.contains("mod ") {
                "module".into()
            } else if s.contains("impl ") {
                "impl".into()
            } else {
                "type".into()
            }
        }
        SourceLanguage::Python => {
            if s.starts_with("class ") {
                "class".into()
            } else {
                "function".into()
            }
        }
        SourceLanguage::Go => {
            if s.contains("type ") {
                "type".into()
            } else if s.contains("func (") {
                "method".into()
            } else {
                "function".into()
            }
        }
        _ => {
            if s.contains("class ") || s.contains("interface ") {
                "type".into()
            } else if s.contains("function ") || s.contains("=>") {
                "function".into()
            } else {
                "symbol".into()
            }
        }
    }
}

fn first_line(source: &str, start_byte: usize) -> String {
    let rest = source.get(start_byte..).unwrap_or("");
    let line = rest.lines().next().unwrap_or(rest).trim();
    let mut out = String::new();
    for ch in line.chars() {
        if out.len() + ch.len_utf8() > 120 {
            break;
        }
        out.push(ch);
    }
    out
}

const RUST_QUERY: &str = r#"
(function_item name: (identifier) @name) @def
(struct_item name: (type_identifier) @name) @def
(enum_item name: (type_identifier) @name) @def
(trait_item name: (type_identifier) @name) @def
(mod_item name: (identifier) @name) @def
(type_item name: (type_identifier) @name) @def
(const_item name: (identifier) @name) @def
(impl_item type: (type_identifier) @name) @def
(macro_definition name: (identifier) @name) @def
(call_expression function: (identifier) @ref)
(macro_invocation macro: (identifier) @ref)
(call_expression function: (field_expression field: (field_identifier) @ref))
"#;

const PYTHON_QUERY: &str = r#"
(function_definition name: (identifier) @name) @def
(class_definition name: (identifier) @name) @def
(call function: (identifier) @ref)
"#;

const JAVASCRIPT_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @def
(class_declaration name: (identifier) @name) @def
(method_definition name: (property_identifier) @name) @def
(lexical_declaration (variable_declarator name: (identifier) @name) @def)
(call_expression function: (identifier) @ref)
"#;

const TYPESCRIPT_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @def
(class_declaration name: (type_identifier) @name) @def
(method_definition name: (property_identifier) @name) @def
(interface_declaration name: (type_identifier) @name) @def
(type_alias_declaration name: (type_identifier) @name) @def
(call_expression function: (identifier) @ref)
"#;

const GO_QUERY: &str = r#"
(function_declaration name: (identifier) @name) @def
(method_declaration name: (field_identifier) @name) @def
(type_declaration (type_spec name: (type_identifier) @name) @def)
(call_expression function: (identifier) @ref)
"#;
