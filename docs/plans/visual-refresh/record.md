# Visual refresh of the web UI: record

The design record for the web UI's visual refresh. Boards: [round 1](round-1.html) (directions),
round 2 (the chosen direction across every screen) and round 3 (states and the final pass), in
this folder. The decisions that last go to [`DESIGN.md`](../../../DESIGN.md); this file keeps how
they were reached.

## What it is

Josh's brief (2026-11-07, condensed): "Make it look a bit sleeker. I like the minimal UI and want to
keep it. At the same time, make it a little more lively, fun and visually striking, within
minimalist bounds. Work on that for a few rounds and try to make it better." He approved shipping
it once every gate passes. The page also stars in a demo video for the Hermes Agent plugin
(agents commenting on a shared Document, desktop and phone).

Case: an existing design system (`DESIGN.md`, extracted from `app.css`), refreshed inside its
rules. Josh delegated the picks to the design agent, which takes each round's recommendation
and records it below as decided by the agent, not by Josh.

## Data

Real fixture data: `fixtures/corpus/rfc-tile-cache.md` and its history (v1 to v4), seeded on a
local `annotary serve` (2026-11-08) with three people (Rowan Vale, owner; Mei Tanaka; Tomasz
Reed) and three agents (Hermes, Scribe, Linter), Threads, replies, two resolved Threads, a send
back on v1 and an approve on v4. The seed script is scratch and not committed.

## Verified

- No web font: the perf budgets (`perf/BUDGETS.md`: FCP 100 ms, CLS 0) and embed mode (every
  request goes through the embedder's `postMessage` transport, `docs/dev/web-document-view.md`)
  rule out a font request; checked 2026-11-08.
- Pink is the mark's color and "means nothing else in the UI"; the accent stays blue
  (`DESIGN.md`, Brand, decided 2026-11-01).
- People and agents read alike: `MemberBrief` carries only how a Member signs in
  (`server/ui.rs`); there is no Member kind to badge.
- Disabled controls (the off Version arrow) are exempt from WCAG 1.4.3; the board's audit skips
  `[aria-disabled="true"]`.

## Round 1 (2026-11-08): directions

1. Direction: A (Refine), **B (Margin note)**, or C (Brand color). Recommended B.
2. Document typeface: **T2 A (system sans)** or T2 B (system serif). Recommended T2 A.
3. Filled positive Action button: **yes** or no. Recommended yes.

Self-critique after rendering: B's tie reads as the mark and is the only new element; its ink
ring is heavy; in dark, Reply / Resolve pills and Comment separators vanish because `--cd-soft`
equals `--cd-card` there; the phone h1 hyphenates "two-tier" across lines at 2.05em. A is tidy
but is not "lively". C is striking and breaks two decided rules.

**Decided (agent, 2026-11-08):** 1 B, 2 T2 A, 3 yes. Round 2 applies B to every screen and
settles the ring color and the dark pill ground.

## Cut from the boards

- Per-Member name colors and an "AI" badge (drawn in C, rejected): identity comes from the avatars
  another change adds.
- A web font (not drawn): see Verified.
