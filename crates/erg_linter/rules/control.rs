//! Rules about control-flow expressions (`if` / `if!`) and reachability.
//!
//! - `needless_bool`: `if cond, do(True), do(False)` is just `cond` (analogous
//!   to `clippy::needless_bool`).
//! - `redundant_branches`: `if cond, do(x), do(x)` is just `x` (analogous to
//!   `clippy::if_same_then_else`).
//! - `unreachable_code`: chunks following an expression that always diverges
//!   (analogous to rustc's `unreachable_code`).
//!
//! In the HIR, `if`/`if!` is a `Call` whose positional arguments are
//! `[cond, then_lambda, else_lambda]`, and each branch is a `do(...)` lambda
//! whose `body` is a `Block`. A diverging expression (`return`/`panic`/…) has
//! type `Never`.

use erg_common::traits::{Locational, NoTypeDisplay, Stream};

use erg_compiler::hir::{Block, Call, Expr};
use erg_compiler::ty::{HasType, Type, ValueObj};

use crate::warn::{needless_bool, redundant_branches, unreachable_code};
use crate::Linter;

impl Linter {
    pub(crate) fn lint_redundant_if(&mut self, expr: &Expr) {
        if let Expr::Call(call) = expr {
            if let Some((cond, then_blk, else_blk)) = as_if_branches(call) {
                if then_blk == else_blk {
                    // `if cond, do(x), do(x)` → `x`
                    self.warns.push(redundant_branches(
                        self.input(),
                        line!() as usize,
                        self.caused_by(),
                        call.loc(),
                        block_suggestion(then_blk),
                    ));
                } else if let (Some(then_b), Some(else_b)) =
                    (single_bool(then_blk), single_bool(else_blk))
                {
                    // `if cond, do(True), do(False)` → `cond`
                    // `if cond, do(False), do(True)` → `not cond`
                    let cond = cond.to_string_notype();
                    let suggestion = match (then_b, else_b) {
                        (true, false) => cond,
                        (false, true) => format!("not ({cond})"),
                        // both branches are the same literal: handled by the
                        // `then_blk == else_blk` arm above.
                        _ => return,
                    };
                    self.warns.push(needless_bool(
                        self.input(),
                        line!() as usize,
                        self.caused_by(),
                        call.loc(),
                        suggestion,
                    ));
                }
            }
        }
        self.check_recursively(&Self::lint_redundant_if, expr);
    }

    /// Flags chunks that follow an always-diverging expression within a block.
    pub(crate) fn lint_unreachable(&mut self, expr: &Expr) {
        match expr {
            Expr::Def(def) => self.check_chunks(def.body.block.iter()),
            Expr::Lambda(lambda) => self.check_chunks(lambda.body.iter()),
            Expr::ReDef(redef) => self.check_chunks(redef.block.iter()),
            _ => {}
        }
        self.check_recursively(&Self::lint_unreachable, expr);
    }

    /// Warns on the first chunk that comes after one whose type is `Never`
    /// (i.e. an expression that always diverges). One warning per block.
    pub(crate) fn check_chunks<'a>(&mut self, chunks: impl Iterator<Item = &'a Expr>) {
        let mut diverged = false;
        for chunk in chunks {
            if diverged {
                self.warns.push(unreachable_code(
                    self.input(),
                    line!() as usize,
                    self.caused_by(),
                    chunk.loc(),
                ));
                return;
            }
            if matches!(chunk.ref_t(), Type::Never) {
                diverged = true;
            }
        }
    }
}

/// If `call` is an `if`/`if!` with both branches present, returns
/// `(condition, then_body, else_body)`.
fn as_if_branches(call: &Call) -> Option<(&Expr, &Block, &Block)> {
    let name = call.obj.local_name()?;
    if name != "if" && name != "if!" {
        return None;
    }
    let pos = &call.args.pos_args;
    if pos.len() != 3 {
        return None;
    }
    let cond = &pos[0].expr;
    let then_blk = as_do_block(&pos[1].expr)?;
    let else_blk = as_do_block(&pos[2].expr)?;
    Some((cond, then_blk, else_blk))
}

/// The body block of a `do(...)` / `do!(...)` branch (a zero-arg lambda).
fn as_do_block(expr: &Expr) -> Option<&Block> {
    match expr {
        Expr::Lambda(lambda) => Some(&lambda.body),
        _ => None,
    }
}

/// If `block` is a single `Bool` literal, returns its value.
fn single_bool(block: &Block) -> Option<bool> {
    if block.len() != 1 {
        return None;
    }
    match block.iter().next()? {
        Expr::Literal(lit) => match &lit.value {
            ValueObj::Bool(b) => Some(*b),
            _ => None,
        },
        _ => None,
    }
}

/// A human-readable rendering of a branch body for the warning hint.
fn block_suggestion(block: &Block) -> String {
    match block.iter().next() {
        Some(expr) if block.len() == 1 => expr.to_string_notype(),
        _ => "the branch body".to_string(),
    }
}
