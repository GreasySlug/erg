//! The safety net, tested on its own.
//!
//! These do not go through the renderer. What they pin down is the property
//! every rendering stage leans on: `compare` must accept exactly the changes a
//! formatter is allowed to make, and reject everything else.

use erg_fmt::{
    compare, format_str, token_streams_equivalent, try_format_str, FmtOptions, Unformatted,
};
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

/// The file's own final newline is the formatter's to add: a source that ends
/// without one is given one, and that must not read as a changed program.
#[test]
fn the_final_newline_may_be_added_or_removed() {
    assert_equivalent("x = 1", "x = 1\n");
    assert_equivalent("x = 1\n", "x = 1");
    // ...but a newline that still separates two statements is not "final"
    assert_differs("x = 1\ny = 2", "x = 1 y = 2\n");
}

#[test]
fn leading_and_trailing_blank_lines_may_be_removed() {
    assert_equivalent("\n\n\nx = 1\n", "x = 1\n");
    assert_equivalent("x = 1\n\n\n", "x = 1\n");
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

/// Blank lines are the formatter's to normalize, but a line break either
/// exists or it does not. Excluding `Newline` from the comparison outright --
/// which is what this did at first -- let this through: the remaining tokens
/// are identical, so joining two statements looked like a no-op.
#[test]
fn joining_or_splitting_statements_must_be_rejected() {
    assert_differs("x = 1\ny = 2\n", "x = 1 y = 2\n");
    assert_differs("x = 1\ny = 2\n", "x = 1; y = 2\n");
    assert_differs("_ = 1 + 2\n", "_ = 1\n+ 2\n");
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

/// Re-indenting a block must not reach inside a multi-line literal: the spaces
/// in there are part of the string. Worth pinning, because the reflow and
/// indent stages will both be tempted to touch those lines.
#[test]
fn reindenting_inside_a_multi_line_string_must_be_rejected() {
    assert_differs(
        "f = (x) ->\n    s = \"\"\"\n    a\n    \"\"\"\n",
        "f = (x) ->\n  s = \"\"\"\n  a\n  \"\"\"\n",
    );
}

/// A comment's own indentation is not part of its token, so moving it with the
/// block around it is fine.
#[test]
fn a_comment_may_be_reindented() {
    assert_equivalent("f = (x) ->\n    # c\n    x\n", "f = (x) ->\n  # c\n  x\n");
}

/// Documents the one gap: whitespace that changes meaning without changing any
/// token. `f(1)` and `f (1)` tokenize identically, so this check cannot tell
/// them apart -- the spacing rules have to preserve it instead (design §5.3).
/// If this ever starts failing, the gap has closed and §5.3 can be relaxed.
#[test]
fn spacing_before_a_bracket_is_a_known_blind_spot() {
    assert_equivalent("_ = f(1)\n", "_ = f (1)\n");
}

// --- format_str ------------------------------------------------------------

/// Already-formatted input must come back untouched -- the fixed point the
/// idempotence property is anchored on.
#[test]
fn well_formatted_input_is_left_alone() {
    let src = "# a comment\nx = 1\n\nf = (y) ->\n    y + 1\n";
    assert_eq!(format_str(src, FmtOptions::default()), src);
}

#[test]
fn a_source_that_does_not_lex_is_returned_unchanged() {
    let src = "_ = \"unterminated\n";
    assert_eq!(format_str(src, FmtOptions::default()), src);
    assert_eq!(
        try_format_str(src, FmtOptions::default()),
        Err(Unformatted::InputDoesNotLex)
    );
}

/// Degenerate inputs must degrade, not panic. Cheap to assert now, and the
/// rendering stages are exactly where an off-by-one on an empty or one-token
/// file would otherwise first show up.
#[test]
fn degenerate_sources_do_not_panic() {
    for src in [
        "",
        "\n",
        "\n\n\n",
        "   ",   // leading indent at BOF: does not lex
        "# c",   // a comment and nothing else
        "x = 1", // no trailing newline
        "x = 1 # c",
        "x = 1\r\ny = 2\r\n",
    ] {
        let out = format_str(src, FmtOptions::default());
        // whatever happens, the result must still be the same program
        if let (Ok(before), Ok(after)) = (
            Lexer::from_str(src.to_string()).keep_comments().lex(),
            Lexer::from_str(out.clone()).keep_comments().lex(),
        ) {
            assert!(
                compare(&before, &after).is_ok(),
                "formatting {src:?} changed the program"
            );
        }
    }
}

#[test]
fn default_options_are_the_documented_ones() {
    let opts = FmtOptions::default();
    assert_eq!(opts.indent, 4);
    assert_eq!(opts.max_blank_lines, 2);
    assert_eq!(opts.max_width, 100);
}

/// The mismatch has to say what moved *and where*, or a formatter bug is a
/// needle in a haystack.
#[test]
fn a_mismatch_names_the_tokens_and_the_line() {
    let err = compare(&lex("x = 1\n_ = 1 + 2\n"), &lex("x = 1\n_ = 1 +2\n")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Plus"), "{msg}");
    assert!(msg.contains("PrePlus"), "{msg}");
    assert!(msg.contains("line 2"), "should locate the token: {msg}");
}

#[test]
fn token_streams_equivalent_agrees_with_compare() {
    assert!(token_streams_equivalent(&lex("x = 1\n"), &lex("x = 1\n\n")));
    assert!(!token_streams_equivalent(
        &lex("_ = 1 + 2\n"),
        &lex("_ = 1 +2\n")
    ));
}

/// The formatter lexes with `keep_comments`; the compiler does not, and the
/// two do not accept the same sources -- a space after a `#[ ]#` is a token
/// gap to one and an illegal character to the other. Verifying only the
/// formatter's reading would let it emit files `erg` cannot compile, so the
/// output has to satisfy both.
#[test]
fn the_output_lexes_the_way_the_compiler_reads_it() {
    for src in [
        "print! #[]#0, 1#[]#, 2,#[]#3\n",
        "#[a]#x #[b]#=#[c]#1#[d]#\n",
        "#[]#print! 0\n",
        "x = 1#[]#+ 2\n",
        "f = (x) ->\n    #[c]#x\n",
    ] {
        let formatted = format_str(src, FmtOptions::default());
        assert!(
            Lexer::from_str(formatted.clone()).lex().is_ok(),
            "the compiler cannot lex what formatting {src:?} produced:\n{formatted}"
        );
    }
}

/// A source the compiler cannot read *either* way is still formatted. The
/// second check has nothing to compare against, and refusing here would take
/// format-on-save away from exactly the files that need it -- this one comes
/// back repaired, since the spacing rules no longer produce what broke it.
#[test]
fn a_source_only_the_formatter_can_lex_is_still_formatted() {
    let src = "print! #[]# 0\n";
    assert!(
        Lexer::from_str(src.to_string()).lex().is_err(),
        "the premise: the compiler rejects this source"
    );
    assert_eq!(
        try_format_str(src, FmtOptions::default()),
        Ok("print! #[]#0\n".to_string())
    );
}
