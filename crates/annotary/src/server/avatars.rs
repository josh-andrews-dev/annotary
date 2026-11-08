//! Member avatars over HTTP (`annotary_store::avatar` checks the images and stores them).
//!
//! | route | who | what |
//! |---|---|---|
//! | `PUT /api/v1/me/avatar {image, static?}` | every Member, their own | set the avatar: `image` the image's bytes in base64 (PNG, JPEG, GIF, WebP or SVG), `static` an optional PNG / JPEG / WebP fallback for reduced motion; the answer is the Member's JSON + `changed` |
//! | `DELETE /api/v1/me/avatar` | every Member, their own | clear it (`changed: false` when there was none) |
//! | `DELETE /api/v1/members/{handle}/avatar` | owner | clear another Member's |
//! | `GET /avatars/{hash}` | any signed-in Member (web pages router: cookie, loopback trust or bearer) | a current avatar or static fallback, with the type the server read, `nosniff`, [`AVATAR_CSP`] and an immutable cache; any other hash 404 |
//!
//! Errors: 400 `invalid_avatar` (the reason the check gave), 400 `invalid` (bad base64 or body),
//! 413 `too_large`, 404 `no Member @h`, 403 for a member clearing someone else's.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::{Avatar, blobs::is_blob_hash};

use super::access::load_member;
use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use super::members::member_json;

/// What `/avatars/{hash}` answers with: nothing may load or run but inline styles and `data:`
/// images, even when an SVG avatar is opened on its own in a tab.
pub const AVATAR_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src data:";

/// The URL a page loads an avatar blob from.
pub fn avatar_url(hash: &str) -> String {
    format!("/avatars/{hash}")
}

/// An avatar in member JSON: `{url, type, size, hash, static_url?, static_type?, static_size?,
/// static_hash?, set_at}`.
pub fn avatar_json(a: &Avatar) -> Value {
    let mut v = json!({
        "url": avatar_url(&a.hash),
        "type": a.content_type,
        "size": a.size,
        "hash": a.hash,
    });
    if let Some(h) = &a.static_hash {
        v["static_url"] = json!(avatar_url(h));
        v["static_type"] = json!(a.static_type);
        v["static_size"] = json!(a.static_size);
        v["static_hash"] = json!(h);
    }
    v["set_at"] = json!(a.set_at);
    v
}

#[derive(Debug, Deserialize)]
pub struct AvatarBody {
    /// The image, base64 (standard alphabet; a `data:…;base64,` prefix is accepted too).
    pub image: String,
    /// The static fallback, base64, or absent / null.
    #[serde(default, rename = "static")]
    pub static_image: Option<String>,
}

fn decode(what: &str, text: &str) -> Result<Vec<u8>, ApiError> {
    let text = text.trim();
    let text = match text.strip_prefix("data:") {
        Some(rest) => rest.split_once(";base64,").map(|(_, b)| b).unwrap_or(rest),
        None => text,
    };
    let clean: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(clean.as_bytes())
        .map_err(|_| ApiError::invalid(format!("{what} is not base64")).hint("send the file's bytes base64-encoded"))
}

fn answer(m: &annotary_store::MemberView, changed: bool) -> Json<Value> {
    let mut v = member_json(m);
    v["changed"] = json!(changed);
    Json(v)
}

/// `PUT /api/v1/me/avatar`.
pub async fn put_me(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<AvatarBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(body) = body?;
    let image = decode("image", &body.image)?;
    let static_image = match body.static_image.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => Some(decode("static", s)?),
        None => None,
    };
    let (m, changed) = st.store.set_avatar(&auth.0.member_id, &auth.0.member_id, &image, static_image.as_deref())?;
    Ok(answer(&m, changed))
}

/// `DELETE /api/v1/me/avatar`.
pub async fn delete_me(State(st): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    let (m, changed) = st.store.clear_avatar(&auth.0.member_id, &auth.0.member_id)?;
    Ok(answer(&m, changed))
}

/// `DELETE /api/v1/members/{handle}/avatar` (owner; a Member clears their own through `/me`).
pub async fn delete_member(
    State(st): State<AppState>,
    auth: Auth,
    Path(handle): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let m = load_member(&st, &handle)?;
    if m.id != auth.0.member_id {
        auth.require_owner("clearing another Member's avatar")?;
    }
    let (m, changed) = st.store.clear_avatar(&auth.0.member_id, &m.id)?;
    if changed && m.id != auth.0.member_id {
        tracing::info!("avatar of @{} cleared by @{}", m.handle, auth.0.handle);
    }
    Ok(answer(&m, changed))
}

/// `GET /avatars/{hash}`.
pub async fn get(State(st): State<AppState>, _auth: Auth, Path(hash): Path<String>) -> Result<Response, ApiError> {
    let missing = || ApiError::not_found(format!("no avatar {hash}"));
    if !is_blob_hash(&hash) {
        return Err(missing());
    }
    let Some(content_type) = st.store.avatar_type(&hash)? else { return Err(missing()) };
    let Some(bytes) = st.store.read_blob(&hash)? else { return Err(missing()) };
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, name: header::HeaderName, value: &str| {
        if let Ok(v) = header::HeaderValue::from_str(value) {
            h.insert(name, v);
        }
    };
    set(h, header::CONTENT_TYPE, &content_type);
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::CONTENT_SECURITY_POLICY, AVATAR_CSP);
    set(h, header::CACHE_CONTROL, "private, max-age=31536000, immutable");
    set(h, header::ETAG, &format!("\"{hash}\""));
    set(h, header::CONTENT_DISPOSITION, "inline");
    Ok(resp)
}
