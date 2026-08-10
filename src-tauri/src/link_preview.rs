//! Open Graph metadata for external links, for preview cards (#62).
//!
//! This is the one place Vellum reaches out to the open internet. Everything
//! else is local or peer-to-peer, so the behaviour here is deliberately narrow:
//!
//! - Only fires for `http(s)` links that are alone on their line, and only while
//!   the Link previews setting is on (the frontend gates the call).
//! - One request per URL per session, cached in memory, so re-rendering a note
//!   while typing doesn't re-hit the site.
//! - Bounded: a short timeout, a capped read, and only the `<head>` is parsed.
//! - Failure is not an error state. Offline, a timeout, a 404 and a page with no
//!   metadata all resolve the same way — no card, plain link stays.
//!
//! Fetching is done here rather than in the webview because CORS would block
//! nearly every cross-origin request, and because a `<meta>` scrape needs no DOM.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a fetched preview stays good. Long enough that reopening a note
/// costs nothing, short enough that a corrected title shows up the same day.
const TTL: Duration = Duration::from_secs(6 * 60 * 60);
/// Stop reading once the head is well past — metadata lives at the top, and this
/// keeps a huge or endless page from filling memory.
const MAX_BYTES: usize = 256 * 1024;
/// A preview is a nicety; never let one stall behind a slow host.
const TIMEOUT: Duration = Duration::from_secs(6);
/// Bound on distinct URLs held in memory. Vastly more than one note's links.
const MAX_CACHE: usize = 512;

#[derive(Debug, Clone, Default, Serialize)]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image: Option<String>,
}

impl LinkPreview {
    /// Whether there is anything worth drawing a card for. A card showing only a
    /// domain is worse than the plain link it replaced.
    fn is_useful(&self) -> bool {
        self.title.is_some() || self.description.is_some()
    }
}

static CACHE: Mutex<Option<HashMap<String, (Instant, Option<LinkPreview>)>>> = Mutex::new(None);

fn cached(url: &str) -> Option<Option<LinkPreview>> {
    let mut guard = CACHE.lock().ok()?;
    let map = guard.as_mut()?;
    match map.get(url) {
        Some((at, v)) if at.elapsed() < TTL => Some(v.clone()),
        // Expired — drop it so a refetch can replace it.
        Some(_) => {
            map.remove(url);
            None
        }
        None => None,
    }
}

fn remember(url: &str, value: Option<LinkPreview>) {
    let Ok(mut guard) = CACHE.lock() else { return };
    let map = guard.get_or_insert_with(HashMap::new);
    // Crude but adequate: a full cache is cleared rather than LRU-evicted. It
    // only ever refills from notes the user is actually reading.
    if map.len() >= MAX_CACHE {
        map.clear();
    }
    map.insert(url.to_string(), (Instant::now(), value));
}

/// Decode the handful of HTML entities that actually show up in `content=`
/// attributes. Not a general unescaper — titles are display text, and a stray
/// `&copy;` reading literally is a much smaller problem than pulling in a full
/// entity table.
fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        // Ampersand last, so "&amp;lt;" doesn't become "<".
        .replace("&amp;", "&")
}

/// Pull `key="value"` out of a tag body, honouring single or double quotes.
fn attr(tag: &str, key: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(key) {
        let at = from + i;
        let rest = &tag[at + key.len()..];
        let rest_trim = rest.trim_start();
        // The match must begin an attribute name, not end one: `data-content=`
        // must not answer a search for `content=`. Attribute names may contain
        // `-`, `_` and `:`, so none of those may precede the match either.
        let before_ok = at == 0 || {
            let c = lower.as_bytes()[at - 1];
            !(c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b':')
        };
        if !before_ok || !rest_trim.starts_with('=') {
            from = at + key.len();
            continue;
        }
        let after_eq = rest_trim[1..].trim_start();
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            // Unquoted value — read to the next whitespace.
            let end = after_eq.find(char::is_whitespace).unwrap_or(after_eq.len());
            return Some(unescape(&after_eq[..end]));
        }
        let body = &after_eq[1..];
        let end = body.find(quote)?;
        return Some(unescape(&body[..end]));
    }
    None
}

/// Scrape Open Graph / `<meta name=…>` / `<title>` out of an HTML head.
///
/// Deliberately not a real HTML parser: we want five string fields from the top
/// of the document, and a parser dependency would cost far more than it earns.
/// Anything malformed simply yields `None` for that field.
pub(crate) fn parse_metadata(html: &str, url: &str) -> LinkPreview {
    let mut out = LinkPreview {
        url: url.to_string(),
        ..Default::default()
    };
    let lower = html.to_ascii_lowercase();

    // Each `<meta …>` tag, in document order. og:* wins over name="…" because
    // it is the tag actually meant for link previews, so it is applied second.
    let mut i = 0;
    let mut plain: LinkPreview = LinkPreview::default();
    while let Some(rel) = lower[i..].find("<meta") {
        let start = i + rel;
        let Some(len) = lower[start..].find('>') else { break };
        let tag = &html[start..start + len];
        i = start + len;
        let key = attr(tag, "property")
            .or_else(|| attr(tag, "name"))
            .map(|k| k.to_ascii_lowercase());
        let Some(key) = key else { continue };
        let Some(content) = attr(tag, "content").filter(|c| !c.trim().is_empty()) else {
            continue;
        };
        let content = content.trim().to_string();
        match key.as_str() {
            "og:title" => out.title = Some(content),
            "og:description" => out.description = Some(content),
            "og:site_name" => out.site_name = Some(content),
            "og:image" => out.image = Some(content),
            "twitter:title" => {
                plain.title.get_or_insert(content);
            }
            "twitter:description" | "description" => {
                plain.description.get_or_insert(content);
            }
            "twitter:image" => {
                plain.image.get_or_insert(content);
            }
            _ => continue,
        };
    }

    // `<title>` is the last resort for a headline.
    if out.title.is_none() {
        out.title = plain.title.or_else(|| {
            let s = lower.find("<title")?;
            let open = lower[s..].find('>')? + s + 1;
            let end = lower[open..].find("</title>")? + open;
            let t = unescape(html[open..end].trim());
            (!t.is_empty()).then_some(t)
        });
    }
    if out.description.is_none() {
        out.description = plain.description;
    }
    if out.image.is_none() {
        out.image = plain.image;
    }
    // A bare domain is a reasonable site name when the page didn't say.
    if out.site_name.is_none() {
        out.site_name = host_of(url);
    }
    out
}

/// The display host for a URL — `www.` dropped, since it is noise on a card.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.split('@').next_back()?;
    let host = host.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    (!host.is_empty()).then(|| host.to_string())
}

async fn fetch(url: &str) -> Option<LinkPreview> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        // Identify honestly. Some sites serve no OG tags to unknown agents, and
        // pretending to be a browser to defeat that would be the wrong call for
        // an app whose whole premise is not phoning home behind your back.
        .user_agent(concat!("Vellum/", env!("CARGO_PKG_VERSION"), " (link preview)"))
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    // Only HTML can carry OG tags; a PDF or image would just be a wasted read.
    let is_html = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html") || v.contains("application/xhtml"));
    if !is_html {
        return None;
    }
    // Read the head only. `chunk()` lets us stop early rather than buffering a
    // whole page we don't want.
    let mut resp = resp;
    let mut body: Vec<u8> = Vec::new();
    while body.len() < MAX_BYTES {
        match resp.chunk().await {
            Ok(Some(c)) => body.extend_from_slice(&c),
            _ => break,
        }
        // Everything we need precedes </head>.
        if let Ok(s) = std::str::from_utf8(&body) {
            if s.to_ascii_lowercase().contains("</head>") {
                break;
            }
        }
    }
    body.truncate(MAX_BYTES);
    let html = String::from_utf8_lossy(&body);
    let preview = parse_metadata(&html, url);
    preview.is_useful().then_some(preview)
}

/// Fetch (or recall) the preview for an external link.
///
/// Returns `None` for anything that shouldn't or couldn't produce a card, so the
/// caller keeps rendering a plain link. Never returns `Err` for a network
/// failure — being offline is an ordinary state here, not an error.
#[tauri::command]
pub async fn fetch_link_preview(url: String) -> Result<Option<LinkPreview>, String> {
    // Only ever speak http(s). A `file:` or custom-scheme link must not be
    // dereferenced just because it appeared in a note.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Ok(None);
    }
    if let Some(hit) = cached(&url) {
        return Ok(hit);
    }
    let got = fetch(&url).await;
    remember(&url, got.clone());
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_strips_www_port_and_path() {
        assert_eq!(host_of("https://example.com/a/b?c#d").as_deref(), Some("example.com"));
        assert_eq!(host_of("https://www.example.com").as_deref(), Some("example.com"));
        assert_eq!(host_of("http://example.com:8080/x").as_deref(), Some("example.com"));
        assert_eq!(host_of("not a url"), None);
    }

    /// og:* is what sites publish for link previews, so it must win over the
    /// generic description, whichever order the tags appear in.
    #[test]
    fn open_graph_wins_over_plain_meta() {
        let html = r#"
            <html><head>
            <meta name="description" content="plain one">
            <title>Fallback Title</title>
            <meta property="og:title" content="OG Title">
            <meta property="og:description" content="OG desc">
            <meta property="og:site_name" content="Example">
            </head></html>"#;
        let p = parse_metadata(html, "https://example.com/post");
        assert_eq!(p.title.as_deref(), Some("OG Title"));
        assert_eq!(p.description.as_deref(), Some("OG desc"));
        assert_eq!(p.site_name.as_deref(), Some("Example"));
    }

    /// With no og:* tags at all, a card still needs a headline.
    #[test]
    fn falls_back_to_title_and_meta_description() {
        let html = "<html><head><title> Just a Title </title>\
                    <meta name='description' content='plain one'></head></html>";
        let p = parse_metadata(html, "https://example.com/x");
        assert_eq!(p.title.as_deref(), Some("Just a Title"));
        assert_eq!(p.description.as_deref(), Some("plain one"));
        // No og:site_name — the host stands in.
        assert_eq!(p.site_name.as_deref(), Some("example.com"));
    }

    /// Attribute parsing has to survive real-world markup: single quotes, extra
    /// whitespace, entities, and attributes whose names contain the key.
    #[test]
    fn attribute_parsing_handles_awkward_markup() {
        let html = r#"<head>
            <meta  property = 'og:title'   content = 'Sam &amp; Max: "Hit the Road"' >
            <meta data-content="decoy" property="og:description" content="real desc">
            </head>"#;
        let p = parse_metadata(html, "https://example.com");
        assert_eq!(p.title.as_deref(), Some(r#"Sam & Max: "Hit the Road""#));
        assert_eq!(p.description.as_deref(), Some("real desc"));
    }

    /// A page with nothing to say must not produce a card — an empty card is
    /// worse than the plain link it would replace.
    #[test]
    fn a_page_with_no_metadata_is_not_useful() {
        let p = parse_metadata("<html><body>hi</body></html>", "https://example.com");
        assert!(!p.is_useful(), "site_name alone must not qualify: {p:?}");
    }

    /// Entity decoding must not double-decode through the ampersand.
    #[test]
    fn unescape_does_not_double_decode() {
        assert_eq!(unescape("&amp;lt;"), "&lt;");
        assert_eq!(unescape("a &quot;b&quot; &amp; c"), r#"a "b" & c"#);
    }
}
