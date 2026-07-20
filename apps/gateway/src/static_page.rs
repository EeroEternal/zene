//! Embedded Web Agent UI.
//!
//! Source of truth lives in `apps/web-agent/index.html` so the frontend can
//! evolve without a Node build step in phase B.

pub const INDEX_HTML: &str = include_str!("../../web-agent/index.html");
