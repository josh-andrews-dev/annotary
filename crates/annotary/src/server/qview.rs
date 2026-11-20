//! Questions on the Document page (`docs/dev/questions.md`): the controls spliced into the first
//! paint from the Version's `questions` (`questions::shown`), the header's count and **Accept
//! recommended**, and the block a Question's Thread card leads with. One renderer: the page, the
//! card fragment and `ui/questions.js` (which only flips pressed states and the who-chose text)
//! read the same [`Shown`] list.
//!
//! Every element this module adds carries `cd-qx`, so `app.js` keeps it out of selection mapping
//! and highlight search like a Thread strip. What it adds:
//! - after a Question's place block, `.cd-qrow[data-q]`: the key, one `.cd-qopt[data-o]` button
//!   per Option (`aria-pressed` = the viewer's Answer; "rec" marks the recommended one), **Other…**,
//!   **Comment**, and `.cd-qwho` (who chose what, in words);
//! - at an Option's own block's leading edge, `input.cd-qradio[type=radio]` named by the block's
//!   text (the text itself stays selectable);
//! - in the decisions table's `Decided` cell, `.cd-qcell` with the Answers (marked apart from the
//!   cell's own text) and the same buttons; a decided Question shows the cell as written;
//! - read-only (an older Version, a deleted Document, no `comment`): the same, buttons disabled.

use super::questions::Shown;
use super::splice::{Block, insertion_point, scan};
use super::ui::esc;

/// What the viewer may do with the controls.
pub struct Ctx<'a> {
    pub me: &'a str,
    /// Head, live Document, `comment`: the buttons work.
    pub can_answer: bool,
}

/// `A @rowan, @sam · B @scribe` (`*` after a handle whose Answer was given on a Version where
/// the Question read differently); "No answer yet" without one.
pub fn who_text(s: &Shown) -> String {
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for a in &s.answers {
        let h = format!("@{}{}", a.handle, if a.changed { "*" } else { "" });
        match groups.iter_mut().find(|(o, _)| *o == a.option) {
            Some((_, g)) => g.push(h),
            None => groups.push((&a.option, vec![h])),
        }
    }
    if groups.is_empty() {
        return "No answer yet".to_string();
    }
    groups.iter().map(|(o, hs)| format!("{} {}", s.label_of(o), hs.join(", "))).collect::<Vec<_>>().join(" · ")
}

fn mine<'a>(s: &'a Shown, me: &str) -> Option<&'a str> {
    s.answers.iter().find(|a| a.handle == me).map(|a| a.option.as_str())
}

/// The Option buttons (and **Other…** / **Comment** for a table Question).
fn buttons(s: &Shown, cx: &Ctx, extra: bool) -> String {
    let dis = if cx.can_answer { "" } else { " disabled" };
    let my = mine(s, cx.me);
    let mut out = String::new();
    for o in &s.options {
        let pressed = my == Some(o.id.as_str());
        let text = o.label.clone().unwrap_or_else(|| o.id.clone());
        let rec = if s.rec.as_deref() == Some(o.id.as_str()) { "<span class=\"cd-qrec\">rec</span>" } else { "" };
        out.push_str(&format!(
            "<button type=\"button\" class=\"cd-qopt\" data-o=\"{}\" aria-pressed=\"{pressed}\"{dis}><span class=\"cd-qmark\" aria-hidden=\"true\">\u{2713}</span>{}{rec}</button>",
            esc(&o.id),
            esc(&text)
        ));
    }
    if extra {
        let pressed = my == Some("other");
        out.push_str(&format!(
            "<button type=\"button\" class=\"cd-qopt cd-qother\" data-o=\"other\" aria-pressed=\"{pressed}\"{dis}><span class=\"cd-qmark\" aria-hidden=\"true\">\u{2713}</span>Other\u{2026}</button>\
             <button type=\"button\" class=\"cd-link cd-qcmt\" data-act=\"qcomment\"{dis}>Comment</button>"
        ));
    }
    out
}

fn who_html(s: &Shown) -> String {
    format!("<span class=\"cd-qwho\" aria-live=\"polite\">{}</span>", esc(&who_text(s)))
}

/// The row at a Question's place.
fn row_html(s: &Shown, cx: &Ctx) -> String {
    let key = esc(&s.key);
    if let Some(d) = &s.decided {
        return format!(
            "<div class=\"cd-qx cd-qrow cd-qdone\" data-q=\"{key}\"><span class=\"cd-qkey\">{key}</span> decided: <b>{}</b></div>",
            esc(d)
        );
    }
    format!(
        "<div class=\"cd-qx cd-qrow\" data-q=\"{key}\" role=\"group\" aria-label=\"{key}: choose an option\"><span class=\"cd-qkey\">{key}</span>{}{}</div>",
        buttons(s, cx, true),
        who_html(s)
    )
}

/// The `Decided` cell's controls (an undecided Question).
fn cell_html(s: &Shown, cx: &Ctx) -> String {
    let key = esc(&s.key);
    format!(
        "<span class=\"cd-qx cd-qcell\" data-q=\"{key}\" role=\"group\" aria-label=\"{key}: answers, not yet in the Document\">{}<span class=\"cd-qbtns\">{}</span></span>",
        who_html(s),
        buttons(s, cx, false)
    )
}

/// What a Question's Thread card leads with: the Answers, and for a Question asked on the
/// Thread its labels as buttons.
pub fn card_html(s: &Shown, cx: &Ctx) -> String {
    let key = esc(&s.key);
    if s.table {
        let state = match &s.decided {
            Some(d) => format!("decided: <b>{}</b>", esc(d)),
            None => esc(&who_text(s)),
        };
        return format!(
            "<div class=\"cd-qx cd-qans\" data-q=\"{key}\"><span class=\"cd-qkey\">{key}</span> <span class=\"cd-qwho\">{state}</span></div>"
        );
    }
    format!(
        "<div class=\"cd-qx cd-qans\" data-q=\"{key}\" role=\"group\" aria-label=\"{key}: choose an answer\"><span class=\"cd-qbtns\">{}</span>{}</div>",
        buttons(s, cx, false),
        who_html(s)
    )
}

/// The header's `N of M answered` and **Accept recommended (K)**: N counts the table Questions the
/// viewer answered or the Version decided; K the undecided ones with a `Recommended` the viewer
/// has not answered. Empty without table Questions.
pub fn header_html(shown: &[Shown], cx: &Ctx) -> String {
    let table: Vec<&Shown> = shown.iter().filter(|s| s.table).collect();
    if table.is_empty() {
        return String::new();
    }
    let done = table.iter().filter(|s| s.is_decided() || mine(s, cx.me).is_some()).count();
    let k = table.iter().filter(|s| !s.is_decided() && s.rec.is_some() && mine(s, cx.me).is_none()).count();
    let accept = if cx.can_answer && k > 0 {
        format!("<button type=\"button\" class=\"cd-btn cd-sm\" id=\"cd-qaccept\">Accept recommended ({k})</button>")
    } else {
        String::new()
    };
    format!("<span class=\"cd-chip cd-qsum\" id=\"cd-qsum\">{done} of {} answered</span>{accept}", table.len())
}

/// The plain text of a block (tags dropped, the renderer's entities decoded), cut to 80 characters.
fn block_text(html: &str, b: &Block) -> String {
    let inner = &html[b.open_end.min(b.close_start)..b.close_start];
    let mut out = String::new();
    let mut in_tag = false;
    for c in inner.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let flat: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut: String = flat.chars().take(80).collect();
    if flat.chars().count() > 80 {
        cut.push('\u{2026}');
    }
    cut
}

/// Where an Option's control goes: just inside its block, at the leading edge (inside a loose
/// item's first paragraph; inside a row's first cell).
fn leading_edge(html: &str, blocks: &[Block], i: usize) -> usize {
    let b = &blocks[i];
    let first = blocks.iter().skip(i + 1).find(|c| c.parent == Some(i));
    match first {
        Some(c) if b.name == "tr" => c.open_end,
        Some(c) if b.name == "li" && c.name == "p" && html[b.open_end..c.open_start].trim().is_empty() => c.open_end,
        _ => b.open_end,
    }
}

/// Splice every Question's controls into the rendered `html`.
pub fn splice(html: &str, shown: &[Shown], cx: &Ctx) -> String {
    let table: Vec<&Shown> = shown.iter().filter(|s| s.table).collect();
    if table.is_empty() {
        return html.to_string();
    }
    let blocks = scan(html);
    // (offset, order, piece): pieces at one offset keep the order they were added in.
    let mut at: Vec<(usize, usize, String)> = Vec::new();
    for s in &table {
        if let Some(p) = &s.place {
            at.push((insertion_point(html.len(), &blocks, p.a), at.len(), row_html(s, cx)));
        }
        if !s.is_decided() {
            for o in &s.options {
                let Some(p) = &o.place else { continue };
                let Some(i) = blocks.iter().position(|b| b.a == p.a && b.name == p.block) else { continue };
                let off = leading_edge(html, &blocks, i);
                let label = format!("{}: {}", s.key, block_text(html, &blocks[i]));
                let checked = if mine(s, cx.me) == Some(o.id.as_str()) { " checked" } else { "" };
                let dis = if cx.can_answer { "" } else { " disabled" };
                at.push((
                    off,
                    at.len(),
                    format!(
                        "<input type=\"radio\" class=\"cd-qx cd-qradio\" name=\"cd-q-{k}\" data-q=\"{k}\" data-o=\"{}\" aria-label=\"{}\"{checked}{dis}>",
                        esc(&o.id),
                        esc(&label),
                        k = esc(&s.key)
                    ),
                ));
            }
            // The decisions table's Decided cell: the row's cells in order, column `cell`.
            if let (Some(row), Some(cell)) = (s.row, s.cell)
                && let Some(tr) = blocks.iter().position(|b| b.name == "tr" && b.a == row)
                && let Some(td) = blocks.iter().enumerate().filter(|(_, b)| b.parent == Some(tr)).nth(cell as usize)
            {
                at.push((td.1.close_start, at.len(), cell_html(s, cx)));
            }
        }
    }
    at.sort_by_key(|(off, n, _)| (*off, *n));
    let mut out = String::with_capacity(html.len() + at.iter().map(|x| x.2.len()).sum::<usize>());
    let mut last = 0;
    for (off, _, piece) in &at {
        let off = (*off).min(html.len()).max(last);
        out.push_str(&html[last..off]);
        out.push_str(piece);
        last = off;
    }
    out.push_str(&html[last..]);
    out
}
