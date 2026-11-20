//! Properties of the decisions-table parser (`annotary_renderer::questions`).

use annotary_renderer::questions::{AnswerBrief, check_recorded, decided_text, fill_decided, find, from_json, to_json};
use proptest::prelude::*;

fn key() -> impl Strategy<Value = String> {
    "[A-Z][0-9]{1,2}"
}

/// A Document with a decisions table of `rows` and a section per key.
fn doc(rows: &[(String, usize, bool)], prose: &str) -> String {
    let mut md = String::from("# Plan\n\n| Decision | Options | Recommended | Decided |\n|---|---|---|---|\n");
    for (k, n, filled) in rows {
        let opts: Vec<String> = (0..*n).map(|i| ((b'A' + i as u8) as char).to_string()).collect();
        md.push_str(&format!("| {k} | {} | A | {} |\n", opts.join(", "), if *filled { "B" } else { "" }));
    }
    md.push('\n');
    for (k, n, _) in rows {
        md.push_str(&format!("## {k}. Title\n\n{prose}\n\n"));
        for i in 0..*n {
            md.push_str(&format!("- {}. option text\n", (b'A' + i as u8) as char));
        }
        md.push('\n');
    }
    md
}

fn rows() -> impl Strategy<Value = Vec<(String, usize, bool)>> {
    proptest::collection::vec((key(), 2usize..=6, any::<bool>()), 1..6).prop_map(|mut v| {
        let mut seen = std::collections::BTreeSet::new();
        v.retain(|(k, _, _)| seen.insert(k.clone()));
        v
    })
}

proptest! {
    // spec: S50-01
    #[test]
    fn any_text_parses_without_panic(md in "(\\PC|\n|\\|){0,400}") {
        let f = find(&md);
        let lines = md.split('\n').count() as u32;
        for q in &f.questions {
            prop_assert!(q.row >= 1 && q.row <= lines);
            if let Some((a, b)) = q.lines { prop_assert!(a <= b && b <= lines); }
            prop_assert_eq!(from_json(&to_json(std::slice::from_ref(q))), Some(vec![q.clone()]));
        }
        let _ = fill_decided(&md, &[("D1".into(), "A".into())]);
    }

    // spec: S50-01, S50-05
    #[test]
    fn generated_tables_are_found_with_their_places(rows in rows(), prose in "[a-z ]{0,30}") {
        let md = doc(&rows, &prose);
        let f = find(&md);
        prop_assert!(f.warnings.is_empty(), "{:?}", f.warnings);
        prop_assert_eq!(f.questions.len(), rows.len());
        for ((k, n, filled), q) in rows.iter().zip(&f.questions) {
            prop_assert_eq!(&q.key, k);
            prop_assert_eq!(q.options.len(), *n);
            prop_assert_eq!(q.decided.is_some(), *filled);
            prop_assert_eq!(q.places.len(), *n, "{:?}", q);
            prop_assert!(q.place.as_ref().is_some_and(|p| p.block == "h2"));
        }
    }

    // spec: S50-05
    #[test]
    fn fill_changes_only_the_empty_decided_cells(rows in rows(), who in "[a-z]{1,8}") {
        let md = doc(&rows, "text");
        let texts: Vec<(String, String)> = rows.iter().map(|(k, _, _)| {
            let a = AnswerBrief { handle: who.clone(), option: "A".into(), at: "2026-11-19T00:00:00Z".into() };
            (k.clone(), decided_text(&[a], None).unwrap())
        }).collect();
        let (out, n) = fill_decided(&md, &texts);
        prop_assert_eq!(n, rows.iter().filter(|r| !r.2).count());
        let (before, after): (Vec<&str>, Vec<&str>) = (md.split('\n').collect(), out.split('\n').collect());
        prop_assert_eq!(before.len(), after.len());
        let f0 = find(&md);
        let f1 = find(&out);
        let row_lines: Vec<usize> = f0.questions.iter().map(|q| q.row as usize - 1).collect();
        for (i, (b, a)) in before.iter().zip(&after).enumerate() {
            if !row_lines.contains(&i) { prop_assert_eq!(b, a); }
        }
        for ((q0, q1), (_, _, filled)) in f0.questions.iter().zip(&f1.questions).zip(&rows) {
            prop_assert_eq!(&q0.digest, &q1.digest);
            prop_assert_eq!(&q0.places, &q1.places);
            let want = if *filled { "B".to_string() } else { format!("A (@{who}, 2026-11-19)") };
            prop_assert_eq!(q1.decided.as_deref(), Some(want.as_str()));
        }
    }

    // spec: S50-05
    #[test]
    fn fill_keeps_a_short_rows_last_cell(
        tails in proptest::collection::vec(prop_oneof![
            Just("x"), Just("x |"), Just("note \\|"), Just("path\\\\|"), Just("a\\\\"), Just("`a\\|`"), Just("")
        ], 1..6),
    ) {
        let mut md = String::from("| Decision | Options | Note | Decided |\n|---|---|---|---|\n");
        for (i, t) in tails.iter().enumerate() {
            md.push_str(&format!("| D{i} | A, B | {t}\n"));
        }
        let notes: Vec<String> = row_notes(&md);
        let fills: Vec<(String, String)> = (0..tails.len()).map(|i| (format!("D{i}"), "A (@rowan)".to_string())).collect();
        let (out, n) = fill_decided(&md, &fills);
        prop_assert_eq!(n, tails.len());
        let f = find(&out);
        prop_assert_eq!(f.questions.len(), tails.len());
        for q in &f.questions {
            prop_assert_eq!(q.decided.as_deref(), Some("A (@rowan)"), "{}", out);
        }
        prop_assert_eq!(row_notes(&out), notes, "{}", out);
    }
}

/// Each body row's `Note` cell as the parser reads it (column 3).
fn row_notes(md: &str) -> Vec<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let (mut out, mut col, mut cell, mut head) = (Vec::new(), 0, String::new(), false);
    for e in Parser::new_ext(md, Options::ENABLE_TABLES) {
        match e {
            Event::Start(Tag::TableHead) => head = true,
            Event::End(TagEnd::TableHead) => head = false,
            Event::Start(Tag::TableRow) => col = 0,
            Event::Start(Tag::TableCell) => cell.clear(),
            Event::Text(t) | Event::Code(t) => cell.push_str(&t),
            Event::End(TagEnd::TableCell) => {
                col += 1;
                if !head && col == 3 {
                    out.push(cell.trim().to_string());
                }
            }
            _ => {}
        }
    }
    out
}

/// `find` and `check_recorded` on a 100-row table in a large Document read the text a bounded
/// number of times, not once per Question. Ratios, so a slow machine does not fail them:
/// `check_recorded` costs about one `fill_decided` of every row (was one per row), and `find`
/// with 100 keys a few times `find` with one (each key scanned every block before).
// spec: S50-06
#[test]
fn a_full_table_on_a_large_document_stays_linear() {
    let doc = |rows: usize| {
        let mut md = String::from("| Decision | Options | Decided |\n|---|---|---|\n");
        for i in 0..rows {
            md.push_str(&format!("| D{i} | A, B | |\n"));
        }
        for i in 0..100 {
            md.push_str(&format!("\n## D{i}. Title\n\n- A. one\n- B. two\n\n"));
            for _ in 0..200 {
                md.push_str("Filler prose that makes the Document large enough to notice a pass per row.\n");
            }
        }
        md
    };
    let best = |f: &dyn Fn()| {
        (0..3)
            .map(|_| {
                let t = std::time::Instant::now();
                f();
                t.elapsed()
            })
            .min()
            .unwrap()
    };
    let (md, md1) = (doc(100), doc(1));
    let head = find(&md).questions;
    assert_eq!(head.len(), 100);
    assert!(head.iter().all(|q| q.places.len() == 2), "{head:?}");
    let texts: Vec<(String, String)> = head.iter().map(|q| (q.key.clone(), "A (@rowan)".to_string())).collect();
    let r = check_recorded(&head, &md, &head, &texts);
    assert_eq!(r.missing.len(), 100);
    assert_eq!(r.missing[99].text, "| D99 | A, B | A (@rowan) |");
    let one = best(&|| drop(fill_decided(&md, &texts)));
    let check = best(&|| drop(check_recorded(&head, &md, &head, &texts)));
    assert!(check < one * 10, "check_recorded {check:?} vs one fill {one:?}");
    let (many, single) = (best(&|| drop(find(&md))), best(&|| drop(find(&md1))));
    assert!(many < single * 6, "find with 100 keys {many:?} vs one key {single:?}");
}
