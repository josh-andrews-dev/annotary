//! Questions and Answers (schema v8; `docs/dev/questions.md`). A Question comes from a Version's
//! decisions table (its key, `D4`) or from a Thread that asked one (its key is the Thread id,
//! `t7`, and its Options `A`, `B`, … carry the asked labels). An Answer is one Member's current
//! choice for a key, an Option id, [`OTHER`] or none; the `answers` table is append-only and a
//! Member's current Answer is their latest row. The parsing is the pure
//! [`annotary_renderer::questions`]; this module stores and reads.

use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;

use annotary_renderer::questions::{self as q, AnswerBrief, OTHER, Question, Recorded};

use crate::error::{Result, StoreError};
use crate::store::{Store, append_event};
use crate::threads::{parse_thread_id, thread_id};
use crate::time;

/// Longest label of a Question asked on a Thread, in characters.
pub const LABEL_MAX: usize = 80;

/// One Member's current Answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnswerView {
    pub key: String,
    pub member_id: String,
    pub handle: String,
    /// An Option id or [`OTHER`].
    pub option: String,
    /// The head when it was given.
    pub version: u32,
    pub at: String,
    /// The Question's digest on that Version (`None` for a Thread's Question).
    pub digest: Option<String>,
}

impl AnswerView {
    pub fn brief(&self) -> AnswerBrief {
        AnswerBrief { handle: self.handle.clone(), option: self.option.clone(), at: self.at.clone() }
    }
}

/// What [`Store::answer`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnswerOutcome {
    pub key: String,
    /// The caller's Answer now (`None` = none).
    pub option: Option<String>,
    /// False when it already was that (nothing written, no event).
    pub changed: bool,
    pub version: u32,
}

/// The Option ids of a Question asked on a Thread with `n` labels: `A`, `B`, …
pub fn thread_option_ids(n: usize) -> Vec<String> {
    (0..n).map(|i| ((b'A' + i as u8) as char).to_string()).collect()
}

/// Check the labels of a Question asked on a Thread: 2 to 6, each 1 to [`LABEL_MAX`] characters
/// on one line, no two alike (ignoring case). Answers them trimmed.
pub fn check_labels(labels: &[String]) -> Result<Vec<String>> {
    let bad = |reason: String| StoreError::Invalid { what: "options", reason };
    if labels.len() < q::OPTIONS_MIN || labels.len() > q::OPTIONS_MAX {
        return Err(bad(format!(
            "a Question has {} to {} options, not {}",
            q::OPTIONS_MIN,
            q::OPTIONS_MAX,
            labels.len()
        )));
    }
    let mut out: Vec<String> = Vec::new();
    for l in labels {
        let l = l.trim();
        if l.is_empty() || l.chars().count() > LABEL_MAX || l.contains(['\n', '\r']) {
            return Err(bad(format!("an option is 1 to {LABEL_MAX} characters on one line: {l:?}")));
        }
        if out.iter().any(|o| o.eq_ignore_ascii_case(l)) {
            return Err(bad(format!("two options read {l:?}")));
        }
        out.push(l.to_string());
    }
    Ok(out)
}

/// The Questions stored on Version `n`, or `None` when it predates them (NULL column).
fn stored(conn: &rusqlite::Connection, doc_id: &str, n: u32) -> Result<(Option<Vec<Question>>, String)> {
    let (json, md): (Option<String>, String) = conn
        .query_row(
            "SELECT questions, md_hash FROM versions WHERE doc_id = ?1 AND number = ?2",
            params![doc_id, n as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::VersionNotFound { doc: doc_id.to_string(), number: n })?;
    let parsed = json.and_then(|j| serde_json::from_str::<Value>(&j).ok()).and_then(|v| q::from_json(&v));
    Ok((parsed, md))
}

/// The current Answers of a Document (oldest first), as they stood on Version `upto` when given
/// (rows given while a later Version was the head are left out).
pub(crate) fn current_answers(conn: &rusqlite::Connection, doc_id: &str, upto: Option<u32>) -> Result<Vec<AnswerView>> {
    let mut stmt = conn.prepare(
        "SELECT a.key, a.member_id, ms.handle, a.option, a.version, a.at, a.digest
         FROM answers a JOIN memberships ms ON ms.member_id = a.member_id
         WHERE a.doc_id = ?1 AND a.seq = (SELECT MAX(b.seq) FROM answers b
             WHERE b.doc_id = a.doc_id AND b.key = a.key AND b.member_id = a.member_id AND (?2 IS NULL OR b.version <= ?2))
         ORDER BY a.seq",
    )?;
    let rows = stmt.query_map(params![doc_id, upto.map(|n| n as i64)], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)? as u32,
            r.get::<_, String>(5)?,
            r.get::<_, Option<String>>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (key, member_id, handle, option, version, at, digest) = row?;
        if let Some(option) = option {
            out.push(AnswerView { key, member_id, handle, option, version, at, digest });
        }
    }
    Ok(out)
}

/// The Thread carrying Document Question `key`, if one was started.
fn question_thread(conn: &rusqlite::Connection, doc_id: &str, key: &str) -> Result<Option<u32>> {
    Ok(conn
        .query_row(
            "SELECT num FROM threads WHERE doc_id = ?1 AND question = ?2 ORDER BY num LIMIT 1",
            params![doc_id, key],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .map(|n| n as u32))
}

/// The `Decided` text each answered, undecided Question of `head` should carry (key → text).
pub(crate) fn decided_texts(
    conn: &rusqlite::Connection,
    doc_id: &str,
    head: &[Question],
) -> Result<Vec<(String, String)>> {
    let answers = current_answers(conn, doc_id, None)?;
    let mut out = Vec::new();
    for question in head.iter().filter(|x| !x.is_decided()) {
        let mine: Vec<AnswerBrief> = answers.iter().filter(|a| a.key == question.key).map(AnswerView::brief).collect();
        let thread = question_thread(conn, doc_id, &question.key)?.map(thread_id);
        if let Some(text) = q::decided_text(&mine, thread.as_deref()) {
            out.push((question.key.clone(), text));
        }
    }
    Ok(out)
}

impl Store {
    /// The Questions of Version `n`. A Version pushed before Questions existed has none stored:
    /// they are computed from its Markdown with the same crate and stored (no event), as a
    /// missing render is.
    pub fn version_questions(&self, doc_id: &str, n: u32) -> Result<Vec<Question>> {
        let (found, md) = stored(&self.conn(), doc_id, n)?;
        if let Some(qs) = found {
            return Ok(qs);
        }
        let text = self.blobs.read(&md)?.ok_or(StoreError::BlobMissing(md))?;
        let qs = q::find(&String::from_utf8_lossy(&text)).questions;
        let json = serde_json::to_string(&q::to_json(&qs))?;
        self.tx(|tx| {
            tx.execute(
                "UPDATE versions SET questions = ?3 WHERE doc_id = ?1 AND number = ?2 AND questions IS NULL",
                params![doc_id, n as i64, json],
            )?;
            Ok(())
        })?;
        Ok(qs)
    }

    /// The current Answers of a Document (oldest first); with `upto`, as they stood on that
    /// Version.
    pub fn answers(&self, doc_id: &str, upto: Option<u32>) -> Result<Vec<AnswerView>> {
        current_answers(&self.conn(), doc_id, upto)
    }

    /// What `pull` writes into the `Decided` cells of the head: key → text, for each answered
    /// Question the head leaves undecided.
    pub fn decided_fills(&self, doc_id: &str) -> Result<Vec<(String, String)>> {
        let head = self.document(doc_id)?.ok_or_else(|| StoreError::DocumentNotFound(doc_id.to_string()))?.head.number;
        let qs = self.version_questions(doc_id, head)?;
        decided_texts(&self.conn(), doc_id, &qs)
    }

    /// Set `by`'s Answer to Question `key` (`option` `None` withdraws it) on the head, which must
    /// be `version`. A Document Question must be on the head and undecided there, and `option`
    /// one of its ids or [`OTHER`]; a Thread's Question (`t7`) takes an id or one of its labels
    /// (ignoring case). The same Answer again writes nothing. Appends `question.answered {key,
    /// option, version}`.
    pub fn answer(
        &self,
        by: &str,
        doc_id: &str,
        key: &str,
        option: Option<&str>,
        version: u32,
    ) -> Result<AnswerOutcome> {
        let key = key.trim();
        self.tx(|tx| {
            let deleted: Option<Option<String>> =
                tx.query_row("SELECT deleted_at FROM documents WHERE id = ?1", [doc_id], |r| r.get(0)).optional()?;
            match deleted {
                None => return Err(StoreError::DocumentNotFound(doc_id.to_string())),
                Some(Some(_)) => return Err(StoreError::DocumentDeleted(doc_id.to_string())),
                Some(None) => {}
            }
            let head: u32 =
                tx.query_row("SELECT MAX(number) FROM versions WHERE doc_id = ?1", [doc_id], |r| r.get::<_, i64>(0))?
                    as u32;
            if version != head {
                return Err(StoreError::NotHead { expected: version, head });
            }
            let not_found = || StoreError::QuestionNotFound { doc: doc_id.to_string(), key: key.to_string() };
            let (key, option, digest) = match parse_thread_id(key) {
                Some(num) => {
                    let labels: Option<String> = tx
                        .query_row("SELECT options FROM threads WHERE doc_id = ?1 AND num = ?2", params![doc_id, num as i64], |r| {
                            r.get(0)
                        })
                        .optional()?
                        .flatten();
                    let labels: Vec<String> =
                        labels.and_then(|l| serde_json::from_str(&l).ok()).ok_or_else(not_found)?;
                    let ids = thread_option_ids(labels.len());
                    let option = match option.map(str::trim) {
                        None => None,
                        Some(o) if o.eq_ignore_ascii_case(OTHER) => Some(OTHER.to_string()),
                        Some(o) => Some(
                            ids.iter()
                                .zip(&labels)
                                .find(|(id, l)| *id == o || l.eq_ignore_ascii_case(o))
                                .map(|(id, _)| id.clone())
                                .ok_or_else(|| {
                                    StoreError::InvalidAnswer(format!(
                                        "{o:?} is not an answer to {key}: {}, or other",
                                        ids.iter().zip(&labels).map(|(i, l)| format!("{i} ({l})")).collect::<Vec<_>>().join(", ")
                                    ))
                                })?,
                        ),
                    };
                    (thread_id(num), option, None)
                }
                None => {
                    let head_qs = questions_in_tx(self, tx, doc_id, head)?;
                    let question = head_qs.iter().find(|x| x.key == key).ok_or_else(not_found)?;
                    if let Some(d) = &question.decided {
                        return Err(StoreError::QuestionDecided { key: key.to_string(), decided: d.clone() });
                    }
                    let option = match option.map(str::trim) {
                        None => None,
                        Some(o) if o.eq_ignore_ascii_case(OTHER) => Some(OTHER.to_string()),
                        Some(o) if question.has_option(o) => Some(o.to_string()),
                        Some(o) => {
                            return Err(StoreError::InvalidAnswer(format!(
                                "{o:?} is not an option of {key}: {}, or other",
                                question.options.join(", ")
                            )));
                        }
                    };
                    (key.to_string(), option, Some(question.digest.clone()))
                }
            };
            let current: Option<Option<String>> = tx
                .query_row(
                    "SELECT option FROM answers WHERE doc_id = ?1 AND key = ?2 AND member_id = ?3 ORDER BY seq DESC LIMIT 1",
                    params![doc_id, key, by],
                    |r| r.get(0),
                )
                .optional()?;
            if current.clone().flatten() == option {
                return Ok(AnswerOutcome { key, option, changed: false, version: head });
            }
            let now = time::now();
            tx.execute(
                "INSERT INTO answers(doc_id, key, member_id, option, version, digest, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![doc_id, key, by, option, head as i64, digest, now],
            )?;
            crate::docs::bump_activity(tx, doc_id, &now)?;
            append_event(
                tx,
                Some(by),
                "question.answered",
                Some(doc_id),
                serde_json::json!({ "key": key, "option": option, "version": head }),
            )?;
            Ok(AnswerOutcome { key, option, changed: true, version: head })
        })
    }
}

/// The commit rule (decision D2): with `head` the head's Questions and `local` the pushed
/// Version's, the Answers the pushed Markdown leaves out of its `Decided` cells.
pub(crate) fn unrecorded(
    tx: &Transaction,
    doc_id: &str,
    head: &[Question],
    md: &str,
    local: &[Question],
) -> Result<Recorded> {
    let texts = decided_texts(tx, doc_id, head)?;
    Ok(q::check_recorded(head, md, local, &texts))
}

/// The head's Questions inside a transaction (computed from its Markdown when not stored).
pub(crate) fn questions_in_tx(store: &Store, tx: &Transaction, doc_id: &str, n: u32) -> Result<Vec<Question>> {
    let (found, md) = stored(tx, doc_id, n)?;
    if let Some(qs) = found {
        return Ok(qs);
    }
    let text = store.blobs.read(&md)?.ok_or(StoreError::BlobMissing(md))?;
    Ok(q::find(&String::from_utf8_lossy(&text)).questions)
}
