//! Rules about the size/shape of definitions.
//!
//! - `too_many_params`: subroutines with too many parameters (analogous to
//!   `clippy::too_many_arguments`).
//! - `too_many_instance_attributes`: classes with too many fields.

use erg_common::traits::{Locational, Stream};

use erg_compiler::hir::{ClassDef, Def, Expr, Signature};
use erg_compiler::ty::{value::TypeObj, Type};

use crate::warn::{too_many_instance_attributes, too_many_params};
use crate::Linter;

impl Linter {
    pub(crate) fn lint_too_many_params(&mut self, expr: &Expr) {
        match expr {
            Expr::Def(Def {
                sig: Signature::Subr(subr),
                body,
            }) => {
                if subr.params.len() >= 8 {
                    self.warns.push(too_many_params(
                        self.input(),
                        self.caused_by(),
                        subr.params.loc(),
                    ));
                }
                for chunk in body.block.iter() {
                    self.lint_too_many_params(chunk);
                }
            }
            _ => self.check_recursively(&Self::lint_too_many_params, expr),
        }
    }

    pub(crate) fn lint_too_many_instance_attributes(&mut self, expr: &Expr) {
        const MAX_INSTANCE_ATTRIBUTES: usize = 36;
        if let Expr::ClassDef(ClassDef { obj, .. }) = expr {
            if let Some(TypeObj::Builtin {
                t: Type::Record(record),
                ..
            }) = obj.base_or_sup()
            {
                if record.len() >= MAX_INSTANCE_ATTRIBUTES {
                    self.warns.push(too_many_instance_attributes(
                        self.input(),
                        line!() as usize,
                        self.caused_by(),
                        expr.loc(),
                    ));
                }
            }
        } else {
            self.check_recursively(&Self::lint_too_many_instance_attributes, expr);
        }
    }
}
