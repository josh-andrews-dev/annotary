//! Compare Versions over HTTP against the in-process server: `GET /d/{id}/diff` (`compare.rs`), its
//! JSON twin `GET /api/v1/docs/{id}/diff`, and the Document page's Compare link (`view.rs`). The
//! browser side is `tests/web/compare.spec.ts`.

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

// spec: S40-04
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
    // An unknown Version on a one-Version Document is still the 404 page.
    for q in ["from=9", "to=9", "from=2&to=3"] {
        let (status, body) = get(&ts, &format!("/d/{one}/diff?{q}"), &[]);
        assert_eq!(status, 404, "{q}: {body}");
        assert!(body.starts_with("<!doctype html>"), "{q}");
    }
    // Same content: said, nothing marked.
    let same = create(&ts, "same", V1, json!([]));
    push(&ts, &same, "same", V2, json!([]));
    push(&ts, &same, "same", V1, json!([]));
    let (_, body) = get(&ts, &format!("/d/{same}/diff?from=1&to=3"), &[]);
    assert!(body.contains("v3 has the same content as v1."), "{body}");
    assert!(!doc_part(&body).contains("data-cd-dx="), "{body}");
}

// spec: S40-04
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

// spec: S40-05
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

// spec: S40-09
#[test]
fn the_page_says_when_blocks_are_shown_whole() {
    let ts = TestServer::start();
    let para = |w: &str| {
        format!(
            "# Long\n\n{}\n",
            (0..11_000).map(|i| format!("word{i}{w}")).collect::<Vec<_>>().join(" ").replacen("word5 ", "word5x ", 1)
        )
    };
    let (a, b) = (para(""), para("").replace("word10 ", "ten "));
    let id = create(&ts, "long", &a, json!([]));
    push(&ts, &id, "long", &b, json!([]));
    let (status, body) = get(&ts, &format!("/d/{id}/diff"), &[]);
    assert_eq!(status, 200);
    assert!(doc_part(&body).contains("id=\"cd-dxwhole\""), "the note");
    assert!(doc_part(&body).contains("<ins class=\"cd-ins\">"), "the newer side, whole");
    // An ordinary comparison has no such note.
    let id = three_versions(&ts);
    let (_, body) = get(&ts, &format!("/d/{id}/diff"), &[]);
    assert!(!doc_part(&body).contains("cd-dxwhole"));
}

// spec: S40-12
#[test]
fn a_repeated_comparison_answers_the_same_page() {
    let ts = TestServer::start();
    let id = three_versions(&ts);
    let path = format!("/d/{id}/diff?from=1&to=3");
    let (s1, first) = get(&ts, &path, &[]);
    let (s2, second) = get(&ts, &path, &[]);
    assert_eq!((s1, s2), (200, 200));
    assert_eq!(doc_part(&first), doc_part(&second));
}

/// `GET /api/v1/docs/{id}/diff?{q}` as the owner: (status, JSON).
fn api(ts: &TestServer, id: &str, q: &str) -> (u16, Value) {
    ts.get(
        &format!("/api/v1/docs/{id}/diff{}", if q.is_empty() { String::new() } else { format!("?{q}") }),
        Some(&ts.owner_token),
    )
}

// spec: S46-01
#[test]
fn the_api_answers_the_pages_pair_diff_and_numbers_as_json() {
    let ts = TestServer::start();
    let id = three_versions(&ts);
    // Neither: the previous Version against the head, every field there.
    let (status, j) = api(&ts, &id, "");
    assert_eq!(status, 200, "{j}");
    let keys: Vec<&str> = j.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["id", "vault", "from", "to", "head", "same_content", "html", "summary"], "{j}");
    assert_eq!((j["id"].as_str(), j["vault"].as_str()), (Some(id.as_str()), Some("notes")));
    assert_eq!((j["from"].as_u64(), j["to"].as_u64(), j["head"].as_u64()), (Some(2), Some(3), Some(3)));
    assert_eq!(j["same_content"], json!(false));
    assert_eq!(
        j["summary"],
        json!({ "lines_added": 2, "lines_removed": 0, "blocks_added": 1, "blocks_removed": 0, "blocks_changed": 0,
                "changes": 1, "shown_whole": false, "too_large": false })
    );
    let html = j["html"].as_str().unwrap();
    assert!(
        html.contains(
            "class=\"cd-dx-add cd-dx-blk\" data-cd-dx=\"1\"><span class=\"cd-sr\">added: </span>A new closing line."
        ),
        "{html}"
    );
    assert!(!html.contains("cd-dx-note") && !html.contains("cd-dxdoc"), "no page chrome: {html}");
    // `from` alone, `to` alone, reversed: the page's pairs.
    for (q, pair) in [("from=1", (1, 3)), ("to=v2", (1, 2)), ("from=3&to=1", (1, 3)), ("from=v1&to=2", (1, 2))] {
        let (status, j) = api(&ts, &id, q);
        assert_eq!(status, 200, "{q}: {j}");
        assert_eq!((j["from"].as_u64().unwrap(), j["to"].as_u64().unwrap()), pair, "{q}");
    }
    let (_, j) = api(&ts, &id, "from=1");
    assert!(
        j["html"]
            .as_str()
            .unwrap()
            .contains("We ship on <del class=\"cd-del\">Friday</del><ins class=\"cd-ins\">Monday</ins>."),
        "{j}"
    );
    // The same Version twice: nothing compared.
    let (status, j) = api(&ts, &id, "from=2&to=2");
    assert_eq!(status, 200);
    assert_eq!(
        (j["from"].as_u64(), j["to"].as_u64(), &j["html"], &j["summary"]),
        (Some(2), Some(2), &Value::Null, &Value::Null)
    );
    // An unknown or malformed Version, or Document: 404 JSON, as reading a Version.
    for q in ["from=9", "to=0", "from=abc"] {
        let (status, j) = api(&ts, &id, q);
        assert_eq!((status, j["error"].as_str()), (404, Some("not_found")), "{q}: {j}");
    }
    let (status, j) = api(&ts, "nosuchid", "");
    assert_eq!((status, j["error"].as_str()), (404, Some("not_found")), "{j}");
    // One Version: nothing to compare; an unknown Version there is still 404.
    let one = create(&ts, "single", V1, json!([]));
    let (status, j) = api(&ts, &one, "");
    assert_eq!(status, 200);
    assert_eq!((j["from"].as_u64(), j["to"].as_u64(), j["head"].as_u64()), (Some(1), Some(1), Some(1)));
    assert_eq!((&j["html"], &j["summary"]), (&Value::Null, &Value::Null));
    assert_eq!(api(&ts, &one, "to=2").0, 404);
    // Same content: said, nothing marked.
    let same = create(&ts, "same", V1, json!([]));
    push(&ts, &same, "same", V2, json!([]));
    push(&ts, &same, "same", V1, json!([]));
    let (_, j) = api(&ts, &same, "from=1&to=3");
    assert_eq!((j["same_content"].as_bool(), j["summary"]["changes"].as_u64()), (Some(true), Some(0)), "{j}");
    assert!(!j["html"].as_str().unwrap().contains("data-cd-dx="), "{j}");
    // A long unchanged run is folded, marked for a client to fold too.
    let long = |last: &str| {
        format!("# Long\n\n{}{last}\n", (1..=12).map(|i| format!("Paragraph {i}.\n\n")).collect::<String>())
    };
    let folded = create(&ts, "folded", &long("The end."), json!([]));
    push(&ts, &folded, "folded", &long("The new end."), json!([]));
    let (_, j) = api(&ts, &folded, "");
    assert!(j["html"].as_str().unwrap().contains("<details class=\"cd-dx-same\">"), "{j}");
    // Too large to compare: no HTML, the line counts still there.
    let big = |w: &str| (0..20_001).map(|i| format!("p{i}{w}\n\n")).collect::<String>();
    let huge = create(&ts, "huge", &big(""), json!([]));
    push(&ts, &huge, "huge", &big("x"), json!([]));
    let (status, j) = api(&ts, &huge, "");
    assert_eq!(status, 200, "{j}");
    assert_eq!((&j["html"], &j["summary"]["too_large"]), (&Value::Null, &json!(true)), "{}", j["summary"]);
    assert_eq!(j["summary"]["lines_added"].as_u64(), Some(20_001));
}

/// The page's numbers above the diff: (+lines, −lines, changes).
fn page_numbers(body: &str) -> (u64, u64, u64) {
    let after = |pre: &str, end: char| -> u64 {
        let s = &body[body.find(pre).unwrap_or_else(|| panic!("{pre}: {body}")) + pre.len()..];
        s[..s.find(end).unwrap()].parse().unwrap()
    };
    (
        after("<b class=\"cd-dxplus\">+", '<'),
        after("<b class=\"cd-dxminus\">\u{2212}", '<'),
        after("data-changes=\"", '"'),
    )
}

// spec: S46-02
#[test]
fn the_page_and_the_api_show_the_same_diff_for_the_corpus() {
    let ts = TestServer::start();
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus");
    let mut compared = 0;
    for name in ["glossary-multilingual", "onboarding", "rfc-tile-cache"] {
        let md = |n: u32| std::fs::read_to_string(corpus.join(name).join(format!("v{n}.md"))).unwrap();
        let id = create(&ts, name, &md(1), json!([]));
        for n in 2..=4 {
            push(&ts, &id, name, &md(n), json!([]));
        }
        let mut queries = vec![String::new(), "from=2".into(), "to=3".into()];
        for a in 1..=4 {
            for b in 1..=4 {
                if a != b {
                    queries.push(format!("from={a}&to={b}"));
                }
            }
        }
        for q in &queries {
            let (status, j) = api(&ts, &id, q);
            assert_eq!(status, 200, "{name} {q}: {j}");
            let (status, page) = get(&ts, &format!("/d/{id}/diff?{q}"), &[]);
            assert_eq!(status, 200, "{name} {q}");
            let (from, to) = (j["from"].as_u64().unwrap(), j["to"].as_u64().unwrap());
            assert_eq!(
                (selected(&page, "cd-dxfrom"), selected(&page, "cd-dxto")),
                (from.to_string(), to.to_string()),
                "{name} {q}: the pair"
            );
            // The page's diff is the API's fragment plus only the page's own additions.
            let html = annotary::server::ui::asset_links_blank(j["html"].as_str().unwrap());
            let d = doc_part(&page);
            let d = &d[d.find('>').unwrap() + 1..];
            let mut rest =
                d.strip_suffix(html.as_str()).unwrap_or_else(|| panic!("{name} {q}: the page's diff differs"));
            while let Some(after) = rest.strip_prefix("<p class=\"cd-dx-note\"") {
                rest = &after[after.find("</p>").unwrap() + 4..];
            }
            assert_eq!(rest, "", "{name} {q}: only notes ahead of the diff");
            let s = &j["summary"];
            let api_numbers = (
                s["lines_added"].as_u64().unwrap(),
                s["lines_removed"].as_u64().unwrap(),
                s["changes"].as_u64().unwrap(),
            );
            assert_eq!(page_numbers(&page), api_numbers, "{name} {q}: the numbers");
            assert!(api_numbers.2 > 0, "{name} {q}: corpus Versions differ");
            compared += 1;
        }
    }
    assert_eq!(compared, 45);
}
