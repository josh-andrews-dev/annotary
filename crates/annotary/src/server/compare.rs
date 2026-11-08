//! Compare Versions: `GET /d/{id}/diff?from=A&to=B` — one server-composed page with Version B's
//! rendered HTML and what changed since Version A marked inline (`annotary_diff::rendered_diff`:
//! removed text struck through, added text highlighted, each with a text label; long unchanged
//! stretches folded). Both Versions' HTML comes from `render::ensure_html`, as on the Document
//! page; nothing is fetched after load.
//!
//! | route | right | what |
//! |---|---|---|
//! | `GET /d/{id}/diff?from=A&to=B` | web auth + `read` | A against B, oldest first (a reversed pair is shown the right way round); `from` alone = against the head, `to` alone = against the Version before it, neither = the previous Version against the head; an unknown Version is the 404 page |
//!
//! The page has no Thread cards (the Document page is where Threads live). The Document page's
//! Compare link (`view.rs`) picks the default pair; the two selects here pick any other.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use annotary_diff::RenderedDiff;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use annotary_store::{Right, VersionView};

use super::app::AppState;
use super::auth::Auth;
use super::docs::load_doc;
use super::error::ApiError;
use super::prefs::WebPrefs;
use super::render::ensure_html;
use super::ui::{self, MemberBrief, esc};

#[derive(Debug, Deserialize, Default)]
pub struct CompareQuery {
    /// Strings, so a malformed number is the HTML 404 page rather than a plain-text rejection.
    pub from: Option<String>,
    pub to: Option<String>,
}

/// `7` or `v7`.
fn version_arg(s: &str) -> Option<u32> {
    s.trim().trim_start_matches(['v', 'V']).parse().ok().filter(|n| *n >= 1)
}

/// The pair a query asks for, oldest first, given the head (see the module table).
pub fn pair_of(from: Option<u32>, to: Option<u32>, head: u32) -> (u32, u32) {
    let (a, b) = match (from, to) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, head),
        (None, Some(b)) => (b.saturating_sub(1).max(1), b),
        (None, None) => (head.saturating_sub(1).max(1), head),
    };
    (a.min(b), a.max(b))
}

/// The link the Document page's Compare button opens for Version `n` (the head: the previous
/// Version against it; an older Version: that Version against the head).
pub fn compare_href(doc_id: &str, n: u32, head: u32) -> String {
    let from = if n == head { head.saturating_sub(1).max(1) } else { n };
    format!("/d/{doc_id}/diff?from={from}&to={head}")
}

/// What makes the Version selects work, inlined ahead of them (see the file).
const SELECTS_JS: &str = include_str!("ui/compare.js");

/// Diffs running at once: each is bounded (`annotary_diff::WORK_BUDGET`), and any reader can ask.
pub const CONCURRENT_DIFFS: usize = 2;
/// How long a request waits for a turn; under `server::DRAIN_MAX`, so shutdown never waits on it.
const TURN_WAIT: Duration = Duration::from_secs(2);
/// Bytes of finished diffs kept: a repeated comparison costs a lookup.
const CACHE_BYTES: usize = 32 << 20;
/// What an entry costs beyond its HTML (its key and bookkeeping), so that even empty diffs fill
/// the cache and evict.
const ENTRY_BYTES: usize = 512;

/// Finished diffs by the two Versions' content (Markdown and rendered HTML hashes), oldest out
/// first. A viewer's own settings are applied after it, so every viewer shares an entry.
#[derive(Default)]
pub struct DiffCache {
    inner: Mutex<CacheInner>,
}

#[derive(Default)]
struct CacheInner {
    order: VecDeque<String>,
    entries: HashMap<String, Arc<RenderedDiff>>,
    bytes: usize,
}

impl DiffCache {
    /// The diff under `key`, computing and keeping it when absent (one too big to keep is returned
    /// and not kept). The lock is not held while `compute` runs.
    pub fn get_or(
        &self,
        key: &str,
        compute: impl FnOnce() -> Result<RenderedDiff, ApiError>,
    ) -> Result<Arc<RenderedDiff>, ApiError> {
        if let Some(d) = self.lock().entries.get(key) {
            return Ok(d.clone());
        }
        let d = Arc::new(compute()?);
        let size = entry_bytes(&d);
        if size <= CACHE_BYTES {
            let mut c = self.lock();
            if !c.entries.contains_key(key) {
                while c.bytes + size > CACHE_BYTES {
                    let Some(old) = c.order.pop_front() else { break };
                    if let Some(e) = c.entries.remove(&old) {
                        c.bytes -= entry_bytes(&e);
                    }
                }
                c.bytes += size;
                c.order.push_back(key.to_string());
                c.entries.insert(key.to_string(), d.clone());
            }
        }
        Ok(d)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn entry_bytes(d: &RenderedDiff) -> usize {
    d.html.len() + ENTRY_BYTES
}

/// A turn to run a diff, or the 503 a request gets when none comes within `wait`.
async fn turn(gate: Arc<Semaphore>, wait: Duration) -> Result<OwnedSemaphorePermit, ApiError> {
    match tokio::time::timeout(wait, gate.acquire_owned()).await {
        Ok(Ok(permit)) => Ok(permit),
        _ => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "the server is busy comparing other Versions; try again in a moment",
        )),
    }
}

pub async fn page(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<CompareQuery>,
) -> Response {
    let permit = match turn(st.diff_gate.clone(), TURN_WAIT).await {
        Ok(p) => p,
        Err(e) => return ui::error_page_for(&st, &auth, &headers, e),
    };
    let task = {
        let (st, auth, headers) = (st.clone(), auth.clone(), headers.clone());
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            render_page(&st, &auth, &id, &q, &headers)
        })
    };
    let result = task.await.unwrap_or_else(|e| Err(ApiError::internal(format!("the diff failed: {e}"))));
    match result {
        Ok(r) => r,
        Err(e) => ui::error_page_for(&st, &auth, &headers, e),
    }
}

fn render_page(
    st: &AppState,
    auth: &Auth,
    id: &str,
    q: &CompareQuery,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let doc = load_doc(st, id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let versions = st.store.versions(&doc.id)?;
    let head = doc.head.number;
    let arg = |s: &Option<String>, what: &str| -> Result<Option<u32>, ApiError> {
        match s.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(None),
            Some(s) => version_arg(s)
                .map(Some)
                .ok_or_else(|| ApiError::not_found(format!("Document {} has no Version {what}={s}", doc.id))),
        }
    };
    let (a, b) = pair_of(arg(&q.from, "from")?, arg(&q.to, "to")?, head);
    let find = |n: u32| {
        versions
            .iter()
            .find(|x| x.number == n)
            .ok_or_else(|| ApiError::not_found(format!("Document {} has no v{n}", doc.id)))
    };
    let prefs = WebPrefs::load(st, auth, headers)?;
    let members = ui::members_by_handle(st)?;
    let member = |h: &str| members.get(h).cloned().unwrap_or_else(|| MemberBrief::unknown(h));
    let name = &doc.head.name;

    let only_one = versions.len() < 2;
    let (va, vb) = (find(a)?, find(b)?);

    // ---- the diff ----
    let (body, summary) = if only_one {
        (
            format!("<p class=\"cd-dx-note\" id=\"cd-dxnote\">Nothing to compare: {} has one Version.</p>", esc(name)),
            String::new(),
        )
    } else if a == b {
        (
            format!(
                "<p class=\"cd-dx-note\" id=\"cd-dxnote\">v{a} and v{b} are the same Version. Pick another one to compare.</p>"
            ),
            String::new(),
        )
    } else {
        let md = |v: &VersionView| -> Result<String, ApiError> {
            let bytes = st.store.read_blob(&v.md_hash)?.unwrap_or_default();
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        };
        let (html_a, hash_a, _) = ensure_html(st, &auth.0.member_id, va)?;
        let (html_b, hash_b, _) = ensure_html(st, &auth.0.member_id, vb)?;
        let key = format!("{}:{}:{hash_a}:{hash_b}", va.md_hash, vb.md_hash);
        let d = st.diffs.get_or(&key, || {
            let text = |h: &[u8]| String::from_utf8_lossy(h).into_owned();
            Ok(annotary_diff::rendered_diff(&md(va)?, &md(vb)?, &text(&html_a), &text(&html_b)))
        })?;
        let mut out = ui::asset_links_blank(&d.html);
        if prefs.embed {
            // M3: media go through the embedder (`embed.rs`); a removed image is the older Version's.
            let mut sizes: HashMap<String, u64> = super::embed::sizes(&va.manifest);
            sizes.extend(super::embed::sizes(&vb.manifest));
            out = super::embed::media(&out, &sizes);
        }
        let same = va.md_hash == vb.md_hash || vb.same_as == Some(a) || va.same_as == Some(b);
        let note = notes(&d.stats, same, a, b);
        (format!("{note}{out}"), summary_html(&d.stats))
    };

    // ---- the header ----
    let select = |id: &str, field: &str, label: &str, chosen: u32| {
        let mut s = format!("<select id=\"{id}\" name=\"{field}\" aria-label=\"{label}\">");
        for x in versions.iter().rev() {
            let pre = format!(
                "v{}{} · {} · ",
                x.number,
                if x.number == head { " (head)" } else { "" },
                member(&x.by_handle).name
            );
            let post = x.same_as.map(|k| format!(" · same as v{k}")).unwrap_or_default();
            s.push_str(&format!(
                "<option value=\"{n}\" data-at=\"{at}\" data-pre=\"{pre}\" data-post=\"{post}\"{sel}>{pre}{when}{post}</option>",
                n = x.number,
                at = esc(&x.at),
                pre = esc(&pre),
                post = esc(&post),
                sel = if x.number == chosen { " selected" } else { "" },
                when = esc(&ui::fallback_time(&x.at)),
            ));
        }
        s.push_str("</select>");
        s
    };
    let href = |n: u32| if n == head { format!("/d/{}", doc.id) } else { format!("/d/{}/v/{n}", doc.id) };
    let form = if only_one {
        String::new()
    } else {
        format!(
            "<form class=\"cd-dxform\" id=\"cd-dxform\" method=\"get\" action=\"/d/{id}/diff\">\
               <span class=\"cd-dxpick\">{from}<span class=\"cd-dxarrow\" aria-hidden=\"true\">\u{2192}</span>{to}</span>\
               <noscript><button type=\"submit\" class=\"cd-btn\">Compare</button></noscript>\
             </form>",
            id = esc(&doc.id),
            from = select("cd-dxfrom", "from", "Compare from Version", a),
            to = select("cd-dxto", "to", "Compare to Version", b),
        )
    };
    let opens = if only_one {
        format!("<a class=\"cd-btn\" id=\"cd-dxopen-to\" href=\"{}\">Back to v{head}</a>", href(head))
    } else {
        format!(
            "<a class=\"cd-btn\" id=\"cd-dxopen-from\" href=\"{}\" title=\"{}\">Open v{a}</a>\
             <a class=\"cd-btn\" id=\"cd-dxopen-to\" href=\"{}\" title=\"{}\">Open v{b}</a>",
            href(a),
            esc(&version_title(va, &member(&va.by_handle))),
            href(b),
            esc(&version_title(vb, &member(&vb.by_handle))),
        )
    };
    let header = format!(
        "<header class=\"cd-hdr cd-dxhdr\">\
           <div class=\"cd-line cd-dxline\">{icon}<a class=\"cd-crumb\" href=\"/\">{vault}</a><span class=\"cd-crumb\">/</span>\
             <a class=\"cd-name cd-dxname\" href=\"{doc_href}\">{name}</a><code class=\"cd-docid\">{id}</code></div>\
           <div class=\"cd-dxbar\">{form}{summary}</div>\
         </header>\
         <div class=\"cd-dxinfo\">{legend}<span class=\"cd-grow\"></span><span class=\"cd-dxopens\">{opens}</span></div>",
        icon = super::brand::header_icon(),
        vault = esc(&doc.vault_name),
        doc_href = href(head),
        name = esc(name),
        id = esc(&doc.id),
        legend = if summary.is_empty() { "" } else { LEGEND },
    );
    let page = format!(
        "<script>{SELECTS_JS}</script><div {} id=\"cd-app\" data-doc=\"{}\" data-from=\"{a}\" data-to=\"{b}\" data-head=\"{head}\">{header}\
         <main class=\"cd-dxmain\"><article class=\"cd-doc cd-dxdoc\" id=\"cd-dxdoc\">{body}</article></main></div>",
        prefs.app_attrs("cd-diffpage"),
        esc(&doc.id),
    );
    let title = if only_one { format!("{name} — annotary") } else { format!("{name} v{a} → v{b} — annotary") };
    Ok(ui::page_for(&prefs, &title, &page))
}

/// What the page says above the diff, when it has something to say.
fn notes(s: &annotary_diff::DiffStats, same: bool, a: u32, b: u32) -> String {
    let note = |id: &str, text: &str| format!("<p class=\"cd-dx-note\" id=\"{id}\">{text}</p>");
    if s.too_large {
        return note("cd-dxnote", "These Versions are too large to compare. Open either one to read it.");
    }
    let mut out = String::new();
    if s.changes == 0 {
        out = if same {
            note("cd-dxnote", &format!("v{b} has the same content as v{a}."))
        } else {
            note("cd-dxnote", "Nothing visible changed (whitespace, front matter or Assets the text does not show).")
        };
    }
    if s.shown_whole {
        out.push_str(&note(
            "cd-dxwhole",
            "These Versions differ too much to compare everything word by word, so some blocks are shown whole.",
        ));
    }
    out
}

/// The key to the marks (the marks themselves carry text labels for screen readers).
const LEGEND: &str = "<span class=\"cd-dxkey\" aria-hidden=\"true\"><span class=\"cd-dxkey-del\">removed</span><span class=\"cd-dxkey-add\">added</span><span class=\"cd-dxkey-mod\">changed block</span></span>";

/// `+N −M lines · K changes` and the change stepper.
fn summary_html(s: &annotary_diff::DiffStats) -> String {
    let changes = match s.changes {
        0 => "no changes".to_string(),
        1 => "1 change".to_string(),
        n => format!("{n} changes"),
    };
    let blocks = [(s.blocks_added, "added"), (s.blocks_removed, "removed"), (s.blocks_changed, "changed")]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, w)| format!("{n} {w}"))
        .collect::<Vec<_>>()
        .join(", ");
    let title = if blocks.is_empty() { String::new() } else { format!(" title=\"blocks: {blocks}\"") };
    let nav = if s.changes > 0 {
        "<span class=\"cd-dxnav\"><button type=\"button\" class=\"cd-btn cd-sm\" id=\"cd-dxprev\" disabled aria-label=\"Previous change\" title=\"Previous change (k)\">\u{2039}</button>\
         <button type=\"button\" class=\"cd-btn cd-sm\" id=\"cd-dxnext\" disabled aria-label=\"Next change\" title=\"Next change (j)\">\u{203A}</button></span>"
    } else {
        ""
    };
    format!(
        "<span class=\"cd-dxsum\" id=\"cd-dxsum\" data-changes=\"{c}\"><span class=\"cd-dxlines\"><b class=\"cd-dxplus\">+{add}</b> <b class=\"cd-dxminus\">\u{2212}{del}</b> lines</span>\
         <span class=\"cd-dxpos\" id=\"cd-dxpos\" aria-live=\"polite\"{title}>{changes}</span>{nav}</span>",
        c = s.changes,
        add = s.lines_added,
        del = s.lines_removed,
    )
}

/// `v3 · Rowan Vale · 2026-03-05 12:04` (a link's title: plain text).
fn version_title(v: &VersionView, by: &MemberBrief) -> String {
    let msg = v.message.as_deref().map(|m| format!(" · \u{201C}{m}\u{201D}")).unwrap_or_default();
    format!("v{} · {} · {}{msg}", v.number, by.name, ui::fallback_time(&v.at))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_pairs() {
        assert_eq!(pair_of(None, None, 7), (6, 7));
        assert_eq!(pair_of(Some(2), None, 7), (2, 7));
        assert_eq!(pair_of(None, Some(4), 7), (3, 4));
        assert_eq!(pair_of(Some(7), Some(2), 7), (2, 7), "oldest first");
        assert_eq!(pair_of(None, None, 1), (1, 1));
        assert_eq!(compare_href("d1", 7, 7), "/d/d1/diff?from=6&to=7");
        assert_eq!(compare_href("d1", 2, 7), "/d/d1/diff?from=2&to=7");
        assert_eq!(version_arg("v3"), Some(3));
        assert_eq!(version_arg("0"), None);
        assert_eq!(version_arg("x"), None);
    }

    // spec: S40-09
    #[test]
    fn what_the_page_says_above_the_diff() {
        use annotary_diff::DiffStats;
        let d = DiffStats { too_large: true, lines_added: 3, ..DiffStats::default() };
        let n = notes(&d, false, 1, 2);
        assert!(n.contains("too large to compare") && !n.contains("Nothing visible"), "{n}");
        let n = notes(&DiffStats::default(), true, 1, 2);
        assert!(n.contains("v2 has the same content as v1.") && !n.contains("cd-dxwhole"), "{n}");
        let n = notes(&DiffStats::default(), false, 1, 2);
        assert!(n.contains("Nothing visible changed"), "{n}");
        let d = DiffStats { changes: 4, shown_whole: true, ..DiffStats::default() };
        let n = notes(&d, false, 1, 2);
        assert!(n.contains("id=\"cd-dxwhole\"") && !n.contains("cd-dxnote"), "{n}");
        assert_eq!(notes(&DiffStats { changes: 1, ..DiffStats::default() }, false, 1, 2), "");
    }

    fn diff_of(html: String) -> RenderedDiff {
        RenderedDiff { html, stats: annotary_diff::DiffStats::default(), work: 0 }
    }

    // spec: S40-12
    #[test]
    fn a_repeated_comparison_is_a_lookup_and_the_cache_keeps_to_its_bytes() {
        let cache = DiffCache::default();
        let calls = std::cell::Cell::new(0);
        let make = |html: String| {
            calls.set(calls.get() + 1);
            Ok(diff_of(html))
        };
        let a = cache.get_or("k1", || make("<p>one</p>".into())).unwrap();
        let b = cache.get_or("k1", || make("<p>other</p>".into())).unwrap();
        assert_eq!((calls.get(), a.html.as_str(), Arc::ptr_eq(&a, &b)), (1, "<p>one</p>", true));
        // Too big to keep: returned, computed again next time.
        let big = "x".repeat(CACHE_BYTES + 1);
        assert_eq!(cache.get_or("big", || make(big.clone())).unwrap().html.len(), big.len());
        cache.get_or("big", || make(big.clone())).unwrap();
        assert_eq!(calls.get(), 3);
        // Full: the oldest goes first, and the total stays under the limit.
        let half = "y".repeat(CACHE_BYTES / 2 + 1);
        cache.get_or("h1", || make(half.clone())).unwrap();
        cache.get_or("h2", || make(half.clone())).unwrap();
        let c = cache.lock();
        assert!(c.bytes <= CACHE_BYTES, "{}", c.bytes);
        assert!(!c.entries.contains_key("k1") && !c.entries.contains_key("h1") && c.entries.contains_key("h2"));
    }

    // spec: S40-12
    #[test]
    fn empty_diffs_fill_the_cache_too() {
        let cache = DiffCache::default();
        let limit = CACHE_BYTES / ENTRY_BYTES;
        for i in 0..limit + 10 {
            cache.get_or(&format!("k{i}"), || Ok(diff_of(String::new()))).unwrap();
        }
        let c = cache.lock();
        assert_eq!(c.entries.len(), limit);
        assert_eq!(c.order.len(), limit);
        assert_eq!(c.bytes, limit * ENTRY_BYTES);
        assert!(!c.entries.contains_key("k0") && c.entries.contains_key(&format!("k{}", limit + 9)));
        drop(c);
        // The floor counts against the limit: an entry that fills it exactly is kept, one more byte is not.
        let cache = DiffCache::default();
        cache.get_or("fits", || Ok(diff_of("x".repeat(CACHE_BYTES - ENTRY_BYTES)))).unwrap();
        cache.get_or("over", || Ok(diff_of("x".repeat(CACHE_BYTES - ENTRY_BYTES + 1)))).unwrap();
        let c = cache.lock();
        assert!(c.entries.contains_key("fits") && !c.entries.contains_key("over"));
    }

    // spec: S40-12
    #[tokio::test]
    async fn a_request_with_no_turn_is_a_503() {
        let gate = Arc::new(Semaphore::new(CONCURRENT_DIFFS));
        let first = turn(gate.clone(), Duration::from_millis(10)).await.unwrap();
        let _second = turn(gate.clone(), Duration::from_millis(10)).await.unwrap();
        let e = turn(gate.clone(), Duration::from_millis(10)).await.unwrap_err();
        assert_eq!((e.status, e.code), (StatusCode::SERVICE_UNAVAILABLE, "busy"));
        drop(first);
        assert!(turn(gate, Duration::from_millis(10)).await.is_ok(), "a finished diff frees its turn");
    }
}
