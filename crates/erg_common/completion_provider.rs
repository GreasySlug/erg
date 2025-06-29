/// Trait for providing code completions in the REPL
///
/// This trait allows external completion providers (like ELS) to be injected
/// into the REPL's stdin reader without creating circular dependencies.
pub trait CompletionProvider: Send + Sync {
    /// Get completions for the given line at the specified cursor position
    ///
    /// Returns a list of completion candidates
    fn get_completions(&self, line: &str, pos: usize) -> Vec<String>;

    /// Add a line to the completion context (for maintaining REPL history)
    fn add_line(&mut self, line: &str);
}

/// Default completion provider using the basic fuzzy completion
pub struct BasicCompletionProvider;

impl CompletionProvider for BasicCompletionProvider {
    fn get_completions(&self, line: &str, pos: usize) -> Vec<String> {
        use crate::completion::{CompletionAction, CompletionContext};

        let context = CompletionContext::new(line.to_string(), pos);
        match context.get_completion_action() {
            CompletionAction::CompleteUnique(completion) => vec![completion],
            CompletionAction::CompleteCommon(common) => vec![common],
            CompletionAction::ShowCandidates(candidates) => candidates,
            CompletionAction::None => vec![],
        }
    }

    fn add_line(&mut self, _line: &str) {
        // Basic provider doesn't maintain state
    }
}
