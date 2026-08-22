use erg_common::error::Location;
use erg_common::traits::{Locational, Stream, Traversable};
use erg_compiler::artifact::BuildRunnable;
use erg_compiler::erg_parser::ast::{Expr, Module};
use erg_compiler::erg_parser::parse::Parsable;

use lsp_types::{FoldingRange, FoldingRangeKind, FoldingRangeParams};

use crate::_log;
use crate::server::{ELSResult, RedirectableStdout, Server};
use crate::util::NormalizedUrl;

fn imports_range(start: &Location, end: &Location) -> Option<FoldingRange> {
    Some(FoldingRange {
        start_line: start.ln_begin()?.saturating_sub(1),
        start_character: start.col_begin(),
        end_line: end.ln_end()?.saturating_sub(1),
        end_character: end.col_end(),
        kind: Some(FoldingRangeKind::Imports),
    })
}

fn region(loc: Location) -> Option<FoldingRange> {
    let start_line = loc.ln_begin()?.saturating_sub(1);
    let end_line = loc.ln_end()?.saturating_sub(1);
    if end_line <= start_line {
        return None;
    }
    Some(FoldingRange {
        start_line,
        start_character: loc.col_begin(),
        end_line,
        end_character: loc.col_end(),
        kind: Some(FoldingRangeKind::Region),
    })
}

fn is_foldable(expr: &Expr) -> bool {
    match expr {
        Expr::Def(def) if def.def_kind().is_import() => false,
        Expr::Def(_)
        | Expr::Methods(_)
        | Expr::ClassDef(_)
        | Expr::PatchDef(_)
        | Expr::Lambda(_)
        | Expr::Call(_)
        | Expr::Record(_)
        | Expr::List(_)
        | Expr::Tuple(_)
        | Expr::Dict(_)
        | Expr::Set(_)
        | Expr::Compound(_)
        | Expr::ReDef(_)
        | Expr::InlineModule(_) => true,
        _ => false,
    }
}

fn fold_expr(expr: &Expr, out: &mut Vec<FoldingRange>) {
    if is_foldable(expr) {
        out.extend(region(expr.loc()));
    }
    expr.traverse(&mut |child| fold_expr(child, out));
}

impl<Checker: BuildRunnable, Parser: Parsable> Server<Checker, Parser> {
    pub(crate) fn handle_folding_range(
        &mut self,
        params: FoldingRangeParams,
    ) -> ELSResult<Option<Vec<FoldingRange>>> {
        _log!(self, "folding range requested: {params:?}");
        let uri = NormalizedUrl::new(params.text_document.uri);
        let mut res = vec![];
        if let Ok(module) = self.build_ast(&uri) {
            res.extend(fold_imports(&module));
            for chunk in module.iter() {
                fold_expr(chunk, &mut res);
            }
            dedup_by_lines(&mut res);
        }
        Ok(Some(res))
    }
}

/// Keeps the first range for each line span.
///
/// Nesting routinely produces coincident regions -- `f = x -> ...` folds as a
/// `Def` and again as the `Call` that is its whole body -- and a client shows
/// one fold marker per range, so the duplicates are visible.
fn dedup_by_lines(ranges: &mut Vec<FoldingRange>) {
    let mut seen = std::collections::HashSet::new();
    ranges.retain(|range| seen.insert((range.start_line, range.end_line)));
}

fn fold_imports(module: &Module) -> Vec<FoldingRange> {
    let mut res = vec![];
    let mut ranges = vec![];
    for chunk in module.iter() {
        match chunk {
            Expr::Def(def) if def.def_kind().is_import() => {
                ranges.push(def.loc());
            }
            _ => {
                if let Some((start, end)) = ranges.first().zip(ranges.last()) {
                    res.extend(imports_range(start, end));
                    ranges.clear();
                }
            }
        }
    }
    if let Some((start, end)) = ranges.first().zip(ranges.last()) {
        res.extend(imports_range(start, end));
    }
    res
}
