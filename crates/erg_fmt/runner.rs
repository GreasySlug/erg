//! `erg fmt` -- the command-line entry point.
//!
//! Three ways to run it, in the order the flags are checked:
//!
//! | invocation | effect |
//! | --- | --- |
//! | `erg fmt <path>` | rewrite the file, or every `.er` under the directory |
//! | `erg fmt --check <path>` | name the files that would change; write none |
//! | `erg fmt --stdout <path>` | write the result to stdout |
//!
//! Reading from a pipe or `-c` writes to stdout regardless, since there is no
//! file to write back to.

use std::io::Write;
use std::path::{Path, PathBuf};

use erg_common::config::{ErgConfig, FmtConfig};
use erg_common::error::{ErrorCore, ErrorKind, Location, SubMessage};
use erg_common::io::{Input, InputKind};
use erg_common::traits::{ExitStatus, New, Runnable};
use erg_parser::error::{ParserRunnerError, ParserRunnerErrors};

use crate::{try_format_str, FmtOptions};

impl From<&FmtConfig> for FmtOptions {
    fn from(cfg: &FmtConfig) -> Self {
        Self {
            indent: cfg.indent,
            max_blank_lines: cfg.max_blank_lines,
            max_width: cfg.max_width,
        }
    }
}

/// The `erg fmt` runner.
#[derive(Debug, Default)]
pub struct Formatter {
    pub cfg: ErgConfig,
}

impl New for Formatter {
    fn new(cfg: ErgConfig) -> Self {
        Self { cfg }
    }
}

impl Runnable for Formatter {
    type Err = ParserRunnerError;
    type Errs = ParserRunnerErrors;
    const NAME: &'static str = "Erg formatter";

    #[inline]
    fn cfg(&self) -> &ErgConfig {
        &self.cfg
    }
    #[inline]
    fn cfg_mut(&mut self) -> &mut ErgConfig {
        &mut self.cfg
    }

    // nothing is carried between runs
    #[inline]
    fn initialize(&mut self) {}
    #[inline]
    fn clear(&mut self) {}
    #[inline]
    fn finish(&mut self) {}

    fn exec(&mut self) -> Result<ExitStatus, Self::Errs> {
        let opts = FmtOptions::from(&self.cfg.fmt);
        match self.cfg.input.kind() {
            InputKind::File { path, .. } if path.is_dir() => {
                let path = path.clone();
                Ok(self.format_tree(&path, opts))
            }
            InputKind::File { path, .. } => {
                let path = path.clone();
                let changed = self
                    .format_file(&path, opts)
                    .map_err(|e| self.io_error(&path, &e))?;
                Ok(self.status(usize::from(changed)))
            }
            // a pipe or `-c` has nowhere to be written back to
            _ => {
                let src = self.cfg.input.read();
                print!("{}", self.format_source(&src, &self.cfg.input, opts));
                Ok(ExitStatus::OK)
            }
        }
    }

    /// Formats one source, for the REPL. Never fails: an unformattable source
    /// comes back as it went in.
    fn eval(&mut self, src: String) -> Result<String, Self::Errs> {
        let opts = FmtOptions::from(&self.cfg.fmt);
        Ok(self.format_source(&src, &self.cfg.input, opts))
    }

    fn completeness_checker(&self) -> Option<erg_common::stdin::CompletenessChecker> {
        Some(Box::new(erg_parser::parse::check_code_completeness))
    }
}

impl Formatter {
    pub fn new(cfg: ErgConfig) -> Self {
        New::new(cfg)
    }

    /// `--check` reports rather than writes, so anything it found is a failure.
    ///
    /// Only a difference counts. A file that could not be formatted at all is
    /// reported on stderr and passes: `erg fmt` cannot fix it, so failing CI
    /// over it would only be telling the author something `erg check` says
    /// better (design §7).
    fn status(&self, changed: usize) -> ExitStatus {
        if self.cfg.fmt.check && changed > 0 {
            ExitStatus::ERR1
        } else {
            ExitStatus::OK
        }
    }

    /// Formats a source, reporting on stderr if it had to be left alone.
    ///
    /// Not an error: a half-written file is the normal state of a file being
    /// edited, and `erg fmt` refusing it is the safety net working. What the
    /// caller gets back is always something it can write out.
    fn format_source(&self, src: &str, input: &Input, opts: FmtOptions) -> String {
        match try_format_str(src, opts) {
            Ok(formatted) => formatted,
            Err(why) => {
                eprintln!("{}: left unformatted: {why}", input.filename());
                src.to_string()
            }
        }
    }

    /// Formats one file. Returns whether its contents would change.
    fn format_file(&mut self, path: &Path, opts: FmtOptions) -> std::io::Result<bool> {
        let src = std::fs::read_to_string(path)?;
        let formatted = self.format_source(&src, &Input::file(path.to_path_buf()), opts);
        let changed = formatted != src;
        if self.cfg.fmt.stdout {
            print!("{formatted}");
        } else if self.cfg.fmt.check {
            if changed {
                let mut stdout = std::io::stdout();
                let _ = writeln!(stdout, "{}", path.display());
            }
        } else if changed {
            std::fs::write(path, &formatted)?;
        }
        Ok(changed)
    }

    /// Formats every `.er` under a directory, in a stable order.
    ///
    /// A file that cannot be read or written is reported and skipped rather
    /// than ending the walk. Stopping would leave the tree half-formatted,
    /// which is worse than the one file that failed -- and an I/O error is not
    /// something running `erg fmt` again will fix. A file named on the command
    /// line is different: there the failure *is* the answer, so it is an error.
    fn format_tree(&mut self, root: &Path, opts: FmtOptions) -> ExitStatus {
        let mut changed = 0;
        for path in self.collect(root) {
            match self.format_file(&path, opts) {
                Ok(did) => changed += usize::from(did),
                Err(err) => eprintln!("{}: skipped: {err}", path.display()),
            }
        }
        self.status(changed)
    }

    /// Every `.er` file under `root`, sorted, minus the excluded ones.
    ///
    /// Sorted because the output of `--check` is read by people and compared by
    /// CI, and directory order is neither stable across platforms nor
    /// meaningful to either.
    ///
    /// Symlinks are not followed. One pointing at an ancestor is a cycle, and
    /// following it walks forever, reformatting the same files under longer and
    /// longer paths. Nothing is missed by declining: a symlinked file inside the
    /// tree is reached through its real path, and one pointing outside is not
    /// this tree's to rewrite.
    fn collect(&self, root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if self.is_excluded(&path) || entry.file_type().is_ok_and(|ty| ty.is_symlink()) {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "er") {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }

    /// Whether `--exclude` covers this path.
    ///
    /// Substring matching rather than globbing: `--exclude tests/should_err`
    /// and `--exclude .d.er` both read naturally, and neither needs a
    /// dependency or a syntax to explain.
    fn is_excluded(&self, path: &Path) -> bool {
        let path = path.to_string_lossy().replace('\\', "/");
        self.cfg
            .fmt
            .exclude
            .iter()
            .any(|pattern| path.contains(*pattern))
    }

    fn io_error(&self, path: &Path, err: &std::io::Error) -> ParserRunnerErrors {
        let core = ErrorCore::new(
            vec![SubMessage::only_loc(Location::Unknown)],
            format!("{}: {err}", path.display()),
            0,
            ErrorKind::IoError,
            Location::Unknown,
        );
        ParserRunnerErrors::new(vec![ParserRunnerError::new(
            core,
            Input::file(path.to_path_buf()),
        )])
    }
}
