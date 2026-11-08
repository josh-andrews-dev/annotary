//! Member avatars against the in-process server: the store's set / clear with their events and
//! the blob GC / export, `PUT|DELETE /api/v1/me/avatar`, the owner's
//! `DELETE /api/v1/members/{handle}/avatar`, the refusals, and `GET /avatars/{hash}` with its
//! headers. The SVG check itself is unit-tested in `annotary_store::avatar`.

use std::time::Duration;

use annotary::testing::TestServer;
use annotary_store::{PushInput, Store};
use base64::Engine;
use serde_json::{Value, json};

const SCRIBE_SVG: &[u8] = include_bytes!("../../../fixtures/avatars/scribe.svg");
const SCRIBE_PNG: &[u8] = include_bytes!("../../../fixtures/avatars/scribe.png");
const LINTER_SVG: &[u8] = include_bytes!("../../../fixtures/avatars/linter.svg");

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn call(ts: &TestServer, method: &str, path: &str, token: &str, body: Option<Value>) -> (u16, Value) {
    let url = format!("{}{}", ts.url, path);
    let auth = format!("Bearer {token}");
    let res = match method {
        "PUT" => ureq::put(&url)
            .header("Authorization", &auth)
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(body.unwrap_or(json!({}))),
        "DELETE" => {
            ureq::delete(&url).header("Authorization", &auth).config().http_status_as_error(false).build().call()
        }
        _ => ureq::get(&url).header("Authorization", &auth).config().http_status_as_error(false).build().call(),
    };
    let mut r = res.unwrap();
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn get_raw(ts: &TestServer, path: &str, token: Option<&str>) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut req = ureq::get(format!("{}{}", ts.url, path)).header("Accept", "application/json");
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let mut r = req.config().http_status_as_error(false).max_redirects(0).build().call().unwrap();
    let status = r.status().as_u16();
    let hs = r
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string()))
        .collect();
    (status, hs, r.body_mut().read_to_vec().unwrap_or_default())
}

fn header<'a>(hs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn avatar_events(store: &Store) -> Vec<(String, Value)> {
    store
        .events_since(0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind.starts_with("member.avatar"))
        .map(|e| (e.kind, e.payload))
        .collect()
}

// spec: S39-03
#[test]
fn set_and_clear_go_through_events_and_the_gc_keeps_only_current_avatars() {
    let ts = TestServer::start();
    let (id, _) = ts.add_member("scribe");
    let (m, changed) = ts.store.set_avatar(&id, &id, SCRIBE_SVG, Some(SCRIBE_PNG)).unwrap();
    assert!(changed);
    let a = m.avatar.clone().expect("avatar");
    assert_eq!(a.content_type, "image/svg+xml");
    assert_eq!(a.size, SCRIBE_SVG.len() as u64);
    assert_eq!(a.static_type.as_deref(), Some("image/png"));
    assert_eq!(a.static_size, Some(SCRIBE_PNG.len() as u64));
    // Every Member read carries it.
    assert_eq!(ts.store.member_by_handle("scribe").unwrap().unwrap().avatar, Some(a.clone()));
    assert!(ts.store.list_members().unwrap().iter().any(|m| m.avatar.as_ref() == Some(&a)));
    // The same images again: nothing changes, no event.
    let (_, again) = ts.store.set_avatar(&id, &id, SCRIBE_SVG, Some(SCRIBE_PNG)).unwrap();
    assert!(!again);
    let evs = avatar_events(&ts.store);
    assert_eq!(evs.len(), 1, "{evs:?}");
    assert_eq!(evs[0].0, "member.avatar_set");
    assert_eq!(
        evs[0].1,
        json!({ "member": id, "handle": "scribe", "hash": a.hash, "type": "image/svg+xml",
                "static_hash": a.static_hash, "static_type": "image/png" })
    );
    // Dropping the static fallback is a change.
    let (m2, changed) = ts.store.set_avatar(&id, &id, SCRIBE_SVG, None).unwrap();
    assert!(changed);
    assert_eq!(m2.avatar.as_ref().unwrap().static_hash, None);

    // The GC keeps a current avatar's blobs (an hour-old file is otherwise fair game) ...
    let removed = ts.store.gc_blobs(Duration::ZERO).unwrap();
    assert!(!removed.contains(&a.hash), "the GC removed a current avatar");
    // ... the static one, no longer referenced, goes.
    assert!(removed.contains(a.static_hash.as_ref().unwrap()), "{removed:?}");
    // export carries the avatar blob.
    let tar = ts.data_dir.join("x.tar");
    let report = annotary_store::export_tar(&ts.data_dir.join("store"), std::fs::File::create(&tar).unwrap()).unwrap();
    assert!(report.blobs >= 1);
    let mut names = Vec::new();
    for e in tar::Archive::new(std::fs::File::open(&tar).unwrap()).entries().unwrap() {
        names.push(e.unwrap().path().unwrap().to_string_lossy().into_owned());
    }
    assert!(names.iter().any(|n| n.ends_with(&a.hash)), "{names:?}");

    // Clear: an event; clearing again: none; the blob goes to the GC.
    let (m3, cleared) = ts.store.clear_avatar(&ts.owner_id, &id).unwrap();
    assert!(cleared && m3.avatar.is_none());
    let (_, again) = ts.store.clear_avatar(&ts.owner_id, &id).unwrap();
    assert!(!again);
    let evs = avatar_events(&ts.store);
    assert_eq!(evs.len(), 3, "{evs:?}");
    assert_eq!(evs[2], ("member.avatar_cleared".to_string(), json!({ "member": id, "handle": "scribe" })));
    assert!(ts.store.gc_blobs(Duration::ZERO).unwrap().contains(&a.hash));

    // A refused image changes nothing.
    let err = ts.store.set_avatar(&id, &id, b"<svg><script/></svg>", None).unwrap_err();
    assert!(err.to_string().starts_with("the avatar:"), "{err}");
    assert_eq!(avatar_events(&ts.store).len(), 3);
}

// spec: S39-04
#[test]
fn every_member_sets_and_clears_their_own_and_an_owner_clears_anyones() {
    let ts = TestServer::start();
    let (_, scribe) = ts.add_member("scribe");
    let (_, linter) = ts.add_member("linter");

    let (s, v) = call(
        &ts,
        "PUT",
        "/api/v1/me/avatar",
        &scribe,
        Some(json!({ "image": b64(SCRIBE_SVG), "static": b64(SCRIBE_PNG) })),
    );
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["handle"], "scribe");
    assert_eq!(v["changed"], true);
    let av = &v["avatar"];
    let hash = av["hash"].as_str().unwrap().to_string();
    let static_hash = av["static_hash"].as_str().unwrap().to_string();
    assert_eq!(av["url"], format!("/avatars/{hash}"));
    assert_eq!(av["type"], "image/svg+xml");
    assert_eq!(av["size"], SCRIBE_SVG.len());
    assert_eq!(av["static_url"], format!("/avatars/{static_hash}"));
    assert_eq!(av["static_type"], "image/png");
    assert_eq!(av["static_size"], SCRIBE_PNG.len());
    assert!(av["set_at"].is_string());
    // The same again (a data: URL prefix is fine too).
    let (s, v) = call(
        &ts,
        "PUT",
        "/api/v1/me/avatar",
        &scribe,
        Some(json!({ "image": format!("data:image/svg+xml;base64,{}", b64(SCRIBE_SVG)), "static": b64(SCRIBE_PNG) })),
    );
    assert_eq!((s, v["changed"].clone()), (200, json!(false)), "{v}");
    // Member JSON everywhere carries it; one without an avatar has null.
    let (_, me) = call(&ts, "GET", "/api/v1/me", &scribe, None);
    assert_eq!(me["avatar"]["hash"], hash.as_str());
    let (_, list) = call(&ts, "GET", "/api/v1/members", &linter, None);
    let by = |h: &str| list["members"].as_array().unwrap().iter().find(|m| m["handle"] == h).unwrap().clone();
    assert_eq!(by("scribe")["avatar"]["url"], format!("/avatars/{hash}"));
    assert_eq!(by("linter")["avatar"], Value::Null);
    let (_, one) = call(&ts, "GET", "/api/v1/members/scribe", &linter, None);
    assert_eq!(one["avatar"]["type"], "image/svg+xml");

    // A member may not clear someone else's; an owner may; an unknown handle is 404.
    let (s, v) = call(&ts, "DELETE", "/api/v1/members/scribe/avatar", &linter, None);
    assert_eq!(s, 403, "{v}");
    let (s, _) = call(&ts, "DELETE", "/api/v1/members/nobody/avatar", &ts.owner_token, None);
    assert_eq!(s, 404);
    let (s, v) = call(&ts, "DELETE", "/api/v1/members/@scribe/avatar", &ts.owner_token, None);
    assert_eq!((s, v["changed"].clone(), v["avatar"].clone()), (200, json!(true), Value::Null), "{v}");

    // Your own: set and clear through /me; clearing nothing is `changed: false`.
    let (s, _) = call(&ts, "PUT", "/api/v1/me/avatar", &linter, Some(json!({ "image": b64(LINTER_SVG) })));
    assert_eq!(s, 200);
    let (s, v) = call(&ts, "DELETE", "/api/v1/me/avatar", &linter, None);
    assert_eq!((s, v["changed"].clone()), (200, json!(true)));
    let (s, v) = call(&ts, "DELETE", "/api/v1/me/avatar", &linter, None);
    assert_eq!((s, v["changed"].clone()), (200, json!(false)));
}

// spec: S39-04
#[test]
fn refused_images_say_why() {
    let ts = TestServer::start();
    let (_, scribe) = ts.add_member("scribe");
    let put = |body: Value| call(&ts, "PUT", "/api/v1/me/avatar", &scribe, Some(body));
    let (s, v) =
        put(json!({ "image": b64(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>") }));
    assert_eq!((s, v["error"].as_str()), (400, Some("invalid_avatar")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("<script>"), "{v}");
    assert!(v["hint"].as_str().unwrap().contains("PNG"), "{v}");
    let (s, v) = put(json!({ "image": b64(SCRIBE_SVG), "static": b64(SCRIBE_SVG) }));
    assert_eq!((s, v["error"].as_str()), (400, Some("invalid_avatar")), "{v}");
    assert!(v["message"].as_str().unwrap().starts_with("the static avatar"), "{v}");
    let (s, v) = put(json!({ "image": b64(b"GIF8 not really") }));
    assert_eq!((s, v["error"].as_str()), (400, Some("invalid_avatar")), "{v}");
    let (s, v) = put(json!({ "image": "!!! not base64" }));
    assert_eq!((s, v["error"].as_str()), (400, Some("invalid")), "{v}");
    let (s, v) = put(json!({ "nope": 1 }));
    assert_eq!(s, 400, "{v}");
    let mut big = SCRIBE_PNG.to_vec();
    big.resize(300 * 1024, 0);
    let (s, v) = put(json!({ "image": b64(&big) }));
    assert_eq!((s, v["error"].as_str()), (413, Some("too_large")), "{v}");
    assert_eq!(v["max"], 256 * 1024);
    assert!(v["hint"].as_str().unwrap().contains("avatars"), "{v}");
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><title>{}</title></svg>", "x".repeat(70 * 1024));
    let (s, v) = put(json!({ "image": b64(svg.as_bytes()) }));
    assert_eq!((s, v["max"].clone()), (413, json!(64 * 1024)), "{v}");
    // Nothing was set.
    assert!(ts.store.member_by_handle("scribe").unwrap().unwrap().avatar.is_none());
}

// spec: S39-05
#[test]
fn avatars_are_served_with_the_servers_type_and_a_locked_down_policy() {
    let ts = TestServer::start();
    let (id, scribe) = ts.add_member("scribe");
    let (m, _) = ts.store.set_avatar(&id, &id, SCRIBE_SVG, Some(SCRIBE_PNG)).unwrap();
    let a = m.avatar.unwrap();

    let (s, hs, body) = get_raw(&ts, &format!("/avatars/{}", a.hash), Some(&scribe));
    assert_eq!(s, 200);
    assert_eq!(body, SCRIBE_SVG);
    assert_eq!(header(&hs, "content-type"), Some("image/svg+xml"));
    assert_eq!(header(&hs, "x-content-type-options"), Some("nosniff"));
    assert_eq!(
        header(&hs, "content-security-policy"),
        Some("default-src 'none'; style-src 'unsafe-inline'; img-src data:")
    );
    assert_eq!(header(&hs, "cache-control"), Some("private, max-age=31536000, immutable"));
    assert_eq!(header(&hs, "etag"), Some(format!("\"{}\"", a.hash).as_str()));
    let static_hash = a.static_hash.clone().unwrap();
    let (s, hs, body) = get_raw(&ts, &format!("/avatars/{static_hash}"), Some(&ts.owner_token));
    assert_eq!((s, header(&hs, "content-type")), (200, Some("image/png")));
    assert_eq!(body, SCRIBE_PNG);

    // Any other blob — a Document's Markdown — is not an avatar.
    let (md, _) = ts.store.put_blob(b"# secret plan\n").unwrap();
    let v = ts.store.vault_by_name("notes").unwrap().unwrap();
    ts.store.create_document(&ts.owner_id, &v.id, PushInput::new("plan", md.clone())).unwrap();
    assert_eq!(get_raw(&ts, &format!("/avatars/{md}"), Some(&scribe)).0, 404);
    assert_eq!(get_raw(&ts, "/avatars/not-a-hash", Some(&scribe)).0, 404);
    // Signed out (and no loopback trust: the access matrix covers the rest): no.
    let locked = TestServer::start_locked();
    let (lid, _) = locked.add_member("scribe");
    let (lm, _) = locked.store.set_avatar(&lid, &lid, SCRIBE_SVG, None).unwrap();
    assert_eq!(get_raw(&locked, &format!("/avatars/{}", lm.avatar.unwrap().hash), None).0, 401);
    // Cleared: gone.
    ts.store.clear_avatar(&id, &id).unwrap();
    assert_eq!(get_raw(&ts, &format!("/avatars/{}", a.hash), Some(&scribe)).0, 404);
}
