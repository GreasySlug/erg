//! End-to-end tests for the REPL driver (`erg_common::repl::run`) using
//! `ParserRunner`, which needs no Python interpreter.
use erg_common::config::ErgConfig;
use erg_common::io::{DummyStdin, Input};
use erg_common::spawn::exec_new_thread;
use erg_common::traits::{ExitStatus, Runnable};

use erg_parser::ParserRunner;

fn run_repl(name: &'static str, lines: &[&str]) -> ExitStatus {
    let lines = lines.iter().map(|s| s.to_string()).collect();
    let cfg = ErgConfig {
        input: Input::dummy_repl(DummyStdin::new(name.to_string(), lines)),
        quiet_repl: true,
        ..Default::default()
    };
    ParserRunner::run(cfg)
}

/// `:exit` must return an `ExitStatus` instead of calling `process::exit`,
/// otherwise this test would kill the whole test runner.
#[test]
fn repl_exit_returns_status() -> Result<(), ()> {
    exec_new_thread(
        || {
            let stat = run_repl("repl_exit", &["i = 1", ":exit"]);
            assert_eq!(stat.code, 0);
            assert_eq!(stat.num_errors, 0);
            Ok(())
        },
        "repl_exit",
    )
}

/// Multi-line blocks are accumulated and evaluated after the dedent line.
#[test]
fn repl_block_eval() -> Result<(), ()> {
    exec_new_thread(
        || {
            let stat = run_repl("repl_block", &["f i =", "i + 1", "", ":exit"]);
            assert_eq!(stat.code, 0);
            assert_eq!(stat.num_errors, 0);
            Ok(())
        },
        "repl_block",
    )
}

/// Multi-line string contents are accumulated verbatim until the closing `"""`.
#[test]
fn repl_multiline_string() -> Result<(), ()> {
    exec_new_thread(
        || {
            let stat = run_repl(
                "repl_mlstr",
                &["s = \"\"\"", "hello", "\"\"\"", "", ":exit"],
            );
            assert_eq!(stat.code, 0);
            assert_eq!(stat.num_errors, 0);
            Ok(())
        },
        "repl_mlstr",
    )
}

/// Parse errors are reported but do not abort the REPL session.
#[test]
fn repl_error_recovery() -> Result<(), ()> {
    exec_new_thread(
        || {
            let stat = run_repl("repl_error", &["1 +", "i = 1", ":exit"]);
            assert_eq!(stat.code, 0);
            assert!(stat.num_errors > 0);
            Ok(())
        },
        "repl_error",
    )
}
