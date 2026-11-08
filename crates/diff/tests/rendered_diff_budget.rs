//! The work budget of a rendered diff (`annotary_diff::WORK_BUDGET`): the worst inputs inside the
//! Markdown cap stay under it, and past it the diff shows blocks whole and says so.

use std::time::{Duration, Instant};

use annotary_diff::{WORK_BUDGET, rendered_diff};

/// A shared vocabulary, so every pair is similar enough to be diffed word by word.
fn sentence(seed: usize, words: usize, step: usize) -> String {
    (0..words).map(|k| format!("w{}", (k * step + seed) % 97)).collect::<Vec<_>>().join(" ")
}

/// The same words each way, in an order that depends on `step`: no block equals another side's.
fn paragraphs(count: usize, words: usize, step: usize) -> String {
    (0..count).map(|i| format!("<p>{}</p>\n", sentence(i, words, step))).collect()
}

// spec: S40-09
#[test]
fn the_worst_inputs_stay_inside_the_budget_and_say_when_they_run_out() {
    // (paragraphs, words each), under the limits: too many to pair, 200 × 200, and few huge ones.
    for (count, words) in [(20_000, 3), (200, 250), (20, 2_000), (1_000, 900)] {
        let (a, b) = (paragraphs(count, words, 7), paragraphs(count, words, 11));
        let t = Instant::now();
        let d = rendered_diff("", "", &a, &b);
        assert!(d.work <= WORK_BUDGET, "{count} × {words}: work {}", d.work);
        // Far above what the budget costs even unoptimized: this is a hang check, the work is the limit.
        assert!(t.elapsed() < Duration::from_secs(120), "{count} × {words}: took {:?}", t.elapsed());
        assert_eq!(d.html.matches("<p").count(), d.html.matches("</p>").count(), "{count} × {words}");
    }
}

// spec: S40-09
#[test]
fn past_the_budget_blocks_are_shown_whole_and_nothing_is_lost() {
    // Changed paragraphs one at a time, so each is its own run and every one pairs.
    let spaced = |step: usize| -> String {
        (0..300).map(|i| format!("<p>{}</p>\n<p>keep {i}</p>\n", sentence(i, 500, step))).collect()
    };
    let (a, b) = (spaced(7), spaced(11));
    let d = rendered_diff("", "", &a, &b);
    assert!(d.stats.shown_whole, "this input needs more than the budget: {}", d.work);
    assert!(d.work <= WORK_BUDGET);
    assert_eq!(d.html.matches("<p").count(), d.html.matches("</p>").count());
    assert!(d.html.contains("<ins class=\"cd-ins\">"));
    // An ordinary edit never gets near it.
    let (a, b) = (paragraphs(50, 80, 7), paragraphs(50, 80, 7).replace("w3 ", "changed "));
    let d = rendered_diff("", "", &a, &b);
    assert!(!d.stats.shown_whole && d.work < WORK_BUDGET / 100, "{}", d.work);
}
