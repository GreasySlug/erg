//! The safety net, tested on its own.
//!
//! `render` is still the identity function, so these do not exercise any
//! formatting. What they pin down is the property every later stage will lean
//! on: `compare` must accept exactly the changes a formatter is allowed to
//! make, and reject everything else.

use erg_fmt::{compare, format_str, try_format_str, FmtOptions, Unformatted};
use erg_parser::lex::Lexer;
use erg_parser::token::TokenStream;

fn lex(src: &str) -> TokenStream {
    Lexer::from_str(src.to_string())
        .keep_comments()
        .lex()
        .unwrap_or_else(|(_, errs)| panic!("lexing {src:?} failed:\n{errs}"))
}

/// Formatting `a` into `b` would be allowed.
fn assert_equivalent(a: &str, b: &str) {
    if let Err(mismatch) = compare(&lex(a), &lex(b)) {
        panic!("{a:?} and {b:?} should be equivalent, but {mismatch}");
    }
}

/// Formatting `a` into `b` would change the program.
fn assert_differs(a: &str, b: &str) {
    if compare(&lex(a), &lex(b)).is_ok() {
        panic!("{a:?} and {b:?} should differ, but compare accepted them");
    }
}

// --- changes a formatter is allowed to make -------------------------------

#[test]
fn indent_width_may_change() {
    assert_equivalent("f = (x) ->\n    x\n", "f = (x) ->\n  x\n");
    assert_equivalent("f = (x) ->\n  x\n", "f = (x) ->\n        x\n");
}

#[test]
fn blank_line_count_may_change() {
    assert_equivalent("x = 1\n\n\n\ny = 2\n", "x = 1\n\ny = 2\n");
    assert_equivalent("x = 1\ny = 2\n", "x = 1\n\ny = 2\n");
}

#[test]
fn spacing_may_change_where_it_carries_no_meaning() {
    assert_equivalent("_ = f(1, 2)\n", "_ = f(1,2)\n");
    assert_equivalent("_ = [1, 2]\n", "_ = [ 1 , 2 ]\n");
}

/// Breaking a line inside brackets is the one line change reflow may make: the
/// lexer swallows those newlines, so the stream is untouched (design §5.7.1).
#[test]
fn a_line_break_inside_brackets_is_allowed() {
    assert_equivalent("_ = f(1, 2)\n", "_ = f(\n    1,\n    2\n)\n");
    assert_equivalent("_ = [1, 2]\n", "_ = [\n    1,\n    2\n]\n");
}

// --- changes that must be rejected ----------------------------------------

/// The reason this whole mechanism exists. `x +1` applies `x` to `+1`; putting
/// a space after the `+` turns it into an addition.
#[test]
fn operator_fixity_must_not_change() {
    assert_differs("f = (x) -> x\n_ = f +1\n", "f = (x) -> x\n_ = f + 1\n");
    assert_differs("_ = 1 + 2\n", "_ = 1 +2\n");
}

/// Also from §5.7.1: an operator moved to the end of a line lexes as prefix.
#[test]
fn an_operator_at_end_of_line_must_be_rejected() {
    assert_differs("_ = (1 + 2)\n", "_ = (1 +\n    2)\n");
    // ...whereas moving it to the start of the next line is fine
    assert_equivalent("_ = (1 + 2)\n", "_ = (1\n    + 2)\n");
}

/// Adding a trailing comma adds a token, so the formatter must not do it.
#[test]
fn a_trailing_comma_must_be_rejected() {
    assert_differs("_ = f(1, 2)\n", "_ = f(1, 2,)\n");
}

#[test]
fn a_dropped_comment_must_be_rejected() {
    assert_differs("x = 1 # c\n", "x = 1\n");
    assert_differs("# c\nx = 1\n", "x = 1\n");
}

#[test]
fn a_changed_comment_must_be_rejected() {
    assert_differs("x = 1 # before\n", "x = 1 # after\n");
}

/// Indent *width* is free, but the block structure is not.
#[test]
fn a_lost_block_must_be_rejected() {
    assert_differs("f = (x) ->\n    x\n", "f = (x) -> x\n");
}

#[test]
fn a_changed_string_literal_must_be_rejected() {
    assert_differs(r#"_ = "a""#, r#"_ = "b""#);
    // the escape and its expansion are different programs to write down, even
    // though they denote the same string
    assert_differs(r#"_ = "a\tb""#, r#"_ = "a    b""#);
}

// --- format_str ------------------------------------------------------------

#[test]
fn formatting_is_a_no_op_for_now() {
    let src = "# a comment\nx = 1\n\nf = (y) ->\n    y + 1\n";
    assert_eq!(format_str(src, FmtOptions::default()), src);
}

#[test]
fn a_source_that_does_not_lex_is_returned_unchanged() {
    let src = "_ = \"unterminated\n";
    assert_eq!(format_str(src, FmtOptions::default()), src);
    assert_eq!(
        try_format_str(src, FmtOptions::default()),
        Err((src.to_string(), Unformatted::InputDoesNotLex))
    );
}

#[test]
fn default_options_are_the_documented_ones() {
    let opts = FmtOptions::default();
    assert_eq!(opts.indent, 4);
    assert_eq!(opts.max_blank_lines, 2);
    assert_eq!(opts.max_width, 100);
}

/// The mismatch has to say what moved, or a formatter bug is a needle in a
/// haystack.
#[test]
fn a_mismatch_names_the_tokens() {
    let err = compare(&lex("_ = 1 + 2\n"), &lex("_ = 1 +2\n")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Plus"), "{msg}");
    assert!(msg.contains("PrePlus"), "{msg}");
}
