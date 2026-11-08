//! `avatar set FILE [--static FILE]`, `avatar clear [--member HANDLE]`, `avatar [show] [HANDLE]`:
//! a Member's picture beside their Comments, over `/api/v1/me/avatar` (`server/avatars.rs`).
//!
//! `set` and `show` print `@handle TYPE SIZE STATIC` (STATIC the fallback's type, or `-`), or
//! `@handle none` for a Member with no avatar; `clear` prints `@handle cleared`. The server reads
//! the images and refuses what is not allowed, so the CLI only sends the bytes; a file over the
//! largest cap is refused here, before it is read into a request.

use std::path::{Path, PathBuf};

use base64::Engine;
use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use annotary_store::avatar::IMAGE_MAX;
use annotary_store::caps::human_size;

use crate::client::qs;
use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds};

use super::Ctx;
use super::member::clean_handle;

#[derive(Subcommand, Debug)]
pub enum AvatarCmd {
    /// Set your avatar: a PNG, JPEG, GIF, WebP or SVG (--static: a still image for reduced motion)
    Set(SetArgs),
    /// Clear your avatar (an owner: anyone's, with --member)
    Clear(ClearArgs),
    /// Show an avatar: yours, or HANDLE's
    Show(ShowArgs),
}

#[derive(Args, Debug)]
pub struct SetArgs {
    /// The image file
    pub file: PathBuf,
    /// A PNG, JPEG or WebP shown instead to people who turn off motion
    #[arg(long = "static", value_name = "FILE")]
    pub static_file: Option<PathBuf>,
}

#[derive(Args, Debug, Default)]
pub struct ClearArgs {
    /// Another Member's handle (owners only)
    #[arg(long, value_name = "HANDLE")]
    pub member: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct ShowArgs {
    /// A Member's handle (default: you)
    pub handle: Option<String>,
}

/// `--json` fields for `avatar set` and `avatar show`.
pub const FIELDS: FieldSpec = FieldSpec {
    all: &[
        "handle",
        "type",
        "size",
        "hash",
        "url",
        "static_type",
        "static_size",
        "static_hash",
        "static_url",
        "set_at",
        "changed",
    ],
    default: &["handle", "type", "size", "url", "static_type", "static_url", "changed"],
};

/// `--json` fields for `avatar clear`.
pub const CLEAR_FIELDS: FieldSpec = FieldSpec { all: &["handle", "changed"], default: &["handle", "changed"] };

pub fn run(ctx: &Ctx, cmd: Option<AvatarCmd>) -> CliResult<()> {
    match cmd.unwrap_or(AvatarCmd::Show(ShowArgs::default())) {
        AvatarCmd::Set(a) => set(ctx, a),
        AvatarCmd::Clear(a) => clear(ctx, a),
        AvatarCmd::Show(a) => show(ctx, a),
    }
}

/// A file's bytes, base64 for the JSON body.
fn read_file(path: &Path) -> CliResult<String> {
    let bytes = std::fs::read(path).map_err(|e| CliError::usage(format!("cannot read {}: {e}", path.display())))?;
    if bytes.len() as u64 > IMAGE_MAX {
        return Err(CliError::refused(format!(
            "{} is {} — the cap is {}",
            path.display(),
            human_size(bytes.len() as u64),
            human_size(IMAGE_MAX)
        ))
        .hint("avatars: an image up to 256 KB (an SVG up to 64 KB), a static fallback up to 256 KB"));
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A member JSON → the flat record for text and JSON.
fn record(m: &Value) -> Map<String, Value> {
    let mut r = Map::new();
    r.insert("handle".into(), json!(s(m, "handle")));
    let a = &m["avatar"];
    for key in ["type", "size", "hash", "url", "static_type", "static_size", "static_hash", "static_url"] {
        r.insert(key.into(), a.get(key).cloned().unwrap_or(Value::Null));
    }
    r.insert("set_at".into(), json!(iso_seconds(&s(a, "set_at"))));
    r.insert("changed".into(), m.get("changed").cloned().unwrap_or(json!(false)));
    r
}

fn print_row(ctx: &Ctx, fields: &[String]) {
    ctx.out.line(&fields.join(if ctx.out.tty { "  " } else { "\t" }));
}

fn print_record(ctx: &Ctx, r: &Map<String, Value>) {
    let handle = format!("@{}", r["handle"].as_str().unwrap_or(""));
    let Some(ty) = r["type"].as_str() else {
        print_row(ctx, &[handle, "none".to_string()]);
        return;
    };
    let size = human_size(r["size"].as_u64().unwrap_or(0));
    let still = r["static_type"].as_str().unwrap_or("-").to_string();
    print_row(ctx, &[handle, ty.to_string(), size, still]);
}

fn set(ctx: &Ctx, a: SetArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let mut body = json!({ "image": read_file(&a.file)? });
    if let Some(path) = &a.static_file {
        body["static"] = json!(read_file(path)?);
    }
    let answer = ctx.api()?.put_json("/api/v1/me/avatar", &body)?;
    let rec = record(&answer);
    if answer["changed"] == json!(false) {
        ctx.out.note(&format!("@{}'s avatar is unchanged", s(&answer, "handle")));
    }
    if ctx.out.is_json() {
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    print_record(ctx, &rec);
    Ok(())
}

fn clear(ctx: &Ctx, a: ClearArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&CLEAR_FIELDS, Vec::new(), true);
    }
    let path = match a.member.as_deref().map(clean_handle) {
        Some(h) => format!("/api/v1/members/{}/avatar", qs(&h)),
        None => "/api/v1/me/avatar".to_string(),
    };
    let answer = ctx.api()?.delete(&path)?;
    let handle = s(&answer, "handle");
    let changed = answer["changed"] == json!(true);
    if !changed {
        ctx.out.note(&format!("@{handle} had no avatar"));
    }
    if ctx.out.is_json() {
        let mut r = Map::new();
        r.insert("handle".into(), json!(handle));
        r.insert("changed".into(), json!(changed));
        return ctx.out.json_records(&CLEAR_FIELDS, vec![r], true);
    }
    print_row(ctx, &[format!("@{handle}"), if changed { "cleared" } else { "none" }.to_string()]);
    Ok(())
}

fn show(ctx: &Ctx, a: ShowArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let path = match a.handle.as_deref().map(clean_handle) {
        Some(h) => format!("/api/v1/members/{}", qs(&h)),
        None => "/api/v1/me".to_string(),
    };
    let rec = record(&ctx.api()?.get(&path)?);
    if ctx.out.is_json() {
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    print_record(ctx, &rec);
    Ok(())
}
