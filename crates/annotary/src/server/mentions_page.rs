//! The Mentions page: `GET /mentions` — every open Thread that mentions the viewer, across the
//! Documents they can read, newest activity first; each row opens the Document at that Thread
//! (`/d/{id}#tN`). The hits are `GET /api/v1/mentions`'s (`Store::mentions_for` over the same
//! readable Vaults, newest activity first), so the page and the API cannot disagree. Web auth via
//! the `web_pages` router, like the list.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;

use annotary_store::{MentionHit, ThreadSubject};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use super::prefs::WebPrefs;
use super::ui::{self, esc};
use super::vaults::{ProfileVaultHeader, scoped_vault_ids};

pub async fn index(State(st): State<AppState>, auth: Auth, headers: HeaderMap) -> Response {
    match render(&st, &auth, &headers) {
        Ok(r) => r,
        Err(e) => ui::error_page_for(&st, &auth, &headers, e),
    }
}

/// The open Threads mentioning the viewer, newest activity first (`Store::mentions_for`'s order).
pub fn hits(st: &AppState, auth: &Auth) -> Result<Vec<MentionHit>, ApiError> {
    let (vault_ids, _) = scoped_vault_ids(st, auth, &ProfileVaultHeader::none(), None)?;
    Ok(st.store.mentions_for(&auth.0.member_id, Some(&vault_ids), None)?)
}

fn label(h: &MentionHit) -> String {
    let t = &h.thread;
    let a = t.anchor.as_ref();
    match t.subject {
        ThreadSubject::Document => "<span class=\"cd-kind\">whole document</span>".to_string(),
        ThreadSubject::Asset => format!(
            "<span class=\"cd-kind\">asset</span><q>{}</q>",
            esc(a.and_then(|a| a.original.reference.as_deref()).unwrap_or(""))
        ),
        ThreadSubject::Span => format!("<q>{}</q>", esc(a.and_then(|a| a.original.exact.as_deref()).unwrap_or(""))),
    }
}

fn render(st: &AppState, auth: &Auth, headers: &HeaderMap) -> Result<Response, ApiError> {
    let prefs = WebPrefs::load(st, auth, headers)?;
    let times = prefs.times();
    let members = ui::members_by_handle(st)?;
    let name_of = |h: &str| members.get(h).map(|m| m.name.clone()).unwrap_or_else(|| format!("@{h}"));
    let hits = hits(st, auth)?;
    let header = ui::top_nav(st, auth, "/mentions", prefs.embed)?;
    let mut rows = String::new();
    for h in &hits {
        let t = &h.thread;
        let last = t.replies.last().unwrap_or(&t.root);
        let said = if last.is_deleted() { "(deleted)".to_string() } else { super::view::snippet(&last.body) };
        let replies = t.replies.len();
        let replies = match replies {
            0 => String::new(),
            1 => " · 1 reply".to_string(),
            n => format!(" · {n} replies"),
        };
        rows.push_str(&format!(
            "<li class=\"cd-mrow\" data-doc=\"{id}\" data-t=\"{tid}\"><a class=\"cd-mlink\" href=\"/d/{id}#{tid}\">\
               <div class=\"cd-mhead\"><b class=\"cd-docname\">{name}</b><span class=\"cd-muted\">{vault} · {tid}{replies} · {at}</span></div>\
               <div class=\"cd-quote\">{label}</div>\
               <div class=\"cd-msaid\"><b>{who}</b> {said}</div></a></li>",
            id = esc(&h.doc.id),
            tid = esc(&t.id),
            name = esc(&h.doc.head.name),
            vault = esc(&h.doc.vault_name),
            at = times.tag(&t.updated_at, true),
            label = label(h),
            who = esc(&name_of(&last.author_handle)),
            said = esc(&said),
        ));
    }
    let list = if hits.is_empty() {
        "<div class=\"cd-empty\" id=\"cd-empty\">No open Threads mention you.</div>".to_string()
    } else {
        format!("<ul class=\"cd-mentions\" id=\"cd-mentions\">{rows}</ul>")
    };
    let n = hits.len();
    let body = format!(
        "<div {} id=\"cd-app\">{header}<main class=\"cd-listmain\"><h1 class=\"cd-pageh\">Mentions</h1>\
         <p class=\"cd-small cd-muted\">Open Threads that mention @{}, newest first.</p>{list}\
         <p class=\"cd-listfoot\" id=\"cd-listfoot\">{n} {}</p></main></div>",
        prefs.app_attrs("cd-mentionspage"),
        esc(&auth.0.handle),
        if n == 1 { "Thread" } else { "Threads" }
    );
    Ok(ui::page_for(&prefs, "Mentions — annotary", &body))
}
