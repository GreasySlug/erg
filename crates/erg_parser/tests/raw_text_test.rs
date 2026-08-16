//! `Token::raw_text` must reproduce the source text of string literals.
//!
//! The lexer decodes escape sequences into `Token::content` (`\n` becomes a
//! real newline, `\t` becomes four spaces, `\x41` becomes `A`), which makes
//! `content` unusable for anything that has to write the source back out.
//! `raw` carries the original text for exactly those tokens; every other kind
//! leaves it `None` and falls back to `content`.
//!
//! Erg sources are written as Rust raw strings here so that a `\n` in the test
//! is a backslash followed by `n`, as it would be in an `.er` file.

use erg_parser::lex::Lexer;
use erg_parser::token::{Token, TokenKind};

fn lex(src: &str) -> Vec<Token> {
    Lexer::from_str(src.to_string())
        .lex()
        .unwrap_or_else(|(_, errs)| panic!("lexing failed:\n{errs}"))
        .into_iter()
        .collect()
}

/// Every string-family token, as `(content, raw_text)`.
fn strings(src: &str) -> Vec<(String, String)> {
    lex(src)
        .into_iter()
        .filter(|tok| {
            matches!(
                tok.kind,
                TokenKind::StrLit
                    | TokenKind::StrInterpLeft
                    | TokenKind::StrInterpMid
                    | TokenKind::StrInterpRight
                    | TokenKind::DocComment
            )
        })
        .map(|tok| (tok.content.to_string(), tok.raw_text().to_string()))
        .collect()
}

#[test]
fn newline_escape_is_kept_verbatim() {
    let (content, raw) = strings(r#"_ = "a\nb""#).pop().unwrap();
    assert_eq!(content, "\"a\nb\"", "content holds a real newline");
    assert_eq!(raw, r#""a\nb""#, "raw holds the backslash and the n");
}

#[test]
fn hex_escape_is_kept_verbatim() {
    let (content, raw) = strings(r#"_ = "\x41""#).pop().unwrap();
    assert_eq!(content, "\"A\"");
    assert_eq!(raw, r#""\x41""#);
}

/// The lossy one: `\t` is decoded to four spaces, so without `raw` there is no
/// way to tell it apart from four spaces that were actually typed.
#[test]
fn tab_escape_is_kept_verbatim() {
    let (content, raw) = strings(r#"_ = "a\tb""#).pop().unwrap();
    assert_eq!(content, "\"a    b\"");
    assert_eq!(raw, r#""a\tb""#);
}

#[test]
fn escape_free_string_stores_no_raw() {
    let tok = lex(r#"_ = "plain""#)
        .into_iter()
        .find(|tok| tok.kind == TokenKind::StrLit)
        .unwrap();
    assert_eq!(
        tok.raw, None,
        "a string that needs no decoding must not allocate a raw"
    );
    assert_eq!(tok.raw_text(), r#""plain""#);
}

#[test]
fn non_string_tokens_store_no_raw() {
    for tok in lex("x = 1 + 2\n") {
        assert_eq!(tok.raw, None, "{tok:?} should not carry a raw");
    }
}

#[test]
fn multi_line_string_is_kept_verbatim() {
    let (content, raw) = strings("_ = \"\"\"a\\nb\"\"\"").pop().unwrap();
    assert_eq!(content, "\"\"\"a\nb\"\"\"");
    assert_eq!(raw, "\"\"\"a\\nb\"\"\"");
}

/// A string that really does span lines keeps its line breaks in both.
#[test]
fn multi_line_string_keeps_real_line_breaks() {
    let (content, raw) = strings("_ = \"\"\"a\nb\"\"\"").pop().unwrap();
    assert_eq!(content, "\"\"\"a\nb\"\"\"");
    assert_eq!(raw, "\"\"\"a\nb\"\"\"");
}

/// Interpolation splits one literal across several tokens; each segment has to
/// carry its own slice of the source.
#[test]
fn interpolation_segments_are_kept_verbatim() {
    let segs = strings(
        r#"x = 1
_ = "a\nb\{x}c\td"
"#,
    );
    assert_eq!(
        segs,
        vec![
            ("\"a\nb\\{".to_string(), r#""a\nb\{"#.to_string()),
            ("}c    d\"".to_string(), r#"}c\td""#.to_string()),
        ]
    );
}

/// A multi-line literal's line number is back-computed from its height, which
/// has to be measured on the source. Decoding `\n` into a real newline makes
/// the content taller than the text, which used to report the token a line too
/// early -- and on line 1 that produced 0, the value `Locational` treats as
/// `Location::Unknown`.
#[test]
fn multi_line_string_lineno_counts_source_lines() {
    let lineno_of = |src: &str, kind: TokenKind| {
        lex(src)
            .into_iter()
            .find(|tok| tok.kind == kind)
            .unwrap_or_else(|| panic!("no {kind:?} in {src:?}"))
            .lineno
    };

    // no escapes: content and source are the same height
    assert_eq!(
        lineno_of("_ = \"\"\"ab\"\"\"\n_ = 1\n", TokenKind::StrLit),
        1
    );
    // `\n` escape: one source line, two content lines
    assert_eq!(
        lineno_of("_ = \"\"\"a\\nb\"\"\"\n_ = 1\n", TokenKind::StrLit),
        1
    );
    // a real line break: two of each
    assert_eq!(
        lineno_of("_ = \"\"\"a\nb\"\"\"\n_ = 1\n", TokenKind::StrLit),
        1
    );
    // and one that genuinely starts further down
    assert_eq!(
        lineno_of("_ = 1\n_ = \"\"\"a\\nb\"\"\"\n", TokenKind::StrLit),
        2
    );
}

/// The property Phase 2 of `erg fmt` rests on: for a source with no comments,
/// concatenating `raw_text` over the tokens reproduces the source exactly once
/// whitespace is disregarded. `content` cannot do this.
#[test]
fn raw_text_round_trips_the_source() {
    let src = r#"f = (x) -> x + 1
_ = f(1)
_ = "a\nb"
_ = "\x41"
_ = ["a\tb", 'raw ident']
"#;
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    // EOF is synthetic and carries "\0", which is not part of the source
    let real = |tok: &Token| tok.kind != TokenKind::EOF;

    let from_raw = lex(src)
        .iter()
        .filter(|tok| real(tok))
        .map(Token::raw_text)
        .collect::<Vec<_>>()
        .concat();
    assert_eq!(squeeze(&from_raw), squeeze(src));

    let from_content = lex(src)
        .iter()
        .filter(|tok| real(tok))
        .map(|tok| tok.content.to_string())
        .collect::<Vec<_>>()
        .concat();
    assert_ne!(
        squeeze(&from_content),
        squeeze(src),
        "if content round-tripped, `raw` would not be needed"
    );
}
