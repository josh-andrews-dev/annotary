//! Dollar amounts in prose are not inline math: Pandoc's rule that a closing `$` is not followed
//! by a digit, applied before the render (`html::escape_currency_math`).

use annotary_renderer::html::{CURRENCY_PASSES, escape_currency_math};
use annotary_renderer::{MIN_RENDERER_VERSION, Options, RENDERER_VERSION, is_current, render, render_html, title};

const COOKIES: &str =
    "Your ingredient cost per pack is maybe $1.00–1.25 (a 24-pack batch is 72 cookies out of a ~$120 shop), so...";
const PACKS: &str = "$4 for a 3-pack, $5 for two";

fn comment(md: &str) -> String {
    render(md, &Options { assets: &[], embeds: false }).html
}

// spec: S44-01
#[test]
fn dollar_amounts_in_prose_render_as_text() {
    for md in [COOKIES, PACKS] {
        let want = format!("<p data-l=\"1-1\">{md}</p>\n");
        assert_eq!(render_html(md, &[]), want, "Document body");
        assert_eq!(comment(md), want, "Comment body");
    }
    let both = format!("{COOKIES}\n\n{PACKS}\n");
    assert!(!render_html(&both, &[]).contains("<math"), "{}", render_html(&both, &[]));
}

// spec: S44-01
#[test]
fn a_span_found_once_its_neighbors_are_escaped_is_escaped_too() {
    // `$a 1   $` cannot close, so the first `$` is text until `$a$1` is escaped; then it pairs
    // with the last `$`, which is followed by a digit as well.
    let md = "$ $a 1   $a$1$1";
    assert_eq!(escape_currency_math(md), "$ \\$a 1   \\$a\\$1\\$1");
    assert!(!render_html(md, &[]).contains("<math"));
}

// spec: S44-02
#[test]
fn real_math_still_renders() {
    let html = render_html("$x^2$ and $2^n$ grows, ($a$), and $5 or $10 cost.", &[]);
    assert_eq!(html.matches("<math").count(), 3, "{html}");
    assert!(html.contains("<msup><mi>x</mi><mn>2</mn></msup>"), "{html}");
    assert!(html.contains("<msup><mn>2</mn><mi>n</mi></msup>"), "{html}");
    assert!(html.contains("(<math xmlns=\"http://www.w3.org/1998/Math/MathML\" display=\"inline\"><mi>a</mi></math>)"));
    assert!(html.contains("$5 or $10 cost."), "{html}");
    assert!(render_html("$2^n$ at the start of a sentence.", &[]).starts_with("<p data-l=\"1-1\"><math"));

    // Display math is left alone, even before a digit.
    let display = render_html("Then $$e = mc^2$$2 follows.", &[]);
    assert!(display.contains("display=\"block\""), "{display}");
    let fence = render_html("```math\n\\$4 x\\$5\n```\n", &[]);
    assert!(fence.contains("<math") && fence.contains("display=\"block\""), "{fence}");
}

// spec: S44-02
#[test]
fn dollars_outside_prose_stay_as_written() {
    let md = "`$4 x$5` and <https://example.com/$4x$5> and <span title=\"$4x$5\">$4x$5</span>\n\n```\n$4 x$5\n```\n";
    assert_eq!(escape_currency_math("`$4 x$5`\n\n```\n$4 x$5\n```\n"), "`$4 x$5`\n\n```\n$4 x$5\n```\n");
    let html = render_html(md, &[]);
    assert!(html.contains("<code>$4 x$5</code>"), "{html}");
    assert!(html.contains("href=\"https://example.com/$4x$5\""), "{html}");
    assert!(html.contains("title=\"$4x$5\""), "{html}");
    assert!(html.contains("<pre data-l=\"3-5\"><code>$4 x$5\n</code></pre>"), "{html}");
    assert!(!html.contains('\\'), "{html}");
}

// spec: S44-03
#[test]
fn escaping_moves_no_line() {
    let md = format!("Intro line\n{PACKS}\nand {COOKIES}\n\n# Pay {PACKS}\n\n- {PACKS}\n");
    let html = render_html(&md, &[]);
    assert!(!html.contains("<math"), "{html}");
    assert!(html.contains("<p data-l=\"1-3\">"), "{html}");
    assert!(html.contains("<h1 data-l=\"5-5\" id=\"pay-4-for-a-3-pack-5-for-two\">"), "{html}");
    assert!(html.contains("<li data-l=\"7-7\">"), "{html}");
    assert_eq!(title(&md).as_deref(), Some(&*format!("Pay {PACKS}")));
}

// spec: S44-03
#[test]
fn crafted_nested_dollars_stop_at_the_pass_cap() {
    // Found by a search for inputs that reveal one new span per pass (about one per `{` level).
    let md = "$${$${$1$a$${$$$1{$1} aa{1}1{${${1a a$$${a$$1$1$1}1{{1}}a{a}$1$1}}$1}$1$1 $aa$11{${{ } 1{a1 }11}$}{1}$11a}aa$1$1a1 }$111$1}a a$1$11a$a 1a1a}aa1a1111{{1aaa{{}11  $}$1{}11}{ $ 11}$11$$a 1} $1} a{$1 ${}$";
    let once = escape_currency_math(md).into_owned();
    assert!(once.len() > md.len());
    assert_ne!(escape_currency_math(&once), once, "the cap of {CURRENCY_PASSES} passes, not the fixpoint, stopped it");
    render_html(md, &[]);
}

// spec: S44-04
#[test]
fn renders_before_the_dollar_rule_are_not_current() {
    assert_eq!(RENDERER_VERSION, "1.1.0");
    assert_eq!(MIN_RENDERER_VERSION, "1.1.0");
    assert!(!is_current(Some("1.0.0")));
    assert!(is_current(Some(RENDERER_VERSION)));
}
