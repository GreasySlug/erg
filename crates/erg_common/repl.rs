//! REPL session management, decoupled from the `Runnable` trait.
//!
//! This module contains:
//! - [`ReplSession`]: a push-based (sans-IO) state machine that absorbs input
//!   lines and tells the driver when the accumulated code is ready to evaluate
//! - [`run`]: the standard execution entry point used by `Runnable::run`,
//!   which dispatches to `exec` for file-like input or drives the REPL loop
//!
//! Block detection works like IPython's `check_complete`: after each line the
//! whole accumulated buffer is passed to a [`CompletenessChecker`] (usually
//! `erg_parser::parse::check_code_completeness`), which reports whether the
//! code is complete, still expects a block / closing delimiter, or is invalid.
//! There is no line-based block guessing.
//!
//! `ReplSession` performs no I/O itself, so it can be driven by a terminal,
//! a test harness, or (in the future) a Jupyter-style kernel frontend.
use std::io::{stdout, BufWriter, Write};

use crate::config::ErgConfig;
use crate::error::{ErrorDisplay, ErrorKind, MultiErrorDisplay};
use crate::io::InputKind;
use crate::stdin::CodeCompleteness;
use crate::traits::{ExitStatus, Runnable, Stream};
use crate::{chomp, log, switch_unreachable};

const INDENT_UNIT: &str = "    ";

/// A single completion candidate handed to a REPL frontend.
#[derive(Debug, Clone)]
pub struct ReplCandidate {
    pub value: String,
    /// short description (e.g. the type) shown next to the candidate
    pub desc: Option<String>,
}

/// Result of a completion query.
#[derive(Debug, Clone)]
pub struct ReplCompletion {
    /// byte offset in the line where the span to be replaced starts
    pub start: usize,
    pub candidates: Vec<ReplCandidate>,
}

/// `(line, cursor byte position) -> completion result`
pub type CompletionProvider = Box<dyn Fn(&str, usize) -> ReplCompletion + Send + Sync>;

/// What the driver should do after feeding a line to a [`ReplSession`].
#[derive(Debug, PartialEq, Eq)]
pub enum SessionStep {
    /// The line was absorbed into the current cell; keep reading input.
    Continue,
    /// The accumulated source is ready; evaluate it.
    Eval(String),
    /// `:exit` / `:quit` was entered.
    Exit,
    /// `:clear` / `:cln` was entered; clear the screen and reset all state.
    ClearScreen,
}

/// A push-based REPL state machine.
///
/// Feed input lines one at a time with [`feed_line`](Self::feed_line) and act
/// on the returned [`SessionStep`]. The session itself performs no I/O and
/// does not evaluate code, so it can be embedded in any frontend.
///
/// Evaluation rules (IPython-like):
/// - a single complete line is evaluated immediately (unless in cell mode)
/// - while the checker reports `ExpectsBlock`, lines are accumulated with
///   auto-indentation; the indentation follows the previous line, so a line
///   the user indented further opens a block at that width
/// - while the checker reports `Unclosed` (inside brackets / `"""` strings),
///   lines are accumulated verbatim (no auto-indentation)
/// - an empty line closes the innermost open block; the cell is evaluated
///   when the outermost one closes (even if still incomplete, so that the
///   user sees the error)
/// - a syntax error is evaluated immediately to show the error
pub struct ReplSession {
    lines: Vec<String>,
    /// indentation widths (in columns) of the blocks the accumulated lines
    /// have opened, innermost last; the outermost level (0) is not stored
    block_indents: Vec<usize>,
    /// indentation (in columns) suggested for the next line
    next_indent: usize,
    /// checker result for the current buffer
    last: CodeCompleteness,
    /// In cell mode (a real terminal REPL without the `full-repl` feature),
    /// even single complete lines are accumulated until an empty line.
    cell_mode: bool,
}

impl ReplSession {
    pub fn new(cell_mode: bool) -> Self {
        Self {
            lines: vec![],
            block_indents: vec![],
            next_indent: 0,
            last: CodeCompleteness::Complete,
            cell_mode,
        }
    }

    /// The indentation expected for the next input line.
    pub fn indent(&self) -> String {
        if self.last == CodeCompleteness::Unclosed {
            // raw content lines (e.g. inside a `"""` string) must not be auto-indented
            String::new()
        } else {
            " ".repeat(self.next_indent)
        }
    }

    /// 1-origin indent depth (used e.g. for `Input::set_indent`).
    pub fn indent_depth(&self) -> usize {
        self.next_indent / INDENT_UNIT.len() + 1
    }

    /// Returns true if some source code is accumulated but not yet evaluated.
    pub fn has_pending_code(&self) -> bool {
        !self.lines.is_empty()
    }

    /// Resets the session to the initial state.
    pub fn reset(&mut self) {
        self.lines.clear();
        self.block_indents.clear();
        self.next_indent = 0;
        self.last = CodeCompleteness::Complete;
    }

    fn current_code(&self) -> String {
        let mut code = self.lines.join("\n");
        code.push('\n');
        code
    }

    fn take_code(&mut self) -> String {
        let code = self.current_code();
        self.reset();
        code
    }

    /// Record the block structure a line at `width` columns implies: it
    /// closes every block indented deeper than itself and opens one if it is
    /// indented deeper than the innermost open block.
    fn enter_line(&mut self, width: usize) {
        while self.block_indents.last().is_some_and(|&w| w > width) {
            self.block_indents.pop();
        }
        if width > self.block_indents.last().copied().unwrap_or(0) {
            self.block_indents.push(width);
        }
    }

    /// Feed a single input line (already chomped / right-trimmed).
    ///
    /// `indent` must be the value returned by [`indent`](Self::indent) for this
    /// iteration (the driver also needs it for the continuation prompt).
    /// `check` reports the completeness of the whole accumulated buffer
    /// (usually `Runnable::completeness_checker`). `on_indent_insert` is called
    /// when the session prepends indentation to the accumulated code, so that
    /// the driver can mirror it into the input history
    /// (`Input::insert_whitespace`).
    pub fn feed_line(
        &mut self,
        line: &str,
        indent: &str,
        check: impl Fn(&str) -> CodeCompleteness,
        mut on_indent_insert: impl FnMut(&str),
    ) -> SessionStep {
        match line {
            ":quit" | ":exit" => {
                return SessionStep::Exit;
            }
            ":clear" | ":cln" => {
                return SessionStep::ClearScreen;
            }
            _ => {}
        }
        if line.is_empty() {
            if self.lines.is_empty() {
                // nothing pending; ignore the blank line
                return SessionStep::Continue;
            }
            if self.last == CodeCompleteness::Unclosed {
                // a blank line is content inside an unclosed delimiter/string
                self.lines.push(String::new());
                return SessionStep::Continue;
            }
            // An empty line closes the innermost open block. The cell is
            // evaluated when the outermost one closes, so that nested blocks
            // can be exited one at a time.
            if self.block_indents.len() <= 1 {
                // evaluate the pending cell: complete code runs, incomplete
                // code is evaluated anyway so that the user sees the error
                return SessionStep::Eval(self.take_code());
            }
            self.block_indents.pop();
            self.next_indent = self.block_indents.last().copied().unwrap_or(0);
            self.lines.push(String::new());
            return SessionStep::Continue;
        }
        let mut full = String::with_capacity(indent.len() + line.len());
        if !indent.is_empty() {
            full.push_str(indent);
            on_indent_insert(indent);
        }
        full.push_str(line);
        let width = full.len() - full.trim_start_matches(' ').len();
        self.enter_line(width);
        self.lines.push(full);
        self.last = check(&self.current_code());
        match self.last {
            CodeCompleteness::Complete => {
                if self.lines.len() == 1 && !self.cell_mode {
                    SessionStep::Eval(self.take_code())
                } else {
                    // multi-line cell: keep the indent of the last line and
                    // wait for an empty line to evaluate
                    self.next_indent = width;
                    SessionStep::Continue
                }
            }
            CodeCompleteness::ExpectsBlock => {
                self.next_indent = width + INDENT_UNIT.len();
                SessionStep::Continue
            }
            // e.g. a decorator line: the next line stays at the same indent
            CodeCompleteness::Continuation => {
                self.next_indent = width;
                SessionStep::Continue
            }
            CodeCompleteness::Unclosed => SessionStep::Continue,
            // evaluate immediately to show the error
            CodeCompleteness::SyntaxError => SessionStep::Eval(self.take_code()),
        }
    }
}

/// Standard execution entry point used by `Runnable::run`.
/// Dispatches to `exec` for file-like input, or drives the REPL loop for
/// interactive input. The REPL terminates by returning an `ExitStatus`
/// instead of calling `process::exit`, so this can be called from tests or
/// embedding applications.
pub fn run<R: Runnable>(cfg: ErgConfig) -> ExitStatus {
    let quiet_repl = cfg.quiet_repl;
    let mut instance = R::new(cfg);
    match &instance.input().kind {
        InputKind::File { .. } | InputKind::Pipe(_) | InputKind::Str(_) => match instance.exec() {
            Ok(status) => status,
            Err(errs) => {
                let num_errors = errs.len();
                errs.write_all_stderr();
                ExitStatus::new(1, 0, num_errors)
            }
        },
        InputKind::REPL | InputKind::DummyREPL(_) => repl_loop(&mut instance, quiet_repl),
        InputKind::Dummy => switch_unreachable!(),
    }
}

/// Gracefully terminate the REPL: notify the instance, log, and return the status.
fn finish_repl<R: Runnable>(
    instance: &mut R,
    output: &mut BufWriter<std::io::StdoutLock>,
    num_errors: usize,
) -> ExitStatus {
    instance.finish();
    if !instance.cfg().quiet_repl {
        log!(info_f output, "The REPL has finished successfully.\n");
    }
    ExitStatus::new(0, 0, num_errors)
}

fn repl_loop<R: Runnable>(instance: &mut R, quiet_repl: bool) -> ExitStatus {
    let mut num_errors = 0;
    let output = stdout();
    let mut output = BufWriter::new(output.lock());
    if !quiet_repl {
        log!(info_f output, "The REPL has started.\n");
        output
            .write_all(instance.start_message().as_bytes())
            .unwrap();
    }
    output.flush().unwrap();

    // Register completeness checker for smart Enter behavior
    if let Some(checker) = instance.completeness_checker() {
        crate::stdin::GLOBAL_STDIN.set_completeness_checker(checker);
    }

    #[cfg(feature = "full-repl")]
    if matches!(&instance.input().kind, InputKind::REPL) {
        return reedline_repl(instance, &mut output);
    }

    // Line-based REPL (DummyREPL, or a real terminal without the `full-repl` feature)
    let checker = instance.completeness_checker();
    let cell_mode = matches!(&instance.input().kind, InputKind::REPL);
    let mut session = ReplSession::new(cell_mode);
    loop {
        let indent = session.indent();
        if session.has_pending_code() {
            output.write_all(instance.ps2().as_bytes()).unwrap();
            output.write_all(indent.as_bytes()).unwrap();
        } else {
            output.write_all(instance.ps1().as_bytes()).unwrap();
        }
        output.flush().unwrap();
        instance.cfg().input.set_indent(session.indent_depth());
        let line = chomp(&instance.cfg_mut().input.read());
        let line = line.trim_end();
        let step = session.feed_line(
            line,
            &indent,
            // without a checker, fall back to a plain line-by-line REPL
            |src| {
                checker
                    .as_ref()
                    .map_or(CodeCompleteness::Complete, |c| c(src))
            },
            |ws| instance.input().insert_whitespace(ws),
        );
        match step {
            SessionStep::Continue => {
                if !session.has_pending_code() {
                    // a blank line was skipped without starting a cell: keep
                    // the lexer's line offset (`block_begin`) in sync with the
                    // input history, otherwise error locations shift by a line
                    instance.input().set_block_begin();
                }
            }
            SessionStep::Exit => {
                return finish_repl(instance, &mut output, num_errors);
            }
            SessionStep::ClearScreen => {
                output.write_all("\x1b[2J\x1b[1;1H".as_bytes()).unwrap();
                output.flush().unwrap();
                instance.input().set_block_begin();
                session.reset();
                instance.clear();
            }
            SessionStep::Eval(code) => {
                match instance.eval(code) {
                    Ok(out) if out.is_empty() => {}
                    Ok(out) => {
                        output.write_all((out + "\n").as_bytes()).unwrap();
                        output.flush().unwrap();
                    }
                    Err(errs) => {
                        if errs
                            .first()
                            .map(|e| e.core().kind == ErrorKind::SystemExit)
                            .unwrap_or(false)
                        {
                            return finish_repl(instance, &mut output, num_errors);
                        }
                        num_errors += errs.len();
                        errs.write_all_stderr();
                    }
                }
                instance.input().set_block_begin();
                instance.clear();
                session.reset();
            }
        }
    }
}

/// reedline-based interactive REPL (the `full-repl` feature):
/// syntax-aware multiline editing (Enter submits only when the buffer is
/// complete), Tab completion, and persistent history.
#[cfg(feature = "full-repl")]
fn reedline_repl<R: Runnable>(
    instance: &mut R,
    output: &mut BufWriter<std::io::StdoutLock>,
) -> ExitStatus {
    use reedline::Signal;

    let mut num_errors = 0;
    let prompt = frontend::ErgPrompt {
        ps1: instance.ps1(),
        ps2: instance.ps2(),
    };
    let mut editor = frontend::build_editor(
        instance.completeness_checker(),
        instance.completion_provider(),
    );
    loop {
        output.flush().unwrap();
        match editor.read_line(&prompt) {
            Ok(Signal::Success(code)) => {
                let trimmed = code.trim();
                match trimmed {
                    ":quit" | ":exit" => {
                        return finish_repl(instance, output, num_errors);
                    }
                    ":clear" | ":cln" => {
                        let _ = editor.clear_scrollback();
                        instance.clear();
                        continue;
                    }
                    _ => {}
                }
                if trimmed.is_empty() {
                    instance.input().set_block_begin();
                    continue;
                }
                // record the cell in the input history (in this order: the
                // lexer numbers lines from `block_begin`) so that error
                // display can re-read the source lines
                instance.input().set_block_begin();
                for line in code.lines() {
                    crate::stdin::GLOBAL_STDIN.push_line(line.to_string());
                }
                let mut code = code;
                if !code.ends_with('\n') {
                    code.push('\n');
                }
                match instance.eval(code) {
                    Ok(out) if out.is_empty() => {}
                    Ok(out) => {
                        output.write_all((out + "\n").as_bytes()).unwrap();
                        output.flush().unwrap();
                    }
                    Err(errs) => {
                        if errs
                            .first()
                            .map(|e| e.core().kind == ErrorKind::SystemExit)
                            .unwrap_or(false)
                        {
                            return finish_repl(instance, output, num_errors);
                        }
                        num_errors += errs.len();
                        errs.write_all_stderr();
                    }
                }
                instance.clear();
            }
            // the buffer is already cleared by reedline
            Ok(Signal::CtrlC) => continue,
            Ok(Signal::CtrlD) => {
                return finish_repl(instance, output, num_errors);
            }
            Ok(_) => continue,
            Err(err) => {
                eprintln!("Readline error: {err}");
                return ExitStatus::new(1, 0, num_errors);
            }
        }
    }
}

/// reedline integration: prompt, validator (multiline), and completer.
#[cfg(feature = "full-repl")]
mod frontend {
    use std::borrow::Cow;

    use reedline::{
        default_emacs_keybindings, ColumnarMenu, Completer, Emacs, FileBackedHistory, KeyCode,
        KeyModifiers, MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch,
        PromptHistorySearchStatus, Reedline, ReedlineEvent, ReedlineMenu, Span, Suggestion,
        ValidationResult, Validator,
    };

    use crate::repl::{CompletionProvider, ReplCompletion};
    use crate::stdin::{CodeCompleteness, CompletenessChecker};

    pub(super) struct ErgPrompt {
        pub ps1: String,
        pub ps2: String,
    }

    impl Prompt for ErgPrompt {
        fn render_prompt_left(&self) -> Cow<'_, str> {
            Cow::Borrowed("")
        }
        fn render_prompt_right(&self) -> Cow<'_, str> {
            Cow::Borrowed("")
        }
        fn render_prompt_indicator(&self, _edit_mode: PromptEditMode) -> Cow<'_, str> {
            Cow::Borrowed(&self.ps1)
        }
        fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
            Cow::Borrowed(&self.ps2)
        }
        fn render_prompt_history_search_indicator(
            &self,
            history_search: PromptHistorySearch,
        ) -> Cow<'_, str> {
            let status = match history_search.status {
                PromptHistorySearchStatus::Passing => "",
                PromptHistorySearchStatus::Failing => "failing ",
            };
            Cow::Owned(format!(
                "({}reverse-search: {}) ",
                status, history_search.term
            ))
        }
    }

    /// Submits on Enter only when the buffer is complete (or has a syntax
    /// error, which is evaluated to show the message); otherwise Enter
    /// inserts a newline.
    pub(super) struct ErgValidator {
        checker: Option<CompletenessChecker>,
    }

    impl Validator for ErgValidator {
        fn validate(&self, line: &str) -> ValidationResult {
            let res = self
                .checker
                .as_ref()
                .map_or(CodeCompleteness::Complete, |c| c(line));
            if res.is_incomplete() {
                return ValidationResult::Incomplete;
            }
            // IPython-style: a multi-line cell is submitted with an empty
            // final line, so that more lines can be added to an
            // already-complete buffer (e.g. a second line of a function body)
            if line.contains('\n') && !line.ends_with('\n') {
                return ValidationResult::Incomplete;
            }
            ValidationResult::Complete
        }
    }

    pub(super) struct ErgCompleter {
        provider: CompletionProvider,
    }

    impl Completer for ErgCompleter {
        fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
            let ReplCompletion { start, candidates } = (self.provider)(line, pos);
            candidates
                .into_iter()
                .map(|c| Suggestion {
                    value: c.value,
                    description: c.desc,
                    span: Span::new(start, pos),
                    append_whitespace: false,
                    ..Default::default()
                })
                .collect()
        }
    }

    pub(super) fn build_editor(
        checker: Option<CompletenessChecker>,
        provider: Option<CompletionProvider>,
    ) -> Reedline {
        let mut editor = Reedline::create().with_validator(Box::new(ErgValidator { checker }));
        if let Some(provider) = provider {
            let menu = ColumnarMenu::default().with_name("completion_menu");
            let mut keybindings = default_emacs_keybindings();
            keybindings.add_binding(
                KeyModifiers::NONE,
                KeyCode::Tab,
                ReedlineEvent::UntilFound(vec![
                    ReedlineEvent::Menu("completion_menu".to_string()),
                    ReedlineEvent::MenuNext,
                ]),
            );
            editor = editor
                .with_completer(Box::new(ErgCompleter { provider }))
                .with_menu(ReedlineMenu::EngineCompleter(Box::new(menu)))
                .with_edit_mode(Box::new(Emacs::new(keybindings)));
        }
        let history_path = crate::env::erg_path().join("repl_history.txt");
        if let Ok(history) = FileBackedHistory::with_file(1000, history_path) {
            editor = editor.with_history(Box::new(history));
        }
        editor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny stand-in for the real parser-based checker, just structured
    /// enough to exercise the session state machine:
    /// - an odd number of `"""` => Unclosed
    /// - last line ending with `=` or `=>` => ExpectsBlock
    /// - containing `!!` => SyntaxError
    fn stub_check(src: &str) -> CodeCompleteness {
        if src.matches("\"\"\"").count() % 2 == 1 {
            return CodeCompleteness::Unclosed;
        }
        if src.contains("!!") {
            return CodeCompleteness::SyntaxError;
        }
        let last = src.lines().last().unwrap_or("").trim_end();
        if last.trim_start().starts_with('@') {
            CodeCompleteness::Continuation
        } else if last.ends_with('=') || last.ends_with("=>") {
            CodeCompleteness::ExpectsBlock
        } else {
            CodeCompleteness::Complete
        }
    }

    /// Feed lines into a session, collecting evaluated code chunks.
    /// Returns the chunks and whether the session was exited via `:exit`.
    fn drive(cell_mode: bool, lines: &[&str]) -> (Vec<String>, bool) {
        let mut session = ReplSession::new(cell_mode);
        let mut evals = vec![];
        for line in lines {
            let indent = session.indent();
            match session.feed_line(line, &indent, stub_check, |_| {}) {
                SessionStep::Continue => {}
                SessionStep::Eval(code) => evals.push(code),
                SessionStep::Exit => return (evals, true),
                SessionStep::ClearScreen => session.reset(),
            }
        }
        (evals, false)
    }

    #[test]
    fn session_single_line() {
        let (evals, exited) = drive(false, &["print! 1"]);
        assert_eq!(evals, vec!["print! 1\n"]);
        assert!(!exited);
    }

    #[test]
    fn session_block_eval_on_empty_line() {
        let (evals, _) = drive(false, &["f i =", "i + 1", "", "x = f 2"]);
        assert_eq!(evals, vec!["f i =\n    i + 1\n", "x = f 2\n"]);
    }

    #[test]
    fn session_nested_blocks() {
        // each blank line dedents one level; the cell is evaluated when a
        // blank line closes the outermost block
        let (evals, _) = drive(false, &["a =", "b =", "1", "", ""]);
        assert_eq!(evals, vec!["a =\n    b =\n        1\n\n"]);
    }

    #[test]
    fn session_dedent_returns_to_enclosing_block() {
        // the user indented the body further than suggested: the next line
        // follows it, and the empty line closes that block (there is no
        // block at 4 columns to fall back to) and evaluates
        let (evals, _) = drive(false, &["a =", "    1", "2", ""]);
        assert_eq!(evals, vec!["a =\n        1\n        2\n"]);
    }

    #[test]
    fn session_decorator_no_auto_indent() {
        // a decorator line expects a definition at the SAME indent level
        let (evals, _) = drive(false, &["@deco", "a = 1", ""]);
        assert_eq!(evals, vec!["@deco\na = 1\n"]);
    }

    #[test]
    fn session_multiline_string_no_auto_indent() {
        let (evals, _) = drive(false, &["s = \"\"\"", "  hello", "\"\"\"", ""]);
        // content lines are kept verbatim (no auto-indentation)
        assert_eq!(evals, vec!["s = \"\"\"\n  hello\n\"\"\"\n"]);
    }

    #[test]
    fn session_blank_line_in_string_is_content() {
        let (evals, _) = drive(false, &["s = \"\"\"", "", "\"\"\"", ""]);
        assert_eq!(evals, vec!["s = \"\"\"\n\n\"\"\"\n"]);
    }

    #[test]
    fn session_syntax_error_evals_immediately() {
        let (evals, _) = drive(false, &["boom !!"]);
        assert_eq!(evals, vec!["boom !!\n"]);
    }

    #[test]
    fn session_exit_command_and_blank_noop() {
        let (evals, exited) = drive(false, &["", "i = 1", ":exit"]);
        // a blank line with no pending code is a no-op
        assert_eq!(evals, vec!["i = 1\n"]);
        assert!(exited);
    }

    #[test]
    fn session_cell_mode_accumulates() {
        let (evals, _) = drive(true, &["x = 1", "x + 1", ""]);
        assert_eq!(evals, vec!["x = 1\nx + 1\n"]);
    }

    #[test]
    fn session_incomplete_block_evals_on_blank_line() {
        // user gives up after the block header: evaluate to show the error
        let (evals, _) = drive(false, &["f i =", ""]);
        assert_eq!(evals, vec!["f i =\n"]);
    }
}
