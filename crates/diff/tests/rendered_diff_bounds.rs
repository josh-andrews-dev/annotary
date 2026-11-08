//! What one rendered diff may cost on the worst inputs inside the Markdown cap. Peak memory is
//! counted by this binary's allocator (deterministic, so no test here depends on the machine's
//! speed except the one that names a limit far above what the bounded code needs).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use annotary_diff::rendered_diff;

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

/// The counters are process-wide: one measurement at a time.
static ONE: Mutex<()> = Mutex::new(());

/// Peak bytes allocated beyond what was live before `f`.
fn peak_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let out = f();
    (out, PEAK.load(Ordering::Relaxed) - base)
}

const MB: usize = 1 << 20;

/// `count` one-word paragraphs, each different from every paragraph of another `tag`.
fn short_blocks(count: usize, tag: &str) -> String {
    (0..count).map(|i| format!("<p>{tag}{i}</p>\n")).collect()
}

// spec: S40-09
#[test]
fn a_run_too_large_to_pair_needs_no_matrix() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (a, b) = (short_blocks(12_000, "a"), short_blocks(12_000, "b"));
    let (d, peak) = peak_during(|| rendered_diff("", "", &a, &b));
    assert_eq!((d.stats.blocks_removed, d.stats.blocks_added), (12_000, 12_000));
    assert!(peak < 64 * MB, "peak {} MB for {} KB of input", peak / MB, (a.len() + b.len()) / 1024);
}

// spec: S40-09
#[test]
fn a_row_too_wide_to_pair_keeps_its_cells_by_position_without_a_matrix() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let row = |tag: &str| {
        let cells: String = (0..12_000).map(|i| format!("<td>{tag}{i}</td>")).collect();
        format!("<table><tbody><tr>{cells}</tr></tbody></table>\n")
    };
    let (a, b) = (row("a"), row("b"));
    let (d, peak) = peak_during(|| rendered_diff("", "", &a, &b));
    assert_eq!((d.stats.blocks_changed, d.stats.blocks_removed, d.stats.blocks_added), (12_000, 0, 0));
    assert!(peak < 64 * MB, "peak {} MB", peak / MB);
}

// spec: S40-09
#[test]
fn long_paragraphs_at_the_pairing_limit_are_not_diffed_pair_by_pair() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let paragraphs = |tag: &str| -> String {
        (0..20)
            .map(|i| format!("<p>{}</p>\n", (0..400).map(|k| format!("{tag}{i}x{k}")).collect::<Vec<_>>().join(" ")))
            .collect()
    };
    let (a, b) = (paragraphs("a"), paragraphs("b"));
    let t = Instant::now();
    let d = rendered_diff("", "", &a, &b);
    assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
    assert_eq!((d.stats.blocks_removed, d.stats.blocks_added, d.stats.blocks_changed), (20, 20, 0));
}

// spec: S40-09
#[test]
fn nesting_past_the_limit_is_compared_whole_and_does_not_overflow_the_stack() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let nested = |word: &str| format!("{}{word}{}", "<blockquote>".repeat(20_000), "</blockquote>".repeat(20_000));
    let (a, b) = (nested("old"), nested("new"));
    let d = rendered_diff("", "", &a, &b);
    assert!(d.html.contains("old") && d.html.contains("new"));
    assert_eq!(d.html.matches("<blockquote").count(), d.html.matches("</blockquote>").count());
    // Unchanged, it is equal and comes out as it went in.
    let same = rendered_diff("", "", &a, &a);
    assert_eq!(same.stats.changes, 0);
    assert_eq!(same.html.matches("<blockquote").count(), 20_000);
}

/// `words` words under 31 nested quotes, with a changed paragraph beside the quote at every level,
/// so every level has two children to measure (the size of the whole subtree, per level).
fn nested_quotes(words: &str, tag: &str) -> String {
    let mut s = format!("<p>{words}</p>");
    for i in 0..31 {
        s = format!("<blockquote><p>level {i} {tag}</p>\n{s}</blockquote>");
    }
    s
}

fn word_list(tag: &str) -> String {
    (0..275_000).map(|k| format!("{tag}{k}")).collect::<Vec<_>>().join(" ")
}

/// A generous limit in a debug build, the real one where the code is built as it ships.
fn time_limit() -> Duration {
    Duration::from_secs(if cfg!(debug_assertions) { 30 } else { 2 })
}

// spec: S40-09
#[test]
fn nested_containers_are_charged_at_every_level() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (words, other) = (word_list("a"), word_list("b"));
    let shared = words.replace("a1 ", "z1 ");
    for (name, a, b) in [
        ("the same words", nested_quotes(&words, "x"), nested_quotes(&shared, "y")),
        ("no word in common", nested_quotes(&words, "x"), nested_quotes(&other, "y")),
    ] {
        let t = Instant::now();
        let (d, peak) = peak_during(|| rendered_diff("", "", &a, &b));
        assert!(t.elapsed() < time_limit(), "{name}: took {:?}", t.elapsed());
        assert!(peak < 64 * MB, "{name}: peak {} MB", peak / MB);
        assert!(d.work <= annotary_diff::WORK_BUDGET, "{name}: {}", d.work);
        // Reading the subtree is charged: at least one level's worth of it, at the rate that words cost.
        assert!(d.work > 20 * (a.len() + b.len()) as u64, "{name}: only {} charged", d.work);
        assert_eq!(d.html.matches("<blockquote").count(), d.html.matches("</blockquote>").count(), "{name}");
    }
}

// spec: S40-09
#[test]
fn a_lone_chain_of_containers_does_not_read_its_words_at_every_level() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let chain = |words: &str| format!("{}<p>{words}</p>{}", "<blockquote>".repeat(31), "</blockquote>".repeat(31));
    let words: String = (0..60_000).map(|k| format!("w{k}")).collect::<Vec<_>>().join(" ");
    let (a, b) = (chain(&words), chain(&words.replace("w5 ", "changed ")));
    let d = rendered_diff("", "", &a, &b);
    // Keys at each level and the one paragraph's words, not the words 31 times over.
    let levels = 31 * (a.len() + b.len()) as u64;
    assert!(d.work < 6 * levels, "charged {} for {levels} bytes across the levels", d.work);
}

// spec: S40-09
#[test]
fn ordinary_documents_are_never_shown_whole() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    // Flat, about 2 MB, every tenth paragraph edited.
    let para = |i: usize, w: &str| {
        format!("<p>Paragraph {i} of the plan says {w} and goes on for a while about {}.</p>\n", "the work ".repeat(8))
    };
    let flat =
        |w: &str, step: usize| (0..18_000).map(|i| para(i, if i % step == 0 { w } else { "same" })).collect::<String>();
    let (a, b) = (flat("same", 10), flat("changed", 10));
    let d = rendered_diff("", "", &a, &b);
    assert!(!d.stats.shown_whole, "flat: {} charged", d.work);
    assert_eq!(d.stats.blocks_changed, 1_800);
    assert!(d.html.contains("says <del class=\"cd-del\">same</del><ins class=\"cd-ins\">changed</ins> and"));
    // Normally nested: quotes holding lists of paragraphs, three levels, as large as a real one.
    let item = |i: usize, w: &str| format!("<li><p>Item {i} of the list says {w} and then a little more.</p></li>\n");
    let quote = |w: &str| {
        let blocks: String = (0..60)
            .map(|b| {
                let items: String = (0..100).map(|i| item(b * 100 + i, if i == 7 { w } else { "same" })).collect();
                format!("<blockquote><p>Group {b}</p><ul>{items}</ul></blockquote>\n")
            })
            .collect();
        format!("<div>{blocks}</div>")
    };
    let (a, b) = (quote("same"), quote("changed"));
    let d = rendered_diff("", "", &a, &b);
    assert!(!d.stats.shown_whole, "nested: {} charged", d.work);
    assert_eq!(d.stats.blocks_changed, 60);
    assert!(d.work < annotary_diff::WORK_BUDGET / 2, "nested: {} charged", d.work);
}

// spec: S40-09
#[test]
fn one_block_over_the_limit_stops_the_whole_comparison() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let max = annotary_diff::MAX_BLOCKS;
    let items = |tag: &str, n: usize| {
        format!("<ul>{}</ul>\n", (0..n - 1).map(|i| format!("<li>{tag}{i}</li>\n")).collect::<String>())
    };
    // Over on either side: nothing parsed, nothing compared, no piece of the Document shown.
    for (a, b) in [(items("a", max + 1), items("b", 3)), (items("a", 3), items("b", max + 1))] {
        let (d, peak) = peak_during(|| rendered_diff("", "", &a, &b));
        assert!(d.stats.too_large && d.html.is_empty(), "{:?}", d.stats);
        assert_eq!(d.work, 0);
        assert!(peak < MB, "peak {} KB: the gate builds nothing", peak / 1024);
    }
    // Table cells count: a row one over is too large as well.
    let row = |tag: &str, n: usize| {
        format!("<table><tr>{}</tr></table>", (0..n - 2).map(|i| format!("<td>{tag}{i}</td>")).collect::<String>())
    };
    assert!(rendered_diff("", "", &row("a", max + 1), &row("b", max + 1)).stats.too_large);
    // At the limit it is compared, and every item is a real item.
    let (a, b) = (items("a", max), items("b", max));
    let d = rendered_diff("", "", &a, &b);
    assert!(!d.stats.too_large);
    assert_eq!(d.html.matches("<li").count(), d.html.matches("</li>").count());
    assert!(
        !d.html.contains("<ins class=\"cd-ins\"><li") && !d.html.contains("<del class=\"cd-del\"><li"),
        "no markup inside a mark"
    );
}

// spec: S40-03, S40-09
#[test]
fn the_line_counts_cannot_blow_up() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let lines = |step: usize| (0..200_000).map(|i| format!("{}\n", i * step % 97)).collect::<String>();
    let (a, b) = (lines(7), lines(11));
    let t = Instant::now();
    let (d, peak) = peak_during(|| rendered_diff(&a, &b, "<p>x</p>", "<p>y</p>"));
    assert!(t.elapsed() < Duration::from_secs(if cfg!(debug_assertions) { 10 } else { 1 }), "took {:?}", t.elapsed());
    assert!(peak < 8 * MB, "peak {} MB", peak / MB);
    // Both lists are almost the same 97 lines in another order.
    assert_eq!(d.stats.lines_added, d.stats.lines_removed);
    assert!(d.stats.lines_added < 100, "{}", d.stats.lines_added);
    // By content: a moved line counts for neither side, a changed one for both.
    assert_eq!(annotary_diff::line_counts("a\nb\nc\n", "c\na\nb\n"), (0, 0));
    assert_eq!(annotary_diff::line_counts("a\nb\n", "a\nx\n"), (1, 1));
    assert_eq!(annotary_diff::line_counts("a\na\n", "a\n"), (0, 1), "a repeated line counts each time");
}

// spec: S40-09
#[test]
fn the_pairing_matrices_are_gone_before_the_recursion() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let kids = |n: usize, tag: &str| (0..n).map(|i| format!("<p>{tag} {i}</p>\n")).collect::<String>();
    // The quote comes first at each level, so it pairs and the recursion runs with the level's
    // matrices already built: about 330,000 cells at each of 31 levels, 1.6 MB each if they stayed.
    let nested = |n: usize, tag: &str| {
        (0..31).fold(kids(n, tag), |s, _| format!("<blockquote>{s}{}</blockquote>", kids(n, tag)))
    };
    let (a, b) = (nested(620, "a"), nested(540, "b"));
    let (d, peak) = peak_during(|| rendered_diff("", "", &a, &b));
    assert!(!d.stats.too_large, "{:?}", d.stats);
    assert!(peak < 24 * MB, "peak {} MB", peak / MB);
}

// spec: S40-09
#[test]
fn a_row_too_wide_to_diff_within_the_budget_stays_one_row() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    // Reversed long paragraphs leave too little budget for a block diff of a row this wide, but
    // enough to reach the row at all.
    let spend = |flip: bool| {
        (0..55)
            .map(|i| {
                let mut w: Vec<String> = (0..500).map(|k| format!("w{k}x{i}")).collect();
                if flip {
                    w.reverse();
                }
                format!("<p>{}</p>\n<p>keep {i}</p>\n", w.join(" "))
            })
            .collect::<String>()
    };
    let cells = 19_000;
    let table = |tag: &str| {
        format!(
            "<table><tbody><tr>{}</tr></tbody></table>\n",
            (0..cells).map(|i| format!("<td>{tag}{i}</td>")).collect::<String>()
        )
    };
    let (a, b) = (format!("{}{}", spend(false), table("a")), format!("{}{}", spend(true), table("b")));
    let d = rendered_diff("", "", &a, &b);
    assert!(d.work > annotary_diff::WORK_BUDGET / 2, "the paragraphs spent the budget: {}", d.work);
    assert_eq!(d.html.matches("<td").count(), cells, "one cell per column");
    assert!(d.stats.blocks_changed >= cells, "every cell compared in place: {:?}", d.stats);
    assert!(d.html.contains("<del class=\"cd-del\">a0</del><ins class=\"cd-ins\">b0</ins>"));
}

// spec: S40-09
#[test]
fn a_version_that_renders_past_the_size_limit_is_not_compared() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let at = |word: &str, bytes: usize| format!("<p>{word}</p>\n").repeat(bytes / (word.len() + 8) + 1);
    let (a, b) = (at("old", annotary_diff::MAX_HTML_BYTES + 1), "<p>new</p>\n".to_string());
    let (d, peak) = peak_during(|| rendered_diff("a\nb\n", "a\n", &a, &b));
    assert!(d.stats.too_large && d.html.is_empty(), "{:?}", d.stats);
    assert_eq!((d.stats.lines_added, d.stats.lines_removed, d.work), (0, 1, 0), "the line count still holds");
    assert!(peak < MB, "peak {} KB", peak / 1024);
    // Just under the limit it is compared (one long paragraph, so the block limit is not what stops it).
    let c = format!("<p>{}</p>", "x".repeat(annotary_diff::MAX_HTML_BYTES - 16));
    assert!(c.len() <= annotary_diff::MAX_HTML_BYTES);
    assert!(!rendered_diff("", "", &c, &b).stats.too_large);
}
