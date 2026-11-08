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

use std::borrow::Cow;

use imara_diff::Interner;
use unicode_segmentation::UnicodeSegmentation;

use crate::mapping::heuristic_str_runs;

/// Unchanged top-level blocks kept visible on each side of a change.
pub const CONTEXT: usize = 2;
/// A fold hides at least this many blocks (fewer stay visible).
pub const MIN_FOLD: usize = 3;
/// Above this many old × new blocks in one changed run nothing is paired: each side is shown
/// whole, in linear time and with no matrix.
const MAX_PAIRING: usize = 1_000_000;
/// A word diff above this many tokens (old + new) shows both sides whole.
const MAX_WORD_TOKENS: usize = 20_000;
/// Up to this many tokens a diff is minimal Myers; above it, the bounded heuristic Myers.
const MAX_MINIMAL_TOKENS: usize = 2_000;
/// Block elements read per side: past it, the rest of each element is one opaque run, because the
/// tree costs a few hundred bytes per block and a list item can take four bytes of Markdown.
const MAX_BLOCKS: usize = 100_000;
/// Rendered HTML per side past which nothing is compared at all: memory follows the HTML, and a
/// few bytes of Markdown can render 20 times their size. An ordinary Document at the Markdown cap
/// renders well under it.
pub const MAX_HTML_BYTES: usize = 12 << 20;
/// In a run of more than this many old × new blocks, a block is only measured against those near
/// its own position, because comparing every pair of similar blocks would outweigh the rest.
const BAND_ABOVE: usize = 400;
const BAND: usize = 32;
/// Blocks nest at most this deep in the tree: anything deeper is compared whole, so a pathological
/// Document can neither overflow the stack nor repeat the work per level.
const MAX_DEPTH: usize = 32;
/// The work one diff may do, in units of about a nanosecond of a release build. Every step that
/// can repeat per nesting level or grow faster than its input charges its worst case first; past
/// the budget, blocks are shown whole (`DiffStats::shown_whole`). What is left uncharged is one
/// linear pass: parsing, the marks and the fold.
pub const WORK_BUDGET: u64 = 400_000_000;
/// Per byte of a subtree: stripping its attributes, normalizing its text and hashing the key.
const KEY_COST: u64 = 2;
/// Per byte of a node: splitting it into words and hashing them for pairing.
const BAG_COST: u64 = 20;
/// Per byte of a leaf: splitting it into tokens for a word diff.
const TOKEN_COST: u64 = 15;

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
    /// The work budget ran out, so some blocks are shown whole rather than word by word.
    pub shown_whole: bool,
    /// A Version renders to more than [`MAX_HTML_BYTES`]: nothing was compared.
    pub too_large: bool,
}

/// A rendered diff: the HTML fragment and its numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedDiff {
    pub html: String,
    pub stats: DiffStats,
    /// The work this diff charged (at most [`WORK_BUDGET`]).
    pub work: u64,
}

/// Diff Version A (`old_md`, rendered as `old_html`) against Version B (`new_md`, `new_html`).
pub fn rendered_diff(old_md: &str, new_md: &str, old_html: &str, new_html: &str) -> RenderedDiff {
    let (lines_added, lines_removed) = line_counts(old_md, new_md);
    if old_html.len().max(new_html.len()) > MAX_HTML_BYTES {
        let stats = DiffStats { lines_added, lines_removed, too_large: true, ..DiffStats::default() };
        return RenderedDiff { html: String::new(), stats, work: 0 };
    }
    let (old, old_cut) = parse(old_html);
    let (new, new_cut) = parse(new_html);
    let stats = DiffStats { lines_added, lines_removed, shown_whole: old_cut || new_cut, ..DiffStats::default() };
    let mut cx = Cx { stats, open: false, work: 0 };
    let pieces = cx.seq(&old, &new, None);
    RenderedDiff { html: fold(pieces), stats: cx.stats, work: cx.work }
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
    // Boxed so that a list of children stays small: there can be one element per few bytes of input.
    El(Box<El<'a>>),
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

/// The close tag of the `name` element opened before `from`, counting nested `name`s, without
/// recursing: `(len, len)` when it never closes.
fn matching_close(s: &str, from: usize, name: &str) -> (usize, usize) {
    let mut depth = 1usize;
    let mut i = from;
    while let Some(k) = s[i..].find('<').map(|p| i + p) {
        if s[k..].starts_with("<!--") {
            i = s[k..].find("-->").map(|p| k + p + 3).unwrap_or(s.len());
            continue;
        }
        let closing = s[k..].starts_with("</");
        if name_at(s, k + if closing { 2 } else { 1 }) == name {
            let end = tag_end(s, k);
            if closing {
                depth -= 1;
                if depth == 0 {
                    return (k, end);
                }
            } else if !s[k..end].ends_with("/>") {
                depth += 1;
            }
            i = end;
        } else {
            i = k + 1;
        }
    }
    (s.len(), s.len())
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
    /// The open block elements around the cursor (at most [`MAX_DEPTH`]).
    stack: Vec<String>,
    /// Block elements read so far (at most [`MAX_BLOCKS`]).
    blocks: usize,
}

/// Parse a rendered fragment into its top-level nodes.
fn parse(html: &str) -> (Vec<Node<'_>>, bool) {
    let mut p = Parser { s: html, i: 0, stack: Vec::new(), blocks: 0 };
    let nodes = p.children().0;
    (nodes, p.blocks >= MAX_BLOCKS)
}

impl<'a> Parser<'a> {
    /// Children up to the close tag of `stack`'s last element (or an ancestor's, which is left
    /// for it, or the end). Returns the children and the close tag's byte range.
    fn children(&mut self) -> (Vec<Node<'a>>, (usize, usize)) {
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
                    if self.stack.last() == Some(&name) {
                        self.i = end;
                        break (k, end);
                    }
                    if self.stack.contains(&name) {
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
            if self.blocks >= MAX_BLOCKS {
                // One opaque run up to this element's own close (balanced: whole children only).
                let close = match self.stack.last() {
                    Some(open) => matching_close(s, k, open),
                    None => (s.len(), s.len()),
                };
                self.i = close.1;
                break close;
            }
            self.blocks += 1;
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
            } else if self.stack.len() >= MAX_DEPTH {
                let (cs, ce) = matching_close(s, end, &name);
                self.i = ce;
                let inner = &s[end..cs];
                El {
                    name,
                    open: tag,
                    inner,
                    close: &s[cs..ce],
                    outer: &s[k..ce],
                    kids: vec![Node::Text(inner)],
                    atomic: true,
                }
            } else {
                self.i = end;
                self.stack.push(name.clone());
                let (kids, (cs, ce)) = self.children();
                self.stack.pop();
                El { name, open: tag, inner: &s[end..cs], close: &s[cs..ce], outer: &s[k..ce], kids, atomic: false }
            };
            let mut el = el;
            if el.has_blocks() {
                // Whitespace between blocks is layout, not content.
                el.kids.retain(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
            }
            out.push(Node::El(Box::new(el)));
            text_start = self.i;
        };
        flush(&mut out, text_start, close.0);
        if out.iter().any(|n| matches!(n, Node::El(_))) {
            out.retain(|n| !matches!(n, Node::Text(t) if t.trim().is_empty()));
        }
        out.shrink_to_fit();
        (out, close)
    }
}

/// Markup without its `data-l` and `id` attributes: what two blocks are compared by, and what a
/// removed block shows. Only tags are touched, because raw HTML text may hold ` id="` literally.
fn strip_attrs(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while let Some(k) = html[i..].find('<').map(|p| i + p) {
        out.push_str(&html[i..k]);
        if html[k..].starts_with("<!--") {
            let e = html[k..].find("-->").map(|p| k + p + 3).unwrap_or(html.len());
            out.push_str(&html[k..e]);
            i = e;
        } else if html.as_bytes().get(k + 1).is_some_and(u8::is_ascii_alphabetic) {
            let e = tag_end(html, k);
            strip_tag(&html[k..e], &mut out);
            i = e;
        } else {
            out.push('<');
            i = k + 1;
        }
    }
    out.push_str(&html[i..]);
    out
}

/// Copy one open tag without its `data-l` and `id` attributes (values may hold anything quoted).
fn strip_tag(tag: &str, out: &mut String) {
    let b = tag.as_bytes();
    let mut j = 1;
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
        j += 1;
    }
    out.push_str(&tag[..j]);
    loop {
        let start = j;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= b.len() || b[j] == b'>' || b[j] == b'/' {
            out.push_str(&tag[start..]);
            return;
        }
        let name_start = j;
        while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'=' | b'>' | b'/') {
            j += 1;
        }
        let name = &tag[name_start..j];
        let mut v = j;
        while v < b.len() && b[v].is_ascii_whitespace() {
            v += 1;
        }
        if v < b.len() && b[v] == b'=' {
            v += 1;
            while v < b.len() && b[v].is_ascii_whitespace() {
                v += 1;
            }
            if v < b.len() && matches!(b[v], b'"' | b'\'') {
                let q = b[v];
                v += 1;
                while v < b.len() && b[v] != q {
                    v += 1;
                }
                v = (v + 1).min(b.len());
            } else {
                while v < b.len() && !b[v].is_ascii_whitespace() && b[v] != b'>' {
                    v += 1;
                }
            }
            j = v;
        }
        if !name.eq_ignore_ascii_case("data-l") && !name.eq_ignore_ascii_case("id") {
            out.push_str(&tag[start..j]);
        }
    }
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

impl<'a> Tok<'a> {
    fn key(&self) -> Cow<'a, str> {
        match self.kind {
            Kind::Text => Cow::Borrowed(self.s),
            _ => Cow::Owned(strip_attrs(self.s)),
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
    scan_tokens(s, true)
}

/// The same with the text between tags left in one piece: enough to show a side whole, and cheap
/// enough to do once the budget is out.
fn coarse_tokens(s: &str) -> Vec<Tok<'_>> {
    scan_tokens(s, false)
}

fn scan_tokens(s: &str, words: bool) -> Vec<Tok<'_>> {
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
        if words {
            text_tokens(&s[i..e], &mut out);
        } else {
            out.push(Tok { s: &s[i..e], kind: Kind::Text });
        }
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

/// The sorted bag of a node's words (and media), as 64-bit hashes: a pure function of the text,
/// and a collision only nudges whether two blocks are shown as one changed block.
fn bag(n: &Node) -> Vec<u64> {
    let mut v: Vec<u64> = tokens(outer(n)).iter().filter(|t| t.is_word()).map(|t| fnv1a(t.key().as_bytes())).collect();
    v.sort_unstable();
    v
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// `2 · common / (a + b)` over two sorted bags of word hashes (1 when both are empty): an upper bound
/// of the ordered similarity, in linear time, so pairing never runs a diff per pair.
fn similarity(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let (mut i, mut j, mut common) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    2.0 * common as f64 / (a.len() + b.len()) as f64
}

/// Could two bags of these sizes reach `threshold` at all?
fn may_reach(a: usize, b: usize, threshold: f64) -> bool {
    a + b == 0 || 2.0 * a.min(b) as f64 >= threshold * (a + b) as f64
}

/// The worst case of a diff of `n` tokens, as measured: minimal Myers reaches about 3 · n²/2 and the
/// heuristic about 20 · n^1.5 (on few distinct tokens in a different order).
fn diff_cost(n: usize) -> u64 {
    let n = n as u64;
    if n as usize <= MAX_MINIMAL_TOKENS { 3 * n * n / 2 + n } else { 20 * n * n.isqrt() }
}

/// Items a diff never has to look at: the common start and end of two sequences.
fn common_ends<T>(a: &[T], b: &[T], same: impl Fn(&T, &T) -> bool) -> usize {
    let start = a.iter().zip(b).take_while(|(x, y)| same(x, y)).count();
    let end = a[start..].iter().rev().zip(b[start..].iter().rev()).take_while(|(x, y)| same(x, y)).count();
    start + end
}

/// Equal runs of two token lists, by the cheapest diff that bounds [`diff_cost`]. The minimal diff
/// compares ids, so a long shared prefix between two keys costs nothing per comparison.
fn runs(os: &[&str], ns: &[&str]) -> Vec<(usize, usize, usize)> {
    if os.len() + ns.len() > MAX_MINIMAL_TOKENS {
        return heuristic_str_runs(os, ns);
    }
    let mut ids: Interner<&str> = Interner::new(os.len() + ns.len());
    let a: Vec<u32> = os.iter().map(|s| ids.intern(s).0).collect();
    let b: Vec<u32> = ns.iter().map(|s| ids.intern(s).0).collect();
    similar::capture_diff_slices(similar::Algorithm::Myers, &a, &b)
        .into_iter()
        .filter_map(|op| match op {
            similar::DiffOp::Equal { old_index, new_index, len } => Some((old_index, new_index, len)),
            _ => None,
        })
        .collect()
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
    /// Comparisons charged so far (see [`WORK_BUDGET`]).
    work: u64,
}

/// A whole-block mark or a pair, in the order a run of changes emits them.
struct Emit {
    top: bool,
    last: Option<Mark>,
}

impl Cx {
    /// Take `cost` from the budget; `false` (and nothing taken) when it does not fit.
    fn charge(&mut self, cost: u64) -> bool {
        if self.work.saturating_add(cost) > WORK_BUDGET {
            self.stats.shown_whole = true;
            return false;
        }
        self.work += cost;
        true
    }

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
        let bytes: usize = old.iter().chain(new).map(|n| outer(n).len()).sum();
        let mut out = Vec::new();
        let mut e = Emit { top: parent.is_none(), last: None };
        if !self.charge(bytes as u64 * KEY_COST) {
            self.whole(old, new, &mut e, &mut out);
            return out;
        }
        // The keys hold a copy of the whole subtree: they are gone before any recursion.
        let equal = {
            let ok: Vec<String> = old.iter().map(key).collect();
            let nk: Vec<String> = new.iter().map(key).collect();
            let or: Vec<&str> = ok.iter().map(String::as_str).collect();
            let nr: Vec<&str> = nk.iter().map(String::as_str).collect();
            let ends = common_ends(&or, &nr, |x, y| x == y);
            self.charge(diff_cost(or.len() + nr.len() - 2 * ends)).then(|| runs(&or, &nr))
        };
        let Some(mut equal) = equal else {
            self.whole(old, new, &mut e, &mut out);
            return out;
        };
        equal.push((old.len(), new.len(), 0));
        let (mut oi, mut ni) = (0, 0);
        for (ro, rn, len) in equal {
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
        let mut e = Emit { top: parent.is_none(), last: None };
        if parent == Some("tr") && m == n {
            for (o, nw) in old.iter().zip(new) {
                self.emit_pair(o, nw, &mut e, out);
            }
            return;
        }
        if m * n > MAX_PAIRING {
            self.stats.shown_whole = true;
        }
        let pair = if m * n <= MAX_PAIRING && m * n > 0 { self.pairs(old, new) } else { None };
        let Some(pair) = pair else {
            self.whole(old, new, &mut e, out);
            return;
        };
        // LCS over the pairs, removals before additions between them (flat: `(m + 1) × (n + 1)`).
        let at = |i: usize, j: usize| i * (n + 1) + j;
        let mut dp = vec![0u32; (m + 1) * (n + 1)];
        for i in (0..m).rev() {
            for j in (0..n).rev() {
                dp[at(i, j)] =
                    if pair[i * n + j] { dp[at(i + 1, j + 1)] + 1 } else { dp[at(i + 1, j)].max(dp[at(i, j + 1)]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < m || j < n {
            if i < m && j < n && pair[i * n + j] && dp[at(i, j)] == dp[at(i + 1, j + 1)] + 1 {
                self.emit_pair(&old[i], &new[j], &mut e, out);
                i += 1;
                j += 1;
            } else if i < m && (j == n || dp[at(i + 1, j)] >= dp[at(i, j + 1)]) {
                self.emit_mark(&old[i], Mark::Del, &mut e, out);
                i += 1;
            } else {
                self.emit_mark(&new[j], Mark::Add, &mut e, out);
                j += 1;
            }
        }
    }

    /// Which old and new nodes may pair (row-major, `m × n`); `None` when the budget is out. Only
    /// nodes some pair has to measure are read, and only after their reading is charged: a lone
    /// container pairs without measuring, so a chain of them costs nothing per level here.
    fn pairs(&mut self, old: &[Node], new: &[Node]) -> Option<Vec<bool>> {
        let (m, n) = (old.len(), new.len());
        // Filling and walking the pairing and LCS matrices: about 3 ns per cell.
        if !self.charge((m * n) as u64 * 3) {
            return None;
        }
        let only = m == 1 && n == 1;
        let (mut read_old, mut read_new) = (vec![false; m], vec![false; n]);
        let mut pair = vec![false; m * n];
        let band = if m * n > BAND_ABOVE { BAND } else { usize::MAX };
        let near = |i: usize| i.saturating_sub(band)..i.saturating_add(band).saturating_add(1).min(n);
        for i in 0..m {
            for j in near(i) {
                match threshold(&old[i], &new[j], only) {
                    Some(t) if t <= 0.0 => pair[i * n + j] = true,
                    Some(_) => (read_old[i], read_new[j]) = (true, true),
                    None => {}
                }
            }
        }
        let to_read = |nodes: &[Node], read: &[bool]| -> u64 {
            nodes.iter().zip(read).filter(|(_, r)| **r).map(|(n, _)| outer(n).len() as u64).sum()
        };
        if !self.charge((to_read(old, &read_old) + to_read(new, &read_new)).saturating_mul(BAG_COST)) {
            return None;
        }
        let bags = |nodes: &[Node], read: &[bool]| -> Vec<Vec<u64>> {
            nodes.iter().zip(read).map(|(n, r)| if *r { bag(n) } else { Vec::new() }).collect()
        };
        let ob = bags(old, &read_old);
        let nb = bags(new, &read_new);
        for i in 0..m {
            for j in near(i) {
                if !(read_old[i] && read_new[j]) {
                    continue;
                }
                let Some(threshold) = threshold(&old[i], &new[j], only).filter(|t| *t > 0.0) else { continue };
                if !may_reach(ob[i].len(), nb[j].len(), threshold) {
                    continue;
                }
                if !self.charge((ob[i].len() + nb[j].len()) as u64) {
                    return Some(pair);
                }
                pair[i * n + j] = similarity(&ob[i], &nb[j]) >= threshold;
            }
        }
        Some(pair)
    }

    /// Every old node removed, then every new one added: linear, no matrices.
    fn whole(&mut self, old: &[Node], new: &[Node], e: &mut Emit, out: &mut Vec<Piece>) {
        for o in old {
            self.emit_mark(o, Mark::Del, e, out);
        }
        for n in new {
            self.emit_mark(n, Mark::Add, e, out);
        }
    }

    fn emit_pair(&mut self, old: &Node, new: &Node, e: &mut Emit, out: &mut Vec<Piece>) {
        if e.top {
            self.open = false;
        }
        e.last = None;
        let html = self.pair(old, new);
        out.push(Piece { html, equal: false });
    }

    fn emit_mark(&mut self, node: &Node, mark: Mark, e: &mut Emit, out: &mut Vec<Piece>) {
        if matches!(node, Node::Text(t) if t.trim().is_empty()) {
            return; // layout between blocks, on a side that has no blocks to separate
        }
        if e.top && e.last.is_none() {
            self.open = false;
        }
        e.last = Some(mark);
        let html = self.mark(node, mark);
        out.push(Piece { html, equal: false });
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

    /// A paired old and new node: one level down when either side holds blocks (a list, a table,
    /// a tight item become loose), else words.
    fn pair(&mut self, old: &Node, new: &Node) -> String {
        match (old, new) {
            (Node::El(o), Node::El(n)) if RESTRICTED.contains(&n.name.as_str()) || o.has_blocks() || n.has_blocks() => {
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
            (Node::El(o), Node::El(n)) => self.changed_block(o.inner, n),
            // A tight item's text against the paragraph that replaced it.
            (Node::Text(t), Node::El(n)) => self.changed_block(t, n),
            (Node::El(o), Node::Text(t)) => self.changed_inline(o.inner, t),
            (Node::Text(o), Node::Text(n)) => self.changed_inline(o, n),
        }
    }

    /// The new block with the words of `old` diffed into it.
    fn changed_block(&mut self, old: &str, n: &El) -> String {
        let (body, words_changed) = self.words_diff(old, n.inner);
        self.stats.blocks_changed += 1;
        let point = self.point();
        let (class, label) =
            if words_changed { ("cd-dx-mod", "changed") } else { ("cd-dx-mod cd-dx-fmt", "formatting changed") };
        format!("{}<span class=\"cd-sr\">{label}: </span>{body}{}", inject(n.open, class, &point), n.close)
    }

    /// The new inline run with the words of `old` diffed into it.
    fn changed_inline(&mut self, old: &str, new: &str) -> String {
        let (body, words_changed) = self.words_diff(old, new);
        self.stats.blocks_changed += 1;
        let point = self.point();
        if words_changed {
            format!("<span class=\"cd-dx-mod\"{point}>{body}</span>")
        } else {
            format!(
                "<span class=\"cd-dx-mod cd-dx-fmt\"{point}><span class=\"cd-sr\">formatting changed: </span>{body}</span>"
            )
        }
    }

    /// Word diff of two inline fragments: the newer fragment with `del` / `ins` marks, and whether
    /// any word (not just markup) changed. Too long, or past the budget: both sides whole.
    fn words_diff(&mut self, old: &str, new: &str) -> (String, bool) {
        if !self.charge((old.len() + new.len()) as u64 * TOKEN_COST) {
            return whole_words(old, new);
        }
        let ot = tokens(old);
        let nt = tokens(new);
        let n = ot.len() + nt.len();
        if n > MAX_WORD_TOKENS {
            self.stats.shown_whole = true;
            return whole_words(old, new);
        }
        if !self.charge(diff_cost(n - 2 * common_ends(&ot, &nt, |x, y| x.s == y.s))) {
            return whole_words(old, new);
        }
        words_merge(new, &ot, &nt)
    }
}

/// Both inline fragments whole: the older side's words struck, the newer side as it is.
fn whole_words(old: &str, new: &str) -> (String, bool) {
    let (ot, nt) = (coarse_tokens(old), coarse_tokens(new));
    let mut out = String::new();
    if ot.iter().any(Tok::visible) {
        out.push_str(&format!("<del class=\"cd-del\">{}</del>", text_only(&ot)));
    }
    if nt.iter().any(Tok::visible) {
        out.push_str(&format!("<ins class=\"cd-ins\">{new}</ins>"));
    }
    (out, old != new)
}

/// The element-similarity threshold for an old and a new node, `None` when they may never pair:
/// like with like, or a tight item's text with the paragraph that replaced it.
fn threshold(o: &Node, n: &Node, only: bool) -> Option<f64> {
    match (o, n) {
        (Node::Text(_), Node::Text(_)) => Some(0.4),
        (Node::Text(_), Node::El(e)) | (Node::El(e), Node::Text(_)) => (e.name == "p" && !e.atomic).then_some(0.4),
        (Node::El(a), Node::El(b)) if a.name == b.name && !a.atomic && !b.atomic => {
            if RESTRICTED.contains(&a.name.as_str()) || a.has_blocks() || b.has_blocks() {
                Some(if only { 0.0 } else { 0.2 })
            } else {
                Some(0.4)
            }
        }
        _ => None,
    }
}

fn outer<'a>(n: &Node<'a>) -> &'a str {
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

/// The word diff itself, on tokens already within the limits: the newer fragment with marks, and
/// whether any word (not just markup) changed.
fn words_merge(new: &str, ot: &[Tok], nt: &[Tok]) -> (String, bool) {
    let ok: Vec<Cow<str>> = ot.iter().map(Tok::key).collect();
    let nk: Vec<Cow<str>> = nt.iter().map(Tok::key).collect();
    let or: Vec<&str> = ok.iter().map(|c| c.as_ref()).collect();
    let nr: Vec<&str> = nk.iter().map(|c| c.as_ref()).collect();
    let mut equal = runs(&or, &nr);
    equal.push((ot.len(), nt.len(), 0));
    // Segments: (kind, old range, new range), in order. Spaces alone between two changes join
    // them into one, for readability.
    type Span = (Seg, std::ops::Range<usize>, std::ops::Range<usize>);
    let mut segs: Vec<Span> = Vec::new();
    let mut push = |seg: Span| {
        if seg.0 == Seg::Chg
            && let [.., before, gap] = segs.as_slice()
            && before.0 == Seg::Chg
            && gap.0 == Seg::Eq
            && nt[gap.2.clone()].iter().all(|t| t.kind == Kind::Text && !t.visible())
        {
            segs.pop();
            let last = segs.last_mut().expect("the change before the gap");
            last.1.end = seg.1.end;
            last.2.end = seg.2.end;
            return;
        }
        segs.push(seg);
    };
    let (mut oi, mut ni) = (0, 0);
    for (ro, rn, len) in equal {
        if ro > oi || rn > ni {
            push((Seg::Chg, oi..ro, ni..rn));
        }
        if len > 0 {
            push((Seg::Eq, ro..ro + len, rn..rn + len));
        }
        (oi, ni) = (ro + len, rn + len);
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
fn fold(pieces: Vec<Piece>) -> String {
    let mut out = String::with_capacity(pieces.iter().map(|p| p.html.len() + 1).sum::<usize>() + pieces.len() / 8);
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
