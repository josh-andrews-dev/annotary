//! Hooks: a Member's outbound webhooks and the deliverer that calls them (docs/dev/hooks.md).
//!
//! | route | who | what |
//! |---|---|---|
//! | `POST /api/v1/me/hooks {url, on, doc?, once?, expires?, secret?}` | any Member, for itself | add a hook → `{hook, secret}` (the secret only here; `secret` given = the receiver's own) |
//! | `GET /api/v1/me/hooks` | any Member | `{hooks[]}`, its own |
//! | `DELETE /api/v1/me/hooks/{id}` | its Member, or an owner | remove → `{removed}` |
//! | `POST /api/v1/me/hooks/{id}/test` | its Member | POST one signed `kind: "test"` body now → `{status, ok}` |
//! | `GET /api/v1/hooks` | owner | `{hooks[]}`, everyone's |
//!
//! A hook is shown by `host` and `url_fp` (the URL's `sha256[:10]`), never the URL: it holds the
//! receiver's own token. Addresses (D4): `https` to public addresses only; a host in
//! `ANNOTARY_HOOK_ALLOW_HOSTS` (comma list) may be private and may use `http`. The check runs at
//! registration and again inside every connection's name lookup ([`GuardedResolver`]), so a
//! name that later resolves inward is refused (DNS rebinding). No proxy, no redirects.
//! `ANNOTARY_HOOK_ONLY_HOSTS` (comma list) is a hard limit on top: no other host is ever called.
//!
//! The deliverer ([`deliverer`]) parks on the commit hook and a 60 s tick. Per live hook it reads
//! the events after the hook's cursor with that Member's rights as they are now (CMD-8), skips
//! the Member's own events, and POSTs one body-free `v: 1` JSON event signed with
//! `X-Annotary-Signature: sha256=<hex HMAC>`. 2xx = delivered (a `once` hook is then removed);
//! 404 / 410 = the receiver is gone (disabled now); anything else retries with backoff until
//! the hook has failed for 24 h, then it is disabled. Each POST runs on its own thread with a
//! 10 s timeout, and the deliverer stops at `st.shutdown` without waiting for it.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::hooks::{self, HookAttempt, HookRow, NewHook, parse_hook_id};
use annotary_store::{AuthVia, EventQuery, EventRow, time};

use super::app::AppState;
use super::auth::Auth;
use super::docs::load_doc;
use super::error::ApiError;
use super::events::{Filter, Kind};
use super::vaults::readable_vaults;

pub const ALLOW_HOSTS_ENV: &str = "ANNOTARY_HOOK_ALLOW_HOSTS";
/// When set (comma list), no host outside it is ever called: `make verify-live` sets it on its
/// copy of the live data, whose hooks would otherwise wake real agents.
pub const ONLY_HOSTS_ENV: &str = "ANNOTARY_HOOK_ONLY_HOSTS";
/// Test seams: the first retry delay (ms) and how long a hook may fail before it is disabled (s).
pub const BACKOFF_ENV: &str = "ANNOTARY_HOOK_BACKOFF_MS";
pub const RETRY_FOR_ENV: &str = "ANNOTARY_HOOK_RETRY_SECS";
/// Longest one delivery may take.
pub const POST_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest URL accepted.
pub const MAX_URL_LEN: usize = 2048;
/// Events read per hook per pass.
const PAGE: usize = 200;
/// The deliverer's idle tick (expiry, retries that came due).
const TICK: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(3600);

/// How hooks behave on this server (from the environment; tests build their own).
#[derive(Clone, Debug)]
pub struct HookConfig {
    /// Lowercased host names allowed to be private (and `http`).
    pub allow_hosts: Vec<String>,
    /// Lowercased host names that are the only ones ever called (None = no such limit).
    pub only_hosts: Option<Vec<String>>,
    pub backoff_base: Duration,
    pub retry_for: Duration,
}

impl Default for HookConfig {
    fn default() -> Self {
        HookConfig {
            allow_hosts: Vec::new(),
            only_hosts: None,
            backoff_base: Duration::from_secs(5),
            retry_for: Duration::from_secs(24 * 3600),
        }
    }
}

impl HookConfig {
    pub fn from_env() -> HookConfig {
        let mut c = HookConfig::default();
        let hosts = |v: String| -> Vec<String> {
            v.split(',').map(|h| h.trim().to_ascii_lowercase()).filter(|h| !h.is_empty()).collect()
        };
        if let Ok(v) = std::env::var(ALLOW_HOSTS_ENV) {
            c.allow_hosts = hosts(v);
        }
        if let Ok(v) = std::env::var(ONLY_HOSTS_ENV) {
            c.only_hosts = Some(hosts(v));
        }
        if let Some(ms) = std::env::var(BACKOFF_ENV).ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|n| *n > 0)
        {
            c.backoff_base = Duration::from_millis(ms);
        }
        if let Some(s) = std::env::var(RETRY_FOR_ENV).ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|n| *n > 0)
        {
            c.retry_for = Duration::from_secs(s);
        }
        c
    }

    fn allowed_host(&self, host: &str) -> bool {
        self.allow_hosts.iter().any(|h| h == host)
    }

    fn outside_only(&self, host: &str) -> Option<String> {
        let only = self.only_hosts.as_ref()?;
        (!only.iter().any(|h| h == host))
            .then(|| format!("{host} is not one of the hosts this server may call ({ONLY_HOSTS_ENV})"))
    }

    /// D4 before any lookup: an absolute `https` URL (or `http` to an allowlisted host), no user
    /// info, at most [`MAX_URL_LEN`].
    pub fn check_url(&self, url: &str) -> Result<String, String> {
        if url.len() > MAX_URL_LEN {
            return Err(format!("the URL is longer than {MAX_URL_LEN} characters"));
        }
        let uri: ureq::http::Uri = url.parse().map_err(|_| "not a URL".to_string())?;
        let host = uri.host().map(|h| h.trim_matches(['[', ']']).to_ascii_lowercase()).ok_or("the URL has no host")?;
        if uri.authority().is_some_and(|a| a.as_str().contains('@')) {
            return Err("the URL may not carry a user name or password".into());
        }
        if let Some(e) = self.outside_only(&host) {
            return Err(e);
        }
        match uri.scheme_str() {
            Some("https") => {}
            Some("http") if self.allowed_host(&host) => {}
            _ => return Err("hooks call https URLs only".into()),
        }
        Ok(host)
    }

    /// D4 at lookup: every address the name resolves to must be public, unless the host is
    /// allowlisted.
    pub fn check_addrs(&self, host: &str, addrs: &[SocketAddr]) -> Result<(), String> {
        if let Some(e) = self.outside_only(host) {
            return Err(e);
        }
        if self.allowed_host(host) {
            return Ok(());
        }
        match addrs.iter().find(|a| !is_public(a.ip())) {
            Some(a) => Err(format!("{host} resolves to {}, which is not a public address", a.ip())),
            None if addrs.is_empty() => Err(format!("{host} does not resolve")),
            None => Ok(()),
        }
    }

    /// Both checks, with a real lookup (blocking).
    pub fn check_reachable(&self, url: &str) -> Result<(), String> {
        let host = self.check_url(url)?;
        let uri: ureq::http::Uri = url.parse().map_err(|_| "not a URL".to_string())?;
        let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("http") { 80 } else { 443 });
        let addrs: Vec<SocketAddr> =
            (host.as_str(), port).to_socket_addrs().map_err(|e| format!("{host}: {e}"))?.collect();
        self.check_addrs(&host, &addrs)
    }
}

/// Public unicast: not loopback, private, link-local, CGNAT (tailnets), unspecified, multicast,
/// broadcast, documentation or benchmarking ranges; IPv4-mapped IPv6 is judged as IPv4.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                || (s[0] == 0x64 && s[1] == 0xff9b))
        }
    }
}

/// ureq's name lookup with the D4 address check inside it: the addresses a connection may use
/// are exactly the ones checked. `unversioned` is outside ureq's semver; `Cargo.lock` pins it.
#[derive(Debug)]
struct GuardedResolver {
    cfg: Arc<HookConfig>,
    inner: ureq::unversioned::resolver::DefaultResolver,
}

impl ureq::unversioned::resolver::Resolver for GuardedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().unwrap_or_default().trim_matches(['[', ']']).to_ascii_lowercase();
        // A host outside ONLY_HOSTS is refused before any lookup is made for it.
        if self.cfg.outside_only(&host).is_some() {
            return Err(ureq::Error::HostNotFound);
        }
        let addrs = self.inner.resolve(uri, config, timeout)?;
        let list: Vec<SocketAddr> = addrs.iter().copied().collect();
        match self.cfg.check_addrs(&host, &list) {
            Ok(()) => Ok(addrs),
            Err(_) => Err(ureq::Error::HostNotFound),
        }
    }
}

/// The HTTP client every delivery uses.
#[derive(Clone)]
pub struct Sender {
    agent: ureq::Agent,
}

/// What a receiver said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Delivered(u16),
    /// 404 / 410: the receiver no longer exists.
    Gone(u16),
    Failed(String),
}

impl Outcome {
    pub fn status(&self) -> String {
        match self {
            Outcome::Delivered(c) | Outcome::Gone(c) => c.to_string(),
            Outcome::Failed(s) => s.clone(),
        }
    }
}

impl Sender {
    pub fn new(cfg: Arc<HookConfig>) -> Sender {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(POST_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .user_agent(format!("annotary/{}", crate::VERSION))
            .build();
        let resolver = GuardedResolver { cfg, inner: Default::default() };
        Sender {
            agent: ureq::Agent::with_parts(config, ureq::unversioned::transport::DefaultConnector::default(), resolver),
        }
    }

    /// One signed POST (blocking).
    pub fn post(&self, url: &str, secret: &str, hook_id: &str, seq: i64, body: &[u8]) -> Outcome {
        let res = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-Annotary-Signature", hooks::signature(secret, body))
            .header("X-Annotary-Hook", hook_id)
            .header("X-Annotary-Seq", seq.to_string())
            .send(body);
        match res {
            Ok(r) => {
                let code = r.status().as_u16();
                match code {
                    200..=299 => Outcome::Delivered(code),
                    404 | 410 => Outcome::Gone(code),
                    _ => Outcome::Failed(code.to_string()),
                }
            }
            Err(ureq::Error::HostNotFound) => Outcome::Failed("refused: not a public address".into()),
            Err(ureq::Error::Timeout(_)) => Outcome::Failed("timeout".into()),
            Err(e) => Outcome::Failed(format!("error: {}", short(&e.to_string()))),
        }
    }

    /// [`Sender::post`] on its own thread, awaited; `None` when the server began to stop first
    /// (the thread finishes on its own and its answer is dropped).
    async fn post_async(
        &self,
        shutdown: &mut tokio::sync::watch::Receiver<bool>,
        url: String,
        secret: String,
        hook_id: String,
        seq: i64,
        body: Vec<u8>,
    ) -> Option<Outcome> {
        if *shutdown.borrow() {
            return None;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        let me = self.clone();
        std::thread::spawn(move || {
            let _ = tx.send(me.post(&url, &secret, &hook_id, seq, &body));
        });
        tokio::select! {
            r = rx => r.ok(),
            _ = shutdown.changed() => None,
        }
    }
}

fn short(s: &str) -> String {
    s.chars().take(120).collect()
}

/// The `v: 1` body for one event (`kind` = what the hook asked for), body-free.
pub fn payload(st: &AppState, h: &HookRow, row: &EventRow, kind: Kind, by: Option<String>) -> Value {
    let e = &row.event;
    let doc = e.doc_id.as_deref().and_then(|d| st.store.document(d).ok().flatten());
    let thread = match kind {
        Kind::Mention => e.payload["thread"]
            .as_str()
            .map(str::to_string)
            .or_else(|| e.payload["comment"].as_str().and_then(|c| c.split_once('.').map(|(t, _)| t.to_string()))),
        _ => None,
    };
    let link = e.doc_id.as_deref().map(|d| doc_url(st, d, thread.as_deref()));
    json!({
        "v": 1,
        "kind": kind.as_str(),
        "doc": e.doc_id,
        "name": doc.as_ref().map(|d| d.head.name.clone()),
        "version": e.payload["version"].as_u64(),
        "thread": thread,
        "action": if kind == Kind::Action { e.payload["action"].clone() } else { Value::Null },
        "by": by.map(|h| format!("@{h}")),
        "seq": e.seq,
        "at": e.at,
        "link": link,
        "hook": h.id(),
    })
}

/// The link `annotary link DOC [t7]` prints, else the server's own URL.
fn doc_url(st: &AppState, doc_id: &str, thread: Option<&str>) -> String {
    let public = st.store.public_url().ok().flatten();
    let relay = st.store.link_base().ok().flatten();
    annotary_store::doc_link(public.as_deref(), relay.as_deref(), doc_id, thread).unwrap_or_else(|| {
        let frag = thread.map(|t| format!("#{t}")).unwrap_or_default();
        format!("{}/d/{doc_id}{frag}", st.url.trim_end_matches('/'))
    })
}

/// A hook as the API shows it: no URL, no secret.
pub fn hook_json(st: &AppState, h: &HookRow) -> Value {
    let handle = st.store.member_by_id(&h.member_id).ok().flatten().map(|m| m.handle);
    let name = h.doc_id.as_deref().and_then(|d| st.store.document(d).ok().flatten()).map(|d| d.head.name);
    let now = time::now();
    json!({
        "id": h.id(),
        "member": handle,
        "doc": h.doc_id,
        "name": name,
        "on": h.kinds,
        "once": h.once,
        "host": h.host(),
        "url_fp": h.url_fingerprint(),
        "created_at": time::to_seconds(&h.created_at),
        "expires_at": h.expires_at.as_deref().map(time::to_seconds),
        "state": if h.disabled_at.is_some() { "disabled" } else if h.is_live(&now) { if h.failures > 0 { "failing" } else { "live" } } else { "expired" },
        "last_status": h.last_status,
        "last_attempt_at": h.last_attempt_at.as_deref().map(time::to_seconds),
        "failures": h.failures,
        "cursor": h.cursor_seq,
    })
}

// ---- routes ----

#[derive(Debug, Deserialize)]
pub struct AddBody {
    pub url: String,
    pub on: String,
    pub doc: Option<String>,
    #[serde(default)]
    pub once: bool,
    /// Seconds.
    pub expires: Option<i64>,
    /// The receiver's own secret (24–128 printable characters); absent = generate one.
    pub secret: Option<String>,
}

fn refused(msg: String) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "hook_address_refused", msg)
        .hint("hooks call public https URLs; an owner can allow a private host with ANNOTARY_HOOK_ALLOW_HOSTS")
}

/// `POST /api/v1/me/hooks`
pub async fn add(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<AddBody>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Json(b) = body?;
    let url = b.url.trim().to_string();
    let kinds = hooks::parse_kinds(&b.on).map_err(ApiError::invalid)?;
    let doc_id = match b.doc.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => {
            let doc = load_doc(&st, d)?;
            auth.require_right(&doc.vault_name, annotary_store::Right::Read)?;
            Some(doc.id)
        }
        None => None,
    };
    let cfg = st.hooks.clone();
    let check_url = url.clone();
    tokio::task::spawn_blocking(move || cfg.check_reachable(&check_url))
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .map_err(refused)?;
    let token_id = match &auth.0.via {
        AuthVia::Token { id } => Some(id.clone()),
        AuthVia::Session { .. } => None,
    };
    let row = st.store.add_hook(NewHook {
        member_id: auth.0.member_id.clone(),
        token_id,
        doc_id,
        kinds,
        once: b.once,
        url,
        expires_in: b.expires,
        secret: b.secret.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
    })?;
    Ok((StatusCode::CREATED, Json(json!({ "hook": hook_json(&st, &row), "secret": row.secret }))))
}

/// `GET /api/v1/me/hooks`
pub async fn list_mine(State(st): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let rows = st.store.hooks(Some(&auth.0.member_id))?;
    Ok(Json(json!({ "hooks": rows.iter().map(|h| hook_json(&st, h)).collect::<Vec<_>>() })))
}

/// `GET /api/v1/hooks` (owner)
pub async fn list_all(State(st): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require_owner("listing every hook")?;
    let rows = st.store.hooks(None)?;
    Ok(Json(json!({ "hooks": rows.iter().map(|h| hook_json(&st, h)).collect::<Vec<_>>() })))
}

fn not_found(id: &str) -> ApiError {
    ApiError::not_found(format!("no hook {id}")).hint("`annotary hook list` shows yours")
}

/// The hook `id` names, if the caller may act on it (its Member; `owner_ok` lets an owner).
fn load_for(st: &AppState, auth: &Auth, id: &str, owner_ok: bool) -> Result<HookRow, ApiError> {
    let n = parse_hook_id(id).ok_or_else(|| not_found(id))?;
    let h = st.store.hook(n)?.ok_or_else(|| not_found(id))?;
    if h.member_id == auth.0.member_id || (owner_ok && auth.0.is_owner()) {
        Ok(h)
    } else {
        // Someone else's hook is not there, as far as this caller can tell.
        Err(not_found(id))
    }
}

/// `DELETE /api/v1/me/hooks/{id}`
pub async fn remove(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let h = load_for(&st, &auth, &id, true)?;
    let removed = st.store.remove_hook(Some(&auth.0.member_id), h.n, "removed")?;
    Ok(Json(json!({ "removed": removed, "id": h.id() })))
}

/// `POST /api/v1/me/hooks/{id}/test`
pub async fn test(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let h = load_for(&st, &auth, &id, false)?;
    let body = json!({
        "v": 1,
        "kind": "test",
        "doc": h.doc_id,
        "name": Value::Null,
        "version": Value::Null,
        "thread": Value::Null,
        "action": Value::Null,
        "by": format!("@{}", auth.0.handle),
        "seq": 0,
        "at": time::now(),
        "link": h.doc_id.as_deref().map(|d| doc_url(&st, d, None)),
        "hook": h.id(),
    });
    let bytes = serde_json::to_vec(&body).map_err(|e| ApiError::internal(e.to_string()))?;
    // On its own thread like every delivery, so a hung receiver never holds shutdown.
    let mut shutdown = st.shutdown.clone();
    let out = st
        .hook_sender
        .post_async(&mut shutdown, h.url.clone(), h.secret.clone(), h.id(), 0, bytes)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "shutting_down", "the server is stopping"))?;
    Ok(Json(json!({ "id": h.id(), "status": out.status(), "ok": matches!(out, Outcome::Delivered(_)) })))
}

// ---- the deliverer ----

/// Run until the server stops: a pass after every commit and every [`TICK`] (or sooner, when a
/// retry comes due).
pub async fn deliverer(st: AppState) {
    let mut rx = st.events.clone();
    let mut shutdown = st.shutdown.clone();
    loop {
        if *shutdown.borrow() {
            return;
        }
        rx.borrow_and_update();
        let next = match pass(&st, &mut shutdown).await {
            Ok(next) => next,
            Err(e) => {
                tracing::warn!("hooks: {}", e.message);
                None
            }
        };
        let sleep = next.unwrap_or(TICK).min(TICK);
        tokio::select! {
            r = rx.changed() => if r.is_err() { return },
            _ = tokio::time::sleep(sleep) => {}
            _ = shutdown.changed() => return,
        }
    }
}

fn backoff(cfg: &HookConfig, failures: i64) -> Duration {
    let exp = failures.clamp(0, 20) as u32;
    cfg.backoff_base.saturating_mul(2u32.saturating_pow(exp)).min(MAX_BACKOFF)
}

fn later(d: Duration) -> String {
    let ts =
        jiff::Timestamp::now() + jiff::SignedDuration::try_from(d).unwrap_or(jiff::SignedDuration::from_secs(3600));
    time::format(ts)
}

fn until(stamp: &str) -> Duration {
    match time::parse(stamp) {
        Some(t) => {
            let d = t.duration_since(jiff::Timestamp::now());
            Duration::try_from(d).unwrap_or(Duration::ZERO)
        }
        None => Duration::ZERO,
    }
}

/// One pass over every live hook. Returns how soon the next retry is due, if any.
async fn pass(st: &AppState, shutdown: &mut tokio::sync::watch::Receiver<bool>) -> Result<Option<Duration>, ApiError> {
    let now = time::now();
    st.store.expire_hooks(&now)?;
    let head = st.store.last_seq()?;
    let mut next: Option<Duration> = None;
    for h in st.store.hooks(None)? {
        if *shutdown.borrow() {
            return Ok(None);
        }
        if !h.is_live(&now) || h.cursor_seq >= head {
            continue;
        }
        if let Some(at) = h.next_attempt_at.as_deref()
            && at > now.as_str()
        {
            let d = until(at);
            next = Some(next.map_or(d, |n| n.min(d)));
            continue;
        }
        if let Some(d) = deliver_hook(st, shutdown, &h, head).await? {
            next = Some(next.map_or(d, |n| n.min(d)));
        }
    }
    Ok(next)
}

/// Deliver what one hook has waiting. `Some(delay)` = it failed and retries after `delay`.
async fn deliver_hook(
    st: &AppState,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
    h: &HookRow,
    head: i64,
) -> Result<Option<Duration>, ApiError> {
    let Some(member) = st.store.hook_member(h)? else {
        st.store.disable_hook(h.n, "credential", "the Token or Member that added it is gone")?;
        return Ok(None);
    };
    let auth = Auth(member);
    let kinds: Vec<Kind> = h.kinds.iter().filter_map(|k| Kind::parse(k)).collect();
    let filter = Filter {
        member_id: auth.0.member_id.clone(),
        handle: auth.0.handle.clone(),
        is_owner: auth.0.is_owner(),
        readable: readable_vaults(st, &auth)?.into_iter().map(|v| v.id).collect(),
        doc_id: h.doc_id.clone(),
        kinds: kinds.iter().copied().collect(),
        types: Default::default(),
        action: None,
        me: false,
    };
    let rows = st.store.events_page(&EventQuery {
        since: h.cursor_seq,
        until: head,
        doc_id: h.doc_id.clone(),
        types: filter.sql_types(),
        limit: PAGE + 1,
        member_events: None,
    })?;
    let cut = rows.len() > PAGE;
    let rows = &rows[..rows.len().min(PAGE)];
    let reached = if cut { rows.last().map_or(head, |r| r.event.seq) } else { head };
    // Failure state as this pass changes it: a delivery resets it, so a later failure in the same
    // pass starts a fresh backoff instead of reading the row loaded before the pass.
    let (mut failures, mut failing_since) = (h.failures, h.failing_since.clone());
    for row in rows {
        // The Member's own Version or Action never wakes it.
        if row.event.by.as_deref() == Some(auth.0.member_id.as_str()) || !filter.readable(row) {
            continue;
        }
        let Some((Some(kind), _)) = filter.admit(&row.event) else { continue };
        let by = match &row.event.by {
            Some(id) => st.store.member_by_id(id)?.map(|m| m.handle),
            None => None,
        };
        let body = payload(st, h, row, kind, by);
        let bytes = serde_json::to_vec(&body).map_err(|e| ApiError::internal(e.to_string()))?;
        let Some(out) =
            st.hook_sender.post_async(shutdown, h.url.clone(), h.secret.clone(), h.id(), row.event.seq, bytes).await
        else {
            return Ok(None);
        };
        match out {
            Outcome::Delivered(_) => {
                st.store.record_hook_attempt(
                    h.n,
                    &HookAttempt { cursor: Some(row.event.seq), status: out.status(), ok: true, next_attempt_at: None },
                )?;
                (failures, failing_since) = (0, None);
                if h.once {
                    st.store.remove_hook(None, h.n, "once")?;
                    return Ok(None);
                }
            }
            Outcome::Gone(_) => {
                st.store.disable_hook(h.n, "gone", &out.status())?;
                return Ok(None);
            }
            Outcome::Failed(status) => {
                let failed_for = failing_since
                    .as_deref()
                    .and_then(time::parse)
                    .map(|t| jiff::Timestamp::now().duration_since(t))
                    .and_then(|d| Duration::try_from(d).ok())
                    .unwrap_or(Duration::ZERO);
                if failing_since.is_some() && failed_for >= st.hooks.retry_for {
                    st.store.disable_hook(h.n, "failing", &status)?;
                    return Ok(None);
                }
                let delay = backoff(&st.hooks, failures);
                st.store.record_hook_attempt(
                    h.n,
                    &HookAttempt { cursor: None, status, ok: false, next_attempt_at: Some(later(delay)) },
                )?;
                return Ok(Some(delay));
            }
        }
    }
    st.store.advance_hook_cursor(h.n, reached)?;
    // A full page: the rest is waiting now, and moving a cursor wakes nobody, so go again at once.
    Ok(cut.then_some(Duration::ZERO))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_addresses() {
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        // Inward ranges, built from octets: public text in this repo holds no private addresses.
        let v4 = |a, b, c, d| std::net::Ipv4Addr::new(a, b, c, d);
        let inward = [
            v4(127, 0, 0, 1),
            v4(10, 1, 2, 3),
            v4(172, 16, 0, 1),
            v4(192, 168, 1, 1),
            v4(169, 254, 169, 254),
            v4(100, 64, 0, 1),
            v4(100, 127, 255, 254),
            v4(0, 0, 0, 0),
            v4(224, 0, 0, 1),
            v4(255, 255, 255, 255),
        ];
        for ip in inward {
            assert!(!is_public(IpAddr::V4(ip)), "{ip}");
            assert!(!is_public(IpAddr::V6(ip.to_ipv6_mapped())), "{ip} mapped");
        }
        for ip in ["::1", "::", "fd00::1", "fe80::1"] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        assert!(is_public(IpAddr::V4(v4(100, 128, 0, 1))));
    }

    #[test]
    fn urls_and_the_allowlist() {
        let cfg = HookConfig { allow_hosts: vec!["127.0.0.1".into()], ..Default::default() };
        assert_eq!(cfg.check_url("https://relay.example.com/v1/x").unwrap(), "relay.example.com");
        assert!(cfg.check_url("http://relay.example.com/x").is_err());
        assert!(cfg.check_url("ftp://relay.example.com/x").is_err());
        assert!(cfg.check_url("https://u:p@relay.example.com/x").is_err());
        assert!(cfg.check_url("relay.example.com/x").is_err());
        assert!(cfg.check_url("http://127.0.0.1:9/x").is_ok());
        let local: SocketAddr = "127.0.0.1:443".parse().unwrap();
        assert!(cfg.check_addrs("127.0.0.1", &[local]).is_ok());
        assert!(cfg.check_addrs("inward.example.com", &[local]).is_err());
        let public: SocketAddr = "8.8.8.8:443".parse().unwrap();
        assert!(cfg.check_addrs("dns.example.com", &[public, local]).is_err());
        assert!(cfg.check_addrs("dns.example.com", &[public]).is_ok());
        // ONLY_HOSTS: nothing outside the list is ever called, public or not.
        let only = HookConfig { only_hosts: Some(vec!["127.0.0.1".into()]), ..cfg.clone() };
        assert!(only.check_url("https://relay.example.com/x").is_err());
        assert!(only.check_addrs("relay.example.com", &[public]).is_err());
        assert!(only.check_addrs("127.0.0.1", &[local]).is_ok());
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let cfg = HookConfig::default();
        assert_eq!(backoff(&cfg, 0), Duration::from_secs(5));
        assert_eq!(backoff(&cfg, 1), Duration::from_secs(10));
        assert_eq!(backoff(&cfg, 30), MAX_BACKOFF);
    }
}
