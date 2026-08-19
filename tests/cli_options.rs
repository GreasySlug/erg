//! Every option `erg --help` advertises has to be one `erg` accepts.
//!
//! The two lists have drifted before. `--dump-as-pyc` and `--compile` were
//! removed from the parser in "chore: remove obsolete options" (f489df26) but
//! left in the help text and in the suggestion list, so `erg --dump-as-pyc`
//! answered `invalid option: --dump-as-pyc (did you mean `--dump-as-pyc`?)` --
//! the suggester offering back the word the parser had just refused.

use std::process::{Command, Stdio};

use erg_common::help_messages::OPTIONS;

/// `erg` falls back to a REPL when no file is given, so stdin is closed: the
/// REPL reads EOF and exits rather than hanging the test.
fn rejects(option: &str) -> bool {
    let out = Command::new(env!("CARGO_BIN_EXE_erg"))
        .arg(option)
        .stdin(Stdio::null())
        .output()
        .expect("failed to run erg");
    let stderr = String::from_utf8_lossy(&out.stderr);
    stderr.contains("invalid option")
}

#[test]
fn every_advertised_option_is_accepted() {
    let refused: Vec<&str> = OPTIONS.iter().copied().filter(|o| rejects(o)).collect();
    assert!(
        refused.is_empty(),
        "advertised in `erg --help` but refused by the parser: {refused:?}"
    );
}
