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

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;

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

pub async fn page(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<CompareQuery>,
) -> Response {
    match render_page(&st, &auth, &id, &q, &headers) {
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
    let (va, vb) = if only_one { (find(head)?, find(head)?) } else { (find(a)?, find(b)?) };

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
        let html = |v: &VersionView| -> Result<String, ApiError> {
            let (h, _, _) = ensure_html(st, &auth.0.member_id, v)?;
            Ok(String::from_utf8_lossy(&h).into_owned())
        };
        let d = annotary_diff::rendered_diff(&md(va)?, &md(vb)?, &html(va)?, &html(vb)?);
        let mut out = ui::asset_links_blank(&d.html);
        if prefs.embed {
            // M3: media go through the embedder (`embed.rs`); a removed image is the older Version's.
            let mut sizes: HashMap<String, u64> = super::embed::sizes(&va.manifest);
            sizes.extend(super::embed::sizes(&vb.manifest));
            out = super::embed::media(&out, &sizes);
        }
        let same = va.md_hash == vb.md_hash || vb.same_as == Some(a) || va.same_as == Some(b);
        let note = if d.stats.changes > 0 {
            String::new()
        } else if same {
            format!("<p class=\"cd-dx-note\" id=\"cd-dxnote\">v{b} has the same content as v{a}.</p>")
        } else {
            "<p class=\"cd-dx-note\" id=\"cd-dxnote\">Nothing visible changed (whitespace, front matter or Assets the text does not show).</p>".to_string()
        };
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
        "<div {} id=\"cd-app\" data-doc=\"{}\" data-from=\"{a}\" data-to=\"{b}\" data-head=\"{head}\">{header}\
         <main class=\"cd-dxmain\"><article class=\"cd-doc cd-dxdoc\" id=\"cd-dxdoc\">{body}</article></main></div>",
        prefs.app_attrs("cd-diffpage"),
        esc(&doc.id),
    );
    let title = if only_one { format!("{name} — annotary") } else { format!("{name} v{a} → v{b} — annotary") };
    Ok(ui::page_for(&prefs, &title, &page))
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
        "<span class=\"cd-dxnav\"><button type=\"button\" class=\"cd-btn cd-sm\" id=\"cd-dxprev\" aria-label=\"Previous change\" title=\"Previous change (k)\">\u{2039}</button>\
         <button type=\"button\" class=\"cd-btn cd-sm\" id=\"cd-dxnext\" aria-label=\"Next change\" title=\"Next change (j)\">\u{203A}</button></span>"
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
}
