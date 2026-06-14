//! Rules about arithmetic with the identity/absorbing elements `0` and `1`.
//!
//! - `identity_op`: `x + 0`, `x - 0`, `x * 1` (analogous to `clippy::identity_op`).
//! - `erasing_op`: `x * 0` (analogous to `clippy::erasing_op`).
//! - `modulo_one`: `x % 1` (analogous to `clippy::modulo_one`).

use erg_common::traits::{Locational, NoTypeDisplay, Stream};

use erg_compiler::hir::{BinOp, Expr};
use erg_compiler::ty::{HasType, Type, ValueObj};

use erg_parser::token::TokenKind;

use crate::warn::{erasing_op, identity_op, modulo_one};
use crate::Linter;

impl Linter {
    pub(crate) fn lint_arithmetic(&mut self, expr: &Expr) {
        if let Expr::BinOp(binop) = expr {
            self.check_arithmetic(binop);
        }
        self.check_recursively(&Self::lint_arithmetic, expr);
    }

    fn check_arithmetic(&mut self, binop: &BinOp) {
        let lhs = binop.lhs.as_ref();
        let rhs = binop.rhs.as_ref();
        let lhs_val = num_literal_value(lhs);
        let rhs_val = num_literal_value(rhs);
        match binop.op.kind {
            // `x + 0` / `0 + x` / `x - 0` are just `x`.
            TokenKind::Plus if rhs_val == Some(0.0) => self.warn_identity(binop, lhs),
            TokenKind::Plus if lhs_val == Some(0.0) => self.warn_identity(binop, rhs),
            TokenKind::Minus if rhs_val == Some(0.0) => self.warn_identity(binop, lhs),
            // `x * 1` / `1 * x` are just `x`; `x * 0` / `0 * x` are `0` (numeric only).
            TokenKind::Star if rhs_val == Some(1.0) => self.warn_identity(binop, lhs),
            TokenKind::Star if lhs_val == Some(1.0) => self.warn_identity(binop, rhs),
            TokenKind::Star
                if (rhs_val == Some(0.0) && is_numeric(lhs.ref_t()))
                    || (lhs_val == Some(0.0) && is_numeric(rhs.ref_t())) =>
            {
                self.warns.push(erasing_op(
                    self.input(),
                    line!() as usize,
                    self.caused_by(),
                    binop.loc(),
                ));
            }
            // `x % 1` is always `0` (numeric only).
            TokenKind::Mod if rhs_val == Some(1.0) && is_numeric(lhs.ref_t()) => {
                self.warns.push(modulo_one(
                    self.input(),
                    line!() as usize,
                    self.caused_by(),
                    binop.loc(),
                ));
            }
            _ => {}
        }
    }

    fn warn_identity(&mut self, binop: &BinOp, kept: &Expr) {
        self.warns.push(identity_op(
            self.input(),
            line!() as usize,
            self.caused_by(),
            binop.loc(),
            kept.to_string_notype(),
        ));
    }
}

/// The numeric value of an `Int`/`Nat`/`Float` literal, if `expr` is one.
///
/// `Bool` is intentionally excluded: `x + True` is unusual enough that
/// rewriting it would be more confusing than helpful.
fn num_literal_value(expr: &Expr) -> Option<f64> {
    let Expr::Literal(lit) = expr else {
        return None;
    };
    match &lit.value {
        ValueObj::Int(i) => Some(*i as f64),
        ValueObj::Nat(n) => Some(*n as f64),
        ValueObj::Float(f) => Some(**f),
        _ => None,
    }
}

/// Whether `t` is a numeric type, looking through refinements.
///
/// Used to gate `* 0` / `% 1`, which only reduce to `0` for numbers
/// (`list * 0` is `[]`, `str * 0` is `""`).
fn is_numeric(t: &Type) -> bool {
    match t {
        Type::Int | Type::Nat | Type::Ratio | Type::Float | Type::Bool => true,
        Type::Refinement(refine) => is_numeric(refine.t.as_ref()),
        _ => false,
    }
}
