# Annotary web UI: design system

The web UI's look, extracted from `crates/annotary/src/server/ui/app.css` (every page) and
`ui/admin.css` (the owner screens and the login page), plus the decisions of the Member settings
mock rounds (rounds 1–3, approved 2026-10-30). The CSS is the source of truth; this file says what
it is and why. The iOS app, which is not in this repo, has its own design notes; the web's dark
palette is taken from them.

## Rules

- **Every id and class the pages own starts with `cd-`.** The rendered Document keeps the bare
  element names and the renderer's few classes (`asset-missing`, `footnotes`, `language-x`, …).
- **The first paint is final.** Threads, placements and the Member's settings are in the first
  HTML response; the script never changes what it showed (CLS 0 is a perf budget). Anything the
  script rewrites after the first paint must keep its width (relative times are rendered by the
  server with the age the script would compute).
- **One renderer.** Card markup comes from Rust only (`view.rs::thread_card`); the script swaps in
  the server's fragment, never builds a card.
- **No `position: fixed`.** The pill, toast, panels and help are `position:absolute` inside
  `.cd-app`; the header is `position:sticky`.
- **Colors are tokens.** A rule never names a color; it names a token defined in both palettes.
  The exceptions are the pairing QR code's white ground (a camera reads dark-on-light) and the
  brand icon, whose colors are the mark's in both palettes.
- **`admin.css` / `admin.js` never go into the Document pages.**
- **American English** in the UI and the docs.

## Appearance

The Member's Appearance setting (Follow system, Light, Dark) is a class on `.cd-app`, written by
the server (`server/prefs.rs`):

| Setting | `.cd-app` class | What applies |
|---|---|---|
| Follow system | `cd-theme-system` | light, and the dark block under `@media (prefers-color-scheme: dark)` |
| Light | `cd-theme-light` | light only |
| Dark | `cd-theme-dark` | the dark block |

Light lives on `:root`; dark is one block on `.cd-app.cd-theme-dark` and the identical block on
`.cd-app.cd-theme-system` inside the media query (a unit test keeps the two equal and checks dark
redefines every light token). `.cd-app` is at least the viewport's height and paints `--cd-bg`;
`html:has(>body>.cd-app.cd-theme-dark)` paints the canvas black too, so overscroll never flashes.
`color-scheme` follows the palette, so native controls and scrollbars match. Pages with nobody
signed in (login, errors) follow the system.

## Tokens

| Token | Light | Dark | Use |
|---|---|---|---|
| `--cd-bg` | `#fff` | `#000` | page ground |
| `--cd-fg` | `#1c1c1e` | `#f2f2f7` | text |
| `--cd-muted` | `#6b6f76` | `#98989f` | secondary text, labels, meta |
| `--cd-line` | `#e5e7eb` | `#38383a` | borders, rules |
| `--cd-soft` | `#f6f7f9` | `#1c1c1e` | hover, code, table heads, current nav |
| `--cd-card` | `#fff` | `#1c1c1e` | cards, chips, buttons, panels, menus |
| `--cd-field` | `#fff` | `#2c2c2e` | text fields, selects in forms |
| `--cd-dim` | `#fafafa` | `#141416` | orphaned cards, disabled Members |
| `--cd-faint` | `#c9cbd3` | `#48484a` | disabled arrows, the Document-level strip rule |
| `--cd-accent` | `#2563eb` | `#60a5fa` | links, mentions, focus, badges |
| `--cd-accent-ink` | `#fff` | `#000` | text on the accent |
| `--cd-sel-bg` / `--cd-sel-ink` | `#eef2ff` / `#1e40af` | `#1e293b` / `#bfdbfe` | a selected row or option |
| `--cd-amber` / `--cd-amber-bg` | `#b45309` / `#fef3c7` | `#fbbf24` / `#3a2a0a` | Mixed, "changed elsewhere" |
| `--cd-err` / `--cd-err-bg` / `--cd-err-line` | `#9f1239` / `#fff1f2` / `#fecdd3` | `#fca5a5` / `#2a1215` / `#7f1d1d` | errors, the toast |
| `--cd-err-strong` | `#b00020` | `#fca5a5` | the error line of a simple page |
| `--cd-hdr-bg` | `rgba(255,255,255,.96)` | `rgba(0,0,0,.92)` | the sticky header (with a 6px blur) |
| `--cd-warn` / `--cd-warn-bg` / `--cd-warn-line` | `#c2410c` / `#fff7ed` / `#fed7aa` | `#fdba74` / `#331a08` / `#7c2d12` | deleted, orphaned, confirms |
| `--cd-note-bg` / `--cd-note-line` / `--cd-note-ink` | `#fffbeb` / `#fde68a` / `#78350f` | `#3a2a0a` / `#78350f` / `#fde68a` | the older-Version banner |
| `--cd-info-bg` / `--cd-info-line` / `--cd-info-ink` | `#eff6ff` / `#bfdbfe` / `#1e3a8a` | `#0f1b2e` / `#1e3a8a` / `#bfdbfe` | the new-Version banner, shown-once Token / pairing panels |
| `--cd-owner-bg` / `--cd-owner-ink` | `#eef2ff` / `#3730a3` | `#1e1b4b` / `#c7d2fe` | the owner role chip |
| `--cd-primary` / `--cd-primary-ink` / `--cd-primary-hover` | `#1c1c1e` / `#fff` / `#000` | `#f2f2f7` / `#000` / `#fff` | primary buttons, the selection pill |
| `--cd-kbd-line` / `--cd-kbd-ink` | `#666` / `#ddd` | `#8e8e93` / `#3a3a3c` | `kbd` on the pill |
| `--cd-stripe-a` / `--cd-stripe-b` / `--cd-dash` | `#fafafa` / `#f3f4f6` / `#d1d5db` | `#111113` / `#1c1c1e` / `#48484a` | the missing-Asset placeholder |
| `--cd-pos` / `--cd-pos-bg` | `#15803d` / `#dcfce7` | `#4ade80` / `#0f2e1a` | positive Action intent |
| `--cd-neu` / `--cd-neu-bg` | `#374151` / `#e5e7eb` | `#d1d5db` / `#2c2c2e` | neutral Action intent |
| `--cd-neg` / `--cd-neg-bg` | `#b91c1c` / `#fee2e2` | `#fca5a5` / `#2a1215` | negative Action intent |
| `--cd-open` / `--cd-res` / `--cd-out` | `#f59e0b` / `#22c55e` / `#9ca3af` | `#fbbf24` / `#4ade80` / `#8e8e93` | Thread status dots: open, resolved, orphaned |
| `--cd-refined` / `--cd-refined-sel` | `#c7d2fe` / `#a5b4fc` | `#818cf8` / `#a5b4fc` | refined highlights |
| `--cd-refined-line` / `--cd-refined-ink` | `#6366f1` / `#4338ca` | `#1e1b4b` / `#a5b4fc` | refined underline; the "now:" quote |
| `--cd-hl` / `--cd-hl-sel` | the Member's light pair | the Member's dark pair | highlights (below) |
| `--cd-hl-ink` | `currentColor` | `#000` | text on a highlight |
| `--cd-hl-link` / `--cd-hl-code-bg` | `#2563eb` / `#f6f7f9` | `#000` / `rgba(0,0,0,.12)` | a link, inline code inside a highlight |
| `--cd-raise` | `#f1f2f5` | `#2c2c2e` | one step above a card: Reply / Resolve pills, rules inside a card, rows in layout C, the current nav pill, the segmented control |
| `--cd-shadow` / `--cd-shadow-pop` | two soft layers / deeper | black, deeper | a card at rest / the selected card and the composer |
| `--cd-mono` | `ui-monospace, SFMono-Regular, Menlo, monospace` | same | code, ids |

The Settings screen uses the same tokens (`--cd-sel-*` for the scope menu, `--cd-amber*` for
Mixed). Shadows stay black at low alpha in both palettes (`0 8px 24px rgba(0,0,0,.15)` for
panels; cards use `--cd-shadow`, a selected card `--cd-shadow-pop` under its ring).

## Highlights

- The Member picks **two** highlight colors, one for light pages and one for dark (Settings →
  Appearance). Presets (`annotary_store::settings::PRESETS`, served in the catalog):

  | Preset | Light | Light selected | Dark | Dark selected |
  |---|---|---|---|---|
  | Yellow (default) | `#fff1a8` | `#ffd54a` | `#facc15` | `#fde047` |
  | Green | `#c8f2d0` | `#86e29b` | `#4ade80` | `#86efac` |
  | Blue | `#cfe3ff` | `#93c1ff` | `#60a5fa` | `#93c5fd` |
  | Pink | `#fbd3e9` | `#f4a6cf` | `#f472b6` | `#f9a8d4` |
  | Orange | `#fed7aa` | `#fdba74` | `#fb923c` | `#fdba74` |
  | Purple | `#e4d4fb` | `#c9a9f6` | `#c084fc` | `#d8b4fe` |

  A custom `#rrggbb` is allowed when the text on it reaches 4.5:1; its selected shade is derived
  (15% toward black on light, 30% toward white on dark).
- The server puts both pairs on `.cd-app` as `--cd-hl-light`, `--cd-hl-light-sel`,
  `--cd-hl-dark`, `--cd-hl-dark-sel`; the palette maps `--cd-hl` / `--cd-hl-sel` to its pair.
- **Dark highlights are bright, solid colors, and the text on them is black** (`#000`), like a
  highlighter on a dark page. Rejected: dark, muted highlights with light text (hard to see on
  black; rounds 2 and 3).
- `mark.cd-hl`: `--cd-hl` ground, 3px radius with a 1px ring of its own color,
  `box-decoration-break: clone`; selected = `--cd-hl-sel` with a 2px ring. Assets get a 3px outline in the same colors; a block-level Thread a 3px left
  rule; in layout C, a strip's left rule.
- **Refined** Threads keep their own indigo (`--cd-refined`) and a 2px dotted underline in both
  appearances, so a refined mark never depends on color alone. Purple stays a preset although it
  is near the indigo, because of that underline (round 1). Every preset keeps a ΔE of at least 12
  from the refined color (a store test).
- A resolved Thread's highlight is gone while Show resolved is off (transparent, text color
  inherited, no pointer).

## Type

- **UI:** `-apple-system, BlinkMacSystemFont, "Segoe UI", Inter, sans-serif`, 15px / 1.5.
- **Document text:** 15px × the Member's **Text size** (`--cd-text-scale`; 80%–200%, 100% =
  15px), line height 1.65, measure `72ch` (scales with the text). Headings 2.05em (1.75em in layout C) / 1.3em / 1.1em, weight 700 and tracked in (-.022em, -.012em, -.006em)
  (h1 line height 1.15); code and `pre` 0.9em in `--cd-mono`. Only the Document text scales; the
  header, cards and lists do not.
- **Cards and panels:** 13px; meta lines 11–12px.
- **Labels:** 11px, uppercase, letter-spacing .06em, weight 600, `--cd-muted` (strip heads, table
  heads, panel titles, the owner screens' keys).
- **Weights:** 400 and 600 (500 for mentions and the Open-in-app chip). No other sizes.

## Spacing, radii, layout

- Header: sticky, `min-height:52px`, padding `8px 20px` (16px in layout C), gap 12px, a bottom `--cd-line` rule.
- **Document tables:** horizontal rules only, the head on a 2px rule, tabular figures.
- Document: padding `24px 0 120px`; layout C pads the page `0 16px`.
- **The 850px rule.** `.cd-app` is a size container (`container-name: cd`). At **850px and
  wider** (layout A) the Document is a grid of the reading column and a **320px margin** (gap
  28px, padding `0 24px`, centered); Thread strips are `position:absolute` in the margin, aligned
  with their highlight by the script and pushed apart (8px gap); hidden until laid out
  (`.cd-ready`), so nothing shifts. **Narrower** (layout C), the same Thread elements sit in the
  flow under their block as one-line rows that expand in place, and the header collapses to one
  line with the rest in a tap-to-open overlay. The same 850px splits the Settings screen (sidebar
  beside the group, or the group list as its own page). iOS uses 900pt.
- Lists: max width 1100px (Document list), 960px (owner screens), 36rem (simple pages).
- Radii: 999px chips, pills, Reply / Resolve and the nav; 8px buttons and fields; 6px code blocks;
  10px layout C rows, toasts, panels, owner-screen boxes; 12px cards and an open row; 4px inline
  code, `kbd` and mention tags; 3px highlights.
- Off-screen blocks and cards from the 61st child of `.cd-doc` on are `content-visibility:auto`
  (except blocks that hold or precede a strip), so a long Document paints fast.

## Components

- **Header (`.cd-hdr`):** the 18 px icon (`.cd-brand`, a link to the Document list) · Vault crumb · Document name (600) · id (mono 12px, hidden under 1600px)
  · Version switcher (`.cd-vsw`: ‹ select ›) · status (`v3 · approved by … · 2 open`) · the
  resolved toggle chip · History and `?` · the Action chip (hidden from 850 to 1600px: the status
  says the same, and the header stays one row) and buttons. The list page and the
  owner screens use the top bar variant (`.cd-hdr-nav`: the icon, `annotary` + nav links, the signed-in
  Member, Sign out), which wraps instead of collapsing.
- **Chips (`.cd-chip`):** pill, 1px `--cd-line`, 12px, `--cd-card`; intent variants `cd-pos`,
  `cd-neu`, `cd-neg` (tinted, no border), `cd-warn`, `cd-none` (muted). A checkbox may sit inside.
- **Buttons (`.cd-btn`):** 1px `--cd-line`, 8px radius, 13px, `--cd-card`, hover `--cd-soft`;
  `.cd-sm` 12px; `.cd-primary` filled `--cd-primary`; Action buttons (`.cd-actbtn`) take the intent
  colors, the positive one filled (`--cd-pos` with `--cd-bg` text, the one thing a reviewer comes to do); a count badge is `--cd-accent` with `--cd-accent-ink`. Text buttons are `.cd-link`
  (dotted underline; a Comment's edit / delete have none, and appear on hover or focus on a pointer
  device, always on a touch screen).
- **Cards (`.cd-thread`):** in layout A, `--cd-card`, 1px `--cd-line`, 12px radius, `--cd-shadow`;
  selected = a 1.5px `--cd-fg` ring and `--cd-shadow-pop`, no movement. Row: status dot (8px; open
  filled, resolved a hollow `--cd-res` ring, orphaned gray), the frozen quote (refined adds "now:"
  in `--cd-refined-ink`), id in mono. Comments separate with a `--cd-raise` rule; the author line
  centers its items and takes a 28px round avatar (`.cd-who > .cd-av`) before the name. Reply and
  Resolve are pills on `--cd-raise`. In layout C a row is a 10px pill on `--cd-raise`, and an open
  row is a card. The editor's textarea is `--cd-field`, 10px, with a 2px accent focus ring; the
  composer has an accent ring. A posting Comment is muted text at full opacity.
- **The tie (`.cd-strip.cd-tied`):** in layout A, the selected card's strip is joined to its
  highlight by a 2px line in `--cd-hl-sel`, the mark's margin note in the page. It leaves the
  column at the highlight's first line and bends down to the card when the margin pushed the card
  lower (`tie()` in `app.js` sets `--cd-tie-a` / `--cd-tie-b` on the strip; on the strip because a
  deep card's `content-visibility` clips outside its box).
- **Mentions (`.cd-mention`):** a resolved `@handle` in a Comment body (server markup,
  `ui::mention_spans`, never in code or links) is a tag in `--cd-sel-bg` / `--cd-sel-ink`; "@you" in
  the status and the list is the same tag, fully rounded.
- **Banners (`.cd-banner`):** full width under the header, 13px: the older-Version banner in the
  note colors, the new-Version banner (`.cd-newver`) in the info colors, each with one button.
- **Errors:** inline `.cd-err` (err colors, 8px radius) inside the card or form that failed; the
  header-level toast (`.cd-toast`) top right, dismissed by a click.
- **Selection pill (`.cd-pill`):** a `--cd-primary` pill with "Comment `c`"; panels (`.cd-panel`:
  History, the keyboard help) are `--cd-card`, 10px radius, shadowed, top right.
- **Owner screens (`admin.css`):** boxes and Member cards on `--cd-card`, the shown-once Token and
  pairing panels in the info colors, forms on `--cd-field`.
- **Compare Versions (`.cd-diffpage`, the end of `app.css`):** the header holds the two Version
  selects (`from → to`), `+N −M lines`, the change count and ‹ › to step; a quiet row under it has the
  legend and Open vA / Open vB. Marks alias the intent tokens (`--cd-dx-*` on `.cd-diffpage`, no new
  colors): added text `--cd-pos-bg` and underlined, removed text `--cd-neg-bg` and struck through, a whole
  added or removed block the same ground with a 3px left rule and a +/− glyph in the gutter, a block
  changed inside a 3px `--cd-amber` rule (dotted when only its markup changed). Never color alone: the
  underline, the strike and the glyph carry it, and every mark has a text label for screen readers.
  Unchanged stretches fold into a dashed `details` line ("12 unchanged blocks"); the current change
  wears a 2px accent outline.

## Member settings: decisions from the mock rounds

From the mock rounds (1–3, 2026-10-29/30):

- **Text in the UI:** no help lines under setting names, no "Default" / "All devices" source
  labels, no explanatory subtitles. Only text that helps.
- **One Settings screen for everyone**, `/settings`, with a sidebar of groups (the same groups,
  same order, as iOS) and a search field that filters by name with no request; the owner's pages
  are its last group, Server. Changes save immediately; there is no Save button. No Mixed badge in the
  sidebar.
- **"Applying to"** is a sentence with a menu ("Applying to: All devices ▾"), as in T3 Code: All
  devices, or one or more Devices (checkbox items); no subtitle under All devices.
- **Mixed, as in T3 Code:** when the chosen Devices disagree the control shows "Mixed" in place
  of a value, the layers icon beside the setting turns amber, and the chain (All devices, then
  each Device with its value, own or following, and Reset on an own value) is one click away.
  When the Devices disagree the All devices line shows no value. "Keep it" was dropped.
- **Highlight color:** two settings, light and dark, each always shown with its own swatches and
  its own sample, "See what's highlighted" with "what's highlighted" marked, on a fixed light or
  dark ground the page's appearance never changes. Black text on the bright dark presets.
- **Text size** has a sample in the same kind of box, "See how the text size looks.", which
  changes size with the setting.
- **Devices:** each row is the kind icon (a computer for Web, a phone, a tablet), the name and
  last seen, nothing else. **Web is one Device** for every browser, with no Rename and no Forget.
- **A new Device asks nothing** (no banner): it gets the All devices values. "Copy from another
  device" is on every device settings page, copies every device setting, and asks first ("Copy
  iPhone 17 Pro's settings to iPad Pro 13″?", Copy / Cancel).
- **The web takes the iOS dark palette** (the iOS app's design notes, decided 8 Oct 2026).
- Comment placement (margin or inline) is not a setting: it stays decided by width.

## Brand: the mark (decided 2026-11-01)

The product's mark is a **margin note**: a page with one highlighted line and a comment dot in its
margin, joined to it. Two mock rounds settled it, and this section is their record: pink over green
and blue because it reads as a highlighter and means nothing else in the product; ink over blue and
paper grounds; a separate small master because the full master's margin dot disappears at 16 px; and
the page's default highlight stays yellow, because changing it would recolor highlights for every
Member who never chose a color.

| Part | Value |
|---|---|
| Ground | ink `#1c1c1e` (the dark card), a full square; corners rounded at 22.4% except where iOS masks them |
| Page | `#f2f2f7`, with four text lines in the ground color at 45% |
| Highlight and dot | the Pink preset's dark shade `#f472b6`. Pink means nothing else in the UI: green is resolved and positive, blue is the accent |
| Small master (32 px and below) | the page, one taller pink bar and a larger dot; no text lines |
| Wordmark | lowercase "annotary", the UI font at weight 600; "Annotary" where a proper name is needed (App Store) |

- **One mark at every size**: the iOS app icon, the favicon set, the Apple touch icon, the web
  manifest icons, the title-bar icon and the logo. The masters are `brand/mark.svg` and
  `brand/mark-small.svg`; every PNG and `.ico` is rendered from them by a script
  (`scripts/brand/render.py`, `make brand`), never exported by hand. `icon.svg` is the small master,
  because a browser draws it in the tab at 16 px.
- **Title bar**: the 18 px icon (small master) first in `.cd-hdr`, linking to the Document list.
  The list's header follows it with the wordmark (`.cd-name`); a Document page follows it with
  the vault crumb and no wordmark.
- **iOS**: an Icon Composer file with a background layer (ink) and two foreground layers (the page
  with its lines; the highlight and dot), set for default, dark, clear and tinted.
- **The icon does not change the UI palette**: the accent stays blue, and the default highlight
  for comments stays Yellow.
- **Sign-in**: the small master at 40 px beside the wordmark (`brand::logo_mark`).

## Motion

Motion says where a thing came from, and nothing moves position (CLS stays 0). Everything below sits
under `prefers-reduced-motion: no-preference`; with reduced motion the end state appears at once.
Easing token: `--cd-ease` = `cubic-bezier(.2,.8,.2,1)`.

| Moment | What moves | Duration |
|---|---|---|
| Selecting a Thread | the tie draws from the column to the card | 220 ms, `--cd-ease` |
| Hover (buttons, nav, rows, cards) | background, ring, shadow | 150 ms |
| A Comment from someone else arrives live (`.cd-fresh`) | its ground fades from `--cd-sel-bg` | 1.2 s ease-out |
| Selecting text | the Comment pill scales in from 96% | 120 ms, `--cd-ease` |

## Visual refresh: decisions (2026-11-08)

Direction "Margin note", from three mock rounds. Josh asked for sleeker, livelier and more striking
within the minimal look, and approved shipping it; the design agent took each round's
recommendation.

- **The one new visual idea is the mark's own:** a selected card is joined to its highlight, in the
  Member's highlight color. Rejected: pink as the accent, a colored header bar, per-Member name
  colors (the brand rule above; a seventh hue).
- **People and agents read alike.** No "AI" badge: the model has no Member kind, and identity comes
  from the avatars.
- **System fonts only.** Rejected: a web font (a request in every page and embed answer, a swap
  against the CLS 0 budget) and a system serif for Documents (differs on every platform).
- **The selected ring is ink.** Rejected: the highlight color (1.4:1 with Yellow, under WCAG 1.4.11)
  and the accent (reads as keyboard focus).

## Keyboard

`j k r e c [ ] ↩ ⌘↩ esc ?` on the Document page (the help panel lists exactly these). The
Keyboard shortcuts setting turns the single keys off (the help hides their rows); the Send key
setting makes `↩` a newline in a reply and `⌘↩` post it. The composer and the edit box post with
`⌘↩` under both. On the Compare page `j` / `k` step to the next / previous change (the same keys,
the same meaning: next and previous item), and Keyboard shortcuts Off turns them off too.

## Times

Relative ages (`now`, `5m`, `3h`, `2d`, `4mo`, `1y`) are rendered by the server. With Times shown
as Absolute and the browser's zone known (the `cd_tz` cookie the script saves), the server renders
`Oct 30, 2:05 PM` (the year only when it is not this year's) in that zone, 12- or 24-hour by the
Clock setting; until the zone is known, times stay relative.
