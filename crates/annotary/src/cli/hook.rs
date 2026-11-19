//! `hook add [DOC] --on KINDS [--once] [--expires DUR]`, `hook list`, `hook remove ID`,
//! `hook test ID` — the CLI's side of `/api/v1/me/hooks` (`server/hooks.rs`, docs/dev/hooks.md).
//!
//! `add` reads the receiver's URL from stdin, never from the command line (it holds the
//! receiver's own token): the first line is the URL, an optional second line the receiver's own
//! signing secret. On a terminal it asks for the URL without echo. It prints `ID SECRET` once;
//! nothing prints the URL back. `list` prints `ID ON DOC STATE HOST LAST EXPIRES`; `remove`
//! prints `ID removed`; `test` prints `ID STATUS ok|failed` and exits 5 when the receiver did not
//! take it.

use std::io::{BufRead, IsTerminal};

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::client::qs;
use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds, relative_age};

use super::Ctx;
use super::docref::{DeletedMode, resolve};
use super::wait::parse_timeout;

#[derive(Subcommand, Debug)]
pub enum HookCmd {
    /// Add a hook: URL (and an optional secret) on stdin; prints ID SECRET once
    Add(AddArgs),
    /// Your hooks: id, kinds, Document, state, host, last answer, expiry
    List,
    /// Remove a hook
    Remove(IdArg),
    /// Send the hook one signed test body now
    Test(IdArg),
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Only this Document (id, unique Name, or path to a pulled .md); default: every one you read
    pub doc: Option<String>,
    /// What wakes it: version, action, mention (comma list)
    #[arg(long, value_name = "KINDS", default_value = "action,mention")]
    pub on: String,
    /// Remove the hook after its first delivery
    #[arg(long)]
    pub once: bool,
    /// How long the hook lives (1m to 30d)
    #[arg(long, value_name = "DUR", default_value = "7d")]
    pub expires: String,
    /// Vault to resolve DOC in
    #[arg(long, value_name = "NAME")]
    pub vault: Option<String>,
}

#[derive(Args, Debug)]
pub struct IdArg {
    /// The hook's id (h3)
    pub id: String,
}

/// `--json` fields for `hook add`.
pub const ADD_FIELDS: FieldSpec = FieldSpec {
    all: &["id", "secret", "on", "once", "doc", "name", "host", "url_fp", "expires_at"],
    default: &["id", "secret", "on", "once", "doc", "host", "expires_at"],
};

/// `--json` fields for `hook list`.
pub const LIST_FIELDS: FieldSpec = FieldSpec {
    all: &[
        "id",
        "on",
        "once",
        "doc",
        "name",
        "state",
        "host",
        "url_fp",
        "last_status",
        "last_attempt_at",
        "failures",
        "created_at",
        "expires_at",
        "member",
    ],
    default: &["id", "on", "once", "doc", "state", "host", "last_status", "expires_at"],
};

/// `--json` fields for `hook remove`.
pub const REMOVE_FIELDS: FieldSpec = FieldSpec { all: &["id", "removed"], default: &["id", "removed"] };

/// `--json` fields for `hook test`.
pub const TEST_FIELDS: FieldSpec = FieldSpec { all: &["id", "status", "ok"], default: &["id", "status", "ok"] };

pub fn run(ctx: &Ctx, cmd: HookCmd) -> CliResult<()> {
    match cmd {
        HookCmd::Add(a) => add(ctx, a),
        HookCmd::List => list(ctx),
        HookCmd::Remove(a) => remove(ctx, a),
        HookCmd::Test(a) => test(ctx, a),
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn kinds(v: &Value) -> String {
    v["on"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(",")).unwrap_or_default()
}

/// The URL and the optional secret: stdin lines, or a no-echo prompt on a terminal.
fn read_url() -> CliResult<(String, Option<String>)> {
    if std::io::stdin().is_terminal() {
        let url = rpassword::prompt_password("hook URL (not echoed): ")
            .map_err(|e| CliError::usage(format!("cannot read the URL: {e}")))?;
        return Ok((url.trim().to_string(), None));
    }
    let mut lines = std::io::stdin().lock().lines();
    let url =
        lines.next().transpose().map_err(|e| CliError::usage(format!("cannot read stdin: {e}")))?.unwrap_or_default();
    let secret = lines.next().transpose().ok().flatten().map(|l| l.trim().to_string()).filter(|l| !l.is_empty());
    Ok((url.trim().to_string(), secret))
}

fn add(ctx: &Ctx, a: AddArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&ADD_FIELDS, Vec::new(), true);
    }
    let expires = parse_timeout(&a.expires)
        .map_err(|_| CliError::usage(format!("--expires {:?}: use a duration such as 30m, 12h or 7d", a.expires)))?
        .ok_or_else(|| CliError::usage("--expires 0: a hook always expires (at most 30d)"))?;
    let (url, secret) = read_url()?;
    if url.is_empty() {
        return Err(CliError::usage("no URL on stdin")
            .hint("pipe it in, never on the command line: `printf '%s\\n' \"$URL\" | annotary hook add DOC --once`"));
    }
    let api = ctx.api()?;
    let doc = match a.doc.as_deref() {
        Some(d) => Some(s(&resolve(ctx, &api, d, a.vault.as_deref(), DeletedMode::Exclude)?, "id")),
        None => None,
    };
    let mut body = json!({ "url": url, "on": a.on, "once": a.once, "expires": expires.as_secs() });
    if let Some(d) = doc {
        body["doc"] = json!(d);
    }
    if let Some(sec) = secret {
        body["secret"] = json!(sec);
    }
    let out = api.post("/api/v1/me/hooks", &body)?;
    let h = &out["hook"];
    let secret = s(&out, "secret");
    if ctx.out.is_json() {
        let mut r = Map::new();
        r.insert("id".into(), json!(s(h, "id")));
        r.insert("secret".into(), json!(secret));
        r.insert("on".into(), json!(kinds(h)));
        r.insert("once".into(), h["once"].clone());
        r.insert("doc".into(), h["doc"].clone());
        r.insert("name".into(), h["name"].clone());
        r.insert("host".into(), json!(s(h, "host")));
        r.insert("url_fp".into(), json!(s(h, "url_fp")));
        r.insert("expires_at".into(), json!(iso_seconds(&s(h, "expires_at"))));
        return ctx.out.json_records(&ADD_FIELDS, vec![r], true);
    }
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&[s(h, "id"), secret].join(sep));
    ctx.out.note(&format!(
        "{} → {} on {}; the secret signs every delivery (X-Annotary-Signature) and is not shown again",
        s(h, "id"),
        s(h, "host"),
        kinds(h)
    ));
    Ok(())
}

fn record(h: &Value) -> Map<String, Value> {
    let mut r = Map::new();
    r.insert("id".into(), json!(s(h, "id")));
    r.insert("on".into(), json!(kinds(h)));
    r.insert("once".into(), h["once"].clone());
    r.insert("doc".into(), h["doc"].clone());
    r.insert("name".into(), h["name"].clone());
    r.insert("state".into(), json!(s(h, "state")));
    r.insert("host".into(), json!(s(h, "host")));
    r.insert("url_fp".into(), json!(s(h, "url_fp")));
    r.insert("last_status".into(), h["last_status"].clone());
    r.insert("last_attempt_at".into(), json!(iso_seconds(&s(h, "last_attempt_at"))));
    r.insert("failures".into(), h["failures"].clone());
    r.insert("created_at".into(), json!(iso_seconds(&s(h, "created_at"))));
    r.insert("expires_at".into(), json!(iso_seconds(&s(h, "expires_at"))));
    r.insert("member".into(), json!(s(h, "member")));
    r
}

fn list(ctx: &Ctx) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let hooks = api.get("/api/v1/me/hooks")?["hooks"].as_array().cloned().unwrap_or_default();
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, hooks.iter().map(record).collect(), false);
    }
    if ctx.out.tty && hooks.is_empty() {
        ctx.out.note("no hooks — `annotary hook add` makes one (`annotary help setup`)");
        return Ok(());
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<Vec<String>> = hooks
        .iter()
        .map(|h| {
            let doc = if s(h, "name").is_empty() { "*".to_string() } else { s(h, "name") };
            let on = if h["once"].as_bool().unwrap_or(false) { format!("{} once", kinds(h)) } else { kinds(h) };
            let last = h["last_status"].as_str().unwrap_or("-").to_string();
            let exp = s(h, "expires_at");
            let exp = if exp.is_empty() { "-".to_string() } else { format!("in {}", relative_age_until(&exp, now)) };
            vec![s(h, "id"), on, doc, s(h, "state"), s(h, "host"), last, exp]
        })
        .collect();
    print!("{}", ctx.out.table(&["ID", "ON", "DOC", "STATE", "HOST", "LAST", "EXPIRES"], &rows, &[]));
    Ok(())
}

/// `3d` until a future stamp (`relative_age` counts back from now; this counts forward).
fn relative_age_until(stamp: &str, now: jiff::Timestamp) -> String {
    match stamp.parse::<jiff::Timestamp>() {
        Ok(t) if t > now => {
            let mirrored = now - t.duration_since(now);
            relative_age(&annotary_store::time::format(mirrored), now)
        }
        _ => "0s".to_string(),
    }
}

fn remove(ctx: &Ctx, a: IdArg) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&REMOVE_FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let out = api.delete(&format!("/api/v1/me/hooks/{}", qs(a.id.trim())))?;
    if ctx.out.is_json() {
        let mut r = Map::new();
        r.insert("id".into(), json!(s(&out, "id")));
        r.insert("removed".into(), out["removed"].clone());
        return ctx.out.json_records(&REMOVE_FIELDS, vec![r], true);
    }
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&[s(&out, "id"), "removed".to_string()].join(sep));
    Ok(())
}

fn test(ctx: &Ctx, a: IdArg) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&TEST_FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let out = api.post(&format!("/api/v1/me/hooks/{}/test", qs(a.id.trim())), &json!({}))?;
    let ok = out["ok"].as_bool().unwrap_or(false);
    if ctx.out.is_json() {
        let mut r = Map::new();
        r.insert("id".into(), json!(s(&out, "id")));
        r.insert("status".into(), json!(s(&out, "status")));
        r.insert("ok".into(), json!(ok));
        ctx.out.json_records(&TEST_FIELDS, vec![r], true)?;
    } else {
        let sep = if ctx.out.tty { "  " } else { "\t" };
        ctx.out.line(&[s(&out, "id"), s(&out, "status"), if ok { "ok" } else { "failed" }.to_string()].join(sep));
    }
    if ok {
        Ok(())
    } else {
        Err(CliError::refused(format!("the receiver did not take the test body ({})", s(&out, "status")))
            .hint("check the URL and the receiver's signature secret; `annotary hook list` shows the host"))
    }
}
