/// REPL Tab completion functionality for Erg
///
/// This module provides basic tab completion for keywords and identifiers
/// in the Erg REPL environment.

/// Check if character is part of a word (identifier)
pub fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '!'
}

/// Extract word at cursor position
pub fn get_word_at_cursor(line: &str, cursor_pos: usize) -> (&str, usize) {
    if line.is_empty() {
        return ("", 0);
    }

    let chars: Vec<char> = line.chars().collect();
    let actual_pos = cursor_pos.min(chars.len());

    // Find start of current word (going backwards from cursor position)
    let mut start = actual_pos;
    while start > 0 {
        let ch = chars[start - 1];
        if !is_word_char(ch) {
            break;
        }
        start -= 1;
    }

    // If we found a word start, find the end
    if start < actual_pos || (start < chars.len() && is_word_char(chars[start])) {
        let mut end = start;
        while end < chars.len() && is_word_char(chars[end]) {
            end += 1;
        }

        // Check if cursor is within or at the end of this word
        if actual_pos <= end {
            return (&line[start..end], start);
        }
    }

    // No word found at cursor position
    ("", actual_pos)
}

const ERG_KEYWORDS: &[&str] = &[
    "if", "if!", "elif", "else", "for", "for!", "while", "while!", "do", "do!", "loop", "break",
    "continue", "Class", "trait", "inherit", "import", "const", "return", "assert", "match",
    "case", "and", "or", "not", "in", "is", "Del", "pass", "True", "False", "self", "Self",
    "print!", "println!", "log!", "input!", "Int", "Float", "Bool", "Str", "List", "Tuple", "Dict",
    "Set", "Int!", "Float!", "Bool!", "Str!", "List!", "Tuple!", "Dict!", "Set!",
];

/// Get keyword completions for given prefix
pub fn get_keyword_completions(prefix: &str) -> Vec<&str> {
    if prefix.is_empty() {
        vec![
            "if", "for", "while", "Class", "pyimport", "print!", "assert",
        ]
    } else {
        ERG_KEYWORDS
            .iter()
            .filter(|&keyword| keyword.starts_with(prefix))
            .copied()
            .collect()
    }
}

/// Get identifier completions from available identifiers
pub fn get_identifier_completions(prefix: &str, identifiers: &[String]) -> Vec<String> {
    identifiers
        .iter()
        .filter(|identifier| identifier.starts_with(prefix))
        .cloned()
        .collect()
}

/// Actions that can result from completion
#[derive(Debug, PartialEq)]
pub enum CompletionAction {
    None,
    CompleteUnique(String),
    CompleteCommon(String),
    ShowCandidates(Vec<String>),
}

/// Process completion candidates and determine action
pub fn process_completion_candidates(
    candidates: Vec<String>,
    line: &str,
    cursor_pos: usize,
) -> CompletionAction {
    match candidates.len() {
        0 => CompletionAction::None,
        1 => CompletionAction::CompleteUnique(candidates[0].clone()),
        _ => {
            // Find common prefix among all candidates
            let common_prefix = find_common_prefix(&candidates);
            let (current_word, _) = get_word_at_cursor(line, cursor_pos);

            // If common prefix is longer than current word, complete to common prefix
            if common_prefix.len() > current_word.len() {
                CompletionAction::CompleteCommon(common_prefix)
            } else {
                // Otherwise show all candidates
                CompletionAction::ShowCandidates(candidates)
            }
        }
    }
}

/// Find the longest common prefix among a list of strings
pub fn find_common_prefix(strings: &[String]) -> String {
    if strings.is_empty() {
        return String::new();
    }

    let first = &strings[0];
    let mut common_len = first.len();

    for string in &strings[1..] {
        let mut current_len = 0;
        for (c1, c2) in first.chars().zip(string.chars()) {
            if c1 == c2 {
                current_len += c1.len_utf8();
            } else {
                break;
            }
        }
        common_len = common_len.min(current_len);
    }

    first[..common_len].to_string()
}

/// Get keyword completions with fuzzy matching using Levenshtein distance
pub fn get_keyword_completions_fuzzy(prefix: &str) -> Vec<&str> {
    if prefix.is_empty() {
        return get_keyword_completions(prefix);
    }

    // First try exact prefix match
    let exact_matches = get_keyword_completions(prefix);
    if !exact_matches.is_empty() {
        return exact_matches;
    }

    // If no exact matches, try fuzzy matching
    // Collect potential fuzzy matches with their distances
    let mut candidates_with_distance: Vec<(&str, usize)> = Vec::new();

    for keyword in ERG_KEYWORDS {
        // Calculate the maximum allowed distance based on the length
        let max_distance = if prefix.len() <= 3 {
            1 // For short inputs, allow only 1 character difference
        } else {
            (prefix.len() as f64).sqrt().round() as usize
        };

        if let Some(distance) = crate::levenshtein::levenshtein(keyword, prefix, max_distance) {
            candidates_with_distance.push((keyword, distance));
        }
    }

    // Sort by distance (ascending) and then by keyword (for stability)
    candidates_with_distance.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    // Return only the keywords, not the distances
    candidates_with_distance
        .into_iter()
        .map(|(keyword, _)| keyword)
        .collect()
}

/// Get identifier completions with fuzzy matching using Levenshtein distance
pub fn get_identifier_completions_fuzzy(prefix: &str, identifiers: &[String]) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }

    // First try exact prefix match
    let exact_matches = get_identifier_completions(prefix, identifiers);
    if !exact_matches.is_empty() {
        return exact_matches;
    }

    // If no exact matches, try fuzzy matching
    let mut candidates_with_distance: Vec<(String, usize)> = Vec::new();

    for identifier in identifiers {
        // Calculate the maximum allowed distance based on the length
        let max_distance = if prefix.len() <= 3 {
            1 // For short inputs, allow only 1 character difference
        } else {
            (prefix.len() as f64).sqrt().round() as usize
        };

        if let Some(distance) = crate::levenshtein::levenshtein(identifier, prefix, max_distance) {
            candidates_with_distance.push((identifier.clone(), distance));
        }
    }

    // Sort by distance (ascending) and then by identifier (for stability)
    candidates_with_distance.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    // Return only the identifiers, not the distances
    candidates_with_distance
        .into_iter()
        .map(|(identifier, _)| identifier)
        .collect()
}

/// CompletionContext holds information about the current completion state
#[derive(Debug, Clone)]
pub struct CompletionContext {
    pub current_line: String,
    pub cursor_pos: usize,
    pub available_identifiers: Vec<String>,
}

impl CompletionContext {
    pub fn new(line: String, cursor_pos: usize) -> Self {
        Self {
            current_line: line,
            cursor_pos,
            available_identifiers: Vec::new(),
        }
    }

    pub fn with_identifiers(mut self, identifiers: Vec<String>) -> Self {
        self.available_identifiers = identifiers;
        self
    }

    /// Get completion candidates for the current context
    pub fn get_completion_candidates(&self) -> Vec<String> {
        let (word, _) = get_word_at_cursor(&self.current_line, self.cursor_pos);

        let mut candidates = Vec::new();

        // Add keyword completions (with fuzzy matching)
        let keyword_completions = get_keyword_completions_fuzzy(word);
        candidates.extend(keyword_completions.into_iter().map(String::from));

        // Add identifier completions (with fuzzy matching)
        let identifier_completions =
            get_identifier_completions_fuzzy(word, &self.available_identifiers);
        candidates.extend(identifier_completions);

        // Remove duplicates and sort
        candidates.sort();
        candidates.dedup();

        candidates
    }

    /// Get completion action for the current context
    pub fn get_completion_action(&self) -> CompletionAction {
        let candidates = self.get_completion_candidates();
        process_completion_candidates(candidates, &self.current_line, self.cursor_pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_extraction() {
        let test_cases = vec![
            ("", 0, "", 0),
            ("pr", 2, "pr", 0),
            ("print! ", 7, "", 7),
            ("let x = p", 9, "p", 8),
            ("if x.l", 6, "l", 5),
            ("class C.", 8, "", 8),
        ];

        for (line, cursor_pos, expected_word, expected_start) in test_cases {
            let (word, start) = get_word_at_cursor(line, cursor_pos);
            assert_eq!(
                (word, start),
                (expected_word, expected_start),
                "Failed for line: '{}', cursor: {}",
                line,
                cursor_pos
            );
        }
    }

    #[test]
    fn test_keyword_completion() {
        let completions = get_keyword_completions("pr");
        assert!(completions.contains(&"print!"));
        assert!(completions.contains(&"proc"));

        let completions = get_keyword_completions("cl");
        assert_eq!(completions, vec!["class"]);

        let completions = get_keyword_completions("xyz");
        assert!(completions.is_empty());
    }

    #[test]
    fn test_completion_context() {
        let mut context = CompletionContext::new("pr".to_string(), 2);
        context = context.with_identifiers(vec!["process".to_string(), "pretty".to_string()]);

        let candidates = context.get_completion_candidates();
        assert!(candidates.contains(&"print!".to_string()));
        assert!(candidates.contains(&"process".to_string()));

        let action = context.get_completion_action();
        match action {
            CompletionAction::ShowCandidates(candidates) => {
                assert!(!candidates.is_empty());
            }
            _ => panic!("Expected ShowCandidates action"),
        }
    }
}
