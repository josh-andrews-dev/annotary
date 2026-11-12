# Decisions

Smaller dated decisions: context, choice, the alternative rejected and why. Architecture decisions
are ADRs in [`adr/`](adr/). Entries use today's names; the project had another name until
2026-11-01. `CMD-n` is the project's old tracker ID: the items are not public, and the label stays
so that code comments and tests still point at the entry that explains them.

## 2026-10-21: `export` verifies its blobs instead of locking out GC (CMD-9)

An export racing a purge or GC could leave out a blob its database snapshot references. Chose to
check, after copying, that every blob the snapshot references is in the archive, and to fail the
attempt otherwise (the CLI retries on a fresh snapshot, up to 3 times, then exits 5 and leaves no
file); rejected a cross-process lock between `export` and the server's purge/GC because `export`
must work with no server and take no LOCK, and a lock file shared by two processes adds a stale
lock to recover from; rejected a grace period on blob deletion because purge removes its blobs at
once by design and a grace only narrows the window an export longer than it still loses.

## 2026-10-21: login throttle and cross-site refusal (CMD-4)

Chose an in-memory failed-login throttle per client (peer IP, IPv6 by /64) and account-wide for
non-loopback peers, shared by the web form and `POST /api/v1/login`; rejected a store table
because a restart clearing the counters is harmless and nothing else needs them. Loopback is never
locked by the account limit, so a remote guesser cannot lock the owner out of their own machine. Chose
`Origin` / `Sec-Fetch-Site` checks on every cookie or form mutation (bearer requests exempt) plus
the existing `SameSite=Lax`; rejected a CSRF token in the form because the pages are stateless and
every current browser sends `Origin` on a cross-origin POST, which the check refuses.
Rejected checking `/login` alone because `SameSite=Lax` still sends the cookie on a same-site,
cross-origin request (another port on the same host).

## 2026-10-21: the binary's version is stamped at build time (CMD-2)

Chose a build-time `ANNOTARY_VERSION` read by `option_env!`, with `0.0.0` in `Cargo.toml` and the
committed skill; rejected a `build.rs` running `git describe` because a dev build would then report
a stale or dirty tag and every crate rebuild would shell out to git. Dev and CI builds report
`0.0.0`; the goldens pin it.

## 2026-10-21: the server writes its own consistent snapshots for backups (CMD-1)

A backup that copies a data dir file by file can catch the live WAL database mid-write. Chose to
have the server write an `export_tar` of the store into `~/.annotary/snapshots/` at start and every
15 minutes (skipped when the event `seq` is unchanged, newest four kept); rejected a pre-backup hook
because the backup job is not this product's, and rejected a separate scheduled job because the
server already owns the store and a ticker beside housekeeping's needs no service manager. Chose
full tars (database plus blobs) over a database-only copy because a database copy paired with blob
files read at another moment could miss a blob a purge removed in between.

## 2026-10-21: no default profile; every CLI call names who it is

An agent replied on a Document as the owner: `config.toml` had a default owner profile next to an
`agent` profile, and the skill wrongly said `ANNOTARY_TOKEN` was already in the agent's environment,
so the CLI silently fell back to the owner. Chose to remove the default profile (`profile
default`, `login --default/--no-default`, `profile add --default`), so a call without `-p`,
`ANNOTARY_TOKEN` or `ANNOTARY_PROFILE` is exit 3 listing the profiles, and to make the `-p`
flag beat the environment so an agent naming its own profile is never overridden by a token or
profile the human exported in a shell the agent also loads; the skill tells agents to pass `-p`
on every command, find their profile with `profile list`, ask the human to create one when there
is none, and use the human's profile only when asked to post for them. Rejected detecting agent
harnesses from their environment variables (`CLAUDECODE`, `AI_AGENT`, `CODEX_*`) because the
list is incomplete and inherited by a human's shell started from a harness, and rejected a
person/agent kind on Members because neither is a security boundary on one machine (an agent can
read `config.toml`) and the goal is preventing mistakes. People typing commands pay one `-p`.

## 2026-10-23: the no-password login page names the loopback URL as text (CMD-14)

A browser on another device that opens `/login` while no password is set cannot be signed in, so
the page says how to get in. Chose to name the server's own loopback URL (`st.url`) in `<code>`;
rejected a link because on the remote device it would open that device's own loopback, and
rejected hard-coding `127.0.0.1:7377` because the port and bind vary. Chose 200 for the page;
rejected 401/403 because S18-02 defines `/login` as never 401 and the page is guidance, not an error.

## 2026-10-29: a request a local reverse proxy forwards is not loopback (CMD-17)

A reverse proxy on the same machine (Tailscale Serve, say) forwards every request from `127.0.0.1`,
so loopback trust would sign any of its clients in as the owner while no password is set, and the
login throttle would see one loopback client. Chose a header rule in `login_guard::client_ip`: a
loopback peer whose request carries `X-Forwarded-For`, `Forwarded`, `X-Real-IP` or
`Tailscale-User-Login` is remote, keyed by the last `X-Forwarded-For` entry (else `0.0.0.0`). It can
only take trust away: a browser on the machine sends none of them, and a remote peer's headers are
never read. Rejected a second listener for the proxy, because it changes `serve`'s flags for the
same result, and rejected a `--behind-proxy` flag that turns loopback trust off, because the owner's
first password needs a loopback browser (an SSH tunnel), which the header rule keeps.

## 2026-10-29: the skill's link rule and review flow

The skill says which link to give (only what `annotary link` prints; never loopback, `.local`, a LAN
address or a hand-built URL) and how a document written for the human is reviewed: the repo file is
the source, pushed as a copy, the approved Version committed in the PR.

- Chose to push a copy from `~/.cache/annotary/review/`; rejected pushing the repo file with
  `*.annotary` git-ignored, because every repo would need a `.gitignore` change and the Name
  would be the bare stem, colliding across repos (every repo has a `plan.md`).
- Chose the Name `<repo>:<path>` with `/` as `:` (`my-repo:docs:plans:x`); rejected allowing
  `/` in Names, because `pull` writes `<Name>.md` as one file and a Name is a file stem.
- Chose to fit it in the 60-line, ±20%-token skill by folding the exit codes and the keep-current
  hook into the commands section and shortening the profile rule; rejected raising the token
  band, which is the token report's contract with the prototype.

## 2026-10-29: the CLI speaks https with rustls and the bundled roots (CMD-18)

A server behind an https proxy answers only on https, and ureq was built with no TLS provider, so
every https server URL failed with "TLS required, but transport is unsecured". Chose ureq's `rustls`
feature: rustls with the ring provider and the bundled Mozilla roots (`webpki-roots`), pure Rust and
the same on macOS and Linux. Rejected `platform-verifier` (the OS trust store), because it adds
per-OS code and the servers the CLI meets have public certificates; rejected `native-tls`, because
it links OpenSSL on Linux. The roots are fixed at build time, so a release rebuild refreshes them. A
TLS failure now says "TLS with URL failed: …" (exit 6, kind `tls`) instead of "not reachable".

## 2026-10-29: Vault verbs, and profiles and Sidecars that follow a renamed Vault (CMD-5)

The CLI gains `vault list | create | rename | delete` over the existing `/api/v1/vaults` routes;
`vault rename` / `delete` also fix this machine's profiles on that server. A Vault renamed
elsewhere (the web Settings screen, another machine) is followed without anyone editing
`config.toml` or a Sidecar.

- Chose to have the CLI send the profile's Vault on every request (`X-Annotary-Profile-Vault`,
  plus `X-Annotary-Profile-Vault-Id` once known; names percent-encoded because header values
  are ASCII) and the Vault-scoped routes resolve it and answer `X-Annotary-Resolved-Vault` /
  `-Id` (or `X-Annotary-Deleted-Vault`) in the same round trip; the CLI then rewrites the
  profile. Rejected a separate "where is my Vault" request before each command (one more round
  trip on every call), and rejected the server writing `config.toml` (it never does, AGENTS.md).
- Chose to keep sending the old `vault=` query / body field beside the headers, the profile's
  rule applying when it equals the header; rejected sending only the headers, because an older
  server would then silently list every Vault instead of the profile's.
- Chose to record the Vault's id beside the name (`vault_id` in the profile, written only by the
  CLI) and to resolve with the events log (`vault.created` / `vault.renamed` / `vault.deleted`):
  an id that ever carried the profile's name follows that Vault through any chain of renames,
  even after a new Vault takes the old name; an id that never carried it means a human edited
  `vault`, and the name wins. Rejected resolving by name alone (a reused name would misroute the
  profile to a different Vault) and by id alone (a hand edit would be undone). No migration: the
  events log already holds the lineage, and the lookup only runs when the fast path (id and name
  agree, or the name is live) misses.
- Chose, for a deleted profile Vault, to clear it from the profile and fall back to every
  readable Vault for `list` / `resolve` / `mentions`, while `POST /api/v1/docs` refuses
  (`vault_deleted`) and `push` picks the Vault again as if the profile named none; rejected
  creating the new Document in some other Vault on the server's own initiative.
- Chose to echo a resolved Vault only when the caller holds the route's right on it (else 403
  naming the profile's name), and to put `Vault OLD was renamed to NEW` in an explicit
  `--vault OLD`'s 404 only for a caller who may read NEW; rejected echoing regardless, which would
  tell a Member a Vault's new name they cannot read.
- Chose to treat a git-conflicted Sidecar whose sides share the Document `id` but differ in
  `vault` as one Document across a rename (the newer side's name, the lower side's base);
  rejected keeping ticket 17's "conflicting Vaults — keep one side" (S17-07), because a Document
  id is unique in the Tenant and a Document never changes Vault, so a human had nothing to decide.
  Sidecars are never rewritten by scanning directories or on read-only verbs; push and pull
  binding by id and rewriting the file is enough.
- Chose to leave `crates/annotary/skill/SKILL.md` alone: Vault lifecycle is owner setup, not
  the agent's review loop (and the skill is at its token ceiling). The top-level `--help` gains
  one line (757 → 771 tokens, within the token report's ±20% of the prototype).

## 2026-10-29: the `daemon` verb stays; plain links in the skill

Chose to keep the `daemon` verb and the CLI's `127.0.0.1:7377` default: a local server on a Mac is a
product feature, whether or not any given install uses it. Chose, in the skill, "a plain Markdown
link (in code it can't be clicked)"; paid for by cutting asides (the `-p` precedence note, the
`*.annotary` token cost, `show`'s gloss, which the commands list keeps, the `wait` timeout default,
which the exit-code line keeps, and "one stderr line").

## 2026-10-30: the CLI's Device id is one top-level `device` line in `config.toml` (CMD-23)

Chose a top-level `device = "…"` (`Config::device`): one id for the install, used for every
profile and server, as iOS keeps one Keychain id across servers; Device rows are keyed by Member
and id, so one id never touches another Member's row. Rejected a `device` per profile because two
profiles on one machine are the same place, and a per-profile id would split one machine into
several Devices. Nothing writes it yet (no starter setting applies to kind `cli`); every rewrite
of `config.toml` keeps it. The `settings` / `devices` verbs send no `X-Annotary-Device`, so
the CLI makes no Device.

## 2026-10-30: `password` is a real data-dir verb (CMD-22)

Chose a shipped `annotary password --clear | --set` on a stopped data dir: it is the owner's way back
in after a forgotten password. Rejected a test-only binary because it would duplicate the store
wiring for one caller, and anyone who can run the verb on a data dir can already read its files, so
it opens nothing. Chose to refuse while a server holds the `LOCK` (it never changes a running store)
and to read a new password only from the prompt, never an argument, where shell history would keep it.

## 2026-10-31: tests that count events lock their server; loopback sign-in is unchanged (CMD-19, CMD-24)

A port watcher (an IDE's preview, say) sends `GET /` to a freshly bound loopback port, which on a
passwordless server signs a browser in as the owner, appends `session.started` and makes a Web
Device, so tests counting events or Devices across a wait flaked. Chose a password on those test
servers (`TestServer::start_locked`, `ANNOTARY_SETUP_PASSWORD` for a spawned `serve`); rejected
making `loopback_sign_in` skip the event or the Device, because loopback trust is the designed
behavior and that sign-in is a real session worth recording; rejected locking every `TestServer`
by default, because about 40 test files and the web tests rely on loopback trust.

## 2026-10-31: spawned test servers stop through a guard in the tests, not a watchdog in `serve` (CMD-28)

Chose `ServeGuard` in `crates/annotary/tests/common/serve.rs`; rejected `annotary::testing`,
because it is compiled into the shipped library. Rejected an `ANNOTARY_EXIT_WITH_PARENT` watchdog
in `serve`, because it adds test-only code to the server and helps only when the test binary itself
is SIGKILLed, and Ctrl-C already reaches the whole process group. If leaked servers still show up,
file it.

## 2026-11-01: renamed to Annotary

The product's first name was taken: another project already had that name and idea. The product,
binary, crates, data dir (`~/.annotary`), env vars (`ANNOTARY_*`), URL scheme (`annotary://`),
Sidecar suffix (`.annotary`), skill, database file (`annotary.sqlite`), snapshot names
(`annotary-*.tar`), Token prefix (`ant_`), LaunchAgent label (`com.annotary.server`) and request
headers (`X-Annotary-*`) all took the new name. Chose no compatibility code for old names; rejected
aliases, because nobody outside the project had installed it and a clean setup is simpler. The `cd-`
prefix of the web pages' ids and classes and the `cd_session` cookie are internal and keep their
names.

## 2026-11-01: the brand renderer and the icon files

Chose a standard-library renderer for `brand/` (`scripts/brand/render.py`: a rounded-box rasterizer
and a PNG writer); rejected a pinned SVG library (resvg, cairosvg) because the freshness test runs
on Linux and macOS and pixels must match exactly, which IEEE arithmetic gives and a library's
platform-specific antialiasing does not promise. The script refuses any shape it cannot draw, so
a master that outgrows it fails loudly. The check compares PNG scanlines rather than compressed
bytes, because zlib differs by Python build.

Chose the small master for `icon.svg`; rejected the full master because a browser draws `icon.svg`
in the tab at 16 px, where the design round found the full master's dot disappears. Chose to serve
the manifest's three PNGs as well as the four routes the design names, because a manifest whose
icons are not public is broken. Chose `display: standalone`, for the installed web app.

## 2026-11-02: the release pipeline and the update notice (M3)

Chose cargo-dist (0.33.1) for the release workflow, tarballs and shell installer; rejected a hand-written
workflow because dist generates what we would otherwise keep up by hand. Three edits to what it generates are
kept (`docs/dev/releases.md`), and each is tested.

Chose to stamp the tag's version into the workspace in each release job; rejected committing the version at
each release, because the shipped version comes from the tag and the repo keeps `0.0.0`, and rejected a tag
that does not match, which dist refuses.

Chose our own job for the Homebrew tap, with a deploy key; rejected dist's `homebrew` job because it needs a
personal access token with `repo` scope and writes no `test do`, so its formula fails `brew audit --strict`.
dist still generates the formula, and our job adds the test, then audits, installs and tests before it pushes.

Chose `.tar.gz`; rejected dist's default `.tar.xz` because a minimal Linux container has no `xz`.

Chose `~/.local/bin` for the shell installer; rejected dist's default of Cargo's folder because the installer
is for people with no Rust.

Chose to ask for the latest release only from `-V` on a terminal and on a thread at `serve` start; rejected a
check on every verb because a CLI an agent calls hundreds of times must not touch the network, and rejected a
check on `-V` with no terminal because scripts call it. A build at `0.0.0` never asks, which keeps the tests
off the network without a setting in each harness. The setting is a line in `config.toml` or an environment
variable; rejected a new verb or flag because nobody needs it twice.

## 2026-11-02: a server's URL lives on the host (M4)

Chose `/etc/annotary/env` for `--url` and `--link-base`, with the unit requiring both; rejected optional
values, because `--url` is stored and the store would quietly keep an old one after the setting was lost.
`none` says "no URL" on purpose. For the same reason the health check reads that file: the health answer
alone stays right after the setting is gone.

Chose to add a missing key to an existing env file and never to change one that is there; rejected "create only
if missing", because an earlier installer created the file empty, and rejected overwriting, because the file is
the host's.


## 2026-11-02: an agent leaves a review alone until it ends

Chose to have the skill's agent wait through a human's review and act when it ends (`send-back`, `approve` or
their word in chat), answering an @mention without pushing, and handling every open Thread in one Version;
rejected acting per Thread, because a push moves head, and a page on the older Version then refuses new comments
while the human is still reading. Rejected a new "review finished" event: the default Actions already say it.

## 2026-11-02: no Code of Conduct, and no personal email address in the repo

Chose to delete `CODE_OF_CONDUCT.md`, listing it in the template's `removed_starter_files`: with one maintainer it
adds nothing to GitHub's own moderation tools, and it published a personal address. Rejected keeping it for the
community profile's checkmark, which is its only remaining use. The template still requires `conduct_email` of a
public repo, so it records the maintainer's GitHub noreply address, which is already on every commit.

## 2026-11-03: repo-standard v0.14.0 no longer asks for `conduct_email`

This corrects the 2026-11-02 entry above: since the upgrade to v0.14.0 the template does not require `conduct_email`, so
`.agents/standard.yml` no longer records it. Deleting the Code of Conduct still stands.

## 2026-11-07: embed mode — the pages in an embedder's sandboxed frame (M3 of the Hermes plan)

The Hermes plugin can show web content only in a sandboxed frame with an opaque origin (no cookies, no storage,
nothing it can load by URL), and on remote surfaces only the plugin's backend reaches this server. So the list and
the Document view render for an embedder and carry every request, navigation and event over `postMessage`
(`server/embed.rs`, `ui/embed.js`; the protocol is in `docs/dev/web-document-view.md`, "Embed mode").

- **A header marks the request, not a query flag.** Chose `X-Annotary-Embed: 1`, which the embedder's backend adds;
  rejected `?embed=1` because a query flag can be put in an `<iframe src>` on any site, and an embed page trusts its
  parent's messages, while a browser cannot add a header to a navigation or a frame (and a cross-site `fetch` with it
  needs a CORS preflight this server never grants). The embed page also only takes the embedder's path when its
  origin is opaque, and carries `Content-Security-Policy: sandbox allow-scripts`.
- **The server serves the shell.** Hermes's plugin validator refuses `<script` in a plugin's `plugin.js`, so the
  bootstrap page is `GET /embed/shell`, which the plugin fetches and turns into a `data:` URL. It writes whatever its
  parent sends into itself, so it carries the sandbox CSP too: loaded from this origin by a browser, it still runs in
  an opaque one. Rejected a JSON route holding the shell because the plugin would build the same page anyway.
- **Pages are written into the frame (`document.write`), not remounted.** The embedder may send `annotary-init` with
  a new path at any time without a new frame, so the frame navigates itself and tells the embedder with
  `annotary-nav`. Each page brings its own copy of the transport, and a generation counter makes the old one's
  listeners and timers stand down. Rejected swapping only the body and re-running `app.js`, because `app.js` binds
  document and window listeners once and is not built to start twice in one document.
- **No message over about 1 MB.** Hermes's gateway stalls on large messages, so an embedded GET whose answer is over
  `embed::PART_MAX` (384 KiB, leaving room for JSON escaping and base64) answers 206 in numbered parts with an etag,
  which the transport joins; parts end on character boundaries. Rejected splitting the Document into lazily loaded
  pages because Threads are spliced server-side for a final first paint, and parts keep that page whole. Images over
  `embed::IMAGE_MAX` (700 KiB), video, audio and PDF are an "open" button (`annotary-open-external`).
- **Live changes are a poll.** The plugin cannot relay events on every Hermes profile yet, so the embedded feed polls
  `GET /api/v1/events` (the same URL and filters as the browser's SSE, from the page's `seq`) every 4 s while the page
  is visible, and an `annotary-event` only makes it look at once. Rejected trusting relayed payloads, because the poll
  already applies the same filters and access checks the server does.
- **Preferences without cookies.** The zone and hour cycle travel as `X-Annotary-Tz` / `X-Annotary-Hc` on every
  embedded request; everything else is already a Member Setting. Owner screens and Settings are not linked (plan D1).

## 2026-11-07: `serve --setup-only --json`, and attested release tarballs

A program that sets up a server (the Hermes plugin) needs the owner's first Token without parsing the transcript.
Chose `--json` on `serve --setup-only`: the transcript moves to stderr and stdout is one object (`created`, `handle`,
`name`, `token`, `vault`, `store`); rejected a `--token-file PATH` as well, because one machine-readable result is
enough and a file of a secret is one more thing to clean up. `serve --json` without `--setup-only` is a usage error
(a running server has no result to print). The release job attests each tarball's build provenance with
`actions/attest-build-provenance`, so a catalog reviewer or an installer can check a download with
`gh attestation verify`; only the job that makes the release gets `id-token` and `attestations` write.

## 2026-11-07: `member enable` brings a disabled Member back with no credential

Chose to add `member enable HANDLE` (owner only, `POST /api/v1/members/{handle}/enable`) as the way back from
`disable`, so a Member that returns, such as a Hermes profile recreated with the same name, keeps its id, handle,
Grants and attribution; rejected adding a new Member, because handles are never reused.

Disable only gates a Member's Tokens and sessions; it does not revoke them. Chose to revoke every Token still live
and end every session in the enable transaction, so the Member comes back with no credential and gets a new Token
from `token mint --member` or pairing. Rejected letting them work again, because a Token given up with a deleted
profile or a departed person would quietly start working again. Rejected revoking them on disable instead, because
that changes what disable has always done and what the Members screen shows for a disabled Member. A local password
is kept: it is the person's own secret, not a credential left lying around.

Chose the same idempotence as disable: enabling an enabled Member exits 0 with a note (`changed: false`) and appends
no event. Enable needs no last-owner guard, since it only adds an active Member.

## 2026-11-08: Compare Versions is an inline diff of the rendered Document

Josh asked for a button that shows what changed between Versions, inline rather than side by side.

- Chose to diff the two Versions' **rendered HTML** (`annotary_diff::rendered_diff`), block by block and then word
  by word inside paired blocks, so headings, lists, tables and code render as on the Document page. Rejected a
  unified diff of the Markdown source, because a reviewer reads the rendered Document and Markdown syntax in a diff
  is noise; rejected rendering a diffed Markdown text, because markers spliced into Markdown change how it parses
  (a list, a table, a code fence).
- Chose to keep the newer block's tags and drop the older block's in a word diff, so the output is always balanced
  HTML; rejected keeping both sides' markup, because removed tags inside kept ones can nest wrongly. A change only
  to markup (a link target, emphasis) is marked as a formatting change instead.
- Chose to diff lists, tables and block quotes one level down (an item, a row, a cell is the unit there) and to pair
  a row's cells by position; rejected a word diff over a whole list or table, because removed text would land where
  a list or table allows no text and a removed cell would shift the columns.
- Chose the oldest-first pair for every URL (`from=7&to=2` shows v2 → v7) and two selects; rejected reverse diffs,
  because the page always reads as the newer text with what went away struck through.
- Chose the Compare link's defaults from Josh's words: on the head, the previous Version against it; on an older
  Version, that Version against the head.
- Chose `del` / `ins` with visually hidden labels (CSS pseudo text on the inline marks, `span.cd-sr` on blocks),
  underline and strike-through, and a +/− glyph on whole blocks; rejected color alone, which fails for color-blind
  readers and screen readers. The colors alias the intent tokens, so both palettes and a later visual refresh
  follow without new tokens.
- Chose to fold unchanged top-level runs into closed `details` (the first block and two blocks of context kept);
  rejected hiding them for good, because the reader sometimes needs the surrounding text.
- Chose a page of its own (`/d/{id}/diff`) with no Thread cards; rejected a mode of the Document page, because the
  card layout keys on the rendered Version's lines, which a diff with removed blocks no longer has.
- Chose a fixed work budget counted in comparisons, with each step charging its worst case before it runs, and blocks shown whole past it. Rejected a wall-clock deadline, because the diff is a pure function and the same two Versions must always give the same page. Rejected caching alone, because the first request would still be unbounded.
- Chose charges measured per byte and per nesting level for all work that can repeat. Rejected charging only the matrix and the diffs, because reading a subtree at every level cost about ten times what the budget counted.
- Chose bags of words after a length check to decide what pairs. Rejected a Myers diff per pair, because a run of long paragraphs costs one diff per pair on every request. A bag ignores word order, which only lets a reordered block pair; its word diff is still charged to the budget.
- Chose a depth limit (32) on the block tree, past which blocks are compared whole. Rejected an iterative tree walk, because no real Document nests that deep and whole-block comparison is correct there.
- Chose a content-keyed cache of finished diffs and two diffs at a time on blocking threads, with a 503 past a 2 s wait. Rejected per-Member rate limits, because the cost belongs to the pair of Versions rather than to who asks.
- Chose one gate before any diff work, with caps low enough that 2 s and 64 MiB hold for every input (20,000 blocks, table cells included, and 4 MiB of rendered HTML per side), and over either the page says the Versions are too large to compare. Rejected comparing part of a Document, because a partial parse produced markup that could not be marked correctly, and rejected higher caps, because the bound would rest on inputs nobody had tried. A Document with tens of thousands of blocks has nothing a reader could review inline anyway.
- Chose to measure each block only against its neighbors (within 32 positions) in a large changed run. Rejected every pair, because many similar sections make that cost more than the rest of the diff, and moved blocks across long distances are rare.
- Chose a count of lines by content (a sort and a merge of hashes) for `+N −M lines`. Rejected a real line diff, because a Histogram diff of repeating lines took 8.7 s on 0.55 MB and no charge makes that a promise; a moved line now counts for neither side, and the change points still show the move.
- Chose a small classic script ahead of the Version selects, listening on `document`, so a change is never dropped however late the page's main script runs. Rejected drawing the selects disabled until `app.js` binds them, because the page's GET form must keep working without the script, and the selects would be dead on exactly the slow devices this is for. The change buttons do need the script, so they are drawn disabled until it has run.

## 2026-11-07: the Apple Silicon build names `macos-15`, not `macos-latest`

GitHub retired `macos-14`, dist 0.33.1's default for `aarch64-apple-darwin`. Chose to name `macos-15` in
`[workspace.metadata.dist.github-custom-runners]`; rejected `macos-latest`, because a moving label changes the build
host without a reviewed change. Rejected regenerating `release.yml` with a newer dist, because that is a larger change
to the release path than one runner name.

## 2026-11-08: the web UI's visual refresh is the mark's margin note

Josh asked for a sleeker, livelier, more striking page inside its minimal look. Chose one new visual idea, the mark's own: the selected card is joined to its highlight by a line in the Member's highlight color, bending down when the margin pushed the card lower. The rest is type, radii, soft shadows, mention tags and a filled positive Action. Rejected the brand's pink as the accent and per-Member name colors, because pink means nothing else in the UI and a colored name is a hue the tokens do not have. Rejected an "AI" badge, because people and agents read alike and the model has no Member kind. Rejected a web font, because it is a request in every page and every embed answer and a swap against the CLS budget of 0. The tie is drawn on the strip rather than the card, because a card deep in the Document has `content-visibility`, whose paint containment clips anything outside its box.

## 2026-11-08: Member avatars

Each Hermes agent's animated face should sit beside its Comments, and people benefit too.

Chose to accept SVG behind an allowlist check; rejected raster-only, because the plugin's faces are 2 to 13 KB of CSS
keyframes and a GIF or animated WebP of them is larger and worse at 28 px. Chose to refuse a file that fails the check;
rejected sanitizing it by rewriting, because a rewriter that keeps a file working must understand everything it passes
on, and refusing tells the author what to change. Wrote the check over quick-xml's events; rejected a DOM parser, which
would have to be trusted to leave DTDs and entities alone, where events make refusing them one match arm. The check is
the second wall: pages show an avatar only through `<img>`, where no script runs, and `/avatars/{hash}` sends a CSP that
blocks scripts and outside loads for someone who opens the URL on its own. CSS functions are an allowlist too, beyond
`url()`; rejected only blocking `url()` and `@import`, because new functions that load things keep arriving
(`image-set()`, `src()`).

Chose to make avatar CSS a language with no comments, strings or escapes; rejected stripping comments and rejected a CSS
tokenizer. The stored bytes are what a browser reads, so a check that reads anything else is bypassable: a stripper that
ignores strings and unquoted `url(` tokens lets `"/*"` hide a live `@import` and `url()` from it (found in review). A
tokenizer would have to match every browser's exactly. Refusing the delimiters leaves nothing to tokenize differently, and
the plugin's faces use none of them. The cost is that an SVG exported from a design tool with a CSS comment or a quoted
font name is refused, with a reason that says what to remove.

Chose one `PUT /api/v1/me/avatar {image, static?}` with base64 in JSON; rejected two raw PUTs, because the avatar and its
still would land in two events with a moment where the animated one has no fallback, and multipart, which needs a new
axum feature and a multipart client in the CLI. The server reads each type from the bytes; rejected a client-supplied
`type`, because `/avatars/{hash}` must never serve a type someone chose. A still must be a PNG, JPEG or WebP; rejected
allowing SVG and GIF there, because what shows under reduced motion must not move.

Chose a route of its own for serving, `/avatars/{hash}`, limited to hashes in `member_avatars`; rejected reusing `/a/{hash}`,
whose types come from a Version's manifest and which needs `read` on a Vault, because an avatar belongs to no Version and
every signed-in Member sees every Member's anyway. Chose an immutable private cache keyed by the hash, so one fetch serves
every card. Nobody sets another Member's avatar; an owner may only clear one, because taking a picture down is moderation
and putting one up in someone's name is not.

A Member without an avatar gets a monogram on one of eight colors picked by a hash of the handle; rejected a color derived
freely from the hash (a hue), because eight fixed colors can each be checked for contrast with white in both themes.

## 2026-11-08: example addresses in tests use the documentation ranges

Chose the reserved ranges (`192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24`) for every example IP in tests; rejected keeping
private-looking addresses and the shared-address range, because `private-check` scans added lines in this public repo and an
example there is indistinguishable from a real one. IPv6 examples stay as they are.

## 2026-11-11: a closing `$` before a digit does not close inline math

pulldown-cmark pairs a `$` with the next one that can close (not after whitespace), so prose with
two dollar amounts (`maybe $1.00–1.25 (… out of a ~$120 shop)`) rendered the text between them as
MathML. Chose Pandoc's rule,
that a closing `$` must not be followed by a digit, applied as a pre-pass in `render()`: it
backslash-escapes both `$` of each offending span at the positions the parser reports and parses
again, so code, HTML and autolinks are never touched and no source line moves. Rejected turning
math off because Documents use `$x^2$` and `$$…$$`; rejected escaping every `$` before a digit
because `$2^n$` at the start of a sentence is math, and because a blind byte rewrite would reach
into code spans and blocks. The re-parse is capped at 8 passes: crafted `{`-nested input can
reveal one new span per pass, and prose needs one or two. `RENDERER_VERSION` and
`MIN_RENDERER_VERSION` went to 1.1.0, so Versions rendered before re-render on read.

## 2026-11-11: sizes on every pull request, timed runs on a Mac, every surface in one run

Speed and the surfaces drifted unseen: the fixture's Document view grew from 1.8 MB to 1.9 MB and the
list from 123 KB to 176 KB with no pull request showing it, and the iPhone and iPad app fell four web
features behind. Chose a committed size report (`perf/size-report.json`, deterministic counts a test
holds equal to what the server sends) with ceilings in `perf/BUDGETS.md`, a `size` check that fails on
any growth unless the body says `Size growth:` why, and a comment that shows main, the pull request,
the change and the ceiling; rejected report-only, where growth of 1% at a time passes unnoticed, and
rejected a label only the maintainer can add, which would stop agents' own pull requests. Chose a
committed file over saved CI results because the change then shows in the diff and a fork's pull
request needs nothing special. Chose to keep times off CI (a shared runner's numbers mean nothing)
and to require the `make perf` table from a Mac in the body of a web view pull request (the `perf`
check), forks exempt; rejected the nightly run alone because it finds a regression after it may have
shipped, and rejected Linux runs as the record because unchanged code misses a budget there. Chose
one run that changes every surface (`docs/dev/surfaces.md`, the `surfaces` check), `follows:` only
for a design answer, an outside contributor or an urgent live fix; rejected "follow later" because
it is how the app fell behind.

## 2026-11-12: the Compare diff over the API

The iPhone and iPad app reads only `/api/v1` with a Bearer Token, so it could not compare two
Versions. Chose `GET /api/v1/docs/{id}/diff?from=A&to=B` with the page's `from`/`to` and defaults,
answering the `rendered_diff` fragment and its numbers as JSON, through the one `compare::compare`
the page calls, so the two cannot disagree; rejected a path under `versions/{n}` because a diff
belongs to two Versions and the page's defaults need neither, and rejected letting the app read the
page because the page is cookie sign-in and chrome. Chose to send the fragment as the renderer and
the diff write it (Asset links `/a/{hash}`, no notes) and to leave the notes to the client, worded
from `same_content` and `summary`, as `/versions/{n}/html` leaves the Document page's chrome out;
rejected sending the page's note sentences because they are web text the app words itself. A
malformed Version is 404 `not_found`, as on the page, rather than a 400, because both are "no such
Version" and the shared code answers the same. No CLI verb: an agent reads Markdown (`pull`, the
Sidecar), and a terminal cannot show an HTML diff.
