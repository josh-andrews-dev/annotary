//! The rendered diff of two Versions (`annotary_diff::rendered_diff`) over real renderer output:
//! what is marked how, that the output stays valid HTML, and that taking the marks away gives
//! back each Version's text (over the made-up corpus's histories).

use std::path::PathBuf;

use annotary_diff::{RenderedDiff, rendered_diff};

fn render(md: &str) -> String {
    annotary_renderer::render_html(md, &[])
}

fn diff(a: &str, b: &str) -> RenderedDiff {
    rendered_diff(a, b, &render(a), &render(b))
}

const VOID: &[&str] = &["area", "br", "col", "embed", "hr", "img", "input", "source", "track", "wbr"];

/// Every open tag is closed in order (void elements excepted).
fn assert_balanced(html: &str) {
    let mut stack: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(p) = rest.find('<') {
        rest = &rest[p..];
        let e = rest.find('>').expect("a tag ends") + 1;
        let tag = &rest[..e];
        rest = &rest[e..];
        if tag.starts_with("<!--") {
            continue;
        }
        let closing = tag.starts_with("</");
        let name: String = tag[if closing { 2 } else { 1 }..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        if closing {
            assert_eq!(stack.pop().as_deref(), Some(name.as_str()), "unbalanced </{name}> in\n{html}");
        } else if !VOID.contains(&name.as_str()) && !tag.ends_with("/>") {
            stack.push(name);
        }
    }
    assert!(stack.is_empty(), "unclosed {stack:?} in\n{html}");
}

/// The text of `html` (no tags, no whitespace), leaving out every element `skip` says to.
fn text(html: &str, skip: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::new();
    let mut skipping: Vec<String> = Vec::new(); // the element being skipped, and nested same-name
    let mut rest = html;
    loop {
        let (txt, tail) = match rest.find('<') {
            Some(p) => rest.split_at(p),
            None => (rest, ""),
        };
        if skipping.is_empty() {
            out.extend(txt.chars().filter(|c| !c.is_whitespace()));
        }
        if tail.is_empty() {
            break;
        }
        let e = tail.find('>').unwrap() + 1;
        let tag = &tail[..e];
        rest = &tail[e..];
        let closing = tag.starts_with("</");
        let name: String = tag[if closing { 2 } else { 1 }..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if VOID.contains(&name.as_str()) {
            continue;
        }
        if !skipping.is_empty() {
            if closing && skipping.last() == Some(&name) {
                skipping.pop();
            } else if !closing && skipping.first() == Some(&name) {
                skipping.push(name);
            }
            continue;
        }
        if !closing && skip(tag) {
            skipping.push(name);
        }
    }
    out
}

fn is_label(tag: &str) -> bool {
    tag.contains("class=\"cd-sr\"") || tag.contains("cd-dx-sum")
}
fn added(tag: &str) -> bool {
    tag.starts_with("<ins") || tag.contains("cd-dx-add")
}
fn removed(tag: &str) -> bool {
    tag.starts_with("<del") || tag.contains("cd-dx-del")
}

fn same(got: &str, want: &str, side: &str, html: &str) {
    if got == want {
        return;
    }
    let at = got.chars().zip(want.chars()).take_while(|(a, b)| a == b).count();
    let ctx = |s: &str| s.chars().skip(at.saturating_sub(40)).take(100).collect::<String>();
    panic!("the {side} side differs at char {at}:\n got: {}\nwant: {}\n{html}", ctx(got), ctx(want));
}

/// Without the marks of one side, the diff reads as the other Version.
fn assert_reads_back(a: &str, b: &str) {
    let d = diff(a, b);
    assert_balanced(&d.html);
    let new_text = text(&render(b), &|_| false);
    let old_text = text(&render(a), &|_| false);
    same(&text(&d.html, &|t| is_label(t) || removed(t)), &new_text, "newer", &d.html);
    same(&text(&d.html, &|t| is_label(t) || added(t)), &old_text, "older", &d.html);
}

// spec: S40-01
#[test]
fn unchanged_added_removed_and_changed_blocks() {
    let a = "# Plan\n\nWe ship the beta on Friday with three people.\n\n## Risks\n\nGone paragraph here.\n\nKept at the end.\n";
    let b = "# Plan\n\nNew opening paragraph.\n\nWe ship the beta on Monday with four people &amp; a dog.\n\n## Risks\n\nKept at the end.\n";
    let d = diff(a, b);
    assert_balanced(&d.html);
    // Unchanged blocks are the newer render, line numbers and ids included.
    assert!(d.html.contains("<h1 data-l=\"1-1\" id=\"plan\">Plan</h1>"), "{}", d.html);
    assert!(d.html.contains("<h2 data-l=\"7-7\" id=\"risks\">Risks</h2>"), "{}", d.html);
    // Added whole, with a text label.
    assert!(
        d.html.contains("<p data-l=\"3-3\" class=\"cd-dx-add cd-dx-blk\" data-cd-dx=\"1\"><span class=\"cd-sr\">added: </span>New opening paragraph.</p>"),
        "{}",
        d.html
    );
    // Changed word by word, entities whole, the markup the newer block's.
    assert!(
        d.html.contains("We ship the beta on <del class=\"cd-del\">Friday</del><ins class=\"cd-ins\">Monday</ins> with <del class=\"cd-del\">three</del><ins class=\"cd-ins\">four</ins> people<ins class=\"cd-ins\"> &amp; a dog</ins>."),
        "{}",
        d.html
    );
    assert!(
        d.html
            .contains("<p data-l=\"5-5\" class=\"cd-dx-mod\" data-cd-dx=\"2\"><span class=\"cd-sr\">changed: </span>")
    );
    // Removed whole: no line numbers, struck by class, labeled.
    assert!(
        d.html.contains("<p class=\"cd-dx-del cd-dx-blk\" data-cd-dx=\"3\"><span class=\"cd-sr\">removed: </span>Gone paragraph here.</p>"),
        "{}",
        d.html
    );
    assert_eq!((d.stats.blocks_added, d.stats.blocks_removed, d.stats.blocks_changed, d.stats.changes), (1, 1, 1, 3));
    assert_reads_back(a, b);

    // A removed heading loses its id (the newer Version may use the same one).
    let d = diff("# T\n\n## Goals\n\ntext\n", "# T\n\ntext\n");
    assert!(
        d.html.contains(
            "<h2 class=\"cd-dx-del cd-dx-blk\" data-cd-dx=\"1\"><span class=\"cd-sr\">removed: </span>Goals</h2>"
        ),
        "{}",
        d.html
    );

    // Rewritten past recognition: one removed block and one added, one change point.
    let d = diff("# T\n\nThe cat sat on the mat.\n", "# T\n\nEntirely different words appear now.\n");
    assert_eq!((d.stats.blocks_removed, d.stats.blocks_added, d.stats.blocks_changed, d.stats.changes), (1, 1, 0, 1));
    assert!(!d.html.contains("<ins class=\"cd-ins\">"), "{}", d.html);

    // Code: diffed inside the block; punctuation counts as a change.
    let d = diff("```\nlet x = 1;\n```\n", "```\nlet x = 2;;\n```\n");
    assert!(
        d.html.contains("<code>let x = <del class=\"cd-del\">1</del><ins class=\"cd-ins\">2;</ins>;"),
        "{}",
        d.html
    );

    // Media compare whole: a new image is a removed image and an added one.
    let d = diff("![a](https://example.com/a.png) caption\n", "![a](https://example.com/b.png) caption\n");
    assert!(d.html.contains("<del class=\"cd-del\"><img src=\"https://example.com/a.png\""), "{}", d.html);
    assert!(d.html.contains("<ins class=\"cd-ins\"><img src=\"https://example.com/b.png\""), "{}", d.html);
    assert_balanced(&d.html);
}

// spec: S40-02
#[test]
fn lists_tables_quotes_and_footnotes_diff_by_item() {
    let a = "- one\n- two\n- three\n  - nested a\n  - nested b\n\n| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n> quoted line one\n>\n> quoted two\n\nNote[^1].\n\n[^1]: The footnote text.\n";
    let b = "- one\n- two and a half\n- four\n- three\n  - nested a\n  - nested c\n\n| a | b |\n|---|---|\n| 1 | 5 |\n\n> quoted line one changed\n>\n> quoted two\n\nNote[^1].\n\n[^1]: The footnote words.\n";
    let d = diff(a, b);
    assert_balanced(&d.html);
    // A list's children are list items, and nothing else, after the diff too.
    assert!(d.html.contains("<li data-l=\"2-2\" class=\"cd-dx-mod\" data-cd-dx=\"1\"><span class=\"cd-sr\">changed: </span>two<ins class=\"cd-ins\"> and a half</ins></li>"), "{}", d.html);
    assert!(
        d.html
            .contains("<li data-l=\"3-3\" class=\"cd-dx-add cd-dx-blk\"><span class=\"cd-sr\">added: </span>four</li>"),
        "{}",
        d.html
    );
    assert!(d.html.contains("<li data-l=\"6-6\" class=\"cd-dx-mod\""), "a nested item: {}", d.html);
    // A table keeps its columns: cells pair by position, a removed row is a row.
    assert!(d.html.contains("<td data-l=\"10-10\" class=\"cd-dx-mod\" data-cd-dx=\"3\"><span class=\"cd-sr\">changed: </span><del class=\"cd-del\">2</del><ins class=\"cd-ins\">5</ins></td>"), "{}", d.html);
    assert!(
        d.html.contains(
            "<tr class=\"cd-dx-del cd-dx-blk\"><td><span class=\"cd-sr\">removed: </span>3</td><td>4</td></tr>"
        ),
        "{}",
        d.html
    );
    // A quote's paragraphs and a footnote's text are diffed inside their own blocks.
    assert!(d.html.contains("quoted line one<ins class=\"cd-ins\"> changed</ins>"), "{}", d.html);
    assert!(
        d.html.contains("The footnote <del class=\"cd-del\">text</del><ins class=\"cd-ins\">words</ins>."),
        "{}",
        d.html
    );
    assert_reads_back(a, b);

    // Markup alone (a link target, emphasis) is a formatting change, not changed words.
    let d = diff("See [the docs](https://example.com/a) now.\n", "See [the docs](https://example.com/b) *now*.\n");
    assert!(d.html.contains("class=\"cd-dx-mod cd-dx-fmt\""), "{}", d.html);
    assert!(d.html.contains("<span class=\"cd-sr\">formatting changed: </span>"), "{}", d.html);
    assert!(!d.html.contains("cd-ins") && !d.html.contains("cd-del"), "{}", d.html);
    assert!(d.html.contains("<a href=\"https://example.com/b\">the docs</a> <em>now</em>"), "{}", d.html);
    // A task ticked: the old box removed, the new one added.
    let d = diff("- [ ] ship\n", "- [x] ship\n");
    assert!(d.html.contains("<del class=\"cd-del\"><input type=\"checkbox\" disabled></del><ins class=\"cd-ins\"><input type=\"checkbox\" checked disabled></ins>"), "{}", d.html);
}

// spec: S40-03
#[test]
fn line_counts_change_points_and_folds() {
    let body: String = (1..=20).map(|i| format!("Paragraph number {i} stays.\n\n")).collect();
    let a = format!("# Title\n\n{body}Last words here.\n");
    let b = format!("# Title\n\n{}Last words there.\n", body.replace("number 10 stays", "number ten stays"));
    let d = diff(&a, &b);
    assert_eq!((d.stats.lines_added, d.stats.lines_removed), (2, 2));
    assert_eq!(d.stats.changes, 2);
    assert!(d.html.contains("data-cd-dx=\"1\"") && d.html.contains("data-cd-dx=\"2\""));
    let first = d.html.find("data-cd-dx=\"1\"").unwrap();
    assert!(
        d.html[..first].contains("number <del class=\"cd-del\">10</del>")
            || d.html[first..].contains("number <del class=\"cd-del\">10</del>")
    );
    // The title stays, 7 folded, 2 of context each side of the change, 6 folded, 2 of context.
    let folds: Vec<&str> = d.html.matches("<details class=\"cd-dx-same\"><summary class=\"cd-dx-sum\">").collect();
    assert_eq!(folds.len(), 2, "{}", d.html);
    assert!(d.html.contains("<summary class=\"cd-dx-sum\">7 unchanged blocks</summary>"), "{}", d.html);
    assert!(d.html.contains("<summary class=\"cd-dx-sum\">6 unchanged blocks</summary>"), "{}", d.html);
    assert!(d.html.starts_with("<h1 data-l=\"1-1\" id=\"title\">Title</h1>\n<details"), "{}", &d.html[..200]);
    assert!(!d.html.contains("<details class=\"cd-dx-same\" open"), "folds start closed");
    assert_balanced(&d.html);
    assert_reads_back(&a, &b);

    // Nothing visible changed: no change point, every block but the first folded.
    let d = diff(&a, &a);
    assert_eq!(d.stats, annotary_diff::DiffStats::default());
    assert!(d.html.contains("<summary class=\"cd-dx-sum\">21 unchanged blocks</summary>"), "{}", d.html);
    // Only front matter changed: lines counted, nothing marked.
    let d = diff("---\ntitle: A\n---\n\nText.\n", "---\ntitle: B\n---\n\nText.\n");
    assert_eq!((d.stats.lines_added, d.stats.lines_removed, d.stats.changes), (1, 1, 0));
    // A short run is not folded.
    let d = diff("# T\n\na\n\nb\n\nc\n", "# T\n\na\n\nb\n\nd\n");
    assert!(!d.html.contains("<details"), "{}", d.html);
    assert_eq!(annotary_diff::line_counts("a\r\nb\n", "a\nb\nc"), (1, 0), "CRLF and a missing final newline");
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus")
}

// spec: S40-01, S40-02
#[test]
fn every_pair_of_corpus_versions_reads_back_both_ways() {
    let mut pairs = 0;
    for name in ["glossary-multilingual", "onboarding", "rfc-tile-cache"] {
        let v: Vec<String> =
            (1..=4).map(|n| std::fs::read_to_string(corpus().join(name).join(format!("v{n}.md"))).unwrap()).collect();
        for i in 0..v.len() {
            for j in 0..v.len() {
                if i != j {
                    assert_reads_back(&v[i], &v[j]);
                    pairs += 1;
                }
            }
        }
    }
    // And the single Documents against an edited copy of themselves.
    for e in std::fs::read_dir(corpus()).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "md") && p.file_name().is_some_and(|n| n != "README.md") {
            let a = std::fs::read_to_string(&p).unwrap();
            let b = a.replacen("the", "a", 3).replacen("\n\n", "\n\nAn added paragraph.\n\n", 2);
            assert_reads_back(&a, &b);
            assert_reads_back(&b, &a);
            pairs += 2;
        }
    }
    assert!(pairs > 30, "{pairs}");
}
