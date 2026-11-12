//! The web's Document additions, server side, over HTTP against the in-process server: the link
//! the page copies (`#cd-data.link`, the same as `annotary link`), the More menu's items per
//! viewer and Version, Copy link on every card, Delete and Restore for `write` only and never in
//! an embed render, the list's Restore, and the Mentions page. The browser behavior is
//! `tests/web/document-additions.spec.ts`.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{Grant, Right};
use serde_json::{Value, json};

const V1: &str = "# Plan\n\nShip a local server + CLI + web viewer.\n\nWe ship the CLI before the web UI so agents can start using it early.\n";
const V2: &str = "# Plan\n\nShip a local server + CLI + web viewer.\n\nWe ship the CLI well before the web UI so agents can start using it early.\n";
const EMBED: (&str, &str) = ("X-Annotary-Embed", "1");

fn upload(ts: &TestServer, token: &str, md: &[u8]) -> String {
    let hash = sha256_hex(md);
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(md)
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    hash
}

fn create(ts: &TestServer, token: &str, vault: &str, name: &str, md: &str) -> String {
    let hash = upload(ts, token, md.as_bytes());
    let (status, doc) = ts.post("/api/v1/docs", Some(token), &json!({ "vault": vault, "name": name, "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn push(ts: &TestServer, token: &str, id: &str, name: &str, md: &str) {
    let hash = upload(ts, token, md.as_bytes());
    let (status, v) =
        ts.post(&format!("/api/v1/docs/{id}/versions"), Some(token), &json!({ "name": name, "md_hash": hash }));
    assert_eq!(status, 201, "{v}");
}

fn thread(ts: &TestServer, token: &str, id: &str, body: Value) -> String {
    let (status, t) = ts.post(&format!("/api/v1/docs/{id}/threads"), Some(token), &body);
    assert_eq!(status, 201, "{t}");
    t["id"].as_str().unwrap().to_string()
}

fn delete(ts: &TestServer, token: &str, id: &str) {
    let r = ureq::delete(format!("{}/api/v1/docs/{id}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
}

/// A browser-like GET with a bearer and extra headers: (status, body).
fn get_with(ts: &TestServer, token: &str, path: &str, extra: &[(&str, &str)]) -> (u16, String) {
    let mut req = ureq::get(format!("{}{path}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "text/html")
        .header("Host", "annotary.example.com:7377");
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let mut r = req.config().http_status_as_error(false).max_redirects(0).build().call().unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap_or_default())
}

fn get(ts: &TestServer, token: &str, path: &str) -> String {
    let (status, body) = get_with(ts, token, path, &[]);
    assert_eq!(status, 200, "{path}: {body}");
    body
}

fn cd_data(html: &str) -> Value {
    const OPEN: &str = "<script type=\"application/json\" id=\"cd-data\">";
    let start = html.find(OPEN).expect("cd-data") + OPEN.len();
    let end = html[start..].find("</script>").unwrap() + start;
    serde_json::from_str(html[start..end].replace("<\\/", "</").as_str()).expect("cd-data parses")
}

fn grant(ts: &TestServer, member_id: &str, rights: Vec<Right>) {
    ts.store.set_grants(&ts.owner_id, member_id, vec![Grant { vault: "notes".into(), rights }]).unwrap();
}

// spec: S48-01
#[test]
fn the_page_carries_the_link_annotary_link_prints_and_every_card_copies_it() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (reader_id, reader) = ts.add_member("reader");
    grant(&ts, &reader_id, vec![Right::Read]);
    let id = create(&ts, &owner, "notes", "plan", V1);
    let t1 = thread(&ts, &owner, &id, json!({ "subject": "span", "range": "5:0-5:33", "body": "Order?" }));

    // No public URL: the page's own origin, as `annotary link` falls back to its server's URL.
    let html = get(&ts, &owner, &format!("/d/{id}"));
    assert_eq!(cd_data(&html)["link"], format!("http://annotary.example.com:7377/d/{id}"));
    assert!(html.contains("data-doc-act=\"link\">Copy link</button>"), "the More menu has Copy link");

    // A public URL, then the relay too: exactly `annotary_store::doc_link`, the CLI's helper.
    ts.store.set_public_url(None, Some("https://annotary.example.ts.net")).unwrap();
    let html = get(&ts, &owner, &format!("/d/{id}"));
    assert_eq!(cd_data(&html)["link"], format!("https://annotary.example.ts.net/d/{id}"));
    ts.store.set_link_base(None, Some("https://go.example.com")).unwrap();
    let html = get(&ts, &owner, &format!("/d/{id}"));
    let want =
        annotary_store::doc_link(Some("https://annotary.example.ts.net"), Some("https://go.example.com"), &id, None)
            .unwrap();
    assert_eq!(cd_data(&html)["link"], want);
    assert_eq!(want, format!("https://go.example.com/annotary.example.ts.net/d/{id}"));
    // The Thread link is the Document link + `#tN`, as `annotary link DOC tN` prints it.
    assert_eq!(
        annotary_store::doc_link(
            Some("https://annotary.example.ts.net"),
            Some("https://go.example.com"),
            &id,
            Some(&t1)
        )
        .unwrap(),
        format!("{want}#{t1}")
    );

    // Every card copies its link, for a reader with no Reply or Resolve too; the fragment as well.
    let html = get(&ts, &reader, &format!("/d/{id}"));
    assert!(html.contains("data-act=\"link\">Copy link</button>"), "a reader's card has Copy link");
    assert!(!html.contains("data-act=\"reply\">Reply</button>"), "a reader cannot reply");
    let frag: Value = serde_json::from_str(&get(&ts, &owner, &format!("/d/{id}/t/{t1}"))).unwrap();
    assert!(frag["html"].as_str().unwrap().contains("data-act=\"link\">Copy link</button>"), "{frag}");

    // In an embed render too (the Hermes page shares the Document page).
    let (_, html) = get_with(&ts, &owner, &format!("/d/{id}"), &[EMBED]);
    assert_eq!(cd_data(&html)["link"], want);
    assert!(html.contains("data-doc-act=\"link\">Copy link") && html.contains("data-act=\"link\">Copy link"));

    // A deleted Document has no link to give (`annotary link` refuses it).
    delete(&ts, &owner, &id);
    let html = get(&ts, &owner, &format!("/d/{id}"));
    assert!(cd_data(&html)["link"].is_null());
    assert!(!html.contains(">Copy link</button>"), "no Copy link anywhere");
}

// spec: S48-02
#[test]
fn comment_on_the_whole_document_is_in_the_menu_on_the_head_for_a_commenter_only() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (reader_id, reader) = ts.add_member("reader");
    grant(&ts, &reader_id, vec![Right::Read]);
    let id = create(&ts, &owner, "notes", "plan", V1);
    const ITEM: &str = "data-doc-act=\"comment\">Comment on the whole document</button>";
    assert!(get(&ts, &owner, &format!("/d/{id}")).contains(ITEM));
    assert!(!get(&ts, &reader, &format!("/d/{id}")).contains(ITEM), "a reader cannot comment");
    push(&ts, &owner, &id, "plan", V2);
    assert!(get(&ts, &owner, &format!("/d/{id}")).contains(ITEM), "the new head");
    assert!(!get(&ts, &owner, &format!("/d/{id}/v/1")).contains(ITEM), "new Threads only on the head");
    // What the composer posts: a Document-level Thread, pinned in the Whole document strip.
    let t = thread(&ts, &owner, &id, json!({ "subject": "document", "body": "Needs a Risks section.", "version": 2 }));
    let html = get(&ts, &owner, &format!("/d/{id}"));
    let strip = html.find("cd-strip cd-strip-doc").expect("the Whole document strip");
    assert!(html[strip..].find(&format!("data-t=\"{t}\"")).is_some());
    delete(&ts, &owner, &id);
    assert!(!get(&ts, &owner, &format!("/d/{id}")).contains(ITEM), "a deleted Document is read-only");
}

// spec: S48-04
#[test]
fn delete_and_restore_are_for_write_only_and_never_in_an_embed_render() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (commenter_id, commenter) = ts.add_member("commenter");
    grant(&ts, &commenter_id, vec![Right::Read, Right::Comment]);
    let (writer_id, writer) = ts.add_member("writer");
    grant(&ts, &writer_id, vec![Right::Read, Right::Comment, Right::Write]);
    let id = create(&ts, &owner, "notes", "plan", V1);
    const DEL: &str = "data-doc-act=\"delete\">Delete Document";
    const CONFIRM: &str = "data-doc-act=\"delete-yes\">Delete</button>";
    for tok in [&owner, &writer] {
        let html = get(&ts, tok, &format!("/d/{id}"));
        assert!(html.contains(DEL) && html.contains(CONFIRM), "write deletes, after a confirm");
        assert_eq!(cd_data(&html)["can_write"], true);
    }
    let html = get(&ts, &commenter, &format!("/d/{id}"));
    assert!(!html.contains(DEL), "comment is not write");
    assert_eq!(cd_data(&html)["can_write"], false);
    let (_, html) = get_with(&ts, &owner, &format!("/d/{id}"), &[EMBED]);
    assert!(!html.contains(DEL), "the Hermes page keeps the review loop");
    assert_eq!(cd_data(&html)["can_write"], false);

    // Deleted: read-only, with Restore for write, and the list's Deleted filter restores in one click.
    delete(&ts, &writer, &id);
    const RESTORE: &str = "id=\"cd-restore\">Restore</button>";
    let html = get(&ts, &writer, &format!("/d/{id}"));
    assert!(html.contains(RESTORE) && html.contains("<span class=\"cd-chip cd-warn\">deleted</span>"));
    assert!(!html.contains(DEL));
    assert!(!get(&ts, &commenter, &format!("/d/{id}")).contains(RESTORE));
    let (_, html) = get_with(&ts, &owner, &format!("/d/{id}"), &[EMBED]);
    assert!(!html.contains(RESTORE));
    let row_restore = format!("class=\"cd-btn cd-sm cd-restore\" data-restore=\"{id}\">Restore</button>");
    assert!(get(&ts, &writer, "/?deleted=1").contains(&row_restore));
    let list = get(&ts, &commenter, "/?deleted=1");
    assert!(list.contains(&format!("data-id=\"{id}\"")) && !list.contains(&row_restore), "{list}");
    let (_, list) = get_with(&ts, &owner, "/?deleted=1", &[EMBED]);
    assert!(list.contains(&format!("data-id=\"{id}\"")) && !list.contains(&row_restore));

    // What Restore sends brings it back: live again, Delete in the menu.
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/restore"), Some(&writer), &json!({}));
    assert_eq!(status, 200);
    let html = get(&ts, &writer, &format!("/d/{id}"));
    assert!(html.contains(DEL) && !html.contains(RESTORE));
}

// spec: S48-03
#[test]
fn the_mentions_page_lists_open_threads_that_mention_me_newest_first() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (agent_id, agent) = ts.add_member("scribe");
    grant(&ts, &agent_id, vec![Right::Read, Right::Comment]);
    let other = ts.create_vault("private");
    let a = create(&ts, &owner, "notes", "alpha", V1);
    let b = create(&ts, &owner, "notes", "beta", V1);
    let c = create(&ts, &owner, &other, "gamma", V1);
    let ta = thread(
        &ts,
        &agent,
        &a,
        json!({ "subject": "span", "range": "5:0-5:33", "body": "Is the **order** right? @rowan" }),
    );
    let tb = thread(&ts, &agent, &b, json!({ "subject": "document", "body": "@rowan please read" }));
    let tr = thread(&ts, &agent, &b, json!({ "subject": "document", "body": "@rowan resolved one" }));
    let (st, _) = ts.post(&format!("/api/v1/docs/{b}/threads/{tr}/resolve"), Some(&owner), &json!({}));
    assert_eq!(st, 200);
    let _tn = thread(&ts, &agent, &a, json!({ "subject": "document", "body": "no mention" }));
    // The oldest Thread gets the newest activity: it comes first.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (st, _) = ts.post(
        &format!("/api/v1/docs/{a}/threads/{ta}/replies"),
        Some(&agent),
        &json!({ "body": "Still waiting on this." }),
    );
    assert_eq!(st, 201);
    // A Vault the scribe cannot read: not on the scribe's page.
    let tc = thread(&ts, &owner, &c, json!({ "subject": "document", "body": "@scribe secret" }));

    let html = get(&ts, &owner, "/mentions");
    assert!(html.contains("cd-mentionspage") && html.contains("id=\"cd-nav-mentions\""), "the top bar links it");
    let rows: Vec<&str> = html.match_indices("<li class=\"cd-mrow\"").map(|(i, _)| &html[i..]).collect();
    assert_eq!(rows.len(), 2, "open mentions only: {html}");
    assert!(rows[0].starts_with(&format!(
        "<li class=\"cd-mrow\" data-doc=\"{a}\" data-t=\"{ta}\"><a class=\"cd-mlink\" href=\"/d/{a}#{ta}\">"
    )));
    assert!(rows[1].starts_with(&format!("<li class=\"cd-mrow\" data-doc=\"{b}\" data-t=\"{tb}\">")));
    let first = &rows[0][..rows[0].find("</li>").unwrap()];
    assert!(first.contains("<b class=\"cd-docname\">alpha</b>") && first.contains("notes · t1 · 1 reply"), "{first}");
    assert!(first.contains("<q>We ship the CLI before the web UI</q>"), "{first}");
    assert!(first.contains("Still waiting on this."), "the last Comment: {first}");
    assert!(rows[1].contains("<span class=\"cd-kind\">whole document</span>"));
    assert!(html.contains("2 Threads</p>"));
    // The API agrees on which Threads count.
    let (_, api) = ts.get("/api/v1/mentions", Some(&owner));
    let mut ids: Vec<String> =
        api["mentions"].as_array().unwrap().iter().map(|m| m["thread"]["id"].as_str().unwrap().to_string()).collect();
    ids.sort();
    let mut want = vec![ta.clone(), tb.clone()];
    want.sort();
    assert_eq!(ids, want);

    let html = get(&ts, &agent, "/mentions");
    assert!(!html.contains(&format!("data-doc=\"{c}\"")), "the scribe cannot read {other} ({tc})");
    assert!(html.contains("No open Threads mention you.") && html.contains("0 Threads</p>"));

    // An embed render has it and its top bar link (the Hermes page shares it); a deleted Document drops out.
    let (status, html) = get_with(&ts, &owner, "/mentions", &[EMBED]);
    assert_eq!(status, 200);
    assert!(html.contains("cd-embed") && html.contains("id=\"cd-nav-mentions\"") && html.contains(&ta));
    delete(&ts, &owner, &a);
    let html = get(&ts, &owner, "/mentions");
    assert!(!html.contains(&format!("data-doc=\"{a}\"")) && html.contains("1 Thread</p>"));
}
