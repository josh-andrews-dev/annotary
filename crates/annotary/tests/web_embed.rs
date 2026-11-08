//! M3 of the Hermes plan: the Document list and the Document view rendered for an embedder
//! (`X-Annotary-Embed: 1`, `server/embed.rs`), the shell route and the parts of a large answer.
//! The browser side (the fake embedder driving a real `serve`) is `tests/web/embed.spec.ts`.

use annotary::server::embed::{IMAGE_MAX, PART_MAX, cuts};
use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::{Value, json};

const MD: &str = "# Embedded\n\nAlpha paragraph to anchor a Thread on.\n\n![shot](img/shot.png)\n\n![big](img/big.png)\n\n![clip](media/clip.mp4)\n\n![spec](docs/spec.pdf)\n\n[notes](notes.txt)\n";

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

fn entry(r: &str, bytes: &[u8], ty: &str) -> Value {
    json!({ "ref": r, "hash": sha256_hex(bytes), "size": bytes.len(), "type": ty })
}

/// A Document with an image, an image over the cap, a video, a PDF, a text file, and an open Thread.
fn seed(ts: &TestServer) -> (String, Value) {
    let small = b"\x89PNG\r\n\x1a\nsmall image bytes".to_vec();
    let big = vec![7u8; IMAGE_MAX as usize + 1];
    let clip = b"\0\0\0\x18ftypmp42 not really a movie".to_vec();
    let pdf = b"%PDF-1.4 not really a pdf".to_vec();
    let txt = b"plain notes".to_vec();
    let md_hash = upload(ts, MD.as_bytes());
    for b in [&small, &big, &clip, &pdf, &txt] {
        upload(ts, b);
    }
    let manifest = json!([
        entry("img/shot.png", &small, "image/png"),
        entry("img/big.png", &big, "image/png"),
        entry("media/clip.mp4", &clip, "video/mp4"),
        entry("docs/spec.pdf", &pdf, "application/pdf"),
        entry("notes.txt", &txt, "text/plain"),
    ]);
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&ts.owner_token),
        &json!({ "vault": "notes", "name": "embedded", "md_hash": md_hash, "manifest": manifest }),
    );
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    let (status, t) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "span", "range": "3:0-3:5", "body": "an open one" }),
    );
    assert_eq!(status, 201, "{t}");
    (id, manifest)
}

struct Page {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Page {
    fn header(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
}

/// A GET with a Bearer Token (what an embedder's backend sends) and extra headers.
fn get(ts: &TestServer, path: &str, headers: &[(&str, &str)]) -> Page {
    let mut req = ureq::get(format!("{}{path}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .header("Accept", "text/html");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut r = req.config().http_status_as_error(false).max_redirects(0).build().call().unwrap();
    let headers =
        r.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string())).collect();
    let status = r.status().as_u16();
    let body = r.body_mut().with_config().limit(64 << 20).read_to_string().unwrap_or_default();
    Page { status, headers, body }
}

const EMBED: (&str, &str) = ("X-Annotary-Embed", "1");

/// The `<div …>` that opens `.cd-app`.
fn app_tag(body: &str) -> &str {
    let at = body.find("<div class=\"cd-app").expect(".cd-app");
    &body[at..at + body[at..].find('>').unwrap()]
}

// spec: S37-01
#[test]
fn an_embed_render_is_self_contained_and_an_ordinary_one_is_unchanged() {
    let ts = TestServer::start();
    let (id, _) = seed(&ts);
    // a second Version, so there is an older-Version page
    let (status, v2) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&ts.owner_token),
        &json!({ "name": "embedded", "md_hash": upload(&ts, b"# Embedded\n\nv2\n"), "manifest": [] }),
    );
    assert_eq!(status, 201, "{v2}");
    for path in ["/", &format!("/d/{id}"), &format!("/d/{id}/v/1")] {
        let e = get(&ts, path, &[EMBED]);
        assert_eq!(e.status, 200, "{path}: {}", e.body);
        assert!(app_tag(&e.body).contains(" cd-embed"), "{path}: {}", app_tag(&e.body));
        assert_eq!(e.header("content-security-policy"), Some("sandbox allow-scripts"), "{path}");
        assert_eq!(e.header("vary"), Some("X-Annotary-Embed"), "{path}");
        // the transport, a classic script in the head, before app.js (a module at the end)
        let core = e.body.find("window.AnnotaryEmbed = {").expect("embed.js inlined");
        assert!(core < e.body.find("<body>").unwrap(), "{path}: embed.js in the head");
        assert!(!e.body.contains("rel=\"icon\""), "{path}: no icon links");
        assert!(!e.body.contains("rel=\"manifest\""), "{path}");
        assert!(!e.body.contains("id=\"cd-openapp\""), "{path}: no Open in Annotary");
        assert!(!e.body.contains("id=\"cd-nav-settings\""), "{path}: no Settings link");
        assert!(!e.body.contains("action=\"/logout\""), "{path}: no Sign out");

        let o = get(&ts, path, &[]);
        assert_eq!(o.status, 200, "{path}");
        assert!(!app_tag(&o.body).contains("cd-embed"), "{path}");
        assert_eq!(o.header("content-security-policy"), None, "{path}");
        assert_eq!(o.header("vary"), Some("X-Annotary-Embed"), "{path}");
        assert!(!o.body.contains("window.AnnotaryEmbed = {"), "{path}: no transport in a browser page");
        assert!(o.body.contains("rel=\"icon\""), "{path}: icons");
    }
    let list = get(&ts, "/", &[]);
    assert!(list.body.contains("id=\"cd-nav-settings\""), "Settings on an ordinary list");
    let doc = get(&ts, &format!("/d/{id}"), &[]);
    assert!(doc.body.contains("id=\"cd-openapp\""), "Open in Annotary on an ordinary page");
}

// spec: S37-01
#[test]
fn an_embedded_list_has_no_sign_out_even_with_a_password() {
    let ts = TestServer::start_locked();
    let e = get(&ts, "/", &[EMBED]);
    assert_eq!(e.status, 200, "{}", e.body);
    assert!(!e.body.contains("action=\"/logout\""));
    assert!(get(&ts, "/", &[]).body.contains("action=\"/logout\""), "Sign out in a browser");
}

// spec: S37-02
#[test]
fn an_embed_renders_media_for_the_bridge() {
    let ts = TestServer::start();
    let (id, manifest) = seed(&ts);
    let hash = |i: usize| manifest[i]["hash"].as_str().unwrap().to_string();
    let e = get(&ts, &format!("/d/{id}"), &[EMBED]).body;
    let doc = &e[e.find("<article class=\"cd-doc\"").unwrap()..e.rfind("</article>").unwrap()];
    // no media element points anywhere by itself
    for tag in ["<img", "<video", "<audio", "<object"] {
        for (at, _) in doc.match_indices(tag) {
            let t = &doc[at..at + doc[at..].find('>').unwrap()];
            assert!(!t.contains(" src=\"/a/") && !t.contains(" data=\"/a/"), "{t}");
        }
    }
    // the image: data-cd-src, the button hidden after it
    let small = format!("data-cd-src=\"/a/{}\"", hash(0));
    let at = doc.find(&small).expect("the image keeps its URL as data-cd-src");
    let img = &doc[doc[..at].rfind("<img").unwrap()..];
    let img_tag = &img[..img.find('>').unwrap() + 1];
    assert!(!img_tag.contains("data-cd-big"), "{img_tag}");
    let after = &img[img_tag.len()..];
    assert!(
        after.starts_with(&format!(
            "<button type=\"button\" class=\"cd-embed-open\" data-cd-open=\"/a/{}\" data-cd-kind=\"image\" aria-label=\"Open image\" title=\"Open image\" hidden></button>",
            hash(0)
        )),
        "{after}"
    );
    // the image over the cap: data-cd-big, the button shown
    let at = doc.find(&format!("data-cd-src=\"/a/{}\"", hash(1))).unwrap();
    let big = &doc[doc[..at].rfind("<img").unwrap()..];
    let big_tag = &big[..big.find('>').unwrap() + 1];
    assert!(big_tag.contains(" data-cd-big=\"1\""), "{big_tag}");
    assert!(big[big_tag.len()..].starts_with("<button type=\"button\" class=\"cd-embed-open\""), "{big}");
    assert!(!big[big_tag.len()..big.find("</button>").unwrap()].contains(" hidden"), "{big}");
    // video and PDF: the button after the element (its fallback link kept), shown
    for (i, tag, kind) in [(2, "video", "video"), (3, "object", "PDF")] {
        let at = doc.find(&format!("data-cd-src=\"/a/{}\"", hash(i))).unwrap_or_else(|| panic!("{tag}"));
        let el = &doc[doc[..at].rfind(&format!("<{tag}")).unwrap()..];
        let close = el.find(&format!("</{tag}>")).unwrap() + tag.len() + 3;
        assert!(el[..close].contains(&format!("href=\"/a/{}\"", hash(i))), "the fallback link: {el}");
        assert!(
            el[close..].starts_with(&format!(
                "<button type=\"button\" class=\"cd-embed-open\" data-cd-open=\"/a/{}\" data-cd-kind=\"{kind}\" aria-label=\"Open {kind}\" title=\"Open {kind}\"></button>",
                hash(i)
            )),
            "{}",
            &el[close..]
        );
    }
    // the button has no text node: the text the selection mapping reads is unchanged
    assert!(!doc.contains("Open image</button>"));
    // Threads are spliced and the cards are the same markup as an ordinary render
    let o = get(&ts, &format!("/d/{id}"), &[]).body;
    let card = |b: &str| {
        let at = b.find("<div class=\"cd-strip").expect("a strip");
        b[at..at + b[at..].find("</div></div>").unwrap()].to_string()
    };
    assert_eq!(card(&e), card(&o));
    assert!(o.contains(&format!("<img src=\"/a/{}\"", hash(0))), "an ordinary render keeps src");
}

// spec: S37-03
#[test]
fn an_embed_render_takes_the_zone_from_headers_never_a_cookie() {
    let ts = TestServer::start();
    let (id, _) = seed(&ts);
    let res = ureq::put(format!("{}/api/v1/me/settings", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(json!({ "key": "times", "value": "absolute", "scope": "user" }))
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let path = format!("/d/{id}");
    let zone = [EMBED, ("X-Annotary-Tz", "Asia/Tokyo"), ("X-Annotary-Hc", "24")];
    let e = get(&ts, &path, &zone).body;
    assert!(app_tag(&e).contains("data-cd-tz=\"Asia/Tokyo\""), "{}", app_tag(&e));
    assert!(e.contains(" JST\">"), "times rendered in the header's zone");
    // a cookie is not read in embed mode
    let c = get(&ts, &path, &[EMBED, ("Cookie", "cd_tz=Europe%2FBerlin; cd_hc=24")]).body;
    assert!(!app_tag(&c).contains("data-cd-tz"), "{}", app_tag(&c));
    // a bad zone is ignored, as a bad cookie is
    let bad = get(&ts, &path, &[EMBED, ("X-Annotary-Tz", "Not/AZone")]).body;
    assert!(!app_tag(&bad).contains("data-cd-tz"));
    // an ordinary request ignores the headers and reads the cookie
    let o = get(&ts, &path, &[("X-Annotary-Tz", "Asia/Tokyo"), ("Cookie", "cd_tz=Europe%2FBerlin")]).body;
    assert!(app_tag(&o).contains("data-cd-tz=\"Europe/Berlin\""), "{}", app_tag(&o));
}

// spec: S37-04
#[test]
fn the_shell_is_the_bootstrap_page_in_a_sandbox() {
    let ts = TestServer::start();
    let s = get(&ts, "/embed/shell", &[]);
    assert_eq!(s.status, 200, "{}", s.body);
    assert!(s.header("content-type").unwrap().starts_with("text/html"));
    assert_eq!(s.header("content-security-policy"), Some("sandbox allow-scripts"));
    assert_eq!(s.header("x-content-type-options"), Some("nosniff"));
    assert!(s.body.contains("<meta name=\"annotary-embed-protocol\" content=\"1\">"), "{}", s.body);
    assert!(s.body.contains("window.AnnotaryEmbed = {"), "the transport");
    assert!(s.body.contains("window.AnnotaryEmbed.boot()"), "the boot call");
    assert!(!s.body.contains("type=\"module\""), "no app.js: the page brings its own");
}

/// Every part of an embedded GET, as the embedder would get them: (status, headers, body) each.
fn all_parts(ts: &TestServer, path: &str) -> Vec<Page> {
    let first = get(ts, path, &[EMBED]);
    let n: usize = first.header("x-annotary-parts").map(|p| p.parse().unwrap()).unwrap_or(1);
    let etag = first.header("x-annotary-etag").unwrap_or_default().to_string();
    let mut out = vec![first];
    for i in 1..n {
        let k = i.to_string();
        out.push(get(ts, path, &[EMBED, ("X-Annotary-Part", &k), ("X-Annotary-Etag", &etag)]));
    }
    out
}

// spec: S37-05
#[test]
fn a_large_embedded_answer_comes_in_parts() {
    let ts = TestServer::start();
    // several Members, as every Hermes install has (one per profile): the page must render the
    // same bytes on every request, or each part would carry another etag and be refused
    for h in ["piper", "coder", "research", "scribe", "bencha", "benchb", "wfi", "ops"] {
        let (status, body) = ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": h }));
        assert_eq!(status, 201, "{body}");
    }
    // a Document whose page is far over one part, with multi-byte text across the cuts
    let mut md = String::from("# Large\n\n");
    while md.len() < 3 * PART_MAX {
        md.push_str("Ein größerer Absatz — 東京の段落 — with plenty of text in it to cut.\n\n");
    }
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&ts.owner_token),
        &json!({ "vault": "notes", "name": "large", "md_hash": upload(&ts, md.as_bytes()) }),
    );
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap();
    let path = format!("/d/{id}");
    let whole = get(&ts, &path, &[]);
    assert_eq!(whole.status, 200);
    assert!(whole.body.len() > PART_MAX);

    let parts = all_parts(&ts, &path);
    assert!(parts.len() >= 3, "{} parts", parts.len());
    let etag = parts[0].header("x-annotary-etag").unwrap().to_string();
    for (i, p) in parts.iter().enumerate() {
        assert_eq!(p.status, 206, "part {i}");
        assert_eq!(p.header("x-annotary-status"), Some("200"), "part {i}");
        assert_eq!(p.header("x-annotary-part"), Some(i.to_string().as_str()));
        assert_eq!(p.header("x-annotary-parts"), Some(parts.len().to_string().as_str()));
        assert_eq!(p.header("x-annotary-etag"), Some(etag.as_str()));
        assert!(p.header("content-type").unwrap().starts_with("text/html"), "the content type is kept");
        assert!(p.body.len() <= PART_MAX, "part {i}: {} bytes", p.body.len());
    }
    // the parts join to the page (each one valid UTF-8 on its own: `get` read each as text)
    let joined: String = parts.iter().map(|p| p.body.as_str()).collect();
    assert!(joined.starts_with("<!doctype html>") && joined.ends_with("</html>\n"), "the whole page");
    assert!(joined.contains(" cd-embed"));
    assert_eq!(joined.matches("東京の段落").count(), whole.body.matches("東京の段落").count());

    // a part asked with another etag (the answer changed) is refused
    let stale = get(&ts, &path, &[EMBED, ("X-Annotary-Part", "1"), ("X-Annotary-Etag", "0000")]);
    assert_eq!(stale.status, 412, "{}", stale.body);
    // a part past the end too
    let past = parts.len().to_string();
    let p = get(&ts, &path, &[EMBED, ("X-Annotary-Part", &past), ("X-Annotary-Etag", &etag)]);
    assert_eq!(p.status, 412);

    // the Markdown blob (bytes) comes in parts as well, and joins to the same bytes
    let blob = all_parts(&ts, &format!("/api/v1/blobs/{}", sha256_hex(md.as_bytes())));
    assert!(blob.len() >= 3);
    assert_eq!(blob.iter().map(|p| p.body.as_str()).collect::<String>(), md);

    // small answers, ordinary requests and non-GETs are untouched
    let small = get(&ts, "/", &[EMBED]);
    assert_eq!(small.status, 200);
    assert_eq!(small.header("x-annotary-parts"), None);
    assert_eq!(whole.header("x-annotary-parts"), None);
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/threads"), Some(&ts.owner_token), &json!({}));
    assert_ne!(status, 206);
}

// spec: S37-05
#[test]
fn parts_end_on_character_boundaries() {
    let text = "aé東".repeat(1000);
    for max in [1, 2, 3, 7, 100] {
        let cs = cuts(text.as_bytes(), max);
        assert_eq!(cs.first().unwrap().0, 0);
        assert_eq!(cs.last().unwrap().1, text.len());
        for w in cs.windows(2) {
            assert_eq!(w[0].1, w[1].0, "contiguous");
        }
        for (a, b) in &cs {
            assert!(text.is_char_boundary(*a) && text.is_char_boundary(*b));
            assert!(b - a <= max.max(4), "{max}: {}", b - a);
        }
    }
    // bytes that are not UTF-8 are cut at the size
    let bytes = vec![0xffu8; 10];
    assert_eq!(cuts(&bytes, 4), vec![(0, 4), (4, 8), (8, 10)]);
    assert_eq!(cuts(&[], 4), vec![(0, 0)]);
}
