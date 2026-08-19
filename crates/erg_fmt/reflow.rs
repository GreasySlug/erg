//! Breaking lines that are too wide.
//!
//! Only inside brackets, and only at commas. Both restrictions come from the
//! lexer rather than from taste:
//!
//! - A newline inside `()`, `[]` or `{}` is swallowed, so the token stream is
//!   bit-for-bit unchanged by a break there. Anywhere else a break adds a
//!   `Newline`, which is a different program.
//! - An operator at end of line has a newline after it, and the lexer reads
//!   that as prefix: `(1 +` / `2)` is `1` applied to `+2`. Splitting after a
//!   comma is the one cut that can never leave an operator dangling, because
//!   the line always ends on the comma itself.
//!
//! No trailing comma is ever added -- that would be one token more than the
//! source had. One the author already wrote is left alone, and since breaks are
//! only ever added and never removed, it keeps its bracket expanded for free:
//! the break beside it survives whether or not the line would now fit.

use std::ops::Range;

use crate::render::{is_closing, is_layout, is_opening, Chunk, Layout};
use erg_parser::token::TokenKind;

/// How much narrower the longest line has to get before a split pays for itself
/// when the bracket was *not* where the width was: it must lose at least a
/// quarter of its width.
///
/// A split whose longest line ends up inside the bracket is never weighed
/// against this -- breaking the bracket is what shortened the line, which is
/// the whole of the question. The ratio is for the other case, where what is
/// still long is the text either side of the bracket:
///
/// ```erg
/// [major, minor, patch, pre] -> major.isnumeric() and ... and pre.isalnum()
/// ```
///
/// Exploding that pattern buys three columns; the length is in the `and` chain
/// after the arrow, which no comma can break. The line above it in that file
/// has three elements, fits, and stays whole -- so the split would also make
/// two neighbouring lines disagree about their shape for nothing.
///
/// A split costs two lines at minimum, so buying a few columns with them is a
/// bad trade. Measured over every split the formatter makes across this
/// repository, on the tree as it stood before it was formatted: the 84 worth
/// keeping all came in at 68% or below, and the three that read worse than the
/// line they replaced were at 79%, 81% and 81%. Three quarters sits in the gap.
///
const WORTH_SPLITTING: (usize, usize) = (3, 4);

/// Splits a chunk until every part fits, or until nothing more can be split.
///
/// A line with no bracket to break inside is emitted over-wide. There is no
/// safe rewrite for it, and inventing one is out of scope -- `erg fmt` does not
/// change expressions, only where they sit.
///
/// A split that barely helps is declined for the same reason: an over-wide line
/// the author wrote is easier to read than five lines that are still over-wide.
pub fn wrap(layout: &Layout, chunk: Chunk) -> Vec<Chunk> {
    let width = layout.width(&chunk);
    if width <= layout.opts.max_width {
        return vec![chunk];
    }
    let Some(parts) = split_at_commas(layout, &chunk) else {
        return vec![chunk];
    };
    // the first and last parts are the text either side of the bracket, which
    // the split does not shorten; the ones between are its contents, which it
    // does. Measured after the recursion, since an element that is still too
    // wide may have been broken up in turn.
    let last = parts.len() - 1;
    let (mut inside, mut outside) = (0, 0);
    let mut split = Vec::new();
    for (at, part) in parts.into_iter().enumerate() {
        let expanded = wrap(layout, part);
        let widest = expanded.iter().map(|p| layout.width(p)).max().unwrap_or(0);
        if at == 0 || at == last {
            outside = outside.max(widest);
        } else {
            inside = inside.max(widest);
        }
        split.extend(expanded);
    }
    if inside >= outside {
        // the bracket held the length, and breaking it is what shortened the
        // line
        return split;
    }
    // it did not: what is left over-long is text the split never touched, so
    // the break has to buy enough width to be worth the lines it costs
    let (num, den) = WORTH_SPLITTING;
    if outside * den <= width * num {
        split
    } else {
        vec![chunk]
    }
}

/// Splits at the commas directly inside the chunk's widest bracket pair.
///
/// `None` when there is nothing to gain: no bracket, no comma inside it, or a
/// token whose text is itself multi-line, whose width the split cannot change.
fn split_at_commas(layout: &Layout, chunk: &Chunk) -> Option<Vec<Chunk>> {
    if chunk.tokens.iter().any(|&i| {
        let tok = layout.tok(i);
        // Moving tokens across a comment would comment them out. A comment is
        // also a line break the author already made, so the chunk beside it is
        // short by construction and there is nothing to do here.
        tok.kind == TokenKind::Comment || tok.raw_text().contains('\n')
    }) {
        return None;
    }
    let (open, close) = widest_pair(layout, chunk)?;
    let inner = &chunk.tokens[open + 1..close];
    let cuts = element_ends(layout, inner);
    if cuts.is_empty() {
        return None;
    }

    let mut elements = Vec::new();
    let mut from = 0;
    for cut in cuts.into_iter().chain([inner.len()]) {
        if from < cut {
            elements.push(from..cut);
        }
        from = cut;
    }
    let level = chunk.level + 1;
    let rows = if is_parameter_list(layout, chunk, close) {
        pack(layout, level, inner, &elements)
    } else {
        elements
    };

    let mut parts = vec![Chunk {
        level: chunk.level,
        tokens: chunk.tokens[..=open].to_vec(),
        continued: false,
    }];
    for row in rows {
        parts.push(Chunk {
            level,
            tokens: inner[row].to_vec(),
            continued: false,
        });
    }
    // every cut here is inside a bracket, where a break needs no `\`; only the
    // tail still ends where the chunk did, so only it keeps the continuation
    parts.push(Chunk {
        level: chunk.level,
        tokens: chunk.tokens[close..].to_vec(),
        continued: chunk.continued,
    });
    Some(parts)
}

/// Whether the pair closing at `close` is a function type's parameter list --
/// `(a, b) -> T` -- rather than a call's argument list.
///
/// Both are commas inside brackets, and until the closing bracket there is
/// nothing to tell them apart, so the arrow after it is the whole test.
fn is_parameter_list(layout: &Layout, chunk: &Chunk, close: usize) -> bool {
    chunk.tokens.get(close + 1).is_some_and(|&index| {
        matches!(
            layout.tok(index).kind,
            TokenKind::FuncArrow | TokenKind::ProcArrow
        )
    })
}

/// Groups adjacent elements into as few lines as the width allows.
///
/// A parameter list is read as a set: you look for one name in it, and the
/// signature it belongs to is the thing you are actually reading. Giving each
/// parameter its own line turns a declaration into a paragraph -- fine in a
/// definition with a body under it, but a declaration file is a column of them,
/// and nine lines per member is a column you can no longer scan.
///
/// An element too wide to share a line is left on its own, where [`wrap`] gets
/// another chance to break it.
fn pack(
    layout: &Layout,
    level: usize,
    inner: &[usize],
    elements: &[Range<usize>],
) -> Vec<Range<usize>> {
    let mut rows: Vec<Range<usize>> = Vec::new();
    for element in elements {
        if let Some(last) = rows.last_mut() {
            let merged = last.start..element.end;
            let candidate = Chunk {
                level,
                tokens: inner[merged.clone()].to_vec(),
                continued: false,
            };
            if layout.width(&candidate) <= layout.opts.max_width {
                *last = merged;
                continue;
            }
        }
        rows.push(element.clone());
    }
    rows
}

/// The outermost bracket pair spanning the most tokens, as positions *within*
/// `chunk.tokens`.
///
/// Outermost so the split has the whole expression to work with; widest so that
/// `f(a) + g(b, c, d)` breaks inside the call that has something to break.
fn widest_pair(layout: &Layout, chunk: &Chunk) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut opened_at = None;
    let mut widest: Option<(usize, usize)> = None;
    for (at, &index) in chunk.tokens.iter().enumerate() {
        let kind = layout.tok(index).kind;
        if is_opening(kind) {
            if depth == 0 {
                opened_at = Some(at);
            }
            depth += 1;
        } else if is_closing(kind) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                if let Some(open) = opened_at.take() {
                    let span = at - open;
                    if widest.is_none_or(|(o, c)| c - o < span) {
                        widest = Some((open, at));
                    }
                }
            }
        }
    }
    widest
}

/// Where each element of a bracket's contents ends, i.e. just past each comma
/// at the bracket's own nesting level.
///
/// The comma stays with the element it follows, which is what keeps a broken
/// line from ending on an operator.
fn element_ends(layout: &Layout, inner: &[usize]) -> Vec<usize> {
    let mut depth = 0usize;
    let mut ends = Vec::new();
    for (at, &index) in inner.iter().enumerate() {
        let kind = layout.tok(index).kind;
        if is_opening(kind) {
            depth += 1;
        } else if is_closing(kind) {
            depth = depth.saturating_sub(1);
        } else if kind == TokenKind::Comma && depth == 0 {
            ends.push(at + 1);
        } else if is_layout(kind) {
            debug_assert!(false, "layout tokens do not reach a chunk");
        }
    }
    // a comma in last position ends nothing; the tail would be empty
    if ends.last() == Some(&inner.len()) {
        ends.pop();
    }
    ends
}

#[cfg(test)]
mod tests {
    use crate::{format_str, FmtOptions};

    fn fmt_at(width: usize, src: &str) -> String {
        format_str(
            src,
            FmtOptions {
                max_width: width,
                ..FmtOptions::default()
            },
        )
    }

    #[test]
    fn a_line_within_the_width_is_left_alone() {
        assert_eq!(fmt_at(40, "_ = f(1, 2)\n"), "_ = f(1, 2)\n");
    }

    #[test]
    fn a_wide_call_breaks_at_its_commas() {
        assert_eq!(
            fmt_at(20, "result = compute(alpha, beta, gamma)\n"),
            "result = compute(\n    alpha,\n    beta,\n    gamma\n)\n"
        );
    }

    /// The last element must not gain a comma: that is one token more than the
    /// source had, and the verification would throw the whole file away.
    #[test]
    fn no_trailing_comma_is_added() {
        let out = fmt_at(10, "_ = f(aaaa, bbbb)\n");
        assert_eq!(out, "_ = f(\n    aaaa,\n    bbbb\n)\n");
    }

    /// ...and one the author wrote is kept, exactly as written.
    #[test]
    fn a_trailing_comma_is_kept() {
        assert_eq!(
            fmt_at(10, "_ = f(aaaa, bbbb,)\n"),
            "_ = f(\n    aaaa,\n    bbbb,\n)\n"
        );
    }

    #[test]
    fn a_still_wide_element_breaks_again() {
        assert_eq!(
            fmt_at(16, "_ = f(a, g(bbbb, cccc), d)\n"),
            "_ = f(\n    a,\n    g(\n        bbbb,\n        cccc\n    ),\n    d\n)\n"
        );
    }

    /// The widest top-level pair is the one worth breaking.
    #[test]
    fn the_widest_bracket_pair_is_chosen() {
        assert_eq!(
            fmt_at(20, "_ = f(x) + g(aaa, bbb, ccc)\n"),
            "_ = f(x) + g(\n    aaa,\n    bbb,\n    ccc\n)\n"
        );
    }

    /// Nothing to break inside: the line goes out over-wide rather than being
    /// rewritten. `erg lint` is the right place to complain about it.
    #[test]
    fn a_line_with_no_bracket_is_left_long() {
        let src = "_ = aaaa + bbbb + cccc + dddd\n";
        assert_eq!(fmt_at(10, src), src);
    }

    #[test]
    fn a_bracket_without_commas_is_left_alone() {
        let src = "_ = f(aaaaaaaaaaaaaaaaaaaa)\n";
        assert_eq!(fmt_at(10, src), src);
    }

    /// Moving tokens past a comment would comment them out.
    #[test]
    fn a_chunk_holding_a_comment_is_never_split() {
        let src = "_ = f(aaaa, bbbb) # a comment that makes this long\n";
        assert_eq!(fmt_at(20, src), src);
    }

    /// A chunk that ends in a `\` continuation may still be too wide. The
    /// backslash belongs to the last part, which is the one that still ends
    /// where the chunk did; the cuts before it are inside a bracket and need
    /// none.
    #[test]
    fn a_split_continuation_keeps_its_backslash() {
        assert_eq!(
            fmt_at(20, "_ = f(alpha, beta) and \\\n    gamma\n"),
            "_ = f(\n    alpha,\n    beta\n) and \\\n    gamma\n"
        );
    }

    /// The length here is *after* the bracket, so breaking the bracket only
    /// moves it: four lines, and the longest still 81% of what it replaced.
    /// An over-wide line the author wrote reads better than that.
    #[test]
    fn a_split_that_barely_helps_is_declined() {
        let src = "_ = f(a, b) + cccccccccccccccccccccccccccccccccccccccc\n";
        assert_eq!(fmt_at(20, src), src);
    }

    /// ...but the same shape is split when the bracket really is where the
    /// width is.
    #[test]
    fn a_split_that_pays_for_itself_is_kept() {
        assert_eq!(
            fmt_at(20, "_ = f(aaaaaaaa, bbbbbbbb) + c\n"),
            "_ = f(\n    aaaaaaaa,\n    bbbbbbbb\n) + c\n"
        );
    }

    /// A parameter list is read as a set -- you look for one name in it -- so
    /// it is packed rather than stacked. A declaration file is a column of
    /// these, and one member per nine lines is a column you cannot scan.
    #[test]
    fn a_parameter_list_is_packed_not_stacked() {
        assert_eq!(
            fmt_at(30, "f: (aaa: Int, bbb: Int, ccc: Int, ddd: Int) -> Int\n"),
            "f: (\n    aaa: Int, bbb: Int,\n    ccc: Int, ddd: Int\n) -> Int\n"
        );
    }

    #[test]
    fn a_procedure_type_is_packed_too() {
        assert_eq!(
            fmt_at(30, "g: (aaa: Int, bbb: Int, ccc: Int, ddd: Int) => Int\n"),
            "g: (\n    aaa: Int, bbb: Int,\n    ccc: Int, ddd: Int\n) => Int\n"
        );
    }

    /// The arrow is the whole test, so a lambda's parameters pack as well.
    #[test]
    fn a_lambdas_parameters_are_packed() {
        assert_eq!(
            fmt_at(20, "f = (aaa, bbb, ccc, ddd) -> aaa\n"),
            "f = (\n    aaa, bbb, ccc,\n    ddd\n) -> aaa\n"
        );
    }

    /// Arguments are not a set to scan but a sequence to read one at a time,
    /// and they stay one per line.
    #[test]
    fn an_argument_list_is_still_stacked() {
        assert_eq!(
            fmt_at(20, "_ = h(aaa, bbb, ccc, ddd, eee)\n"),
            "_ = h(\n    aaa,\n    bbb,\n    ccc,\n    ddd,\n    eee\n)\n"
        );
    }

    #[test]
    fn a_parameter_too_wide_to_share_gets_its_own_line() {
        assert_eq!(
            fmt_at(
                30,
                "f: (aaaaaaaaaaaaaaaaaaaaaaaa: Int, b: Int, c: Int) -> Int\n"
            ),
            "f: (\n    aaaaaaaaaaaaaaaaaaaaaaaa: Int,\n    b: Int, c: Int\n) -> Int\n"
        );
    }

    /// Packing leaves the longest line just under the limit, so measuring it
    /// against a line only slightly over would decline every marginal split:
    /// here 28 of 35 columns is 80%, past the 3/4 the ratio asks for. A split
    /// that reaches the width is not weighed against it at all.
    #[test]
    fn a_split_that_reaches_the_width_is_not_weighed() {
        assert_eq!(
            fmt_at(30, "f: (aaaaaa: Int, bbbbbb: Int) -> Int\n"),
            "f: (\n    aaaaaa: Int, bbbbbb: Int\n) -> Int\n"
        );
    }

    #[test]
    fn breaking_is_idempotent() {
        for src in [
            "result = compute(alpha, beta, gamma)\n",
            "_ = f(a, g(bbbb, cccc), d)\n",
            "f = (x) ->\n    result = compute(alpha, beta, gamma)\n",
            "_ = f(alpha, beta) and \\\n    gamma\n",
            "_ = f(a, b) + cccccccccccccccccccccccccccccccccccccccc\n",
            "f: (aaa: Int, bbb: Int, ccc: Int, ddd: Int) -> Int\n",
            "f = (aaa, bbb, ccc, ddd) -> aaa\n",
        ] {
            let once = fmt_at(20, src);
            assert_eq!(fmt_at(20, &once), once, "not idempotent for {src:?}");
        }
    }
}
