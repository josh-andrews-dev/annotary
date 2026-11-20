//! Questions in the CLI (`docs/dev/questions.md`): `answer DOC KEY OPTION|none`, and the pieces
//! `push`, `pull` and `show` share, all over `GET /api/v1/docs/{id}/questions`.

use clap::Args;
use serde_json::{Map, Value, json};

use annotary_renderer::questions::{self as q, AnswerBrief, Question, Recorded};

use crate::client::Api;
use crate::exit::{CliError, CliResult, Exit};
use crate::output::{FieldSpec, JsonMode};

use super::Ctx;
use super::docref::{DeletedMode, resolve};

/// The Questions of Document `id` on Version `version` (the head when `None`); `None` from a
/// server older than Questions (it has no such route).
pub fn fetch(api: &Api, id: &str, version: Option<u64>) -> CliResult<Option<Vec<Value>>> {
    let path = match version {
        Some(n) => format!("/api/v1/docs/{id}/questions?version={n}"),
        None => format!("/api/v1/docs/{id}/questions"),
    };
    match api.get(&path) {
        Ok(v) => Ok(Some(v["questions"].as_array().cloned().unwrap_or_default())),
        Err(e) if e.exit == Exit::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn answers_of(qj: &Value) -> Vec<AnswerBrief> {
    qj["answers"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| AnswerBrief {
                    handle: x["by"].as_str().unwrap_or("").to_string(),
                    option: x["option"].as_str().unwrap_or("").to_string(),
                    at: x["at"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_table(qj: &Value) -> bool {
    qj["source"].as_str() == Some("table")
}

/// What `pull` writes into the `Decided` cells: key → text, for each answered table Question
/// that is not decided.
pub fn fills(questions: &[Value]) -> Vec<(String, String)> {
    questions
        .iter()
        .filter(|x| is_table(x) && x["decided"].is_null())
        .filter_map(|x| {
            let text = q::decided_text(&answers_of(x), x["thread"].as_str())?;
            Some((x["key"].as_str()?.to_string(), text))
        })
        .collect()
}

/// The head's table Questions as the recording rule reads them (key, options, decided).
fn head_questions(questions: &[Value]) -> Vec<Question> {
    questions
        .iter()
        .filter(|x| is_table(x))
        .map(|x| Question {
            key: x["key"].as_str().unwrap_or("").to_string(),
            row: x["row"].as_u64().unwrap_or(1) as u32,
            cell: 0,
            options: x["options"]
                .as_array()
                .map(|o| o.iter().filter_map(|o| o["id"].as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            rec: x["rec"].as_str().map(str::to_string),
            decided: x["decided"].as_str().map(str::to_string),
            place: None,
            lines: None,
            places: Vec::new(),
            digest: String::new(),
        })
        .collect()
}

/// The rule `push` checks before uploading (decision D2): the head's Answers against the file.
pub fn recorded(head: &[Value], md: &str, local: &[Question]) -> Recorded {
    q::check_recorded(&head_questions(head), md, local, &fills(head))
}

/// The refusal `push` prints: exit 5 `answers_missing`, the rows to put in as detail lines (or
/// `missing[]` under `--json`).
pub fn missing_error(ctx: &Ctx, file: &std::path::Path, head: u64, r: &Recorded) -> CliError {
    let keys: Vec<&str> = r.missing.iter().map(|m| m.key.as_str()).collect();
    let mut e = CliError::new(
        Exit::Refused,
        "answers_missing",
        format!(
            "{} answered on v{head} {} not in the Decided cells of {}",
            keys.join(", "),
            if keys.len() == 1 { "is" } else { "are" },
            file.display()
        ),
    )
    .hint(format!(
        "put these rows in and push again (`annotary pull` writes them in; --discard-answers pushes without them)"
    ));
    if ctx.out.is_json() {
        let mut extra = Map::new();
        extra.insert(
            "missing".into(),
            json!(r.missing.iter().map(|m| json!({ "key": m.key, "row": m.row, "text": m.text })).collect::<Vec<_>>()),
        );
        e.extra = Some(Box::new(extra));
        return e;
    }
    for m in &r.missing {
        e = e.detail(format!("line {}: {}", m.row, m.text));
    }
    e
}

/// A Question's Answers in the `show` line: `B`, `B@rowan,A@sam`, `-` (none); `*` when one was
/// given on a Version whose Question read differently.
fn cell(qj: &Value) -> String {
    if !qj["decided"].is_null() {
        return "decided".to_string();
    }
    let answers = qj["answers"].as_array().cloned().unwrap_or_default();
    if answers.is_empty() {
        return "-".to_string();
    }
    let star = |a: &Value| if a["changed"].as_bool().unwrap_or(false) { "*" } else { "" };
    let opts: Vec<&str> = answers.iter().filter_map(|a| a["option"].as_str()).collect();
    if opts.iter().all(|o| *o == opts[0]) && answers.len() == 1 {
        return format!("{}{}", opts[0], star(&answers[0]));
    }
    if opts.iter().all(|o| *o == opts[0]) {
        let changed = answers.iter().any(|a| a["changed"].as_bool().unwrap_or(false));
        return format!("{}{}", opts[0], if changed { "*" } else { "" });
    }
    answers
        .iter()
        .map(|a| format!("{}{}@{}", a["option"].as_str().unwrap_or(""), star(a), a["by"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join(",")
}

/// `show`'s one line: `questions: D4=B D5=- D7=A* t9=other` (`None` when there are none).
pub fn show_line(questions: &[Value]) -> Option<String> {
    if questions.is_empty() {
        return None;
    }
    let parts: Vec<String> =
        questions.iter().map(|x| format!("{}={}", x["key"].as_str().unwrap_or(""), cell(x))).collect();
    Some(format!("questions: {}", parts.join(" ")))
}

// ---------------------------------------------------------------- `answer`

#[derive(Args, Debug)]
pub struct AnswerArgs {
    /// The Document: id | unique Name | path to a pulled .md
    pub doc: String,
    /// The Question: a key of the decisions table (D4) or a Thread that asks one (t7)
    pub key: String,
    /// An option id (A), a Thread Question's label, `other`, or `none` to withdraw your answer
    pub option: String,
    #[arg(long, value_name = "NAME")]
    pub vault: Option<String>,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &["key", "option", "changed", "version", "doc", "name"],
    default: &["key", "option", "changed", "version"],
};

pub fn run(ctx: &Ctx, a: AnswerArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    super::thread::reject_composite(&a.doc)?;
    let api = ctx.api()?;
    let doc = resolve(ctx, &api, &a.doc, a.vault.as_deref(), DeletedMode::Exclude)?;
    let id = doc["id"].as_str().unwrap_or("").to_string();
    let head = doc["version"].as_u64().unwrap_or(0);
    let option = if a.option.trim().eq_ignore_ascii_case("none") { Value::Null } else { json!(a.option.trim()) };
    let r = api.put_json(
        &format!("/api/v1/docs/{id}/questions/{}/answer", crate::client::qs(a.key.trim())),
        &json!({ "option": option, "version": head }),
    )?;
    let key = r["key"].as_str().unwrap_or("").to_string();
    let shown = r["option"].as_str().unwrap_or("none").to_string();
    let changed = r["changed"].as_bool().unwrap_or(false);
    if !changed {
        ctx.out.note(&format!("{key} was already {shown}"));
    }
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("key".into(), json!(key));
        rec.insert("option".into(), r["option"].clone());
        rec.insert("changed".into(), json!(changed));
        rec.insert("version".into(), r["version"].clone());
        rec.insert("doc".into(), json!(id));
        rec.insert("name".into(), r["name"].clone());
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&format!("{key}{sep}{shown}"));
    Ok(())
}
