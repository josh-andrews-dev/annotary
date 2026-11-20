//! Questions and Answers (`docs/dev/questions.md`): a Version's decisions table and the Questions
//! asked on Threads, with each Member's current Answer.
//!
//! | route | right | what |
//! |---|---|---|
//! | `GET /api/v1/docs/{id}/questions?version=N` | read | `{id, name, version, head, questions: [...]}`: every Question of Version N (default the head) and the Threads' Questions, each with its Answers as they stood on N |
//! | `PUT /api/v1/docs/{id}/questions/{key}/answer {option: ID\|null, version}` | comment | the caller's Answer (`null` withdraws); `{key, option, changed, version, question}`; 404 unknown key, 400 `invalid_answer`, 409 `decided` / `not_head` / `deleted` |
//!
//! A Question as JSON: `{key, source: "table"|"thread", options: [{id, label, place}], rec,
//! decided, row, place, lines, thread, answers: [{by, option, at, version, changed}]}`. The web
//! page and the card renderer read the same [`Shown`] list, so the page and the API agree.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_renderer::questions::{self as q, Place, Question};
use annotary_store::{AnswerView, DocumentView, Right, ThreadView, VersionView, thread_option_ids};

use super::app::AppState;
use super::auth::Auth;
use super::docs::load_doc;
use super::error::ApiError;

/// One Answer as shown.
#[derive(Clone, Debug)]
pub struct ShownAnswer {
    pub handle: String,
    pub option: String,
    pub at: String,
    pub version: u32,
    /// Given on a Version whose Question read differently (its row or its section changed since).
    pub changed: bool,
}

/// One Option as shown: its id, the label of a Thread's Question, its own block in the prose.
#[derive(Clone, Debug)]
pub struct ShownOption {
    pub id: String,
    pub label: Option<String>,
    pub place: Option<Place>,
}

/// A Question as the API, the page and the CLI show it.
#[derive(Clone, Debug)]
pub struct Shown {
    pub key: String,
    /// From the decisions table (else asked on a Thread).
    pub table: bool,
    pub options: Vec<ShownOption>,
    pub rec: Option<String>,
    pub decided: Option<String>,
    /// The table row's line and the `Decided` column (table Questions).
    pub row: Option<u32>,
    pub cell: Option<u32>,
    pub place: Option<Place>,
    pub lines: Option<(u32, u32)>,
    /// The Question's Thread (a table Question's, once its first Comment exists; a Thread's own).
    pub thread: Option<String>,
    pub answers: Vec<ShownAnswer>,
}

impl Shown {
    pub fn is_decided(&self) -> bool {
        self.decided.is_some()
    }
    /// The `Decided` text these Answers record (what `pull` writes).
    pub fn decided_text(&self) -> Option<String> {
        let briefs: Vec<q::AnswerBrief> = self
            .answers
            .iter()
            .map(|a| q::AnswerBrief { handle: a.handle.clone(), option: a.option.clone(), at: a.at.clone() })
            .collect();
        q::decided_text(&briefs, self.thread.as_deref())
    }
    pub fn label_of(&self, id: &str) -> String {
        if id == q::OTHER {
            return "Other".to_string();
        }
        self.options.iter().find(|o| o.id == id).and_then(|o| o.label.clone()).unwrap_or_else(|| id.to_string())
    }
}

/// The Questions of `v` and of the Threads in `threads` (the Threads the caller shows, which
/// must be the Document's), with the Answers as they stood on `v` (the live ones on the head).
/// Reads the Answers only when there is a Question.
pub fn shown(
    st: &AppState,
    doc: &DocumentView,
    v: &VersionView,
    threads: &[ThreadView],
) -> Result<Vec<Shown>, ApiError> {
    let table: Vec<Question> = match v.questions.as_ref().and_then(q::from_json) {
        Some(qs) => qs,
        None => st.store.version_questions(&doc.id, v.number)?,
    };
    let asked: Vec<&ThreadView> = threads.iter().filter(|t| t.options.is_some() && !t.is_fully_deleted()).collect();
    if table.is_empty() && asked.is_empty() {
        return Ok(Vec::new());
    }
    let upto = (v.number != doc.head.number).then_some(v.number);
    let answers = st.store.answers(&doc.id, upto)?;
    let of = |key: &str, digest: Option<&str>| -> Vec<ShownAnswer> {
        answers
            .iter()
            .filter(|a| a.key == key)
            .map(|a: &AnswerView| ShownAnswer {
                handle: a.handle.clone(),
                option: a.option.clone(),
                at: a.at.clone(),
                version: a.version,
                changed: matches!((digest, &a.digest), (Some(d), Some(ad)) if d != ad),
            })
            .collect()
    };
    let mut out = Vec::new();
    for x in &table {
        let thread = threads.iter().find(|t| t.question.as_deref() == Some(x.key.as_str())).map(|t| t.id.clone());
        out.push(Shown {
            key: x.key.clone(),
            table: true,
            options: x
                .options
                .iter()
                .map(|id| ShownOption {
                    id: id.clone(),
                    label: None,
                    place: x.places.iter().find(|p| &p.id == id).map(|p| p.place.clone()),
                })
                .collect(),
            rec: x.rec.clone(),
            decided: x.decided.clone(),
            row: Some(x.row),
            cell: Some(x.cell),
            place: x.place.clone(),
            lines: x.lines,
            thread,
            answers: of(&x.key, Some(&x.digest)),
        });
    }
    for t in asked {
        let labels = t.options.clone().unwrap_or_default();
        out.push(Shown {
            key: t.id.clone(),
            table: false,
            options: thread_option_ids(labels.len())
                .into_iter()
                .zip(labels)
                .map(|(id, l)| ShownOption { id, label: Some(l), place: None })
                .collect(),
            rec: None,
            decided: None,
            row: None,
            cell: None,
            place: None,
            lines: None,
            thread: Some(t.id.clone()),
            answers: of(&t.id, None),
        });
    }
    Ok(out)
}

fn place_json(p: &Place) -> Value {
    json!({ "a": p.a, "b": p.b, "block": p.block })
}

pub fn shown_json(s: &Shown) -> Value {
    json!({
        "key": s.key,
        "source": if s.table { "table" } else { "thread" },
        "options": s.options.iter().map(|o| json!({ "id": o.id, "label": o.label, "place": o.place.as_ref().map(place_json) })).collect::<Vec<_>>(),
        "rec": s.rec,
        "decided": s.decided,
        "row": s.row,
        "place": s.place.as_ref().map(place_json),
        "lines": s.lines.map(|(a, b)| json!([a, b])),
        "thread": s.thread,
        "answers": s.answers.iter().map(|a| json!({ "by": a.handle, "option": a.option, "at": a.at, "version": a.version, "changed": a.changed })).collect::<Vec<_>>(),
    })
}

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub version: Option<u32>,
}

/// The Threads that hold Questions, live (the head) or as they stood on Version `n`.
fn threads_for(st: &AppState, doc: &DocumentView, n: u32) -> Result<Vec<ThreadView>, ApiError> {
    let filter = annotary_store::ThreadFilter { status: annotary_store::ThreadStatusFilter::All, ..Default::default() };
    let all = if n == doc.head.number {
        st.store.threads(&doc.id, &filter)?
    } else {
        st.store.threads_at(&doc.id, n, &filter)?
    };
    Ok(all.into_iter().filter(|t| t.question.is_some() || t.options.is_some()).collect())
}

pub async fn list(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let n = query.version.unwrap_or(doc.head.number);
    let v =
        st.store.version(&doc.id, n)?.ok_or_else(|| ApiError::not_found(format!("Document {} has no v{n}", doc.id)))?;
    let threads = threads_for(&st, &doc, n)?;
    let qs = shown(&st, &doc, &v, &threads)?;
    Ok(Json(json!({
        "id": doc.id,
        "name": doc.head.name,
        "version": n,
        "head": doc.head.number,
        "questions": qs.iter().map(shown_json).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct AnswerBody {
    /// An Option id, a Thread Question's label, `other`, or `null` to withdraw.
    pub option: Option<String>,
    /// The head the caller saw.
    pub version: u32,
}

pub async fn answer(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, key)): Path<(String, String)>,
    body: Result<Json<AnswerBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Comment)?;
    let Json(body) = body?;
    let option = body.option.as_deref().map(str::trim).filter(|o| !o.is_empty());
    let out = st.store.answer(&auth.0.member_id, &doc.id, &key, option, body.version).map_err(|e| match e {
        annotary_store::StoreError::QuestionNotFound { key, .. } => {
            ApiError::not_found(format!("no Question {key} in {} ({})", doc.head.name, doc.id))
                .hint("`annotary show DOC` lists its Questions on the questions: line")
        }
        annotary_store::StoreError::NotHead { expected, head } => {
            ApiError::new(axum::http::StatusCode::CONFLICT, "not_head", format!("head is v{head}, not v{expected}"))
                .hint("answer on the head Version")
        }
        other => other.into(),
    })?;
    if out.changed {
        tracing::info!("{} answered {} on {} ({})", auth.0.handle, out.key, doc.head.name, doc.id);
    }
    let doc = load_doc(&st, &id)?;
    let threads = threads_for(&st, &doc, doc.head.number)?;
    let question =
        shown(&st, &doc, &doc.head, &threads)?.into_iter().find(|s| s.key == out.key).map(|s| shown_json(&s));
    Ok(Json(json!({
        "doc": doc.id,
        "name": doc.head.name,
        "key": out.key,
        "option": out.option,
        "changed": out.changed,
        "version": out.version,
        "question": question,
    })))
}
