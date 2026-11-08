//! Embed mode (M3 of the Hermes plan; `docs/dev/web-document-view.md`, "Embed mode"): the Document
//! list and the Document view rendered for an **embedder**, a host that shows them in a sandboxed
//! frame with an opaque origin (no cookies, no storage, nothing it can load by URL) and carries
//! every request, navigation and event over `postMessage` to a backend that calls this server
//! with a Bearer Token.
//!
//! - A request is embedded when it carries `X-Annotary-Embed: 1` ([`is_embed`]). A browser cannot
//!   add a header to a navigation or a frame, and a cross-site `fetch` with it needs a CORS
//!   preflight this server never grants, so no site can get an embed render in someone's browser;
//!   a query flag could be put in an `<iframe src>` (`docs/decisions.md`, 2026-11-07).
//! - `GET /embed/shell` ([`shell`]) is the page the embedder turns into a `data:` URL and loads
//!   into its frame: `ui/embed.js` (the transport) and a boot call. It carries
//!   `Content-Security-Policy: sandbox allow-scripts`, so a browser that loads it from this origin
//!   runs it in an opaque origin too (the shell writes whatever its parent sends into itself).
//! - The page wears `.cd-app.cd-embed`; `ui::page_for` inlines `ui/embed.js` before `app.js`,
//!   leaves out the icon links, and adds the same sandbox CSP and `Vary: X-Annotary-Embed`. The
//!   list's top bar links Documents only (no Settings, no Sign out); the Document page has no
//!   "Open in Annotary".
//! - Preferences a browser keeps in cookies arrive as headers: the zone `X-Annotary-Tz` and the
//!   hour cycle `X-Annotary-Hc` (`prefs.rs`). A cookie is never read on an embed request.
//! - Media go through the embedder ([`media`]): an image's `src` becomes `data-cd-src` (the script
//!   fetches it, up to [`IMAGE_MAX`]); video, audio and PDF lose theirs and get an "open" button
//!   (`button.cd-embed-open[data-cd-open]`, an `annotary-open-external` message); so does a larger
//!   image.
//! - No message carries more than about [`PART_MAX`] of body ([`parts`]): a larger embedded GET
//!   answers 206 with the first part, and `embed.js` asks for the rest by number.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use serde_json::Value;

use super::auth::Auth;

/// The request header that asks for an embed render (value `1`).
pub const HEADER: &str = "x-annotary-embed";
/// The embedder's IANA time zone (what the `cd_tz` cookie carries in a browser).
pub const TZ_HEADER: &str = "x-annotary-tz";
/// The embedder's hour cycle, `12` or `24` (what the `cd_hc` cookie carries in a browser).
pub const HC_HEADER: &str = "x-annotary-hc";
/// Which part of a large answer an embedded GET asks for (`1`, `2`, …; none = the first).
pub const PART_HEADER: &str = "x-annotary-part";
/// The answer's identity, sent with every part, so parts of two different answers never mix.
pub const ETAG_HEADER: &str = "x-annotary-etag";
/// How many parts the answer has.
pub const PARTS_HEADER: &str = "x-annotary-parts";
/// The answer's own status, on a 206 part.
pub const STATUS_HEADER: &str = "x-annotary-status";
/// The version of the message protocol between the page, the shell and the embedder.
pub const PROTOCOL: u32 = 1;
/// The largest image (bytes, from the Version's manifest) an embedded page fetches through the
/// embedder; a larger one is an "open" button. The one place the cap lives.
pub const IMAGE_MAX: u64 = 2 * 1024 * 1024;
/// The most body bytes one embedded answer carries ([`parts`]): Hermes's gateway stalls on
/// messages over about 1 MB, and a body can grow on the way (JSON escaping, `\u` for non-ASCII,
/// base64 for bytes), so a part stays well under it.
pub const PART_MAX: usize = 384 * 1024;
/// The transport both the shell and every embed render inline (`embed.js` has the protocol).
pub const JS: &str = include_str!("ui/embed.js");

/// True when the request asks for an embed render.
pub fn is_embed(headers: &HeaderMap) -> bool {
    headers.get(HEADER).and_then(|v| v.to_str().ok()).map(str::trim) == Some("1")
}

/// A header's value, trimmed (`None` when absent or not text).
pub fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(|s| s.trim().to_string())
}

/// The sandbox the embed pages and the shell carry: whoever loads them by URL gets an opaque origin.
pub const SANDBOX_CSP: &str = "sandbox allow-scripts";

/// `GET /embed/shell` (any Member): the bootstrap page an embedder loads into its frame as a
/// `data:` URL (optionally with the first path in its fragment, `#%2Fd%2FID`). It holds the
/// transport and asks the embedder for the page (`annotary-ready`, then `annotary-init`).
pub async fn shell(_auth: Auth) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"annotary-embed-protocol\" content=\"{PROTOCOL}\"><title>annotary</title>\
         <style>body{{margin:0;font:15px/1.5 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;color:#6b6f76}}\
         @media (prefers-color-scheme:dark){{body{{background:#000;color:#98989f}}}}</style>\
         <script>{JS}</script></head><body><script>window.AnnotaryEmbed&&window.AnnotaryEmbed.boot()</script></body></html>\n"
    );
    let mut resp = axum::response::Html(html).into_response();
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(SANDBOX_CSP));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    resp
}

/// Middleware: an embedded GET whose body is larger than [`PART_MAX`] answers `206` with one part
/// (`X-Annotary-Part: N` asks for part N, from 0; none = 0), `X-Annotary-Parts`, the answer's
/// `X-Annotary-Etag` and its own status in `X-Annotary-Status`; the content type is kept. Parts end
/// on character boundaries when the body is UTF-8 text, so an embedder that passes text on as text
/// never splits a character. A later part asked with another etag (the answer changed between
/// parts) is `412`; `embed.js` starts over. Streams (`text/event-stream`) pass untouched.
pub async fn parts(req: Request, next: Next) -> Response {
    if req.method() != Method::GET || !is_embed(req.headers()) {
        return next.run(req).await;
    }
    let part = header(req.headers(), PART_HEADER).and_then(|p| p.parse::<usize>().ok());
    let want = header(req.headers(), ETAG_HEADER);
    let resp = next.run(req).await;
    let streaming = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/event-stream"));
    if streaming {
        return resp;
    }
    let (mut head, body) = resp.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if part.unwrap_or(0) == 0 && bytes.len() <= PART_MAX {
        return Response::from_parts(head, Body::from(bytes));
    }
    let cuts = cuts(&bytes, PART_MAX);
    let etag = annotary_sidecar::identity::sha256_hex(&bytes)[..20].to_string();
    let i = part.unwrap_or(0);
    if i >= cuts.len() || (i > 0 && want.as_deref() != Some(etag.as_str())) {
        let mut r = (StatusCode::PRECONDITION_FAILED, "the answer changed; ask from part 0").into_response();
        r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return r;
    }
    let (from, to) = cuts[i];
    let status = head.status;
    head.status = StatusCode::PARTIAL_CONTENT;
    head.headers.remove(header::CONTENT_LENGTH);
    head.headers.remove(header::ETAG);
    let set = |h: &mut HeaderMap, k: &'static str, v: String| {
        if let Ok(v) = HeaderValue::from_str(&v) {
            h.insert(HeaderName::from_static(k), v);
        }
    };
    set(&mut head.headers, PARTS_HEADER, cuts.len().to_string());
    set(&mut head.headers, ETAG_HEADER, etag);
    set(&mut head.headers, STATUS_HEADER, status.as_u16().to_string());
    set(&mut head.headers, PART_HEADER, i.to_string());
    head.headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Response::from_parts(head, Body::from(bytes.slice(from..to)))
}

/// The `(from, to)` byte ranges of each part: at most `max` bytes, ending on a character boundary
/// when `bytes` is UTF-8.
pub fn cuts(bytes: &[u8], max: usize) -> Vec<(usize, usize)> {
    let text = std::str::from_utf8(bytes).ok();
    let mut out = Vec::new();
    let mut from = 0;
    while from < bytes.len() {
        let mut to = (from + max).min(bytes.len());
        if let Some(t) = text {
            while to > from + 1 && !t.is_char_boundary(to) {
                to -= 1;
            }
            // `max` is smaller than this character: the part takes the whole character
            while !t.is_char_boundary(to) {
                to += 1;
            }
        }
        out.push((from, to));
        from = to;
    }
    if out.is_empty() {
        out.push((0, 0));
    }
    out
}

/// Asset sizes by hash, from a Version's manifest (`[{hash, size, …}]`).
pub fn sizes(manifest: &Value) -> HashMap<String, u64> {
    manifest
        .as_array()
        .map(|m| {
            m.iter().filter_map(|e| Some((e["hash"].as_str()?.to_string(), e["size"].as_u64().unwrap_or(0)))).collect()
        })
        .unwrap_or_default()
}

/// The Document HTML of an embed render: every `<img>`, `<video>`, `<audio>` and `<object>` that
/// points at an Asset (`/a/…`) loses that URL (no request leaves the frame by itself) and keeps it
/// as `data-cd-src`; an "open" button follows each one (hidden after an image the script fetches;
/// shown after a larger image, a video, an audio clip or a PDF). The button has no text node
/// (`app.css` draws its label), so the text the selection mapping reads is unchanged.
pub fn media(html: &str, sizes: &HashMap<String, u64>) -> String {
    let mut out = String::with_capacity(html.len() + 256);
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some((tag, attr, kind)) = media_tag(rest) else {
            out.push('<');
            rest = &rest[1..];
            continue;
        };
        let Some(end) = tag_end(rest) else {
            out.push_str(rest);
            return out;
        };
        let open = &rest[..end];
        let needle = format!(" {attr}=\"/a/");
        let Some(p) = open.find(&needle) else {
            out.push_str(open);
            rest = &rest[end..];
            continue;
        };
        let value_at = p + needle.len() - 3; // at the `/a/`
        let value_end = open[value_at..].find('"').map(|q| value_at + q).unwrap_or(open.len());
        let path = &open[value_at..value_end];
        let big = tag == "img" && {
            let hash = path.trim_start_matches("/a/").split('?').next().unwrap_or("");
            sizes.get(hash).is_some_and(|s| *s > IMAGE_MAX)
        };
        let mut tag_html = format!("{} data-cd-src=\"{}", &open[..p], &open[value_at..]);
        if big {
            // `data-cd-big`: the script leaves it alone and the button is the way to it.
            let close = tag_html.len() - 1 - usize::from(tag_html.ends_with("/>"));
            tag_html.insert_str(close, " data-cd-big=\"1\"");
        }
        out.push_str(&tag_html);
        rest = &rest[end..];
        // Hidden after an image the script fetches (it shows the button if the fetch fails).
        let hidden = tag == "img" && !big;
        // A video / audio / object: the button goes after its closing tag (its fallback link stays).
        if tag != "img" {
            let closing = format!("</{tag}>");
            if let Some(c) = rest.find(&closing) {
                out.push_str(&rest[..c + closing.len()]);
                rest = &rest[c + closing.len()..];
            }
        }
        out.push_str(&format!(
            "<button type=\"button\" class=\"cd-embed-open\" data-cd-open=\"{path}\" data-cd-kind=\"{kind}\" aria-label=\"Open {kind}\" title=\"Open {kind}\"{}></button>",
            if hidden { " hidden" } else { "" }
        ));
    }
    out.push_str(rest);
    out
}

/// `(tag, URL attribute, label)` when `s` opens a media element.
fn media_tag(s: &str) -> Option<(&'static str, &'static str, &'static str)> {
    for (tag, attr, kind) in
        [("img", "src", "image"), ("video", "src", "video"), ("audio", "src", "audio"), ("object", "data", "PDF")]
    {
        let Some(after) = s.strip_prefix('<').and_then(|r| r.strip_prefix(tag)) else { continue };
        if after.starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/') {
            return Some((tag, attr, kind));
        }
    }
    None
}

/// The byte index just past the `>` that closes the tag `s` opens (quotes respected).
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, b) in s.bytes().enumerate() {
        match (quote, b) {
            (Some(q), _) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'>') => return Some(i + 1),
            _ => {}
        }
    }
    None
}
