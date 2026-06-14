//! Rules about redundant constructs.
//!
//! - `double_negation`: `not (not x)` is equivalent to `x` (analogous to
//!   `clippy::nonminimal_bool` / `double_neg`).

use erg_common::traits::{Locational, Stream};

use erg_compiler::hir::{Call, Expr};

use crate::warn::double_negation;
use crate::Linter;

impl Linter {
    /// `not (not x)` is equivalent to `x` (for `Bool`), so it is redundant.
    pub(crate) fn lint_double_negation(&mut self, expr: &Expr) {
        if let Expr::Call(call) = expr {
            if is_not_call(call) {
                if let Some(inner) = call.args.pos_args.first() {
                    if let Expr::Call(inner_call) = &inner.expr {
                        if is_not_call(inner_call) {
                            if let Some(operand) = inner_call.args.pos_args.first() {
                                self.warns.push(double_negation(
                                    self.input(),
                                    line!() as usize,
                                    self.caused_by(),
                                    call.loc(),
                                    operand.expr.clone(),
                                ));
                            }
                        }
                    }
                }
            }
        }
        self.check_recursively(&Self::lint_double_negation, expr);
    }
}

/// Whether `call` is a unary `not x` application.
fn is_not_call(call: &Call) -> bool {
    call.attr_name.is_none()
        && call.args.pos_args.len() == 1
        && call.obj.local_name() == Some("not")
}
