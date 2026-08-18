//! Runs the formatter over every example in the repository.
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
use std::path::{Path, PathBuf};

fn er_files(dir: &str) -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir);
    let mut found = Vec::new();
    collect(&root, &mut found);
    found.sort();
    found
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "er") {
            out.push(path);
        }
    }
}

/// The safety net must never trip on real code. A source that does not lex is
/// fine -- some test fixtures are deliberately malformed -- but the formatter
/// producing something unlexable, or a different program, is a bug.
fn assert_never_breaks(dir: &str) {
    let files = er_files(dir);
    assert!(!files.is_empty(), "no .er files found under {dir}");

    let mut unlexable = 0;
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
                    path.display()
                );
            }
            Err(Unformatted::InputDoesNotLex) => unlexable += 1,
            Err(why) => panic!("{}: {why}", path.display()),
        }
    }
    println!(
        "{dir}: {} files, {unlexable} of them unlexable",
        files.len()
    );
}

#[test]
fn every_example_survives_formatting() {
    assert_never_breaks("examples");
}

#[test]
fn every_passing_test_case_survives_formatting() {
    assert_never_breaks("tests/should_ok");
}

/// The shipped standard library, which is the largest body of Erg in the tree
/// and the least like the examples: declaration files, dense signatures, and
/// the only place `\` continuations appear in quantity.
#[test]
fn every_standard_library_file_survives_formatting() {
    assert_never_breaks("crates/erg_compiler/lib");
}

/// Sources that are *meant* to be broken. The formatter still must not blow up
/// on them -- editors format files that are mid-edit and full of errors.
#[test]
fn deliberately_broken_sources_do_not_break_the_formatter() {
    assert_never_breaks("tests/should_err");
}
