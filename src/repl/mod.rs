/// REPL-specific functionality including ELS integration
///
/// This module provides enhanced REPL features such as:
/// - ELS-based code completion
/// - Type-aware suggestions
/// - Auto-import functionality

#[cfg(feature = "els")]
pub mod els_integration;

#[cfg(feature = "els")]
pub mod els_completion_provider;

#[cfg(feature = "els")]
pub use els_integration::ReplElsIntegration;

#[cfg(feature = "els")]
pub use els_completion_provider::ElsCompletionProvider;
