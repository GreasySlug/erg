/// REPL-specific completion context that bridges ELS completion with REPL
///
/// This module provides a wrapper around ELS completion functionality
/// specifically designed for REPL usage, maintaining state between
/// REPL interactions and managing virtual documents.
use std::path::PathBuf;

use crate::completion;
use crate::completion::get_keyword_completions_fuzzy;
use crate::config::ErgConfig;
use crate::pathutil::NormalizedPathBuf;

/// Manages completion context for REPL sessions
#[derive(Debug)]
pub struct ReplCompletionContext {
    /// Configuration for the Erg compiler (reserved for future use)
    _config: ErgConfig,

    /// Virtual document content (accumulated REPL input)
    virtual_document: Vec<String>,

    /// Placeholder for future ELS integration
    /// Will store type information when integrated with erg_compiler
    _reserved_context: Option<()>,

    /// Evaluated variables from the compiler context
    /// (name, type_info)
    evaluated_variables: Vec<(String, String)>,

    /// Path to the virtual REPL module (reserved for future use)
    _repl_path: NormalizedPathBuf,

    /// Whether the context has been initialized
    initialized: bool,
}

impl ReplCompletionContext {
    /// Create a new REPL completion context
    pub fn new(config: ErgConfig) -> Self {
        let repl_path = NormalizedPathBuf::from(PathBuf::from("<repl>"));

        Self {
            _config: config,
            virtual_document: Vec::new(),
            _reserved_context: None,
            evaluated_variables: Vec::new(),
            _repl_path: repl_path,
            initialized: true,
        }
    }

    /// Check if the context is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get the REPL module name
    pub fn get_repl_module_name(&self) -> &str {
        "<repl>"
    }

    /// Get the number of lines in the virtual document
    pub fn get_line_count(&self) -> usize {
        self.virtual_document.len()
    }

    /// Add a line to the REPL history and update context
    pub fn add_line(&mut self, line: &str) {
        self.virtual_document.push(line.to_string());
        // TODO: Update module context with the new line
        // This will involve parsing and type checking
    }

    /// Update evaluated variables from the compiler context
    pub fn update_evaluated_variables(&mut self, variables: Vec<(String, String)>) {
        self.evaluated_variables = variables;
    }

    /// Get the virtual document content as a single string
    pub fn get_virtual_document_content(&self) -> String {
        self.virtual_document.join("\n")
    }

    /// Get completions for the given input at the specified position
    pub fn get_completions(&self, line: &str, position: usize) -> Vec<String> {
        // For now, use our basic completion logic
        // TODO: Integrate with ELS completion

        // Check if we're completing after a dot (method completion)
        if position > 0 && line.chars().nth(position - 1) == Some('.') {
            // Find the object name before the dot
            let prefix = &line[..position - 1];
            if let Some(obj_name) = prefix.split_whitespace().last() {
                return get_method_completions(obj_name, &self.virtual_document);
            }
        }

        // Extract the word being completed
        let (word, _) = completion::get_word_at_cursor(line, position);

        let mut completions = Vec::new();

        // Add keyword completions
        let keywords = get_keyword_completions_fuzzy(word);
        completions.extend(keywords.into_iter().map(String::from));

        // Add variables and functions from virtual document
        completions.extend(extract_identifiers(&self.virtual_document, word));

        // Add evaluated variables from compiler context
        for (var_name, _type_info) in &self.evaluated_variables {
            if var_name.starts_with(word) && !completions.contains(var_name) {
                completions.push(var_name.clone());
            }
        }

        // Remove duplicates and sort
        completions.sort();
        completions.dedup();

        completions
    }

    /// Get completions with type hint for better filtering
    pub fn get_completions_with_type_hint(
        &self,
        line: &str,
        position: usize,
        expected_type: &str,
    ) -> Vec<String> {
        let all_completions = self.get_completions(line, position);

        // For MVP, prioritize based on simple heuristics
        // TODO: Use actual type information from ELS
        match expected_type {
            "Int" => {
                // Prioritize numeric variables
                let mut sorted = all_completions;
                sorted.sort_by_key(|name| if self.is_numeric_variable(name) { 0 } else { 1 });
                sorted
            }
            _ => all_completions,
        }
    }

    /// Check if a variable is likely numeric based on its definition
    fn is_numeric_variable(&self, name: &str) -> bool {
        for line in &self.virtual_document {
            if line.contains(&format!("{} =", name)) && line.contains(char::is_numeric) {
                return true;
            }
        }
        false
    }
}

/// Extract identifiers (variables and functions) that match the prefix
fn extract_identifiers(document: &[String], prefix: &str) -> Vec<String> {
    let mut identifiers = Vec::new();

    for line in document {
        let trimmed = line.trim();

        // Python-style function definition: "def func_name(params):"
        if trimmed.starts_with("def ") {
            let rest = trimmed.strip_prefix("def ").unwrap();
            if let Some(paren_pos) = rest.find('(') {
                let func_name = rest[..paren_pos].trim();
                if is_valid_identifier(func_name) && func_name.starts_with(prefix) {
                    identifiers.push(func_name.to_string());
                }
            }
        }

        // Variable assignment: "name = value"
        if let Some(equals_pos) = trimmed.find('=') {
            let left_side = trimmed[..equals_pos].trim();

            // Function definition: "func_name param1, param2 ="
            if left_side.contains(' ') {
                if let Some(func_name) = left_side.split_whitespace().next() {
                    if is_valid_identifier(func_name) && func_name.starts_with(prefix) {
                        identifiers.push(func_name.to_string());
                    }
                }
            } else if is_valid_identifier(left_side) && left_side.starts_with(prefix) {
                // Simple variable
                identifiers.push(left_side.to_string());
            }
        }

        // Import statement: "import module_name"
        if trimmed.starts_with("import ") {
            if let Some(module_name) = trimmed.strip_prefix("import ").map(str::trim) {
                if module_name.starts_with(prefix) {
                    identifiers.push(module_name.to_string());
                }
            }
        }
    }

    identifiers
}

/// Check if a string is a valid identifier
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        // First character must be alphabetic or underscore
        if !first.is_alphabetic() && first != '_' {
            return false;
        }

        // Rest can be alphanumeric, underscore, or exclamation mark
        chars.all(|c| c.is_alphanumeric() || c == '_' || c == '!')
    } else {
        false
    }
}

/// Get method completions for an object
fn get_method_completions(obj_name: &str, document: &[String]) -> Vec<String> {
    // For MVP, return common string methods if the object is a string
    for line in document {
        if line.contains(&format!("{} = \"", obj_name))
            || line.contains(&format!("{} = '", obj_name))
        {
            // It's a string
            return vec![
                "upper".to_string(),
                "lower".to_string(),
                "split".to_string(),
                "strip".to_string(),
                "replace".to_string(),
                "startswith".to_string(),
                "endswith".to_string(),
            ];
        }
    }

    // Check for imports
    for line in document {
        if line.trim() == format!("import {}", obj_name) {
            // Return common math functions for math module
            if obj_name == "math" {
                return vec![
                    "sin".to_string(),
                    "cos".to_string(),
                    "tan".to_string(),
                    "sqrt".to_string(),
                    "pi".to_string(),
                    "e".to_string(),
                ];
            }
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_identifiers() {
        let document = vec![
            "x = 1".to_string(),
            "my_var = \"test\"".to_string(),
            "  y  =  2  ".to_string(),
            "add a, b = a + b".to_string(),
            "import math".to_string(),
        ];

        assert_eq!(extract_identifiers(&document, "x"), vec!["x"]);
        assert_eq!(extract_identifiers(&document, "m"), vec!["my_var", "math"]);
        assert_eq!(extract_identifiers(&document, "a"), vec!["add"]);
        assert!(extract_identifiers(&document, "z").is_empty());
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("valid"));
        assert!(is_valid_identifier("_private"));
        assert!(is_valid_identifier("print!"));
        assert!(is_valid_identifier("var123"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123invalid"));
        assert!(!is_valid_identifier("has-dash"));
    }

    #[test]
    fn test_get_method_completions() {
        let document = vec!["text = \"hello\"".to_string(), "num = 42".to_string()];

        let string_methods = get_method_completions("text", &document);
        assert!(string_methods.contains(&"upper".to_string()));
        assert!(string_methods.contains(&"lower".to_string()));

        let num_methods = get_method_completions("num", &document);
        assert!(num_methods.is_empty()); // No methods for numbers in MVP
    }

    #[test]
    fn test_evaluated_variables_completion() {
        let mut context = ReplCompletionContext::new(ErgConfig::default());

        // Add some lines to the virtual document
        context.add_line("x = 1");
        context.add_line("y = \"hello\"");

        // Test without evaluated variables
        let completions = context.get_completions("x", 1);
        assert!(completions.contains(&"x".to_string()));

        // Add evaluated variables from compiler context
        let evaluated_vars = vec![
            ("x".to_string(), "Int".to_string()),
            ("y".to_string(), "Str".to_string()),
            ("z".to_string(), "Float".to_string()),
        ];
        context.update_evaluated_variables(evaluated_vars);

        // Test with evaluated variables - should include z
        let completions = context.get_completions("z", 1);
        assert!(completions.contains(&"z".to_string()));

        // Test that it doesn't duplicate variables
        let completions = context.get_completions("x", 1);
        let x_count = completions.iter().filter(|&s| s == "x").count();
        assert_eq!(x_count, 1);
    }
}
