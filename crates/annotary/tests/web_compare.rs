//! Compare Versions over HTTP against the in-process server: `GET /d/{id}/diff` (`compare.rs`) and
//! the Document page's Compare link (`view.rs`). The browser side is `tests/web/compare.spec.ts`.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::{Value, json};

fn upload(ts: &TestServer, bytes: &[u8]) -> String {
    let hash = sha256_hex(bytes);
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(bytes)
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    hash
}

fn create(ts: &TestServer, name: &str, md: &str, manifest: Value) -> String {
    let md_hash = upload(ts, md.as_bytes());
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&ts.owner_token),
        &json!({ "vault": "notes", "name": name, "md_hash": md_hash, "manifest": manifest }),
    );
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn push(ts: &TestServer, id: &str, name: &str, md: &str, manifest: Value) {
    let md_hash = upload(ts, md.as_bytes());
    let (status, v) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&ts.owner_token),
        &json!({ "name": name, "md_hash": md_hash, "manifest": manifest }),
    );
    assert_eq!(status, 201, "{v}");
}

/// A GET as the owner (Bearer), as a page: (status, body).
fn get(ts: &TestServer, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let mut req = ureq::get(format!("{}{path}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .header("Accept", "text/html");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut r = req.config().http_status_as_error(false).max_redirects(0).build().call().unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap_or_default())
}

const V1: &str = "# Compare\n\nWe ship on Friday.\n\nKept paragraph.\n";
const V2: &str = "# Compare\n\nWe ship on Monday.\n\nKept paragraph.\n";
const V3: &str = "# Compare\n\nWe ship on Monday.\n\nKept paragraph.\n\nA new closing line.\n";

fn three_versions(ts: &TestServer) -> String {
    let id = create(ts, "cmp", V1, json!([]));
    push(ts, &id, "cmp", V2, json!([]));
    push(ts, &id, "cmp", V3, json!([]));
    id
}

/// The value of the selected option of `<select id="{sel}">`.
fn selected(body: &str, sel: &str) -> String {
    let s = &body[body.find(&format!("<select id=\"{sel}\"")).expect(sel)..];
    let s = &s[..s.find("</select>").unwrap()];
    let at = s.find(" selected>").expect("a selected option");
    let opt = &s[s[..at].rfind("<option value=\"").unwrap() + 15..];
    opt[..opt.find('"').unwrap()].to_string()
}

fn doc_part(body: &str) -> &str {
    let at = body.find("<article class=\"cd-doc cd-dxdoc\"").expect("the diff");
    &body[at..at + body[at..].find("</article></main>").unwrap()]
}

// spec: S39-04
#[test]
fn the_diff_page_composes_the_pair_with_its_defaults() {
    let ts = TestServer::start();
    let id = three_versions(&ts);
    let (status, t) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "span", "range": "3:0-3:7", "body": "a card the Document page shows" }),
    );
    assert_eq!(status, 201, "{t}");
    // Neither: the previous Version against the head.
    let (status, body) = get(&ts, &format!("/d/{id}/diff"), &[]);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("class=\"cd-app") && body.contains("cd-diffpage"), "{body}");
    assert_eq!((selected(&body, "cd-dxfrom"), selected(&body, "cd-dxto")), ("2".into(), "3".into()));
    let d = doc_part(&body);
    assert!(
        d.contains(
            "class=\"cd-dx-add cd-dx-blk\" data-cd-dx=\"1\"><span class=\"cd-sr\">added: </span>A new closing line."
        ),
        "{d}"
    );
    assert!(body.contains("<b class=\"cd-dxplus\">+2</b> <b class=\"cd-dxminus\">\u{2212}0</b> lines"), "{body}");
    assert!(body.contains("id=\"cd-dxpos\" aria-live=\"polite\" title=\"blocks: 1 added\">1 change</span>"), "{body}");
    assert!(body.contains("id=\"cd-dxnext\""));
    assert!(body.contains("<title>cmp v2 → v3 — annotary</title>"), "{body}");
    assert!(!doc_part(&body).contains("cd-thread") && !doc_part(&body).contains("cd-strip"), "no Thread cards: {body}");
    // `from` alone: against the head; `to` alone: against the one before; reversed: oldest first.
    let (_, body) = get(&ts, &format!("/d/{id}/diff?from=1"), &[]);
    assert_eq!((selected(&body, "cd-dxfrom"), selected(&body, "cd-dxto")), ("1".into(), "3".into()));
    let d = doc_part(&body);
    assert!(d.contains("We ship on <del class=\"cd-del\">Friday</del><ins class=\"cd-ins\">Monday</ins>."), "{d}");
    let (_, body) = get(&ts, &format!("/d/{id}/diff?to=v2"), &[]);
    assert_eq!((selected(&body, "cd-dxfrom"), selected(&body, "cd-dxto")), ("1".into(), "2".into()));
    let (_, body) = get(&ts, &format!("/d/{id}/diff?from=3&to=1"), &[]);
    assert_eq!((selected(&body, "cd-dxfrom"), selected(&body, "cd-dxto")), ("1".into(), "3".into()));
    assert!(body.contains(&format!("id=\"cd-dxopen-from\" href=\"/d/{id}/v/1\"")), "{body}");
    assert!(body.contains(&format!("id=\"cd-dxopen-to\" href=\"/d/{id}\"")), "{body}");
    // The same Version twice: nothing to compare, the selects still there.
    let (status, body) = get(&ts, &format!("/d/{id}/diff?from=2&to=2"), &[]);
    assert_eq!(status, 200);
    assert!(body.contains("v2 and v2 are the same Version."), "{body}");
    // An unknown or malformed Version: the HTML 404 page.
    for q in ["from=9", "to=0", "from=abc"] {
        let (status, body) = get(&ts, &format!("/d/{id}/diff?{q}"), &[]);
        assert_eq!(status, 404, "{q}: {body}");
        assert!(body.starts_with("<!doctype html>"), "{q}: {body}");
    }
    let (status, _) = get(&ts, "/d/nosuchid/diff", &[]);
    assert_eq!(status, 404);
    // One Version: nothing to compare, no selects.
    let one = create(&ts, "single", V1, json!([]));
    let (status, body) = get(&ts, &format!("/d/{one}/diff"), &[]);
    assert_eq!(status, 200);
    assert!(body.contains("Nothing to compare: single has one Version."), "{body}");
    assert!(!body.contains("id=\"cd-dxfrom\""));
    // Same content: said, nothing marked.
    let same = create(&ts, "same", V1, json!([]));
    push(&ts, &same, "same", V2, json!([]));
    push(&ts, &same, "same", V1, json!([]));
    let (_, body) = get(&ts, &format!("/d/{same}/diff?from=1&to=3"), &[]);
    assert!(body.contains("v3 has the same content as v1."), "{body}");
    assert!(!doc_part(&body).contains("data-cd-dx="), "{body}");
}

// spec: S39-04
#[test]
fn an_embed_render_of_the_diff_keeps_media_for_the_bridge() {
    let ts = TestServer::start();
    let a = b"\x89PNG\r\n\x1a\nfirst image".to_vec();
    let b = b"\x89PNG\r\n\x1a\nsecond image".to_vec();
    let entry = |bytes: &[u8]| json!([{ "ref": "img/shot.png", "hash": sha256_hex(bytes), "size": bytes.len(), "type": "image/png" }]);
    upload(&ts, &a);
    upload(&ts, &b);
    let md = "# Shots\n\n![shot](img/shot.png)\n";
    let id = create(&ts, "shots", md, entry(&a));
    push(&ts, &id, "shots", md, entry(&b));
    let (status, body) = get(&ts, &format!("/d/{id}/diff"), &[("X-Annotary-Embed", "1")]);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("cd-embed") && body.contains("cd-diffpage"), "{body}");
    let d = doc_part(&body);
    // The old image's paragraph removed, the new one's added, neither loading by itself in the frame.
    assert!(
        d.contains(&format!("<span class=\"cd-sr\">removed: </span><img data-cd-src=\"/a/{}\"", sha256_hex(&a))),
        "{d}"
    );
    assert!(
        d.contains(&format!("<span class=\"cd-sr\">added: </span><img data-cd-src=\"/a/{}\"", sha256_hex(&b))),
        "{d}"
    );
    assert!(!d.contains(" src=\"/a/"), "{d}");
    // An ordinary render keeps the URLs.
    let (_, body) = get(&ts, &format!("/d/{id}/diff"), &[]);
    assert!(doc_part(&body).contains(&format!(" src=\"/a/{}\"", sha256_hex(&b))), "{body}");
}

// spec: S39-05
#[test]
fn the_document_page_links_compare_only_with_more_than_one_version() {
    let ts = TestServer::start();
    let one = create(&ts, "single", V1, json!([]));
    let (_, body) = get(&ts, &format!("/d/{one}"), &[]);
    assert!(!body.contains("id=\"cd-compare\""), "{body}");
    let id = three_versions(&ts);
    let (_, body) = get(&ts, &format!("/d/{id}"), &[]);
    assert!(
        body.contains(&format!("<a class=\"cd-btn\" id=\"cd-compare\" href=\"/d/{id}/diff?from=2&amp;to=3\" title=\"Show what changed from v2 to v3\">Compare</a>")),
        "{body}"
    );
    let (_, body) = get(&ts, &format!("/d/{id}/v/1"), &[]);
    assert!(
        body.contains(&format!("href=\"/d/{id}/diff?from=1&amp;to=3\" title=\"Show what changed from v1 to v3\"")),
        "{body}"
    );
    let (_, body) = get(&ts, &format!("/d/{id}/v/2"), &[("X-Annotary-Embed", "1")]);
    assert!(body.contains(&format!("href=\"/d/{id}/diff?from=2&amp;to=3\"")), "an embed render too: {body}");
}
