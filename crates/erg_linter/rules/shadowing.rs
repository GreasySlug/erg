//! Rules about shadowing built-in names.
//!
//! The compiler already handles most cases: shadowing a built-in **type**
//! (`Int = 1`) is an error, and shadowing a built-in **function** or a local
//! variable (`map = 1`) is a `NameWarning`. The one case it does **not** report
//! is a **parameter** that shadows a built-in (`f map = ...`), so that is the
//! gap this rule fills (avoiding duplicate warnings). Analogous to
//! pylint's `redefined-builtin` (W0622) for arguments.

use erg_common::traits::{Locational, Stream};

use erg_compiler::hir::{Def, Expr, Params, Signature};

use crate::warn::builtin_shadowing;
use crate::Linter;

impl Linter {
    pub(crate) fn lint_builtin_shadowing(&mut self, expr: &Expr) {
        match expr {
            Expr::Def(Def {
                sig: Signature::Subr(subr),
                ..
            }) => self.check_params_shadowing(&subr.params),
            Expr::Lambda(lambda) => self.check_params_shadowing(&lambda.params),
            _ => {}
        }
        self.check_recursively(&Self::lint_builtin_shadowing, expr);
    }

    fn check_params_shadowing(&mut self, params: &Params) {
        let non_defaults = params.non_defaults.iter();
        let var_params = params.var_params.as_deref().into_iter();
        let defaults = params.defaults.iter().map(|d| &d.sig);
        for param in non_defaults.chain(var_params).chain(defaults) {
            let Some(name) = param.inspect() else {
                continue;
            };
            if self.is_builtin_name(name) {
                self.warns.push(builtin_shadowing(
                    self.input(),
                    line!() as usize,
                    self.caused_by(),
                    param.loc(),
                    name,
                ));
            }
        }
    }
}
