//! The shared HIR traversal engine used by every lint rule.
//!
//! A rule inspects the node it cares about and then calls
//! [`Linter::check_recursively`] with itself, so it visits every descendant
//! expression without each rule re-implementing the walk.

use erg_common::log;

use erg_compiler::hir::{Accessor, Dict, Expr, List, Set, Signature, Tuple};

use crate::Linter;

impl Linter {
    pub(crate) fn check_recursively(&mut self, lint_fn: &impl Fn(&mut Linter, &Expr), expr: &Expr) {
        match expr {
            Expr::Literal(_) => {}
            Expr::Record(record) => {
                for attr in record.attrs.iter() {
                    for chunk in attr.body.block.iter() {
                        lint_fn(self, chunk);
                    }
                    if let Signature::Subr(subr) = &attr.sig {
                        for param in subr.params.defaults.iter() {
                            lint_fn(self, &param.default_val);
                        }
                    }
                }
            }
            Expr::List(list) => match list {
                List::Normal(lis) => {
                    for elem in lis.elems.pos_args.iter() {
                        lint_fn(self, &elem.expr);
                    }
                }
                List::WithLength(lis) => {
                    lint_fn(self, &lis.elem);
                    if let Some(len) = &lis.len {
                        lint_fn(self, len);
                    }
                }
                List::Comprehension(lis) => {
                    lint_fn(self, &lis.elem);
                    lint_fn(self, &lis.guard);
                }
            },
            Expr::Tuple(tuple) => match tuple {
                Tuple::Normal(tup) => {
                    for elem in tup.elems.pos_args.iter() {
                        lint_fn(self, &elem.expr);
                    }
                }
            },
            Expr::Set(set) => match set {
                Set::Normal(set) => {
                    for elem in set.elems.pos_args.iter() {
                        lint_fn(self, &elem.expr);
                    }
                }
                Set::WithLength(set) => {
                    lint_fn(self, &set.elem);
                    lint_fn(self, &set.len);
                }
            },
            Expr::Dict(dict) => match dict {
                Dict::Normal(dic) => {
                    for kv in dic.kvs.iter() {
                        lint_fn(self, &kv.key);
                        lint_fn(self, &kv.value);
                    }
                }
                _ => {
                    log!("Dict comprehension not implemented");
                }
            },
            Expr::BinOp(binop) => {
                lint_fn(self, &binop.lhs);
                lint_fn(self, &binop.rhs);
            }
            Expr::UnaryOp(unaryop) => {
                lint_fn(self, &unaryop.expr);
            }
            Expr::Call(call) => {
                lint_fn(self, &call.obj);
                for arg in call.args.iter() {
                    lint_fn(self, arg);
                }
            }
            Expr::Def(def) => {
                for chunk in def.body.block.iter() {
                    lint_fn(self, chunk);
                }
                if let Signature::Subr(subr) = &def.sig {
                    for param in subr.params.defaults.iter() {
                        lint_fn(self, &param.default_val);
                    }
                }
            }
            Expr::ClassDef(class_def) => {
                if let Signature::Subr(subr) = &class_def.sig {
                    for param in subr.params.defaults.iter() {
                        lint_fn(self, &param.default_val);
                    }
                }
                for methods in class_def.methods_list.iter() {
                    for method in methods.defs.iter() {
                        lint_fn(self, method);
                    }
                }
            }
            Expr::PatchDef(class_def) => {
                if let Signature::Subr(subr) = &class_def.sig {
                    for param in subr.params.defaults.iter() {
                        lint_fn(self, &param.default_val);
                    }
                }
                for method in class_def.methods.iter() {
                    lint_fn(self, method);
                }
            }
            Expr::ReDef(redef) => {
                self.check_acc_recursively(lint_fn, &redef.attr);
                for chunk in redef.block.iter() {
                    lint_fn(self, chunk);
                }
            }
            Expr::Lambda(lambda) => {
                for chunk in lambda.body.iter() {
                    lint_fn(self, chunk);
                }
                for param in lambda.params.defaults.iter() {
                    lint_fn(self, &param.default_val);
                }
            }
            Expr::TypeAsc(tasc) => {
                lint_fn(self, &tasc.expr);
            }
            Expr::Accessor(acc) => {
                self.check_acc_recursively(lint_fn, acc);
            }
            Expr::Compound(exprs) => {
                for chunk in exprs.iter() {
                    lint_fn(self, chunk);
                }
            }
            Expr::Dummy(exprs) => {
                for chunk in exprs.iter() {
                    lint_fn(self, chunk);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn check_acc_recursively(
        &mut self,
        lint_fn: impl Fn(&mut Linter, &Expr),
        acc: &Accessor,
    ) {
        match acc {
            Accessor::Attr(attr) => lint_fn(self, &attr.obj),
            Accessor::Ident(_) => {}
        }
    }
}
