# Member settings and Devices: store and API (CMD-23 — extend, don't rewrite)

The web Settings screen is [web-settings.md](web-settings.md), the Document page's use of the
settings [web-document-view.md](web-document-view.md), the CLI verbs
[cli-conventions.md](cli-conventions.md), the deploy export `docs/ops-health-check.md` P7.

- **Files:** `crates/store/src/settings.rs` (pure: `CATALOG` / `GROUPS` / `PRESETS` /
  `TEXT_SIZES`, `validate` → `Refusal`, `catalog_json`, the colors `contrast` / `delta_e` /
  `highlight_colors`, `Stamp` + `MAX_AHEAD_MS`, Device ids `is_device_id` / `new_device_id` /
  `WEB_DEVICE`), `crates/store/src/devices.rs` (the `Store` methods below, the server clock),
  migration v5 in `migrations.rs`, `server/me_settings.rs` (the routes; its module docs have the
  table and the device headers), `server/prefs.rs` (`WebPrefs`: what a page applies),
  `auth.rs::check_device`, the `me=1` rule in `server/events.rs` + `EventQuery.member_events`.
- **Schema v5:** `devices (member_id, id)` (`name`, `kind` iphone|ipad|web|cli, `created_at`,
  `last_seen_at`, `forgotten_at`); `tokens.device_id` / `sessions.device_id` (the Device last
  used from); `member_settings`, `all_device_settings` (`value` JSON, `stamp`), `device_settings`
  (`value` NULL = tombstone). The migration gives every Member with a session the Web Device.
  The previous binary can't open a v5 store (`SchemaTooNew`): the deploy exports first (P7).
- **Devices:** `ensure_device(member, id, Some((kind, name)))` is the only maker (an upsert in one
  `tx`; `device.added`; cap `MAX_DEVICES` = 20 not forgotten, Web included → `TooManyDevices`;
  a forgotten id → `DeviceForgotten`; a malformed one → `BadDevice`); only the settings routes
  and `WebPrefs::load` / `open_session` (Web) call it. `touch_device(member, id, token_id)`
  (every Token request with `X-Annotary-Device`, from `auth::check_device`) and
  `authenticate_session` (Web) stamp `last_seen_at` at most once a minute (`SEEN_EVERY_SECS`)
  and the Token's `device_id`, in a hand-made transaction that never fires the commit hook.
  `rename_device` / `forget_device` refuse Web (`WebDevice` → 409 `web_device`); forget revokes
  the Tokens with that `device_id` and deletes its sessions, sets `forgotten_at`, keeps the row
  and values (`device.forgotten {member, device, tokens, sessions}`).
- **Reading:** `resolved_settings(member, device)` / `resolved_setting` → `Resolved {def, value,
  source}` (`device` > `all` > `default` for device settings; `user` / `default` for user ones;
  `display_name` comes from `members`; a Default Vault id that no longer exists reads as the
  default). `settings_matrix(member)` → the live Devices + one `MatrixEntry` per device setting
  (`all`, each Device's `own` row, `resolved`, `mixed` = more than one value among Devices the
  setting `applies_to`). `device_sync(member, device)` → `SyncRow`s for iOS.
- **Writing:** `put_setting(member, PutSetting {key, value, scope, stamp})` → `PutOutcome
  {changed, stamp}`; `reset_setting` (user: row deleted; all: the default written as All;
  device: a tombstone); `copy_device_settings(member, from, to | None)` (forced, re-stamped
  writes of the settings `from`'s kind has; an iPhone's copy leaves the keyboard settings alone). All of them go through `write_one` inside `Store::tx`; a change appends
  `setting.changed {member, key, scope, device?, stamp}` (one per Device at scope `devices`),
  never the value; no change, no event. Scope `all` tombstones every live Device's own value
  older than the new stamp. `display_name` writes go through `rename_member`.
- **Stamps:** `MS-COUNTER-WRITER` (`Stamp`, ordered by all three). The server's writer is
  `s<tenant id>`; its clock is the `settings` row `hlc` (`hlc_next` / `hlc_observe`): every
  accepted client stamp moves it, so the next server stamp is later than anything stored. A
  client stamp more than five minutes ahead of the wall clock → `ClockSkew` (409 `clock_skew`,
  `server_time_ms`); not newer than the stored one → `Stale` (409 `stale`, `stored {value,
  stamp}`) unless the value is the same (a no-op). A stamped `all` push older than the All row never
  rewrites it (it only tombstones own values older than the push). User settings take no client
  stamp (400 `invalid_setting`): a reset deletes the row, so there is nothing an older push
  could be checked against, and only device settings sync across servers.
- **Errors** (`server/error.rs`): 400 `unknown_setting` / `invalid_setting` / `low_contrast`
  (+`ratio`) / `bad_device` → exit 1; 404 no Device → 2; 409 `device_forgotten` /
  `too_many_devices` / `web_device` / `clock_skew` / `stale` → 5.
- **Live feed:** `setting.changed` and `device.*` (`events::is_member_event`) are readable only
  by `payload.member`, owners included, and only on a feed with `me=1`; `me=1` adds them beside
  `doc=` / `kinds=` (SQL: `EventQuery.member_events`).
- **Decisions to keep:** a Device id is a label keyed per Member, not a credential (any of the
  Member's credentials may name any of their Devices); the browser never stamps; Web is one
  Device for every browser and has no cookie of its own; the events log never holds a value;
  the catalog is the one list (a new setting = an entry in `CATALOG` + client behavior); light
  Purple and Blue are the presets nearest refined (ΔE 9.6 / 10.6), so `MIN_REFINED_DELTA_E` is 9
  and refined keeps its dotted underline.
- **Tests:** `tests/settings_store.rs`, `tests/settings_api.rs`, the CMD-23 rows in
  `tests/access_api.rs`; spec ids S29-01…S29-15.

## Avatars

A Member's picture beside their name. The page side is in [web-document-view.md](web-document-view.md) and
[web-settings.md](web-settings.md), the CLI in [cli-conventions.md](cli-conventions.md). Why it is built this way:
`docs/decisions.md`, 2026-11-08.

- **Files:** `crates/store/src/avatar.rs` (pure: `check_image`, `check_static`, `check_svg`, the caps and allowlists;
  and `Store::set_avatar` / `clear_avatar` / `avatar_type`), migration v6, `server/avatars.rs` (the routes; its module
  docs have the table), `ui.rs::MemberBrief::avatar_html` (the one renderer), `cli/avatar.rs`.
- **Schema v6:** `member_avatars (member_id PK, hash, type, size, static_hash, static_type, static_size, set_at, set_by)`;
  clearing deletes the row and the events log keeps the history. `MEMBER_SELECT` left-joins it; `row_avatar` reads the
  `av_*` columns by name, so a query that selects none of them gives `avatar: None`.
- **What may be stored:** the type is always read from the bytes. An image is a PNG, JPEG, GIF or WebP up to 256 KiB with
  dimensions its header gives (at most 4096 a side), or an SVG up to 64 KiB. A static fallback is a PNG, JPEG or WebP up
  to 256 KiB. Refusals: `invalid_avatar` (400, the reason) and `too_large` (413).
- **The SVG check refuses and never rewrites**, so what is served is what was sent. It reads XML events (quick-xml: no
  DTD, no entity expansion). Allowed: the elements `SVG_ELEMENTS`; the attributes `SVG_ATTRIBUTES` on any of them; `href` /
  `xlink:href` only to `#id`; `url()` only to `#id`; CSS at-rules `@keyframes` / `@media`; the functions `CSS_FUNCTIONS`.
  CSS comments are removed before the checks and backslash escapes are refused, so neither can hide a word; character
  references in a `<style>` are resolved before the CSS is read. Also refused: a DOCTYPE, an unknown entity, a processing
  instruction, CDATA outside `<style>`, text outside `<style>` / `<title>`, a nested `<svg>`, a root without
  `xmlns="http://www.w3.org/2000/svg"`, XML other than 1.0 in UTF-8, more than 32 levels or 4000 elements. To allow
  something new, add it to a list in `avatar.rs` with a malicious-sample test beside it.
- **Writes:** `set_avatar` checks both images, then writes the blobs and the row in one `Store::tx` (the GC takes the
  same connection, so it cannot sweep a blob a cleared avatar left behind in between; that is why it calls `blobs.put`,
  not `put_blob`, which would deadlock). The same images again ⇒ `changed: false`, no event. Events are
  `member.avatar_set {member, handle, hash, type, static_hash, static_type}` and `member.avatar_cleared {member, handle}`,
  Tenant-level, so the feed shows them to owners only.
- **Blobs:** `docs::referenced_hashes` includes every current avatar, so the GC and `export` keep them. `export` reads
  stores older than v6 without migrating them: `avatar_blob_refs` treats a missing table as no avatars.
- **Serving:** `GET /avatars/{hash}` is in the `web_pages` router, so a browser's cookie and loopback trust apply. It
  serves only a hash in `member_avatars`, so it cannot be used to read any other blob.
- **Tests:** `avatar.rs` unit tests (S41-01, S41-02), `tests/avatars_api.rs` (S41-03…05, S41-07), the avatar rows in
  `tests/access_api.rs`, `tests/e2e/88-avatars/` (S41-06), `tests/web/avatars.spec.ts` (S41-08, S41-10), the avatar test in
  `tests/web/embed.spec.ts` (S41-09). Fixtures: `fixtures/avatars/` (`make.py` draws the PNGs).
