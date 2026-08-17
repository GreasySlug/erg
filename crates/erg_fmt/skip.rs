//! `# fmt:` directives — the author's way of telling the formatter to keep out.
//!
//! Comment directives rather than attributes (`rustfmt::skip`) or a decorator:
//! reading a decorator would mean parsing, and the formatter has to work on
//! sources that do not parse. A comment needs nothing but the `Comment` tokens
//! the lexer already produces.
//!
//! | written | effect |
//! | --- | --- |
//! | `# fmt: off` | stop formatting from this line on |
//! | `# fmt: on` | resume |
//! | `# fmt: skip` at the end of a line | leave that one logical line alone |

use std::ops::RangeInclusive;

use erg_common::traits::DequeStream;
use erg_parser::token::{TokenKind, TokenStream};

/// Recognised spellings. Matched exactly, after trailing whitespace is dropped.
const OFF: &str = "# fmt: off";
const ON: &str = "# fmt: on";
const SKIP: &str = "# fmt: skip";

/// What a comment asks the formatter to do, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Directive {
    Off,
    On,
    Skip,
}

/// Reads a directive out of a comment's text.
///
/// Deliberately strict: `#fmt:off` and `# FMT: OFF` are ordinary comments. A
/// directive that *looks* like it is working but is not would be worse than one
/// that plainly does nothing, since the author would only find out by noticing
/// their code got reformatted.
///
/// Trailing whitespace is the one exception. It is invisible in an editor, so
/// being strict about it would produce exactly the silent surprise the rule is
/// meant to prevent.
fn directive_of(comment: &str) -> Option<Directive> {
    match comment.trim_end() {
        OFF => Some(Directive::Off),
        ON => Some(Directive::On),
        SKIP => Some(Directive::Skip),
        _ => None,
    }
}

/// Tokens that do not count as "code on this line" when deciding whether a
/// comment trails something.
fn is_layout(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent | TokenKind::EOF
    )
}

/// The directives found in one source.
#[derive(Debug, Clone, Default)]
pub struct Directives {
    /// 1-origin inclusive line ranges to leave alone, from `off`/`on` pairs.
    off: Vec<RangeInclusive<u32>>,
    /// Lines carrying a trailing `# fmt: skip`.
    skip: Vec<u32>,
}

impl Directives {
    /// Scans a token stream lexed with `keep_comments`.
    ///
    /// Without that flag there are no comment tokens and this finds nothing --
    /// which is correct, just useless.
    pub fn scan(tokens: &TokenStream) -> Self {
        let mut found = Self::default();
        let mut off_since: Option<u32> = None;
        // the last line on which a non-layout token appeared, to tell a
        // trailing comment from one that has a line to itself
        let mut code_on_line: Option<u32> = None;

        for tok in tokens.iter() {
            if tok.kind != TokenKind::Comment {
                if !is_layout(tok.kind) {
                    code_on_line = Some(tok.lineno);
                }
                continue;
            }
            match directive_of(&tok.content) {
                // `off` while already off changes nothing: the first one wins,
                // so the region still ends at the next `on`
                Some(Directive::Off) => off_since = off_since.or(Some(tok.lineno)),
                Some(Directive::On) => {
                    // the directive lines themselves are part of the region
                    if let Some(start) = off_since.take() {
                        found.off.push(start..=tok.lineno);
                    }
                    // an `on` with no `off` is a no-op rather than an error;
                    // the formatter is not a linter
                }
                // only meaningful after code -- on a line of its own there is
                // nothing for it to suppress
                Some(Directive::Skip) if code_on_line == Some(tok.lineno) => {
                    found.skip.push(tok.lineno)
                }
                Some(Directive::Skip) => {}
                None => {}
            }
        }
        // an unterminated `off` runs to the end of the file, so a single one at
        // the top means "never format this file"
        if let Some(start) = off_since {
            found.off.push(start..=u32::MAX);
        }
        found
    }

    /// Whether this source has any directive at all.
    pub fn is_empty(&self) -> bool {
        self.off.is_empty() && self.skip.is_empty()
    }

    /// Whether `lineno` falls in an `off` region.
    pub fn is_off(&self, lineno: u32) -> bool {
        self.off.iter().any(|range| range.contains(&lineno))
    }

    /// Whether a logical line spanning `lines` carries a `# fmt: skip`.
    ///
    /// Takes a range because a logical line can cover several source lines when
    /// it is broken inside brackets; the directive may sit on any of them.
    pub fn has_skip(&self, lines: RangeInclusive<u32>) -> bool {
        self.skip.iter().any(|lineno| lines.contains(lineno))
    }

    /// Whether a logical line spanning `lines` must be emitted verbatim.
    ///
    /// True if any of its lines is suppressed: a region that begins mid-way
    /// through a construct still takes the whole construct with it, which is
    /// the safe reading.
    pub fn suppresses(&self, lines: RangeInclusive<u32>) -> bool {
        self.has_skip(lines.clone()) || lines.into_iter().any(|lineno| self.is_off(lineno))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use erg_parser::lex::Lexer;

    fn scan(src: &str) -> Directives {
        let tokens = Lexer::from_str(src.to_string())
            .keep_comments()
            .lex()
            .unwrap_or_else(|(_, errs)| panic!("lexing failed:\n{errs}"));
        Directives::scan(&tokens)
    }

    #[test]
    fn a_source_without_directives_has_none() {
        let d = scan("# an ordinary comment\nx = 1 # another\n");
        assert!(d.is_empty());
        assert!(!d.is_off(1));
        assert!(!d.suppresses(1..=2));
    }

    #[test]
    fn off_and_on_bound_an_inclusive_region() {
        let d = scan("x = 1\n# fmt: off\ny = 2\n# fmt: on\nz = 3\n");
        assert!(!d.is_off(1));
        assert!(d.is_off(2), "the `off` line itself is kept verbatim");
        assert!(d.is_off(3));
        assert!(d.is_off(4), "the `on` line itself is kept verbatim");
        assert!(!d.is_off(5));
    }

    #[test]
    fn an_unterminated_off_runs_to_the_end() {
        let d = scan("x = 1\n# fmt: off\ny = 2\n");
        assert!(!d.is_off(1));
        assert!(d.is_off(2));
        assert!(d.is_off(9999));
    }

    #[test]
    fn a_leading_off_suppresses_the_whole_file() {
        let d = scan("# fmt: off\nx = 1\ny = 2\n");
        assert!(d.is_off(1));
        assert!(d.is_off(3));
    }

    #[test]
    fn several_regions_are_tracked_separately() {
        let d = scan("# fmt: off\na = 1\n# fmt: on\nb = 2\n# fmt: off\nc = 3\n# fmt: on\n");
        assert!(d.is_off(1));
        assert!(d.is_off(2));
        assert!(d.is_off(3));
        assert!(!d.is_off(4));
        assert!(d.is_off(5));
        assert!(d.is_off(7));
    }

    #[test]
    fn a_repeated_off_does_not_shorten_the_region() {
        let d = scan("# fmt: off\na = 1\n# fmt: off\nb = 2\n# fmt: on\nc = 3\n");
        assert!(d.is_off(1));
        assert!(
            d.is_off(4),
            "the second `off` must not end the first region"
        );
        assert!(!d.is_off(6));
    }

    #[test]
    fn an_unmatched_on_is_ignored() {
        let d = scan("x = 1\n# fmt: on\ny = 2\n");
        assert!(d.is_empty());
    }

    /// The `off`/`on` pair is a line range, so it need not be balanced against
    /// the block structure.
    #[test]
    fn a_region_may_cross_block_boundaries() {
        let d = scan("f = (x) ->\n    # fmt: off\n    x\n# fmt: on\ny = 2\n");
        assert!(d.is_off(2));
        assert!(d.is_off(3));
        assert!(d.is_off(4));
        assert!(!d.is_off(5));
    }

    #[test]
    fn skip_applies_to_the_line_it_trails() {
        let d = scan("x = 1 # fmt: skip\ny = 2\n");
        assert!(d.has_skip(1..=1));
        assert!(!d.has_skip(2..=2));
        assert!(d.suppresses(1..=1));
    }

    /// A logical line broken inside brackets covers several source lines, and
    /// the directive may sit on any of them.
    #[test]
    fn skip_is_found_anywhere_in_a_logical_line() {
        let d = scan("_ = f(\n    1,\n    2\n) # fmt: skip\n");
        assert!(d.has_skip(1..=4));
    }

    /// On a line of its own there is nothing for it to suppress, so it is just
    /// a comment.
    #[test]
    fn a_standalone_skip_does_nothing() {
        let d = scan("# fmt: skip\nx = 1\n");
        assert!(d.is_empty());
    }

    /// Strict on spelling, forgiving about invisible whitespace.
    #[test]
    fn recognition_is_exact() {
        assert!(scan("#fmt:off\nx = 1\n").is_empty(), "no spaces");
        assert!(scan("# FMT: OFF\nx = 1\n").is_empty(), "wrong case");
        assert!(scan("# fmt:off\nx = 1\n").is_empty(), "missing space");
        assert!(scan("#  fmt: off\nx = 1\n").is_empty(), "extra space");
        assert!(
            scan("# fmt: off please\nx = 1\n").is_empty(),
            "trailing text"
        );

        assert!(
            !scan("# fmt: off   \nx = 1\n").is_empty(),
            "trailing whitespace is invisible and must not defeat the directive"
        );
    }

    #[test]
    fn a_directive_inside_a_string_is_not_a_directive() {
        assert!(scan("_ = \"# fmt: off\"\nx = 1\n").is_empty());
    }
}
