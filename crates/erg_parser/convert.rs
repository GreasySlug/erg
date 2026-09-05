//! Re-reads expressions as the left-hand sides they turn out to be.
//!
//! The parser reads `f(x, y := 1) = ...` and `(x, y) -> ...` as an ordinary call
//! and tuple first, since only the `=` or `->` that follows tells it they were a
//! signature and a parameter list. The methods here convert the resulting `Expr`
//! into `Signature`, `Params` and the pattern types, and fail with a syntax error
//! on a shape that cannot be a left-hand side.

use erg_common::error::Location;
use erg_common::set;
use erg_common::traits::{Locational, Stream};

use crate::ast::*;
use crate::error::{ParseError, ParseResult};
use crate::parse::trace;
use crate::token::{Token, TokenKind};
use crate::Parser;

impl Parser {
    /// Converts the left-hand side of a definition into its signature.
    ///
    /// A name or a type application gives a variable signature, a call `f(x)` a
    /// subroutine signature, and a list, tuple, record or data pack a destructuring
    /// pattern; a type ascription adds its type to any of these.
    pub(crate) fn convert_rhs_to_sig(&mut self, rhs: Expr) -> ParseResult<Signature> {
        trace!(self);
        match rhs {
            Expr::Accessor(accessor) => {
                Ok(Signature::Var(self.convert_accessor_to_var_sig(accessor)?))
            }
            Expr::Call(call) => Ok(Signature::Subr(self.convert_call_to_subr_sig(call)?)),
            Expr::List(list) => {
                let list_pat = self.convert_list_to_list_pat(list)?;
                Ok(Signature::Var(VarSignature::new(
                    VarPattern::List(list_pat),
                    None,
                    None,
                )))
            }
            Expr::Record(record) => {
                let record_pat = self.convert_record_to_record_pat(record)?;
                Ok(Signature::Var(VarSignature::new(
                    VarPattern::Record(record_pat),
                    None,
                    None,
                )))
            }
            Expr::DataPack(pack) => {
                let data_pack = self.convert_data_pack_to_data_pack_pat(pack)?;
                Ok(Signature::Var(VarSignature::new(
                    VarPattern::DataPack(data_pack),
                    None,
                    None,
                )))
            }
            Expr::Tuple(tuple) => {
                let tuple_pat = self.convert_tuple_to_tuple_pat(tuple)?;
                Ok(Signature::Var(VarSignature::new(
                    VarPattern::Tuple(tuple_pat),
                    None,
                    None,
                )))
            }
            Expr::TypeAscription(tasc) => Ok(self.convert_type_asc_to_sig(tasc)?),
            other => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                other.loc(),
            )),
        }
    }

    /// Converts a name (`_` becomes the discard pattern) or a type application
    /// `T|...|` into a variable signature.
    fn convert_accessor_to_var_sig(&mut self, accessor: Accessor) -> ParseResult<VarSignature> {
        trace!(self);
        match accessor {
            Accessor::Ident(ident) => {
                let pat = if &ident.inspect()[..] == "_" {
                    VarPattern::Discard(ident.name.into_token())
                } else {
                    VarPattern::Ident(ident)
                };
                Ok(VarSignature::new(pat, None, None))
            }
            Accessor::TypeApp(t_app) => {
                let (ident, bounds) = self.convert_accessor_to_ident(Accessor::TypeApp(t_app))?;
                let pat = VarPattern::Ident(ident);
                Ok(VarSignature::new(pat, None, Some(bounds)))
            }
            other => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                other.loc(),
            )),
        }
    }

    /// Converts `[a, b, *rest]` into a list pattern; every element must itself be a
    /// variable pattern.
    ///
    /// # Errors
    ///
    /// A subroutine signature as an element, a comprehension, or `[x; n]`, which is
    /// not supported as a pattern.
    fn convert_list_to_list_pat(&mut self, list: List) -> ParseResult<VarListPattern> {
        trace!(self);
        match list {
            List::Normal(lis) => {
                let mut vars = Vars::empty();
                for elem in lis.elems.pos_args {
                    let pat = self.convert_rhs_to_sig(elem.expr)?;
                    match pat {
                        Signature::Var(v) => {
                            vars.push(v);
                        }
                        Signature::Subr(subr) => {
                            return self.fail(ParseError::simple_syntax_error(
                                line!() as usize,
                                subr.loc(),
                            ));
                        }
                    }
                }
                if let Some(var) = lis.elems.var_args {
                    let pat = self.convert_rhs_to_sig(var.expr)?;
                    match pat {
                        Signature::Var(v) => {
                            vars.starred = Some(Box::new(v));
                        }
                        Signature::Subr(subr) => {
                            return self.fail(ParseError::simple_syntax_error(
                                line!() as usize,
                                subr.loc(),
                            ));
                        }
                    }
                }
                let sqbrs = Location::concat(&lis.l_sqbr, &lis.r_sqbr);
                Ok(VarListPattern::new(sqbrs, vars))
            }
            List::Comprehension(lis) => {
                self.fail(ParseError::simple_syntax_error(line!() as usize, lis.loc()))
            }
            List::WithLength(lis) => self.fail(ParseError::feature_error(
                line!() as usize,
                lis.loc(),
                "list-with-length pattern",
            )),
        }
    }

    /// Converts one `x = y` attribute of a record pattern; `y` must be a variable
    /// pattern.
    ///
    /// # Panics
    ///
    /// If the attribute's body is not a single expression, which the parser
    /// guarantees for record attributes.
    fn convert_def_to_var_record_attr(&mut self, mut attr: Def) -> ParseResult<VarRecordAttr> {
        trace!(self);
        let Signature::Var(VarSignature {
            pat: VarPattern::Ident(lhs),
            ..
        }) = attr.sig
        else {
            return self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                attr.sig.loc(),
            ));
        };
        assert_eq!(attr.body.block.len(), 1);
        let first = attr.body.block.remove(0);
        let Expr::Accessor(rhs) = first else {
            return self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                first.loc(),
            ));
        };
        let rhs = self.convert_accessor_to_var_sig(rhs)?;
        Ok(VarRecordAttr::new(lhs, rhs))
    }

    /// Converts `{x = a; y}` into a record pattern; the shorthand `y` binds `y`.
    fn convert_record_to_record_pat(&mut self, record: Record) -> ParseResult<VarRecordPattern> {
        trace!(self);
        match record {
            Record::Normal(rec) => {
                let pats = rec
                    .attrs
                    .into_iter()
                    .map(|attr| self.convert_def_to_var_record_attr(attr))
                    .collect::<ParseResult<Vec<_>>>()?;
                let attrs = VarRecordAttrs::new(pats);
                let braces = Location::concat(&rec.l_brace, &rec.r_brace);
                Ok(VarRecordPattern::new(braces, attrs))
            }
            Record::Mixed(rec) => {
                let pats = rec
                    .attrs
                    .into_iter()
                    .map(|attr_or_ident| match attr_or_ident {
                        RecordAttrOrIdent::Attr(attr) => self.convert_def_to_var_record_attr(attr),
                        RecordAttrOrIdent::Ident(ident) => {
                            let rhs =
                                VarSignature::new(VarPattern::Ident(ident.clone()), None, None);
                            Ok(VarRecordAttr::new(ident, rhs))
                        }
                    })
                    .collect::<ParseResult<Vec<_>>>()?;
                let attrs = VarRecordAttrs::new(pats);
                let braces = Location::concat(&rec.l_brace, &rec.r_brace);
                Ok(VarRecordPattern::new(braces, attrs))
            }
        }
    }

    /// Converts `C::{x = a}` into a data pack pattern: the class as a type and the
    /// record as a pattern.
    fn convert_data_pack_to_data_pack_pat(
        &mut self,
        pack: DataPack,
    ) -> ParseResult<VarDataPackPattern> {
        trace!(self);
        let class = Self::expr_to_type_spec(*pack.class.clone()).map_err(|e| self.errs.push(e))?;
        let args = self.convert_record_to_record_pat(pack.args)?;
        Ok(VarDataPackPattern::new(class, pack.class, args))
    }

    /// Converts `(a, b, *rest)` into a tuple pattern; every element must itself be a
    /// variable pattern.
    ///
    /// # Errors
    ///
    /// A subroutine signature as an element, or a tuple comprehension.
    fn convert_tuple_to_tuple_pat(&mut self, tuple: Tuple) -> ParseResult<VarTuplePattern> {
        trace!(self);
        let mut vars = Vars::empty();
        match tuple {
            Tuple::Normal(tup) => {
                let (pos_args, var_args, _kw_args, _kw_var, paren) = tup.elems.deconstruct();
                for arg in pos_args {
                    let sig = self.convert_rhs_to_sig(arg.expr)?;
                    match sig {
                        Signature::Var(var) => {
                            vars.push(var);
                        }
                        other => {
                            let err =
                                ParseError::simple_syntax_error(line!() as usize, other.loc());
                            return self.fail(err);
                        }
                    }
                }
                if let Some(var_args) = var_args {
                    let sig = self.convert_rhs_to_sig(var_args.expr)?;
                    match sig {
                        Signature::Var(var) => {
                            vars.starred = Some(Box::new(var));
                        }
                        other => {
                            let err =
                                ParseError::simple_syntax_error(line!() as usize, other.loc());
                            return self.fail(err);
                        }
                    }
                }
                Ok(VarTuplePattern::new(paren, vars))
            }
            Tuple::Comprehension(comp) => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                comp.loc(),
            )),
        }
    }

    /// Converts `lhs: T` into the signature of `lhs` with `T` as its type.
    fn convert_type_asc_to_sig(&mut self, tasc: TypeAscription) -> ParseResult<Signature> {
        trace!(self);
        let sig = self.convert_rhs_to_sig(*tasc.expr)?;
        let sig = match sig {
            Signature::Var(var) => {
                let var = VarSignature::new(var.pat, Some(tasc.t_spec), Some(var.bounds));
                Signature::Var(var)
            }
            Signature::Subr(subr) => {
                let subr = SubrSignature::new(
                    subr.decorators,
                    subr.ident,
                    subr.bounds,
                    subr.params,
                    Some(tasc.t_spec),
                );
                Signature::Subr(subr)
            }
        };
        Ok(sig)
    }

    /// Converts `f(params)` or `f|T|(params)` into a subroutine signature.
    ///
    /// # Errors
    ///
    /// If the callee is not a plain name, a method call for instance.
    fn convert_call_to_subr_sig(&mut self, call: Call) -> ParseResult<SubrSignature> {
        trace!(self);
        let (ident, bounds) = match *call.obj {
            Expr::Accessor(acc) => self.convert_accessor_to_ident(acc)?,
            other => {
                return self.fail(ParseError::simple_syntax_error(
                    line!() as usize,
                    other.loc(),
                ));
            }
        };
        let params = self.convert_args_to_params(call.args)?;
        Ok(SubrSignature::new(set! {}, ident, bounds, params, None))
    }

    /// Splits `name` or `name|T <: U|` into the identifier and its type bounds.
    ///
    /// # Errors
    ///
    /// For an attribute or subscript accessor.
    fn convert_accessor_to_ident(
        &mut self,
        accessor: Accessor,
    ) -> ParseResult<(Identifier, TypeBoundSpecs)> {
        trace!(self);
        let (ident, bounds) = match accessor {
            Accessor::Ident(ident) => (ident, TypeBoundSpecs::empty()),
            Accessor::TypeApp(t_app) => {
                let sig = self.convert_rhs_to_sig(*t_app.obj)?;
                let Signature::Var(VarSignature {
                    pat: VarPattern::Ident(ident),
                    ..
                }) = sig
                else {
                    return self.fail(ParseError::simple_syntax_error(line!() as usize, sig.loc()));
                };
                let bounds = self.convert_type_args_to_bounds(t_app.type_args)?;
                (ident, bounds)
            }
            other => {
                return self.fail(ParseError::simple_syntax_error(
                    line!() as usize,
                    other.loc(),
                ));
            }
        };
        Ok((ident, bounds))
    }

    /// Converts the arguments of a type application into type bounds, `T` or
    /// `T: Bound`; a `|<: T|` subtype bound gives no type bounds.
    pub(crate) fn convert_type_args_to_bounds(
        &mut self,
        type_args: TypeAppArgs,
    ) -> ParseResult<TypeBoundSpecs> {
        trace!(self);
        let TypeAppArgsKind::Args(args) = type_args.args else {
            return Ok(TypeBoundSpecs::empty());
        };
        let mut bounds = vec![];
        let (pos_args, _var_args, _kw_args, _kw_var, _paren) = args.deconstruct();
        for arg in pos_args.into_iter() {
            let bound = self.convert_type_arg_to_bound(arg)?;
            bounds.push(bound);
        }
        Ok(TypeBoundSpecs::new(bounds))
    }

    /// Converts one type argument: `T` is a type parameter without a bound, `T: Bound`
    /// one with a bound.
    fn convert_type_arg_to_bound(&mut self, arg: PosArg) -> ParseResult<TypeBoundSpec> {
        match arg.expr {
            Expr::TypeAscription(tasc) => {
                let lhs = self.convert_rhs_to_sig(*tasc.expr)?;
                let Signature::Var(VarSignature {
                    pat: VarPattern::Ident(lhs),
                    ..
                }) = lhs
                else {
                    return self.fail(ParseError::simple_syntax_error(line!() as usize, lhs.loc()));
                };
                Ok(TypeBoundSpec::non_default(lhs.name, tasc.t_spec))
            }
            Expr::Accessor(Accessor::Ident(ident)) => Ok(TypeBoundSpec::Omitted(ident.name)),
            other => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                other.loc(),
            )),
        }
    }

    /// Converts the arguments of a call into the parameters of a definition.
    ///
    /// Positional arguments become non-default parameters, `*xs` and `**kw` the
    /// variadic ones, and `k := v` the default parameters. The first positional
    /// parameter may be `self`.
    pub(crate) fn convert_args_to_params(&mut self, args: Args) -> ParseResult<Params> {
        trace!(self);
        let (pos_args, var_args, kw_args, kw_var, parens) = args.deconstruct();
        let mut params = Params::new(vec![], None, vec![], None, parens);
        for (i, arg) in pos_args.into_iter().enumerate() {
            let nd_param = self.convert_pos_arg_to_non_default_param(arg, i == 0)?;
            params.non_defaults.push(nd_param);
        }
        if let Some(var_args) = var_args {
            let var_params = self.convert_pos_arg_to_non_default_param(var_args, false)?;
            params.var_params = Some(Box::new(var_params));
        }
        // TODO: varargs
        for arg in kw_args.into_iter() {
            let d_param = self.convert_kw_arg_to_default_param(arg)?;
            params.defaults.push(d_param);
        }
        if let Some(kw_var) = kw_var {
            let kw_var_params = self.convert_pos_arg_to_non_default_param(kw_var, false)?;
            params.kw_var_params = Some(Box::new(kw_var_params));
        }
        Ok(params)
    }

    /// Converts a positional argument into a non-default parameter; see
    /// [`Parser::convert_rhs_to_param`].
    fn convert_pos_arg_to_non_default_param(
        &mut self,
        arg: PosArg,
        allow_self: bool,
    ) -> ParseResult<NonDefaultParamSignature> {
        trace!(self);
        self.convert_rhs_to_param(arg.expr, allow_self)
    }

    /// Converts an expression into a non-default parameter.
    ///
    /// A name binds a variable, a literal matches itself, and a list, tuple or record
    /// destructures; `ref x` and `ref! x` take the argument by reference; `x: T` adds
    /// a type. `self` is only allowed where `allow_self` says so, as the first
    /// parameter of a method.
    fn convert_rhs_to_param(
        &mut self,
        expr: Expr,
        allow_self: bool,
    ) -> ParseResult<NonDefaultParamSignature> {
        trace!(self);
        match expr {
            Expr::Accessor(Accessor::Ident(ident)) => {
                if &ident.inspect()[..] == "self" && !allow_self {
                    return self.fail(ParseError::simple_syntax_error(
                        line!() as usize,
                        ident.loc(),
                    ));
                }
                // FIXME deny: public
                let pat = ParamPattern::VarName(ident.name);
                Ok(NonDefaultParamSignature::new(pat, None))
            }
            Expr::Literal(lit) => {
                let pat = ParamPattern::Lit(lit);
                Ok(NonDefaultParamSignature::new(pat, None))
            }
            Expr::List(list) => {
                let list_pat = self.convert_list_to_param_list_pat(list)?;
                let pat = ParamPattern::List(list_pat);
                Ok(NonDefaultParamSignature::new(pat, None))
            }
            Expr::Record(record) => {
                let record_pat = self.convert_record_to_param_record_pat(record)?;
                let pat = ParamPattern::Record(record_pat);
                Ok(NonDefaultParamSignature::new(pat, None))
            }
            Expr::Tuple(tuple) => {
                let tuple_pat = self.convert_tuple_to_param_tuple_pat(tuple)?;
                let pat = ParamPattern::Tuple(tuple_pat);
                Ok(NonDefaultParamSignature::new(pat, None))
            }
            Expr::TypeAscription(tasc) => {
                Ok(self.convert_type_asc_to_param_pattern(tasc, allow_self)?)
            }
            Expr::UnaryOp(unary) => match unary.op.kind {
                TokenKind::RefOp => {
                    let var = unary.args.into_iter().next().unwrap();
                    let (var, t_spec) = match *var {
                        Expr::Accessor(Accessor::Ident(var)) => (var, None),
                        Expr::TypeAscription(tasc) => {
                            let var = match *tasc.expr {
                                Expr::Accessor(Accessor::Ident(var)) => var,
                                _ => {
                                    return self.fail(ParseError::simple_syntax_error(
                                        line!() as usize,
                                        tasc.loc(),
                                    ));
                                }
                            };
                            (var, Some(tasc.t_spec))
                        }
                        _ => {
                            return self.fail(ParseError::simple_syntax_error(
                                line!() as usize,
                                var.loc(),
                            ));
                        }
                    };
                    let pat = ParamPattern::Ref(var.name);
                    Ok(NonDefaultParamSignature::new(pat, t_spec))
                }
                TokenKind::RefMutOp => {
                    let var = unary.args.into_iter().next().unwrap();
                    let (var, t_spec) = match *var {
                        Expr::Accessor(Accessor::Ident(var)) => (var, None),
                        Expr::TypeAscription(tasc) => {
                            let var = match *tasc.expr {
                                Expr::Accessor(Accessor::Ident(var)) => var,
                                _ => {
                                    return self.fail(ParseError::simple_syntax_error(
                                        line!() as usize,
                                        tasc.loc(),
                                    ));
                                }
                            };
                            (var, Some(tasc.t_spec))
                        }
                        _ => {
                            return self.fail(ParseError::simple_syntax_error(
                                line!() as usize,
                                var.loc(),
                            ));
                        }
                    };
                    let pat = ParamPattern::RefMut(var.name);
                    Ok(NonDefaultParamSignature::new(pat, t_spec))
                }
                // TODO: Spread
                _other => self.fail(ParseError::simple_syntax_error(
                    line!() as usize,
                    unary.loc(),
                )),
            },
            other => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                other.loc(),
            )),
        }
    }

    /// Converts `k := v`, or `k: T := v`, into a parameter with the default `v`.
    fn convert_kw_arg_to_default_param(
        &mut self,
        arg: KwArg,
    ) -> ParseResult<DefaultParamSignature> {
        trace!(self);
        let pat = ParamPattern::VarName(VarName::new(arg.keyword));
        let sig = NonDefaultParamSignature::new(pat, arg.t_spec);
        Ok(DefaultParamSignature::new(sig, arg.expr))
    }

    /// Converts `[a, b]` into a list parameter pattern.
    ///
    /// # Errors
    ///
    /// A comprehension or `[x; n]`, which are not supported as patterns.
    fn convert_list_to_param_list_pat(&mut self, list: List) -> ParseResult<ParamListPattern> {
        trace!(self);
        match list {
            List::Normal(lis) => {
                let mut params = vec![];
                for arg in lis.elems.into_iters().0 {
                    params.push(self.convert_pos_arg_to_non_default_param(arg, false)?);
                }
                let params = Params::new(params, None, vec![], None, None);
                Ok(ParamListPattern::new(lis.l_sqbr, params, lis.r_sqbr))
            }
            other => self.fail(ParseError::feature_error(
                line!() as usize,
                other.loc(),
                "?",
            )),
        }
    }

    /// Converts one `x = pat` attribute of a record parameter pattern.
    ///
    /// # Panics
    ///
    /// If the attribute's body is not a single expression, which the parser
    /// guarantees for record attributes.
    fn convert_def_to_param_record_attr(&mut self, mut attr: Def) -> ParseResult<ParamRecordAttr> {
        let Signature::Var(VarSignature {
            pat: VarPattern::Ident(lhs),
            ..
        }) = attr.sig
        else {
            return self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                attr.sig.loc(),
            ));
        };
        assert_eq!(attr.body.block.len(), 1);
        let first = attr.body.block.remove(0);
        let rhs = self.convert_rhs_to_param(first, false)?;
        Ok(ParamRecordAttr::new(lhs, rhs))
    }

    /// Converts `{x = a; y}` into a record parameter pattern; the shorthand `y`
    /// binds `y`.
    fn convert_record_to_param_record_pat(
        &mut self,
        record: Record,
    ) -> ParseResult<ParamRecordPattern> {
        trace!(self);
        match record {
            Record::Normal(rec) => {
                let pats = rec
                    .attrs
                    .into_iter()
                    .map(|attr| self.convert_def_to_param_record_attr(attr))
                    .collect::<ParseResult<Vec<_>>>()?;
                let attrs = ParamRecordAttrs::new(pats);
                Ok(ParamRecordPattern::new(rec.l_brace, attrs, rec.r_brace))
            }
            Record::Mixed(rec) => {
                let pats = rec
                    .attrs
                    .into_iter()
                    .map(|attr_or_ident| match attr_or_ident {
                        RecordAttrOrIdent::Attr(attr) => {
                            self.convert_def_to_param_record_attr(attr)
                        }
                        RecordAttrOrIdent::Ident(ident) => {
                            let rhs = NonDefaultParamSignature::new(
                                ParamPattern::VarName(ident.name.clone()),
                                None,
                            );
                            Ok(ParamRecordAttr::new(ident, rhs))
                        }
                    })
                    .collect::<ParseResult<Vec<_>>>()?;
                let attrs = ParamRecordAttrs::new(pats);
                Ok(ParamRecordPattern::new(rec.l_brace, attrs, rec.r_brace))
            }
        }
    }

    /// Converts `(a, b, *rest)` into a tuple parameter pattern.
    ///
    /// # Errors
    ///
    /// A tuple comprehension.
    fn convert_tuple_to_param_tuple_pat(&mut self, tuple: Tuple) -> ParseResult<ParamTuplePattern> {
        trace!(self);
        match tuple {
            Tuple::Normal(tup) => {
                let mut params = vec![];
                let (elems, var_args, _, _, parens) = tup.elems.deconstruct();
                for arg in elems.into_iter() {
                    params.push(self.convert_pos_arg_to_non_default_param(arg, false)?);
                }
                let var_params = if let Some(var_args) = var_args {
                    let var_params = self.convert_pos_arg_to_non_default_param(var_args, false)?;
                    Some(var_params)
                } else {
                    None
                };
                Ok(ParamTuplePattern::new(Params::new(
                    params,
                    var_params,
                    vec![],
                    None,
                    parens,
                )))
            }
            Tuple::Comprehension(comp) => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                comp.loc(),
            )),
        }
    }

    /// Converts `pat: T` into the parameter `pat` with the type `T`.
    fn convert_type_asc_to_param_pattern(
        &mut self,
        tasc: TypeAscription,
        allow_self: bool,
    ) -> ParseResult<NonDefaultParamSignature> {
        trace!(self);
        let param = self.convert_rhs_to_param(*tasc.expr, allow_self)?;
        Ok(NonDefaultParamSignature::new(param.pat, Some(tasc.t_spec)))
    }

    /// Converts the left-hand side of `->` or `=>` into a lambda signature.
    ///
    /// A single name, literal, list or record is a one-parameter lambda, a tuple a
    /// full parameter list, and `x: T` a typed parameter. Two shapes stand for a type
    /// pattern rather than a binding: a call `C(x)` and a type expression `T or U` /
    /// `T and U`, which match the argument against the type without naming it.
    pub(crate) fn convert_rhs_to_lambda_sig(&mut self, rhs: Expr) -> ParseResult<LambdaSignature> {
        trace!(self);
        match rhs {
            Expr::Literal(lit) => {
                let param = NonDefaultParamSignature::new(ParamPattern::Lit(lit), None);
                let params = Params::single(param);
                Ok(LambdaSignature::new(params, None, TypeBoundSpecs::empty()))
            }
            Expr::Accessor(accessor) => {
                let param = self.convert_accessor_to_param_sig(accessor)?;
                let params = Params::single(param);
                Ok(LambdaSignature::new(params, None, TypeBoundSpecs::empty()))
            }
            Expr::Call(call) => {
                let param = self.convert_call_to_param_sig(call)?;
                let params = Params::single(param);
                Ok(LambdaSignature::new(params, None, TypeBoundSpecs::empty()))
            }
            Expr::Tuple(tuple) => {
                let params = self.convert_tuple_to_params(tuple)?;
                Ok(LambdaSignature::new(params, None, TypeBoundSpecs::empty()))
            }
            Expr::List(list) => {
                let lis = self.convert_list_to_param_list_pat(list)?;
                let param = NonDefaultParamSignature::new(ParamPattern::List(lis), None);
                let params = Params::single(param);
                Ok(LambdaSignature::new(params, None, TypeBoundSpecs::empty()))
            }
            Expr::Record(record) => {
                let rec = self.convert_record_to_param_record_pat(record)?;
                let param = NonDefaultParamSignature::new(ParamPattern::Record(rec), None);
                let params = Params::single(param);
                Ok(LambdaSignature::new(params, None, TypeBoundSpecs::empty()))
            }
            Expr::TypeAscription(tasc) => Ok(self.convert_type_asc_to_lambda_sig(tasc)?),
            Expr::BinOp(bin) => match bin.op.kind {
                TokenKind::OrOp | TokenKind::AndOp => {
                    let pat = ParamPattern::Discard(bin.op.clone());
                    let expr = Expr::BinOp(bin);
                    let t_spec =
                        Self::expr_to_type_spec(expr.clone()).map_err(|e| self.errs.push(e))?;
                    let t_spec = TypeSpecWithOp::new(Token::DUMMY, t_spec, expr);
                    let param = NonDefaultParamSignature::new(pat, Some(t_spec));
                    let params = Params::single(param);
                    Ok(LambdaSignature::new(params, None, TypeBoundSpecs::empty()))
                }
                _ => self.fail(ParseError::simple_syntax_error(line!() as usize, bin.loc())),
            },
            other => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                other.loc(),
            )),
        }
    }

    /// Converts a name into a parameter; `_` is the discard pattern.
    ///
    /// # Errors
    ///
    /// For any other accessor.
    fn convert_accessor_to_param_sig(
        &mut self,
        accessor: Accessor,
    ) -> ParseResult<NonDefaultParamSignature> {
        trace!(self);
        match accessor {
            Accessor::Ident(ident) => {
                let pat = if &ident.name.inspect()[..] == "_" {
                    ParamPattern::Discard(ident.name.into_token())
                } else {
                    ParamPattern::VarName(ident.name)
                };
                Ok(NonDefaultParamSignature::new(pat, None))
            }
            other => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                other.loc(),
            )),
        }
    }

    /// Converts a call written as a lambda parameter, like `List(Int)` in
    /// `List(Int) -> ...`, into a discard pattern typed with the type the call names.
    fn convert_call_to_param_sig(&mut self, call: Call) -> ParseResult<NonDefaultParamSignature> {
        let predecl = Self::call_to_predecl_type_spec(call.clone()).map_err(|_| ())?;
        let t_spec =
            TypeSpecWithOp::new(Token::DUMMY, TypeSpec::PreDeclTy(predecl), Expr::Call(call));
        Ok(NonDefaultParamSignature::new(
            ParamPattern::Discard(Token::DUMMY),
            Some(t_spec),
        ))
    }

    /// Converts a tuple written as a lambda's parameters, `(x, *xs, k := v, **kw)`,
    /// into a parameter list; the first parameter may be `self`.
    ///
    /// # Errors
    ///
    /// A tuple comprehension.
    fn convert_tuple_to_params(&mut self, tuple: Tuple) -> ParseResult<Params> {
        trace!(self);
        match tuple {
            Tuple::Normal(tup) => {
                let (pos_args, var_args, kw_args, kw_var, paren) = tup.elems.deconstruct();
                let mut params = Params::new(vec![], None, vec![], None, paren);
                for (i, arg) in pos_args.into_iter().enumerate() {
                    let param = self.convert_pos_arg_to_non_default_param(arg, i == 0)?;
                    params.non_defaults.push(param);
                }
                if let Some(var_args) = var_args {
                    let param = self.convert_pos_arg_to_non_default_param(var_args, false)?;
                    params.var_params = Some(Box::new(param));
                }
                for arg in kw_args {
                    let param = self.convert_kw_arg_to_default_param(arg)?;
                    params.defaults.push(param);
                }
                if let Some(kw_var) = kw_var {
                    let param = self.convert_pos_arg_to_non_default_param(kw_var, false)?;
                    params.kw_var_params = Some(Box::new(param));
                }
                Ok(params)
            }
            Tuple::Comprehension(comp) => self.fail(ParseError::simple_syntax_error(
                line!() as usize,
                comp.loc(),
            )),
        }
    }

    /// Converts `x: T` written as a lambda's sole parameter into its signature.
    fn convert_type_asc_to_lambda_sig(
        &mut self,
        tasc: TypeAscription,
    ) -> ParseResult<LambdaSignature> {
        trace!(self);
        let sig = self.convert_rhs_to_param(Expr::TypeAscription(tasc), true)?;
        Ok(LambdaSignature::new(
            Params::single(sig),
            None,
            TypeBoundSpecs::empty(),
        ))
    }
}
