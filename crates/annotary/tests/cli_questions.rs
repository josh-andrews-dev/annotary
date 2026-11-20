//! Questions through the real binary against the in-process server: a decisions table pushed,
//! answered, read with `show`, pulled with the Answers filled in, the push refusal and its
//! escape, `--strict`, `thread new --ask`, and `answer`'s exits.

use std::path::Path;
use std::process::{Command, Output};

use annotary::testing::TestServer;
use serde_json::Value;

fn run(ts: &TestServer, home: &Path, cwd: &Path, token: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_TOKEN", token)
        .env("ANNOTARY_SERVER", &ts.url)
        .env_remove("ANNOTARY_PROFILE")
        .env("ANNOTARY_TTY", "0")
        .output()
        .unwrap()
}

fn text(out: &Output) -> (String, String) {
    (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

fn ok(out: Output) -> (String, String) {
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    (so, se)
}

fn code(out: Output, want: i32) -> (String, String) {
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(want), "stdout: {so}\nstderr: {se}");
    (so, se)
}

pub const PLAN: &str = "# Plan\n\
\n\
| Decision | Options | Recommended | Decided |\n\
|---|---|---|---|\n\
| D1 | A, B | A | |\n\
| D2 | A, B, C | B | |\n\
\n\
## D1. Where the service runs\n\
\n\
- Recommended: A. The existing box.\n\
- B. A new VM.\n\
\n\
## D2. Which region\n\
\n\
- A. us-east\n\
- B. eu-west\n\
- C. us-west\n";

struct Env {
    ts: TestServer,
    home: tempfile::TempDir,
    a: tempfile::TempDir,
    agent: String,
}

impl Env {
    fn new() -> Env {
        let ts = TestServer::start();
        let (_, agent) = ts.add_member("scribe");
        Env { ts, home: tempfile::tempdir().unwrap(), a: tempfile::tempdir().unwrap(), agent }
    }
    fn owner(&self, args: &[&str]) -> Output {
        run(&self.ts, self.home.path(), self.a.path(), &self.ts.owner_token, args)
    }
    fn scribe(&self, args: &[&str]) -> Output {
        run(&self.ts, self.home.path(), self.a.path(), &self.agent, args)
    }
    fn write(&self, md: &str) {
        std::fs::write(self.a.path().join("plan.md"), md).unwrap();
    }
    fn read(&self) -> String {
        std::fs::read_to_string(self.a.path().join("plan.md")).unwrap()
    }
}

// spec: S50-03, S50-05, S50-06, S50-10
#[test]
fn a_decisions_table_round_trips_through_answer_show_pull_and_push() {
    let e = Env::new();
    e.write(PLAN);
    let (so, _) = ok(e.owner(&["push", "plan.md"]));
    assert!(so.starts_with("plan\t"), "{so}");
    // The Version carries the Questions the CLI computed.
    let (so, _) = ok(e.owner(&["show", "plan", "--json", "questions"]));
    let v: Value = serde_json::from_str(&so).unwrap();
    let keys: Vec<&str> = v["questions"].as_array().unwrap().iter().map(|q| q["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["D1", "D2"]);
    assert_eq!(v["questions"][0]["options"][0]["place"]["block"], "li");
    let (so, _) = ok(e.owner(&["show", "plan"]));
    assert!(so.lines().nth(1) == Some("questions: D1=- D2=-"), "{so}");

    // Answers: two Members on D1, one on D2; `none` withdraws; the same again is a no-op.
    assert_eq!(ok(e.owner(&["answer", "plan", "D1", "A"])).0, "D1\tA\n");
    let (so, se) = ok(e.owner(&["answer", "plan", "D1", "A"]));
    assert_eq!(so, "D1\tA\n");
    assert!(se.contains("D1: it was already A"), "{se}");
    ok(e.scribe(&["answer", "plan", "D1", "B"]));
    ok(e.scribe(&["answer", "plan", "D2", "C"]));
    ok(e.scribe(&["answer", "plan", "D2", "none"]));
    ok(e.scribe(&["answer", "plan", "D2", "C"]));
    let (so, _) = ok(e.owner(&["show", "plan"]));
    assert!(so.contains("questions: D1=A@rowan,B@scribe D2=C\n"), "{so}");
    // `--json` omits false, as every verb does.
    let (so, _) = ok(e.scribe(&["answer", "plan", "D2", "C", "--json"]));
    let j: Value = serde_json::from_str(&so).unwrap();
    assert_eq!((j["key"].as_str(), j["option"].as_str(), j.get("changed")), (Some("D2"), Some("C"), None));

    // Exits: unknown key 2, an option that is not one 1, a reader without `comment` 4.
    code(e.owner(&["answer", "plan", "D9", "A"]), 2);
    code(e.owner(&["answer", "plan", "D1", "Z"]), 1);
    let (mid, reader) = e.ts.add_member("reader");
    let read = annotary_store::Grant { vault: "notes".into(), rights: vec![annotary_store::Right::Read] };
    e.ts.store.set_grants(&e.ts.owner_id, &mid, vec![read]).unwrap();
    let (_, se) = code(run(&e.ts, e.home.path(), e.a.path(), &reader, &["answer", "plan", "D1", "A"]), 4);
    assert!(se.starts_with("error: "), "{se}");

    // An unchanged file cannot be pushed over the Answers: exit 5 with the rows to put in.
    let edited = PLAN.replace("- B. A new VM.", "- B. A new VM, bigger.");
    e.write(&edited);
    let (_, se) = code(e.owner(&["push", "plan.md"]), 5);
    assert!(se.contains("error: D1, D2 answered on v1 are not in the Decided cells of plan.md"), "{se}");
    assert!(se.contains("line 5: | D1 | A, B | A | A (@rowan), B (@scribe) |"), "{se}");
    assert!(se.contains("line 6: | D2 | A, B, C | B | C (@scribe, "), "{se}");
    let (_, se) = code(e.owner(&["push", "plan.md", "--json"]), 5);
    let err: Value = serde_json::from_str(se.trim()).unwrap();
    assert_eq!(err["error"], "answers_missing");
    assert_eq!(err["missing"][0]["key"], "D1");

    // `pull` writes them in (only those cells) and says so; a second pull is no local edit.
    e.write(PLAN);
    let (_, se) = ok(e.owner(&["pull", "plan"]));
    assert!(se.contains("2 answers written into the decisions table; push to record them"), "{se}");
    let filled = e.read();
    assert!(filled.contains("| D1 | A, B | A | A (@rowan), B (@scribe) |\n"), "{filled}");
    assert_eq!(filled.lines().count(), PLAN.lines().count());
    ok(e.owner(&["pull", "plan"]));
    // `--as-pushed` gives the exact bytes.
    let (_, se) = ok(e.owner(&["pull", "plan", "--as-pushed", "--force"]));
    assert!(!se.contains("answers written"), "{se}");
    assert_eq!(e.read(), PLAN);
    // The filled file pushes; its Questions are decided now.
    ok(e.owner(&["pull", "plan", "--force"]));
    let (so, _) = ok(e.owner(&["push", "plan.md", "-m", "record"]));
    assert!(so.ends_with("\tv2\n"), "{so}");
    let (so, _) = ok(e.owner(&["show", "plan"]));
    assert!(so.contains("questions: D1=decided D2=decided"), "{so}");
    let (_, se) = code(e.owner(&["answer", "plan", "D1", "B"]), 5);
    assert!(se.contains("decided"), "{se}");
    // An older Version pulls exact.
    let (_, se) = ok(e.owner(&["pull", "plan", "--version", "1", "--force"]));
    assert!(!se.contains("answers written"), "{se}");
    assert_eq!(e.read(), PLAN);
}

// spec: S50-06
#[test]
fn removed_rows_pass_with_a_note_and_discard_answers_is_logged() {
    let e = Env::new();
    e.write(PLAN);
    ok(e.owner(&["push", "plan.md"]));
    ok(e.owner(&["answer", "plan", "D1", "B"]));
    ok(e.owner(&["answer", "plan", "D2", "A"]));
    // D2's row removed: a note; D1 kept empty: refused until discarded.
    let md = PLAN.replace("| D2 | A, B, C | B | |\n", "");
    e.write(&md);
    let (_, se) = code(e.owner(&["push", "plan.md"]), 5);
    assert!(se.contains("note: D2's row was removed; its answers (A (@rowan, "), "{se}");
    assert!(se.contains("error: D1 answered on v1 is not in the Decided cells"), "{se}");
    let (so, se) = ok(e.owner(&["push", "plan.md", "--discard-answers"]));
    assert!(se.contains("warning: discarding the answers to D1 given on v1"), "{se}");
    assert!(so.ends_with("\tv2\n"), "{so}");
    let ev = e.ts.store.events_since(0).unwrap();
    let pushed = ev.iter().rfind(|x| x.kind == "version.pushed").unwrap();
    assert_eq!(pushed.payload["discarded_answers"], serde_json::json!(["D1"]));
    // Answers follow the key onto v2.
    let (so, _) = ok(e.owner(&["show", "plan"]));
    assert!(so.contains("questions: D1=B\n"), "{so}");
}

// spec: S50-03
#[test]
fn table_warnings_print_and_strict_refuses() {
    let e = Env::new();
    e.write("# P\n\n| Decision | Options | Decided |\n|---|---|---|\n| Q1 | A | |\n| Q2 | A, B | |\n\nQ2 here.\n");
    let (_, se) = code(e.owner(&["push", "plan.md", "--strict"]), 5);
    assert!(se.contains("1 decisions table warning (--strict)"), "{se}");
    assert!(se.contains("Q1 has 1 options"), "{se}");
    let (_, se) = ok(e.owner(&["push", "plan.md"]));
    assert!(se.contains("warning: decisions table line 5: Q1 has 1 options"), "{se}");
    let (so, _) = ok(e.owner(&["show", "plan"]));
    assert!(so.contains("questions: Q2=-\n"), "{so}");
}

// spec: S50-08, S50-10
#[test]
fn a_question_asked_mid_review() {
    let e = Env::new();
    e.write("# Plan\n\nWe ship on Friday.\n");
    ok(e.owner(&["push", "plan.md"]));
    let (so, _) = ok(e.owner(&["show", "plan"]));
    assert!(!so.contains("questions:"), "{so}");
    code(e.scribe(&["thread", "new", "plan", "--document", "-m", "Ship?", "--ask", "Yes"]), 1);
    let (so, _) = ok(e.scribe(&[
        "thread",
        "new",
        "plan",
        "--document",
        "-m",
        "Ship on Friday?",
        "--ask",
        "Yes",
        "--ask",
        "Wait a week",
    ]));
    let tid = so.split('\t').next().unwrap().to_string();
    assert_eq!(tid, "t1");
    // A label or an id answers it.
    assert_eq!(ok(e.owner(&["answer", "plan", "t1", "wait a week"])).0, "t1\tB\n");
    let (so, _) = ok(e.owner(&["thread", "show", "plan", "t1"]));
    assert!(so.contains("  asks: A \"Yes\" B \"Wait a week\" · B@rowan"), "{so}");
    let (so, _) = ok(e.owner(&["show", "plan"]));
    assert!(so.contains("questions: t1=B\n"), "{so}");
    let (so, _) = ok(e.owner(&["thread", "show", "plan", "t1", "--json"]));
    let t: Value = serde_json::from_str::<Value>(&so).unwrap()[0].clone();
    assert_eq!(t["options"][1]["label"], "Wait a week");
    assert_eq!(t["answers"][0]["option"], "B");
    // The Markdown never holds it: pull writes the exact bytes.
    let (_, se) = ok(e.owner(&["pull", "plan", "--force"]));
    assert!(!se.contains("answers written"), "{se}");
    assert_eq!(e.read(), "# Plan\n\nWe ship on Friday.\n");
    code(e.owner(&["answer", "plan", "t1", "Maybe"]), 1);
}
