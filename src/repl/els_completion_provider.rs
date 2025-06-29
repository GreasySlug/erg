/// ELS-based completion provider for REPL
use std::sync::{Arc, Mutex};

use crate::ReplElsIntegration;
use erg_common::completion_provider::CompletionProvider;

/// Wrapper to make ReplElsIntegration work as a CompletionProvider
pub struct ElsCompletionProvider {
    integration: Arc<Mutex<ReplElsIntegration>>,
}

impl ElsCompletionProvider {
    /// Create a new ELS completion provider
    pub fn new(integration: Arc<Mutex<ReplElsIntegration>>) -> Self {
        Self { integration }
    }
}

impl CompletionProvider for ElsCompletionProvider {
    fn get_completions(&self, line: &str, pos: usize) -> Vec<String> {
        let mut integration = self.integration.lock().unwrap();
        let completions = integration.get_els_completions(line, pos);

        // Convert LSP CompletionItems to simple strings
        completions.into_iter().map(|item| item.label).collect()
    }

    fn add_line(&mut self, line: &str) {
        let mut integration = self.integration.lock().unwrap();
        integration.add_repl_line(line);
    }
}
