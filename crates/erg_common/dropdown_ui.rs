/// Dropdown UI for displaying and selecting completion candidates
///
/// This module provides a terminal-based dropdown interface for
/// tab completion candidates in the Erg REPL.

#[cfg(feature = "full-repl")]
use crossterm::{
    cursor::{self, MoveToColumn},
    execute,
    style::Print,
    terminal::{self, Clear, ClearType},
};
#[cfg(feature = "full-repl")]
use std::io::Write;

/// Manages the dropdown UI state and rendering
#[derive(Debug, Clone)]
pub struct DropdownUI {
    candidates: Vec<String>,
    selected_index: usize,
    cursor_column: u16,
    terminal_width: u16,
    is_active: bool,
    scroll_offset: usize,
}

impl DropdownUI {
    /// Maximum number of visible items in the dropdown
    pub const MAX_HEIGHT: usize = 10;

    /// Create a new dropdown UI
    pub fn new(candidates: Vec<String>, cursor_column: u16, terminal_width: u16) -> Self {
        Self {
            candidates,
            selected_index: 0,
            cursor_column,
            terminal_width,
            is_active: false,
            scroll_offset: 0,
        }
    }

    /// Getters for immutable access
    pub fn candidates(&self) -> &[String] {
        &self.candidates
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Returns a new DropdownUI with selection moved down (with wrapping)
    pub fn with_selection_down(self) -> Self {
        match self.candidates.is_empty() {
            true => self,
            false => {
                let new_index = (self.selected_index + 1) % self.candidates.len();
                self.with_selected_index(new_index)
            }
        }
    }

    /// Returns a new DropdownUI with selection moved up (with wrapping)
    pub fn with_selection_up(self) -> Self {
        match self.candidates.is_empty() {
            true => self,
            false => {
                let new_index = match self.selected_index {
                    0 => self.candidates.len() - 1,
                    n => n - 1,
                };
                self.with_selected_index(new_index)
            }
        }
    }

    /// Helper function to create a new DropdownUI with updated index and scroll
    fn with_selected_index(self, new_index: usize) -> Self {
        let scroll_offset =
            Self::calculate_scroll_offset(new_index, self.scroll_offset, self.candidates.len());

        Self {
            selected_index: new_index,
            scroll_offset,
            ..self
        }
    }

    /// Calculate scroll offset to keep selected item visible (pure function)
    fn calculate_scroll_offset(
        selected_index: usize,
        current_offset: usize,
        candidates_len: usize,
    ) -> usize {
        let max_visible = Self::MAX_HEIGHT.min(candidates_len);

        match selected_index {
            idx if idx < current_offset => idx,
            idx if idx >= current_offset + max_visible => idx.saturating_sub(max_visible - 1),
            _ => current_offset,
        }
    }

    /// Get currently visible candidates with their indices
    pub fn visible_candidates(&self) -> impl Iterator<Item = (usize, &String)> {
        let max_visible = Self::MAX_HEIGHT.min(self.candidates.len());
        let end = (self.scroll_offset + max_visible).min(self.candidates.len());

        self.candidates[self.scroll_offset..end]
            .iter()
            .enumerate()
            .map(move |(i, c)| (self.scroll_offset + i, c))
    }

    /// Returns a new DropdownUI with activated state
    pub fn with_active(self, active: bool) -> Self {
        Self {
            is_active: active,
            ..self
        }
    }

    /// Get the currently selected candidate
    pub fn selected_candidate(&self) -> Option<&String> {
        match (self.is_active, self.candidates.get(self.selected_index)) {
            (true, Some(candidate)) => Some(candidate),
            _ => None,
        }
    }

    /// Calculate the width needed for the dropdown
    fn dropdown_width(&self) -> usize {
        self.candidates
            .iter()
            .map(|c| c.len())
            .max()
            .unwrap_or(0)
            .saturating_add(4) // padding for selection indicator and borders
            .min(40) // cap at reasonable maximum
    }

    /// Calculate render position considering terminal boundaries
    pub fn render_position(&self) -> (u16, u16) {
        let width = self.dropdown_width() as u16;
        let x = match self.cursor_column + width > self.terminal_width {
            true => self.terminal_width.saturating_sub(width),
            false => self.cursor_column,
        };

        (x, 1) // y position will be calculated during rendering
    }

    /// Render the dropdown UI (returns formatted string)
    pub fn render(&self) -> String {
        let width = self.dropdown_width() + 1;
        let visible: Vec<_> = self.visible_candidates().collect();

        let top_border = self.render_top_border(width);
        let bottom_border = self.render_bottom_border(width, visible.len());
        let _empty_line = format!("│{}│", " ".repeat(width.saturating_sub(2)));

        let candidates_str = visible
            .iter()
            .map(|(idx, candidate)| self.render_candidate(*idx, candidate, width))
            .collect::<Vec<_>>()
            .join("\n");

        format!("{}\n{}\n{}", top_border, candidates_str, bottom_border)
    }

    /// Render top border with scroll indicator
    fn render_top_border(&self, width: usize) -> String {
        match self.scroll_offset > 0 {
            true => format!("╭↑{}╮", "─".repeat(width.saturating_sub(3))),
            false => format!("╭{}╮", "─".repeat(width.saturating_sub(2))),
        }
    }

    /// Render bottom border with scroll indicator
    fn render_bottom_border(&self, width: usize, visible_count: usize) -> String {
        let has_more = self.scroll_offset + visible_count < self.candidates.len();
        match has_more {
            true => format!("╰{}↓╯", "─".repeat(width.saturating_sub(3))),
            false => format!("╰{}╯", "─".repeat(width.saturating_sub(2))),
        }
    }

    /// Render a single candidate line
    fn render_candidate(&self, idx: usize, candidate: &str, width: usize) -> String {
        let indicator = match idx == self.selected_index {
            true => "▶ ",
            false => "  ",
        };
        let padded = format!("{:<width$}", candidate, width = width.saturating_sub(4));
        format!("│{}{}│", indicator, padded)
    }

    /// Display the dropdown in the terminal
    #[cfg(feature = "full-repl")]
    pub fn display(&self, stdout: &mut std::io::Stdout) -> std::io::Result<()> {
        match (self.is_active, self.candidates.is_empty()) {
            (false, _) | (_, true) => return Ok(()),
            _ => {}
        }

        let (x, _) = self.render_position();
        let rendered = self.render();
        let dropdown_lines = rendered.lines().count();

        // Calculate rendering constraints
        let (_, terminal_height) = terminal::size().unwrap_or((80, 24));
        let cursor_pos = cursor::position().unwrap_or((0, 0));
        let available_lines = terminal_height
            .saturating_sub(cursor_pos.1)
            .saturating_sub(1) as usize;

        // If dropdown doesn't fit, scroll the terminal
        if dropdown_lines > available_lines {
            let lines_to_scroll = dropdown_lines.saturating_sub(available_lines);
            // Scroll the terminal by the necessary amount
            for _ in 0..lines_to_scroll {
                execute!(stdout, Print("\n"))?;
            }
            // Update cursor position after scrolling
            execute!(stdout, cursor::MoveToPreviousLine(lines_to_scroll as u16))?;
        }

        // Execute terminal commands
        execute!(stdout, cursor::SavePosition)?;
        execute!(stdout, cursor::MoveToNextLine(1))?;

        rendered.lines().try_for_each(|line| {
            execute!(
                stdout,
                MoveToColumn(x),
                Print(line),
                cursor::MoveToNextLine(1)
            )
        })?;

        execute!(stdout, cursor::RestorePosition)?;
        stdout.flush()
    }

    /// Clear the dropdown from the terminal
    #[cfg(feature = "full-repl")]
    pub fn clear(&self, stdout: &mut std::io::Stdout) -> std::io::Result<()> {
        use crossterm::terminal;

        if self.candidates.is_empty() {
            return Ok(());
        }

        let visible_count = self.visible_candidates().count();
        let total_lines = visible_count + 4; // +2 for borders +2 for empty lines

        // Calculate rendering constraints
        let (_, terminal_height) = terminal::size().unwrap_or((80, 24));
        let cursor_pos = cursor::position().unwrap_or((0, 0));
        let _available_lines = terminal_height
            .saturating_sub(cursor_pos.1)
            .saturating_sub(1) as usize;

        // Clear all dropdown lines, even if they required scrolling
        let lines_to_clear = total_lines;

        // Execute terminal commands
        execute!(stdout, cursor::SavePosition)?;
        execute!(stdout, cursor::MoveToNextLine(1))?;

        (0..lines_to_clear).try_for_each(|_| {
            execute!(
                stdout,
                Clear(ClearType::CurrentLine),
                cursor::MoveToNextLine(1)
            )
        })?;

        execute!(stdout, cursor::RestorePosition)?;
        stdout.flush()
    }
}
