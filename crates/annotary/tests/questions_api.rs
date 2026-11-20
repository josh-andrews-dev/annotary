//! Questions against the in-process server: `PUT …/questions/{key}/answer` and its refusals, the
//! event, `GET …/questions` (time-slice, `changed`), the commit's `questions` (computed when
//! absent, refused under `--verify-edge` when wrong), the server-side `answers_missing`, the
//! Question's Thread, the Sidecar, and purge.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::{Value, json};

const PLAN: &str = "# Plan\n\n| Decision | Options | Recommended | Decided |\n|---|---|---|---|\n| D1 | A, B | A | |\n\n## D1. Where it runs\n\n- A. The box.\n- B. A VM.\n";

fn upload(ts: &TestServer, md: &str) -> String {
    let hash = sha256_hex(md.as_bytes());
    let r = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .content_type("application/octet-stream")
        .send(md.as_bytes())
        .unwrap();
    assert!(r.status().is_success());
    hash
}

fn create(ts: &TestServer, name: &str, md: &str) -> String {
    let hash = upload(ts, md);
    let (s, d) =
        ts.post("/api/v1/docs", Some(&ts.owner_token), &json!({ "vault": "notes", "name": name, "md_hash": hash }));
    assert_eq!(s, 201, "{d}");
    d["id"].as_str().unwrap().to_string()
}

fn push(ts: &TestServer, id: &str, md: &str, extra: Value) -> (u16, Value) {
    let hash = upload(ts, md);
    let mut body = json!({ "name": "plan", "md_hash": hash });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    ts.post(&format!("/api/v1/docs/{id}/versions"), Some(&ts.owner_token), &body)
}

fn put(ts: &TestServer, token: &str, path: &str, body: &Value) -> (u16, Value) {
    let mut r = ureq::put(format!("{}{}", ts.url, path))
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(body)
        .unwrap();
    let status = r.status().as_u16();
    (status, r.body_mut().read_json().unwrap_or(Value::Null))
}

fn answer(ts: &TestServer, token: &str, id: &str, key: &str, option: Value, version: u32) -> (u16, Value) {
    put(
        ts,
        token,
        &format!("/api/v1/docs/{id}/questions/{key}/answer"),
        &json!({ "option": option, "version": version }),
    )
}

// spec: S50-04
#[test]
fn answers_are_one_per_member_with_their_refusals_and_event() {
    let ts = TestServer::start();
    let id = create(&ts, "plan", PLAN);
    let tok = ts.owner_token.clone();
    let (s, v) = answer(&ts, &tok, &id, "D1", json!("B"), 1);
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["option"].as_str(), v["changed"].as_bool()), (Some("B"), Some(true)));
    assert_eq!(v["question"]["answers"][0]["by"], "rowan");
    // The same again: nothing written, no event.
    let before = ts.store.events_since(0).unwrap().len();
    let (_, v) = answer(&ts, &tok, &id, "D1", json!("B"), 1);
    assert_eq!(v["changed"], false);
    assert_eq!(ts.store.events_since(0).unwrap().len(), before);
    // Refusals.
    assert_eq!(answer(&ts, &tok, &id, "D9", json!("A"), 1).0, 404);
    let (s, v) = answer(&ts, &tok, &id, "D1", json!("Z"), 1);
    assert_eq!((s, v["error"].as_str()), (400, Some("invalid_answer")), "{v}");
    let (s, v) = answer(&ts, &tok, &id, "D1", json!("A"), 7);
    assert_eq!((s, v["error"].as_str()), (409, Some("not_head")), "{v}");
    // `other` and withdrawing.
    assert_eq!(answer(&ts, &tok, &id, "D1", json!("other"), 1).1["option"], "other");
    let (_, v) = answer(&ts, &tok, &id, "D1", Value::Null, 1);
    assert!(v["option"].is_null() && v["question"]["answers"].as_array().unwrap().is_empty(), "{v}");
    // The event: `question.answered {key, option, version}`, not a `wait` kind, never a mention.
    let ev: Vec<_> = ts.store.events_since(0).unwrap().into_iter().filter(|e| e.kind == "question.answered").collect();
    assert_eq!(ev.len(), 3);
    assert_eq!(ev[0].payload, json!({ "key": "D1", "option": "B", "version": 1 }));
    let (_, body) = ts.get(&format!("/api/v1/events?since=0&doc={id}&kinds=version,action,mention"), Some(&tok));
    assert!(body["events"].as_array().unwrap().iter().all(|e| e["type"] != "question.answered"), "{body}");
    // A reader without `comment` may not answer; a deleted Document refuses.
    let (mid, reader) = ts.add_member("reader");
    let read = annotary_store::Grant { vault: "notes".into(), rights: vec![annotary_store::Right::Read] };
    ts.store.set_grants(&ts.owner_id, &mid, vec![read]).unwrap();
    assert_eq!(answer(&ts, &reader, &id, "D1", json!("A"), 1).0, 403);
    let (s, _) = ts.get(&format!("/api/v1/docs/{id}/questions"), Some(&reader));
    assert_eq!(s, 200);
    ts.store.delete_document(&ts.owner_id, &id).unwrap();
    let (s, v) = answer(&ts, &tok, &id, "D1", json!("A"), 1);
    assert_eq!((s, v["error"].as_str()), (409, Some("deleted")), "{v}");
}

// spec: S50-03, S50-06, S50-07
#[test]
fn commits_carry_questions_and_the_server_enforces_the_recording_rule() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let id = create(&ts, "plan", PLAN);
    // Created without `questions` (an older CLI): the server computed them.
    let (_, v) = ts.get(&format!("/api/v1/docs/{id}/versions/1"), Some(&tok));
    assert_eq!(v["questions"][0]["key"], "D1");
    answer(&ts, &tok, &id, "D1", json!("A"), 1);
    // A wrong `questions` is refused while the server verifies the edge…
    let edited = PLAN.replace("The box.", "The box!");
    let (s, v) = push(&ts, &id, &edited, json!({ "base_version": 1, "questions": [], "discard_answers": true }));
    assert_eq!((s, v["error"].as_str()), (400, Some("invalid_questions")), "{v}");
    let (s, v) = push(&ts, &id, &edited, json!({ "base_version": 1, "questions": {"x": 1} }));
    assert_eq!((s, v["error"].as_str()), (400, Some("invalid_questions")), "{v}");
    // … the server refuses a file without the Answers from a client that did not check …
    let (s, v) = push(&ts, &id, &edited, json!({ "base_version": 1 }));
    assert_eq!((s, v["error"].as_str()), (409, Some("answers_missing")), "{v}");
    assert_eq!(v["missing"][0]["text"].as_str().unwrap().starts_with("| D1 | A, B | A | A (@rowan, "), true, "{v}");
    // … and takes one whose row changed its options (the answer then reads as changed).
    let reworded = edited.replace("| D1 | A, B | A | |", "| D1 | A, B, C | A | |");
    let (s, v) = push(&ts, &id, &reworded, json!({ "base_version": 1 }));
    assert_eq!(s, 201, "{v}");
    let (_, q) = ts.get(&format!("/api/v1/docs/{id}/questions"), Some(&tok));
    assert_eq!(q["questions"][0]["answers"][0]["changed"], true, "{q}");
    // The time-slice: v1 shows the Answer as it stood, unchanged there.
    let (_, q1) = ts.get(&format!("/api/v1/docs/{id}/questions?version=1"), Some(&tok));
    assert_eq!(q1["questions"][0]["answers"][0]["changed"], false, "{q1}");
    // With verification off, a sent `questions` of the right shape is stored as sent.
    ts.store.set_verify_edge(false);
    let filled = reworded.replace("| D1 | A, B, C | A | |", "| D1 | A, B, C | A | A |");
    let qs = annotary_renderer::questions::to_json(&annotary_renderer::questions::find(&filled).questions);
    let (s, _) = push(&ts, &id, &filled, json!({ "base_version": 2, "questions": qs }));
    assert_eq!(s, 201);
    let (s, v) = answer(&ts, &tok, &id, "D1", json!("B"), 3);
    assert_eq!((s, v["error"].as_str()), (409, Some("decided")), "{v}");
}

// spec: S50-09, S50-15
#[test]
fn a_question_thread_the_sidecar_and_purge() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let id = create(&ts, "plan", PLAN);
    // The first Comment on D1 creates a span Thread at its place (line 7, the heading).
    let (s, t) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&tok),
        &json!({ "subject": "span", "question": "D1", "body": "Neither, really", "version": 1 }),
    );
    assert_eq!(s, 201, "{t}");
    assert_eq!(t["question"], "D1");
    assert_eq!(t["anchor"]["original"]["range"], "7:0-7:20");
    let (s, v) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&tok),
        &json!({ "subject": "span", "question": "D1", "body": "again", "version": 1 }),
    );
    assert_eq!((s, v["error"].as_str()), (409, Some("conflict")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("t1"), "{v}");
    let (s, _) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&tok),
        &json!({ "subject": "span", "question": "D7", "body": "x", "version": 1 }),
    );
    assert_eq!(s, 404);
    // Other on D1: the Decided text names the Thread.
    answer(&ts, &tok, &id, "D1", json!("other"), 1);
    let (_, q) = ts.get(&format!("/api/v1/docs/{id}/questions"), Some(&tok));
    assert_eq!(q["questions"][0]["thread"], "t1");
    assert_eq!(ts.store.decided_fills(&id).unwrap()[0].1.starts_with("Other: t1 (@rowan, "), true);
    // A Question asked on a Thread, answered by label.
    let (s, t2) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&tok),
        &json!({ "subject": "document", "body": "Ship?", "options": ["Yes", "No"], "version": 1 }),
    );
    assert_eq!(s, 201, "{t2}");
    assert_eq!(t2["options"], json!([{ "id": "A", "label": "Yes" }, { "id": "B", "label": "No" }]));
    assert_eq!(answer(&ts, &tok, &id, "t2", json!("no"), 1).1["option"], "B");
    let (s, v) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&tok),
        &json!({ "subject": "document", "body": "x", "options": ["Yes"], "version": 1 }),
    );
    assert_eq!(s, 400, "{v}");
    // The Sidecar carries both Threads' fields and the Answers.
    let r = ureq::get(format!("{}/api/v1/docs/{id}/sidecar", ts.url))
        .header("Authorization", format!("Bearer {tok}"))
        .call()
        .unwrap()
        .into_body()
        .read_to_string()
        .unwrap();
    let sc = annotary_sidecar::parse(&r).unwrap();
    assert_eq!(sc.threads[0].question.as_deref(), Some("D1"));
    assert_eq!(sc.threads[1].options, ["Yes", "No"]);
    let keys: Vec<(&str, &str)> = sc.answers.iter().map(|a| (a.key.as_str(), a.option.as_str())).collect();
    assert_eq!(keys, [("D1", "other"), ("t2", "B")]);
    // Purge removes the rows.
    ts.store.delete_document(&ts.owner_id, &id).unwrap();
    ts.store.purge_document(&ts.owner_id, &id).unwrap();
    assert!(ts.store.answers(&id, None).unwrap().is_empty());
}
