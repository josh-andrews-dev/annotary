//! Markup over HTTP (`annotary_store::markup` checks the files and stores them).
//!
//! | route | right | what |
//! |---|---|---|
//! | `POST /api/v1/docs/{id}/markups {image, strokes}` | comment | check and store a Markup (201): `image` the PNG's bytes in base64, `strokes` the strokes JSON itself (an object, `markup::check_strokes`); the answer is `{id, image, width, height, strokes, image_size, strokes_size}`, and `POST …/threads` / `…/replies` take `markup: ID` |
//! | `GET /api/v1/docs/{id}/markups/{hash}` | read | the image (`image/png`) or the strokes (`application/json`) of a Markup on a Comment of this Document, `nosniff`, an immutable cache and an `ETag` of the hash; any other hash 404 |
//!
//! Errors: 400 `invalid_markup` (the reason the check gave), 400 `invalid` (bad base64 or body),
//! 413 `too_large`, 409 `deleted` for a deleted Document; at attach, 409 `markup_missing` for an
//! id whose blobs are gone.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::{Markup, Right, blobs::is_blob_hash};

use super::app::AppState;
use super::auth::Auth;
use super::docs::load_doc;
use super::error::ApiError;

/// Largest upload body: the image's base64 (2 MiB of PNG is about 2.7 MiB) plus the strokes.
pub const BODY_MAX: usize = 4 * 1024 * 1024;

/// Where a Markup's image or strokes are served.
pub fn markup_url(doc_id: &str, hash: &str) -> String {
    format!("/api/v1/docs/{doc_id}/markups/{hash}")
}

/// A Markup in Comment JSON: `{image, width, height, strokes}` (the two blob hashes;
/// [`markup_url`] serves them).
pub fn markup_json(m: &Markup) -> Value {
    json!({ "image": m.image, "width": m.width, "height": m.height, "strokes": m.strokes })
}

#[derive(Debug, Deserialize)]
pub struct UploadBody {
    /// The PNG, base64 (standard alphabet; a `data:…;base64,` prefix is accepted too).
    pub image: String,
    /// The strokes JSON (`{"strokes": [...]}`).
    pub strokes: Value,
}

/// `POST /api/v1/docs/{id}/markups`.
pub async fn upload(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Result<Json<UploadBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Comment)?;
    let Json(body) = body.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::too_large("the Markup upload".into(), BODY_MAX as u64 + 1, BODY_MAX as u64)
        } else {
            e.into()
        }
    })?;
    let text = body.image.trim();
    let text = match text.strip_prefix("data:") {
        Some(rest) => rest.split_once(";base64,").map(|(_, b)| b).unwrap_or(rest),
        None => text,
    };
    let clean: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let image = base64::engine::general_purpose::STANDARD
        .decode(clean.as_bytes())
        .map_err(|_| ApiError::invalid("image is not base64").hint("send the PNG's bytes base64-encoded"))?;
    let strokes = serde_json::to_vec(&body.strokes).map_err(|e| ApiError::internal(e.to_string()))?;
    let up = st.store.upload_markup(&doc.id, &image, &strokes)?;
    tracing::info!(
        "markup {}×{} on {} ({}) by @{}",
        up.markup.width,
        up.markup.height,
        doc.head.name,
        doc.id,
        auth.0.handle
    );
    let mut v = markup_json(&up.markup);
    v["id"] = json!(up.id);
    v["image_size"] = json!(up.markup.image_size);
    v["strokes_size"] = json!(up.markup.strokes_size);
    Ok((StatusCode::CREATED, Json(v)))
}

/// `GET /api/v1/docs/{id}/markups/{hash}`.
pub async fn get(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, hash)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let missing = || ApiError::not_found(format!("no Markup {hash} in {} ({})", doc.head.name, doc.id));
    if !is_blob_hash(&hash) {
        return Err(missing());
    }
    let Some(content_type) = st.store.markup_blob_type(&doc.id, &hash)? else { return Err(missing()) };
    let Some(bytes) = st.store.read_blob(&hash)? else { return Err(missing()) };
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, name: header::HeaderName, value: &str| {
        if let Ok(v) = header::HeaderValue::from_str(value) {
            h.insert(name, v);
        }
    };
    set(h, header::CONTENT_TYPE, content_type);
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::CACHE_CONTROL, "private, max-age=31536000, immutable");
    set(h, header::ETAG, &format!("\"{hash}\""));
    set(h, header::CONTENT_DISPOSITION, "inline");
    Ok(resp)
}
