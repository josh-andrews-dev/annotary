//! The rendered diff of two Versions (the web UI's Compare page): the newer Version's rendered
//! HTML with what changed since the older one marked **inline**, so headings, lists, tables and
//! code still render as they do on the Document page.
//!
//! **Input** is the renderer's output for both Versions (`docs/dev/rendering.md`, "HTML
//! contract"): a regular fragment whose blocks are lowercase elements, text escaped, raw HTML
//! sanitized. [`parse`] reads it into a tree of block elements (`BLOCK`); everything between
//! blocks is an inline run, kept verbatim.
//!
//! **Algorithm.** Sibling blocks are diffed as a sequence (Myers over each block's markup with
//! `data-l` and `id` dropped, so a block that only moved lines is equal). Inside a run of changed
//! siblings, old and new blocks of the same element are paired when their words are similar
//! enough (an LCS over the run, so order is kept); an unpaired old block is **removed**, an
//! unpaired new one **added**, each marked whole. A pair of lists, tables (sections, rows) or
//! containers whose both sides hold blocks is diffed again one level down, so an item, a row or a
//! cell is the unit there and no text ever lands where a list or table allows none. Any other
//! pair is diffed **word by word**: tokens are tags, whole media / math elements, HTML entities
//! and UAX #29 words; the output keeps the newer side's tags (every tag of the newer block, in
//! order, so it is balanced), puts deleted words in `del.cd-del` and inserted ones in
//! `ins.cd-ins`. A pair whose words are equal but whose markup differs is a formatting change.
//!
//! Every marked block gets `cd-dx-add` / `cd-dx-del` / `cd-dx-mod` and a visually hidden label
//! (`span.cd-sr`), so the marks never rest on color alone. The first mark of each run of
//! adjacent changes carries `data-cd-dx="N"` (1-based, document order): the page steps through
//! those. Long runs of unchanged top-level blocks are folded into a closed
//! `details.cd-dx-same`, [`CONTEXT`] blocks kept open on each side of a change.
//!
//! Everything here is a pure function of the four texts (no clock, no randomness).

use unicode_segmentation::UnicodeSegmentation;

use crate::mapping::equal_str_runs;

/// Unchanged top-level blocks kept visible on each side of a change.
pub const CONTEXT: usize = 2;
/// A fold hides at least this many blocks (fewer stay visible).
pub const MIN_FOLD: usize = 3;
/// Above this many old × new blocks in one changed run, nothing is paired (each side is shown
/// whole): pairing costs a word diff per pair.
const MAX_PAIRING: usize = 400;
/// A run of changed text above this many tokens (old + new) is not diffed word by word.
const MAX_WORD_TOKENS: usize = 20_000;

/// What a diff found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffStats {
    /// Markdown source lines added / removed (histogram line diff).
    pub lines_added: usize,
    pub lines_removed: usize,
    /// Blocks marked whole as added / removed, and blocks changed inside.
    pub blocks_added: usize,
    pub blocks_removed: usize,
    pub blocks_changed: usize,
    /// Change points (`data-cd-dx`): runs of adjacent marks.
    pub changes: usize,
}

/// A rendered diff: the HTML fragment and its numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedDiff {
    pub html: String,
    pub stats: DiffStats,
}

/// Diff Version A (`old_md`, rendered as `old_html`) against Version B (`new_md`, `new_html`).
pub fn rendered_diff(old_md: &str, new_md: &str, old_html: &str, new_html: &str) -> RenderedDiff {
    let (lines_added, lines_removed) = line_counts(old_md, new_md);
    let old = parse(old_html);
    let new = parse(new_html);
    let mut cx = Cx { stats: DiffStats { lines_added, lines_removed, ..DiffStats::default() }, open: false };
    let pieces = cx.seq(&old, &new, None);
    RenderedDiff { html: fold(&pieces), stats: cx.stats }
}

/// Lines added and removed between two Markdown texts (histogram line diff, CRLF tolerated).
pub fn line_counts(old: &str, new: &str) -> (usize, usize) {
    use imara_diff::{Algorithm, Diff, Interner};
    let (a, b) = (annotary_sidecar::range::lines(old), annotary_sidecar::range::lines(new));
    let mut interner: Interner<&str> = Interner::new(a.len() + b.len());
    let before: Vec<_> = a.iter().map(|l| interner.intern(l)).collect();
    let after: Vec<_> = b.iter().map(|l| interner.intern(l)).collect();
    let mut diff = Diff::default();
    diff.compute_with(Algorithm::Histogram, &before, &after, interner.num_tokens());
    diff.hunks().fold((0, 0), |(add, del), h| {
        (add + (h.after.end - h.after.start) as usize, del + (h.before.end - h.before.start) as usize)
    })
}

// ---------------------------------------------------------------------------------------------
// The tree

/// Block-level elements the renderer (and the sanitizer's raw HTML) can open. `math` is a block
/// only with `display="block"`.
const BLOCK: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "th",
    "td",
    "hr",
    "section",
    "dl",
    "dt",
    "dd",
    "div",
    "figure",
    "details",
];
/// Elements whose children are restricted (no text, no wrapper): diffed child by child.
const RESTRICTED: &[&str] = &["ul", "ol", "dl", "table", "thead", "tbody", "tfoot", "tr"];
/// Void elements: no close tag.
const VOID: &[&str] = &["area", "br", "col", "embed", "hr", "img", "input", "source", "track", "wbr"];
/// Inline elements compared and shown whole (their content is not words to diff).
const ATOM: &[&str] = &["math", "video", "audio", "object", "svg", "picture", "iframe"];

#[derive(Debug)]
enum Node<'a> {
    El(El<'a>),
    /// An inline run between blocks (text and inline markup), verbatim.
    Text(&'a str),
}

#[derive(Debug)]
struct El<'a> {
    name: String,
    open: &'a str,
    inner: &'a str,
    close: &'a str,
    outer: &'a str,
    kids: Vec<Node<'a>>,
    /// Compared whole, never diffed inside (block math).
    atomic: bool,
}

impl El<'_> {
    fn has_blocks(&self) -> bool {
        self.kids.iter().any(|k| matches!(k, Node::El(_)))
    }
}

/// Where a tag ends (after `>`), honoring quoted attribute values.
fn tag_end(s: &str, from: usize) -> usize {
    let b = s.as_bytes();
    let mut i = from;
    let mut quote = 0u8;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return i + 1;
        }
        i += 1;
    }
    b.len()
}

/// The lowercase element name at `s[from..]` (after `<` or `</`).
fn name_at(s: &str, from: usize) -> String {
    s[from..]
        .bytes()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'-')
        .map(|c| c.to_ascii_lowercase() as char)
        .collect()
}

fn is_block_open(name: &str, tag: &str) -> bool {
    BLOCK.contains(&name) || (name == "math" && tag.contains("display=\"block\""))
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

/// Parse a rendered fragment into its top-level nodes.
fn parse(html: &str) -> Vec<Node<'_>> {
    let mut p = Parser { s: html, i: 0 };
    p.children(&[]).0
}

impl<'a> Parser<'a> {
    /// Children up to the close tag of `stack`'s last element (or an ancestor's, which is left
    /// for it, or the end). Returns the children and the close tag's byte range.
    fn children(&mut self, stack: &[String]) -> (Vec<Node<'a>>, (usize, usize)) {
        let s = self.s;
        let mut out: Vec<Node<'a>> = Vec::new();
        let mut text_start = self.i;
        let flush = |out: &mut Vec<Node<'a>>, a: usize, b: usize| {
            if b > a {
                out.push(Node::Text(&s[a..b]));
            }
        };
        let close = loop {
            let Some(k) = s[self.i..].find('<').map(|k| self.i + k) else {
                self.i = s.len();
                break (s.len(), s.len());
            };
            if s[k..].starts_with("<!--") {
                self.i = s[k..].find("-->").map(|p| k + p + 3).unwrap_or(s.len());
                continue;
            }
            if s[k..].starts_with("</") {
                let name = name_at(s, k + 2);
                let end = tag_end(s, k);
                if BLOCK.contains(&name.as_str()) || name == "math" {
                    if stack.last() == Some(&name) {
                        self.i = end;
                        break (k, end);
                    }
                    if stack.contains(&name) {
                        self.i = k;
                        break (k, k);
                    }
                }
                self.i = end; // an inline close tag, or a stray one: part of the text
                continue;
            }
            let name = name_at(s, k + 1);
            if name.is_empty() || !s.as_bytes()[k + 1].is_ascii_alphabetic() {
                self.i = k + 1;
                continue;
            }
            let end = tag_end(s, k);
            let tag = &s[k..end];
            if !is_block_open(&name, tag) {
                self.i = end;
                continue;
            }
            flush(&mut out, text_start, k);
            let el = if VOID.contains(&name.as_str()) || tag.ends_with("/>") {
                self.i = end;
                El { name, open: tag, inner: "", close: "", outer: tag, kids: Vec::new(), atomic: false }
            } else if name == "pre" || name == "math" {
                // Raw content: no block inside a `pre`, and block math is compared whole.
                let pat = format!("</{name}");
                let (cs, ce) = match s[end..].find(&pat) {
                    Some(p) => (end + p, tag_end(s, end + p)),
                    None => (s.len(), s.len()),
                };
                self.i = ce;
                let inner = &s[end..cs];
                let atomic = name == "math";
                El {
                    name,
                    open: tag,
                    inner,
                    close: &s[cs..ce],
                    outer: &s[k..ce],
                    kids: vec![Node::Text(inner)],
                    atomic,
                }
            } else {
                self.i = end;
                let mut st = stack.to_vec();
                st.push(name.clone());
                let (kids, (cs, ce)) = self.children(&st);
                El { name, open: tag, inner: &s[end..cs], close: &s[cs..ce], outer: &s[k..ce], kids, atomic: false }
            };
            let mut el = el;
            if el.has_blocks() {
                // Whitespace between blocks is layout, not content.
                el.kids.retain(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
            }
            out.push(Node::El(el));
            text_start = self.i;
        };
        flush(&mut out, text_start, close.0);
        if out.iter().any(|n| matches!(n, Node::El(_))) {
            out.retain(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
        }
        (out, close)
    }
}

/// Markup without ` data-l="…"` and ` id="…"` (attribute values are escaped, so both only ever
/// appear inside a tag): what two blocks are compared by, and what a removed block shows.
fn strip_attrs(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let a = rest.find(" data-l=\"");
        let b = rest.find(" id=\"");
        let (pos, skip) = match (a, b) {
            (Some(a), Some(b)) if a < b => (a, 9),
            (Some(a), None) => (a, 9),
            (_, Some(b)) => (b, 5),
            (None, None) => break,
        };
        out.push_str(&rest[..pos]);
        let after = &rest[pos + skip..];
        rest = match after.find('"') {
            Some(q) => &after[q + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn key(n: &Node) -> String {
    match n {
        Node::El(e) => strip_attrs(e.outer),
        Node::Text(t) => t.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

// ---------------------------------------------------------------------------------------------
// Word tokens

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Tag,
    /// Shown and compared whole (media, math, an entity-free word is `Text`).
    Atom,
    Text,
}

#[derive(Debug)]
struct Tok<'a> {
    s: &'a str,
    kind: Kind,
}

impl Tok<'_> {
    fn key(&self) -> std::borrow::Cow<'_, str> {
        match self.kind {
            Kind::Text => std::borrow::Cow::Borrowed(self.s),
            _ => std::borrow::Cow::Owned(strip_attrs(self.s)),
        }
    }
    /// A word or a medium: what pairing compares.
    fn is_word(&self) -> bool {
        self.kind == Kind::Atom || (self.kind == Kind::Text && self.s.chars().any(char::is_alphanumeric))
    }
    /// Anything a reader sees (punctuation too): what gets marked.
    fn visible(&self) -> bool {
        self.kind == Kind::Atom || (self.kind == Kind::Text && !self.s.trim().is_empty())
    }
}

/// An inline fragment as tokens: tags, whole media / math, entities, words, spaces, punctuation.
fn tokens(s: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'<' {
            if s[i..].starts_with("<!--") {
                let e = s[i..].find("-->").map(|p| i + p + 3).unwrap_or(s.len());
                out.push(Tok { s: &s[i..e], kind: Kind::Tag });
                i = e;
                continue;
            }
            let e = tag_end(s, i);
            let closing = s[i..].starts_with("</");
            let name = name_at(s, i + if closing { 2 } else { 1 });
            if !closing && ATOM.contains(&name.as_str()) && !s[i..e].ends_with("/>") {
                let pat = format!("</{name}");
                let e2 = s[e..].find(&pat).map(|p| tag_end(s, e + p)).unwrap_or(s.len());
                out.push(Tok { s: &s[i..e2], kind: Kind::Atom });
                i = e2;
                continue;
            }
            let kind = if !closing && VOID.contains(&name.as_str()) { Kind::Atom } else { Kind::Tag };
            out.push(Tok { s: &s[i..e], kind });
            i = e;
            continue;
        }
        let e = s[i..].find('<').map(|p| i + p).unwrap_or(s.len());
        text_tokens(&s[i..e], &mut out);
        i = e;
    }
    out
}

/// Text between tags: entities whole, the rest by UAX #29 word boundaries.
fn text_tokens<'a>(t: &'a str, out: &mut Vec<Tok<'a>>) {
    let mut rest = t;
    while !rest.is_empty() {
        let amp = rest.find('&');
        let (plain, tail) = match amp {
            Some(p) => rest.split_at(p),
            None => (rest, ""),
        };
        out.extend(plain.split_word_bounds().map(|w| Tok { s: w, kind: Kind::Text }));
        if tail.is_empty() {
            break;
        }
        let ent = tail[1..]
            .char_indices()
            .take(12)
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '#'))
            .filter(|(_, c)| *c == ';')
            .map(|(k, _)| k + 2);
        let n = ent.unwrap_or(1);
        out.push(Tok { s: &tail[..n], kind: Kind::Text });
        rest = &tail[n..];
    }
}

/// Words (and media) of a node, for pairing.
fn words(n: &Node) -> Vec<String> {
    let src = match n {
        Node::El(e) => e.outer,
        Node::Text(t) => t,
    };
    tokens(src).iter().filter(|t| t.is_word()).map(|t| t.key().into_owned()).collect()
}

/// `2 · common / (a + b)` over two word lists (1 when both are empty).
fn similarity(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let ar: Vec<&str> = a.iter().map(String::as_str).collect();
    let br: Vec<&str> = b.iter().map(String::as_str).collect();
    let common: usize = equal_str_runs(&ar, &br).iter().map(|r| r.2).sum();
    2.0 * common as f64 / (a.len() + b.len()) as f64
}

// ---------------------------------------------------------------------------------------------
// The diff

/// One emitted top-level piece: its HTML and whether it is unchanged (foldable).
struct Piece {
    html: String,
    equal: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Add,
    Del,
}

struct Cx {
    stats: DiffStats,
    /// A change point is open (the last thing emitted was a mark).
    open: bool,
}

impl Cx {
    /// ` data-cd-dx="N"` when this mark starts a run of changes.
    fn point(&mut self) -> String {
        if self.open {
            return String::new();
        }
        self.open = true;
        self.stats.changes += 1;
        format!(" data-cd-dx=\"{}\"", self.stats.changes)
    }

    /// Diff two sibling sequences; `parent` is their element (`None` = the top level).
    fn seq(&mut self, old: &[Node], new: &[Node], parent: Option<&str>) -> Vec<Piece> {
        let ok: Vec<String> = old.iter().map(key).collect();
        let nk: Vec<String> = new.iter().map(key).collect();
        let or: Vec<&str> = ok.iter().map(String::as_str).collect();
        let nr: Vec<&str> = nk.iter().map(String::as_str).collect();
        let mut runs = equal_str_runs(&or, &nr);
        runs.push((old.len(), new.len(), 0));
        let mut out = Vec::new();
        let (mut oi, mut ni) = (0, 0);
        for (ro, rn, len) in runs {
            self.hunk(&old[oi..ro], &new[ni..rn], parent, &mut out);
            for n in &new[rn..rn + len] {
                self.open = false;
                out.push(Piece { html: outer(n).to_string(), equal: true });
            }
            (oi, ni) = (ro + len, rn + len);
        }
        out
    }

    /// A run of changed siblings: pair what is similar (order kept), mark the rest whole. Cells
    /// of a row pair by position when the row kept its width (a column is a position). At the
    /// top level every changed block is a change point of its own, except that removed blocks
    /// followed by added ones (a replacement) are one.
    fn hunk(&mut self, old: &[Node], new: &[Node], parent: Option<&str>, out: &mut Vec<Piece>) {
        let (m, n) = (old.len(), new.len());
        if m == 0 && n == 0 {
            return;
        }
        let top = parent.is_none();
        let mut last: Option<Mark> = None;
        let mut pair = vec![vec![false; n]; m];
        if parent == Some("tr") && m == n {
            for (i, row) in pair.iter_mut().enumerate() {
                row[i] = true;
            }
        } else if m * n <= MAX_PAIRING {
            let ow: Vec<Vec<String>> = old.iter().map(words).collect();
            let nw: Vec<Vec<String>> = new.iter().map(words).collect();
            for i in 0..m {
                for j in 0..n {
                    pair[i][j] = pairable(&old[i], &new[j], m == 1 && n == 1, &ow[i], &nw[j]);
                }
            }
        }
        // LCS over `pairable`, removals before additions between pairs.
        let mut dp = vec![vec![0usize; n + 1]; m + 1];
        for i in (0..m).rev() {
            for j in (0..n).rev() {
                dp[i][j] = if pair[i][j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < m || j < n {
            if i < m && j < n && pair[i][j] && dp[i][j] == dp[i + 1][j + 1] + 1 {
                if top {
                    self.open = false;
                }
                last = None;
                let html = self.pair(&old[i], &new[j]);
                out.push(Piece { html, equal: false });
                i += 1;
                j += 1;
            } else if i < m && (j == n || dp[i + 1][j] >= dp[i][j + 1]) {
                if top && last.is_none() {
                    self.open = false;
                }
                last = Some(Mark::Del);
                let html = self.mark(&old[i], Mark::Del);
                out.push(Piece { html, equal: false });
                i += 1;
            } else {
                if top && last.is_none() {
                    self.open = false;
                }
                last = Some(Mark::Add);
                let html = self.mark(&new[j], Mark::Add);
                out.push(Piece { html, equal: false });
                j += 1;
            }
        }
    }

    /// A node marked whole as added or removed.
    fn mark(&mut self, node: &Node, mark: Mark) -> String {
        let (class, label) = match mark {
            Mark::Add => ("cd-dx-add", "added"),
            Mark::Del => ("cd-dx-del", "removed"),
        };
        match mark {
            Mark::Add => self.stats.blocks_added += 1,
            Mark::Del => self.stats.blocks_removed += 1,
        }
        let point = self.point();
        match node {
            Node::Text(t) => {
                let (tag, cls) = if mark == Mark::Add { ("ins", "cd-ins") } else { ("del", "cd-del") };
                let body = if mark == Mark::Del { strip_attrs(t) } else { t.to_string() };
                format!("<{tag} class=\"{cls} {class}\"{point}>{body}</{tag}>")
            }
            Node::El(e) => {
                let sr = format!("<span class=\"cd-sr\">{label}: </span>");
                let (open, inner, close) = if mark == Mark::Del {
                    (strip_attrs(e.open), strip_attrs(e.inner), e.close.to_string())
                } else {
                    (e.open.to_string(), e.inner.to_string(), e.close.to_string())
                };
                let open = inject(&open, &format!("{class} cd-dx-blk"), &point);
                match e.name.as_str() {
                    // The label may not go inside these: before them (their parent takes flow).
                    "ul" | "ol" | "dl" | "table" | "hr" | "math" => format!("{sr}{open}{inner}{close}"),
                    // A row (or section): inside its first cell.
                    "tr" | "thead" | "tbody" | "tfoot" => format!("{open}{}{close}", in_first_cell(&inner, &sr)),
                    _ => format!("{open}{sr}{inner}{close}"),
                }
            }
        }
    }

    /// A paired old and new node: one level down for lists, tables and containers, else words.
    fn pair(&mut self, old: &Node, new: &Node) -> String {
        match (old, new) {
            (Node::El(o), Node::El(n))
                if RESTRICTED.contains(&n.name.as_str()) || (o.has_blocks() && n.has_blocks()) =>
            {
                let kids = self.seq(&o.kids, &n.kids, Some(&n.name));
                let sep = if n.has_blocks() { "\n" } else { "" };
                let mut s = String::from(n.open);
                s.push_str(sep);
                for p in kids {
                    s.push_str(&p.html);
                    s.push_str(sep);
                }
                s.push_str(n.close);
                s
            }
            (Node::El(o), Node::El(n)) => {
                let (body, words_changed) = words_diff(o.inner, n.inner);
                self.stats.blocks_changed += 1;
                let point = self.point();
                let (class, label) = if words_changed {
                    ("cd-dx-mod", "changed")
                } else {
                    ("cd-dx-mod cd-dx-fmt", "formatting changed")
                };
                format!("{}<span class=\"cd-sr\">{label}: </span>{body}{}", inject(n.open, class, &point), n.close)
            }
            (Node::Text(ot), Node::Text(nt)) => {
                let (body, words_changed) = words_diff(ot, nt);
                self.stats.blocks_changed += 1;
                let point = self.point();
                let class = if words_changed { "cd-dx-mod" } else { "cd-dx-mod cd-dx-fmt" };
                format!("<span class=\"{class}\"{point}>{body}</span>")
            }
            _ => {
                // Not reached (`pairable` pairs like with like); kept total.
                let d = self.mark(old, Mark::Del);
                format!("{d}{}", self.mark(new, Mark::Add))
            }
        }
    }
}

/// May an old and a new node be shown as one changed block?
fn pairable(o: &Node, n: &Node, only: bool, ow: &[String], nw: &[String]) -> bool {
    match (o, n) {
        (Node::Text(_), Node::Text(_)) => similarity(ow, nw) >= 0.4,
        (Node::El(a), Node::El(b)) if a.name == b.name && !a.atomic && !b.atomic => {
            if RESTRICTED.contains(&a.name.as_str()) || (a.has_blocks() && b.has_blocks()) {
                only || similarity(ow, nw) >= 0.2
            } else {
                similarity(ow, nw) >= 0.4
            }
        }
        _ => false,
    }
}

fn outer<'a>(n: &'a Node) -> &'a str {
    match n {
        Node::El(e) => e.outer,
        Node::Text(t) => t,
    }
}

/// Add `class` (and `extra` attributes) to an open tag.
fn inject(open: &str, class: &str, extra: &str) -> String {
    let with_class = match open.find(" class=\"") {
        Some(p) => format!("{}{class} {}", &open[..p + 8], &open[p + 8..]),
        None => {
            let at = gt_at(open);
            format!("{} class=\"{class}\"{}", &open[..at], &open[at..])
        }
    };
    if extra.is_empty() {
        return with_class;
    }
    let at = gt_at(&with_class);
    format!("{}{extra}{}", &with_class[..at], &with_class[at..])
}

/// Where an open tag's closing `>` (or `/>`) starts.
fn gt_at(open: &str) -> usize {
    if open.ends_with("/>") { open.len() - 2 } else { open.len().saturating_sub(1) }
}

/// `label` right after the first `<td …>` / `<th …>` open tag in `inner` (or first, if none).
fn in_first_cell(inner: &str, label: &str) -> String {
    let at = [inner.find("<td"), inner.find("<th")].into_iter().flatten().min();
    match at {
        Some(p) => {
            let e = tag_end(inner, p);
            format!("{}{label}{}", &inner[..e], &inner[e..])
        }
        None => format!("{label}{inner}"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seg {
    Eq,
    Chg,
}

/// Word diff of two inline fragments: the newer fragment with `del` / `ins` marks, and whether
/// any word (not just markup) changed.
fn words_diff(old: &str, new: &str) -> (String, bool) {
    let ot = tokens(old);
    let nt = tokens(new);
    if ot.len() + nt.len() > MAX_WORD_TOKENS {
        let changed = old != new;
        let d = text_only(&ot);
        let i = text_only(&nt);
        return (format!("<del class=\"cd-del\">{d}</del><ins class=\"cd-ins\">{i}</ins>"), changed);
    }
    let ok: Vec<std::borrow::Cow<str>> = ot.iter().map(Tok::key).collect();
    let nk: Vec<std::borrow::Cow<str>> = nt.iter().map(Tok::key).collect();
    let or: Vec<&str> = ok.iter().map(|c| c.as_ref()).collect();
    let nr: Vec<&str> = nk.iter().map(|c| c.as_ref()).collect();
    let mut runs = equal_str_runs(&or, &nr);
    runs.push((ot.len(), nt.len(), 0));
    // Segments: (kind, old range, new range), alternating.
    let mut segs: Vec<(Seg, std::ops::Range<usize>, std::ops::Range<usize>)> = Vec::new();
    let (mut oi, mut ni) = (0, 0);
    for (ro, rn, len) in runs {
        if ro > oi || rn > ni {
            segs.push((Seg::Chg, oi..ro, ni..rn));
        }
        if len > 0 {
            segs.push((Seg::Eq, ro..ro + len, rn..rn + len));
        }
        (oi, ni) = (ro + len, rn + len);
    }
    // Readability: spaces alone between two changes join them into one.
    let mut k = 1;
    while k + 1 < segs.len() {
        let (kind, _, ref nr) = segs[k];
        let trivial = kind == Seg::Eq
            && segs[k - 1].0 == Seg::Chg
            && segs[k + 1].0 == Seg::Chg
            && nt[nr.clone()].iter().all(|t| t.kind == Kind::Text && !t.visible());
        if trivial {
            let o = segs[k - 1].1.start..segs[k + 1].1.end;
            let n = segs[k - 1].2.start..segs[k + 1].2.end;
            segs.splice(k - 1..k + 2, [(Seg::Chg, o, n)]);
        } else {
            k += 1;
        }
    }
    let mut out = String::with_capacity(new.len() + 64);
    let mut changed = false;
    for (kind, o, n) in segs {
        if kind == Seg::Eq {
            for t in &nt[n] {
                out.push_str(t.s);
            }
            continue;
        }
        // Deleted words first (their markup is the older Version's: dropped), then the newer
        // side's tokens in order, its words grouped into `ins` between its tags.
        let del: String = ot[o.clone()].iter().filter(|t| t.kind != Kind::Tag).map(|t| strip_attrs(t.s)).collect();
        if ot[o].iter().any(Tok::visible) {
            out.push_str("<del class=\"cd-del\">");
            out.push_str(&del);
            out.push_str("</del>");
            changed = true;
        }
        let mut run = String::new();
        let mut run_words = false;
        let flush = |out: &mut String, run: &mut String, words: &mut bool, changed: &mut bool| {
            if *words {
                out.push_str("<ins class=\"cd-ins\">");
                out.push_str(run);
                out.push_str("</ins>");
                *changed = true;
            } else {
                out.push_str(run);
            }
            run.clear();
            *words = false;
        };
        for t in &nt[n] {
            if t.kind == Kind::Tag {
                flush(&mut out, &mut run, &mut run_words, &mut changed);
                out.push_str(t.s);
            } else {
                run.push_str(t.s);
                run_words |= t.visible();
            }
        }
        flush(&mut out, &mut run, &mut run_words, &mut changed);
    }
    (out, changed)
}

fn text_only(t: &[Tok]) -> String {
    t.iter().filter(|t| t.kind != Kind::Tag).map(|t| strip_attrs(t.s)).collect()
}

/// Fold long runs of unchanged top-level pieces.
fn fold(pieces: &[Piece]) -> String {
    let mut out = String::new();
    let mut i = 0;
    let n = pieces.len();
    while i < n {
        if !pieces[i].equal {
            out.push_str(&pieces[i].html);
            out.push('\n');
            i += 1;
            continue;
        }
        let s = i;
        while i < n && pieces[i].equal {
            i += 1;
        }
        let e = i;
        // The first block of the Document (its title) stays; the end needs no trailing context.
        let head = if s == 0 { 1 } else { CONTEXT };
        let tail = if e == n { 0 } else { CONTEXT };
        let len = e - s;
        if len >= head + tail + MIN_FOLD {
            for p in &pieces[s..s + head] {
                out.push_str(&p.html);
                out.push('\n');
            }
            let hidden = len - head - tail;
            out.push_str(&format!(
                "<details class=\"cd-dx-same\"><summary class=\"cd-dx-sum\">{hidden} unchanged block{}</summary>\n",
                if hidden == 1 { "" } else { "s" }
            ));
            for p in &pieces[s + head..e - tail] {
                out.push_str(&p.html);
                out.push('\n');
            }
            out.push_str("</details>\n");
            for p in &pieces[e - tail..e] {
                out.push_str(&p.html);
                out.push('\n');
            }
        } else {
            for p in &pieces[s..e] {
                out.push_str(&p.html);
                out.push('\n');
            }
        }
    }
    out
}
