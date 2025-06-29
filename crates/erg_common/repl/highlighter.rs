use crate::style::{Color, SyntaxColors, THEME};
use std::borrow::Cow;

/// Lightweight syntax highlighter for REPL
#[derive(Debug)]
pub struct ReplHighlighter {
    colors: SyntaxColors,
    enabled: bool,
}

impl ReplHighlighter {
    pub fn new() -> Self {
        Self {
            colors: THEME.syntax,
            enabled: Self::is_color_supported(),
        }
    }

    pub fn with_colors(colors: SyntaxColors) -> Self {
        Self {
            colors,
            enabled: Self::is_color_supported(),
        }
    }

    pub fn disable_colors(&mut self) {
        self.enabled = false;
    }

    pub fn enable_colors(&mut self) {
        self.enabled = true;
    }

    /// Check if the terminal supports color output
    fn is_color_supported() -> bool {
        // Check for common environment variables that indicate color support
        if let Ok(term) = std::env::var("TERM") {
            if term.contains("color") || term == "xterm" || term.starts_with("xterm-") {
                return true;
            }
        }

        // Check for NO_COLOR environment variable
        if std::env::var("NO_COLOR").is_ok() {
            return false;
        }

        // Check for FORCE_COLOR environment variable
        if std::env::var("FORCE_COLOR").is_ok() {
            return true;
        }

        // Default to enabled for common terminals
        true
    }

    /// Classify a symbol (identifier) for syntax highlighting
    fn classify_symbol(&self, name: &str) -> &'static str {
        // Check control flow keywords
        match name {
            "if" | "if!" | "while" | "while!" | "for" | "for!" | "match" | "match!" | "try"
            | "try!" | "with" | "with!" | "discard" | "assert" => self.colors.keyword.as_str(),
            // Operator keywords
            "and" | "or" | "in" | "notin" | "contains" | "is!" | "isnot!" | "ref" | "ref!"
            | "as" => self.colors.operator.as_str(),
            // Literal keywords
            "True" | "False" | "None" | "Ellipsis" | "Inf" => self.colors.keyword.as_str(),
            // Builtin procedures
            name if name.ends_with('!') => match name {
                "print!" | "input!" | "assert!" | "panic!" | "todo!" | "unreachable!"
                | "type_of!" | "isinstance!" | "issubclass!" | "hasattr!" | "getattr!"
                | "setattr!" | "delattr!" | "dir!" | "vars!" | "globals!" | "locals!"
                | "callable!" | "compile!" | "eval!" | "exec!" | "exit!" | "quit!" | "open!"
                | "import!" | "reload!" => self.colors.keyword.as_str(),
                _ => self.colors.function.as_str(),
            },
            // Other reserved words
            "class" | "trait" | "import" | "from" | "def" | "lambda" | "return" | "yield"
            | "break" | "continue" | "pass" | "del" | "global" | "nonlocal" | "async" | "await"
            | "except" | "finally" | "raise" => self.colors.keyword.as_str(),
            // Function names (heuristic: lowercase ending with function-like pattern)
            name if name.chars().next().map_or(false, |c| c.is_lowercase()) => {
                self.colors.function.as_str()
            }
            // Class names (heuristic: starts with uppercase)
            name if name.chars().next().map_or(false, |c| c.is_uppercase()) => {
                self.colors.class.as_str()
            }
            // Default to no color
            _ => Color::Reset.as_str(),
        }
    }

    /// Simple token-based highlighting for basic cases
    pub fn highlight_line(&self, line: &str) -> String {
        if !self.enabled || line.trim().is_empty() {
            return line.to_string();
        }

        // Simple token-based highlighting using character-by-character parsing
        let mut result = String::new();
        let mut i = 0;

        while i < line.len() {
            let ch = line.chars().nth(i).unwrap();

            match ch {
                // String literals
                '"' => {
                    result.push_str(self.colors.string.as_str());
                    result.push(ch);
                    i += 1;

                    // Find end of string
                    while i < line.len() {
                        let ch = line.chars().nth(i).unwrap();
                        result.push(ch);
                        if ch == '"' {
                            i += 1;
                            break;
                        }
                        if ch == '\\' && i + 1 < line.len() {
                            // Skip escaped character
                            i += 1;
                            if i < line.len() {
                                result.push(line.chars().nth(i).unwrap());
                                i += 1;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    result.push_str(Color::Reset.as_str());
                }
                // Single-character string literals
                '\'' => {
                    result.push_str(self.colors.string.as_str());
                    result.push(ch);
                    i += 1;

                    // Find end of string
                    while i < line.len() {
                        let ch = line.chars().nth(i).unwrap();
                        result.push(ch);
                        if ch == '\'' {
                            i += 1;
                            break;
                        }
                        if ch == '\\' && i + 1 < line.len() {
                            // Skip escaped character
                            i += 1;
                            if i < line.len() {
                                result.push(line.chars().nth(i).unwrap());
                                i += 1;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    result.push_str(Color::Reset.as_str());
                }
                // Numbers
                ch if ch.is_ascii_digit() => {
                    result.push_str(self.colors.number.as_str());
                    while i < line.len() {
                        let ch = line.chars().nth(i).unwrap();
                        if ch.is_ascii_digit() || ch == '.' || ch == '_' {
                            result.push(ch);
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    result.push_str(Color::Reset.as_str());
                }
                // Identifiers and keywords
                ch if ch.is_alphabetic() || ch == '_' => {
                    let start = i;
                    while i < line.len() {
                        let ch = line.chars().nth(i).unwrap();
                        if ch.is_alphanumeric() || ch == '_' || ch == '!' {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    let word = &line[start..i];
                    let color = self.classify_symbol(word);
                    if color != Color::Reset.as_str() {
                        result.push_str(color);
                        result.push_str(word);
                        result.push_str(Color::Reset.as_str());
                    } else {
                        result.push_str(word);
                    }
                }
                // Operators
                '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' => {
                    result.push_str(self.colors.operator.as_str());
                    result.push(ch);
                    result.push_str(Color::Reset.as_str());
                    i += 1;
                }
                // Comments
                '#' => {
                    result.push_str(self.colors.comment.as_str());
                    while i < line.len() {
                        result.push(line.chars().nth(i).unwrap());
                        i += 1;
                    }
                    result.push_str(Color::Reset.as_str());
                }
                // Everything else
                _ => {
                    result.push(ch);
                    i += 1;
                }
            }
        }

        result
    }

    /// Highlight a line for rustyline integration
    pub fn highlight_for_rustyline<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if !self.enabled {
            return Cow::Borrowed(line);
        }

        let highlighted = self.highlight_line(line);
        if highlighted == line {
            Cow::Borrowed(line)
        } else {
            Cow::Owned(highlighted)
        }
    }
}

impl Default for ReplHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_highlighting() {
        let highlighter = ReplHighlighter::new();

        // Test control flow keywords
        let result = highlighter.highlight_line("if! True:");
        assert!(result.contains("\x1b[95m")); // Should contain magenta for keyword

        // Test builtin procedures
        let result = highlighter.highlight_line("print! \"hello\"");
        assert!(result.contains("\x1b[95m")); // Should contain magenta for print!
        assert!(result.contains("\x1b[92m")); // Should contain green for string
    }

    #[test]
    fn test_literal_highlighting() {
        let highlighter = ReplHighlighter::new();

        let result = highlighter.highlight_line("x = 42");
        assert!(result.contains("\x1b[93m")); // Should contain yellow for number

        let result = highlighter.highlight_line("s = \"hello\"");
        assert!(result.contains("\x1b[92m")); // Should contain green for string
    }

    #[test]
    fn test_no_color_when_disabled() {
        let mut highlighter = ReplHighlighter::new();
        highlighter.disable_colors();

        let result = highlighter.highlight_line("if! True:");
        assert_eq!(result, "if! True:");
    }

    #[test]
    fn test_operators() {
        let highlighter = ReplHighlighter::new();

        let result = highlighter.highlight_line("x = 1 + 2");
        assert!(result.contains("\x1b[96m")); // Should contain cyan for operators
    }

    #[test]
    fn test_comments() {
        let highlighter = ReplHighlighter::new();

        let result = highlighter.highlight_line("# This is a comment");
        assert!(result.contains("\x1b[97m")); // Should contain white for comment
    }
}
