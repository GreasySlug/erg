//! Rules that use Erg's effect system.
//!
//! - `effect_free_proc`: a procedure (`f! x = ...`) whose body has no side
//!   effects could be a plain function (`f x = ...`). There is no direct Clippy
//!   analogue — this leverages Erg's `!`/effect distinction.
//!
//! A side effect in Erg always flows through a **procedure call**: either the
//! callee's type is procedural (`=>`), or it is a `!`-method. We therefore scan
//! the procedure body for any such call. This mirrors how the compiler's
//! `SideEffectChecker` decides effects (`call.obj.t().is_procedure()` / a
//! procedural `attr_name`).

use erg_common::traits::{Locational, Stream};

use erg_compiler::hir::{Accessor, Block, Dict, Expr, List, Set, Signature, Tuple};
use erg_compiler::ty::HasType;

use crate::warn::effect_free_proc;
use crate::Linter;

impl Linter {
    pub(crate) fn lint_effect_free_proc(&mut self, expr: &Expr) {
        if let Expr::Def(def) = expr {
            if def.sig.is_subr() && def.sig.is_procedural() && !block_has_effect(&def.body.block) {
                // suggest the same name without the trailing `!`
                let suggested = def.sig.inspect().trim_end_matches('!').to_string();
                self.warns.push(effect_free_proc(
                    self.input(),
                    line!() as usize,
                    self.caused_by(),
                    def.sig.loc(),
                    &suggested,
                ));
            }
        }
        self.check_recursively(&Self::lint_effect_free_proc, expr);
    }
}

fn block_has_effect(block: &Block) -> bool {
    block.iter().any(expr_has_effect)
}

/// Whether evaluating `expr` (or any sub-expression) performs a side effect,
/// i.e. calls a procedure (`=>`-typed callee or a `!`-method).
fn expr_has_effect(expr: &Expr) -> bool {
    if let Expr::Call(call) = expr {
        if call.obj.ref_t().is_procedure()
            || call
                .attr_name
                .as_ref()
                .is_some_and(|name| name.is_procedural())
        {
            return true;
        }
    }
    // recurse into children (mirrors `Linter::check_recursively`)
    match expr {
        Expr::Record(record) => record.attrs.iter().any(|attr| {
            attr.body.block.iter().any(expr_has_effect) || subr_defaults_have_effect(&attr.sig)
        }),
        Expr::List(List::Normal(lis)) => {
            lis.elems.pos_args.iter().any(|e| expr_has_effect(&e.expr))
        }
        Expr::List(List::WithLength(lis)) => {
            expr_has_effect(&lis.elem) || lis.len.as_deref().is_some_and(expr_has_effect)
        }
        Expr::List(List::Comprehension(lis)) => {
            expr_has_effect(&lis.elem) || expr_has_effect(&lis.guard)
        }
        Expr::Tuple(Tuple::Normal(tup)) => {
            tup.elems.pos_args.iter().any(|e| expr_has_effect(&e.expr))
        }
        Expr::Set(Set::Normal(set)) => set.elems.pos_args.iter().any(|e| expr_has_effect(&e.expr)),
        Expr::Set(Set::WithLength(set)) => expr_has_effect(&set.elem) || expr_has_effect(&set.len),
        Expr::Dict(Dict::Normal(dic)) => dic
            .kvs
            .iter()
            .any(|kv| expr_has_effect(&kv.key) || expr_has_effect(&kv.value)),
        Expr::BinOp(binop) => expr_has_effect(&binop.lhs) || expr_has_effect(&binop.rhs),
        Expr::UnaryOp(unaryop) => expr_has_effect(&unaryop.expr),
        Expr::Call(call) => expr_has_effect(&call.obj) || call.args.iter().any(expr_has_effect),
        Expr::Def(def) => {
            def.body.block.iter().any(expr_has_effect) || subr_defaults_have_effect(&def.sig)
        }
        Expr::Lambda(lambda) => {
            lambda.body.iter().any(expr_has_effect)
                || lambda
                    .params
                    .defaults
                    .iter()
                    .any(|p| expr_has_effect(&p.default_val))
        }
        Expr::ReDef(redef) => {
            acc_has_effect(&redef.attr) || redef.block.iter().any(expr_has_effect)
        }
        Expr::TypeAsc(tasc) => expr_has_effect(&tasc.expr),
        Expr::Accessor(acc) => acc_has_effect(acc),
        Expr::Compound(exprs) => exprs.iter().any(expr_has_effect),
        Expr::Dummy(exprs) => exprs.iter().any(expr_has_effect),
        _ => false,
    }
}

fn subr_defaults_have_effect(sig: &Signature) -> bool {
    matches!(sig, Signature::Subr(subr) if subr
        .params
        .defaults
        .iter()
        .any(|p| expr_has_effect(&p.default_val)))
}

fn acc_has_effect(acc: &Accessor) -> bool {
    match acc {
        Accessor::Attr(attr) => expr_has_effect(&attr.obj),
        Accessor::Ident(_) => false,
    }
}
