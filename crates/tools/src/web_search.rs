use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use zene_config::{WebSearchConfig, WebSearchProviderKind};
use zene_llm::ToolDefinition;

use crate::registry::{Tool, ToolContext, ToolResult};

const SEARCH_TIMEOUT_SECS: u64 = 30;
const DEFAULT_NUM_RESULTS: u32 = 5;
const MAX_NUM_RESULTS: u32 = 20;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
struct WebSearchArgs {
    query: String,
    #[serde(default)]
    num_results: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    results: Vec<TavilyResult>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
}

pub struct WebSearchTool {
    config: WebSearchConfig,
}

impl WebSearchTool {
    pub fn new(config: WebSearchConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "WebSearch".to_string(),
            description: "Search the web for current information. Returns titles, URLs, and snippets.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The query text to search for."
                    },
                    "num_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_NUM_RESULTS,
                        "description": "Number of results to return (default 5)."
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, arguments: &str, ctx: &ToolContext) -> Result<ToolResult> {
        let args: WebSearchArgs =
            serde_json::from_str(arguments).context("parse WebSearch args")?;
        if args.query.trim().is_empty() {
            return Ok(ToolResult {
                content: "WebSearch requires a non-empty `query`.".to_string(),
                is_error: true,
            });
        }

        if let Some(cancel) = &ctx.cancel {
            if cancel.is_cancelled() {
                return Ok(ToolResult {
                    content: "Search cancelled.".to_string(),
                    is_error: true,
                });
            }
        }

        let num_results = args
            .num_results
            .unwrap_or(DEFAULT_NUM_RESULTS)
            .clamp(1, MAX_NUM_RESULTS);

        let egress_url = match self.config.effective_provider() {
            WebSearchProviderKind::Tavily => "https://api.tavily.com/search",
            WebSearchProviderKind::DuckDuckGo => "https://html.duckduckgo.com/html/",
        };
        if let Err(err) = ctx.sandbox.authorize_egress(egress_url).await {
            return Ok(ToolResult {
                content: format!("WebSearch blocked by sandbox: {err}"),
                is_error: true,
            });
        }

        match run_search(&self.config, &args.query, num_results).await {
            Ok(results) => Ok(ToolResult {
                content: format_results(&results),
                is_error: false,
            }),
            Err(err) => Ok(ToolResult {
                content: classify_search_error(&err),
                is_error: true,
            }),
        }
    }
}

async fn run_search(
    config: &WebSearchConfig,
    query: &str,
    num_results: u32,
) -> Result<Vec<SearchResult>> {
    match config.effective_provider() {
        WebSearchProviderKind::Tavily => {
            let api_key = config
                .resolved_api_key()
                .context("Tavily search requires an API key. Set [web_search].api_key in config.toml or ZENE_WEB_SEARCH_API_KEY, or use provider = \"duckduckgo\" for a no-key fallback (limited results).")?;
            search_tavily(query, num_results, &api_key).await
        }
        WebSearchProviderKind::DuckDuckGo => search_duckduckgo(query, num_results).await,
    }
}

async fn search_tavily(query: &str, num_results: u32, api_key: &str) -> Result<Vec<SearchResult>> {
    let client = http_client()?;
    let body = json!({
        "api_key": api_key,
        "query": query,
        "max_results": num_results,
        "search_depth": "basic",
    });

    let response = client
        .post("https://api.tavily.com/search")
        .json(&body)
        .send()
        .await
        .context("network error calling Tavily")?;

    let status = response.status();
    if status.as_u16() == 401 {
        anyhow::bail!("HTTP 401 (authentication failed)");
    }
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        anyhow::bail!("HTTP {}. {}", status.as_u16(), detail.trim());
    }

    let raw = response.text().await.context("read Tavily response")?;
    parse_tavily_response(&raw)
}

async fn search_duckduckgo(query: &str, num_results: u32) -> Result<Vec<SearchResult>> {
    let client = http_client()?;
    let response = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header(
            reqwest::header::USER_AGENT,
            "Zene/0.1 (+https://github.com/zene)",
        )
        .send()
        .await
        .context("network error calling DuckDuckGo")?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {}", status.as_u16());
    }

    let html = response.text().await.context("read DuckDuckGo response")?;
    let mut results = parse_duckduckgo_html(&html)?;
    results.truncate(num_results as usize);
    Ok(results)
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(SEARCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("build HTTP client")
}

pub fn parse_tavily_response(raw: &str) -> Result<Vec<SearchResult>> {
    let parsed: TavilyResponse =
        serde_json::from_str(raw).context("parse Tavily JSON response")?;
    Ok(parsed
        .results
        .into_iter()
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            snippet: r.content,
        })
        .collect())
}

pub fn parse_duckduckgo_html(html: &str) -> Result<Vec<SearchResult>> {
    let block_re = Regex::new(
        r#"(?is)<div class="result[^"]*">.*?<a[^>]*class="result__a"[^>]*href="([^"]*)"[^>]*>(.*?)</a>.*?<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#,
    )
    .context("compile DuckDuckGo block regex")?;

    let tag_re = Regex::new(r"(?s)<[^>]+>").unwrap();
    let mut results = Vec::new();

    for cap in block_re.captures_iter(html) {
        let url = decode_duckduckgo_url(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
        let title = decode_html_entities(&strip_tags(
            cap.get(2).map(|m| m.as_str()).unwrap_or(""),
            &tag_re,
        ));
        let snippet = decode_html_entities(&strip_tags(
            cap.get(3).map(|m| m.as_str()).unwrap_or(""),
            &tag_re,
        ));
        if url.is_empty() && title.is_empty() {
            continue;
        }
        results.push(SearchResult {
            title,
            url,
            snippet,
        });
    }

    if results.is_empty() {
        anyhow::bail!(
            "no results parsed from DuckDuckGo HTML (page layout may have changed or query returned no hits)"
        );
    }

    Ok(results)
}

fn strip_tags(input: &str, tag_re: &Regex) -> String {
    tag_re.replace_all(input, " ").into_owned().trim().to_string()
}

fn decode_duckduckgo_url(raw: &str) -> String {
    if let Some(idx) = raw.find("uddg=") {
        let encoded = &raw[idx + 5..];
        let encoded = encoded.split('&').next().unwrap_or(encoded);
        return urlencoding_decode(encoded);
    }
    raw.to_string()
}

fn urlencoding_decode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(byte as char);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(' ');
            i += 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn decode_html_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

pub fn format_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No search results found.".to_string();
    }

    let mut out = String::new();
    for (idx, result) in results.iter().enumerate() {
        if idx > 0 {
            out.push_str("---\n\n");
        }
        out.push_str(&format!("Title: {}\n", result.title));
        out.push_str(&format!("URL: {}\n", result.url));
        out.push_str(&format!("Snippet: {}\n", result.snippet));
    }
    out
}

fn classify_search_error(err: &anyhow::Error) -> String {
    let message = err.to_string();
    let lower = message.to_lowercase();

    if lower.contains("abort") {
        return format!("Search cancelled: {message}");
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return format!("Search timed out: {message}");
    }
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("auth") {
        return format!("Search failed (authentication): {message}");
    }
    if lower.contains("http")
        || lower.contains("network")
        || lower.contains("connection")
        || lower.contains("dns")
    {
        return format!("Search failed (network): {message}");
    }
    format!("Search failed: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tavily_fixture() {
        let raw = r#"{
            "results": [
                {"title": "Rust Book", "url": "https://doc.rust-lang.org/book/", "content": "The official guide."},
                {"title": "Rust by Example", "url": "https://doc.rust-lang.org/rust-by-example/", "content": "Examples."}
            ]
        }"#;
        let results = parse_tavily_response(raw).expect("parse tavily");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Book");
        assert_eq!(results[0].snippet, "The official guide.");
    }

    #[test]
    fn parse_duckduckgo_fixture() {
        let html = include_str!("../tests/fixtures/duckduckgo_results.html");
        let results = parse_duckduckgo_html(html).expect("parse ddg");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Domain");
        assert!(results[0].url.contains("example.com"));
        assert!(results[0].snippet.contains("Illustrative examples"));
    }

    #[test]
    fn format_results_text() {
        let text = format_results(&[SearchResult {
            title: "A".into(),
            url: "https://a.test".into(),
            snippet: "Snippet A".into(),
        }]);
        assert!(text.contains("Title: A"));
        assert!(text.contains("URL: https://a.test"));
        assert!(text.contains("Snippet: Snippet A"));
    }

    #[test]
    fn format_empty_results() {
        assert_eq!(format_results(&[]), "No search results found.");
    }

    #[test]
    fn decode_duckduckgo_redirect_url() {
        let raw = "https://duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F&rut=abc";
        assert_eq!(decode_duckduckgo_url(raw), "https://example.com/");
    }
}
