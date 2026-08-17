//! The `erg fmt` command, driven through `Formatter` rather than the binary.
//!
//! What is pinned here is the part a user notices: which files get written,
//! which get named, and what the exit code says.

use std::path::{Path, PathBuf};

use erg_common::config::{ErgConfig, ErgMode, FmtConfig};
use erg_common::io::Input;
use erg_common::traits::Runnable;
use erg_fmt::Formatter;

/// A throwaway directory of `.er` files.
///
/// Named after the test so concurrent tests do not collide, and removed on
/// drop -- including when the test fails, so a failure leaves nothing behind.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("erg_fmt_test_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn config(path: &Path, fmt: FmtConfig) -> ErgConfig {
    ErgConfig {
        mode: ErgMode::Fmt,
        input: Input::file(path.to_path_buf()),
        fmt,
        ..ErgConfig::default()
    }
}

/// Runs the formatter over `path`, returning the process exit code.
fn run(path: &Path, fmt: FmtConfig) -> i32 {
    Formatter::run(config(path, fmt)).code
}

const UNFORMATTED: &str = "f = (x) ->\n  y   =  x+1\n  y\n";
const FORMATTED: &str = "f = (x) ->\n    y = x + 1\n    y\n";

#[test]
fn a_file_is_rewritten_in_place() {
    let sandbox = Sandbox::new("rewrite");
    let path = sandbox.write("a.er", UNFORMATTED);
    assert_eq!(run(&path, FmtConfig::default()), 0);
    assert_eq!(sandbox.read("a.er"), FORMATTED);
}

/// An already-formatted file must not be rewritten at all -- an unchanged file
/// keeps its mtime, which is what stops a formatter from invalidating every
/// build cache it touches.
#[test]
fn an_unchanged_file_is_not_written() {
    let sandbox = Sandbox::new("untouched");
    let path = sandbox.write("a.er", FORMATTED);
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(run(&path, FmtConfig::default()), 0);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
}

#[test]
fn check_reports_without_writing() {
    let sandbox = Sandbox::new("check");
    let path = sandbox.write("a.er", UNFORMATTED);
    let fmt = FmtConfig {
        check: true,
        ..FmtConfig::default()
    };
    assert_eq!(run(&path, fmt), 1, "a file that would change fails --check");
    assert_eq!(sandbox.read("a.er"), UNFORMATTED, "and is left alone");
}

#[test]
fn check_passes_on_formatted_input() {
    let sandbox = Sandbox::new("check_clean");
    let path = sandbox.write("a.er", FORMATTED);
    let fmt = FmtConfig {
        check: true,
        ..FmtConfig::default()
    };
    assert_eq!(run(&path, fmt), 0);
}

/// A file the formatter cannot handle is not a `--check` failure. It cannot be
/// fixed by running `erg fmt`, so failing CI over it would be a dead end.
#[test]
fn a_source_that_does_not_lex_passes_check_untouched() {
    let sandbox = Sandbox::new("broken");
    let broken = "_ = \"unterminated\n";
    let path = sandbox.write("a.er", broken);
    let fmt = FmtConfig {
        check: true,
        ..FmtConfig::default()
    };
    assert_eq!(run(&path, fmt), 0);
    assert_eq!(sandbox.read("a.er"), broken);
    assert_eq!(run(&path, FmtConfig::default()), 0, "nor in write mode");
    assert_eq!(sandbox.read("a.er"), broken);
}

#[test]
fn a_directory_is_walked_recursively() {
    let sandbox = Sandbox::new("tree");
    sandbox.write("a.er", UNFORMATTED);
    sandbox.write("nested/deep/b.er", UNFORMATTED);
    sandbox.write("nested/c.txt", "not erg\n");
    assert_eq!(run(&sandbox.root, FmtConfig::default()), 0);
    assert_eq!(sandbox.read("a.er"), FORMATTED);
    assert_eq!(sandbox.read("nested/deep/b.er"), FORMATTED);
    assert_eq!(sandbox.read("nested/c.txt"), "not erg\n", "left alone");
}

#[test]
fn exclude_skips_matching_paths() {
    let sandbox = Sandbox::new("exclude");
    sandbox.write("a.er", UNFORMATTED);
    sandbox.write("vendor/b.er", UNFORMATTED);
    let fmt = FmtConfig {
        exclude: ["vendor"].into(),
        ..FmtConfig::default()
    };
    assert_eq!(run(&sandbox.root, fmt), 0);
    assert_eq!(sandbox.read("a.er"), FORMATTED);
    assert_eq!(sandbox.read("vendor/b.er"), UNFORMATTED);
}

#[test]
fn the_options_reach_the_formatter() {
    let sandbox = Sandbox::new("options");
    let path = sandbox.write("a.er", UNFORMATTED);
    let fmt = FmtConfig {
        indent: 2,
        ..FmtConfig::default()
    };
    assert_eq!(run(&path, fmt), 0);
    assert_eq!(sandbox.read("a.er"), "f = (x) ->\n  y = x + 1\n  y\n");
}

/// Reflow is off by default at 100 columns, so `--max-width` has to be the
/// thing that turns it on.
#[test]
fn max_width_drives_wrapping() {
    let sandbox = Sandbox::new("width");
    let path = sandbox.write("a.er", "_ = f(alpha, beta)\n");
    assert_eq!(run(&path, FmtConfig::default()), 0);
    assert_eq!(sandbox.read("a.er"), "_ = f(alpha, beta)\n");

    let fmt = FmtConfig {
        max_width: 10,
        ..FmtConfig::default()
    };
    assert_eq!(run(&path, fmt), 0);
    assert_eq!(sandbox.read("a.er"), "_ = f(\n    alpha,\n    beta\n)\n");
}

/// `eval` is what the REPL and `-c` go through.
#[test]
fn eval_formats_a_string() {
    let mut formatter = Formatter::new(ErgConfig::default());
    assert_eq!(formatter.eval(UNFORMATTED.to_string()).unwrap(), FORMATTED);
}
