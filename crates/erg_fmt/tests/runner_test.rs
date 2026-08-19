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

    /// A file `read_to_string` will refuse.
    fn write_invalid_utf8(&self, rel: &str) {
        std::fs::write(self.root.join(rel), [0xff, 0xfe, 0x00]).unwrap();
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

/// A file the walk cannot read is reported and skipped. Ending the walk there
/// would leave everything after it unformatted, which is a worse outcome than
/// the one file that failed -- and no rerun of `erg fmt` fixes an I/O error.
#[test]
fn an_unreadable_file_does_not_strand_the_rest_of_the_tree() {
    let sandbox = Sandbox::new("unreadable");
    sandbox.write("a.er", UNFORMATTED);
    sandbox.write_invalid_utf8("b.er");
    sandbox.write("c.er", UNFORMATTED);
    assert_eq!(run(&sandbox.root, FmtConfig::default()), 0);
    assert_eq!(sandbox.read("a.er"), FORMATTED);
    assert_eq!(sandbox.read("c.er"), FORMATTED, "sorted after the bad one");
}

/// Naming the file directly is the other case: there the failure is the whole
/// answer, so it is an error rather than a line on stderr.
#[test]
fn an_unreadable_file_named_directly_is_an_error() {
    let sandbox = Sandbox::new("unreadable_named");
    sandbox.write_invalid_utf8("a.er");
    assert_ne!(run(&sandbox.root.join("a.er"), FmtConfig::default()), 0);
}

/// A symlink to an ancestor is a cycle: following it walks forever, formatting
/// the same files under ever longer paths.
#[cfg(unix)]
#[test]
fn a_symlink_cycle_does_not_hang_the_walk() {
    let sandbox = Sandbox::new("symlink_cycle");
    sandbox.write("d/a.er", UNFORMATTED);
    std::os::unix::fs::symlink("../", sandbox.root.join("d/up")).unwrap();
    assert_eq!(run(&sandbox.root, FmtConfig::default()), 0);
    assert_eq!(
        sandbox.read("d/a.er"),
        FORMATTED,
        "reached by its real path"
    );
}

/// A dot-directory is not the author's source tree: `.git` holds objects,
/// `.venv` holds someone else's code, and a git worktree under `.claude` holds
/// a second copy of files already reached where they really live. Rewriting any
/// of them is at best pointless and at worst destructive.
#[test]
fn a_dot_directory_is_not_walked() {
    let sandbox = Sandbox::new("hidden");
    sandbox.write("a.er", UNFORMATTED);
    sandbox.write(".venv/b.er", UNFORMATTED);
    sandbox.write("nested/.git/c.er", UNFORMATTED);
    assert_eq!(run(&sandbox.root, FmtConfig::default()), 0);
    assert_eq!(sandbox.read("a.er"), FORMATTED);
    assert_eq!(sandbox.read(".venv/b.er"), UNFORMATTED);
    assert_eq!(sandbox.read("nested/.git/c.er"), UNFORMATTED);
}

/// ...but naming one is how you ask for it, so the root is always walked.
#[test]
fn a_dot_directory_named_directly_is_walked() {
    let sandbox = Sandbox::new("hidden_named");
    sandbox.write(".config/a.er", UNFORMATTED);
    assert_eq!(run(&sandbox.root.join(".config"), FmtConfig::default()), 0);
    assert_eq!(sandbox.read(".config/a.er"), FORMATTED);
}
