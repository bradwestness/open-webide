//! Web search and documentation fetching implementation for the Spin backend.
//!
//! Provides internet research capabilities to the agent via [`openwebide_agent::WebClient`],
//! backed by Spin's WASI outbound HTTP capability (`wasi:http`).
//!
//! Combines multi-source search providers (DuckDuckGo Instant Answer, StackOverflow,
//! Crates.io, GitHub, Wikipedia) with proper User-Agent headers, redirect following,
//! and clean Markdown extraction so developer queries return actionable results
//! without requiring paid API keys or triggering bot blockers.

use std::future::Future;

use bytes::Bytes;
use http_body_util::BodyExt;
use openwebide_agent::WebClient;
use openwebide_core::{WebSearchResult, html_to_markdown};
use spin_sdk::http::{self, FullBody, Request, Response, box_body};

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 OpenWebIDE/0.1 (https://github.com/bradwestness/open-webide)";

/// Outbound web client backed by Spin's HTTP handler.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpinWebClient;

impl WebClient for SpinWebClient {
    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<WebSearchResult>, String>> + Send {
        let query = query.to_string();
        async move { search_web_internal(&query, limit).await }
    }

    fn fetch_page(&self, url: &str) -> impl Future<Output = Result<String, String>> + Send {
        let url = url.to_string();
        async move { fetch_page_internal(&url).await }
    }
}

/// Send an outbound HTTP GET request with standard browser headers and a descriptive User-Agent.
async fn http_get(url: &str) -> Result<Response, String> {
    let req = Request::builder()
        .method("GET")
        .uri(url)
        .header("user-agent", USER_AGENT)
        .header("accept", "application/json, text/html, */*")
        .header("accept-language", "en-US,en;q=0.9")
        .body(box_body(FullBody::new(Bytes::new())))
        .map_err(|e| format!("build request to {url}: {e}"))?;

    http::send(req)
        .await
        .map_err(|e| format!("HTTP request to {url} failed: {e}"))
}

/// Send an HTTP GET request and follow redirects (up to `max_redirects`).
async fn http_get_follow_redirects(
    mut url: String,
    max_redirects: usize,
) -> Result<Response, String> {
    for _ in 0..=max_redirects {
        let res = http_get(&url).await?;
        let status = res.status().as_u16();
        if (status == 301 || status == 302 || status == 303 || status == 307 || status == 308)
            && let Some(loc) = res.headers().get("location").and_then(|v| v.to_str().ok())
        {
            if loc.starts_with("http://") || loc.starts_with("https://") {
                url = loc.to_string();
            } else if loc.starts_with('/') {
                if let Ok(uri) = url.parse::<http::Uri>() {
                    let scheme = uri.scheme_str().unwrap_or("https");
                    let host = uri.host().unwrap_or("");
                    url = format!("{scheme}://{host}{loc}");
                } else {
                    return Ok(res);
                }
            }
            continue;
        }
        return Ok(res);
    }
    Err("too many redirects".to_string())
}

/// Read the complete response body as Bytes.
async fn read_body_bytes(res: Response) -> Result<Bytes, String> {
    let collected = res
        .into_body()
        .collect()
        .await
        .map_err(|e| format!("read response body: {e}"))?;
    Ok(collected.to_bytes())
}

/// Perform HTTP GET and parse the response body as JSON.
async fn fetch_json(url: &str) -> Result<serde_json::Value, String> {
    let res = http_get(url).await?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("GET {url} returned HTTP {status}"));
    }
    let bytes = read_body_bytes(res).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse JSON from {url}: {e}"))
}

/// Perform web search by querying DuckDuckGo, StackOverflow, Crates.io, GitHub, and Wikipedia.
pub async fn search_web_internal(
    query: &str,
    limit: usize,
) -> Result<Vec<WebSearchResult>, String> {
    let limit = limit.clamp(1, 10);
    let mut results = Vec::new();
    let q_lower = query.to_lowercase();
    let dev_keywords = [
        "rust",
        "python",
        "javascript",
        "typescript",
        "html",
        "css",
        "sql",
        "c++",
        "go",
        "error",
        "exception",
        "failed",
        "bug",
        "crash",
        "undefined",
        "cannot",
        "panic",
        "how to",
        "function",
        "api",
        "struct",
        "class",
        "async",
        "await",
        "trait",
        "impl",
        "cargo",
        "npm",
        "pip",
        "git",
        "crate",
        "package",
        "library",
        "framework",
        "repo",
        "leptos",
        "react",
        "vue",
        "tokio",
        "axum",
        "actix",
        "serde",
        "tailwind",
        "vite",
        "wasm",
        "docker",
        "podman",
        "sqlite",
        "postgres",
        "redis",
        "compile",
        "build",
    ];
    let is_dev_query = dev_keywords.iter().any(|k| q_lower.contains(k));
    let is_rust_query = q_lower.contains("rust")
        || q_lower.contains("cargo")
        || q_lower.contains("crate")
        || q_lower.contains("leptos")
        || q_lower.contains("tokio")
        || q_lower.contains("axum")
        || q_lower.contains("serde")
        || q_lower.contains("wasm");

    // 1. DuckDuckGo Instant Answer
    if let Ok(ddg_results) = search_duckduckgo(query, limit).await {
        for res in ddg_results {
            if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                results.push(res);
            }
        }
    }

    if is_dev_query {
        // Dev flow: StackOverflow -> Crates.io -> GitHub -> Wikipedia

        // 2. StackOverflow (essential for developer queries, compiler errors, framework solutions)
        if results.len() < limit
            && let Ok(so_results) = search_stackoverflow(query, limit).await
        {
            for res in so_results {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                    results.push(res);
                }
            }
        }

        // 3. Crates.io (for Rust/crates ecosystem queries)
        if results.len() < limit
            && is_rust_query
            && let Ok(crate_results) = search_crates(query, limit).await
        {
            for res in crate_results {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                    results.push(res);
                }
            }
        }

        // 4. GitHub Repositories (for finding official packages, libraries, tools)
        if results.len() < limit
            && let Ok(gh_results) = search_github(query, limit).await
        {
            for res in gh_results {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                    results.push(res);
                }
            }
        }

        // 5. Wikipedia Opensearch fallback
        if results.len() < limit
            && let Ok(wiki_results) = search_wikipedia_opensearch(query, limit).await
        {
            for res in wiki_results {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                    results.push(res);
                }
            }
        }
    } else {
        // General flow: Wikipedia Opensearch -> StackOverflow -> GitHub -> Wikipedia Full-Text

        // 2. Wikipedia Opensearch (exact article title matches, e.g. "Alan Turing", "Quicksort")
        if results.len() < limit
            && let Ok(wiki_results) = search_wikipedia_opensearch(query, limit).await
        {
            for res in wiki_results {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                    results.push(res);
                }
            }
        }

        // 3. StackOverflow
        if results.len() < limit
            && let Ok(so_results) = search_stackoverflow(query, limit).await
        {
            for res in so_results {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                    results.push(res);
                }
            }
        }

        // 4. GitHub Repositories
        if results.len() < limit
            && let Ok(gh_results) = search_github(query, limit).await
        {
            for res in gh_results {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                    results.push(res);
                }
            }
        }
    }

    // Fallback: Wikipedia Full-Text search
    if results.len() < limit {
        let remaining = limit - results.len();
        if let Ok(wiki_results) = search_wikipedia_fulltext(query, remaining).await {
            for res in wiki_results {
                if results.len() >= limit {
                    break;
                }
                if !results.iter().any(|r: &WebSearchResult| r.url == res.url) {
                    results.push(res);
                }
            }
        }
    }

    Ok(results)
}

/// Search DuckDuckGo Instant Answer JSON API.
async fn search_duckduckgo(query: &str, limit: usize) -> Result<Vec<WebSearchResult>, String> {
    let encoded = url_encode(query);
    let url =
        format!("https://api.duckduckgo.com/?q={encoded}&format=json&no_html=1&skip_disambig=0");

    let json = fetch_json(&url).await?;
    let mut results = Vec::new();

    // Check primary Abstract
    let heading = json["Heading"].as_str().unwrap_or(query);
    let abstract_text = json["AbstractText"].as_str().unwrap_or("");
    let abstract_url = json["AbstractURL"].as_str().unwrap_or("");

    if !abstract_text.is_empty() && !abstract_url.is_empty() {
        results.push(WebSearchResult {
            title: heading.to_string(),
            url: abstract_url.to_string(),
            snippet: abstract_text.to_string(),
        });
    }

    // Check direct Results array (often contains official sites)
    if let Some(res_arr) = json["Results"].as_array() {
        for item in res_arr {
            if results.len() >= limit {
                break;
            }
            if let (Some(url), Some(text)) = (item["FirstURL"].as_str(), item["Text"].as_str())
                && !url.is_empty()
                && !text.is_empty()
            {
                let title = match text.split_once(" - ") {
                    Some((prefix, _)) => prefix.trim().to_string(),
                    None => text.chars().take(60).collect(),
                };
                if !results.iter().any(|r| r.url == url) {
                    results.push(WebSearchResult {
                        title,
                        url: url.to_string(),
                        snippet: text.to_string(),
                    });
                }
            }
        }
    }

    // Extract RelatedTopics
    if let Some(topics) = json["RelatedTopics"].as_array() {
        extract_topics(topics, &mut results, limit);
    }

    Ok(results)
}

fn extract_topics(topics: &[serde_json::Value], results: &mut Vec<WebSearchResult>, limit: usize) {
    for item in topics {
        if results.len() >= limit {
            break;
        }

        // Direct topic
        if let (Some(text), Some(url)) = (item["Text"].as_str(), item["FirstURL"].as_str())
            && !text.is_empty()
            && !url.is_empty()
        {
            let title = match text.split_once(" - ") {
                Some((prefix, _)) => prefix.trim().to_string(),
                None => text.chars().take(60).collect(),
            };
            if !results.iter().any(|r| r.url == url) {
                results.push(WebSearchResult {
                    title,
                    url: url.to_string(),
                    snippet: text.to_string(),
                });
            }
        }

        // Nested category topic group
        if let Some(sub_topics) = item["Topics"].as_array() {
            extract_topics(sub_topics, results, limit);
        }
    }
}

/// Search StackOverflow via StackExchange API.
async fn search_stackoverflow(query: &str, limit: usize) -> Result<Vec<WebSearchResult>, String> {
    let encoded = url_encode(query);
    let url = format!(
        "https://api.stackexchange.com/2.3/search/advanced?order=desc&sort=relevance&q={encoded}&pagesize={limit}&site=stackoverflow"
    );

    let json = fetch_json(&url).await?;
    let mut results = Vec::new();

    if let Some(items) = json["items"].as_array() {
        for item in items {
            if results.len() >= limit {
                break;
            }
            let title_raw = item["title"].as_str().unwrap_or("");
            let link = item["link"].as_str().unwrap_or("");
            if title_raw.is_empty() || link.is_empty() {
                continue;
            }

            let title = decode_html_entities(title_raw);
            let score = item["score"].as_i64().unwrap_or(0);
            let answer_count = item["answer_count"].as_i64().unwrap_or(0);
            let is_answered = item["is_answered"].as_bool().unwrap_or(false);

            let mut tags_str = String::new();
            if let Some(tags) = item["tags"].as_array() {
                let tag_names: Vec<&str> = tags.iter().filter_map(|t| t.as_str()).take(5).collect();
                if !tag_names.is_empty() {
                    tags_str = format!("[{}] ", tag_names.join(", "));
                }
            }

            let status = if is_answered { "solved" } else { "open" };
            let snippet = format!("{tags_str}Score: {score} | Answers: {answer_count} ({status})");

            results.push(WebSearchResult {
                title,
                url: link.to_string(),
                snippet,
            });
        }
    }

    Ok(results)
}

/// Search Crates.io API for Rust crates.
async fn search_crates(query: &str, limit: usize) -> Result<Vec<WebSearchResult>, String> {
    let encoded = url_encode(query);
    let url = format!("https://crates.io/api/v1/crates?q={encoded}&per_page={limit}");

    let json = fetch_json(&url).await?;
    let mut results = Vec::new();

    if let Some(crates) = json["crates"].as_array() {
        for c in crates {
            if results.len() >= limit {
                break;
            }
            let name = c["name"].as_str().unwrap_or("");
            let desc = c["description"].as_str().unwrap_or("");
            let max_ver = c["max_version"].as_str().unwrap_or("");
            let doc_url = c["documentation"].as_str();

            if !name.is_empty() {
                let url = doc_url
                    .filter(|u| !u.is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| format!("https://crates.io/crates/{name}"));

                let snippet = if !desc.is_empty() {
                    format!("v{max_ver}: {desc}")
                } else {
                    format!("Rust crate '{name}' v{max_ver}")
                };

                results.push(WebSearchResult {
                    title: format!("crates.io: {name}"),
                    url,
                    snippet,
                });
            }
        }
    }

    Ok(results)
}

/// Search GitHub repositories API.
async fn search_github(query: &str, limit: usize) -> Result<Vec<WebSearchResult>, String> {
    let encoded = url_encode(query);
    let url = format!("https://api.github.com/search/repositories?q={encoded}&per_page={limit}");

    let json = fetch_json(&url).await?;
    let mut results = Vec::new();

    if let Some(items) = json["items"].as_array() {
        for item in items {
            if results.len() >= limit {
                break;
            }
            let name = item["full_name"].as_str().unwrap_or("");
            let html_url = item["html_url"].as_str().unwrap_or("");
            let desc = item["description"].as_str().unwrap_or("");
            let stars = item["stargazers_count"].as_u64().unwrap_or(0);
            let lang = item["language"].as_str().unwrap_or("");

            if !name.is_empty() && !html_url.is_empty() {
                let mut snippet = String::new();
                if !lang.is_empty() {
                    snippet.push_str(&format!("[{lang}] "));
                }
                if !desc.is_empty() {
                    snippet.push_str(desc);
                    snippet.push(' ');
                }
                snippet.push_str(&format!("(★ {stars})"));

                results.push(WebSearchResult {
                    title: format!("GitHub - {name}"),
                    url: html_url.to_string(),
                    snippet,
                });
            }
        }
    }

    Ok(results)
}

/// Search Wikipedia Opensearch for exact article title matches.
async fn search_wikipedia_opensearch(
    query: &str,
    limit: usize,
) -> Result<Vec<WebSearchResult>, String> {
    let mut results = Vec::new();
    let encoded = url_encode(query);
    let open_url = format!(
        "https://en.wikipedia.org/w/api.php?action=opensearch&search={encoded}&limit={limit}&format=json"
    );

    let json = fetch_json(&open_url).await?;
    let titles = json.get(1).and_then(|v| v.as_array());
    let snippets = json.get(2).and_then(|v| v.as_array());
    let urls = json.get(3).and_then(|v| v.as_array());

    if let (Some(titles), Some(urls)) = (titles, urls) {
        for (i, title_val) in titles.iter().enumerate() {
            if results.len() >= limit {
                break;
            }
            let title = title_val.as_str().unwrap_or("").to_string();
            let url = urls
                .get(i)
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = snippets
                .and_then(|s| s.get(i))
                .and_then(|sn| sn.as_str())
                .unwrap_or("")
                .to_string();

            let snippet = if snippet.is_empty() {
                title.clone()
            } else {
                snippet
            };

            if !title.is_empty() && !url.is_empty() {
                results.push(WebSearchResult {
                    title,
                    url,
                    snippet,
                });
            }
        }
    }

    Ok(results)
}

/// Search Wikipedia Full-Text Search API for topic content matches.
async fn search_wikipedia_fulltext(
    query: &str,
    limit: usize,
) -> Result<Vec<WebSearchResult>, String> {
    let mut results = Vec::new();
    let encoded = url_encode(query);
    let srch_url = format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={encoded}&srlimit={limit}&format=json"
    );

    let json = fetch_json(&srch_url).await?;
    if let Some(items) = json["query"]["search"].as_array() {
        for item in items {
            if results.len() >= limit {
                break;
            }
            let title = item["title"].as_str().unwrap_or("");
            let snippet_raw = item["snippet"].as_str().unwrap_or("");
            if !title.is_empty() {
                let page_url = format!(
                    "https://en.wikipedia.org/wiki/{}",
                    url_encode(&title.replace(' ', "_"))
                );
                let snippet = strip_html_tags(snippet_raw);
                results.push(WebSearchResult {
                    title: title.to_string(),
                    url: page_url,
                    snippet: if snippet.is_empty() {
                        title.to_string()
                    } else {
                        snippet
                    },
                });
            }
        }
    }

    Ok(results)
}

/// Fetch a web page URL and convert HTML to clean Markdown.
pub async fn fetch_page_internal(url: &str) -> Result<String, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".into());
    }

    // StackOverflow blocks automated HTML scrapers via Cloudflare; use their official API instead
    if let Some(pos) = url.find("stackoverflow.com/questions/") {
        let sub = &url[pos + "stackoverflow.com/questions/".len()..];
        let id_str = sub.split(['/', '?', '#']).next().unwrap_or("");
        if let Ok(id) = id_str.parse::<u64>()
            && let Ok(content) = fetch_stackoverflow_question(id).await
        {
            return Ok(content);
        }
    }

    let res = http_get_follow_redirects(url.to_string(), 3).await?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("fetch {url} returned HTTP {status}"));
    }

    let raw_bytes = read_body_bytes(res).await?;

    // Limit raw HTML processing to 512KB
    let slice = if raw_bytes.len() > 524_288 {
        &raw_bytes[..524_288]
    } else {
        &raw_bytes[..]
    };
    let html_content = String::from_utf8_lossy(slice);

    // Convert HTML to Markdown bounded to 16KB (~4,000 tokens)
    let markdown = html_to_markdown(&html_content, 16_384);
    if markdown.trim().is_empty() {
        Ok(format!(
            "(Web page {url} returned no readable text content)"
        ))
    } else {
        Ok(markdown)
    }
}

/// Fetch a StackOverflow question and top answers via the StackExchange API.
async fn fetch_stackoverflow_question(id: u64) -> Result<String, String> {
    let q_url = format!(
        "https://api.stackexchange.com/2.3/questions/{id}?site=stackoverflow&filter=withbody"
    );
    let q_json = fetch_json(&q_url).await?;
    let item = q_json["items"]
        .as_array()
        .and_then(|arr| arr.first())
        .ok_or("question not found")?;

    let title = decode_html_entities(item["title"].as_str().unwrap_or(""));
    let body_html = item["body"].as_str().unwrap_or("");
    let body_md = html_to_markdown(body_html, 8_192);

    let mut out = format!("# {title}\n\n{body_md}\n\n## Answers\n\n");

    let a_url = format!(
        "https://api.stackexchange.com/2.3/questions/{id}/answers?order=desc&sort=votes&pagesize=3&site=stackoverflow&filter=withbody"
    );
    if let Ok(a_json) = fetch_json(&a_url).await
        && let Some(answers) = a_json["items"].as_array()
    {
        for (i, ans) in answers.iter().enumerate() {
            let score = ans["score"].as_i64().unwrap_or(0);
            let is_accepted = ans["is_accepted"].as_bool().unwrap_or(false);
            let ans_html = ans["body"].as_str().unwrap_or("");
            let ans_md = html_to_markdown(ans_html, 8_192);
            let badge = if is_accepted { " (Accepted)" } else { "" };
            let num = i + 1;
            out.push_str(&format!(
                "### Answer {num} (Score: {score}{badge})\n\n{ans_md}\n\n---\n\n"
            ));
        }
    }

    Ok(out.trim_end_matches("\n---\n\n").to_string())
}

/// Strip basic HTML tags from a text string.
fn strip_html_tags(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    decode_html_entities(out.trim())
}

/// Minimal HTML entity decoder.
fn decode_html_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

/// URL encode query parameter components.
pub fn url_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}
