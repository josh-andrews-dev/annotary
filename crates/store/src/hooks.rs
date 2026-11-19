//! Hooks: outbound webhooks that wake an agent when a Document it handed over gets a Version, an
//! Action or an @mention of it (docs/dev/hooks.md). The store keeps the rows and their cursors;
//! the server's deliverer (`server/hooks.rs`) reads the events log after each cursor and POSTs.
//!
//! A hook's `url` holds the receiver's own token and its `secret` signs every body, so neither
//! leaves the store through the API: a hook is shown by its host and the URL's
//! [`fingerprint`]. Adding, removing and disabling a hook append `hook.added` / `hook.removed` /
//! `hook.disabled` (no URL, no secret). A delivery attempt only moves the hook's own bookkeeping
//! (cursor, status, backoff) in a hand-made transaction with no event and no commit hook, as
//! `touch_device` does: an event per delivery would wake the deliverer again for nothing.

use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::{Result, StoreError};
use crate::model::{AuthMember, AuthVia, Role};
use crate::store::{Store, append_event};
use crate::{ids, time};

/// The event words a hook may carry (`wait`'s words for a handoff).
pub const HOOK_KINDS: &[&str] = &["version", "action", "mention"];
/// Live hooks one Member may hold.
pub const MAX_HOOKS_PER_MEMBER: usize = 20;
/// A hook's life when `expires` is not given: 7 days.
pub const DEFAULT_EXPIRY_SECS: i64 = 7 * 86_400;
/// The longest life a hook may ask for: 30 days.
pub const MAX_EXPIRY_SECS: i64 = 30 * 86_400;
/// Characters of the generated signing secret (base62, > 190 bits).
pub const SECRET_LEN: usize = 32;

/// A hook to add. `kinds` are [`HOOK_KINDS`] words; `expires_in` seconds (None = the default).
#[derive(Clone, Debug)]
pub struct NewHook {
    pub member_id: String,
    pub token_id: Option<String>,
    pub doc_id: Option<String>,
    pub kinds: Vec<String>,
    pub once: bool,
    pub url: String,
    pub expires_in: Option<i64>,
}

/// One stored hook, secrets included: only the server's deliverer and the create response see
/// `url` / `secret`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookRow {
    pub n: i64,
    pub member_id: String,
    pub token_id: Option<String>,
    pub doc_id: Option<String>,
    pub kinds: Vec<String>,
    pub once: bool,
    pub url: String,
    pub secret: String,
    pub cursor_seq: i64,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub last_status: Option<String>,
    pub last_attempt_at: Option<String>,
    pub failures: i64,
    pub failing_since: Option<String>,
    pub next_attempt_at: Option<String>,
    pub disabled_at: Option<String>,
}

impl HookRow {
    /// The id people type: `h` + the row number.
    pub fn id(&self) -> String {
        hook_id(self.n)
    }

    pub fn host(&self) -> String {
        url_host(&self.url).unwrap_or_default()
    }

    pub fn url_fingerprint(&self) -> String {
        fingerprint(&self.url)
    }

    /// Live: not disabled, not expired at `now`.
    pub fn is_live(&self, now: &str) -> bool {
        self.disabled_at.is_none() && self.expires_at.as_deref().is_none_or(|e| e > now)
    }
}

/// What one delivery attempt did to a hook's bookkeeping.
#[derive(Clone, Debug, Default)]
pub struct HookAttempt {
    /// The new cursor (after a delivered event, or past events the hook skips); None = unchanged.
    pub cursor: Option<i64>,
    /// `200`, `503`, `timeout`, `refused: …` — what the receiver (or the address check) said.
    pub status: String,
    /// Delivered: failures reset. Otherwise `failures` + 1 and `next_attempt_at` set.
    pub ok: bool,
    pub next_attempt_at: Option<String>,
}

pub fn hook_id(n: i64) -> String {
    format!("h{n}")
}

/// `h12` or `12` → 12.
pub fn parse_hook_id(s: &str) -> Option<i64> {
    let s = s.trim();
    let digits = s.strip_prefix('h').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|n| *n > 0)
}

/// `sha256(text)` as hex, first 10 characters: how a secret is compared and shown.
pub fn fingerprint(text: &str) -> String {
    let d = Sha256::digest(text.as_bytes());
    to_hex(&d)[..10].to_string()
}

pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

/// HMAC-SHA256 (RFC 2104) of `msg` keyed by `key`.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

/// The `X-Annotary-Signature` value for `body`: `sha256=<hex>`.
pub fn signature(secret: &str, body: &[u8]) -> String {
    format!("sha256={}", to_hex(&hmac_sha256(secret.as_bytes(), body)))
}

/// The host of an `http(s)://` URL (no user info, no port), lowercased; None when it has none.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split_once(']')?.0
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Kinds as given (`action,mention`), checked against [`HOOK_KINDS`], deduplicated, in catalog
/// order.
pub fn parse_kinds(words: &str) -> std::result::Result<Vec<String>, String> {
    let mut seen = Vec::new();
    for w in words.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        if !HOOK_KINDS.contains(&w) {
            return Err(format!("{w:?} is not a hook kind: use {}", HOOK_KINDS.join(",")));
        }
        if !seen.iter().any(|s| s == w) {
            seen.push(w.to_string());
        }
    }
    if seen.is_empty() {
        return Err(format!("name at least one kind: {}", HOOK_KINDS.join(",")));
    }
    seen.sort_by_key(|w| HOOK_KINDS.iter().position(|k| k == w));
    Ok(seen)
}

const HOOK_SELECT: &str = "SELECT n, member_id, token_id, doc_id, kinds, once, url, secret, cursor_seq, expires_at,
    created_at, last_status, last_attempt_at, failures, failing_since, next_attempt_at, disabled_at FROM hooks";

fn row_hook(r: &Row) -> rusqlite::Result<HookRow> {
    let kinds: String = r.get(4)?;
    Ok(HookRow {
        n: r.get(0)?,
        member_id: r.get(1)?,
        token_id: r.get(2)?,
        doc_id: r.get(3)?,
        kinds: kinds.split(',').filter(|k| !k.is_empty()).map(str::to_string).collect(),
        once: r.get::<_, i64>(5)? != 0,
        url: r.get(6)?,
        secret: r.get(7)?,
        cursor_seq: r.get(8)?,
        expires_at: r.get(9)?,
        created_at: r.get(10)?,
        last_status: r.get(11)?,
        last_attempt_at: r.get(12)?,
        failures: r.get(13)?,
        failing_since: r.get(14)?,
        next_attempt_at: r.get(15)?,
        disabled_at: r.get(16)?,
    })
}

fn load_hook(tx: &Transaction, n: i64) -> Result<Option<HookRow>> {
    Ok(tx.query_row(&format!("{HOOK_SELECT} WHERE n = ?1"), [n], row_hook).optional()?)
}

fn hook_payload(h: &HookRow) -> serde_json::Value {
    json!({
        "member": h.member_id,
        "hook": h.id(),
        "doc": h.doc_id,
        "kinds": h.kinds,
        "once": h.once,
        "host": h.host(),
        "url_fp": h.url_fingerprint(),
    })
}

impl Store {
    /// Add a hook for `h.member_id` (`by` is that Member). Its cursor starts at the head, so it
    /// carries only what happens from now on. Refusals: an unknown kind, a bad expiry, more than
    /// [`MAX_HOOKS_PER_MEMBER`] live hooks (`TooManyHooks`). `hook.added`.
    pub fn add_hook(&self, h: NewHook) -> Result<HookRow> {
        let kinds = parse_kinds(&h.kinds.join(",")).map_err(|reason| StoreError::Invalid { what: "kinds", reason })?;
        let life = h.expires_in.unwrap_or(DEFAULT_EXPIRY_SECS);
        if !(60..=MAX_EXPIRY_SECS).contains(&life) {
            return Err(StoreError::Invalid {
                what: "expires",
                reason: "a hook lives from 1 minute to 30 days".to_string(),
            });
        }
        let now = jiff::Timestamp::now();
        let expires_at = time::format(now + jiff::SignedDuration::from_secs(life));
        let created_at = time::format(now);
        self.tx(|tx| {
            let live: i64 = tx.query_row(
                "SELECT COUNT(*) FROM hooks WHERE member_id = ?1 AND disabled_at IS NULL
                   AND (expires_at IS NULL OR expires_at > ?2)",
                params![h.member_id, created_at],
                |r| r.get(0),
            )?;
            if live as usize >= MAX_HOOKS_PER_MEMBER {
                return Err(StoreError::TooManyHooks(MAX_HOOKS_PER_MEMBER));
            }
            let head: i64 = tx.query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |r| r.get(0))?;
            tx.execute(
                "INSERT INTO hooks(member_id, token_id, doc_id, kinds, once, url, secret, cursor_seq, expires_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    h.member_id,
                    h.token_id,
                    h.doc_id,
                    kinds.join(","),
                    h.once as i64,
                    h.url,
                    ids::random_base62(SECRET_LEN),
                    head,
                    expires_at,
                    created_at
                ],
            )?;
            let row = load_hook(tx, tx.last_insert_rowid())?.ok_or_else(|| StoreError::HookNotFound(String::new()))?;
            append_event(tx, Some(&h.member_id), "hook.added", None, hook_payload(&row))?;
            Ok(row)
        })
    }

    pub fn hook(&self, n: i64) -> Result<Option<HookRow>> {
        Ok(self.conn().query_row(&format!("{HOOK_SELECT} WHERE n = ?1"), [n], row_hook).optional()?)
    }

    /// Every hook (`member` None, the owner's list) or one Member's, oldest first.
    pub fn hooks(&self, member: Option<&str>) -> Result<Vec<HookRow>> {
        let conn = self.conn();
        let rows = match member {
            Some(m) => {
                let mut st = conn.prepare(&format!("{HOOK_SELECT} WHERE member_id = ?1 ORDER BY n"))?;
                st.query_map([m], row_hook)?.collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => {
                let mut st = conn.prepare(&format!("{HOOK_SELECT} ORDER BY n"))?;
                st.query_map([], row_hook)?.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        Ok(rows)
    }

    /// Remove a hook. `by` None = the server itself (a `--once` hook delivered, or expired);
    /// `reason` is `removed`, `once` or `expired`. `Ok(false)` when there was none.
    /// `hook.removed {…, reason}`.
    pub fn remove_hook(&self, by: Option<&str>, n: i64, reason: &str) -> Result<bool> {
        self.tx(|tx| {
            let Some(row) = load_hook(tx, n)? else { return Ok(false) };
            tx.execute("DELETE FROM hooks WHERE n = ?1", [n])?;
            let mut payload = hook_payload(&row);
            payload["reason"] = json!(reason);
            append_event(tx, by, "hook.removed", None, payload)?;
            Ok(true)
        })
    }

    /// Stop delivering to a hook (kept, so its Member and the owner see why). `reason` is
    /// `failing` (retries ran out) or `gone` (the receiver said 404 / 410). `hook.disabled`.
    pub fn disable_hook(&self, n: i64, reason: &str, status: &str) -> Result<bool> {
        self.tx(|tx| {
            let Some(row) = load_hook(tx, n)? else { return Ok(false) };
            if row.disabled_at.is_some() {
                return Ok(false);
            }
            tx.execute(
                "UPDATE hooks SET disabled_at = ?1, last_status = ?2 WHERE n = ?3",
                params![time::now(), status, n],
            )?;
            let mut payload = hook_payload(&row);
            payload["reason"] = json!(reason);
            payload["status"] = json!(status);
            append_event(tx, None, "hook.disabled", None, payload)?;
            Ok(true)
        })
    }

    /// Record one delivery attempt (see the module docs: no event, no commit hook).
    pub fn record_hook_attempt(&self, n: i64, a: &HookAttempt) -> Result<()> {
        let now = time::now();
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(c) = a.cursor {
            tx.execute("UPDATE hooks SET cursor_seq = ?1 WHERE n = ?2 AND cursor_seq < ?1", params![c, n])?;
        }
        if a.ok {
            tx.execute(
                "UPDATE hooks SET last_status = ?1, last_attempt_at = ?2, failures = 0, failing_since = NULL,
                   next_attempt_at = NULL WHERE n = ?3",
                params![a.status, now, n],
            )?;
        } else {
            tx.execute(
                "UPDATE hooks SET last_status = ?1, last_attempt_at = ?2, failures = failures + 1,
                   failing_since = COALESCE(failing_since, ?2), next_attempt_at = ?3 WHERE n = ?4",
                params![a.status, now, a.next_attempt_at, n],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Move a hook's cursor past events it does not carry (no event, as above).
    pub fn advance_hook_cursor(&self, n: i64, cursor: i64) -> Result<()> {
        self.conn().execute("UPDATE hooks SET cursor_seq = ?1 WHERE n = ?2 AND cursor_seq < ?1", params![cursor, n])?;
        Ok(())
    }

    /// Remove every hook expired at `now` (`hook.removed`, reason `expired`); returns their rows.
    pub fn expire_hooks(&self, now: &str) -> Result<Vec<i64>> {
        let expired: Vec<i64> = {
            let conn = self.conn();
            let mut st = conn.prepare("SELECT n FROM hooks WHERE expires_at IS NOT NULL AND expires_at <= ?1")?;
            st.query_map([now], |r| r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for n in &expired {
            self.remove_hook(None, *n, "expired")?;
        }
        Ok(expired)
    }

    /// Who a hook delivers as, as they are now: through the Token that added it (revoked,
    /// expired or disabled ⇒ None), else the Member by id (disabled ⇒ None).
    pub fn hook_member(&self, h: &HookRow) -> Result<Option<AuthMember>> {
        if let Some(t) = &h.token_id {
            return self.reauthenticate(&AuthVia::Token { id: t.clone() });
        }
        let found = self
            .conn()
            .query_row(
                "SELECT m.id, m.display_name, ms.handle, ms.role, ms.grants
                 FROM members m JOIN memberships ms ON ms.member_id = m.id
                 WHERE m.id = ?1 AND m.disabled_at IS NULL AND ms.disabled_at IS NULL",
                [&h.member_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((member_id, display_name, handle, role, grants)) = found else { return Ok(None) };
        Ok(Some(AuthMember {
            member_id,
            handle,
            display_name,
            role: Role::parse(&role).unwrap_or(Role::Member),
            grants: serde_json::from_str(&grants)?,
            via: AuthVia::Session { id: String::new() },
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        // RFC 4231 test case 2.
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(to_hex(&mac), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // Test case 6: a key longer than the block is hashed first.
        let key = [0xaau8; 131];
        let mac = hmac_sha256(&key, b"Test Using Larger Than Block-Size Key - Hash Key First");
        assert_eq!(to_hex(&mac), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn ids_hosts_and_kinds() {
        assert_eq!(parse_hook_id("h12"), Some(12));
        assert_eq!(parse_hook_id("7"), Some(7));
        assert_eq!(parse_hook_id("h"), None);
        assert_eq!(parse_hook_id("h0"), None);
        assert_eq!(parse_hook_id("hx1"), None);
        assert_eq!(url_host("https://Relay.Example.com/v1/x?y").as_deref(), Some("relay.example.com"));
        assert_eq!(url_host("https://u:p@example.com:8443/x").as_deref(), Some("example.com"));
        assert_eq!(url_host("http://[::1]:7000/x").as_deref(), Some("::1"));
        assert_eq!(url_host("nope"), None);
        assert_eq!(parse_kinds("mention, action,action").unwrap(), vec!["action", "mention"]);
        assert!(parse_kinds("reply").is_err());
        assert!(parse_kinds(" ").is_err());
        assert_eq!(fingerprint("abc").len(), 10);
        assert!(signature("s", b"{}").starts_with("sha256="));
    }
}
