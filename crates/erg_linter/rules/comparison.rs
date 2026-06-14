//! Rules about comparison operators.
//!
//! - `tautology` / `contradiction`: self-comparisons that are always true/false
//!   (analogous to `clippy::eq_op`).
//! - `bool_comparison`: redundant `== True` / `!= False` (analogous to
//!   `clippy::bool_comparison`).

use erg_common::traits::{Locational, Stream};

use erg_compiler::hir::{Expr, Literal};
use erg_compiler::ty::{HasType, Type, ValueObj};

use erg_parser::token::TokenKind;

use crate::warn::{
    contradiction, false_comparison, nat_bound_comparison, tautology, true_comparison,
};
use crate::Linter;

impl Linter {
    pub(crate) fn lint_tautology(&mut self, expr: &Expr) {
        if let Expr::BinOp(binop) = expr {
            // `x >= x`, `x <= x`, `x == x` are always true (tautology),
            // while `x < x`, `x > x`, `x != x` are always false (contradiction).
            let kind = match binop.op.kind {
                TokenKind::GreEq | TokenKind::LessEq | TokenKind::DblEq => Some(true),
                TokenKind::Less | TokenKind::Gre | TokenKind::NotEq => Some(false),
                _ => None,
            };
            if let Some(is_tautology) = kind {
                let lhs = binop.lhs.as_ref();
                let rhs = binop.rhs.as_ref();
                if lhs == rhs {
                    let warn = if is_tautology {
                        tautology(
                            self.input(),
                            line!() as usize,
                            self.caused_by(),
                            binop.loc(),
                            lhs.clone(),
                        )
                    } else {
                        contradiction(
                            self.input(),
                            line!() as usize,
                            self.caused_by(),
                            binop.loc(),
                            lhs.clone(),
                        )
                    };
                    self.warns.push(warn);
                }
            }
        }
        self.check_recursively(&Self::lint_tautology, expr);
    }

    pub(crate) fn lint_bool_comparison(&mut self, expr: &Expr) {
        match expr {
            Expr::BinOp(binop) => {
                let lhs = <&Literal>::try_from(binop.lhs.as_ref()).map(|lit| &lit.value);
                let rhs = <&Literal>::try_from(binop.rhs.as_ref()).map(|lit| &lit.value);
                let lhs_or_rhs = lhs.or(rhs);
                let func = match (binop.op.kind, lhs_or_rhs) {
                    (TokenKind::DblEq, Ok(ValueObj::Bool(true))) => true_comparison,
                    (TokenKind::DblEq, Ok(ValueObj::Bool(false))) => false_comparison,
                    (TokenKind::NotEq, Ok(ValueObj::Bool(true))) => false_comparison,
                    (TokenKind::NotEq, Ok(ValueObj::Bool(false))) => true_comparison,
                    _ => {
                        self.lint_bool_comparison(&binop.lhs);
                        self.lint_bool_comparison(&binop.rhs);
                        return;
                    }
                };
                let expr = if lhs.is_ok_and(|val| val.is_bool()) {
                    binop.rhs.as_ref()
                } else if rhs.is_ok_and(|val| val.is_bool()) {
                    binop.lhs.as_ref()
                } else {
                    self.lint_bool_comparison(&binop.lhs);
                    self.lint_bool_comparison(&binop.rhs);
                    return;
                };
                let warn = func(expr, self.input(), self.caused_by(), binop.loc());
                self.warns.push(warn);
                self.lint_bool_comparison(&binop.lhs);
                self.lint_bool_comparison(&binop.rhs);
            }
            _ => self.check_recursively(&Self::lint_bool_comparison, expr),
        }
    }

    /// Comparisons of a `Nat` against `0` whose result is fixed by the
    /// non-negativity of `Nat` (e.g. `n < 0` is always false, `n >= 0` is
    /// always true). Analogous to `clippy::absurd_extreme_comparisons`.
    pub(crate) fn lint_absurd_comparison(&mut self, expr: &Expr) {
        if let Expr::BinOp(binop) = expr {
            let lhs = binop.lhs.as_ref();
            let rhs = binop.rhs.as_ref();
            // Skip self-comparisons (`0 < 0` etc.), which `lint_tautology` owns.
            if lhs != rhs {
                let result = match binop.op.kind {
                    // `nat < 0` / `nat >= 0`
                    _ if is_nat(lhs.ref_t()) && is_zero_literal(rhs) => match binop.op.kind {
                        TokenKind::Less => Some(false),
                        TokenKind::GreEq => Some(true),
                        _ => None,
                    },
                    // `0 > nat` / `0 <= nat`
                    _ if is_zero_literal(lhs) && is_nat(rhs.ref_t()) => match binop.op.kind {
                        TokenKind::Gre => Some(false),
                        TokenKind::LessEq => Some(true),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(result) = result {
                    self.warns.push(nat_bound_comparison(
                        self.input(),
                        line!() as usize,
                        self.caused_by(),
                        binop.loc(),
                        result,
                    ));
                }
            }
        }
        self.check_recursively(&Self::lint_absurd_comparison, expr);
    }
}

/// Whether `t` is `Nat` (or a refinement of it). `Int` is excluded because it
/// can be negative.
fn is_nat(t: &Type) -> bool {
    match t {
        Type::Nat => true,
        Type::Refinement(refine) => is_nat(refine.t.as_ref()),
        _ => false,
    }
}

/// Whether `expr` is the integer literal `0`.
fn is_zero_literal(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Literal(lit) if matches!(&lit.value, ValueObj::Nat(0) | ValueObj::Int(0))
    )
}
