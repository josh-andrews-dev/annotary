# Hit every surface

The most common defect in Annotary is a change that works on the path you tested and is missing
everywhere else. Annotary is read and written through several surfaces, and only some of them share
code with this repo. Before you call a feature or a fix done, walk this list, change what applies,
and say in the pull request's `## Surfaces` section what you did for each surface (the format is
below; the `surfaces` check reads it).

## The list

- **The web page.** Wide (margin cards, from 850 px) and narrow (inline rows, a phone) layouts; the
  light and the dark theme; the head and an older Version (the time-slice: `/d/{id}/v/{n}`); a
  deleted Document (the list's `deleted=1`, restore); the Compare page when the change touches the
  Document's HTML.
- **Embed mode** (`docs/dev/web-document-view.md`, "Embed mode"). Every request goes through
  `transport` in `app.js`, never `fetch`, `EventSource` or `location` directly; an answer over
  384 KiB (`PART_MAX`) arrives in parts and must still work; the frame has no cookies and no
  storage, so a preference arrives as a header. Test it in the fake embedder
  (`tests/web/embed.spec.ts`).
- **The API.** The iPhone and iPad app cannot read a page, so whatever a page shows must be in
  `/api/v1`: a new field on the page is a new field in the API, with a row in
  `tests/access_api.rs`'s `endpoints()` for a new route.
- **The iPhone and iPad app.** It builds its screens natively from the API: the Document list,
  Thread cards, composers, keyboard keys, mapping a text selection to source lines, the margin
  layout, relative times, error sentences, optimistic updates, settings and the owner's admin
  screens. Each of these is a copy of logic here, so a change to one here is a change there.
- **The Hermes page (annotary-hermes).** It shows the list and the Document view in embed mode,
  but it pins an annotary release and has its own pieces: its `annotary_*` tools, its own skill
  file, the chat card with its Reply, Resolve and Action buttons, and layout constants that must
  match this repo's. A change to a page or the CLI reaches it only through an annotary release
  (`docs/dev/releases.md`) and a new pin there.
- **CLI output and the agent skill.** `crates/annotary/skill/SKILL.md`, the goldens (`make
  goldens`) and `tests/token-report.md` (`make token-report`) change together.
- **Reverse states.** Whatever can be done can be undone, on every surface: resolve and reopen,
  delete and restore, disable and enable a Member, set and clear an avatar.
- **The pull request checks' copies.** annotary carries the originals of
  `.github/scripts/size-report.cjs` (with `size-report.test.cjs`, run by `size.yml` and
  `size-comment.yml`) and of the `surfaces` check (`.github/workflows/surfaces.yml`, with
  `.github/scripts/surfaces.test.cjs`). annotary-hermes carries copies, and so does the iPhone and
  iPad app's repo; each repo's checks run only its own copy, so a fix here never reaches them by
  itself. A fix to the script or the workflow here is ported to both copies in the same run, as the
  rule below says (`perf/README.md`, "The size report").
- **Docs.** `docs/dev/` for the area you changed, `docs/decisions.md` for a choice, the guide
  (`docs/annotary-guide.html`) for what a person sees.

## The rule

The maintainer's run that changes one surface changes the others before it reports done: this
repo's pull request first, then the app's and the plugin's in their own repos, with an annotary
release and a new pin when the plugin needs a page or CLI change. A surface may say `follows:`
only in three cases:

1. it needs a design answer from the maintainer;
2. the pull request is an outside contributor's, who cannot reach the other surfaces;
3. it is an urgent fix to a live server, which ships first and is followed the same day.

## The `## Surfaces` section

A pull request titled `feat`, `fix` or `perf` needs it; other titles skip the check. One line
per surface, each `changed`, `not affected:` with why, or `follows:` with one of the three
reasons (its words must say `design`, `contributor` or `urgent`):

```markdown
## Surfaces

- Web: changed
- Hermes page: follows: urgent fix to a live server; the plugin's pin moves today
- App: not affected: the app does not show the list's filters
- CLI: not affected: no verb prints it
- Agents: changed (the skill names the new flag)
```

"Agents" is the skill and what an agent reads through the CLI; "CLI" is what a person sees in a
terminal. Editing the body runs the check again (`.github/workflows/surfaces.yml`).
