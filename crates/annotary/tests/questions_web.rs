//! Questions on the web pages, in Rust (the browser tests are `tests/web/questions.spec.ts`): the
//! controls spliced into the first paint, the header count, a decided Question, the read-only
//! states, the card of a Question's Thread, and pages without Questions left as they were.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::{Value, json};

const PLAN: &str = "# Plan\n\n| Decision | Options | Recommended | Decided |\n|---|---|---|---|\n| D1 | A, B | A | |\n| D2 | A, B | B | done: B |\n\n## D1. Where it runs\n\n- A. The existing box.\n- B. A new VM.\n";

fn upload(ts: &TestServer, md: &str) -> String {
    let hash = sha256_hex(md.as_bytes());
    ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .content_type("application/octet-stream")
        .send(md.as_bytes())
        .unwrap();
    hash
}

fn create(ts: &TestServer, name: &str, md: &str) -> String {
    let hash = upload(ts, md);
    let (_, d) =
        ts.post("/api/v1/docs", Some(&ts.owner_token), &json!({ "vault": "notes", "name": name, "md_hash": hash }));
    d["id"].as_str().unwrap().to_string()
}

fn page(ts: &TestServer, path: &str) -> String {
    let mut r = ureq::get(format!("{}{path}", ts.url))
        .header("Accept", "text/html")
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    r.body_mut().read_to_string().unwrap()
}

/// The page up to the end of the Document (the inlined script holds the same class names).
fn rendered(html: &str) -> &str {
    &html[..html.find("</article></main>").unwrap()]
}

fn answer(ts: &TestServer, id: &str, key: &str, option: &str, version: u32) {
    let r = ureq::put(format!("{}/api/v1/docs/{id}/questions/{key}/answer", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .send_json(json!({ "option": option, "version": version }))
        .unwrap();
    assert!(r.status().is_success());
}

// spec: S50-11, S50-12
#[test]
fn the_controls_are_in_the_first_paint() {
    let ts = TestServer::start();
    let id = create(&ts, "plan", PLAN);
    answer(&ts, &id, "D1", "B", 1);
    let html = page(&ts, &format!("/d/{id}"));
    let doc = rendered(&html);
    // The row at D1's place, the viewer's Answer pressed, "rec" on A, the who-chose text.
    assert!(doc.contains("<div class=\"cd-qx cd-qrow\" data-q=\"D1\" role=\"group\""), "{doc}");
    assert!(doc.contains("data-o=\"B\" aria-pressed=\"true\""), "{doc}");
    assert!(doc.contains("<span class=\"cd-qrec\">rec</span>"));
    assert!(doc.contains("<span class=\"cd-qwho\" aria-live=\"polite\">B @rowan</span>"));
    assert!(doc.contains("Other\u{2026}</button>") && doc.contains("data-act=\"qcomment\""));
    // A radio at each Option's own list item, named by its text, B checked.
    assert!(doc.contains("<li data-l=\"10-10\"><input type=\"radio\" class=\"cd-qx cd-qradio\" name=\"cd-q-D1\" data-q=\"D1\" data-o=\"A\" aria-label=\"D1: A. The existing box.\">"), "{doc}");
    assert!(doc.contains("data-o=\"B\" aria-label=\"D1: B. A new VM.\" checked>"), "{doc}");
    // The Decided cell of D1 holds the Answers; D2's cell is the Version's text, decided, no controls.
    assert!(doc.contains("<span class=\"cd-qx cd-qcell\" data-q=\"D1\""), "{doc}");
    assert!(!doc.contains("data-q=\"D2\" role"), "{doc}");
    assert!(doc.contains("done: B"));
    // The header: 2 of 2 answered (D1 by me, D2 decided); nothing left to accept.
    assert!(html.contains("<span class=\"cd-chip cd-qsum\" id=\"cd-qsum\">2 of 2 answered</span>"));
    assert!(!html.contains("id=\"cd-qaccept\""));
    // The script and styles come along, and the data names the Questions.
    assert!(html.contains("cd-qaccept") && html.contains(".cd-qrow{"));
    let start = html.find("id=\"cd-data\">").unwrap() + 13;
    let data: Value = serde_json::from_str(&html[start..start + html[start..].find("</script>").unwrap()]).unwrap();
    assert_eq!(data["questions"][0]["key"], "D1");
}

// spec: S50-12
#[test]
fn read_only_states_and_pages_without_questions() {
    let ts = TestServer::start();
    let id = create(&ts, "plan", PLAN.replace("| D2 | A, B | B | done: B |", "| D2 | A, B | B | |").as_str());
    // Accept recommended counts the undecided Questions with a Recommended I have not answered.
    let html = page(&ts, &format!("/d/{id}"));
    assert!(html.contains("0 of 2 answered") && html.contains("Accept recommended (2)"), "{html}");
    answer(&ts, &id, "D1", "A", 1);
    // A new Version: v1 is read-only, its controls disabled, its Answers shown as they stood.
    let hash = upload(&ts, &format!("{PLAN}\nMore.\n"));
    let (s, v) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&ts.owner_token),
        &json!({ "name": "plan", "md_hash": hash, "base_version": 1, "discard_answers": true }),
    );
    assert_eq!(s, 201, "{v}");
    let old = page(&ts, &format!("/d/{id}/v/1"));
    let doc = rendered(&old);
    assert!(doc.contains("aria-pressed=\"true\" disabled"), "{doc}");
    assert!(!old.contains("id=\"cd-qaccept\""));
    // A deleted Document: read-only too.
    ts.store.delete_document(&ts.owner_id, &id).unwrap();
    let del = page(&ts, &format!("/d/{id}"));
    assert!(rendered(&del).contains(" disabled>"), "{del}");
    // Without Questions: no controls, no Question script or styles, on the Document and the list.
    let plain = create(&ts, "notes", "# Notes\n\nNothing to decide.\n");
    let html = page(&ts, &format!("/d/{plain}"));
    assert!(!html.contains("cd-qaccept") && !html.contains(".cd-qrow{") && !html.contains("cd-qsum"));
    let list = page(&ts, "/");
    assert!(!list.contains("cd-qaccept") && !list.contains(".cd-qrow{"));
}

// spec: S50-09, S50-12
#[test]
fn a_question_threads_card_leads_with_its_answers() {
    let ts = TestServer::start();
    let id = create(&ts, "plan", PLAN);
    answer(&ts, &id, "D1", "A", 1);
    let (s, _) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "span", "question": "D1", "body": "Why A?", "version": 1 }),
    );
    assert_eq!(s, 201);
    let (s, _) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "document", "body": "Ship?", "options": ["Yes", "Wait"], "version": 1 }),
    );
    assert_eq!(s, 201);
    answer(&ts, &id, "t2", "B", 1);
    let html = page(&ts, &format!("/d/{id}"));
    assert!(html.contains("<div class=\"cd-qx cd-qans\" data-q=\"D1\"><span class=\"cd-qkey\">D1</span> <span class=\"cd-qwho\">A @rowan</span></div>"), "{html}");
    assert!(html.contains("<div class=\"cd-qx cd-qans\" data-q=\"t2\" role=\"group\""));
    assert!(
        html.contains("Wait</button>")
            && html.contains("<span class=\"cd-qwho\" aria-live=\"polite\">Wait @rowan</span>")
    );
    // The fragment draws the same card.
    let r = ureq::get(format!("{}/d/{id}/t/t2", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .call()
        .unwrap()
        .into_body()
        .read_to_string()
        .unwrap();
    let frag: Value = serde_json::from_str(&r).unwrap();
    assert!(frag["html"].as_str().unwrap().contains("Wait @rowan"), "{frag}");
}
