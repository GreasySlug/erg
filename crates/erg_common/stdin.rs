use std::fmt;
#[cfg(not(feature = "full-repl"))]
use std::io::{stdin, BufRead, BufReader};
use std::sync::OnceLock;

#[cfg(feature = "full-repl")]
use crossterm::{
    cursor::MoveToColumn,
    event::{read, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::Print,
    terminal::{self, disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
#[cfg(feature = "full-repl")]
use std::process::{Command, Output};

use crate::shared::Shared;

use crate::completion_provider::CompletionProvider;
use crate::repl::highlighter::ReplHighlighter;

/// e.g.
/// ```erg
/// >>> print! 1
/// >>>
/// >>> while! False, do!:
/// >>>    print! ""
/// >>>
/// ```
/// ↓
///
/// `{ lineno: 5, buf: ["print! 1\n", "\n", "while! False, do!:\n", "print! \"\"\n", "\n"] }`
pub struct StdinReader {
    block_begin: usize,
    lineno: usize,
    buf: Vec<String>,
    #[cfg(feature = "full-repl")]
    history_input_position: usize,
    indent: u16,
    highlighter: ReplHighlighter,
    #[cfg(feature = "full-repl")]
    dropdown: Option<crate::dropdown_ui::DropdownUI>,
    completion_provider: Option<Box<dyn CompletionProvider>>,
}

impl fmt::Debug for StdinReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug_struct = f.debug_struct("StdinReader");
        debug_struct
            .field("block_begin", &self.block_begin)
            .field("lineno", &self.lineno)
            .field("buf", &self.buf);
        #[cfg(feature = "full-repl")]
        debug_struct.field("history_input_position", &self.history_input_position);
        debug_struct
            .field("indent", &self.indent)
            .field("highlighter", &self.highlighter);
        #[cfg(feature = "full-repl")]
        debug_struct.field("dropdown", &self.dropdown);
        debug_struct
            .field("completion_provider", &"<CompletionProvider>")
            .finish()
    }
}

impl StdinReader {
    /// Set a custom completion provider
    pub fn set_completion_provider(&mut self, provider: Box<dyn CompletionProvider>) {
        self.completion_provider = Some(provider);
    }

    #[cfg(all(feature = "full-repl", target_os = "linux"))]
    fn access_clipboard() -> Option<Output> {
        if let Ok(str) = fs::read("/proc/sys/kernel/osrelease") {
            if let Ok(str) = str::from_utf8(&str) {
                if str.to_ascii_lowercase().contains("microsoft") {
                    return Some(
                        Command::new("powershell")
                            .args(["get-clipboard"])
                            .output()
                            .expect("failed to get clipboard"),
                    );
                }
            }
        }
        match Command::new("xsel")
            .args(["--output", "--clipboard"])
            .output()
        {
            Ok(output) => Some(output),
            Err(_) => {
                execute!(
                    std::io::stdout(),
                    Print("You need to install `xsel` to use the paste feature on Linux desktop"),
                )
                .unwrap();
                None
            }
        }
    }
    #[cfg(all(feature = "full-repl", target_os = "macos"))]
    fn access_clipboard() -> Option<Output> {
        Some(
            Command::new("pbpast")
                .output()
                .expect("failed to get clipboard"),
        )
    }

    #[cfg(all(feature = "full-repl", target_os = "windows"))]
    fn access_clipboard() -> Option<Output> {
        Some(
            Command::new("powershell")
                .args(["get-clipboard"])
                .output()
                .expect("failed to get clipboard"),
        )
    }

    #[cfg(not(feature = "full-repl"))]
    pub fn read(&mut self) -> String {
        let mut line = "".to_string();
        let stdin = stdin();
        let mut reader = BufReader::new(stdin.lock());
        reader.read_line(&mut line).unwrap();
        self.lineno += 1;
        self.buf.push(line.trim_end().to_string());
        self.buf.last().cloned().unwrap_or_default()
    }

    /// Get highlighted version of the last read line
    pub fn read_highlighted(&self) -> String {
        if let Some(last_line) = self.buf.last() {
            self.highlighter.highlight_line(last_line)
        } else {
            String::new()
        }
    }

    /// Highlight any given line using the same highlighter
    pub fn highlight_line(&self, line: &str) -> String {
        self.highlighter.highlight_line(line)
    }

    #[cfg(feature = "full-repl")]
    pub fn read(&mut self) -> String {
        enable_raw_mode().unwrap();
        let mut output = std::io::stdout();
        let mut line = String::new();
        self.input(&mut line).unwrap();
        disable_raw_mode().unwrap();
        execute!(output, MoveToColumn(0)).unwrap();
        self.lineno += 1;
        self.buf.push(line);
        self.buf.last().cloned().unwrap_or_default()
    }

    #[cfg(feature = "full-repl")]
    fn input(&mut self, line: &mut String) -> std::io::Result<()> {
        let mut position = 0;
        let mut consult_history = false;
        let mut stdout = std::io::stdout();
        while let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = read()?
        {
            // Handle dropdown navigation if active
            if let Some(dropdown) = self.dropdown.take() {
                if dropdown.is_active() {
                    match code {
                        KeyCode::Down => {
                            dropdown.clear(&mut stdout)?;
                            let new_dropdown = dropdown.with_selection_down();
                            new_dropdown.display(&mut stdout)?;
                            self.dropdown = Some(new_dropdown);
                            continue;
                        }
                        KeyCode::Up => {
                            dropdown.clear(&mut stdout)?;
                            let new_dropdown = dropdown.with_selection_up();
                            new_dropdown.display(&mut stdout)?;
                            self.dropdown = Some(new_dropdown);
                            continue;
                        }
                        KeyCode::Tab => {
                            dropdown.clear(&mut stdout)?;
                            let new_dropdown = dropdown.with_selection_down();
                            new_dropdown.display(&mut stdout)?;
                            self.dropdown = Some(new_dropdown);
                            continue;
                        }
                        KeyCode::Enter => {
                            if let Some(selected) = dropdown.selected_candidate() {
                                dropdown.clear(&mut stdout)?;
                                let (word, start) = completion::get_word_at_cursor(&line, position);
                                line.replace_range(start..start + word.len(), selected);
                                position = start + selected.len();
                                self.dropdown = None;

                                // Redraw the line with the completion
                                let highlighted_line = self.highlighter.highlight_line(&line);
                                execute!(
                                    stdout,
                                    MoveToColumn(4),
                                    Clear(ClearType::UntilNewLine),
                                    MoveToColumn(self.indent * 4),
                                    Print(highlighted_line),
                                    MoveToColumn(self.indent * 4 + position as u16)
                                )?;
                                continue;
                            }
                        }
                        KeyCode::Esc => {
                            dropdown.clear(&mut stdout)?;
                            self.dropdown = None;
                            continue;
                        }
                        _ => {
                            // Any other key closes the dropdown
                            dropdown.clear(&mut stdout)?;
                            self.dropdown = None;
                            // Continue to process the key normally
                        }
                    }
                } else {
                    // Dropdown is not active, put it back
                    self.dropdown = Some(dropdown);
                }
            }

            consult_history = false;
            match (code, modifiers) {
                (KeyCode::Char('z'), KeyModifiers::CONTROL)
                | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                    println!();
                    line.clear();
                    line.push_str(":exit");
                    return Ok(());
                }
                (KeyCode::Char('v'), KeyModifiers::CONTROL) => {
                    let output = match Self::access_clipboard() {
                        None => {
                            continue;
                        }
                        Some(output) => output,
                    };
                    let clipboard = {
                        let this = String::from_utf8_lossy(&output.stdout).to_string();
                        this.trim_matches(|c: char| c.is_whitespace())
                            .to_string()
                            .replace(['\n', '\r'], "")
                            .replace(|c: char| c.len_utf8() >= 2, "")
                    };
                    line.insert_str(position, &clipboard);
                    position += clipboard.len();
                }
                (_, KeyModifiers::CONTROL) => continue,
                (KeyCode::Tab, _) => {
                    // Use completion provider if available, otherwise use basic completion
                    let candidates = if let Some(provider) = &self.completion_provider {
                        provider.get_completions(&line, position)
                    } else {
                        // Use basic completion
                        let context =
                            crate::completion::CompletionContext::new(line.clone(), position);
                        match context.get_completion_action() {
                            crate::completion::CompletionAction::CompleteUnique(completion) => {
                                vec![completion]
                            }
                            crate::completion::CompletionAction::CompleteCommon(common) => {
                                vec![common]
                            }
                            crate::completion::CompletionAction::ShowCandidates(candidates) => {
                                candidates
                            }
                            crate::completion::CompletionAction::None => vec![],
                        }
                    };

                    // Process completion results
                    if candidates.is_empty() {
                        // No completion available, insert 4 spaces as before
                        line.insert_str(position, "    ");
                        position += 4;
                    } else if candidates.len() == 1 {
                        // Single candidate - complete immediately
                        let (word, start) = crate::completion::get_word_at_cursor(&line, position);
                        let completion = &candidates[0];
                        line.replace_range(start..start + word.len(), completion);
                        position = start + completion.len();
                    } else {
                        // Multiple candidates - show dropdown
                        let terminal_width = terminal::size().unwrap_or((80, 24)).0;
                        let dropdown = crate::dropdown_ui::DropdownUI::new(
                            candidates,
                            self.indent * 4 + position as u16,
                            terminal_width,
                        )
                        .with_active(true);

                        // First, make sure the current line is rendered properly
                        let highlighted_line = self.highlighter.highlight_line(&line);
                        execute!(
                            stdout,
                            MoveToColumn(4),
                            Clear(ClearType::UntilNewLine),
                            MoveToColumn(self.indent * 4),
                            Print(&highlighted_line),
                            MoveToColumn(self.indent * 4 + position as u16)
                        )?;

                        // Now display the dropdown below the current line
                        dropdown.display(&mut stdout)?;
                        self.dropdown = Some(dropdown);

                        // Skip the normal line redraw since we already did it
                        continue;
                    }
                }
                (KeyCode::Home, _) => {
                    position = 0;
                }
                (KeyCode::End, _) => {
                    position = line.len();
                }
                (KeyCode::Backspace, _) => {
                    if position == 0 {
                        continue;
                    }
                    line.remove(position - 1);
                    position -= 1;
                }
                (KeyCode::Delete, _) => {
                    if position == line.len() {
                        continue;
                    }
                    line.remove(position);
                }
                (KeyCode::Up, _) => {
                    consult_history = true;
                    if self.history_input_position == 0 {
                        continue;
                    }
                    self.history_input_position -= 1;
                    execute!(stdout, MoveToColumn(4), Clear(ClearType::UntilNewLine))?;
                    if let Some(l) = self.buf.get(self.history_input_position) {
                        position = l.len();
                        line.clear();
                        line.push_str(l);
                    }
                }
                (KeyCode::Down, _) => {
                    if self.history_input_position == self.buf.len() {
                        continue;
                    }
                    if self.history_input_position == self.buf.len() - 1 {
                        *line = "".to_string();
                        position = 0;
                        self.history_input_position += 1;
                        execute!(
                            stdout,
                            MoveToColumn(4),
                            Clear(ClearType::UntilNewLine),
                            MoveToColumn(self.indent * 4),
                            Print(line.to_owned()),
                            MoveToColumn(self.indent * 4 + position as u16)
                        )?;
                        continue;
                    }
                    self.history_input_position += 1;
                    execute!(stdout, MoveToColumn(4), Clear(ClearType::UntilNewLine))?;
                    if let Some(l) = self.buf.get(self.history_input_position) {
                        position = l.len();
                        line.clear();
                        line.push_str(l);
                    }
                }
                (KeyCode::Left, _) => {
                    if position == 0 {
                        continue;
                    }
                    position -= 1;
                }
                (KeyCode::Right, _) => {
                    if position == line.len() {
                        continue;
                    }
                    position += 1;
                }
                (KeyCode::Enter, _) => {
                    println!();
                    break;
                }
                // TODO: check a full-width char and possible to insert
                (KeyCode::Char(c), _) if c.len_utf8() < 2 => {
                    line.insert(position, c);
                    position += 1;
                }
                _ => {}
            }
            let highlighted_line = self.highlighter.highlight_line(&line);
            execute!(
                stdout,
                MoveToColumn(4),
                Clear(ClearType::UntilNewLine),
                MoveToColumn(self.indent * 4),
                Print(highlighted_line),
                MoveToColumn(self.indent * 4 + position as u16)
            )?;
        }
        if !consult_history {
            self.history_input_position = self.buf.len() + 1;
        }
        Ok(())
    }

    pub fn reread(&self) -> String {
        self.buf.last().cloned().unwrap_or_default()
    }

    pub fn reread_lines(&self, ln_begin: usize, ln_end: usize) -> Vec<String> {
        if let Some(lines) = self.buf.get(ln_begin - 1..=ln_end - 1) {
            lines.to_vec()
        } else {
            self.buf.clone()
        }
    }

    pub fn last_line(&mut self) -> Option<&mut String> {
        self.buf.last_mut()
    }
}

#[derive(Debug)]
pub struct GlobalStdin(OnceLock<Shared<StdinReader>>);

pub static GLOBAL_STDIN: GlobalStdin = GlobalStdin(OnceLock::new());

impl GlobalStdin {
    fn get(&'static self) -> &'static Shared<StdinReader> {
        self.0.get_or_init(|| {
            Shared::new(StdinReader {
                block_begin: 1,
                lineno: 1,
                buf: vec![],
                #[cfg(feature = "full-repl")]
                history_input_position: 1,
                indent: 1,
                highlighter: ReplHighlighter::new(),
                #[cfg(feature = "full-repl")]
                dropdown: None,
                completion_provider: None,
            })
        })
    }

    pub fn read(&'static self) -> String {
        self.get().borrow_mut().read()
    }

    /// Set a custom completion provider for the global stdin reader
    pub fn set_completion_provider(&'static self, provider: Box<dyn CompletionProvider>) {
        self.get().borrow_mut().set_completion_provider(provider);
    }

    pub fn reread(&'static self) -> String {
        self.get().borrow_mut().reread()
    }

    pub fn reread_lines(&'static self, ln_begin: usize, ln_end: usize) -> Vec<String> {
        self.get().borrow_mut().reread_lines(ln_begin, ln_end)
    }

    pub fn lineno(&'static self) -> usize {
        self.get().borrow_mut().lineno
    }

    pub fn block_begin(&'static self) -> usize {
        self.get().borrow_mut().block_begin
    }

    pub fn set_block_begin(&'static self, n: usize) {
        self.get().borrow_mut().block_begin = n;
    }

    pub fn set_indent(&'static self, n: usize) {
        self.get().borrow_mut().indent = n as u16;
    }

    pub fn insert_whitespace(&'static self, whitespace: &str) {
        if let Some(line) = self.get().borrow_mut().last_line() {
            line.insert_str(0, whitespace);
        }
    }

    pub fn read_highlighted(&'static self) -> String {
        self.get().borrow().read_highlighted()
    }

    pub fn highlight_line(&'static self, line: &str) -> String {
        self.get().borrow().highlight_line(line)
    }
}
