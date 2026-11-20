# Markup (extend, don't rewrite)

A Markup is a drawing a Member makes over the rendered Document in the app, kept on one Comment
(glossary: **Markup**). This repo stores and serves it, shows it on every Thread card and lets agents
read it through the CLI; the app draws it, and nothing here draws. Why each choice:
`docs/decisions.md`, 2026-11-19.

- **Files:** `crates/store/src/markup.rs` (pure: `check_png`, `check_strokes`, the caps
  `IMAGE_MAX` 2 MiB, `STROKES_MAX` 512 KiB, `MAX_SIDE` 4096, `markup_id` / `parse_markup_id`; and
  `Store::upload_markup` / `attach_markup` / `markup_blob_type`, `markup_blob_refs` for the GC),
  migration v7 in `migrations.rs`, `server/markups.rs` (the routes; its module docs have the table),
  `view.rs::markup_html` (the card), `cli/thread.rs` (`[markup]`, `--markup`), `fixtures/markup/`
  (`make.py` draws `cross-out.png` and writes its strokes).
- **Schema v7:** `comment_markups (doc_id, thread_num, comment_num)` → `comments`, with
  `image_hash`, `image_size`, `width`, `height`, `strokes_hash`, `strokes_size`. At most one per
  Comment. `threads.rs`'s `COMMENT_SELECT` left-joins it, so every Comment read carries
  `CommentView.markup` with no extra statement (the size report counts statements).
- **Upload, then attach.** `POST /api/v1/docs/{id}/markups {image, strokes}` (the `comment` Right;
  `image` base64, `strokes` the JSON object; body limit `markups::BODY_MAX` 4 MiB) checks both files
  and stores them as blobs with **no row and no event**, then answers `{id, image, width, height,
  strokes, image_size, strokes_size}` with `id` = `<image hash>.<strokes hash>`. `POST …/threads` and
  `…/replies` take `markup: ID`; the Comment's transaction reads both blobs, checks them again
  (whoever stored them: `PUT /api/v1/blobs` could have), writes the row, and its
  `thread.created` / `thread.replied` carries `markup: true`. A re-upload of bytes already stored
  touches the blob (`BlobStore::touch`) so the GC's grace period starts again.
- **The checks.** The PNG is decoded whole with the `png` crate (every row, then `finish` to
  `IEND`): signature, size, dimensions (both sides 1 to 4096), damage. The strokes are
  `{"strokes": [{color, width, tool?, points}]}` and nothing else (`check_strokes` has the exact
  rules); `x` / `y` are the PNG's pixels from its top left, `t` seconds since the Markup's first
  point. They are stored as the server wrote the JSON (compact), not the client's bytes.
- **Body.** A Comment with a Markup may have an empty body (`validate_body(body, has_markup)`); an
  edit may empty it too. Every other Comment still needs words.
- **Delete, purge, GC, export.** Tombstoning a Comment deletes its row (the blobs go to the GC).
  `docs::referenced_hashes` includes every row's two hashes, so the GC and `export` keep them;
  `purge_document` adds them to its candidates and deletes the rows before the Comments. `export`
  reads stores older than v7 without migrating them: a missing table is no Markups.
- **Serving.** `GET /api/v1/docs/{id}/markups/{hash}` (the `read` Right) serves only a hash a row of
  that Document names: `image/png` or `application/json`, `nosniff`, `private, max-age=31536000,
  immutable`, `ETag` of the hash. An upload nobody attached is not served.
- **JSON.** `comment_json` adds `markup {image, width, height, strokes}` (the two hashes) only when
  there is one, so Comments without one are unchanged. The Sidecar's Thread and Reply carry the same
  as `markup` (`MarkupEntry`), after `body`.
- **The card** (`view.rs::markup_html`, the one renderer): `a.cd-mk[target=_blank]` (opens the image
  full size; an embedder opens it outside its frame) around `img.cd-mk-img` with the image's `width`
  / `height`, between `.cd-who` and `.cd-cbody`. It keeps a white ground in the dark theme: the image
  is a crop of the light page. A root that is only a Markup shows "Markup" in the narrow row's
  snippet. An embed render writes the URL as `data-cd-mk` (no `src`), which `loadAvatars` in
  `app.js` fetches through the transport as a blob; above `embed::IMAGE_MAX` it writes only the link.
  The web has no drawing UI.
- **CLI.** `show` / `thread list` / `mentions` rows start the body with `[markup]` when the root or
  a Reply holds one (`thread::has_markup`), and their `--json` has `markup: true`. `thread show`
  prints `[markup] W×H` above each such Comment's body (an empty body is left out), and `--markup
  DIR` writes `DIR/t7.png` / `DIR/t7.1.png` (DIR made, the bytes checked against the hash), puts the
  path on that line and adds `path` to the JSON's `markup`. `thread new` / `thread reply` take
  `--markup PNG --strokes JSON` (uploaded first; `-m` then optional; an image over 2 MiB is refused
  before sending, exit 5). The skill names only `thread show --markup DIR`.
- **Errors** (`server/error.rs`): 400 `invalid_markup` → exit 1; 413 `too_large` → 5; 409
  `markup_missing` (an id whose blobs are gone) → 5; 409 `deleted` for a deleted Document.
- **Tests:** `markup.rs` unit tests (S49-01, S49-02), `tests/markup_api.rs` (S49-03…06), the
  Markup rows in `tests/access_api.rs`, `tests/cli_threads.rs` and `tests/e2e/89-markup/` (S49-09),
  `tests/web/markup.spec.ts` (S49-07), the Markup test in `tests/web/embed.spec.ts` (S49-08),
  `tests/cli_skill.rs` (S49-10); the live scenario `verify/scenarios/markup.spec.ts`.
