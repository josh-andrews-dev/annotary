//! The worst inputs we know for `annotary_diff::rendered_diff`, each checked against the limits
//! S40-09 promises: under 2 s and 64 MiB. It prints one row per shape and fails listing every shape
//! over a limit. Ignored in `cargo test` (a debug build takes over a minute); run it with
//! `make diff-worst-case`, which is:
//!
//!     cargo test --release -p annotary-diff --test worst_case -- --ignored --nocapture --test-threads=1
//!
//! Memory is counted by this binary's allocator from the moment `rendered_diff` is called, so it
//! leaves out the two input strings and holds in any build. The time limit means nothing for an
//! unoptimized build, so a debug build checks memory only and says so in its output.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use annotary_diff::{MAX_BLOCKS, MAX_HTML_BYTES, WORK_BUDGET, rendered_diff};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to `System` unchanged; the counters do not touch the allocation.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let live = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const MIB: f64 = 1_048_576.0;
const MAX_MS: f64 = 2_000.0;
const MAX_MIB: f64 = 64.0;

#[derive(Default)]
struct Table {
    over: Vec<String>,
    rows: usize,
}

impl Table {
    /// One comparison, measured, printed and checked.
    fn run(&mut self, name: &str, md: (&str, &str), html: (&str, &str)) {
        let base = LIVE.load(Ordering::Relaxed);
        PEAK.store(base, Ordering::Relaxed);
        let t = Instant::now();
        let d = rendered_diff(md.0, md.1, html.0, html.1);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let peak = (PEAK.load(Ordering::Relaxed) - base) as f64 / MIB;
        let what = if d.stats.too_large {
            "too large"
        } else if d.stats.shown_whole {
            "partly whole"
        } else {
            "word level"
        };
        println!(
            "| {name} | {:.1} | {:.0} | {peak:.1} | {} | {what} |",
            html.0.len().max(html.1.len()) as f64 / MIB,
            ms,
            d.work / 1_000_000
        );
        self.rows += 1;
        if peak >= MAX_MIB {
            self.over.push(format!("{name}: peak {peak:.1} MiB"));
        }
        // An optimized build only: unoptimized code is several times slower and proves nothing.
        if !cfg!(debug_assertions) && ms >= MAX_MS {
            self.over.push(format!("{name}: {ms:.0} ms"));
        }
        if d.work > WORK_BUDGET {
            self.over.push(format!("{name}: work {} over the budget", d.work));
        }
        if d.html.matches("<p").count() != d.html.matches("</p>").count() {
            self.over.push(format!("{name}: unbalanced output"));
        }
    }

    fn html(&mut self, name: &str, a: &str, b: &str) {
        self.run(name, ("", ""), (a, b));
    }

    fn markdown(&mut self, name: &str, a: &str, b: &str) {
        let (x, y) = (annotary_renderer::render_html(a, &[]), annotary_renderer::render_html(b, &[]));
        self.run(name, (a, b), (&x, &y));
    }
}

fn words(tag: &str, n: usize) -> String {
    (0..n).map(|k| format!("{tag}{k}")).collect::<Vec<_>>().join(" ")
}

fn para(s: &str) -> String {
    format!("<p>{s}</p>")
}

/// `body` under `depth` levels of `open`/`close`, with `level(i)` beside it at each level.
fn nest(body: String, depth: usize, level: &dyn Fn(usize) -> String, open: &str, close: &str) -> String {
    (0..depth).fold(body, |s, i| format!("{open}{}\n{s}{close}", level(i)))
}

fn quotes(body: String, level: &dyn Fn(usize) -> String) -> String {
    nest(body, 31, level, "<blockquote>", "</blockquote>")
}

fn short_blocks(tag: &str, n: usize) -> String {
    (0..n).map(|i| format!("<p>{tag}{i}</p>\n")).collect()
}

/// Words from a vocabulary of 97, in an order that depends on `step`.
fn sentence(seed: usize, words: usize, step: usize) -> String {
    (0..words).map(|k| format!("w{}", (k * step + seed) % 97)).collect::<Vec<_>>().join(" ")
}

fn paragraphs(count: usize, words: usize, step: usize) -> String {
    (0..count).map(|i| format!("<p>{}</p>\n", sentence(i, words, step))).collect()
}

/// Changed paragraphs one at a time, an unchanged one after each, so every one pairs.
fn spaced(count: usize, words: usize, step: usize) -> String {
    (0..count).map(|i| format!("<p>{}</p>\n<p>keep {i}</p>\n", sentence(i, words, step))).collect()
}

fn reversed(count: usize, words: usize, flip: bool) -> String {
    (0..count)
        .map(|i| {
            let mut w: Vec<String> = (0..words).map(|k| format!("w{k}x{i}")).collect();
            if flip {
                w.reverse();
            }
            format!("<p>{}</p>\n<p>keep {i}</p>\n", w.join(" "))
        })
        .collect()
}

fn row(tag: &str, cells: usize) -> String {
    let c: String = (0..cells).map(|i| format!("<td>{tag}{i}</td>")).collect();
    format!("<table><tbody><tr>{c}</tr></tbody></table>\n")
}

fn repeat_to(unit: &str, bytes: usize) -> String {
    unit.repeat(bytes / unit.len())
}

fn lines(n: usize, step: usize) -> String {
    (0..n).map(|i| format!("{}\n", i * step % 97)).collect()
}

#[test]
#[ignore = "takes over a minute in a debug build; run through `make diff-worst-case`"]
fn every_known_worst_case_stays_within_the_limits() {
    let mut t = Table::default();
    println!("\n| Shape | HTML MiB | ms | peak MiB | work (M) | outcome |\n|---|---|---|---|---|---|");

    // ---- the inputs of the reviews ----
    let (a, b) = (lines(200_000, 7), lines(200_000, 11));
    t.run("review: 200,000 repeating lines (0.55 MB)", (&a, &b), ("<p>x</p>", "<p>y</p>"));
    let (a, b) = (lines(690_000, 7), lines(690_000, 11));
    t.run("review: 690,000 repeating lines (1.9 MB)", (&a, &b), ("<p>x</p>", "<p>y</p>"));
    let kids = |n: usize, tag: &str| (0..n).map(|i| format!("<p>{tag} {i}</p>\n")).collect::<String>();
    let many =
        |n: usize, tag: &'static str| nest(kids(n, tag), 31, &move |_| kids(n, tag), "<blockquote>", "</blockquote>");
    let (a, b) = (many(1_050, "a"), many(952, "b"));
    t.html("review: 31 quotes, 1,050 vs 952 children each", &a, &b);
    let (a, b) = (many(620, "a"), many(540, "b"));
    t.html("same shape, just under the block limit", &a, &b);
    let one_level = |n: usize, tag: &str| quotes(kids(n, tag), &|_| String::new());
    let (a, b) = (one_level(1_050, "a"), one_level(952, "b"));
    t.html("31 quotes, 1,050 vs 952 children at one level", &a, &b);
    let wide = |tag: &str| format!("|{}\n|{}\n", format!("{tag}|").repeat(330_000), "-|".repeat(330_000));
    t.markdown("review: 330,000-column table", &wide("a"), &wide("b"));
    let (a, b) = (row("a", MAX_BLOCKS - 3), row("b", MAX_BLOCKS - 3));
    t.html("review: widest row under the block limit", &a, &b);
    let (a, b) = (short_blocks("a", MAX_BLOCKS + 1), short_blocks("b", MAX_BLOCKS + 1));
    t.html("review: one block over the limit", &a, &b);
    let w = words("a", 275_000);
    let level = |t: &'static str| move |i: usize| para(&format!("level {i} {t}"));
    t.html(
        "review: 275k words under 31 quotes",
        &quotes(para(&w), &level("x")),
        &quotes(para(&w.replace("a1 ", "z1 ")), &level("y")),
    );
    t.html("same, no word in common", &quotes(para(&w), &level("x")), &quotes(para(&words("b", 275_000)), &level("y")));

    // ---- exactly at the limits ----
    let (a, b) = (short_blocks("a", MAX_BLOCKS), short_blocks("b", MAX_BLOCKS));
    t.html("at the block limit, all changed", &a, &b);
    let fill = MAX_HTML_BYTES / MAX_BLOCKS / 5;
    let (a, b) = (paragraphs(MAX_BLOCKS, fill, 7), paragraphs(MAX_BLOCKS, fill, 11));
    t.html("at both limits, all changed", &a, &b);
    let every_other = |step: usize| spaced(MAX_BLOCKS / 2, 2 * fill, step);
    t.html("at both limits, every other block changed", &every_other(7), &every_other(11));
    let half = |tag: &str| {
        (0..MAX_BLOCKS / 2)
            .map(|i| format!("<p id=\"id{i}\" title=\"{tag}{i}\">{tag} {} text</p>\n", sentence(i, fill / 2, 3)))
            .collect::<String>()
    };
    t.html("at the block limit, attributes", &half("a"), &half("b"));
    let tiny = |c: &str| format!("<p>{}</p>", repeat_to(&format!("{c} "), MAX_HTML_BYTES - 16));
    t.html("one paragraph of tiny words at the size limit", &tiny("a"), &tiny("b"));
    let tags = |c: &str| format!("<p>{}</p>", repeat_to(&format!("<b>{c}</b> "), MAX_HTML_BYTES - 16));
    t.html("one paragraph of inline tags at the size limit", &tags("a"), &tags("b"));
    let imgs = |c: &str| format!("<p>{}</p>", repeat_to(&format!("<img src=\"{c}\"> "), MAX_HTML_BYTES - 16));
    t.html("one paragraph of images at the size limit", &imgs("a"), &imgs("b"));
    let (a, b) = (row("a", 2_000), row("b", 2_000));
    let rows = |x: &str, n: usize| x.repeat(n);
    t.html("many wide rows", &rows(&a, 8), &rows(&b, 8));
    let items = |tag: &str, n: usize| {
        format!("<ul>{}</ul>", (0..n).map(|i| format!("<li>{tag} {} text</li>\n", i % 7)).collect::<String>())
    };
    t.html("one list at the block limit", &items("a", MAX_BLOCKS - 1), &items("b", MAX_BLOCKS - 1));
    t.html(
        "nesting to the block limit",
        &"<blockquote>".repeat(MAX_BLOCKS).replace("<blockquote><blockquote>", "<blockquote><blockquote>"),
        &format!("{}x", "<blockquote>".repeat(MAX_BLOCKS)),
    );

    // ---- the shapes of the earlier rounds ----
    let (x, y) = (
        quotes(para("deep"), &|i| para(&format!("p {}", words(&format!("l{i}x"), 8_000)))),
        quotes(para("deeper"), &|i| para(&format!("q {}", words(&format!("l{i}x"), 8_000)))),
    );
    t.html("31 levels, 8k words at each", &x, &y);
    let two = |tag: &str| {
        (0..31).fold(para(&format!("deep {tag} {}", words("q", 130_000))), |s, i| {
            format!(
                "<div><blockquote><p>a{i}</p>{s}</blockquote>\n<blockquote><p>b{i}</p><p>{tag}</p></blockquote></div>"
            )
        })
    };
    t.html("31 levels, two containers each", &two("x"), &two("y"));
    let lists = |tag: &str| {
        nest(
            format!("<ul><li>{}</li></ul>", words(tag, 250_000)),
            30,
            &|i| format!("item {i} {tag}"),
            "<ul><li>",
            "</li></ul>",
        )
    };
    t.html("nested lists 30 deep", &lists("a"), &lists("b"));
    let prefix = format!("<p>{}", words("s", 400));
    let shared = |tag: &str| (0..990).map(|i| format!("{prefix} {tag}{i}</p>\n")).collect::<String>();
    t.html("990 blocks, long shared prefix", &shared("a"), &shared("b"));
    t.html("50,000 short blocks", &short_blocks("a", 50_000), &short_blocks("b", 50_000));
    t.html("200 x 200 paragraphs, 250 words", &paragraphs(200, 250, 7), &paragraphs(200, 250, 11));
    t.html("400 reversed 500-word pairs", &reversed(400, 500, false), &reversed(400, 500, true));
    t.html("60 reversed 5,000-word pairs", &reversed(60, 5_000, false), &reversed(60, 5_000, true));
    let dense =
        |c: &str| format!("<p>{}</p>", (0..500_000).map(|i| ["a", "b", "c", c][i % 4]).collect::<Vec<_>>().join(" "));
    t.html("one paragraph of 500k tiny words", &dense("d"), &dense("e"));
    let tagged = |tag: &str| (0..60_000).map(|i| format!("<p id=\"x{i}\"><b>{tag}</b></p>\n")).collect::<String>();
    t.html("60,000 tagged blocks", &tagged("a"), &tagged("b"));
    let deep = |c: &str| format!("{}{c}{}", "<blockquote>".repeat(200_000), "</blockquote>".repeat(200_000));
    t.html("200,000 levels deep", &deep("a"), &deep("b"));
    t.html("100,000-cell row", &row("a", 100_000), &row("b", 100_000));
    let tight = |tag: &str| {
        format!("<ul>{}</ul>", (0..50_000).map(|i| format!("<li>{tag} {} text</li>\n", i % 7)).collect::<String>())
    };
    t.html("50,000 tight items", &tight("a"), &tight("b"));
    let list = |w: [&str; 3]| (0..500_000).map(|i| format!("- {}\n", w[i % 3])).collect::<String>();
    t.markdown("2 MB of '- a' items", &list(["a", "b", "c"]), &list(["d", "e", "f"]));
    let short = |w: [&str; 3]| (0..660_000).map(|i| format!("{}\n\n", w[i % 3])).collect::<String>();
    t.markdown("2 MB of 1-character paragraphs", &short(["a", "b", "c"]), &short(["d", "e", "f"]));
    t.markdown(
        "2 MB of > nesting",
        &format!("{}x\n", "> ".repeat(1_000_000)),
        &format!("{}y\n", "> ".repeat(1_000_000)),
    );
    let md = |step: usize| (0..1_000).map(|i| format!("{}\n\n", sentence(i, 300, step))).collect::<String>();
    t.markdown("1,000 x 1,000 paragraphs", &md(7), &md(11));
    let few =
        |tag: &str, step: usize| (0..50_000).map(|i| format!("<p>{tag}{}</p>\n", i * step % 97)).collect::<String>();
    t.html("50,000 blocks, few distinct, reordered", &few("k", 7), &few("k", 11));
    let alt = |tag: &str| {
        (0..50_000)
            .map(|i| if i % 2 == 0 { "<p>same</p>\n".to_string() } else { format!("<p>{tag}{}</p>\n", i % 5) })
            .collect::<String>()
    };
    t.html("50,000 blocks, alternating shared", &alt("a"), &alt("b"));
    t.html("300 pairs of 1,000 tokens, 97 words", &spaced(300, 500, 7), &spaced(300, 500, 11));
    t.html("100 pairs of 9,000 tokens, 97 words", &spaced(100, 4_500, 7), &spaced(100, 4_500, 11));
    t.html("1,000 pairs of 300 tokens", &spaced(1_000, 150, 7), &spaced(1_000, 150, 11));

    println!("\n{} shapes. Limits: {MAX_MIB:.0} MiB in every build, {MAX_MS:.0} ms in an optimized one.", t.rows);
    if cfg!(debug_assertions) {
        println!("This is a debug build: the time limit was not checked.");
    }
    assert!(t.over.is_empty(), "over a limit:\n{}", t.over.join("\n"));
}
