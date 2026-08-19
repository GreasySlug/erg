//! Runs the formatter over every `.er` file in the repository.
//!
//! This is the non-destructiveness check from the design doc (§9), and the one
//! test that will keep meaning something as the rendering stages land: whatever
//! the formatter starts doing to these files, it must never trip its own safety
//! net, and running it twice must change nothing the second time.
//!
//! It deliberately does *not* assert what the output looks like. Golden files
//! come with the stages that produce output; this only pins that real code goes
//! through without the formatter breaking it.

use erg_fmt::{format_str, try_format_str, FmtOptions, Unformatted};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Every corpus swept, relative to the repository root, and whether all of it
/// is expected to lex.
///
/// `false` marks a corpus holding deliberately malformed fixtures, where a file
/// the lexer rejects is the point. Everywhere else an unlexable file is a
/// regression worth failing on: the sweep *skips* what it cannot lex, so a
/// lexer bug would otherwise turn a whole corpus into a no-op while the test
/// went on passing.
///
/// [`no_er_file_is_left_unswept`] checks this list against the tree, so a
/// corpus added later cannot quietly go unswept.
const SWEPT: &[(&str, bool)] = &[
    // the shipped standard library: the largest body of Erg in the tree and the
    // least like the examples -- declaration files, dense signatures, and the
    // only place `\` continuations appear in quantity
    ("crates/erg_compiler/lib", true),
    ("examples", true),
    ("tests/should_ok", true),
    // sources that are *meant* to be broken. The formatter still must not blow
    // up on them: editors format files that are mid-edit and full of errors
    ("tests/should_err", false),
    ("tests/not_yet", true),
    ("tests/linter", true),
    // the same fixtures for each Python version; swept three times over so the
    // coverage check below stays exhaustive without an exception
    ("tests/bytecode312", true),
    ("tests/bytecode313", true),
    ("tests/bytecode314", true),
    // written to exercise the parser, so the densest syntax in the tree -- and
    // two files that are deliberately unlexable
    ("crates/erg_parser/tests", false),
    ("crates/els/tests", true),
    ("crates/erg_compiler/tests", true),
    ("doc/scripts", true),
    ("bump_version.er", true),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.er` file at `rel`, which may name a directory or a single file.
fn er_files(rel: &str) -> Vec<PathBuf> {
    let path = repo_root().join(rel);
    let mut found = Vec::new();
    if path.is_file() {
        found.push(path);
    } else {
        collect(&path, &mut found);
    }
    found.sort();
    found
}

/// Every `.er` file in the repository.
fn all_er_files() -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(&repo_root(), &mut found);
    found.sort();
    found
}

/// Build output and dot-directories are skipped. Neither holds a source of
/// ours: `target/` is generated, and a git worktree under `.claude/` is a copy
/// of files already swept where they really live.
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name != "target" && !name.starts_with('.') {
                collect(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "er") {
            out.push(path);
        }
    }
}

/// The path as written in the repository, which is how anyone reading a
/// failure knows where to look.
fn relative(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Formats everything at `rel`, returning the files that did not lex.
fn sweep(rel: &str) -> Vec<String> {
    let files = er_files(rel);
    assert!(!files.is_empty(), "no .er files found under {rel}");

    let mut unlexable = Vec::new();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        match try_format_str(&src, FmtOptions::default()) {
            Ok(formatted) => {
                // §5.5: formatting is idempotent
                assert_eq!(
                    format_str(&formatted, FmtOptions::default()),
                    formatted,
                    "formatting {} twice differs from formatting it once",
                    relative(path)
                );
            }
            Err(Unformatted::InputDoesNotLex) => unlexable.push(relative(path)),
            Err(why) => panic!("{}: {why}", relative(path)),
        }
    }
    println!(
        "{rel}: {} files, {} of them unlexable",
        files.len(),
        unlexable.len()
    );
    unlexable
}

/// The safety net must never trip on real code. A source that does not lex is
/// fine where the corpus says so, but the formatter producing something
/// unlexable, or a different program, is a bug.
#[test]
fn every_er_file_in_the_repository_survives_formatting() {
    for (rel, all_lex) in SWEPT {
        let unlexable = sweep(rel);
        if *all_lex {
            assert!(
                unlexable.is_empty(),
                "{rel} is valid Erg, but these no longer lex: {unlexable:#?}\n\
                 the sweep skips what it cannot lex, so it is testing that much \
                 less than it says"
            );
        }
    }
}

/// A corpus nobody added to [`SWEPT`] is one the formatter is never run over.
/// That is how a sweep stops meaning anything -- not by failing, but by
/// covering a smaller share of the tree each time someone adds Erg to it.
#[test]
fn no_er_file_is_left_unswept() {
    let swept: HashSet<PathBuf> = SWEPT.iter().flat_map(|(rel, _)| er_files(rel)).collect();
    let missed: Vec<String> = all_er_files()
        .into_iter()
        .filter(|path| !swept.contains(path))
        .map(|path| relative(&path))
        .collect();
    assert!(
        missed.is_empty(),
        "swept by nothing in SWEPT: {missed:#?}\n\
         add the directory holding them to SWEPT in this file"
    );
}
