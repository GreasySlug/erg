//! `Lexer::keep_comments` emits comments as tokens instead of discarding them.
//!
//! Two things matter here. Comments must be *available* to tools that
//! reproduce the source (`erg fmt`), and they must be *invisible* to everyone
//! else — both when the flag is off, and, for the code around them, when it is
//! on. Erg decides operator fixity from surrounding whitespace, so a lexer that
//! let a comment perturb that would silently change what the code means.

use erg_parser::lex::Lexer;
use erg_parser::token::{Token, TokenKind};

fn lex(src: &str) -> Vec<Token> {
    Lexer::from_str(src.to_string())
        .lex()
        .unwrap_or_else(|(_, errs)| panic!("lexing failed:\n{errs}"))
        .into_iter()
        .collect()
}

fn lex_keeping_comments(src: &str) -> Vec<Token> {
    Lexer::from_str(src.to_string())
        .keep_comments()
        .lex()
        .unwrap_or_else(|(_, errs)| panic!("lexing failed:\n{errs}"))
        .into_iter()
        .collect()
}

fn comments(src: &str) -> Vec<String> {
    lex_keeping_comments(src)
        .into_iter()
        .filter(|tok| tok.kind == TokenKind::Comment)
        .map(|tok| tok.content.to_string())
        .collect()
}

/// Dropping the comment tokens must leave exactly the stream the default lexer
/// produces — that is what makes the flag safe to add to a shared lexer.
fn assert_same_modulo_comments(src: &str) {
    let default = lex(src);
    let kept = lex_keeping_comments(src);
    let without = kept
        .iter()
        .filter(|tok| tok.kind != TokenKind::Comment)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        without
            .iter()
            .map(|t| (t.kind, t.content.to_string()))
            .collect::<Vec<_>>(),
        default
            .iter()
            .map(|t| (t.kind, t.content.to_string()))
            .collect::<Vec<_>>(),
        "keeping comments changed the surrounding tokens for {src:?}",
    );
}

#[test]
fn comments_are_dropped_by_default() {
    let toks = lex("# leading\nx = 1 # trailing\n");
    assert!(
        toks.iter().all(|tok| tok.kind != TokenKind::Comment),
        "the default lexer must not emit comments"
    );
}

#[test]
fn line_comments_are_emitted_verbatim() {
    assert_eq!(
        comments("# leading\nx = 1 # trailing\n"),
        vec!["# leading", "# trailing"]
    );
}

#[test]
fn multi_line_comments_are_emitted_verbatim() {
    assert_eq!(
        comments("#[ a\nb ]#\nx = 1\n"),
        vec!["#[ a\nb ]#"],
        "the token spans from `#[` to `]#`, newlines included"
    );
}

#[test]
fn nested_multi_line_comments_are_one_token() {
    assert_eq!(
        comments("#[ a #[ b ]# c ]#\nx = 1\n"),
        vec!["#[ a #[ b ]# c ]#"]
    );
}

#[test]
fn comment_lineno_points_at_its_first_line() {
    let toks = lex_keeping_comments("x = 1\n#[ a\nb ]#\ny = 2\n");
    let comment = toks
        .iter()
        .find(|tok| tok.kind == TokenKind::Comment)
        .unwrap();
    assert_eq!(comment.lineno, 2, "a multi-line comment starts on line 2");
}

/// R1, the reason `emit_comment` restores `prev_token`. `op_fix_of` reads the
/// previous token's category to choose between prefix and infix, so a comment
/// standing between an operand and an operator must not be recorded as the
/// previous token — `PrePlus` here would mean `x` applied to `+1`.
#[test]
fn a_comment_does_not_change_operator_fixity() {
    let kinds = |src: &str| {
        lex_keeping_comments(src)
            .into_iter()
            .filter(|tok| matches!(tok.kind, TokenKind::Plus | TokenKind::PrePlus))
            .map(|tok| tok.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds("_ = 1 + 2\n"), vec![TokenKind::Plus]);
    assert_eq!(kinds("_ = 1 # c\n_ = 1 + 2\n"), vec![TokenKind::Plus]);
    // the prefix reading has to survive too
    assert_eq!(kinds("f = (x) -> x\n_ = f +1\n"), vec![TokenKind::PrePlus]);
    assert_eq!(
        kinds("f = (x) -> x\n# c\n_ = f +1\n"),
        vec![TokenKind::PrePlus]
    );
}

#[test]
fn keeping_comments_does_not_disturb_the_other_tokens() {
    for src in [
        "# leading\nx = 1\n",
        "x = 1 # trailing\n",
        "#[ a\nb ]#\nx = 1\n",
        "x = 1\n# between\ny = 2\n",
        "f = (x) ->\n    # inside a block\n    x + 1\n",
        "_ = [1, 2] # after a list\n",
        "_ = \"# not a comment\"\n",
        "x = 1\n\n\n# after blank lines\ny = 2\n",
    ] {
        assert_same_modulo_comments(src);
    }
}

#[test]
fn a_hash_inside_a_string_is_not_a_comment() {
    assert!(comments("_ = \"a # b\"\n").is_empty());
}

/// What Phase 2 of `erg fmt` actually needs: with comments kept, concatenating
/// `raw_text` reproduces the whole source once whitespace is disregarded.
#[test]
fn raw_text_round_trips_a_source_with_comments() {
    let src = r#"# a leading comment
x = 1 # a trailing one
#[ a multi
   line one ]#
_ = "a\nb"  # and one after a string
"#;
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let joined = lex_keeping_comments(src)
        .iter()
        .filter(|tok| tok.kind != TokenKind::EOF) // synthetic, carries "\0"
        .map(Token::raw_text)
        .collect::<Vec<_>>()
        .concat();
    assert_eq!(squeeze(&joined), squeeze(src));
}
