//! The size report: deterministic counts (bytes, elements, requests, database statements, parts),
//! never times, of what the server sends for the generated perf fixture (`crates/perfgen`, full
//! scale, seed 1), seeded through the real binary against an in-process server.
//!
//! The committed `perf/size-report.json` is the golden of these numbers: the test fails when it
//! differs from what was measured, and when a number is above its ceiling in `perf/BUDGETS.md`
//! ("Size ceilings"). `make size-report` rewrites the file; `make size-check` runs the test. It is
//! `#[ignore]`d so `make test` stays under a minute (seeding the full fixture in a debug build
//! takes seconds). What each number counts: `docs/dev/perf-suite.md`, "The size report".
//!
//! Two things in an answer depend on the run, not the code, and are normalized before counting:
//! the server's address (`http://127.0.0.1:PORT`, also percent-encoded) and the text of relative
//! times (`<time class="cd-rel">now</time>` becomes `1m` once a minute has passed).

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use annotary::testing::TestServer;
use annotary_perfgen::{Handles, Plan, Scale, Target, seed};
use annotary_store::stmt_count;
use serde_json::{Value, json};

const REPORT: &str = "perf/size-report.json";
const BUDGETS: &str = "perf/BUDGETS.md";
/// What a normalized answer says in place of this run's server address.
const SERVER: &str = "http://127.0.0.1:7377";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

// ---------------------------------------------------------------- HTTP

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    fn text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("a UTF-8 answer")
    }
}

struct Client {
    url: String,
    agent: ureq::Agent,
}

impl Client {
    fn new(url: &str) -> Client {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(120)))
            .build()
            .new_agent();
        Client { url: url.to_string(), agent }
    }

    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Answer {
        let uri = format!("{}{path}", self.url);
        let mut resp = match method {
            "GET" => {
                let mut r = self.agent.get(&uri);
                for (k, v) in headers {
                    r = r.header(*k, *v);
                }
                r.call()
            }
            "POST" => {
                let mut r = self.agent.post(&uri);
                for (k, v) in headers {
                    r = r.header(*k, *v);
                }
                r.send_json(body.expect("a POST body"))
            }
            _ => unreachable!(),
        }
        .unwrap_or_else(|e| panic!("{method} {path}: {e}"));
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect();
        let body = resp.body_mut().with_config().limit(256 * 1024 * 1024).read_to_vec().unwrap();
        Answer { status, headers, body }
    }

    fn get(&self, path: &str, headers: &[(&str, &str)]) -> Answer {
        self.send("GET", path, headers, None)
    }
}

/// One answer, measured warm: a first request fills the server's caches (rendered titles, …),
/// then the same request is repeated until two in a row agree on bytes and statements, so a
/// stray request from outside the test (a port probe on a developer's machine) cannot land in
/// the count.
fn measured(what: &str, mut request: impl FnMut() -> Answer) -> (Answer, u64) {
    let warm = request();
    assert!(warm.status < 300, "{what}: HTTP {}\n{}", warm.status, String::from_utf8_lossy(&warm.body));
    let mut last: Option<(Answer, u64)> = None;
    for _ in 0..6 {
        let before = stmt_count::statements();
        let a = request();
        let n = stmt_count::statements() - before;
        if let Some((prev, m)) = &last
            && *m == n
            && prev.body.len() == a.body.len()
        {
            return (a, n);
        }
        last = Some((a, n));
    }
    panic!("{what}: no two requests in a row ran the same number of statements");
}

// ---------------------------------------------------------------- normalizing

fn normalize(text: &str, server: &str) -> String {
    let encoded = server.replace(':', "%3A").replace('/', "%2F");
    let text = text.replace(server, SERVER).replace(&encoded, &SERVER.replace(':', "%3A").replace('/', "%2F"));
    // `\"` inside a JSON string (the card fragment) as well as `"` in a page.
    let rel = regex::Regex::new(r#"(<time class=\\?"cd-rel\\?"[^>]*>)[^<]*(</time>)"#).unwrap();
    rel.replace_all(&text, "${1}now${2}").into_owned()
}

// ---------------------------------------------------------------- reading a page

/// What one page is made of. Every byte of the page is in exactly one of `script`, `data`,
/// `style`, `document`, `cards` and `other`.
#[derive(Debug, Default)]
struct Page {
    bytes: u64,
    /// Executable inline `<script>` elements, tags included.
    script: u64,
    /// `<script type="application/json">` (the page's `#cd-data`), tags included.
    data: u64,
    /// Inline `<style>` elements, tags included.
    style: u64,
    /// The rendered Document: `article#cd-doc` without the Thread strips spliced into it.
    document: u64,
    /// Every `.cd-strip` (the Thread cards, the Document-level strip, the orphaned group).
    cards: u64,
    other: u64,
    /// Start tags, `<svg>` children included (`<!doctype>` and comments are not elements).
    elements: u64,
    /// The page itself plus what must load before anything paints: stylesheets, classic scripts
    /// with a `src` and no `async` / `defer`, CSS `@import`.
    requests_before_paint: u64,
}

const VOID: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        let before = lower[..at].chars().last().unwrap_or(' ');
        let rest = &tag[at + name.len()..];
        if before.is_ascii_whitespace() && rest.starts_with('=') {
            let rest = &rest[1..];
            return Some(match rest.chars().next() {
                Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or(""),
                _ => rest.split(|c: char| c.is_ascii_whitespace() || c == '>').next().unwrap_or(""),
            });
        }
        if before.is_ascii_whitespace()
            && (rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/'))
        {
            return Some("");
        }
        from = at + name.len();
    }
    None
}

fn has_class(tag: &str, class: &str) -> bool {
    attr(tag, "class").is_some_and(|c| c.split_ascii_whitespace().any(|t| t == class))
}

/// The end of a tag that starts at `i` (`<`), quotes respected: the index after its `>`.
fn tag_end(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    let mut quote = 0u8;
    while j < b.len() {
        let c = b[j];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return j + 1;
        }
        j += 1;
    }
    b.len()
}

fn find_ci(hay: &str, from: usize, needle: &str) -> Option<usize> {
    hay[from..].to_ascii_lowercase().find(needle).map(|i| from + i)
}

fn read_page(html: &str) -> Page {
    #[derive(PartialEq)]
    enum Kind {
        Plain,
        Doc,
        Card,
    }
    let b = html.as_bytes();
    let mut p = Page { bytes: b.len() as u64, requests_before_paint: 1, ..Page::default() };
    let mut stack: Vec<(String, usize, Kind)> = Vec::new();
    let mut doc = 0u64;
    let mut in_card = 0usize;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if html[i..].starts_with("<!--") {
            i = html[i + 4..].find("-->").map(|j| i + 4 + j + 3).unwrap_or(b.len());
            continue;
        }
        if html[i..].starts_with("<!") {
            i = tag_end(b, i);
            continue;
        }
        let closing = b.get(i + 1) == Some(&b'/');
        let name_from = if closing { i + 2 } else { i + 1 };
        let name_len = html[name_from..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(0);
        if name_len == 0 {
            i += 1; // a bare `<` in text
            continue;
        }
        let name = html[name_from..name_from + name_len].to_ascii_lowercase();
        let end = tag_end(b, i);
        let tag = &html[i..end];
        if closing {
            if let Some(at) = stack.iter().rposition(|(n, _, _)| *n == name) {
                while stack.len() > at {
                    let (_, start, kind) = stack.pop().unwrap();
                    let len = (end - start) as u64;
                    match kind {
                        Kind::Doc => doc += len,
                        Kind::Card => {
                            in_card -= 1;
                            if in_card == 0 {
                                p.cards += len;
                            }
                        }
                        Kind::Plain => {}
                    }
                }
            }
            i = end;
            continue;
        }
        p.elements += 1;
        if name == "script" || name == "style" {
            let close = find_ci(html, end, &format!("</{name}")).unwrap_or(b.len());
            let after = if close < b.len() { tag_end(b, close) } else { b.len() };
            let len = (after - i) as u64;
            if name == "style" {
                p.style += len;
                p.requests_before_paint += html[end..close].matches("@import").count() as u64;
            } else if attr(tag, "type").is_some_and(|t| t.eq_ignore_ascii_case("application/json")) {
                p.data += len;
            } else {
                p.script += len;
                let module = attr(tag, "type").is_some_and(|t| t.eq_ignore_ascii_case("module"));
                if attr(tag, "src").is_some() && !module && attr(tag, "async").is_none() && attr(tag, "defer").is_none()
                {
                    p.requests_before_paint += 1;
                }
            }
            i = after;
            continue;
        }
        if name == "link"
            && attr(tag, "rel")
                .is_some_and(|r| r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case("stylesheet")))
        {
            p.requests_before_paint += 1;
        }
        let self_closing = tag.ends_with("/>");
        if !VOID.contains(&name.as_str()) && !self_closing {
            let kind = if attr(tag, "id") == Some("cd-doc") {
                Kind::Doc
            } else if has_class(tag, "cd-strip") {
                in_card += 1;
                Kind::Card
            } else {
                Kind::Plain
            };
            stack.push((name, i, kind));
        }
        i = end;
    }
    assert!(stack.iter().all(|(_, _, k)| *k == Kind::Plain), "a Document or a strip is never closed");
    p.document = doc.saturating_sub(p.cards);
    p.other = p.bytes - p.script - p.data - p.style - p.document - p.cards;
    p
}

// ---------------------------------------------------------------- the report

struct Metric {
    id: String,
    label: String,
    unit: &'static str,
    value: u64,
}

#[derive(Default)]
struct Report(Vec<Metric>);

impl Report {
    fn add(&mut self, id: &str, label: &str, unit: &'static str, value: u64) {
        assert!(!self.0.iter().any(|m| m.id == id), "{id} twice");
        self.0.push(Metric { id: id.into(), label: label.into(), unit, value });
    }

    fn page(&mut self, prefix: &str, name: &str, page: &Page, statements: u64, with_document: bool) {
        self.add(&format!("{prefix}.bytes"), &format!("{name}: bytes"), "bytes", page.bytes);
        self.add(&format!("{prefix}.script_bytes"), &format!("{name}: inline script"), "bytes", page.script);
        self.add(&format!("{prefix}.data_bytes"), &format!("{name}: page data (JSON)"), "bytes", page.data);
        self.add(&format!("{prefix}.style_bytes"), &format!("{name}: inline style"), "bytes", page.style);
        if with_document {
            self.add(&format!("{prefix}.document_bytes"), &format!("{name}: Document HTML"), "bytes", page.document);
            self.add(&format!("{prefix}.cards_bytes"), &format!("{name}: Thread cards"), "bytes", page.cards);
        } else {
            assert_eq!(page.document + page.cards, 0, "{name} has no Document");
        }
        self.add(&format!("{prefix}.other_bytes"), &format!("{name}: the rest"), "bytes", page.other);
        self.add(&format!("{prefix}.elements"), &format!("{name}: elements"), "count", page.elements);
        self.add(
            &format!("{prefix}.requests_before_paint"),
            &format!("{name}: requests before first paint"),
            "count",
            page.requests_before_paint,
        );
        self.add(&format!("{prefix}.db_statements"), &format!("{name}: database statements"), "count", statements);
    }
}

/// `| id | metric | ceiling | unit |` under "## Size ceilings" in `perf/BUDGETS.md`.
fn ceilings(md: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in md.lines() {
        let l = line.trim();
        if let Some(h) = l.strip_prefix("## ") {
            inside = h.starts_with("Size ceilings");
            continue;
        }
        if !inside || !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        let cells = &cells[1..cells.len() - 1];
        if cells.len() < 4 || cells[0] == "id" || cells[0].chars().all(|c| c == '-') {
            continue;
        }
        let id = cells[0].trim_matches('`');
        let value: u64 = cells[2]
            .replace(',', "")
            .parse()
            .unwrap_or_else(|_| panic!("{BUDGETS}: the ceiling of {id} is not a whole number: {:?}", cells[2]));
        out.push((id.to_string(), value));
    }
    out
}

/// The SSE frame of the first `event` after `cursor` on the Document's live feed, as the page
/// receives it (`id:`, `event:`, `data:` lines and the blank line that ends it).
fn sse_frame(url: &str, cookie: &str, doc: &str, cursor: u64, event: &str) -> String {
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().new_agent();
    let mut resp = agent
        .get(format!("{url}/api/v1/events?doc={doc}"))
        .header("Accept", "text/event-stream")
        .header("Cookie", cookie)
        .header("Last-Event-ID", cursor.to_string())
        .call()
        .expect("the live feed answers");
    let mut lines = BufReader::new(resp.body_mut().as_reader());
    let mut frame = String::new();
    loop {
        let mut line = String::new();
        if lines.read_line(&mut line).unwrap() == 0 {
            panic!("the live feed ended before a {event} event");
        }
        frame.push_str(&line);
        if line == "\n" {
            if frame.lines().any(|l| l == format!("event: {event}")) {
                return frame;
            }
            frame.clear();
        }
    }
}

fn measure() -> (Value, Vec<Metric>) {
    let ts = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let scale = Scale::FULL;
    let plan = Plan::generate(&scale, 1, &Handles::default());
    let target = Target {
        bin: PathBuf::from(env!("CARGO_BIN_EXE_annotary")),
        server: ts.url.clone(),
        owner_token: ts.owner_token.clone(),
        home: tmp.path().join("home"),
        work: tmp.path().join("work"),
    };
    let mut log = Vec::new();
    let seeded = seed(&plan, &target, &mut |l: &str| log.push(l.to_string()))
        .unwrap_or_else(|e| panic!("seed: {e}\n{}", log.join("\n")));
    let id = seeded.doc_id.clone();
    let head = seeded.head;
    let c = Client::new(&ts.url);
    let bearer = format!("Bearer {}", ts.owner_token);
    let norm = |a: &Answer| normalize(&a.text(), &ts.url);

    // A browser: the first page on loopback (no password) signs it in as the owner.
    let first = c.get("/", &[("Accept", "text/html")]);
    let cookie = first
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .find_map(|(_, v)| v.split(';').next().filter(|kv| kv.starts_with("cd_session=")))
        .unwrap_or_else(|| panic!("no session cookie from GET / ({})", first.status))
        .to_string();
    let browser = [("Accept", "text/html"), ("Cookie", cookie.as_str())];

    let mut r = Report::default();
    for (prefix, name, path, with_document) in [
        ("view", "Document view", format!("/d/{id}"), true),
        ("version", "Version page", format!("/d/{id}/v/{}", head - 1), true),
        ("list", "Document list", "/".to_string(), false),
    ] {
        let (a, n) = measured(name, || c.get(&path, &browser));
        r.page(prefix, name, &read_page(&norm(&a)), n, with_document);
    }

    // The API as the iPhone and iPad app reads it.
    let api = [("Authorization", bearer.as_str())];
    for (prefix, name, path) in [
        ("api.docs", "API Document list", "/api/v1/docs".to_string()),
        ("api.html", "API Version HTML", format!("/api/v1/docs/{id}/versions/{head}/html")),
        ("api.threads", "API Threads", format!("/api/v1/docs/{id}/threads?status=all")),
    ] {
        let (a, n) = measured(name, || c.get(&path, &api));
        r.add(&format!("{prefix}.bytes"), &format!("{name}: bytes"), "bytes", norm(&a).len() as u64);
        r.add(&format!("{prefix}.db_statements"), &format!("{name}: database statements"), "count", n);
    }

    // Embed mode, as the Hermes page's backend calls it.
    let embed = [("Authorization", bearer.as_str()), ("X-Annotary-Embed", "1"), ("Accept", "text/html")];
    let shell = c.get("/embed/shell", &embed);
    assert_eq!(shell.status, 200, "{}", String::from_utf8_lossy(&shell.body));
    r.add("embed.shell.bytes", "Embed shell: bytes", "bytes", norm(&shell).len() as u64);
    let first = c.get(&format!("/d/{id}"), &embed);
    let parts: u64 = first.header("x-annotary-parts").map_or(1, |p| p.parse().expect("a part count"));
    let mut whole = first.body.clone();
    if parts > 1 {
        assert_eq!(first.status, 206);
        let etag = first.header("x-annotary-etag").expect("parts carry an etag").to_string();
        for n in 1..parts {
            let part = n.to_string();
            let mut h = embed.to_vec();
            h.push(("X-Annotary-Part", &part));
            h.push(("X-Annotary-Etag", &etag));
            let a = c.get(&format!("/d/{id}"), &h);
            assert_eq!(a.status, 206, "part {n}");
            whole.extend_from_slice(&a.body);
        }
    }
    let whole = Answer { status: 200, headers: Vec::new(), body: whole };
    let whole = norm(&whole);
    r.add("embed.view.bytes", "Embed Document view: bytes", "bytes", whole.len() as u64);
    // The server cut the raw body; the same cuts of the normalized body are what this run's address
    // and clock cannot move.
    let normalized_parts = annotary::server::embed::cuts(whole.as_bytes(), annotary::server::embed::PART_MAX).len();
    assert!(
        normalized_parts.abs_diff(parts as usize) <= 1,
        "{parts} parts from the server, {normalized_parts} normalized"
    );
    r.add("embed.view.parts", "Embed Document view: parts", "count", normalized_parts.max(1) as u64);

    // One reply: the card fragment the page swaps in, and the live event that announces it.
    let threads = c.get(&format!("/api/v1/docs/{id}/threads"), &api);
    let threads: Value = serde_json::from_slice(&threads.body).unwrap();
    let tid = threads["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["anchor"]["status"] == "anchored")
        .or_else(|| threads["threads"].as_array().unwrap().first())
        .and_then(|t| t["id"].as_str())
        .expect("an open Thread")
        .to_string();
    let cursor = serde_json::from_slice::<Value>(&c.get("/api/v1/events", &api).body).unwrap()["seq"]
        .as_u64()
        .expect("the events cursor");
    let posted = c.send(
        "POST",
        &format!("/api/v1/docs/{id}/threads/{tid}/replies"),
        &api,
        Some(&json!({"body": "Measured by the size report: one reply."})),
    );
    assert!(posted.status < 300, "reply: {}", posted.text());
    let fragment = c.get(&format!("/d/{id}/t/{tid}"), &[("Cookie", cookie.as_str()), ("Accept", "application/json")]);
    assert_eq!(fragment.status, 200, "{}", fragment.text());
    r.add("reply.fragment.bytes", "Reply: card fragment bytes", "bytes", norm(&fragment).len() as u64);
    let frame = sse_frame(&ts.url, &cookie, &id, cursor, "thread.replied");
    r.add("reply.event.bytes", "Reply: live event bytes", "bytes", normalize(&frame, &ts.url).len() as u64);

    let fixture = json!({
        "scale": "full",
        "seed": 1,
        "lines": plan.stats.lines_v1,
        "versions": seeded.head,
        "threads": seeded.threads,
        "replies": seeded.replies,
        "list_documents": seeded.list_ids.len(),
    });
    drop(ts);
    (fixture, r.0)
}

fn render(fixture: &Value, metrics: &[Metric], ceilings: &[(String, u64)]) -> String {
    let metrics: Vec<Value> = metrics
        .iter()
        .map(|m| {
            let ceiling = ceilings.iter().find(|(id, _)| *id == m.id).map(|(_, c)| *c);
            json!({"id": m.id, "label": m.label, "unit": m.unit, "value": m.value, "ceiling": ceiling})
        })
        .collect();
    let mut s = serde_json::to_string_pretty(&json!({"schema": 1, "fixture": fixture, "metrics": metrics})).unwrap();
    s.push('\n');
    s
}

// spec: S45-01, S45-02
#[test]
#[ignore = "seeds the full perf fixture: `make size-check` (CI runs it on Linux and macOS)"]
fn the_size_report_matches_what_the_server_sends_and_stays_under_its_ceilings() {
    let (fixture, metrics) = measure();
    let root = repo_root();
    let md = std::fs::read_to_string(root.join(BUDGETS)).unwrap();
    let ceilings = ceilings(&md);
    assert!(!ceilings.is_empty(), "{BUDGETS} has no \"## Size ceilings\" table");
    for (id, _) in &ceilings {
        assert!(metrics.iter().any(|m| m.id == *id), "{BUDGETS}: the size ceiling {id} names no measured number");
    }
    let want = render(&fixture, &metrics, &ceilings);
    let path = root.join(REPORT);
    if std::env::var("SIZE_REPORT").as_deref() == Ok("overwrite") {
        std::fs::write(&path, &want).unwrap();
    }
    let over: Vec<String> = metrics
        .iter()
        .filter_map(|m| {
            let (_, c) = ceilings.iter().find(|(id, _)| *id == m.id)?;
            (m.value > *c).then(|| format!("{} ({}): {} > ceiling {c}", m.label, m.id, m.value))
        })
        .collect();
    assert!(over.is_empty(), "above the ceilings in {BUDGETS}:\n{}", over.join("\n"));
    let mut have = String::new();
    std::fs::File::open(&path).and_then(|mut f| f.read_to_string(&mut have)).ok();
    assert!(
        have == want,
        "{REPORT} is stale: regenerate it with `make size-report` and review the diff\n--- measured ---\n{want}"
    );
}

#[test]
fn a_page_is_split_into_parts_that_add_up() {
    let html = concat!(
        "<!doctype html><html><head><style>a{}</style><link rel=\"stylesheet\" href=\"/x.css\">",
        "<script src=\"/a.js\"></script><script type=\"module\">if (a<b) {}</script></head><body>",
        "<!-- <p>not an element</p> --><article class=\"cd-doc\" id=\"cd-doc\"><p data-l=\"1\">One<br>two</p>",
        "<div class=\"cd-strip\"><article class=\"cd-thread\"><div class=\"cd-strip\">x</div></article></div>",
        "<svg><circle r=\"1\"/></svg></article>",
        "<script type=\"application/json\" id=\"cd-data\">{\"a\":\"</p>\"}</script></body></html>",
    );
    let p = read_page(html);
    assert_eq!(p.bytes, html.len() as u64);
    assert_eq!(p.style, "<style>a{}</style>".len() as u64);
    assert_eq!(
        p.script,
        ("<script src=\"/a.js\"></script>".len() + "<script type=\"module\">if (a<b) {}</script>".len()) as u64
    );
    assert_eq!(p.data, "<script type=\"application/json\" id=\"cd-data\">{\"a\":\"</p>\"}</script>".len() as u64);
    let card = "<div class=\"cd-strip\"><article class=\"cd-thread\"><div class=\"cd-strip\">x</div></article></div>";
    assert_eq!(p.cards, card.len() as u64);
    let doc = "<article class=\"cd-doc\" id=\"cd-doc\"><p data-l=\"1\">One<br>two</p>".len()
        + "<svg><circle r=\"1\"/></svg></article>".len();
    assert_eq!(p.document, doc as u64);
    assert_eq!(p.script + p.data + p.style + p.document + p.cards + p.other, p.bytes);
    // html head style link script script body article p br div article div svg circle script
    assert_eq!(p.elements, 16);
    // the page, the stylesheet, the classic script (the module is deferred)
    assert_eq!(p.requests_before_paint, 3);
}

#[test]
fn normalizing_replaces_the_address_and_relative_times() {
    let server = "http://127.0.0.1:45123";
    let text = concat!(
        "<a href=\"annotary://open?server=http%3A%2F%2F127.0.0.1%3A45123\">",
        "<time class=\"cd-rel\" datetime=\"2026-01-01T00:00:00.000Z\" title=\"x\">12m</time>",
        "{\"html\":\"<time class=\\\"cd-rel\\\" datetime=\\\"x\\\">3h</time>\"} http://127.0.0.1:45123/d/x"
    );
    let n = normalize(text, server);
    assert!(n.contains("server=http%3A%2F%2F127.0.0.1%3A7377\""), "{n}");
    assert!(n.contains("title=\"x\">now</time>"), "{n}");
    assert!(n.contains("\\\">now</time>"), "{n}");
    assert!(n.ends_with(" http://127.0.0.1:7377/d/x"), "{n}");
}

#[test]
fn ceilings_come_from_their_own_table() {
    let md = "## The budgets\n\n| id | metric | budget | unit |\n|---|---|---|---|\n| fcp | FCP | 100 | ms |\n\n\
              ## Size ceilings\n\n| id | metric | ceiling | unit |\n|---|---|---:|---|\n\
              | `view.bytes` | Document view: bytes | 2,200,000 | bytes |\n| view.elements | x | 70000 | count |\n\n## Next\n| a | b | 1 | c |\n";
    assert_eq!(ceilings(md), vec![("view.bytes".to_string(), 2_200_000), ("view.elements".to_string(), 70_000)]);
}
