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
