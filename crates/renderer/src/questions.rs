//! Questions from a Document's decisions table (decision D1 / D2 of the markup-and-questions
//! plan; `docs/dev/questions.md` has the why). Pure, shared by the pushing CLI (which computes a
//! Version's `questions` like its render) and the server (which re-checks them under
//! `--verify-edge` and computes them for Versions an older CLI pushed). It builds for `wasm32`.
//!
//! - The **decisions table** is the first table whose header names the columns `Decision`,
//!   `Options` and `Decided` (case-insensitive, any order; other columns free, `Recommended`
//!   optional). A second one is ignored with a warning.
//! - Each row is a [`Question`]: a key (letters, digits and `-`, at most [`KEY_MAX`], unique) and
//!   [`OPTIONS_MIN`] to [`OPTIONS_MAX`] option ids separated by commas. A malformed row is skipped
//!   with a warning; nothing is guessed.
//! - **Places** ([`find`]): the key's first whole-word use outside the table and outside code is
//!   the Question's place; its section runs to the next Question's place or a heading of the same
//!   or a higher level. In that section, a block (list item, table row, heading, paragraph) whose
//!   text starts with an option id is that Option's place.
//! - [`fill_decided`] writes Answers into empty `Decided` cells (what `pull` hands back) and
//!   changes nothing else; [`check_recorded`] is the rule `push` and the server's commit share.
//!
//! Everything is parsed with pulldown-cmark and [`crate::refs::MARKDOWN_OPTIONS`], never with
//! patterns over raw lines.

use std::collections::BTreeMap;
use std::ops::Range;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde_json::{Map, Value, json};

use crate::html::{Lines, collect, escape_currency_math};
use crate::refs::MARKDOWN_OPTIONS;

/// Longest key or option id.
pub const KEY_MAX: usize = 16;
/// Fewest and most Options a Question has.
pub const OPTIONS_MIN: usize = 2;
pub const OPTIONS_MAX: usize = 6;
/// Most Questions one Document holds (rows past it are ignored with a warning).
pub const QUESTIONS_MAX: usize = 100;
/// The answer that is none of the Options: its words are a Comment on the Question's Thread.
pub const OTHER: &str = "other";
/// Words an Option's block may start with before its id (`Recommended: A.`, `Option B —`).
const LEAD_WORDS: &[&str] = &["recommended", "option", "choice"];

/// A block in the source: its 1-based, inclusive lines (the renderer's `data-l`) and its tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub a: u32,
    pub b: u32,
    /// `p`, `h1`…`h6`, `li`, `tr` or `td`.
    pub block: String,
}

/// Where an Option's own block is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionPlace {
    pub id: String,
    pub place: Place,
}

/// One row of the decisions table, with where it is found in the prose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub key: String,
    /// The row's source line.
    pub row: u32,
    /// The `Decided` column's index (0-based) in the table.
    pub cell: u32,
    pub options: Vec<String>,
    /// The `Recommended` cell, when it names one of the options.
    pub rec: Option<String>,
    /// The `Decided` cell's text when it is not empty: the Question is decided, the cell the truth.
    pub decided: Option<String>,
    /// The key's first block outside the table and outside code; `None` when the prose never names it.
    pub place: Option<Place>,
    /// The Question's section: from its place to the next Question's place or heading.
    pub lines: Option<(u32, u32)>,
    pub places: Vec<OptionPlace>,
    /// A digest of the row (without its `Decided` cell) and the section's text: an Answer given
    /// on a Version whose digest differs is flagged `changed`.
    pub digest: String,
}

impl Question {
    pub fn is_decided(&self) -> bool {
        self.decided.is_some()
    }
    pub fn has_option(&self, id: &str) -> bool {
        self.options.iter().any(|o| o == id)
    }
}

/// What [`find`] answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub questions: Vec<Question>,
    /// One sentence each: a malformed row, a key the prose never names, a second table.
    pub warnings: Vec<String>,
}

/// A key or an option id: letters, digits and `-`, 1 to [`KEY_MAX`] characters.
pub fn is_key(s: &str) -> bool {
    !s.is_empty() && s.len() <= KEY_MAX && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Whether `s` reads as a Thread id (`t7`): the key of a Question asked on a Thread, so a
/// decisions table may not use it.
pub fn is_thread_key(s: &str) -> bool {
    s.len() > 1 && s.starts_with('t') && s[1..].bytes().all(|b| b.is_ascii_digit())
}

/// The cells of one table row: plain text and source range.
#[derive(Clone, Debug)]
struct Cell {
    text: String,
    range: Range<usize>,
}

#[derive(Clone, Debug)]
struct Row {
    range: Range<usize>,
    cells: Vec<Cell>,
}

/// The decisions table, as found in a text.
#[derive(Clone, Debug)]
struct Table {
    range: Range<usize>,
    decision: usize,
    options: usize,
    decided: usize,
    rec: Option<usize>,
    rows: Vec<Row>,
}

/// Every table in `events` that qualifies, in document order.
fn decision_tables(events: &[(Event<'_>, Range<usize>)]) -> Vec<Table> {
    let mut out = Vec::new();
    let mut head: Option<Vec<Cell>> = None;
    let mut rows: Vec<Row> = Vec::new();
    let mut row: Option<Row> = None;
    let mut cell: Option<Cell> = None;
    let mut in_head = false;
    let mut start = 0;
    for (ev, range) in events {
        match ev {
            Event::Start(Tag::Table(_)) => {
                head = None;
                rows.clear();
                start = range.start;
            }
            Event::Start(Tag::TableHead) => {
                in_head = true;
                row = Some(Row { range: range.clone(), cells: Vec::new() });
            }
            Event::Start(Tag::TableRow) => row = Some(Row { range: range.clone(), cells: Vec::new() }),
            Event::Start(Tag::TableCell) => cell = Some(Cell { text: String::new(), range: range.clone() }),
            Event::Text(t) | Event::Code(t) => {
                if let Some(c) = cell.as_mut() {
                    c.text.push_str(t);
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let (Some(c), Some(r)) = (cell.take(), row.as_mut()) {
                    r.cells.push(Cell { text: c.text.trim().to_string(), range: c.range });
                }
            }
            Event::End(TagEnd::TableHead) => {
                in_head = false;
                head = row.take().map(|r| r.cells);
            }
            Event::End(TagEnd::TableRow) if !in_head => {
                if let Some(r) = row.take() {
                    rows.push(r);
                }
            }
            Event::End(TagEnd::Table) => {
                let Some(h) = head.take() else { continue };
                let col = |name: &str| h.iter().position(|c| c.text.eq_ignore_ascii_case(name));
                if let (Some(decision), Some(options), Some(decided)) =
                    (col("decision"), col("options"), col("decided"))
                {
                    out.push(Table {
                        range: start..range.end,
                        decision,
                        options,
                        decided,
                        rec: col("recommended"),
                        rows: std::mem::take(&mut rows),
                    });
                }
                rows.clear();
            }
            _ => {}
        }
    }
    out
}

/// FNV-1a, 64 bits, as 16 hex digits: a change detector, not a security boundary.
fn digest(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h ^= 0xff;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        for b in p.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `key` occurs in `text` as a whole word (`D4.` and `(D4)` do; `D45`, `xD4`, `D4-2` do
/// not).
fn has_word(text: &str, key: &str) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find(key).map(|i| i + from) {
        let before = text[..i].chars().next_back();
        let mut after = text[i + key.len()..].chars();
        let next = after.next();
        let before_ok = match before {
            None => true,
            Some('-') => !text[..i - 1].chars().next_back().is_some_and(is_word),
            Some(c) => !is_word(c),
        };
        let after_ok = match next {
            None => true,
            Some('-') => !after.next().is_some_and(is_word),
            Some(c) => !is_word(c),
        };
        if before_ok && after_ok {
            return true;
        }
        from = i + key.len().max(1);
        while !text.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

/// For each key, the blocks that name it ([`has_word`]). With many keys, one pass over each
/// block's words: a key that [`has_word`] finds starts with a maximal run of word characters, so
/// only a block holding that run is scanned for it. A few keys are cheaper scanned directly.
fn named_in<'b>(blocks: &[&'b Block], keys: &[&str]) -> Vec<Vec<&'b Block>> {
    const DIRECT: usize = 16;
    if keys.len() <= DIRECT {
        return keys.iter().map(|k| blocks.iter().copied().filter(|b| has_word(&b.text, k)).collect()).collect();
    }
    let mut by_run: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut always: Vec<usize> = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        match k.split(|c: char| !is_word(c)).find(|s| !s.is_empty()) {
            Some(run) => by_run.entry(run).or_default().push(i),
            None => always.push(i),
        }
    }
    let mut hits: Vec<Vec<&Block>> = vec![Vec::new(); keys.len()];
    let mut cands: Vec<usize> = Vec::new();
    for b in blocks {
        cands.clear();
        cands.extend_from_slice(&always);
        for w in b.text.split(|c: char| !is_word(c)) {
            if let Some(ix) = by_run.get(w) {
                for &i in ix {
                    if !cands.contains(&i) {
                        cands.push(i);
                    }
                }
            }
        }
        for &i in &cands {
            if has_word(&b.text, keys[i]) {
                hits[i].push(b);
            }
        }
    }
    hits
}

/// Whether a block whose text is `lead` starts with option `id`: after an optional word of
/// [`LEAD_WORDS`] (then any non-word character may follow the id), else followed by `.`, `)`,
/// `:`, a dash or the end, so a paragraph starting "A new VM" is not Option A.
fn leads_with(lead: &str, id: &str) -> bool {
    let mut s = lead.trim_start();
    let mut worded = false;
    for w in LEAD_WORDS {
        if s.len() > w.len() && s.is_char_boundary(w.len()) && s[..w.len()].eq_ignore_ascii_case(w) {
            let rest = &s[w.len()..];
            if rest.starts_with([':', ' ']) {
                s = rest.trim_start_matches(':').trim_start();
                worded = true;
                break;
            }
        }
    }
    let Some(rest) = s.strip_prefix(id) else { return false };
    let mut chars = rest.chars();
    match chars.next() {
        None => true,
        Some('-') if worded => !chars.next().is_some_and(is_word),
        Some(c) if worded => !is_word(c),
        Some('.' | ')' | ':' | '\u{2014}' | '\u{2013}') => true,
        Some(' ') => matches!(chars.next(), Some('\u{2014}' | '\u{2013}' | '-')),
        Some(_) => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Para,
    Heading(u8),
    Item,
    Row,
    Cell,
}

/// A text-holding block met while walking the prose.
#[derive(Clone, Debug)]
struct Block {
    kind: Kind,
    start: usize,
    a: u32,
    b: u32,
    text: String,
    /// The text the block starts with (before a nested block), for option places.
    lead: String,
    /// A nested block has started: `lead` is complete.
    lead_done: bool,
    /// A paragraph directly inside a list item (the item is the candidate, not the paragraph).
    in_item: bool,
    in_head: bool,
}

impl Block {
    fn tag(&self) -> String {
        match self.kind {
            Kind::Para => "p".into(),
            Kind::Heading(l) => format!("h{l}"),
            Kind::Item => "li".into(),
            Kind::Row => "tr".into(),
            Kind::Cell => (if self.in_head { "th" } else { "td" }).into(),
        }
    }
    fn place(&self) -> Place {
        Place { a: self.a, b: self.b, block: self.tag() }
    }
}

/// The blocks of the prose outside `skip` (the decisions table), in the order they close.
fn prose_blocks(md: &str, events: &[(Event<'_>, Range<usize>)], skip: &Range<usize>, lines: &Lines) -> Vec<Block> {
    let mut out = Vec::new();
    let mut stack: Vec<Block> = Vec::new();
    let mut code = 0usize; // inside a code block, metadata or an HTML block
    let mut in_head = false;
    let span = |r: &Range<usize>| {
        let (a, b) = lines.span(md, r);
        (a as u32, b as u32)
    };
    for (ev, range) in events {
        if range.start >= skip.start && range.end <= skip.end && skip.end > skip.start {
            continue;
        }
        let kind = match ev {
            Event::Start(Tag::Paragraph) => Some(Kind::Para),
            Event::Start(Tag::Heading { level, .. }) => Some(Kind::Heading(*level as u8)),
            Event::Start(Tag::Item) => Some(Kind::Item),
            Event::Start(Tag::TableRow) => Some(Kind::Row),
            Event::Start(Tag::TableHead) => {
                in_head = true;
                Some(Kind::Row)
            }
            Event::Start(Tag::TableCell) => Some(Kind::Cell),
            _ => None,
        };
        if let Some(kind) = kind {
            if let Some(top) = stack.last_mut()
                && !(kind == Kind::Para && top.kind == Kind::Item)
                && kind != Kind::Cell
            {
                top.lead_done = true;
            }
            let in_item = kind == Kind::Para && stack.last().is_some_and(|t| t.kind == Kind::Item);
            let (a, b) = span(range);
            stack.push(Block {
                kind,
                start: range.start,
                a,
                b,
                text: String::new(),
                lead: String::new(),
                lead_done: false,
                in_item,
                in_head,
            });
            continue;
        }
        match ev {
            Event::Start(Tag::CodeBlock(_) | Tag::MetadataBlock(_) | Tag::HtmlBlock) => code += 1,
            Event::End(TagEnd::CodeBlock | TagEnd::MetadataBlock(_) | TagEnd::HtmlBlock) => {
                code = code.saturating_sub(1)
            }
            Event::Start(Tag::List(_) | Tag::BlockQuote(_) | Tag::Table(_)) => {
                if let Some(top) = stack.last_mut() {
                    top.lead_done = true;
                }
            }
            Event::Text(t) if code == 0 => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(t);
                    if !top.lead_done {
                        top.lead.push_str(t);
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(top) = stack.last_mut() {
                    top.text.push(' ');
                    if !top.lead_done {
                        top.lead.push(' ');
                    }
                }
            }
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::TableRow
                | TagEnd::TableHead
                | TagEnd::TableCell,
            ) => {
                if matches!(ev, Event::End(TagEnd::TableHead)) {
                    in_head = false;
                }
                let Some(done) = stack.pop() else { continue };
                if let Some(parent) = stack.last_mut() {
                    // A loose item starts with its first paragraph; a row with its first cell.
                    let first = parent.lead.is_empty() && !parent.lead_done;
                    if first && ((done.kind == Kind::Para && parent.kind == Kind::Item) || done.kind == Kind::Cell) {
                        parent.lead = done.lead.clone();
                    }
                }
                out.push(done);
            }
            _ => {}
        }
    }
    out
}

/// Find the decisions table in `md` and each Question's places. See the module docs.
pub fn find(md: &str) -> Found {
    let text = escape_currency_math(md);
    let text: &str = &text;
    let events = collect(text);
    let lines = Lines::new(text);
    let mut found = Found::default();
    let tables = decision_tables(&events);
    let Some(table) = tables.first() else { return found };
    for t in &tables[1..] {
        found.warnings.push(format!(
            "a second decisions table at line {} is ignored: a Document keeps one",
            lines.line_of(t.range.start)
        ));
    }
    fn cell(r: &Row, i: usize) -> &str {
        r.cells.get(i).map(|c| c.text.as_str()).unwrap_or("")
    }
    let mut rows: Vec<(&Row, Question)> = Vec::new();
    for r in &table.rows {
        let line = lines.line_of(r.range.start) as u32;
        let key = cell(r, table.decision);
        let warn = |w: &mut Vec<String>, why: String| w.push(format!("decisions table line {line}: {why}"));
        if key.is_empty() && r.cells.iter().all(|c| c.text.is_empty()) {
            continue;
        }
        if !is_key(key) {
            warn(
                &mut found.warnings,
                format!("{key:?} is not a key (letters, digits and -, at most {KEY_MAX}); row skipped"),
            );
            continue;
        }
        if is_thread_key(key) {
            warn(&mut found.warnings, format!("{key} reads as a Thread id; row skipped"));
            continue;
        }
        if rows.iter().any(|(_, q)| q.key == key) {
            warn(&mut found.warnings, format!("{key} repeats a key; row skipped"));
            continue;
        }
        if rows.len() == QUESTIONS_MAX {
            warn(&mut found.warnings, format!("more than {QUESTIONS_MAX} Questions; {key} and later rows ignored"));
            break;
        }
        let options: Vec<String> =
            cell(r, table.options).split(',').map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
        if let Some(bad) = options.iter().find(|o| !is_key(o) || o.eq_ignore_ascii_case(OTHER)) {
            warn(
                &mut found.warnings,
                format!("{key}: {bad:?} is not an option id (letters, digits and -, not \"other\"); row skipped"),
            );
            continue;
        }
        if options.iter().enumerate().any(|(i, o)| options[..i].contains(o)) {
            warn(&mut found.warnings, format!("{key} repeats an option; row skipped"));
            continue;
        }
        if options.len() < OPTIONS_MIN || options.len() > OPTIONS_MAX {
            warn(
                &mut found.warnings,
                format!(
                    "{key} has {} options; a Question has {OPTIONS_MIN} to {OPTIONS_MAX}; row skipped",
                    options.len()
                ),
            );
            continue;
        }
        let rec = table.rec.map(|i| cell(r, i).to_string()).filter(|s| !s.is_empty());
        let rec = match rec {
            Some(x) if options.contains(&x) => Some(x),
            Some(x) => {
                warn(&mut found.warnings, format!("{key}: Recommended {x:?} is not one of its options"));
                None
            }
            None => None,
        };
        let decided = Some(cell(r, table.decided).to_string()).filter(|s| !s.is_empty());
        rows.push((
            r,
            Question {
                key: key.to_string(),
                row: line,
                cell: table.decided as u32,
                options,
                rec,
                decided,
                place: None,
                lines: None,
                places: Vec::new(),
                digest: String::new(),
            },
        ));
    }
    if rows.is_empty() {
        return found;
    }
    let blocks = prose_blocks(text, &events, &table.range, &lines);
    let table_line = lines.line_of(table.range.start) as u32;
    let last_line = {
        let (_, b) = lines.span(text, &(0..text.len()));
        b as u32
    };
    // Each key's first block (the smallest start; text containers only, never a row).
    let holders: Vec<&Block> = blocks.iter().filter(|b| b.kind != Kind::Row).collect();
    let firsts: Vec<Option<&Block>> = {
        let keys: Vec<&str> = rows.iter().map(|(_, q)| q.key.as_str()).collect();
        named_in(&holders, &keys)
            .into_iter()
            .map(|hits| {
                hits.into_iter()
                    .min_by_key(|b| (b.start, std::cmp::Reverse(b.kind == Kind::Para || b.kind == Kind::Cell)))
            })
            .collect()
    };
    let src: Vec<&str> = text.split('\n').collect();
    let mut by_start: Vec<&Block> = blocks.iter().collect();
    by_start.sort_by_key(|b| b.start);
    for (i, (r, q)) in rows.iter_mut().enumerate() {
        let Some(place) = firsts[i] else {
            found.warnings.push(format!(
                "decisions table line {}: {} is not named in the text; its controls are only in the table",
                q.row, q.key
            ));
            q.digest = digest(&[&row_without(text, r, table.decided), ""]);
            continue;
        };
        let level = match place.kind {
            Kind::Heading(l) => l,
            _ => 6,
        };
        let mut end = last_line + 1;
        for (j, other) in firsts.iter().enumerate() {
            if let Some(o) = other
                && j != i
                && o.start > place.start
            {
                end = end.min(o.a);
            }
        }
        for b in &by_start {
            if let Kind::Heading(l) = b.kind
                && b.start > place.start
                && l <= level
            {
                end = end.min(b.a);
                break;
            }
        }
        if table_line > place.a {
            end = end.min(table_line);
        }
        let mut b_line = end.saturating_sub(1).max(place.b);
        while b_line > place.b && src.get(b_line as usize - 1).is_some_and(|l| l.trim().is_empty()) {
            b_line -= 1;
        }
        let section = (place.a, b_line);
        for id in &q.options {
            let hit = by_start.iter().find(|b| {
                b.start > place.start
                    && b.a >= section.0
                    && b.a <= section.1
                    && matches!(b.kind, Kind::Item | Kind::Heading(_) | Kind::Row | Kind::Para)
                    && !b.in_item
                    && !b.in_head
                    && leads_with(&b.lead, id)
            });
            if let Some(b) = hit {
                q.places.push(OptionPlace { id: id.clone(), place: b.place() });
            }
        }
        let body = src[(section.0 as usize - 1)..(section.1 as usize).min(src.len())].join("\n");
        q.digest = digest(&[&row_without(text, r, table.decided), &body]);
        q.place = Some(place.place());
        q.lines = Some(section);
    }
    found.questions = rows.into_iter().map(|(_, q)| q).collect();
    found
}

/// A row's source text without its `Decided` cell.
fn row_without(text: &str, r: &Row, decided: usize) -> String {
    let src = &text[r.range.clone()];
    match r.cells.get(decided) {
        Some(c) if c.range.start >= r.range.start && c.range.end <= r.range.end => {
            let (s, e) = (c.range.start - r.range.start, c.range.end - r.range.start);
            format!("{}{}", &src[..s], &src[e..]).trim().to_string()
        }
        _ => src.trim().to_string(),
    }
}

/// Write `fills` (key → text) into the empty `Decided` cells of `md`'s decisions table and
/// change nothing else: a filled cell, a row that is not there, or a text with no table is left
/// as it is. Answers how many cells were written. `|` in a text is escaped.
pub fn fill_decided(md: &str, fills: &[(String, String)]) -> (String, usize) {
    let events: Vec<(Event<'_>, Range<usize>)> = Parser::new_ext(md, MARKDOWN_OPTIONS).into_offset_iter().collect();
    let tables = decision_tables(&events);
    let Some(table) = tables.first() else { return (md.to_string(), 0) };
    // (offset, remove up to, insert) in source order.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for r in &table.rows {
        let key = r.cells.get(table.decision).map(|c| c.text.as_str()).unwrap_or("");
        let Some((_, value)) = fills.iter().find(|(k, _)| k == key) else { continue };
        let value = value.replace('|', "\\|");
        let line_end = {
            let s = &md[r.range.clone()];
            r.range.start + s.trim_end_matches(['\n', '\r']).len()
        };
        let real = r.cells.iter().filter(|c| c.range.start < line_end).count();
        match r.cells.get(table.decided) {
            Some(c) if c.range.start < line_end => {
                if !md[c.range.clone()].trim().is_empty() {
                    continue;
                }
                edits.push((c.range.start, c.range.end, format!(" {value} ")));
            }
            _ => {
                // The row stops short of the column: add the cells it lacks.
                let content = &md[r.range.start..line_end];
                let at = r.range.start + content.trim_end().len();
                // Closed when the parser's last cell ends before the line does (a pipe follows
                // it). Never from the source: GFM reads `note \|` and `path\\|` as a cell's `|`.
                let closed = r.cells.iter().rfind(|c| c.range.start < line_end).is_some_and(|c| c.range.end < at);
                let mut add = String::new();
                if !closed {
                    add.push_str(" |");
                }
                for _ in real..table.decided {
                    add.push_str("  |");
                }
                add.push_str(&format!(" {value} |"));
                edits.push((at, at, add));
            }
        }
    }
    if edits.is_empty() {
        return (md.to_string(), 0);
    }
    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(md.len() + 64);
    let mut last = 0;
    for (s, e, v) in &edits {
        out.push_str(&md[last..*s]);
        out.push_str(v);
        last = *e;
    }
    out.push_str(&md[last..]);
    (out, edits.len())
}

/// One Member's current Answer, for [`decided_text`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnswerBrief {
    pub handle: String,
    pub option: String,
    /// ISO 8601; the date is its first ten characters.
    pub at: String,
}

/// What a `Decided` cell records for these Answers (oldest first): one Option as
/// `B (@rowan, 2026-11-19)` (several Members alike: `B (@rowan, @sam, 2026-11-19)`), differing
/// ones each with its Members, `A (@rowan), B (@sam)`. "Other" is `Other: t7` when the Question
/// has a Thread (its words are there). `None` without Answers.
pub fn decided_text(answers: &[AnswerBrief], thread: Option<&str>) -> Option<String> {
    let mut groups: Vec<(&str, Vec<&AnswerBrief>)> = Vec::new();
    for a in answers {
        match groups.iter_mut().find(|(o, _)| *o == a.option) {
            Some((_, g)) => g.push(a),
            None => groups.push((&a.option, vec![a])),
        }
    }
    let name = |o: &str| match (o, thread) {
        (OTHER, Some(t)) => format!("Other: {t}"),
        (OTHER, None) => "Other".to_string(),
        _ => o.to_string(),
    };
    let who = |g: &[&AnswerBrief]| g.iter().map(|a| format!("@{}", a.handle)).collect::<Vec<_>>().join(", ");
    match groups.as_slice() {
        [] => None,
        [(o, g)] => {
            let date = g.iter().map(|a| a.at.get(..10).unwrap_or(&a.at)).max().unwrap_or("");
            Some(format!("{} ({}, {date})", name(o), who(g)))
        }
        many => Some(many.iter().map(|(o, g)| format!("{} ({})", name(o), who(g))).collect::<Vec<_>>().join(", ")),
    }
}

/// A Question whose Answers a pushed file leaves out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unrecorded {
    pub key: String,
    /// The row's line in the pushed file.
    pub row: u32,
    /// The row as it should read, with the Answers in its `Decided` cell.
    pub text: String,
}

/// What [`check_recorded`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    /// Rows to put the Answers in before the push can go: `push` refuses (`answers_missing`).
    pub missing: Vec<Unrecorded>,
    /// Questions let through without their Answers: the row was removed or its Options changed.
    pub notes: Vec<String>,
}

/// The rule `push` and the server's commit share (decision D2): for each Question of the head
/// (`head`) that is undecided there and has Answers (`texts`: key → [`decided_text`]), the new
/// file (`md`, its Questions `local`) must hold a decision in that row's `Decided` cell, unless
/// the row was removed or its Options changed (a note each). Any words in the cell count: the
/// author may settle two differing Answers by writing one.
pub fn check_recorded(head: &[Question], md: &str, local: &[Question], texts: &[(String, String)]) -> Recorded {
    let mut out = Recorded::default();
    let mut fills: Vec<(String, String)> = Vec::new();
    let mut rows: Vec<(String, u32)> = Vec::new();
    for (key, text) in texts {
        let Some(h) = head.iter().find(|q| &q.key == key) else { continue };
        if h.is_decided() {
            continue;
        }
        match local.iter().find(|q| &q.key == key) {
            None => out.notes.push(format!("{key}'s row was removed; its answers ({text}) are not recorded")),
            Some(l) if l.options != h.options => {
                out.notes.push(format!("{key}'s options changed; its answers ({text}) are not recorded"))
            }
            Some(l) if l.is_decided() => {}
            Some(l) => {
                fills.push((key.clone(), text.clone()));
                rows.push((key.clone(), l.row));
            }
        }
    }
    if rows.is_empty() {
        return out;
    }
    // One fill for every missing row: filling never adds a line, so each row keeps its number.
    let (filled, _) = fill_decided(md, &fills);
    let lines: Vec<&str> = filled.split('\n').collect();
    for (key, row) in rows {
        let text = lines.get(row as usize - 1).copied().unwrap_or("").trim_end_matches('\r');
        out.missing.push(Unrecorded { key, row, text: text.to_string() });
    }
    out
}

/// The Questions as stored on a Version and sent with a commit.
pub fn to_json(questions: &[Question]) -> Value {
    Value::Array(questions.iter().map(question_json).collect())
}

fn place_json(p: &Place) -> Value {
    json!({ "a": p.a, "b": p.b, "block": p.block })
}

fn question_json(q: &Question) -> Value {
    json!({
        "key": q.key,
        "row": q.row,
        "cell": q.cell,
        "options": q.options,
        "rec": q.rec,
        "decided": q.decided,
        "place": q.place.as_ref().map(place_json),
        "lines": q.lines.map(|(a, b)| json!([a, b])),
        "places": q.places.iter().map(|p| {
            let mut m = Map::new();
            m.insert("id".into(), json!(p.id));
            m.insert("a".into(), json!(p.place.a));
            m.insert("b".into(), json!(p.place.b));
            m.insert("block".into(), json!(p.place.block));
            Value::Object(m)
        }).collect::<Vec<_>>(),
        "digest": q.digest,
    })
}

const BLOCKS: &[&str] = &["p", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr", "td", "th"];

fn line_of(v: &Value) -> Option<u32> {
    v.as_u64().filter(|n| *n >= 1 && *n <= u32::MAX as u64).map(|n| n as u32)
}

fn place_of(v: &Value) -> Option<Place> {
    let (a, b) = (line_of(&v["a"])?, line_of(&v["b"])?);
    let block = v["block"].as_str().filter(|s| BLOCKS.contains(s))?;
    (b >= a).then(|| Place { a, b, block: block.to_string() })
}

/// Read stored or sent Questions back; `None` when the JSON is not of the shape [`to_json`] writes.
pub fn from_json(v: &Value) -> Option<Vec<Question>> {
    let arr = v.as_array()?;
    if arr.len() > QUESTIONS_MAX {
        return None;
    }
    let mut out: Vec<Question> = Vec::with_capacity(arr.len());
    for q in arr {
        let key = q["key"].as_str().filter(|k| is_key(k) && !is_thread_key(k))?;
        if out.iter().any(|o| o.key == key) {
            return None;
        }
        let options: Vec<String> = q["options"]
            .as_array()?
            .iter()
            .map(|o| o.as_str().filter(|s| is_key(s)).map(str::to_string))
            .collect::<Option<_>>()?;
        if options.len() < OPTIONS_MIN || options.len() > OPTIONS_MAX {
            return None;
        }
        let opt = |k: &str| -> Option<Option<String>> {
            match &q[k] {
                Value::Null => Some(None),
                Value::String(s) => Some(Some(s.clone())),
                _ => None,
            }
        };
        let rec = opt("rec")?;
        if rec.as_ref().is_some_and(|r| !options.contains(r)) {
            return None;
        }
        let decided = opt("decided")?.filter(|d| !d.trim().is_empty());
        let place = match &q["place"] {
            Value::Null => None,
            p => Some(place_of(p)?),
        };
        let lines = match &q["lines"] {
            Value::Null => None,
            Value::Array(l) if l.len() == 2 => {
                let (a, b) = (line_of(&l[0])?, line_of(&l[1])?);
                (b >= a).then_some((a, b))
            }
            _ => return None,
        };
        let mut places = Vec::new();
        for p in q["places"].as_array()? {
            let id = p["id"].as_str().filter(|i| options.iter().any(|o| o == i))?;
            places.push(OptionPlace { id: id.to_string(), place: place_of(p)? });
        }
        let digest = q["digest"].as_str().filter(|d| d.len() == 16 && d.bytes().all(|b| b.is_ascii_hexdigit()))?;
        out.push(Question {
            key: key.to_string(),
            row: line_of(&q["row"])?,
            cell: q["cell"].as_u64().filter(|c| *c < 64)? as u32,
            options,
            rec,
            decided,
            place,
            lines,
            places,
            digest: digest.to_string(),
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "# Plan\n\
\n\
| Decision | Options | Recommended | Decided |\n\
|---|---|---|---|\n\
| D1 | A, B | A | |\n\
| D4 | A, B, C | B | |\n\
\n\
## D1. Where the service runs\n\
\n\
- Recommended: A. The existing box.\n\
- B. A new VM.\n\
\n\
## D4. Which region\n\
\n\
**Option A** is the cheapest.\n\
\n\
1. B) eu-west\n\
2. C: us-west\n\
\n\
A new paragraph that is not Option A.\n";

    fn q<'a>(f: &'a Found, key: &str) -> &'a Question {
        f.questions.iter().find(|q| q.key == key).unwrap()
    }

    // spec: S50-01, S50-02
    #[test]
    fn finds_the_table_and_the_places() {
        let f = find(PLAN);
        assert_eq!(f.warnings, Vec::<String>::new());
        let d1 = q(&f, "D1");
        assert_eq!((d1.row, d1.cell), (5, 3));
        assert_eq!(d1.options, ["A", "B"]);
        assert_eq!(d1.rec.as_deref(), Some("A"));
        assert_eq!(d1.decided, None);
        assert_eq!(d1.place, Some(Place { a: 8, b: 8, block: "h2".into() }));
        assert_eq!(d1.lines, Some((8, 11)));
        let ids: Vec<(&str, u32, &str)> =
            d1.places.iter().map(|p| (p.id.as_str(), p.place.a, p.place.block.as_str())).collect();
        assert_eq!(ids, [("A", 10, "li"), ("B", 11, "li")]);
        let d4 = q(&f, "D4");
        assert_eq!(d4.lines, Some((13, 20)));
        let ids: Vec<(&str, u32, &str)> =
            d4.places.iter().map(|p| (p.id.as_str(), p.place.a, p.place.block.as_str())).collect();
        assert_eq!(ids, [("A", 15, "p"), ("B", 17, "li"), ("C", 18, "li")]);
    }

    // spec: S50-02
    #[test]
    fn many_keys_find_the_same_blocks_as_a_direct_scan() {
        let texts = [
            "D4.",
            "(D4)",
            "D45",
            "xD4",
            "D4-2",
            "—D4—",
            "é D4",
            "éD4",
            "us-east is near",
            "us east",
            "-D4",
            "a-D4",
            "K-1 here",
            "_D4",
        ];
        let blocks: Vec<Block> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| Block {
                kind: Kind::Para,
                start: i,
                a: 1,
                b: 1,
                text: t.to_string(),
                lead: String::new(),
                lead_done: false,
                in_item: false,
                in_head: false,
            })
            .collect();
        let refs: Vec<&Block> = blocks.iter().collect();
        let mut keys = vec!["D4", "us-east", "us", "K-1", "D45", "x"];
        let pad: Vec<String> = (0..20).map(|i| format!("Z{i}")).collect();
        keys.extend(pad.iter().map(String::as_str));
        let starts =
            |v: Vec<Vec<&Block>>| v.iter().map(|h| h.iter().map(|b| b.start).collect::<Vec<_>>()).collect::<Vec<_>>();
        let direct: Vec<Vec<usize>> =
            keys.iter().map(|k| refs.iter().filter(|b| has_word(&b.text, k)).map(|b| b.start).collect()).collect();
        assert!(keys.len() > 16);
        assert_eq!(starts(named_in(&refs, &keys)), direct);
        assert_eq!(direct[0], [0, 1, 5, 6, 10]);
    }

    // spec: S50-02
    #[test]
    fn whole_words_only() {
        assert!(has_word("see D4.", "D4"));
        assert!(has_word("(D4)", "D4"));
        assert!(has_word("D4", "D4"));
        assert!(!has_word("D45", "D4"));
        assert!(!has_word("xD4", "D4"));
        assert!(!has_word("D4-2", "D4"));
        assert!(has_word("D4 - and", "D4"));
        assert!(has_word("é D4 é", "D4"));
        assert!(!has_word("éD4", "D4"));
    }

    // spec: S50-02
    #[test]
    fn option_leads() {
        assert!(leads_with("A. The box", "A"));
        assert!(leads_with("Recommended: A. The box", "A"));
        assert!(leads_with("Option A is cheap", "A"));
        assert!(leads_with("B) eu", "B"));
        assert!(leads_with("C — us", "C"));
        assert!(leads_with("C", "C"));
        assert!(!leads_with("A new VM", "A"));
        assert!(!leads_with("AB. x", "A"));
        assert!(!leads_with("Options A", "A"));
        // A lead word, then an id that is only the start of a longer one.
        assert!(!leads_with("Option us-east is faster", "us"));
        assert!(leads_with("Option us-east is faster", "us-east"));
        assert!(leads_with("Option A - desc", "A"));
        assert!(leads_with("Option A — desc", "A"));
        assert!(leads_with("Option A-", "A"));
    }

    // spec: S50-02
    #[test]
    fn code_and_the_table_do_not_count() {
        let md = "| decision | options | decided |\n|---|---|---|\n| K1 | x, y | |\n\n`K1` in code\n\n```\nK1\n```\n\nThen K1 here.\n";
        let f = find(md);
        assert_eq!(q(&f, "K1").place, Some(Place { a: 11, b: 11, block: "p".into() }));
    }

    // spec: S50-01
    #[test]
    fn bad_rows_are_warned_and_skipped() {
        let md = "| Decided | Options | Decision | Recommended |\n|---|---|---|---|\n\
| | A | Q1 | |\n| | A, B | Q1 | |\n| | A, B | Q1 | |\n| | A, other | Q2 | |\n| | A, A | Q3 | |\n| | A, B | not a key | |\n| | A, B | t7 | |\n| | A, B | Q4 | C |\n| x | A,B | Q5 | |\n";
        let f = find(md);
        let keys: Vec<&str> = f.questions.iter().map(|q| q.key.as_str()).collect();
        assert_eq!(keys, ["Q1", "Q4", "Q5"]);
        let w = f.warnings.join("\n");
        assert!(w.contains("line 3: Q1 has 1 options"), "{w}");
        assert!(w.contains("line 5: Q1 repeats a key"), "{w}");
        assert!(w.contains("\"other\""), "{w}");
        assert!(w.contains("Q3 repeats an option"), "{w}");
        assert!(w.contains("\"not a key\" is not a key"), "{w}");
        assert!(w.contains("Recommended \"C\""), "{w}");
        assert!(w.contains("t7 reads as a Thread id"), "{w}");
        assert!(w.contains("Q4 is not named in the text"), "{w}");
        assert_eq!(q(&f, "Q5").decided.as_deref(), Some("x"));
        assert_eq!(q(&f, "Q4").cell, 0);
    }

    // spec: S50-01
    #[test]
    fn one_table_per_document() {
        let t = "| Decision | Options | Decided |\n|---|---|---|\n| A1 | x, y | |\n";
        let f = find(&format!("{t}\nA1 here\n\n{t}"));
        assert_eq!(f.questions.len(), 1);
        assert!(f.warnings[0].contains("second decisions table at line 7"), "{:?}", f.warnings);
        assert_eq!(find("| a | b |\n|---|---|\n| 1 | 2 |\n"), Found::default());
    }

    // spec: S50-05
    #[test]
    fn fill_changes_only_empty_decided_cells() {
        let fills = vec![("D1".to_string(), "A (@rowan, 2026-11-19)".to_string()), ("D4".into(), "B | C".into())];
        let (out, n) = fill_decided(PLAN, &fills);
        assert_eq!(n, 2);
        let want = PLAN
            .replace("| D1 | A, B | A | |", "| D1 | A, B | A | A (@rowan, 2026-11-19) |")
            .replace("| D4 | A, B, C | B | |", "| D4 | A, B, C | B | B \\| C |");
        assert_eq!(out, want);
        // Filled again: nothing changes (the cells are no longer empty).
        assert_eq!(fill_decided(&out, &fills), (out.clone(), 0));
        let f = find(&out);
        assert_eq!(q(&f, "D1").decided.as_deref(), Some("A (@rowan, 2026-11-19)"));
        // The digests ignore the Decided cell.
        assert_eq!(q(&f, "D1").digest, q(&find(PLAN), "D1").digest);
    }

    // spec: S50-05
    #[test]
    fn fill_handles_short_and_tight_rows() {
        let md = "| Decision | Options | Decided |\n|---|---|---|\n|D1|A,B||\n| D2 | A, B |\n| D3 | A, B\n";
        let fills: Vec<(String, String)> =
            ["D1", "D2", "D3"].iter().map(|k| (k.to_string(), "A".to_string())).collect();
        let (out, n) = fill_decided(md, &fills);
        assert_eq!(n, 3);
        assert_eq!(
            out,
            "| Decision | Options | Decided |\n|---|---|---|\n|D1|A,B| A |\n| D2 | A, B | A |\n| D3 | A, B | A |\n"
        );
        let f = find(&out);
        assert!(f.questions.iter().all(|q| q.decided.as_deref() == Some("A")), "{f:?}");
        // A short row ending in an escaped pipe is still open, after any number of backslashes
        // (GFM escapes the pipe before backslashes are read).
        let esc = "| Decision | Options | Note | Decided |\n|---|---|---|---|\n| D1 | A, B | note \\|\n| D2 | A, B | path\\\\|\n| D3 | A, B | x |\n";
        let (out, n) = fill_decided(esc, &fills);
        assert_eq!(n, 3);
        assert_eq!(
            out,
            "| Decision | Options | Note | Decided |\n|---|---|---|---|\n| D1 | A, B | note \\| | A |\n| D2 | A, B | path\\\\| | A |\n| D3 | A, B | x | A |\n"
        );
        let f = find(&out);
        assert!(f.questions.iter().all(|q| q.decided.as_deref() == Some("A")), "{f:?}");
        // CRLF rows keep their line ends.
        let crlf = md.replace('\n', "\r\n");
        let (out, _) = fill_decided(&crlf, &fills);
        assert_eq!(
            out,
            "| Decision | Options | Decided |\r\n|---|---|---|\r\n|D1|A,B| A |\r\n| D2 | A, B | A |\r\n| D3 | A, B | A |\r\n"
        );
    }

    // spec: S50-05
    #[test]
    fn decided_texts() {
        let a = |h: &str, o: &str, at: &str| AnswerBrief { handle: h.into(), option: o.into(), at: at.into() };
        assert_eq!(decided_text(&[], None), None);
        assert_eq!(
            decided_text(&[a("rowan", "B", "2026-11-19T10:00:00.000Z")], None).as_deref(),
            Some("B (@rowan, 2026-11-19)")
        );
        assert_eq!(
            decided_text(&[a("rowan", "B", "2026-11-18T10:00:00Z"), a("sam", "B", "2026-11-19T10:00:00Z")], None)
                .as_deref(),
            Some("B (@rowan, @sam, 2026-11-19)")
        );
        assert_eq!(
            decided_text(&[a("rowan", "A", "2026-11-18"), a("sam", "B", "2026-11-19")], None).as_deref(),
            Some("A (@rowan), B (@sam)")
        );
        assert_eq!(
            decided_text(&[a("rowan", OTHER, "2026-11-19")], Some("t7")).as_deref(),
            Some("Other: t7 (@rowan, 2026-11-19)")
        );
    }

    // spec: S50-06
    #[test]
    fn the_recording_rule() {
        let head = find(PLAN).questions;
        let texts = vec![("D1".to_string(), "A (@rowan, 2026-11-19)".to_string())];
        // Unchanged file: refused, with the row to put in.
        let r = check_recorded(&head, PLAN, &head, &texts);
        assert_eq!(
            r.missing,
            [Unrecorded { key: "D1".into(), row: 5, text: "| D1 | A, B | A | A (@rowan, 2026-11-19) |".into() }]
        );
        // Filled (with anything): passes.
        let md = PLAN.replace("| D1 | A, B | A | |", "| D1 | A, B | A | B, after all |");
        assert_eq!(check_recorded(&head, &md, &find(&md).questions, &texts), Recorded::default());
        // Options changed or the row removed: passes with a note.
        let md = PLAN.replace("| D1 | A, B | A | |", "| D1 | A, B, C | A | |");
        let r = check_recorded(&head, &md, &find(&md).questions, &texts);
        assert!(r.missing.is_empty() && r.notes[0].contains("options changed"), "{r:?}");
        let md = PLAN.replace("| D1 | A, B | A | |\n", "");
        let r = check_recorded(&head, &md, &find(&md).questions, &texts);
        assert!(r.missing.is_empty() && r.notes[0].contains("row was removed"), "{r:?}");
    }

    // spec: S50-01
    #[test]
    fn json_round_trips_and_refuses_bad_shapes() {
        let qs = find(PLAN).questions;
        let v = to_json(&qs);
        assert_eq!(from_json(&v), Some(qs));
        let mut bad = v.clone();
        bad[0]["options"] = json!(["A"]);
        assert_eq!(from_json(&bad), None);
        let mut bad = v.clone();
        bad[0]["rec"] = json!("Z");
        assert_eq!(from_json(&bad), None);
        let mut bad = v.clone();
        bad[0]["place"]["block"] = json!("script");
        assert_eq!(from_json(&bad), None);
        let mut bad = v;
        bad[1]["key"] = json!("D1");
        assert_eq!(from_json(&bad), None);
        assert_eq!(from_json(&json!({})), None);
    }
}
