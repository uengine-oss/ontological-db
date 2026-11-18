//! Every statement of the compatibility corpus, parsed with the engine's
//! parser and written out as its tree (`{:?}`) — to show that a change to the
//! shared parser leaves what the engine reads unchanged:
//!
//! ```text
//! OG_CORPUS_DUMP=/tmp/before.txt cargo test --test corpus_dump -- --ignored
//! # change the parser
//! OG_CORPUS_DUMP=/tmp/after.txt OG_CORPUS_DUMP_DROP=label_groups \
//!     cargo test --test corpus_dump -- --ignored
//! cmp /tmp/before.txt /tmp/after.txt
//! ```
//!
//! `OG_CORPUS_DUMP_DROP` names fields (comma-separated) left out of the dump:
//! a field the change adds, which the engine does not read.
//!
//! Read: `tests/compat/corpus/*/queries/*.cypher` (one statement each) and
//! `tests/compat/corpus/*/fixture/*.cypher` (statements ending in `;`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "cypher") {
            out.push(p);
        }
    }
}

/// A fixture file's statements: split after a `;` that ends a line.
fn statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        cur.push_str(line);
        cur.push('\n');
        if line.trim_end().ends_with(';') {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|s| s.trim().trim_end_matches(';').to_string())
        .filter(|s| s.lines().any(|l| !l.trim().is_empty() && !l.trim_start().starts_with("//")))
        .collect()
}

/// The end of the value that starts at `at` in a `{:?}` dump: a bracketed
/// value to its closing bracket, anything else to the next `,` / `}` / `)` /
/// `]` at depth 0. Quoted strings are skipped whole.
fn value_end(s: &[u8], mut at: usize) -> usize {
    let mut depth = 0i32;
    while at < s.len() {
        match s[at] {
            b'"' => {
                at += 1;
                while at < s.len() && s[at] != b'"' {
                    if s[at] == b'\\' {
                        at += 1;
                    }
                    at += 1;
                }
            }
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => {
                if depth == 0 {
                    return at;
                }
                depth -= 1;
                if depth == 0 {
                    return at + 1;
                }
            }
            b',' if depth == 0 => return at,
            _ => {}
        }
        at += 1;
    }
    at
}

/// `dump` without `, field: value` for each named field.
fn drop_fields(dump: &str, fields: &[String]) -> String {
    let mut s = dump.to_string();
    for f in fields {
        let key = format!(", {f}: ");
        while let Some(at) = s.find(&key) {
            let end = value_end(s.as_bytes(), at + key.len());
            s.replace_range(at..end, "");
        }
    }
    s
}

#[test]
#[ignore = "writes the corpus dump named by OG_CORPUS_DUMP"]
fn dump_the_corpus() {
    let Ok(out_path) = std::env::var("OG_CORPUS_DUMP") else {
        panic!("set OG_CORPUS_DUMP to the file to write");
    };
    let drop: Vec<String> = std::env::var("OG_CORPUS_DUMP_DROP")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/compat/corpus");
    let mut all = Vec::new();
    files(&root, &mut all);
    assert!(!all.is_empty(), "no corpus under {}", root.display());
    let mut out = String::new();
    let (mut n, mut ok) = (0usize, 0usize);
    for f in &all {
        let text = std::fs::read_to_string(f).unwrap();
        let rel = f.strip_prefix(&root).unwrap().display().to_string();
        let stmts = if rel.contains("/fixture/") { statements(&text) } else { vec![text] };
        for (k, st) in stmts.iter().enumerate() {
            n += 1;
            let _ = writeln!(out, "== {rel}#{k}");
            match cypher_syntax::parse(st) {
                Ok(q) => {
                    ok += 1;
                    let _ = writeln!(out, "{}", drop_fields(&format!("{q:?}"), &drop));
                }
                Err(e) => {
                    let _ = writeln!(out, "ERR {e}");
                }
            }
        }
    }
    let _ = writeln!(out, "== {n} statements, {ok} parsed");
    std::fs::write(&out_path, out).unwrap();
    eprintln!("{n} statements, {ok} parsed -> {out_path}");
}

#[test]
fn a_dropped_field_leaves_the_rest() {
    let d = r#"NodePat { var: Some("n"), labels: ["A", "B"], label_groups: [["A"], ["B, ]"]], props: [] }"#;
    assert_eq!(drop_fields(d, &["label_groups".into()]), r#"NodePat { var: Some("n"), labels: ["A", "B"], props: [] }"#);
    assert_eq!(drop_fields("X { a: 1, b: Some(2) }", &["b".into()]), "X { a: 1 }");
    assert_eq!(statements("// c\nMATCH (n)\nRETURN n;\n\nCREATE (m);\n// end\n"), vec!["// c\nMATCH (n)\nRETURN n", "CREATE (m)"]);
}
