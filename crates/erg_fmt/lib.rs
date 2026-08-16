//! The Erg code formatter.
//!
//! Works on the token stream rather than the AST: the lexer already emits
//! `Newline`/`Indent`/`Dedent` as tokens, blank lines survive as runs of
//! `Newline`, and with [`Lexer::keep_comments`] comments come through too, so
//! everything needed to rewrite the source is present without a CST.
//!
//! Two properties shape the design, both of them consequences of Erg being
//! whitespace-sensitive:
//!
//! - Formatting is **verified**. Erg decides operator fixity from surrounding
//!   whitespace, so a spacing rule that is subtly wrong would change what a
//!   program does. Every result is lexed again and compared with the input
//!   ([`verify`]); if they differ, the original source is returned unchanged.
//!   A bug in the formatter can therefore cost you formatting, never code.
//! - It does not depend on `erg_compiler`. The formatter has to work on
//!   sources that do not type-check -- otherwise editor format-on-save would
//!   stop working exactly when the file is half-written.
//!
//! See `docs/erg-fmt-design.md` for the full design.
//!
//! # Status
//!
//! Skeleton. [`format_str`] currently returns its input unchanged; the
//! verification path around it is complete, so the rendering stages can be
//! filled in one at a time under a working safety net.

pub mod verify;

use erg_parser::lex::Lexer;
use erg_parser::token::TokenStream;

pub use verify::{compare, token_streams_equivalent, Mismatch};

/// Everything `erg fmt` lets you configure.
///
/// Three knobs, no config file, by deliberate choice -- see
/// `docs/erg-fmt-design.md` §12.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FmtOptions {
    /// Spaces per nesting level.
    pub indent: usize,
    /// How many consecutive blank lines to allow.
    pub max_blank_lines: usize,
    /// Column beyond which a line is wrapped, when it can be wrapped at all.
    pub max_width: usize,
}

impl Default for FmtOptions {
    fn default() -> Self {
        Self {
            indent: 4,
            max_blank_lines: 2,
            max_width: 100,
        }
    }
}

/// Why a source was returned unformatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unformatted {
    /// The input does not lex. Nothing can be said about it, so it is left be.
    InputDoesNotLex,
    /// The output does not lex. Always a bug in the formatter.
    OutputDoesNotLex,
    /// The output lexes but is not the same program. Always a bug.
    TokensChanged(String),
}

impl std::fmt::Display for Unformatted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InputDoesNotLex => write!(f, "the source could not be tokenized"),
            Self::OutputDoesNotLex => write!(f, "the formatted output could not be tokenized"),
            Self::TokensChanged(at) => write!(f, "formatting would change the program: {at}"),
        }
    }
}

fn lex(src: &str) -> Option<TokenStream> {
    Lexer::from_str(src.to_string()).keep_comments().lex().ok()
}

/// Formats `src`, or returns it unchanged if that cannot be done safely.
///
/// Never fails and never panics in release builds: anything that would alter
/// the program falls back to the input. Use [`try_format_str`] when the reason
/// matters, e.g. to report it under `--check`.
pub fn format_str(src: &str, opts: FmtOptions) -> String {
    match try_format_str(src, opts) {
        Ok(formatted) => formatted,
        Err((src, why)) => {
            // A formatter that changes the program is a bug, and one that would
            // otherwise pass unnoticed because the fallback hides it. Surface it
            // wherever assertions are on.
            debug_assert!(
                matches!(why, Unformatted::InputDoesNotLex),
                "erg fmt refused to format: {why}"
            );
            src
        }
    }
}

/// [`format_str`], reporting why the source was left alone.
///
/// On failure returns the original source alongside the reason, so a caller
/// that wants to warn does not have to re-read the file.
pub fn try_format_str(src: &str, opts: FmtOptions) -> Result<String, (String, Unformatted)> {
    let Some(before) = lex(src) else {
        return Err((src.to_string(), Unformatted::InputDoesNotLex));
    };
    let formatted = render(src, &before, opts);
    let Some(after) = lex(&formatted) else {
        return Err((src.to_string(), Unformatted::OutputDoesNotLex));
    };
    match compare(&before, &after) {
        Ok(()) => Ok(formatted),
        Err(mismatch) => Err((
            src.to_string(),
            Unformatted::TokensChanged(mismatch.to_string()),
        )),
    }
}

/// Builds the formatted text from the token stream.
///
/// Not implemented yet -- returns the source unchanged, which is trivially a
/// valid formatting. The stages named in the design doc (§5.2) get added here.
fn render(src: &str, _tokens: &TokenStream, _opts: FmtOptions) -> String {
    src.to_string()
}
