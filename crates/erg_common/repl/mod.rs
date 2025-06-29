//! REPL-specific functionality
//!
//! This module contains REPL-specific features including:
//! - Code completion
//! - Syntax highlighting
//! - Variable tracking
//! - Context management

pub mod completion;
pub mod highlighter;

// Re-export commonly used items
pub use completion::ReplCompletionContext;
pub use highlighter::ReplHighlighter;