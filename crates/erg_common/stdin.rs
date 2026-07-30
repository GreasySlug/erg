#[cfg(feature = "full-repl")]
use std::io::Write;
#[cfg(feature = "full-repl")]
use std::process::Command;
#[cfg(feature = "full-repl")]
use std::process::Output;
use std::sync::OnceLock;

#[cfg(feature = "full-repl")]
use crossterm::{
    cursor::{self, MoveToColumn},
    event::{read, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::Print,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};

use crate::shared::Shared;

/// Result of cell input operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellResult {
    /// Cell is ready to execute (Ctrl+Enter pressed)
    Execute(Cell),
    /// User requested exit (Ctrl+D/Z pressed)
    Exit,
}

/// Result of checking if code is complete
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeCompleteness {
    /// Code is syntactically complete and can be executed
    Complete,
    /// More lines are needed: an indented block is expected
    /// (e.g. after `=`, `=>`, `->`, or a class accessor line)
    ExpectsBlock,
    /// More lines are needed at the SAME indent level
    /// (e.g. after a decorator line, which must be followed by a
    /// definition at the same indentation)
    Continuation,
    /// More lines are needed: an opening delimiter is not closed yet
    /// (brackets, `"""` strings, `#[ ]#` comments)
    Unclosed,
    /// Code has a syntax error (should still be executed to show error)
    SyntaxError,
}

impl CodeCompleteness {
    /// Returns true if more input lines are needed to complete the code
    pub const fn is_incomplete(&self) -> bool {
        matches!(
            self,
            Self::ExpectsBlock | Self::Continuation | Self::Unclosed
        )
    }
}

/// Function type for checking code completeness
/// Takes the accumulated code string and returns completeness status
pub type CompletenessChecker = Box<dyn Fn(&str) -> CodeCompleteness + Send + Sync>;

/// Cursor position within a cell
#[derive(Debug, Clone, Default)]
pub struct CellCursor {
    /// Row index within the cell (0-based)
    pub row: usize,
    /// Column (character) position within the row
    pub col: usize,
}

/// A multi-line input cell
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cell {
    /// Lines within this cell
    pub lines: Vec<String>,
}

impl Cell {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
        }
    }

    pub fn with_initial_indent(indent: &str) -> Self {
        Self {
            lines: vec![indent.to_string()],
        }
    }

    pub fn to_code(&self) -> String {
        self.lines.join("\n")
    }

    pub fn from_code(code: &str) -> Self {
        let lines = if code.is_empty() {
            vec![String::new()]
        } else {
            code.lines().map(|l| l.to_string()).collect()
        };
        Self { lines }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.trim().is_empty())
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

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
    history_input_position: usize,
    indent: u16,
    // Cell-based input fields
    cell_history: Vec<Cell>,
    cell_history_position: usize,
    /// Optional callback for checking code completeness (for smart Enter behavior)
    completeness_checker: Option<CompletenessChecker>,
}

impl std::fmt::Debug for StdinReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdinReader")
            .field("block_begin", &self.block_begin)
            .field("lineno", &self.lineno)
            .field("buf", &self.buf)
            .field("history_input_position", &self.history_input_position)
            .field("indent", &self.indent)
            .field("cell_history", &self.cell_history)
            .field("cell_history_position", &self.cell_history_position)
            .field(
                "completeness_checker",
                &self.completeness_checker.as_ref().map(|_| "<fn>"),
            )
            .finish()
    }
}

impl StdinReader {
    #[cfg(all(feature = "full-repl", target_os = "linux"))]
    fn access_clipboard() -> Option<Output> {
        if let Ok(str) = std::fs::read("/proc/sys/kernel/osrelease") {
            if let Ok(str) = std::str::from_utf8(&str) {
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
        let indent_spaces = "    ".repeat((self.indent.saturating_sub(1)) as usize);
        line.push_str(&indent_spaces);
        let mut position = line.len();
        let mut consult_history = false;
        let mut stdout = std::io::stdout();
        execute!(
            stdout,
            MoveToColumn(4),
            Clear(ClearType::UntilNewLine),
            Print(line.to_owned()),
            MoveToColumn(4 + position as u16)
        )?;
        while let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = read()?
        {
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
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                    if position > 0 {
                        *line = line[position..].to_string();
                        position = 0;
                    }
                }
                (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                    line.truncate(position);
                }
                (_, KeyModifiers::CONTROL) => continue,
                (KeyCode::Tab, _) => {
                    line.insert_str(position, "    ");
                    position += 4;
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
                    let spaces_before = line[..position]
                        .chars()
                        .rev()
                        .take_while(|&c| c == ' ')
                        .count();
                    let delete_count = if spaces_before >= 4 && position.is_multiple_of(4) {
                        4
                    } else if spaces_before > 0 && !position.is_multiple_of(4) {
                        position % 4
                    } else {
                        1
                    };
                    for _ in 0..delete_count.min(position) {
                        line.remove(position - 1);
                        position -= 1;
                    }
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
                        // Reset to new line with indent spaces
                        let indent_spaces = "    ".repeat((self.indent.saturating_sub(1)) as usize);
                        *line = indent_spaces;
                        position = line.len();
                        self.history_input_position += 1;
                        execute!(
                            stdout,
                            MoveToColumn(4),
                            Clear(ClearType::UntilNewLine),
                            Print(line.to_owned()),
                            MoveToColumn(4 + position as u16)
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
            execute!(
                stdout,
                MoveToColumn(4),
                Clear(ClearType::UntilNewLine),
                Print(line.to_owned()),
                MoveToColumn(4 + position as u16)
            )?;
        }
        if !consult_history {
            self.history_input_position = self.buf.len() + 1;
        }
        Ok(())
    }

    #[cfg(feature = "full-repl")]
    fn render_cell(
        &self,
        stdout: &mut impl Write,
        cell: &Cell,
        cursor: &CellCursor,
        start_row: u16,
    ) -> std::io::Result<()> {
        for (i, line) in cell.lines.iter().enumerate() {
            let prompt = if i == 0 { ">>> " } else { "... " };
            execute!(
                stdout,
                cursor::MoveTo(0, start_row + i as u16),
                Clear(ClearType::CurrentLine),
                Print(prompt),
                Print(line),
            )?;
        }
        // Position cursor
        execute!(
            stdout,
            cursor::MoveTo(4 + cursor.col as u16, start_row + cursor.row as u16),
        )?;
        stdout.flush()?;
        Ok(())
    }

    #[cfg(feature = "full-repl")]
    fn cell_insert_newline(cell: &mut Cell, cursor: &mut CellCursor) {
        let current_line = &cell.lines[cursor.row];
        let (before, after) = current_line.split_at(cursor.col);
        let before = before.to_string();
        let after = after.to_string();

        let indent: String = current_line
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();

        cell.lines[cursor.row] = before;
        cell.lines
            .insert(cursor.row + 1, format!("{}{}", indent, after));
        cursor.row += 1;
        cursor.col = indent.len();
    }

    #[cfg(feature = "full-repl")]
    fn cell_backspace(cell: &mut Cell, cursor: &mut CellCursor) -> bool {
        if cursor.col == 0 {
            if cursor.row == 0 {
                return false;
            }
            let current_line = cell.lines.remove(cursor.row);
            cursor.row -= 1;
            cursor.col = cell.lines[cursor.row].len();
            cell.lines[cursor.row].push_str(&current_line);
            true
        } else {
            let line = &cell.lines[cursor.row];
            let spaces_before = line[..cursor.col]
                .chars()
                .rev()
                .take_while(|&c| c == ' ')
                .count();
            let delete_count = if spaces_before >= 4 && cursor.col.is_multiple_of(4) {
                4
            } else if spaces_before > 0 && !cursor.col.is_multiple_of(4) {
                cursor.col % 4
            } else {
                1
            };
            for _ in 0..delete_count.min(cursor.col) {
                cell.lines[cursor.row].remove(cursor.col - 1);
                cursor.col -= 1;
            }
            true
        }
    }

    #[cfg(feature = "full-repl")]
    fn cell_delete(cell: &mut Cell, cursor: &CellCursor) -> bool {
        let line_len = cell.lines[cursor.row].len();
        if cursor.col == line_len {
            if cursor.row == cell.lines.len() - 1 {
                return false;
            }
            let next_line = cell.lines.remove(cursor.row + 1);
            cell.lines[cursor.row].push_str(&next_line);
            true
        } else {
            cell.lines[cursor.row].remove(cursor.col);
            true
        }
    }

    #[cfg(feature = "full-repl")]
    pub fn read_cell(&mut self) -> CellResult {
        let mut stdout = std::io::stdout();

        stdout.flush().unwrap();
        let start_row = cursor::position().map(|(_, r)| r).unwrap_or(0);

        enable_raw_mode().unwrap();

        let indent_spaces = "    ".repeat((self.indent.saturating_sub(1)) as usize);
        let mut cell = Cell::with_initial_indent(&indent_spaces);
        let mut cursor = CellCursor {
            row: 0,
            col: indent_spaces.len(),
        };

        self.render_cell(&mut stdout, &cell, &cursor, start_row)
            .unwrap();

        loop {
            if let Ok(Event::Key(KeyEvent {
                code, modifiers, ..
            })) = read()
            {
                match (code, modifiers) {
                    (KeyCode::Enter, KeyModifiers::CONTROL) => {
                        execute!(
                            stdout,
                            cursor::MoveTo(0, start_row + cell.lines.len() as u16),
                            Print("\n"),
                        )
                        .unwrap();
                        stdout.flush().unwrap();
                        disable_raw_mode().unwrap();
                        self.cell_history.push(cell.clone());
                        self.cell_history_position = self.cell_history.len();
                        for line in &cell.lines {
                            self.buf.push(line.clone());
                            self.lineno += 1;
                        }
                        return CellResult::Execute(cell);
                    }
                    (KeyCode::Char('d'), KeyModifiers::CONTROL)
                    | (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
                        execute!(
                            stdout,
                            cursor::MoveTo(0, start_row + cell.lines.len() as u16),
                            Print("\n"),
                        )
                        .unwrap();
                        stdout.flush().unwrap();
                        disable_raw_mode().unwrap();
                        return CellResult::Exit;
                    }
                    (KeyCode::Enter, KeyModifiers::NONE) => {
                        let code = cell.to_code();

                        // Check if we should execute (code is complete or has syntax error)
                        let should_execute = if let Some(ref checker) = self.completeness_checker {
                            match checker(&code) {
                                CodeCompleteness::Complete | CodeCompleteness::SyntaxError => {
                                    !code.trim().is_empty()
                                }
                                CodeCompleteness::ExpectsBlock
                                | CodeCompleteness::Continuation
                                | CodeCompleteness::Unclosed => false,
                            }
                        } else {
                            // No checker: always insert newline (legacy behavior)
                            false
                        };

                        if should_execute {
                            // Execute the code (same as Ctrl+Enter path)
                            execute!(
                                stdout,
                                cursor::MoveTo(0, start_row + cell.lines.len() as u16),
                                Print("\n"),
                            )
                            .unwrap();
                            stdout.flush().unwrap();
                            disable_raw_mode().unwrap();
                            self.cell_history.push(cell.clone());
                            self.cell_history_position = self.cell_history.len();
                            for line in &cell.lines {
                                self.buf.push(line.clone());
                                self.lineno += 1;
                            }
                            return CellResult::Execute(cell);
                        }

                        // Insert newline (code is incomplete)
                        Self::cell_insert_newline(&mut cell, &mut cursor);
                        for i in 0..cell.lines.len() {
                            execute!(
                                stdout,
                                cursor::MoveTo(0, start_row + i as u16),
                                Clear(ClearType::CurrentLine),
                            )
                            .unwrap();
                        }
                    }
                    (KeyCode::Up, KeyModifiers::NONE) => {
                        if cursor.row == 0 {
                            if self.cell_history_position > 0 {
                                for i in 0..cell.lines.len() {
                                    execute!(
                                        stdout,
                                        cursor::MoveTo(0, start_row + i as u16),
                                        Clear(ClearType::CurrentLine),
                                    )
                                    .unwrap();
                                }
                                self.cell_history_position -= 1;
                                cell = self.cell_history[self.cell_history_position].clone();
                                cursor.row = 0;
                                cursor.col = cell.lines[0].len();
                            }
                        } else {
                            cursor.row -= 1;
                            cursor.col = cursor.col.min(cell.lines[cursor.row].len());
                        }
                    }
                    (KeyCode::Down, KeyModifiers::NONE) => {
                        if cursor.row == cell.lines.len() - 1 {
                            if self.cell_history_position < self.cell_history.len() {
                                for i in 0..cell.lines.len() {
                                    execute!(
                                        stdout,
                                        cursor::MoveTo(0, start_row + i as u16),
                                        Clear(ClearType::CurrentLine),
                                    )
                                    .unwrap();
                                }
                                self.cell_history_position += 1;
                                if self.cell_history_position < self.cell_history.len() {
                                    cell = self.cell_history[self.cell_history_position].clone();
                                } else {
                                    let indent_spaces =
                                        "    ".repeat((self.indent.saturating_sub(1)) as usize);
                                    cell = Cell::with_initial_indent(&indent_spaces);
                                }
                                cursor.row = 0;
                                cursor.col = cell.lines[0].len();
                            }
                        } else {
                            cursor.row += 1;
                            cursor.col = cursor.col.min(cell.lines[cursor.row].len());
                        }
                    }
                    (KeyCode::Left, KeyModifiers::NONE) => {
                        if cursor.col == 0 {
                            if cursor.row > 0 {
                                cursor.row -= 1;
                                cursor.col = cell.lines[cursor.row].len();
                            }
                        } else {
                            cursor.col -= 1;
                        }
                    }
                    (KeyCode::Right, KeyModifiers::NONE) => {
                        if cursor.col == cell.lines[cursor.row].len() {
                            if cursor.row < cell.lines.len() - 1 {
                                cursor.row += 1;
                                cursor.col = 0;
                            }
                        } else {
                            cursor.col += 1;
                        }
                    }
                    (KeyCode::Home, KeyModifiers::NONE) => {
                        cursor.col = 0;
                    }
                    (KeyCode::End, KeyModifiers::NONE) => {
                        cursor.col = cell.lines[cursor.row].len();
                    }
                    (KeyCode::Backspace, KeyModifiers::NONE) => {
                        Self::cell_backspace(&mut cell, &mut cursor);
                        for i in cursor.row..cell.lines.len() + 1 {
                            execute!(
                                stdout,
                                cursor::MoveTo(0, start_row + i as u16),
                                Clear(ClearType::CurrentLine),
                            )
                            .unwrap();
                        }
                    }
                    (KeyCode::Delete, KeyModifiers::NONE) => {
                        Self::cell_delete(&mut cell, &cursor);
                        for i in cursor.row..cell.lines.len() + 1 {
                            execute!(
                                stdout,
                                cursor::MoveTo(0, start_row + i as u16),
                                Clear(ClearType::CurrentLine),
                            )
                            .unwrap();
                        }
                    }
                    (KeyCode::Tab, KeyModifiers::NONE) => {
                        cell.lines[cursor.row].insert_str(cursor.col, "    ");
                        cursor.col += 4;
                    }
                    (KeyCode::Char('v'), KeyModifiers::CONTROL) => {
                        if let Some(output) = Self::access_clipboard() {
                            let clipboard =
                                String::from_utf8_lossy(&output.stdout).trim().to_string();
                            let paste_lines: Vec<&str> = clipboard.lines().collect();
                            if paste_lines.is_empty() {
                                continue;
                            }
                            let first_line =
                                paste_lines[0].replace(|c: char| c.len_utf8() >= 2, "");
                            cell.lines[cursor.row].insert_str(cursor.col, &first_line);
                            cursor.col += first_line.len();
                            for (i, line) in paste_lines.iter().skip(1).enumerate() {
                                let clean_line = line.replace(|c: char| c.len_utf8() >= 2, "");
                                cell.lines.insert(cursor.row + 1 + i, clean_line);
                            }
                            if paste_lines.len() > 1 {
                                cursor.row += paste_lines.len() - 1;
                                cursor.col = cell.lines[cursor.row].len();
                            }
                        }
                    }
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                        if cursor.col > 0 {
                            cell.lines[cursor.row] =
                                cell.lines[cursor.row][cursor.col..].to_string();
                            cursor.col = 0;
                        }
                    }
                    (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                        cell.lines[cursor.row].truncate(cursor.col);
                    }
                    (_, KeyModifiers::CONTROL) => continue,
                    (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT)
                        if c.len_utf8() < 2 =>
                    {
                        cell.lines[cursor.row].insert(cursor.col, c);
                        cursor.col += 1;
                    }
                    _ => {}
                }
                self.render_cell(&mut stdout, &cell, &cursor, start_row)
                    .unwrap();
            }
        }
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
                history_input_position: 1,
                indent: 1,
                cell_history: vec![],
                cell_history_position: 0,
                completeness_checker: None,
            })
        })
    }

    #[cfg(feature = "full-repl")]
    pub fn read(&'static self) -> String {
        self.get().borrow_mut().read()
    }

    #[cfg(feature = "full-repl")]
    pub fn read_cell(&'static self) -> CellResult {
        self.get().borrow_mut().read_cell()
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
            // Don't insert if:
            // 1. Line already starts with the expected whitespace (full-repl pre-fill)
            // 2. Line starts with non-whitespace (user intentionally omitted indent)
            if !line.starts_with(whitespace)
                && (line.is_empty() || line.starts_with(char::is_whitespace))
            {
                line.insert_str(0, whitespace);
            }
        }
    }

    /// Set the completeness checker callback for smart Enter behavior
    pub fn set_completeness_checker(&'static self, checker: CompletenessChecker) {
        self.get().borrow_mut().completeness_checker = Some(checker);
    }

    /// Clear the completeness checker (revert to basic REPL mode)
    pub fn clear_completeness_checker(&'static self) {
        self.get().borrow_mut().completeness_checker = None;
    }

    /// Push a line to the buffer (for non-full-repl mode)
    pub fn push_line(&'static self, line: String) {
        let mut reader = self.get().borrow_mut();
        reader.buf.push(line);
        reader.lineno += 1;
    }
}
