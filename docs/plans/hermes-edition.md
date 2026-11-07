# Plan: Annotary for Hermes (the Hermes Agent Plugin Hackathon entry)

Status: draft for Josh's review, 2026-11-06. Nothing built.

## What Josh asked for

- "a Hermes add-on that can work well with the Hermes desktop, the Hermes mobile app, and the
  Hermes terminal interface to provide the annotation functionality for sessions that happen in
  Hermes", "shipped in time for the Hermes Hackathon".
- "fully contained on the computer that is running Hermes and works out of the box"; "the web
  interface for Annotary needs to be served through Hermes"; works "on their own computer or on a
  remote computer, a VPS"; mobile "if at all possible", or at least through the Desktop's web mode.
- Setup from the command line, and a Desktop UI only if there is something to configure.
- "by default, the user should get an account", and "each profile that is in Hermes should
  automatically get an account", for existing and future profiles, with removed profiles handled
  cleanly.
- An off-ramp to full Annotary "so that they can use what they've done in Hermes, and it works
  across harnesses", and a path to the iOS app later.
- Goals: win, ship on time, give Hermes users every Annotary feature, and a distribution channel
  for Annotary.

## The contest (checked 2026-11-06, x.com through Grok, and the entry form)

- **Hermes Agent Plugin Hackathon**, run by Nous Research, announced 2026-11-04
  ([post](https://x.com/NousResearch/status/2118094256438936062)). $20,000 cash: $10,000, $5,000,
  $2,500, $1,500, $1,000, plus five runners-up (merch, "creds"). One pool, no tracks.
- **Deadline: Wednesday 2026-11-11.** The post says 5 PM EST; the
  [entry form](https://form.typeform.com/to/FwJLvOln) says 1 PM PST. **We plan to the earlier
  one, 1 PM PT**, and aim to finish on Tuesday the 10th.
- **To enter:** (1) open a PR adding `plugin-catalog/<name>.yaml` to `NousResearch/hermes-agent`
  ([guide](https://hermes-agent.nousresearch.com/docs/developer-guide/plugins/catalog-submission));
  (2) post a demo video under 90 s on X tagging @NousResearch; (3) submit both links, an email
  and a Discord username on the form. The form can be sent as soon as the PR is open. Maintainers
  review catalog entries "every few days", and catalog CI runs only once a maintainer starts it.
- **Judging:** "usefulness, reliability, polish and creativity", favoring "plugins that do more
  than a prompt or a skill could". Judges are not named. Accepted entries are listed in the new
  Hermes Marketplace. Multiple entries allowed; one win per person.
- Not stated: team size, region, whether the work must be new. Building on Annotary is not
  excluded.
- The field so far is small single-purpose plugins. The closest one is `mock-review` (review
  boards for UI mockups, served from a port in the gateway, waking the agent through the webhook
  platform). Nothing in the catalog does Markdown review with anchored Threads.

## What was checked (2026-11-06)

Hermes source read at `NousResearch/hermes-agent` `8153953654` (2026-11-07, after release
`v0.23.0` of 2026-11-05), plus Josh's two shipped Hermes plugins (the Winley Notes Desktop page and the
hermes-gadget platform plugin), whose notes on the plugin API are confirmed by the source.

Conflicts and hang-ups first:

1. **Hermes frames are opaque-origin, always.** Desktop plugins show web content only through the
   SDK's `SandboxedFrame`: `allow-same-origin` is stripped, so the frame has no cookies and no
   storage, and its requests are cross-site (`desktop-plugin-sdk.md`, "Embedding external
   content"). Annotary's web pages sign in with a `cd_session` cookie, store prefs in cookies, call
   `/api/v1/...` by absolute path, use `EventSource`, and refuse cross-site writes
   (`login_guard::refuse_cross_site`). **The web pages cannot run unchanged inside Hermes.**
2. **"Served through Hermes" means through the plugin's backend, not a URL.** On a remote backend
   (Desktop over SSH, the Webapp on a phone, a VPS) the browser cannot reach a server on the Hermes
   machine's loopback. The one path that reaches the Hermes machine from every surface is the
   plugin's own backend, `ctx.rest` → `plugin_api.py` at `/api/plugins/<id>/`. Those routes sit
   behind Hermes's own sign-in, so a frame cannot load them by URL. The page has to get its content
   from the plugin through `postMessage`.
3. **Hermes now serves the Desktop workspace to an ordinary browser** (the "Webapp": `hermes
   webapp`, port 9120; **Settings → Web Access** turns it on for other devices behind a Nous
   account sign-in, or behind an HTTPS address you run). Desktop plugins run there too, and the SDK
   documents how `SandboxedFrame` behaves in it. This is the route to phones for a plugin UI.
4. **The Hermes iPhone app is a waitlist beta with no plugin surface.** It connects through the
   gateway's dashboard sign-in (`desktop.md` names phone chats labeled `mobile` / `ios`). Teknium on
   2026-11-06: "There is a waitlist"
   ([post](https://x.com/Teknium/status/2118536433845875104)). No Nous post or doc says it renders
   plugin pages, web views or Markdown, and one user reports reconnect loops after 0.23. What beta
   testers are told to use meanwhile is the Webapp in a phone browser: "I would try serving Desktop
   in mobile browser via tailscale" (a beta tester,
   [post](https://x.com/3rosika/status/2118697545635447009)). So **mobile means the Webapp**, and
   the native app only gets links.
4a. **Hermes v0.23.0 ("The Hearth Release", 2026-11-05)** is the release that serves the Desktop
   workspace in a browser and loads installed Desktop plugins in a Webapp tab. The entry requires
   `>=0.23.0`. The web dashboard (`hermes dashboard`) is "going away" (Teknium,
   [post](https://x.com/Teknium/status/2118536701081772236)), so nothing here builds on dashboard
   tabs. Practitioners' main pain is a plugin that must run both on a headless server and in Desktop
   ([post](https://x.com/dmkjr/status/2118886632925569342)); this design is built for exactly that
   split, which is worth saying in the demo.
4b. **Desktop's in-app browser has an "Annotate" mode** that pins comments on a live web page and
   drops them into the composer. It is a one-shot note to the agent, with no document, Versions,
   Threads or replies. Annotary is different, and the README should say how in one line.
5. **No profile lifecycle events.** `VALID_HOOKS` has session hooks but nothing for a profile
   being created or deleted. Profiles are folders under `~/.hermes/profiles/` with one of
   `config.yaml`, `.env`, `SOUL.md`, `profile.yaml`, `auth.json`, `state.db`; the default profile
   is `~/.hermes` itself. Accounts per profile need a reconcile loop, not a hook.
6. **The agent half is switched on per profile.** A user plugin's Python loads only when it is
   in that profile's `plugins.enabled`. `profile create --clone` copies `plugins/`; a blank new
   profile does not get it. The Desktop half is app-level, once for every profile.
7. **The catalog forbids bundled binaries and self-updating code** (rules 3 and 17). A native
   binary the user's machine fetches at setup is allowed "only from the upstream tool's official
   source, pinned by checksum, and disclosed". Annotary's own GitHub Releases are that source; the
   plugin pins one release's checksums per plugin version and never updates itself.
8. **Approvals.** An agent that runs the `annotary` CLI through the terminal tool meets Hermes's
   command approvals on every call, and messaging-only profiles may not have a terminal at all.
   Native plugin tools avoid both.
9. **Windows.** Hermes runs on native Windows; Annotary ships for macOS and Linux only. The entry
   declares `platforms: [linux, macos]` (WSL works as Linux).
10. **Hermes Cloud** (hosted, scales to zero, no inbound ports) is out of scope.
11. **This repo is public**, and its rules bind the Annotary half: a web-page change needs
    `make verify-live` on the live server and a review through `review-gate`; a release is a tag.

What exists to reuse:

- **The plugin shape**, proven twice by Josh: one folder with `plugin.yaml` + `__init__.py` (tools,
  hooks, CLI), `dashboard/manifest.json` + `plugin_api.py` (FastAPI routes), `desktop/plugin.js`
  (plain ESM with `jsx()`, a page in the sidebar), `skills/`. Install with `hermes plugins install
  owner/repo[/subdir]`; one-click `hermes://plugin/install?catalog=<name>` once listed.
- **Hermes APIs that make this more than a skill:**
  - `ctx.register_tool` for native tools;
  - `ctx.inject_message(content, session_key=…)` to start a turn in a given session, gated by the
    plugin's `allow_gateway_injection` setting;
  - `broadcast_plugin_event` to push from the backend to every connected Desktop or Webapp window;
  - `TRANSCRIPT_DIRECTIVE_AREA`, so the agent writes `::annotary{doc="…"}` and Desktop draws an
    inline Document card;
  - `ctx.os.notify` with a deep link;
  - `register_command` for slash commands and `register_cli_command` for `hermes annotary …`;
  - `config_schema` for a settings form in Desktop.
- **Annotary already has what the accounts need:**
  - scriptable first-run setup (`ANNOTARY_SETUP_HANDLE/NAME/PASSWORD`, `serve --setup-only`
    style);
  - Members, Tokens and Grants by API;
  - `member disable`, never delete, with handles never reused;
  - `ANNOTARY_TOKEN` + `ANNOTARY_SERVER` for the CLI;
  - `annotary export` for a consistent archive;
  - loopback-only by default.
- **The off-ramp is already designed in:** the Hermes edition is the same binary and the same
  store format, so "upgrading" is pointing other tools at it, or moving its data dir. Phase 2 cloud
  sync was designed as "a transport problem, not a re-model" (`docs/phase1/spec.md`).

## The design in one paragraph

A Hermes plugin named `annotary` runs a private Annotary server on the Hermes machine (loopback,
its own data dir under the Hermes home, a random owner password nobody types), creates an owner
Member for the person and an agent Member for every profile, and keeps those in step with the
profile list. Agents get native tools (share a Markdown file, read Threads, reply, resolve, push a
new Version, take an Action). People get an **Annotary** page in the Desktop sidebar and in the
Webapp (and so on a phone). The page shows Annotary's own server-rendered pages in a sandboxed
frame, fed through the plugin's backend over `postMessage`. Desktop chats show an inline card for
each shared Document. When someone comments, mentions the agent or approves, the plugin wakes the
session that shared the Document, with the new comments as the turn. Everything is one install
command; the only settings are optional.

```
 Desktop / Webapp (any machine, phone)          Hermes machine (laptop, VPS, Docker)
 ┌─────────────────────────────────┐           ┌───────────────────────────────────────────┐
 │ Annotary page (desktop/plugin.js)│ ctx.rest  │ plugin_api.py  ──► annotary serve          │
 │  └ SandboxedFrame (opaque)  ◄────┼─ events ──┤   /api/plugins/annotary/*   127.0.0.1:PORT │
 │     Annotary pages, embed mode   │           │ agent tools (per profile) ──► same server  │
 │ ::annotary card in chats         │           │ watcher ──► inject_message(session)        │
 └─────────────────────────────────┘           └───────────────────────────────────────────┘
 TUI / CLI / Telegram: tools + links; `hermes annotary …`
```

## Decisions

**D1. Scope of the entry.** Five days do not cover every Annotary feature inside Hermes. The core
loop is: an agent shares a plan, a person comments on any surface, the agent wakes and answers or
pushes a new Version, the person approves. Owner screens (Members, Tokens, Vaults, server
settings) are the risky, low-value part to embed.
- **Recommended: A.** Ship the core loop everywhere, polished: Document list, Document view with
  comment, reply, resolve, Actions and Version switch; the agent tools; the wake-up; the inline
  card. Owner screens stay on the CLI (`hermes annotary …`) and come after the contest. This is the
  version that can be reliable by Tuesday, and reliability is a judging criterion.
- B. Also embed the Settings and owner screens. Doubles the Annotary web changes and their review,
  and puts the deadline at risk.

**D2. Where the plugin lives.** The catalog clones a public repo at a pinned SHA. The Marketplace
page renders the README at that SHA, and the one-click `?repo=` link takes `owner/repo` only, with
no subfolder.
- **Recommended: A.** A new **public** repo, `josh-andrews-dev/annotary-hermes`, on the repo
  standard (GitHub Issues as tracker, like this repo). Its README is the Marketplace page, its CI
  is small (Python and JS, no Rust), and installs are a small clone. Annotary changes (embed mode,
  release attestation) still land here, in this repo. The global rule makes new repos private
  unless you say otherwise, so this needs your yes.
- B. A `hermes/` folder in this repo, with the catalog's `subdir`. One repo, but the Marketplace
  README and the `?repo=` link both point at the whole Annotary project, and every plugin change
  runs the Rust CI.

**D3. Handles for the accounts.** Annotary handles are permanent and never reused. Your own
Hermes Members use `@hermes-<profile>` today.
- **Recommended: A.** The profile's name as the handle (`@coder`, `@research`), the default
  profile as `@hermes`, and the person as their OS user name. `@coder` is what people will type in
  comments. On a clash, add `-agent` to the agent's handle.
- B. `@hermes-<profile>` for every agent, as in your own setup. It never clashes with a person,
  and it says which harness wrote the comment once the store is shared with other harnesses (the
  off-ramp). It is longer to type.

**D4. "An account like we do with Notetaker".** I found no project called Notetaker in your repos.
I read it as "the person gets an account without being asked", which Annotary's first-run setup
already does when run without prompts.
- **Recommended: A.** At install, the person gets the owner Member automatically: the handle from
  their OS user name, the display name from their account, no prompt. Hermes's own sign-in is the
  gate (Desktop session, Webapp sign-in), so the person never types an Annotary password; the
  plugin sets a long random one so that a browser on the machine cannot sign in to the bare server
  by loopback trust.
- B. It means something else; tell me what Notetaker does and I will match it.

**D5. Waking the agent.** Hermes lets a plugin start turns only when the plugin's
`allow_gateway_injection` setting is on, and the catalog forbids routing around user consent. You
want no configuration.
- **Recommended: A.** Ask once, at the moment it matters: the first time an agent shares a
  Document, the tool's reply tells the person "Comments on this Document can wake this chat: turn on
  in Settings → Plugins → Annotary, or run `hermes annotary wake on`". The Annotary page shows the
  same switch. Until it is on, new comments show as a Desktop notification and on the inline card,
  and the agent sees them the next time it calls a tool.
- B. Turn it on during install, with a line in the install output. No step for the person, but a
  catalog reviewer may read it as working around the gate (rule 12), which risks the listing.

**D6. Agent tools in profiles created later.** Accounts can follow profiles automatically (D3's
reconcile), but the agent half loads only in profiles that enabled the plugin, and a blank new
profile does not get it. Writing other profiles' `config.yaml` from a running plugin is the kind of
core-adjacent change catalog review dislikes.
- **Recommended: A.** Every profile gets its account automatically. Its tools come with
  `hermes annotary enable --all` (run once at install; enables the plugin in every profile, the way
  your Winley Notes installer does), and later profiles show a one-click **Turn on for this
  profile** on the Annotary page (it uses Desktop's own **Install here** path).
- B. Same, but the reconcile loop also enables the plugin in each new profile by itself. Zero
  steps, but it edits profiles nobody asked it to.

**D7. A real VPS for testing.** "Works on a VPS" has to be shown on one: Desktop connected over
SSH, the Webapp from a phone, and a Docker install. My recommendation for the machine is under
*Choices I made*, since your standing rule (D6) covers it. One thing only you can settle: the
test Hermes needs a model provider key to run agent turns.
- **Recommended: A.** Use the key from your Hermes test setup, copied to the VM through a secret
  request (never printed), and revoked or left unused when the VM is deleted on Wednesday.
- B. A separate key just for the test VM, made by you (5 minutes).

**D8. The demo and the post.** The post goes out from your X account, the form needs your email
and Discord username, and the demo has to be under 90 s. All three are outward-facing.
- **Recommended: A.**
  - I script the demo and record the Desktop part on your Mac through `cua-driver` (with the
    display lease), then export it with OpenScreen.
  - You record about 15 s on your phone: open the Webapp, comment, see the agent reply.
  - I cut it together and draft the post text.
  - You post it, fill in the form and send me the post link. That is about 20 minutes of your time
    on Tuesday.
- B. You record the whole demo yourself from a script I write.

**D9. The off-ramp for the contest.** "Use what they've done in Hermes, and it works across
harnesses."
- **Recommended: A.** Ship `hermes annotary connect`, which prints, for Claude Code, Codex or any
  shell, the two lines that give that harness its own Member and the Annotary skill against the
  same server. The same Documents and Threads then work across harnesses on day one. Also ship
  `hermes annotary export` (Annotary's own archive) and a README section, "Move to standalone
  Annotary": install `annotary`, restore the archive, done. Cloud sync stays Phase 2.
- B. Also build "attach to an existing Annotary server" (use your own server in place of the
  private one). Useful to you, but it adds a second mode to test before Tuesday; it comes after the
  contest.

**D10. Mobile for the contest.** The Webapp's Web Access is the path to any phone browser, and
plugin pages run there.
- **Recommended: A.** Ship and demo mobile through the Webapp (Annotary links the agent sends open
  the Annotary page in the Webapp, signed in with Hermes). Treat the Hermes iPhone app as "links
  open in the browser" until Nous says it renders plugins. Pairing the Annotary iPhone app to a
  Hermes-run server comes after the contest.
- B. Also try to make the Hermes iPhone app show the page. That is blocked on a private beta we
  cannot read, so it is not plannable in five days.

## M1: from Josh

- Answer D1–D10 ("all recommended" works). Tonight if possible: M2 starts at your go.
- D2: say yes to a public repo `annotary-hermes`.
- D7: the key (A, or make one).
- On Monday evening: 10 minutes on your iPhone. Open the Webapp link I send and leave a comment
  (Web Access needs your Nous account in the Desktop app's **Providers → Accounts**; if it is not
  signed in there, that is 2 minutes in the Desktop app).
- On Tuesday: record about 15 s on your phone, post the demo on X, fill in the form, send me the
  post link (D8). About 20 minutes.

## M2: Spikes (Fri night to Sat morning, in parallel)

1. **Frame bridge spike** (the plan's biggest risk). Build a throwaway Desktop plugin with:
   - a `SandboxedFrame` whose `src` is a tiny `data:` shell;
   - the plugin answering the shell's `postMessage` with HTML that came from `ctx.rest`;
   - the shell posting back a form submit.

   Run it on native Desktop, on the Webapp in a desktop browser, and on the Webapp in iPhone
   Safari (the simulator first). Measure: whether messages and `onLoad` arrive on both, and the
   largest page we can pass.

   *Done when:* a round trip works on all three, with screenshots.
2. **Validate spike.** Run an empty `annotary` package with `plugin.yaml`, `plugin_api.py`,
   `desktop/plugin.js` and one tool through `hermes plugins validate --install-deps` on the
   current Hermes.

   *Done when:* it passes and the warnings are written down.
3. **Server-in-Hermes spike.** From the plugin, start the pinned `annotary` binary with its data dir
   under `$HERMES_HOME/annotary/`, a fixed loopback port and scripted first-run setup. Restart the
   gateway and run two profiles' gateways, and check that one server survives and only one ever
   holds the LOCK.

   *Done when:* the health check answers from the plugin's backend after each restart.

If spike 1 fails on the Webapp, fall back (reported, not silent): the Webapp gets a native SDK
Document list plus a link that opens the Document in a new tab through the Webapp's own sign-in,
and native Desktop keeps the frame.

## M3: Annotary embed mode (this repo, Sat to Sun; reviewed; released as `v0.4.0`)

- The server renders the list and Document pages for an **embedded** client: no cookies, no
  `EventSource`, and every request, link and asset goes through one transport object that the
  Hermes page provides over `postMessage`. Card markup keeps one renderer (Rust).
- In `app.js`, one `transport` replaces the direct `fetch`, `EventSource`, `location.href` and
  `document.cookie` uses (about 15 call sites). In a normal browser nothing changes.
- Prefs move to a Member Setting (already server-side) or are passed in by the embedder.
- Live updates arrive as messages from the embedder in place of the SSE stream. The data is the
  same events (`/api/v1/events`), relayed by the plugin.
- Assets: images through the bridge (size-capped); video and PDF get an "open" button that downloads
  through the plugin.
- Spec ids first (`tests/SPEC.md`, a section for the embed mode), then tests: Rust for the
  rendering, Playwright with a fake embedder page for the bridge, and the existing suites unchanged.
- `release.yml`: add `actions/attest-build-provenance` for the four tarballs. This is a CI change,
  so it is reviewed. It makes the catalog's binary rule an easy yes.
- `make check` green, `make verify-live` on the live server, `review-gate`, deploy, tag `v0.4.0`.

*Done when:* `v0.4.0` is released with attested tarballs, and the fake-embedder test drives
comment, reply, resolve and Approve against a real `serve`.

## M4: The plugin (new repo `annotary-hermes`, Sat to Mon)

**Install and server**
- `hermes plugins install josh-andrews-dev/annotary-hermes --enable` (and `catalog=annotary` once
  listed).
- On first use it downloads the pinned `annotary` tarball for the platform from the `v0.4.0`
  release, checks its SHA-256 against the value in the plugin source, and installs it under
  `$HERMES_HOME/annotary/bin/`.
- **Ensure-running:** every entry point (gateway load, a tool call, a backend route) checks the
  server's health and starts it if needed. It runs detached, so it outlives a gateway restart; the
  LOCK prevents a second copy. `hermes annotary status|stop|logs`.
- It never touches `~/.annotary`, so a standalone Annotary on 7377 keeps running beside it.

**Accounts**
- First start runs the scriptable setup: the owner Member (D4), a random password the plugin keeps
  and nobody types, and the owner Token kept in the plugin's data (mode 600).
- **Reconcile** (on gateway start, on every backend call at most once a minute, and every 5 min):
  - list the profiles;
  - create a Member for each new one (D3), with comment and action rights on every Vault plus
    write on the Vault it shares into, and a Token kept in that profile's plugin data;
  - disable the Member of a profile that is gone (its Comments keep their author);
  - re-enable the same Member when a profile with that name comes back.
- `hermes annotary members` shows the mapping.

**Agent tools** (native, so there are no approval prompts and they work in messaging-only profiles)
- `annotary_share(path | markdown, name, message)` → id, Webapp link, Desktop deep link, and the
  `::annotary{doc}` card on Desktop. The tool runs the bundled CLI for `push`, so Assets and Sidecars
  behave as in Annotary.
- `annotary_threads(doc, open_only)`, `annotary_reply(doc, thread, text, resolve)`,
  `annotary_push_version(doc, path, message)`, `annotary_act(doc, action)`, `annotary_list()`.
- A skill that teaches the loop in under 40 lines: share, read Threads, reply, push, and when to use
  the card. It is named after the plugin.
- **Surface-aware output:** the card directive only on Desktop (`ctx.session_surface()`), plain
  links on TUI, CLI and messaging.

**Wake-up**
- A watcher follows Annotary's events for the Members it manages.
- On a new Comment on a Document an agent shared, an @mention, or an Action, it calls
  `inject_message` on the session that shared it (recorded at share time) with the new Comments
  quoted, when the setting is on (D5). Otherwise it sends a Desktop notification that opens the
  Document.
- Debounced (one wake per Document per 30 s), and never for the agent's own Comments.

**Desktop and Webapp half**
- An **Annotary** sidebar page: the Document list, then the Document view in the frame (M3 embed
  mode), following Hermes's light and dark theme.
- The inline card shows the name, Version, open Threads and the latest Action, with an Open
  button.
- `broadcast_plugin_event` drives live updates, with polling as the fallback (the SDK says
  sockets can drop).
- Settings: one switch for the wake-up, nothing required.

**Slash command:** `/annotary review`. It shares the last assistant message as a Document, the way
Hermes users can "annotate a session": the plan the agent just wrote becomes a reviewable Document
in one step.

**Off-ramp (D9):** `hermes annotary connect [claude|codex|shell]` and `hermes annotary export`.

**Tests:**
- pytest for reconcile, the profile mapping, tools and the wake rules, against a real `annotary
  serve`;
- a Desktop plugin smoke test (it loads and renders the list);
- `hermes plugins validate --install-deps` in CI.

**Review:** the plugin handles Tokens and starts a server, so it goes through `review-gate`.

*Done when:* on a real Hermes install, a fresh profile gets its Member within 5 minutes, a deleted
one is disabled, and the full loop runs from the TUI.

## M5: Proof on real targets (Mon)

Same checklist on each:
1. Install from the repo with one command.
2. An agent shares a plan.
3. A person comments in the page.
4. The agent wakes and replies, then pushes a new Version.
5. The person approves.

The targets:
- **macOS, native Desktop, local backend** (your Mac).
- **Linux VPS**:
  - Desktop connected over SSH;
  - the Webapp over HTTPS (Tailscale Serve, or Web Access with Nous sign-in);
  - the Webapp in iPhone Safari (you, 10 minutes).
- **Docker** (`docker compose` from the Hermes repo) on the same VM: the data dir sits on the volume
  and survives a container rebuild.
- **TUI and Telegram-style messaging:** links and tools only.

*Done when:* every row passes, with screenshots and a short screen recording for each. The
results go in the plugin README's "Tested on" table.

## M6: Ship the entry (Mon night to Tue)

- Open the catalog PR by **Monday night**, so a maintainer has a day to start CI:
  - `plugin-catalog/annotary.yaml`, pinned SHA, `category: tools`, `platforms: [linux, macos]`;
  - `capabilities` that match the code exactly;
  - a 1200×600 banner and up to 6 screenshots;
  - a rule-13 disclosure: it downloads one pinned, attested binary from Annotary's GitHub Release,
    runs a loopback server in the background, keeps Tokens in the plugin's data, and sends nothing
    off the machine.
- Then the demo (D8), the X post and the form, by **Tuesday 6 PM PT**.
- A review request on the PR is answered with a SHA bump the same day.
- Wednesday morning is buffer only. Nothing new is built after Tuesday noon.

**Distribution, built into the work:**
- The Marketplace listing and its one-click install link.
- A "Use Annotary from Hermes" section in this repo's README.
- The demo post (and, when the contest closes, a reply in the Nous Discord thread opened for every
  entry).
- `hermes annotary connect` as the hook into the full product: "the same review, from Claude Code
  and Codex".

## Schedule (Pacific time)

| When | What |
|---|---|
| Fri 11-06 evening | Your answers (M1). Spikes 1–3 start at go |
| Sat 11-07 | Spikes report by noon. M3 embed mode and M4 server, accounts and tools in parallel threads |
| Sun 11-08 | M3 review, verify-live, `v0.4.0`. M4 Desktop half, card, wake-up |
| Mon 11-09 | M5 on Mac, VPS, Docker; your 10-min phone check in the evening. **Catalog PR open by Monday night** |
| Tue 11-10 | Fixes from M5, demo recorded and cut, README and screenshots final. **You post and submit by 6 PM** |
| Wed 11-11 | Buffer; answer catalog review. Deadline **1 PM PT** |

## Stop rules

- Spike 1 fails on native Desktop, so the core UI cannot be served through Hermes: stop and
  report with options before M3.
- `hermes plugins validate` refuses the design (rules 3, 9, 12 or 17): stop and report.
- Sunday 6 PM with M3 not merged: ship the plugin against embed mode on a release candidate tag,
  and report it.
- Monday 6 PM with any M5 row failing: cut that surface from the demo and the README claims,
  rather than slip the PR.
- The run carries on through waits for CI, review and the catalog maintainer.

## Not doing (before the contest)

- Owner screens inside Hermes (D1); attaching to an existing Annotary server (D9 B); cloud sync.
- The Hermes iPhone app showing the page (D10 B). The Annotary iPhone app pairing to a Hermes-run
  server comes later: it needs the server reachable from the phone (`annotary pair` exists).
- Windows and Hermes Cloud.
- Multiple people on one Hermes. Every Webapp user is the one owner, which matches how Hermes
  treats Webapp users.

## Risks and cost

- **Time is the main risk.** M3 and M4 run in parallel threads, and M3's review and verify-live are
  on the critical path. Embed mode is the one piece that needs real design care.
- **Catalog review latency** ("every few days"): the form only needs the PR open, so a late merge
  does not lose the entry. It does delay the Marketplace listing.
- **Hermes moves daily** (about 1,950 merged PRs between 0.22 and 0.23): `requires_hermes:
  ">=0.23.0"`, test on the newest release on Monday, re-run validate on Tuesday morning.
- **Store versions:** an older plugin with an older binary cannot open a store a newer one
  migrated. The plugin exports before it upgrades the binary.
- **Money:** one Google Cloud VM, `e2-small` in `us-central1` (about $13 a month list price,
  about $2 for the 4 days), in project TBD (named in the run log), deleted Wednesday. Prizes aside,
  nothing recurring.

## Choices I made

- The test VPS is a Google Cloud VM under your standing rule, not a home machine, so the remote
  path is a real internet one. It is deleted on Wednesday.
- The server runs detached with ensure-running, not as a gateway child, so CLI-only use and gateway
  restarts both keep it up.
- The private server gets its own data dir under the Hermes home and its own port, never
  `~/.annotary`.
- Native tools call the bundled CLI (with `ANNOTARY_TOKEN`/`ANNOTARY_SERVER`, argv only, no shell)
  rather than reimplementing push in Python.
- A random owner password closes loopback trust on shared machines; all human access goes through
  Hermes's sign-in.
- Embedding through `postMessage`, not a URL with a ticket: it is the only path that works on every
  remote surface.
- Catalog name `annotary`, title "Annotary", category `tools`.
- The plan to the earlier deadline (1 PM PT), with the submission on Tuesday.

## Run log

- 2026-11-06: research done (Hermes source, Josh's two Hermes plugins, x.com through Grok);
  plan drafted. Nothing built.
