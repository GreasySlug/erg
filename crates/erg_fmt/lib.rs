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
//! Normalizes the layout around a line -- indentation, blank lines, trailing
//! whitespace, the final newline -- and copies the line's own text from the
//! source. Spacing within a line, and wrapping, are still to come.

pub mod line;
pub mod render;
pub mod skip;
pub mod source;
pub mod verify;

use erg_parser::lex::Lexer;
use erg_parser::token::TokenStream;

pub use line::Item;
pub use skip::Directives;
pub use source::SourceLines;
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
        Err(why) => {
            // A source that does not lex is expected and handled. The other two
            // mean the formatter is broken, and the fallback would otherwise
            // hide that -- so make it loud wherever assertions are on.
            debug_assert!(
                matches!(why, Unformatted::InputDoesNotLex),
                "erg fmt refused to format: {why}"
            );
            src.to_string()
        }
    }
}

/// [`format_str`], reporting why the source was left alone.
///
/// Use this when the reason matters, e.g. for `--check` to warn about a file it
/// could not format. On failure the caller keeps its own `src`.
pub fn try_format_str(src: &str, opts: FmtOptions) -> Result<String, Unformatted> {
    let before = lex(src).ok_or(Unformatted::InputDoesNotLex)?;
    let formatted = render::render(src, &before, opts);
    let after = lex(&formatted).ok_or(Unformatted::OutputDoesNotLex)?;
    match compare(&before, &after) {
        Ok(()) => Ok(formatted),
        Err(mismatch) => Err(Unformatted::TokensChanged(mismatch.to_string())),
    }
}
