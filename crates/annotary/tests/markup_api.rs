//! Markup against the in-process server: the store's upload and attach with their events, the
//! empty body, delete, the blob GC / export / purge, `POST /api/v1/docs/{id}/markups`, `markup` on
//! Threads and Replies, Comment JSON and the Sidecar, `GET …/markups/{hash}` with its headers, and
//! the card on the page and in the fragment. The PNG and strokes checks are unit-tested in
//! `annotary_store::markup`.

use std::time::Duration;

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{NewThread, Store};
use base64::Engine;
use serde_json::{Value, json};

const PNG: &[u8] = include_bytes!("../../../fixtures/markup/cross-out.png");
const STROKES: &str = include_str!("../../../fixtures/markup/cross-out.strokes.json");
const MD: &str = "# Plan\n\nWe ship on Friday.\n\nThe second line goes.\n";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn strokes() -> Value {
    serde_json::from_str(STROKES).unwrap()
}

fn create(ts: &TestServer, name: &str) -> String {
    let hash = sha256_hex(MD.as_bytes());
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .content_type("application/octet-stream")
        .send(MD.as_bytes())
        .unwrap();
    assert!(res.status().is_success());
    let (status, doc) =
        ts.post("/api/v1/docs", Some(&ts.owner_token), &json!({ "vault": "notes", "name": name, "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn upload(ts: &TestServer, token: &str, doc: &str) -> (u16, Value) {
    ts.post(&format!("/api/v1/docs/{doc}/markups"), Some(token), &json!({ "image": b64(PNG), "strokes": strokes() }))
}

fn get_raw(ts: &TestServer, path: &str, token: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut r = ureq::get(format!("{}{}", ts.url, path))
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
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

fn thread_events(store: &Store) -> Vec<(String, Value)> {
    store
        .events_since(0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind.starts_with("thread.") || e.kind.starts_with("markup"))
        .map(|e| (e.kind, e.payload))
        .collect()
}

// spec: S49-03
#[test]
fn upload_stores_blobs_only_and_the_comment_attaches_them_with_its_event() {
    let ts = TestServer::start();
    let doc = create(&ts, "mk-store");
    let store = &ts.store;
    let before = thread_events(store).len();
    let up = store.upload_markup(&doc, PNG, STROKES.as_bytes()).unwrap();
    assert_eq!((up.markup.width, up.markup.height), (480, 160));
    assert_eq!(up.id, format!("{}.{}", sha256_hex(PNG), sha256_hex(STROKES.as_bytes())));
    assert_eq!(thread_events(store).len(), before, "an upload appends no event");

    // A root Comment that is only a Markup.
    let mut input = NewThread::span("3:0-3:18", "");
    input.markup = Some(up.id.clone());
    let t = store.new_thread(&ts.owner_id, &doc, input).unwrap();
    assert_eq!(t.root.body, "");
    assert_eq!(t.root.markup.as_ref().unwrap(), &up.markup);
    assert!(t.has_markup());
    // Without a Markup the body still may not be empty.
    assert!(store.new_thread(&ts.owner_id, &doc, NewThread::document(" ")).is_err());
    // A Reply with one and some words.
    let up2 =
        store.upload_markup(&doc, PNG, br##"{"strokes":[{"color":"#000000","width":1,"points":[[0,0,0]]}]}"##).unwrap();
    let (t, c) = store.reply_with_markup(&ts.owner_id, &doc, t.num, "and this", false, Some(&up2.id)).unwrap();
    assert_eq!(c.markup.as_ref().unwrap().strokes, up2.markup.strokes);
    let evs = thread_events(store);
    let created = evs.iter().find(|(k, _)| k == "thread.created").unwrap();
    assert_eq!(created.1["markup"], json!(true));
    let replied = evs.iter().find(|(k, _)| k == "thread.replied").unwrap();
    assert_eq!(replied.1["markup"], json!(true));
    // An edit may empty a Comment that holds a Markup.
    let edited = store.edit_comment(&ts.owner_id, true, &doc, &c.id, " ").unwrap();
    assert!(edited.markup.is_some());

    // Ids that are not one, or whose blobs are gone.
    let bad = |id: &str| {
        let mut i = NewThread::document("x");
        i.markup = Some(id.into());
        store.new_thread(&ts.owner_id, &doc, i).unwrap_err().to_string()
    };
    assert!(bad("nope").contains("not a Markup id"));
    let gone = format!("{}.{}", "1".repeat(64), "2".repeat(64));
    assert!(bad(&gone).contains("upload it again"));

    // The GC keeps both Markups' blobs (an hour-old file is otherwise fair game); export carries them.
    let removed = store.gc_blobs(Duration::ZERO).unwrap();
    for h in [&up.markup.image, &up.markup.strokes, &up2.markup.strokes] {
        assert!(!removed.contains(h), "the GC removed an attached Markup's blob");
    }
    let tar = ts.data_dir.join("x.tar");
    annotary_store::export_tar(&ts.data_dir.join("store"), std::fs::File::create(&tar).unwrap()).unwrap();
    let mut names = Vec::new();
    for e in tar::Archive::new(std::fs::File::open(&tar).unwrap()).entries().unwrap() {
        names.push(e.unwrap().path().unwrap().to_string_lossy().into_owned());
    }
    assert!(names.iter().any(|n| n.ends_with(&up2.markup.strokes)), "{names:?}");

    // Deleting the Reply drops its Markup: its own strokes go to the GC, the shared image stays.
    let (c2, _) = store.delete_comment(&ts.owner_id, true, &doc, &c.id).unwrap();
    assert!(c2.markup.is_none());
    assert!(!t.replies.is_empty());
    let removed = store.gc_blobs(Duration::ZERO).unwrap();
    assert!(removed.contains(&up2.markup.strokes), "{removed:?}");
    assert!(!removed.contains(&up.markup.image));
    // A Markup nobody attached goes after the grace period too.
    let lone =
        store.upload_markup(&doc, PNG, br##"{"strokes":[{"color":"#123456","width":2,"points":[[1,1,0]]}]}"##).unwrap();
    assert!(store.gc_blobs(Duration::from_secs(3600)).unwrap().is_empty(), "young uploads survive");
    assert!(store.gc_blobs(Duration::ZERO).unwrap().contains(&lone.markup.strokes));

    // Purge removes the rest.
    store.delete_document(&ts.owner_id, &doc).unwrap();
    let out = store.purge_document(&ts.owner_id, &doc).unwrap();
    assert!(out.blobs_removed.contains(&up.markup.image), "{:?}", out.blobs_removed);
    assert!(out.blobs_removed.contains(&up.markup.strokes));
}

// spec: S49-04
#[test]
fn the_api_uploads_attaches_and_shows_markups() {
    let ts = TestServer::start();
    let doc = create(&ts, "mk-api");
    let (reader_id, reader) = ts.add_member("reader");
    let read_only = annotary_store::Grant { vault: "notes".into(), rights: vec![annotary_store::Right::Read] };
    ts.store.set_grants(&ts.owner_id, &reader_id, vec![read_only]).unwrap();
    let (status, body) = upload(&ts, &reader, &doc);
    assert_eq!(status, 403, "{body}");

    let (status, up) = upload(&ts, &ts.owner_token, &doc);
    assert_eq!(status, 201, "{up}");
    assert_eq!(up["width"], json!(480));
    assert_eq!(up["height"], json!(160));
    assert_eq!(up["image"], json!(sha256_hex(PNG)));
    assert_eq!(up["image_size"], json!(PNG.len()));
    // The strokes are stored as the JSON the request carried, written compactly.
    assert_eq!(up["strokes"], json!(sha256_hex(&serde_json::to_vec(&strokes()).unwrap())));
    let id = up["id"].as_str().unwrap().to_string();

    // Refusals.
    let post = |body: Value| ts.post(&format!("/api/v1/docs/{doc}/markups"), Some(&ts.owner_token), &body);
    let (s, e) = post(json!({ "image": b64(b"GIF89a...."), "strokes": strokes() }));
    assert_eq!((s, e["error"].as_str()), (400, Some("invalid_markup")), "{e}");
    let (s, e) = post(json!({ "image": "!!!", "strokes": strokes() }));
    assert_eq!((s, e["error"].as_str()), (400, Some("invalid")), "{e}");
    let (s, e) = post(json!({ "image": b64(PNG), "strokes": { "strokes": [] } }));
    assert_eq!((s, e["error"].as_str()), (400, Some("invalid_markup")), "{e}");
    let big = vec![0u8; 2 * 1024 * 1024 + 1];
    let (s, e) = post(json!({ "image": b64(&big), "strokes": strokes() }));
    assert_eq!((s, e["error"].as_str()), (413, Some("too_large")), "{e}");

    // A Thread whose root is only the Markup, and a Reply with one.
    let (s, t) = ts.post(
        &format!("/api/v1/docs/{doc}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "span", "range": "5:0-5:21", "markup": id }),
    );
    assert_eq!(s, 201, "{t}");
    assert_eq!(t["body"], json!(""));
    assert_eq!(t["markup"], json!({ "image": sha256_hex(PNG), "width": 480, "height": 160, "strokes": up["strokes"] }));
    let tid = t["id"].as_str().unwrap();
    let (s, r) = ts.post(
        &format!("/api/v1/docs/{doc}/threads/{tid}/replies"),
        Some(&ts.owner_token),
        &json!({ "body": "the loop: move it", "markup": id }),
    );
    assert_eq!(s, 201, "{r}");
    assert_eq!(r["comment"]["markup"]["width"], json!(480));
    // An id whose blobs are gone: 409.
    let (s, e) = ts.post(
        &format!("/api/v1/docs/{doc}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "document", "markup": format!("{}.{}", "1".repeat(64), "2".repeat(64)) }),
    );
    assert_eq!((s, e["error"].as_str()), (409, Some("markup_missing")), "{e}");
    // No body and no Markup: still refused.
    let (s, _) =
        ts.post(&format!("/api/v1/docs/{doc}/threads"), Some(&ts.owner_token), &json!({ "subject": "document" }));
    assert_eq!(s, 400);

    // The list and the Sidecar carry it.
    let (_, list) = ts.get(&format!("/api/v1/docs/{doc}/threads"), Some(&ts.owner_token));
    assert_eq!(list["threads"][0]["markup"]["height"], json!(160));
    assert_eq!(list["threads"][0]["replies"][0]["markup"]["image"], json!(sha256_hex(PNG)));
    let (s, sc) = ts.get(&format!("/api/v1/docs/{doc}/sidecar"), Some(&ts.owner_token));
    assert_eq!(s, 200);
    let parsed: annotary_sidecar::Sidecar = serde_json::from_value(sc).unwrap();
    let th = &parsed.threads[0];
    assert_eq!(th.markup.as_ref().unwrap().width, 480);
    assert_eq!(th.replies[0].markup.as_ref().unwrap().image, sha256_hex(PNG));

    // A deleted Document takes no upload.
    ts.store.delete_document(&ts.owner_id, &doc).unwrap();
    let (s, e) = upload(&ts, &ts.owner_token, &doc);
    assert_eq!((s, e["error"].as_str()), (409, Some("deleted")), "{e}");
}

// spec: S49-05
#[test]
fn markup_blobs_are_served_only_for_their_document() {
    let ts = TestServer::start();
    let doc = create(&ts, "mk-serve");
    let other = create(&ts, "mk-other");
    let (_, up) = upload(&ts, &ts.owner_token, &doc);
    let id = up["id"].as_str().unwrap();
    let image = up["image"].as_str().unwrap();
    let strokes_hash = up["strokes"].as_str().unwrap();
    // Not served before it is on a Comment.
    let (s, _, _) = get_raw(&ts, &format!("/api/v1/docs/{doc}/markups/{image}"), &ts.owner_token);
    assert_eq!(s, 404);
    let (s, _) = ts.post(
        &format!("/api/v1/docs/{doc}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "document", "body": "see", "markup": id }),
    );
    assert_eq!(s, 201);
    let (s, hs, bytes) = get_raw(&ts, &format!("/api/v1/docs/{doc}/markups/{image}"), &ts.owner_token);
    assert_eq!(s, 200);
    assert_eq!(bytes, PNG);
    assert_eq!(header(&hs, "content-type"), Some("image/png"));
    assert_eq!(header(&hs, "x-content-type-options"), Some("nosniff"));
    assert!(header(&hs, "cache-control").unwrap().contains("immutable"));
    assert_eq!(header(&hs, "etag"), Some(format!("\"{image}\"").as_str()));
    let (s, hs, bytes) = get_raw(&ts, &format!("/api/v1/docs/{doc}/markups/{strokes_hash}"), &ts.owner_token);
    assert_eq!(s, 200);
    assert_eq!(bytes, STROKES.as_bytes().trim_ascii_end());
    assert_eq!(header(&hs, "content-type"), Some("application/json"));
    // Another Document, the Markdown blob, a malformed hash: 404.
    for path in [
        format!("/api/v1/docs/{other}/markups/{image}"),
        format!("/api/v1/docs/{doc}/markups/{}", sha256_hex(MD.as_bytes())),
        format!("/api/v1/docs/{doc}/markups/x"),
    ] {
        let (s, _, _) = get_raw(&ts, &path, &ts.owner_token);
        assert_eq!(s, 404, "{path}");
    }
}

// spec: S49-06
#[test]
fn the_card_shows_the_markup_above_the_body() {
    let ts = TestServer::start();
    let doc = create(&ts, "mk-card");
    let (_, up) = upload(&ts, &ts.owner_token, &doc);
    let (_, t) = ts.post(
        &format!("/api/v1/docs/{doc}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "span", "range": "5:0-5:21", "markup": up["id"] }),
    );
    let tid = t["id"].as_str().unwrap().to_string();
    let image = up["image"].as_str().unwrap();
    let url = format!("/api/v1/docs/{doc}/markups/{image}");
    let page = |path: &str, embed: bool| {
        let mut req = ureq::get(format!("{}{}", ts.url, path))
            .header("Authorization", format!("Bearer {}", ts.owner_token))
            .header("Accept", "text/html");
        if embed {
            req = req.header("X-Annotary-Embed", "1");
        }
        req.call().unwrap().body_mut().read_to_string().unwrap()
    };
    let html = page(&format!("/d/{doc}"), false);
    let card = &html[html.find(&format!("data-t=\"{tid}\"")).expect("the card")..];
    let mk = card.find("class=\"cd-mk\"").expect("the Markup link");
    assert!(mk < card.find("class=\"cd-cbody\"").unwrap(), "the Markup comes before the body");
    assert!(card.contains(&format!("<a class=\"cd-mk\" href=\"{url}\" target=\"_blank\"")));
    assert!(card.contains(&format!("<img class=\"cd-mk-img\" src=\"{url}\" width=\"480\" height=\"160\"")));
    assert!(card.contains("Markup</div>"), "the narrow row says Markup for a root with no words");

    // The fragment draws the same card; an older Version's view too.
    let (_, frag) = ts.get(&format!("/d/{doc}/t/{tid}"), Some(&ts.owner_token));
    assert!(frag["html"].as_str().unwrap().contains(&format!("src=\"{url}\"")));
    let hash = sha256_hex(b"# Plan\n\nWe ship on Friday.\n\nThe second line goes.\n\nMore.\n");
    ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .send(&b"# Plan\n\nWe ship on Friday.\n\nThe second line goes.\n\nMore.\n"[..])
        .unwrap();
    let (s, _) = ts.post(
        &format!("/api/v1/docs/{doc}/versions"),
        Some(&ts.owner_token),
        &json!({ "name": "mk-card", "md_hash": hash }),
    );
    assert_eq!(s, 201);
    let old = page(&format!("/d/{doc}/v/1"), false);
    assert!(old.contains(&format!("<img class=\"cd-mk-img\" src=\"{url}\"")));

    // An embed render keeps the URL for the transport.
    let embed = page(&format!("/d/{doc}"), true);
    assert!(embed.contains(&format!("<img class=\"cd-mk-img\" data-cd-mk=\"{url}\" width=\"480\"")));
    assert!(!embed.contains(&format!("src=\"{url}\"")));
}
