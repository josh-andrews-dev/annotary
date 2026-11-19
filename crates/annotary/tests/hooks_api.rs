//! Hooks over HTTP against the in-process server and a local receiver: registration and its
//! refusals, the lists, delivery (signed, once, body-free, rights re-checked, own events
//! skipped), retry and disable, `once`, expiry, the test route and shutdown. The rights matrix
//! rows are in `access_api.rs`; the store's pure helpers are unit-tested in `hooks.rs`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use annotary::server::hooks::HookConfig;
use annotary::testing::TestServer;
use annotary_store::hooks::signature;
use annotary_store::{NewThread, PushInput};
use serde_json::{Value, json};

/// One request the receiver got.
#[derive(Clone, Debug)]
struct Got {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Got {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

/// What the receiver answers: a status per request in turn (the last one repeats), or no answer.
#[derive(Clone)]
enum Reply {
    Codes(Vec<u16>),
    Hang,
}

/// A local HTTP receiver on 127.0.0.1 that records every request.
struct Receiver {
    url: String,
    got: Arc<Mutex<Vec<Got>>>,
}

impl Receiver {
    fn start(reply: Reply) -> Receiver {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/hook/tok3n", listener.local_addr().unwrap().port());
        let got: Arc<Mutex<Vec<Got>>> = Arc::default();
        let seen = got.clone();
        std::thread::spawn(move || {
            let mut n = 0usize;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut headers = Vec::new();
                let mut line = String::new();
                reader.read_line(&mut line).unwrap_or(0);
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = line.trim_end().split_once(':') {
                        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                    }
                }
                let len =
                    headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap_or(());
                seen.lock().unwrap().push(Got { headers, body });
                match &reply {
                    Reply::Hang => {
                        // Hold the connection open, answer nothing.
                        std::thread::sleep(Duration::from_secs(60));
                    }
                    Reply::Codes(codes) => {
                        let code = codes[n.min(codes.len() - 1)];
                        n += 1;
                        let _ = write!(stream, "HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        Receiver { url, got }
    }

    fn count(&self) -> usize {
        self.got.lock().unwrap().len()
    }

    fn all(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }

    /// Wait until at least `n` requests arrived (or panic after `secs`).
    fn wait_for(&self, n: usize, secs: u64) -> Vec<Got> {
        let end = Instant::now() + Duration::from_secs(secs);
        while self.count() < n {
            assert!(Instant::now() < end, "receiver got {} of {n} requests", self.count());
            std::thread::sleep(Duration::from_millis(20));
        }
        self.all()
    }
}

fn local(backoff_ms: u64, retry_secs: u64) -> HookConfig {
    HookConfig {
        allow_hosts: vec!["127.0.0.1".into()],
        backoff_base: Duration::from_millis(backoff_ms),
        retry_for: Duration::from_secs(retry_secs),
    }
}

fn call(ts: &TestServer, method: &str, path: &str, token: &str, body: Option<Value>) -> (u16, Value) {
    let url = format!("{}{}", ts.url, path);
    let auth = format!("Bearer {token}");
    let res = match method {
        "GET" => ureq::get(&url).header("Authorization", &auth).config().http_status_as_error(false).build().call(),
        "DELETE" => {
            ureq::delete(&url).header("Authorization", &auth).config().http_status_as_error(false).build().call()
        }
        _ => ureq::post(&url)
            .header("Authorization", &auth)
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(body.unwrap_or(json!({}))),
    };
    let mut r = res.unwrap();
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// A Document by the owner in `notes`; returns its id.
fn doc(ts: &TestServer, name: &str) -> String {
    let md = format!("# {name}\n\nWe ship the CLI before the web UI.\n");
    let (hash, _) = ts.store.put_blob(md.as_bytes()).unwrap();
    let v = ts.store.vault_by_name("notes").unwrap().unwrap();
    ts.store.create_document(&ts.owner_id, &v.id, PushInput::new(name, hash)).unwrap().id
}

fn add(ts: &TestServer, token: &str, body: Value) -> (u16, Value) {
    call(ts, "POST", "/api/v1/me/hooks", token, Some(body))
}

fn events_of(ts: &TestServer, kind: &str) -> Vec<Value> {
    ts.store.events_since(0).unwrap().into_iter().filter(|e| e.kind == kind).map(|e| e.payload).collect()
}

// spec: S49-01, S49-02
#[test]
fn a_member_adds_lists_and_removes_its_own_hooks_and_never_sees_a_url() {
    let rx = Receiver::start(Reply::Codes(vec![200]));
    let ts = TestServer::start_hooks(local(50, 60));
    let (agent_id, agent) = ts.add_member("scribe");
    let (_, other) = ts.add_member("quill");
    let d = doc(&ts, "plan");

    let (st, body) = add(&ts, &agent, json!({ "url": rx.url, "on": "mention,action", "doc": d, "once": true }));
    assert_eq!(st, 201, "{body}");
    let secret = body["secret"].as_str().unwrap().to_string();
    assert!(secret.len() >= 32);
    let hook = &body["hook"];
    assert_eq!(hook["id"], "h1");
    assert_eq!(hook["member"], "scribe");
    assert_eq!(hook["doc"], d.as_str());
    assert_eq!(hook["name"], "plan");
    assert_eq!(hook["on"], json!(["action", "mention"]));
    assert_eq!(hook["once"], true);
    assert_eq!(hook["host"], "127.0.0.1");
    assert_eq!(hook["url_fp"], annotary_store::hooks::fingerprint(&rx.url));
    assert_eq!(hook["state"], "live");
    assert!(hook["expires_at"].as_str().is_some());
    assert!(!body["hook"].to_string().contains("tok3n"), "{body}");

    // The lists: own, and the owner's of everyone's; never the URL or the secret.
    let (st, mine) = call(&ts, "GET", "/api/v1/me/hooks", &agent, None);
    assert_eq!(st, 200);
    assert_eq!(mine["hooks"].as_array().unwrap().len(), 1);
    let (_, theirs) = call(&ts, "GET", "/api/v1/me/hooks", &other, None);
    assert_eq!(theirs["hooks"], json!([]));
    let (st, every) = call(&ts, "GET", "/api/v1/hooks", &ts.owner_token, None);
    assert_eq!(st, 200);
    assert_eq!(every["hooks"][0]["member"], "scribe");
    let (st, _) = call(&ts, "GET", "/api/v1/hooks", &agent, None);
    assert_eq!(st, 403);
    for text in [mine.to_string(), every.to_string()] {
        assert!(!text.contains("tok3n") && !text.contains(&secret), "{text}");
    }
    let added = events_of(&ts, "hook.added");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["member"], agent_id.as_str());
    assert!(!added[0].to_string().contains("tok3n") && !added[0].to_string().contains(&secret));

    // Refusals.
    let (st, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "reply" }));
    assert_eq!((st, b["error"].as_str()), (400, Some("invalid")), "{b}");
    let (st, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "action", "expires": 10 }));
    assert_eq!(st, 400, "{b}");
    let (st, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "action", "expires": 31 * 86400 }));
    assert_eq!(st, 400, "{b}");
    let (st, _) = add(&ts, &agent, json!({ "url": rx.url, "on": "action", "doc": "zzzzzzzz" }));
    assert_eq!(st, 404);

    // Someone else's hook is not there for a Member; an owner removes anyone's.
    let (st, _) = call(&ts, "DELETE", "/api/v1/me/hooks/h1", &other, None);
    assert_eq!(st, 404);
    let (st, _) = call(&ts, "POST", "/api/v1/me/hooks/h1/test", &ts.owner_token, None);
    assert_eq!(st, 404, "only the hook's Member tests it");
    let (st, b) = call(&ts, "DELETE", "/api/v1/me/hooks/h1", &ts.owner_token, None);
    assert_eq!((st, &b["removed"]), (200, &json!(true)), "{b}");
    let removed = events_of(&ts, "hook.removed");
    assert_eq!(removed[0]["reason"], "removed");
    let (st, _) = call(&ts, "DELETE", "/api/v1/me/hooks/h1", &agent, None);
    assert_eq!(st, 404);

    // At most 20 live hooks.
    for _ in 0..20 {
        let (st, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "action" }));
        assert_eq!(st, 201, "{b}");
    }
    let (st, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "action" }));
    assert_eq!((st, b["error"].as_str()), (409, Some("too_many_hooks")), "{b}");
}

// spec: S49-03
#[test]
fn private_and_plain_http_addresses_are_refused_unless_allowed() {
    let ts = TestServer::start();
    let (_, agent) = ts.add_member("scribe");
    // Inward ranges, built from octets: public text in this repo holds no private addresses.
    let inward: Vec<String> =
        [(10, 1, 2, 3), (192, 168, 1, 10), (169, 254, 169, 254), (100, 64, 1, 1), (172, 16, 0, 9)]
            .into_iter()
            .map(|(a, b, c, d)| format!("https://{}/x", std::net::Ipv4Addr::new(a, b, c, d)))
            .collect();
    let mut urls: Vec<String> = inward;
    urls.extend(
        [
            "http://127.0.0.1:9/x",
            "https://127.0.0.1:9/x",
            "https://localhost:9/x",
            "https://[::1]:9/x",
            "https://[fd00::1]/x",
            "https://u:p@1.1.1.1/x",
            "ftp://1.1.1.1/x",
            "not a url",
        ]
        .map(String::from),
    );
    for url in urls {
        let (st, b) = add(&ts, &agent, json!({ "url": url, "on": "action" }));
        assert_eq!((st, b["error"].as_str()), (409, Some("hook_address_refused")), "{url}: {b}");
    }
    // The same check runs inside the delivery's own lookup: a hook stored for a private
    // address (as one would be after its name re-pointed inward) is never called.
    let rx = Receiver::start(Reply::Codes(vec![200]));
    let secure = rx.url.replace("http://", "https://");
    let (agent_id, _) = (ts.store.member_by_handle("scribe").unwrap().unwrap().id, ());
    let d = doc(&ts, "plan");
    let row = ts
        .store
        .add_hook(annotary_store::NewHook {
            member_id: agent_id,
            token_id: None,
            doc_id: Some(d.clone()),
            kinds: vec!["action".into()],
            once: false,
            url: secure,
            expires_in: None,
        })
        .unwrap();
    ts.store.act(&ts.owner_id, &d, "send-back", None).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let h = ts.store.hook(row.n).unwrap().unwrap();
        if let Some(s) = h.last_status {
            assert!(s.starts_with("refused"), "{s}");
            break;
        }
        assert!(Instant::now() < end, "no attempt recorded");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(rx.count(), 0);
}

// spec: S49-04, S49-05, S49-07
#[test]
fn a_send_back_reaches_the_receiver_signed_once_with_no_comment_text() {
    let rx = Receiver::start(Reply::Codes(vec![200]));
    let ts = TestServer::start_hooks(local(50, 60));
    let (agent_id, agent) = ts.add_member("scribe");
    let d = doc(&ts, "plan");
    let other = doc(&ts, "other");
    let (_, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "action,mention", "doc": d, "once": true }));
    let secret = b["secret"].as_str().unwrap().to_string();

    // What it does not carry: the agent's own Action, a Version (not asked for), another
    // Document, a Thread that does not mention the agent.
    ts.store.act(&agent_id, &d, "send-back", None).unwrap();
    ts.store.act(&ts.owner_id, &other, "send-back", None).unwrap();
    ts.store.new_thread(&ts.owner_id, &d, NewThread::document("SECRETWORD the intro is long")).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(rx.count(), 0, "{:?}", rx.all());

    let taken = ts.store.act(&ts.owner_id, &d, "send-back", None).unwrap();
    let got = rx.wait_for(1, 5);
    let g = &got[0];
    assert_eq!(g.header("x-annotary-signature"), Some(signature(&secret, &g.body).as_str()));
    assert_eq!(g.header("x-annotary-hook"), Some("h1"));
    assert_eq!(g.header("content-type"), Some("application/json"));
    let body = g.json();
    assert_eq!(body["v"], 1);
    assert_eq!(body["kind"], "action");
    assert_eq!(body["action"], "send-back");
    assert_eq!(body["doc"], d.as_str());
    assert_eq!(body["name"], "plan");
    assert_eq!(body["version"], taken.version);
    assert_eq!(body["by"], "@rowan");
    assert_eq!(body["thread"], Value::Null);
    assert_eq!(body["hook"], "h1");
    assert_eq!(g.header("x-annotary-seq"), Some(body["seq"].to_string().as_str()));
    assert!(body["link"].as_str().unwrap().ends_with(&format!("/d/{d}")), "{body}");
    assert!(body["at"].as_str().is_some());
    assert!(!String::from_utf8_lossy(&g.body).contains("SECRETWORD"));

    // `once`: removed after the 2xx; later events go nowhere.
    let end = Instant::now() + Duration::from_secs(5);
    while ts.store.hook(1).unwrap().is_some() {
        assert!(Instant::now() < end, "once hook still there");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(events_of(&ts, "hook.removed")[0]["reason"], "once");
    ts.store.act(&ts.owner_id, &d, "approve", None).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(rx.count(), 1);
}

// spec: S49-04
#[test]
fn a_mention_wakes_with_its_thread_and_rights_are_checked_at_delivery() {
    let rx = Receiver::start(Reply::Codes(vec![200]));
    let ts = TestServer::start_hooks(local(50, 60));
    let (agent_id, agent) = ts.add_member("scribe");
    let d = doc(&ts, "plan");
    let (st, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "mention,version" }));
    assert_eq!(st, 201, "{b}");
    ts.store.new_thread(&ts.owner_id, &d, NewThread::document("@scribe SECRETWORD please look")).unwrap();
    let got = rx.wait_for(1, 5);
    let body = got[0].json();
    assert_eq!(body["kind"], "mention");
    assert_eq!(body["thread"], "t1");
    assert!(body["link"].as_str().unwrap().ends_with("#t1"), "{body}");
    assert!(!String::from_utf8_lossy(&got[0].body).contains("SECRETWORD"));

    // The Member loses `read`: nothing more is delivered, and the cursor moves on.
    ts.store.set_grants(&ts.owner_id, &agent_id, vec![]).unwrap();
    let (hash, _) = ts.store.put_blob(b"# plan\n\nv2\n").unwrap();
    ts.store.push_version(&ts.owner_id, &d, PushInput::new("plan", hash)).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(rx.count(), 1);
    let h = ts.store.hook(1).unwrap().unwrap();
    assert_eq!(h.cursor_seq, ts.store.last_seq().unwrap());
}

// spec: S49-06
#[test]
fn failures_retry_with_backoff_then_deliver_or_disable() {
    // 503, 503, then 200: the same event three times, then delivered.
    let rx = Receiver::start(Reply::Codes(vec![503, 503, 200]));
    let ts = TestServer::start_hooks(local(50, 60));
    let (_, agent) = ts.add_member("scribe");
    let d = doc(&ts, "plan");
    add(&ts, &agent, json!({ "url": rx.url, "on": "action", "doc": d }));
    ts.store.act(&ts.owner_id, &d, "approve", None).unwrap();
    let got = rx.wait_for(3, 10);
    let seqs: Vec<_> = got.iter().map(|g| g.json()["seq"].clone()).collect();
    assert_eq!(seqs[0], seqs[1]);
    assert_eq!(seqs[1], seqs[2]);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let h = ts.store.hook(1).unwrap().unwrap();
        if h.last_status.as_deref() == Some("200") {
            assert_eq!(h.failures, 0);
            assert!(h.disabled_at.is_none());
            break;
        }
        assert!(Instant::now() < end, "{h:?}");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Always 503 with a 1 s retry window: disabled as failing.
    let bad = Receiver::start(Reply::Codes(vec![503]));
    let ts2 = TestServer::start_hooks(local(200, 1));
    let (_, agent2) = ts2.add_member("scribe");
    let d2 = doc(&ts2, "plan");
    add(&ts2, &agent2, json!({ "url": bad.url, "on": "action", "doc": d2 }));
    ts2.store.act(&ts2.owner_id, &d2, "approve", None).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while events_of(&ts2, "hook.disabled").is_empty() {
        assert!(Instant::now() < end, "never disabled; {} attempts", bad.count());
        std::thread::sleep(Duration::from_millis(50));
    }
    let dis = &events_of(&ts2, "hook.disabled")[0];
    assert_eq!(dis["reason"], "failing");
    assert_eq!(dis["status"], "503");
    assert!(bad.count() >= 2);
    let (_, list) = call(&ts2, "GET", "/api/v1/me/hooks", &agent2, None);
    assert_eq!(list["hooks"][0]["state"], "disabled");
    let before = bad.count();
    ts2.store.act(&ts2.owner_id, &d2, "send-back", None).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(bad.count(), before, "a disabled hook delivers nothing");

    // 404: the receiver is gone, disabled at once.
    let gone = Receiver::start(Reply::Codes(vec![404]));
    let ts3 = TestServer::start_hooks(local(50, 60));
    let (_, agent3) = ts3.add_member("scribe");
    let d3 = doc(&ts3, "plan");
    add(&ts3, &agent3, json!({ "url": gone.url, "on": "action", "doc": d3 }));
    ts3.store.act(&ts3.owner_id, &d3, "approve", None).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while events_of(&ts3, "hook.disabled").is_empty() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(events_of(&ts3, "hook.disabled")[0]["reason"], "gone");
    assert_eq!(gone.count(), 1);

    // The Token that added it is revoked: disabled, nothing sent.
    let rx4 = Receiver::start(Reply::Codes(vec![200]));
    let ts4 = TestServer::start_hooks(local(50, 60));
    let (agent4_id, agent4) = ts4.add_member("scribe");
    let d4 = doc(&ts4, "plan");
    add(&ts4, &agent4, json!({ "url": rx4.url, "on": "action", "doc": d4 }));
    let tok = ts4.store.list_tokens(Some(&agent4_id)).unwrap().remove(0);
    ts4.store.revoke_token(&ts4.owner_id, &tok.info.id).unwrap();
    ts4.store.act(&ts4.owner_id, &d4, "approve", None).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while events_of(&ts4, "hook.disabled").is_empty() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(events_of(&ts4, "hook.disabled")[0]["reason"], "credential");
    assert_eq!(rx4.count(), 0);
}

// spec: S49-08
#[test]
fn an_expired_hook_is_removed_and_delivers_nothing() {
    let rx = Receiver::start(Reply::Codes(vec![200]));
    let ts = TestServer::start_hooks(local(50, 60));
    let (_, agent) = ts.add_member("scribe");
    let d = doc(&ts, "plan");
    let (_, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "action", "doc": d, "expires": 60 }));
    let exp = b["hook"]["expires_at"].as_str().unwrap().to_string();
    // The deliverer's own call, at a time after the expiry.
    let removed = ts.store.expire_hooks("9999-01-01T00:00:00.000Z").unwrap();
    assert_eq!(removed, vec![1]);
    assert!(exp.as_str() < "9999");
    assert_eq!(events_of(&ts, "hook.removed")[0]["reason"], "expired");
    ts.store.act(&ts.owner_id, &d, "approve", None).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(rx.count(), 0);
}

// spec: S49-10
#[test]
fn the_test_route_sends_one_signed_test_body() {
    let rx = Receiver::start(Reply::Codes(vec![200]));
    let ts = TestServer::start_hooks(local(50, 60));
    let (_, agent) = ts.add_member("scribe");
    let (_, b) = add(&ts, &agent, json!({ "url": rx.url, "on": "action" }));
    let secret = b["secret"].as_str().unwrap().to_string();
    let (st, out) = call(&ts, "POST", "/api/v1/me/hooks/h1/test", &agent, None);
    assert_eq!(st, 200, "{out}");
    assert_eq!(out, json!({ "id": "h1", "status": "200", "ok": true }));
    let got = rx.wait_for(1, 5);
    assert_eq!(got[0].json()["kind"], "test");
    assert_eq!(got[0].json()["by"], "@scribe");
    assert_eq!(got[0].header("x-annotary-signature"), Some(signature(&secret, &got[0].body).as_str()));
}

// spec: S49-09
#[test]
fn the_server_stops_within_drain_max_while_a_delivery_hangs() {
    let rx = Receiver::start(Reply::Hang);
    let mut ts = TestServer::start_hooks(local(50, 60));
    let (_, agent) = ts.add_member("scribe");
    let d = doc(&ts, "plan");
    add(&ts, &agent, json!({ "url": rx.url, "on": "action", "doc": d }));
    ts.store.act(&ts.owner_id, &d, "approve", None).unwrap();
    rx.wait_for(1, 5);
    let t = Instant::now();
    ts.shutdown();
    assert!(t.elapsed() < annotary::server::DRAIN_MAX, "took {:?}", t.elapsed());
}
