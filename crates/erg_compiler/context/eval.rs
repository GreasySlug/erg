use std::mem;
use std::ops::Drop;
use std::sync::Arc;

use erg_common::consts::DEBUG_MODE;
use erg_common::dict::Dict;
use erg_common::error::{ErrorKind, Location};
#[allow(unused)]
use erg_common::log;
use erg_common::macros::RecursionCounter;
use erg_common::pathutil::NormalizedPathBuf;
use erg_common::set::Set;
use erg_common::shared::Shared;
use erg_common::traits::{Locational, Stream};
use erg_common::{dict, fmt_vec, fn_name, set, set_recursion_limit, Triple};
use erg_common::{ArcArray, Str};
use OpKind::*;

use erg_parser::ast::Dict as AstDict;
use erg_parser::ast::Set as AstSet;
use erg_parser::ast::*;
use erg_parser::desugar::Desugarer;
use erg_parser::token::{Token, TokenKind};

use crate::module::ConstCallKey;
use crate::ty::constructors::{
    bounded, callable, dict_t, func, guard, interval, list_t, mono, mono_q, named_free_var, poly,
    proj, proj_call, ref_, ref_mut, refinement, set_t, subr_t, subtypeof, tp_enum, try_v_enum,
    tuple_t, unknown_len_list_t, unsized_list_t, v_enum,
};
use crate::ty::free::HasLevel;
use crate::ty::typaram::{IntervalOp, OpKind, TyParam};
use crate::ty::value::{EvalValueError, GenTypeObj, TypeObj, ValueObj};
use crate::ty::{
    ConstSubr, HasType, Predicate, SubrKind, SubrType, Type, UserConstSubr, ValueArgs, Visibility,
};
use crate::{mono_type_pattern, mono_value_pattern};

use crate::context::instantiate_spec::ParamKind;
use crate::context::{ClassDefType, Context, ContextKind, RegistrationMode};
use crate::error::{EvalError, EvalErrors, EvalResult, Failable, SingleEvalResult};
use crate::varinfo::{AbsLocation, VarInfo};

use super::instantiate::TyVarCache;
use Type::{Failure, Never};

macro_rules! feature_error {
    ($ctx: expr, $loc: expr, $name: expr) => {
        $crate::feature_error!(EvalErrors, EvalError, $ctx, $loc, $name)
    };
}
macro_rules! unreachable_error {
    ($ctx: expr) => {
        $crate::unreachable_error!(EvalErrors, EvalError, $ctx)
    };
}

#[inline]
pub fn type_from_token_kind(kind: TokenKind) -> Type {
    use TokenKind::*;

    match kind {
        NatLit | BinLit | OctLit | HexLit => Type::Nat,
        IntLit => Type::Int,
        RatioLit => Type::Ratio,
        StrLit | DocComment => Type::Str,
        BoolLit => Type::Bool,
        NoneLit => Type::NoneType,
        EllipsisLit => Type::Ellipsis,
        InfLit => Type::Inf,
        other => panic!("this has no type: {other}"),
    }
}

/// The dunder method `op` dispatches to.
/// Must agree with `binop_to_dname`/`unaryop_to_dname` (`crate::error`) and with the
/// `OP_*` names the builtin classes are registered under.
fn op_to_name(op: OpKind) -> &'static str {
    match op {
        OpKind::Add => "__add__",
        OpKind::Sub => "__sub__",
        OpKind::Mul => "__mul__",
        OpKind::Div => "__div__",
        OpKind::FloorDiv => "__floordiv__",
        OpKind::Mod => "__mod__",
        OpKind::Pow => "__pow__",
        OpKind::Pos => "__pos__",
        OpKind::Neg => "__neg__",
        OpKind::Eq => "__eq__",
        OpKind::Ne => "__ne__",
        OpKind::Lt => "__lt__",
        OpKind::Le => "__le__",
        OpKind::Gt => "__gt__",
        OpKind::Ge => "__ge__",
        OpKind::As => "__as__",
        OpKind::And => "__and__",
        OpKind::Or => "__or__",
        OpKind::Not => "__not__",
        OpKind::Invert => "__invert__",
        OpKind::BitAnd => "__and__",
        OpKind::BitOr => "__or__",
        OpKind::BitXor => "__xor__",
        OpKind::Shl => "__lshift__",
        OpKind::Shr => "__rshift__",
        OpKind::ClosedRange => "__rng__",
        OpKind::LeftOpenRange => "__lorng__",
        OpKind::RightOpenRange => "__rorng__",
        OpKind::OpenRange => "__orng__",
    }
}

#[derive(Debug, Default)]
pub struct UndoableLinkedList {
    tys: Shared<Vec<Type>>, // not Set
    tps: Shared<Vec<TyParam>>,
}

impl Drop for UndoableLinkedList {
    fn drop(&mut self) {
        for t in self.tys.borrow().iter() {
            t.undo();
        }
        for tp in self.tps.borrow().iter() {
            tp.undo();
        }
    }
}

impl UndoableLinkedList {
    pub fn new() -> Self {
        Self {
            tys: Shared::new(vec![]),
            tps: Shared::new(vec![]),
        }
    }

    pub fn push_t(&self, t: &Type) {
        self.tys.borrow_mut().push(t.clone());
    }

    pub fn push_tp(&self, tp: &TyParam) {
        self.tps.borrow_mut().push(tp.clone());
    }
}

/// Substitute concrete type/type parameters to the type containing type variables and hold until dropped.
#[derive(Debug)]
pub struct Substituter<'c> {
    ctx: &'c Context,
    undoable_linked: UndoableLinkedList,
    child: Vec<Substituter<'c>>,
}

impl<'c> Substituter<'c> {
    fn new(ctx: &'c Context) -> Self {
        Self {
            ctx,
            undoable_linked: UndoableLinkedList::new(),
            child: vec![],
        }
    }

    fn with_child(mut self, child: Self) -> Self {
        self.child.push(child);
        Self {
            ctx: self.ctx,
            undoable_linked: self.undoable_linked,
            child: self.child,
        }
    }

    /// e.g.
    /// ```erg
    /// qt: List(T, N), st: List(Int, 3)
    /// qt: T or NoneType, st: NoneType or Int (T == Int)
    /// qt: {M}, st: {3}
    /// ```
    /// invalid (no effect):
    /// ```erg
    /// qt: Iterable(T), st: List(Int, 3)
    /// qt: List(T, N), st: List!(Int, 3) # TODO
    /// ```
    pub(crate) fn substitute_typarams(
        ctx: &'c Context,
        qt: &Type,
        st: &Type,
    ) -> EvalResult<Option<Self>> {
        if qt == st {
            return Ok(None);
        }
        let (mut qtps, mut stps) =
            if let Some((qtp, stp)) = qt.singleton_value().zip(st.singleton_value()) {
                (vec![qtp.clone()], vec![stp.clone()])
            } else {
                (qt.typarams(), st.typarams())
            };
        // Or, And are commutative, choose fitting order
        if qt.qual_name() == st.qual_name() {
            if st.is_union_type() || st.is_intersection_type() {
                let mut q_indices = vec![];
                let mut s_indices = vec![];
                for (i, qtp) in qtps.iter().enumerate() {
                    if let Some(j) = stps.iter().position(|stp| stp == qtp) {
                        q_indices.push(i);
                        s_indices.push(j);
                    }
                }
                for q_index in q_indices {
                    qtps[q_index] = TyParam::Failure;
                }
                for s_index in s_indices {
                    stps[s_index] = TyParam::Failure;
                }
                // REVIEW: correct condition?
                if qt != st
                    && ctx.covariant_supertype_of_tp(&qtps[0], &stps[1])
                    && ctx.covariant_supertype_of_tp(&qtps[1], &stps[0])
                {
                    stps.swap(0, 1);
                }
            }
        } else {
            // e.g. qt: Iterable(T), st: Vec(<: Iterable(Int))
            if let Some(st_sups) = ctx.get_super_types(st) {
                for sup in st_sups.skip(1) {
                    if sup.qual_name() == qt.qual_name() {
                        let mut child = Self::new(ctx);
                        let sup_tps = sup.typarams();
                        for (sup_tp, stp) in sup_tps.into_iter().zip(stps) {
                            let _ = child.substitute_typaram(sup_tp, stp);
                        }
                        if st == &sup {
                            return Ok(Some(child));
                        } else {
                            return Self::substitute_typarams(ctx, qt, &sup).map(|opt_subs| {
                                Some(match opt_subs {
                                    None => child,
                                    Some(sub) => sub.with_child(child),
                                })
                            });
                        }
                    }
                }
            }
            if let Some(inner) = st.ref_inner().or_else(|| st.ref_mut_inner()) {
                return Self::substitute_typarams(ctx, qt, &inner);
            } else if let Some(sub) = st.get_sub() {
                return Self::substitute_typarams(ctx, qt, &sub);
            }
            log!(err "{qt} / {st}");
            log!(err "[{}] [{}]", erg_common::fmt_vec(&qtps), erg_common::fmt_vec(&stps));
            return Ok(None); // TODO: e.g. Sub(Int) / Eq and Sub(?T)
        }
        let mut self_ = Self::new(ctx);
        let mut errs = EvalErrors::empty();
        for (qtp, stp) in qtps.into_iter().zip(stps) {
            if let Err(err) = self_.substitute_typaram(qtp, stp) {
                errs.extend(err);
            }
        }
        if !errs.is_empty() {
            Err(errs)
        } else {
            Ok(Some(self_))
        }
    }

    pub(crate) fn overwrite_typarams(
        ctx: &'c Context,
        qt: &Type,
        st: &Type,
    ) -> EvalResult<Option<Self>> {
        if qt == st {
            return Ok(None);
        }
        let (mut qtps, mut stps) =
            if let Some((qtp, stp)) = qt.singleton_value().zip(st.singleton_value()) {
                (vec![qtp.clone()], vec![stp.clone()])
            } else {
                (qt.typarams(), st.typarams())
            };
        if qt.qual_name() == st.qual_name() {
            if st.is_union_type() || st.is_intersection_type() {
                let mut q_indices = vec![];
                let mut s_indices = vec![];
                for (i, qtp) in qtps.iter().enumerate() {
                    if let Some(j) = stps.iter().position(|stp| stp == qtp) {
                        q_indices.push(i);
                        s_indices.push(j);
                    }
                }
                for q_index in q_indices {
                    qtps[q_index] = TyParam::Failure;
                }
                for s_index in s_indices {
                    stps[s_index] = TyParam::Failure;
                }
                // REVIEW: correct condition?
                if qt != st
                    && ctx.covariant_supertype_of_tp(&qtps[0], &stps[1])
                    && ctx.covariant_supertype_of_tp(&qtps[1], &stps[0])
                {
                    stps.swap(0, 1);
                }
            }
        } else {
            // e.g. qt: Iterable(T), st: Vec(<: Iterable(Int))
            if let Some(st_sups) = ctx.get_super_types(st) {
                for sup in st_sups.skip(1) {
                    if sup.qual_name() == qt.qual_name() {
                        let mut child = Self::new(ctx);
                        let sup_tps = sup.typarams();
                        for (sup_tp, stp) in sup_tps.into_iter().zip(stps) {
                            let _ = child.overwrite_typaram(sup_tp, stp);
                        }
                        if st == &sup {
                            return Ok(Some(child));
                        } else {
                            return Self::overwrite_typarams(ctx, qt, &sup).map(|opt_subs| {
                                Some(match opt_subs {
                                    None => child,
                                    Some(sub) => sub.with_child(child),
                                })
                            });
                        }
                    }
                }
            }
            if let Some(inner) = st.ref_inner().or_else(|| st.ref_mut_inner()) {
                return Self::overwrite_typarams(ctx, qt, &inner);
            } else if let Some(sub) = st.get_sub() {
                return Self::overwrite_typarams(ctx, qt, &sub);
            }
            log!(err "{qt} / {st}");
            log!(err "[{}] [{}]", erg_common::fmt_vec(&qtps), erg_common::fmt_vec(&stps));
            return Ok(None); // TODO: e.g. Sub(Int) / Eq and Sub(?T)
        }
        let mut self_ = Self::new(ctx);
        let mut errs = EvalErrors::empty();
        for (qtp, stp) in qtps.into_iter().zip(stps) {
            if let Err(err) = self_.overwrite_typaram(qtp, stp) {
                errs.extend(err);
            }
        }
        if !errs.is_empty() {
            Err(errs)
        } else {
            Ok(Some(self_))
        }
    }

    fn substitute_typaram(&mut self, qtp: TyParam, stp: TyParam) -> EvalResult<()> {
        match qtp {
            TyParam::FreeVar(ref fv) if fv.is_generalized() => {
                qtp.undoable_link(&stp, &self.undoable_linked);
                /*if let Err(errs) = self.sub_unify_tp(&stp, &qtp, None, &(), false) {
                    log!(err "{errs}");
                }*/
                Ok(())
            }
            TyParam::Type(qt) => self.substitute_type(*qt, stp),
            TyParam::Value(ValueObj::Type(qt)) => self.substitute_type(qt.into_typ(), stp),
            TyParam::App { name: _, args } => {
                let tps = stp.typarams();
                debug_assert_eq!(args.len(), tps.len());
                let mut errs = EvalErrors::empty();
                for (qtp, stp) in args.iter().zip(tps) {
                    if let Err(err) = self.substitute_typaram(qtp.clone(), stp) {
                        errs.extend(err);
                    }
                }
                if !errs.is_empty() {
                    Err(errs)
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    fn substitute_type(&mut self, qt: Type, stp: TyParam) -> EvalResult<()> {
        let st = self.ctx.convert_tp_into_type(stp).map_err(|tp| {
            EvalError::not_a_type_error(
                self.ctx.cfg.input.clone(),
                line!() as usize,
                ().loc(),
                self.ctx.caused_by(),
                &tp.to_string(),
            )
        })?;
        if !qt.is_undoable_linked_var() && qt.is_generalized() && qt.is_free_var() {
            qt.undoable_link(&st, &self.undoable_linked);
        } else if qt.is_undoable_linked_var() && qt != st {
            // e.g. List(T, N) <: Add(List(T, M))
            // List((Int), (3)) <: Add(List((Int), (4))): OK
            // List((Int), (3)) <: Add(List((Str), (4))): NG
            if let Some(union) = self.ctx.unify(&qt, &st) {
                qt.undoable_link(&union, &self.undoable_linked);
            } else {
                return Err(EvalError::unification_error(
                    self.ctx.cfg.input.clone(),
                    line!() as usize,
                    &qt,
                    &st,
                    ().loc(),
                    self.ctx.caused_by(),
                )
                .into());
            }
        }
        if !st.is_unbound_var() || !st.is_generalized() {
            self.child
                .extend(Self::substitute_typarams(self.ctx, &qt, &st)?);
        }
        if st.has_no_unbound_var() && qt.has_no_unbound_var() {
            return Ok(());
        }
        let qt = if qt.has_undoable_linked_var() {
            let mut tv_cache = TyVarCache::new(self.ctx.level, self.ctx);
            self.ctx.detach(qt, &mut tv_cache)
        } else {
            qt
        };
        if let Err(errs) = self
            .ctx
            .undoable_sub_unify(&st, &qt, &(), &self.undoable_linked, None)
        {
            log!(err "{errs}");
        }
        Ok(())
    }

    fn overwrite_typaram(&mut self, qtp: TyParam, stp: TyParam) -> EvalResult<()> {
        match qtp {
            TyParam::FreeVar(ref fv) if fv.is_undoable_linked() => {
                qtp.undoable_link(&stp, &self.undoable_linked);
                /*if let Err(errs) = self.sub_unify_tp(&stp, &qtp, None, &(), false) {
                    log!(err "{errs}");
                }*/
                Ok(())
            }
            // NOTE: Rarely, double overwriting occurs.
            // Whether this could be a problem is under consideration.
            // e.g. `T` of List(T, N) <: Add(T, M)
            TyParam::FreeVar(ref fv) if fv.is_generalized() => {
                qtp.undoable_link(&stp, &self.undoable_linked);
                /*if let Err(errs) = self.sub_unify_tp(&stp, &qtp, None, &(), false) {
                    log!(err "{errs}");
                }*/
                Ok(())
            }
            TyParam::Type(qt) => self.overwrite_type(*qt, stp),
            TyParam::Value(ValueObj::Type(qt)) => self.overwrite_type(qt.into_typ(), stp),
            TyParam::App { name: _, args } => {
                let tps = stp.typarams();
                debug_assert_eq!(args.len(), tps.len());
                let mut errs = EvalErrors::empty();
                for (qtp, stp) in args.iter().zip(tps) {
                    if let Err(err) = self.overwrite_typaram(qtp.clone(), stp) {
                        errs.extend(err);
                    }
                }
                if !errs.is_empty() {
                    Err(errs)
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    fn overwrite_type(&mut self, qt: Type, stp: TyParam) -> EvalResult<()> {
        let st = self.ctx.convert_tp_into_type(stp).map_err(|tp| {
            EvalError::not_a_type_error(
                self.ctx.cfg.input.clone(),
                line!() as usize,
                ().loc(),
                self.ctx.caused_by(),
                &tp.to_string(),
            )
        })?;
        if qt.is_undoable_linked_var() {
            qt.undoable_link(&st, &self.undoable_linked);
        }
        if !st.is_unbound_var() || !st.is_generalized() {
            self.child
                .extend(Self::overwrite_typarams(self.ctx, &qt, &st)?);
        }
        let qt = if qt.has_undoable_linked_var() {
            let mut tv_cache = TyVarCache::new(self.ctx.level, self.ctx);
            self.ctx.detach(qt, &mut tv_cache)
        } else {
            qt
        };
        if let Err(errs) = self
            .ctx
            .undoable_sub_unify(&st, &qt, &(), &self.undoable_linked, None)
        {
            log!(err "{errs}");
        }
        Ok(())
    }

    /// ```erg
    /// substitute_self(Iterable('Self), Int)
    /// -> Iterable(Int)
    /// ```
    pub(crate) fn substitute_self(qt: &Type, subtype: &Type, ctx: &'c Context) -> Option<Self> {
        #[allow(clippy::blocks_in_conditions)]
        for t in qt.contained_ts() {
            if t.is_qvar()
                && &t.qual_name()[..] == "Self"
                && t.get_super().is_some_and(|sup| {
                    let fv = t.as_free().unwrap();
                    fv.dummy_link();
                    let res = ctx.supertype_of(&sup, subtype);
                    fv.undo();
                    res
                })
            {
                let mut _self = Self::new(ctx);
                t.undoable_link(subtype, &_self.undoable_linked);
                return Some(_self);
            }
        }
        None
    }
}

thread_local! {
    /// How deep the current thread is inside [`Context::eval_const_lambda`]'s
    /// eager body evaluation.
    static LAMBDA_BODY_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

thread_local! {
    /// How deep compile-time evaluation is nested on this thread.
    ///
    /// This bounds the *host stack*, so it has to count every nesting that
    /// costs Rust frames: calling a const function, and evaluating an `if`
    /// branch in place. One counter, not one per site -- separate counters
    /// would each get the whole budget and together overrun the stack.
    static CONST_EVAL_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many stacks the evaluation has been handed so far. Unlike the depth,
    /// this has to be carried into each new stack by hand.
    static CONST_EVAL_STACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Context {
    fn try_get_op_kind_from_token(&self, token: &Token) -> EvalResult<OpKind> {
        match token.kind {
            TokenKind::Plus => Ok(OpKind::Add),
            TokenKind::Minus => Ok(OpKind::Sub),
            TokenKind::Star => Ok(OpKind::Mul),
            TokenKind::Slash => Ok(OpKind::Div),
            TokenKind::FloorDiv => Ok(OpKind::FloorDiv),
            TokenKind::Pow => Ok(OpKind::Pow),
            TokenKind::Mod => Ok(OpKind::Mod),
            TokenKind::DblEq => Ok(OpKind::Eq),
            TokenKind::NotEq => Ok(OpKind::Ne),
            TokenKind::Less => Ok(OpKind::Lt),
            TokenKind::Gre => Ok(OpKind::Gt),
            TokenKind::LessEq => Ok(OpKind::Le),
            TokenKind::GreEq => Ok(OpKind::Ge),
            TokenKind::AndOp => Ok(OpKind::And),
            TokenKind::OrOp => Ok(OpKind::Or),
            TokenKind::BitAnd => Ok(OpKind::BitAnd),
            TokenKind::BitXor => Ok(OpKind::BitXor),
            TokenKind::BitOr => Ok(OpKind::BitOr),
            TokenKind::Shl => Ok(OpKind::Shl),
            TokenKind::Shr => Ok(OpKind::Shr),
            TokenKind::PrePlus => Ok(OpKind::Pos),
            TokenKind::PreMinus => Ok(OpKind::Neg),
            TokenKind::PreBitNot => Ok(OpKind::Invert),
            TokenKind::Closed => Ok(OpKind::ClosedRange),
            TokenKind::LeftOpen => Ok(OpKind::LeftOpenRange),
            TokenKind::RightOpen => Ok(OpKind::RightOpenRange),
            TokenKind::Open => Ok(OpKind::OpenRange),
            _other => Err(EvalErrors::from(EvalError::not_const_expr(
                self.cfg.input.clone(),
                line!() as usize,
                token.loc(),
                self.caused_by(),
            ))),
        }
    }

    fn get_mod_ctx_from_acc(&self, acc: &Accessor) -> Option<&Context> {
        match acc {
            Accessor::Ident(ident) => self.get_mod(ident.inspect()),
            Accessor::Attr(attr) => {
                let Expr::Accessor(acc) = attr.obj.as_ref() else {
                    return None;
                };
                self.get_mod_ctx_from_acc(acc)
                    .and_then(|ctx| ctx.get_mod(attr.ident.inspect()))
            }
            _ => None,
        }
    }

    fn eval_const_acc(&self, acc: &Accessor) -> Failable<ValueObj> {
        match acc {
            Accessor::Ident(ident) => self
                .eval_const_ident(ident)
                .map_err(|err| (ValueObj::Failure, err)),
            Accessor::Attr(attr) => match self.eval_const_expr(&attr.obj) {
                Ok(obj) => Ok(self
                    .eval_attr(obj, &attr.ident)
                    .map_err(|e| (ValueObj::Failure, e.into()))?),
                Err((obj, err)) => {
                    if let Ok(attr) = self.eval_attr(obj.clone(), &attr.ident) {
                        return Err((attr, err));
                    }
                    if let Expr::Accessor(acc) = attr.obj.as_ref() {
                        if let Some(mod_ctx) = self.get_mod_ctx_from_acc(acc) {
                            if let Ok(obj) = mod_ctx.eval_const_ident(&attr.ident) {
                                return Ok(obj);
                            }
                        }
                    }
                    Err((obj, err))
                }
            },
            other => feature_error!(self, other.loc(), &format!("eval {other}"))
                .map_err(|err| (ValueObj::Failure, err)),
        }
    }

    /// `obj[index]`, i.e. `obj.__getitem__(index)`.
    ///
    /// `Tuple` and `Str` are indexed here because they have no const
    /// `__getitem__`; everything else goes through the one its class registers,
    /// so the behaviour is shared with `obj.__getitem__(index)` written out.
    fn eval_getitem(&self, obj: ValueObj, index: ValueObj, loc: Location) -> Failable<ValueObj> {
        let out_of_range = |len: usize| {
            (
                ValueObj::Failure,
                EvalErrors::from(EvalError::index_out_of_range(
                    self.cfg.input.clone(),
                    line!() as usize,
                    loc,
                    self.caused_by(),
                    len,
                    index.to_string(),
                )),
            )
        };
        match &obj {
            ValueObj::Tuple(elems) => {
                return usize::try_from(&index)
                    .ok()
                    .and_then(|i| elems.get(i).cloned())
                    .ok_or_else(|| out_of_range(elems.len()));
            }
            ValueObj::Str(s) => {
                return usize::try_from(&index)
                    .ok()
                    .and_then(|i| s.chars().nth(i))
                    .map(|c| ValueObj::Str(c.to_string().into()))
                    .ok_or_else(|| out_of_range(s.chars().count()));
            }
            _ => {}
        }
        let Some(subr) = self.get_const_op_subr(&obj.class(), "__getitem__") else {
            return Err((
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    loc,
                    self.caused_by(),
                )),
            ));
        };
        let args = ValueArgs::pos_only(vec![obj, index]);
        let tp = self
            .call(subr, args, loc)
            .map_err(|(_tp, errs)| (ValueObj::Failure, errs))?;
        self.convert_tp_into_value(tp).map_err(|_| {
            (
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    loc,
                    self.caused_by(),
                )),
            )
        })
    }

    fn get_value_from_tv_cache(&self, ident: &Identifier) -> Option<ValueObj> {
        if let Some(val) = self.tv_cache.as_ref().and_then(|tv| {
            tv.get_tyvar(ident.inspect())
                .map(|t| ValueObj::builtin_type(t.clone()))
        }) {
            Some(val)
        } else if let Some(TyParam::Value(val)) = self
            .tv_cache
            .as_ref()
            .and_then(|tv| tv.get_typaram(ident.inspect()))
        {
            Some(val.clone())
        } else {
            // The type variables of a polymorphic type definition must be visible
            // in the inner scopes (e.g. `T` in `Wrapper|T| = Class {value = T}`
            // is evaluated in the ctx of the requirement record `{value = T}`)
            self.get_outer_scope()
                .and_then(|outer| outer.get_value_from_tv_cache(ident))
        }
    }

    fn eval_const_ident(&self, ident: &Identifier) -> EvalResult<ValueObj> {
        if let Some(val) = self.get_value_from_tv_cache(ident) {
            Ok(val)
        } else if let Some(val) = self.rec_get_const_obj(ident.inspect()) {
            Ok(val.clone())
        } else if self.kind.is_subr() {
            feature_error!(self, ident.loc(), "const parameters")
        } else if ident.is_const() {
            Err(EvalErrors::from(EvalError::no_var_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
                ident.inspect(),
                self.get_similar_name(ident.inspect()),
            )))
        } else {
            Err(EvalErrors::from(EvalError::not_const_expr(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
            )))
        }
    }

    fn eval_attr(&self, obj: ValueObj, ident: &Identifier) -> SingleEvalResult<ValueObj> {
        self.eval_attr_and_recv(obj, ident).map(|(val, _)| val)
    }

    /// An attribute, and whether the receiver is its first argument.
    ///
    /// `"abc".replace("a", "z")` calls `Str.replace(self, ...)`, so the receiver
    /// is an argument. `C.Twice(3)` is not: `Twice` is a constant *of* the class,
    /// found on the class rather than taken by it, and passing the class along
    /// bound it to `Twice`'s first parameter.
    fn eval_attr_and_recv(
        &self,
        obj: ValueObj,
        ident: &Identifier,
    ) -> SingleEvalResult<(ValueObj, bool)> {
        let field = self
            .instantiate_field(ident)
            .map_err(|(_, mut errs)| errs.remove(0))?;
        if let Some(val) = obj.try_get_attr(&field) {
            return Ok((val, false));
        }
        if let ValueObj::Type(t) = &obj {
            if let Some(sups) = self.get_nominal_super_type_ctxs(t.typ()) {
                for ctx in sups {
                    if let Some(val) = ctx.consts.get(ident.inspect()) {
                        return Ok((val.clone(), false));
                    }
                    for methods in ctx.methods_list.iter() {
                        if let Some(v) = methods.consts.get(ident.inspect()) {
                            return Ok((v.clone(), false));
                        }
                    }
                }
            }
            if let Some(val) = self.const_of_enclosing_methods(t.typ(), ident.inspect()) {
                return Ok((val, false));
            }
        }
        // A const method called on a value (`"abc".replace("a", "z")`) rather than
        // on the type. This is the same lookup the binary operators do, so every
        // const method registered on a builtin class is reachable both ways.
        if let Some(subr) = self.get_const_op_subr(&obj.class(), ident.inspect()) {
            return Ok((ValueObj::Subr(subr), true));
        }
        Err(EvalError::no_attr_error(
            self.cfg.input.clone(),
            line!() as usize,
            ident.loc(),
            self.caused_by(),
            &obj.t(),
            ident.inspect(),
            None,
        ))
    }

    /// A constant the methods block of `typ` has registered so far, when this
    /// is (in) that block: `C.Tripled = C.Base * 3` a few lines below `Base`.
    /// The block's context goes onto the class only when the block ends, so
    /// until then its constants are found in the scope itself.
    fn const_of_enclosing_methods(&self, typ: &Type, name: &str) -> Option<ValueObj> {
        let local = typ.local_name();
        let mut ctx = Some(self);
        while let Some(c) = ctx {
            let of_typ = match &c.kind {
                ContextKind::MethodDefs {
                    class: Some(class), ..
                } => class.local_name() == local,
                // a monomorphic class is not recorded on the kind; the block
                // is named after it (`grow`)
                ContextKind::MethodDefs { class: None, .. } => {
                    c.name.rsplit(['.', ':']).next() == Some(&local[..])
                }
                _ => false,
            };
            if of_typ {
                if let Some(val) = c.consts.get(name) {
                    return Some(val.clone());
                }
            }
            ctx = c.get_outer();
        }
        None
    }

    fn eval_const_bin(&self, bin: &BinOp) -> Failable<ValueObj> {
        let lhs = self.eval_const_expr(&bin.args[0])?;
        let rhs = self.eval_const_expr(&bin.args[1])?;
        if bin.op.is(TokenKind::ContainsOp) {
            return self
                .eval_contains(lhs, rhs, bin.op.loc())
                .map_err(|e| (ValueObj::Failure, e));
        }
        let op = self
            .try_get_op_kind_from_token(&bin.op)
            .map_err(|e| (ValueObj::Failure, e))?;
        self.eval_bin(op, lhs, rhs, bin.op.loc())
            .map_err(|e| (ValueObj::Failure, e))
    }

    fn eval_const_unary(&self, unary: &UnaryOp) -> Failable<ValueObj> {
        let val = self.eval_const_expr(&unary.args[0])?;
        let op = self
            .try_get_op_kind_from_token(&unary.op)
            .map_err(|e| (ValueObj::Failure, e))?;
        self.eval_unary_val(op, val, unary.op.loc())
            .map_err(|e| (ValueObj::Failure, e))
    }

    fn eval_args(&self, args: &Args) -> Failable<ValueArgs> {
        let mut errs = EvalErrors::empty();
        let mut evaluated_pos_args = vec![];
        for arg in args.pos_args().iter() {
            match self.eval_const_expr(&arg.expr) {
                Ok(val) => evaluated_pos_args.push(val),
                Err((val, es)) => {
                    evaluated_pos_args.push(val);
                    errs.extend(es);
                }
            }
        }
        let mut evaluated_kw_args = dict! {};
        for arg in args.kw_args().iter() {
            match self.eval_const_expr(&arg.expr) {
                Ok(val) => {
                    evaluated_kw_args.insert(arg.keyword.inspect().clone(), val);
                }
                Err((val, es)) => {
                    evaluated_kw_args.insert(arg.keyword.inspect().clone(), val);
                    errs.extend(es);
                }
            }
        }
        let args = ValueArgs::new(evaluated_pos_args, evaluated_kw_args);
        if errs.is_empty() {
            Ok(args)
        } else {
            Err((args, errs))
        }
    }

    fn eval_const_call(&self, call: &Call) -> Failable<ValueObj> {
        let (tp, errs) = match self.tp_eval_const_call(call) {
            Ok(tp) => (tp, EvalErrors::empty()),
            Err((tp, errs)) => (tp, errs),
        };
        match self.convert_tp_into_value(tp) {
            Ok(val) => {
                if errs.is_empty() {
                    Ok(val)
                } else {
                    Err((val, errs))
                }
            }
            Err(_) => {
                if errs.is_empty() {
                    Err((
                        ValueObj::Failure,
                        EvalErrors::from(EvalError::not_const_expr(
                            self.cfg.input.clone(),
                            line!() as usize,
                            call.loc(),
                            self.caused_by(),
                        )),
                    ))
                } else {
                    Err((ValueObj::Failure, errs))
                }
            }
        }
    }

    fn tp_eval_const_call(&self, call: &Call) -> Failable<TyParam> {
        if let Some(attr) = &call.attr_name {
            let res_obj = self.eval_const_expr(&call.obj);
            let mut is_method = true;
            let callee = match res_obj.clone() {
                // The desugarer rewrites `x[i]` into `x.__getitem__(i)` and `x.0`
                // into `x.__Tuple_getitem__(0)`. `Tuple` and `Str` declare those as
                // typed methods rather than const subroutines, so index them here.
                Ok(obj @ (ValueObj::Tuple(_) | ValueObj::Str(_)))
                    if matches!(&attr.inspect()[..], "__getitem__" | "__Tuple_getitem__") =>
                {
                    let (args, errs) = match self.eval_args(&call.args) {
                        Ok(args) => (args, EvalErrors::empty()),
                        Err((args, es)) => (args, es),
                    };
                    let Some(index) = args.pos_args.into_iter().next() else {
                        return Err((TyParam::Failure, errs));
                    };
                    let val = self
                        .eval_getitem(obj, index, call.loc())
                        .map_err(|(val, es)| (TyParam::Value(val), es))?;
                    return if errs.is_empty() {
                        Ok(TyParam::Value(val))
                    } else {
                        Err((TyParam::Value(val), errs))
                    };
                }
                Ok(obj) => {
                    let (callee, recv_is_arg) = self
                        .eval_attr_and_recv(obj, attr)
                        .map_err(|err| (TyParam::Failure, err.into()))?;
                    is_method = recv_is_arg;
                    callee
                }
                Err((_val, errs)) => {
                    let acc = Accessor::attr(*call.obj.clone(), attr.clone());
                    self.eval_const_acc(&acc)
                        .inspect(|_| is_method = false)
                        .map_err(|(_val, _errs)| (TyParam::Failure, errs))?
                }
            };
            let ValueObj::Subr(subr) = callee else {
                return Err((
                    TyParam::Failure,
                    EvalError::type_mismatch_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        Location::concat(call.obj.as_ref(), attr),
                        self.caused_by(),
                        attr.inspect(),
                        None,
                        &mono("Subroutine"),
                        &callee.t(),
                        self.get_candidates(&callee.t()),
                        None,
                    )
                    .into(),
                ));
            };
            let (mut args, mut errs) = match self.eval_args(&call.args) {
                Ok(args) => (args, EvalErrors::empty()),
                Err((args, es)) => (args, es),
            };
            let obj = match res_obj {
                Ok(obj) => obj,
                Err((obj, es)) => {
                    if is_method {
                        errs.extend(es);
                    }
                    obj
                }
            };
            if is_method {
                args.pos_args.insert(0, obj);
            }
            let tp = match self.call(subr.clone(), args, call.loc()) {
                Ok(tp) => tp,
                Err((tp, es)) => {
                    errs.extend(es);
                    tp
                }
            };
            if errs.is_empty() {
                Ok(tp)
            } else {
                Err((tp, errs))
            }
        } else {
            // `match` is a special form rather than a name: lowering gives it a
            // magic type and `get_call_t` intercepts it, so there is nothing to
            // look up here either. `match!` is a procedure and is not folded.
            if let Some(ident) = call.obj.as_ref().as_ident() {
                if ident.vis.is_private() && &ident.inspect()[..] == "match" {
                    return self
                        .eval_const_match(call)
                        .map(TyParam::Value)
                        .map_err(|(val, errs)| (TyParam::value(val), errs));
                }
            }
            let callee = match call.obj.as_ref() {
                Expr::Accessor(acc) => self
                    .eval_const_acc(acc)
                    .map_err(|(val, errs)| (TyParam::value(val), errs)),
                other => Err((
                    TyParam::Failure,
                    EvalErrors::from(EvalError::feature_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        other.loc(),
                        &format!("const call: {other}"),
                        self.caused_by(),
                    )),
                )),
            }?;
            // TODO: __call__
            let ValueObj::Subr(subr) = callee else {
                return Err((
                    TyParam::Failure,
                    EvalError::type_mismatch_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        call.obj.loc(),
                        self.caused_by(),
                        &call.obj.full_name().unwrap_or("_".into()),
                        None,
                        &mono("Subroutine"),
                        &callee.t(),
                        self.get_candidates(&callee.t()),
                        None,
                    )
                    .into(),
                ));
            };
            let (args, mut errs) = match self.eval_args(&call.args) {
                Ok(args) => (args, EvalErrors::empty()),
                Err((args, es)) => (args, es),
            };
            let tp = match self.call(subr.clone(), args, call.loc()) {
                Ok(tp) => tp,
                Err((tp, es)) => {
                    errs.extend(es);
                    tp
                }
            };
            if errs.is_empty() {
                Ok(tp)
            } else {
                Err((tp, errs))
            }
        }
    }

    /// Assume that `args` has already been evaluated.
    /// Projection types may remain, but how they are handled varies by each const function.
    /// For example, `list_union` returns `Type::Obj` if it contains a projection type.
    pub(crate) fn call(
        &self,
        subr: ConstSubr,
        args: ValueArgs,
        loc: impl Locational,
    ) -> Failable<TyParam> {
        match subr {
            ConstSubr::User(user) => {
                let key = self.const_call_key(&user, &args);
                if let Some(cached) = key
                    .as_ref()
                    .and_then(|key| self.shared.as_ref()?.const_calls.get(key))
                {
                    return Ok(TyParam::Value(cached));
                }
                // This stack is nearly spent; give the rest of the evaluation a
                // fresh one rather than let how deep a const function may
                // recurse be decided by how much stack happens to be left.
                if CONST_EVAL_DEPTH.with(|d| d.get()) >= erg_common::spawn::CONST_CALL_LIMIT {
                    let loc = loc.loc();
                    return self
                        .on_new_stack(loc, move || self.call(ConstSubr::User(user), args, loc));
                }
                // Guard against infinitely recursive constant evaluation,
                // e.g. `F n = F n; X = F 1` (self- or mutually-recursive const functions)
                let _counter =
                    RecursionCounter::new(&CONST_EVAL_DEPTH, erg_common::spawn::CONST_CALL_LIMIT);
                let mut errs = EvalErrors::empty();
                // HACK: should avoid cloning
                let def_ctx = self.const_def_ctx(&user);
                // Name the frame inside the scope the subroutine was defined in.
                // A bare name is not a namespace, and several lookups ask whether
                // the current scope is inside another one -- `get_mono_type` does,
                // so a frame called `F` was inside nothing and no class of the
                // module was reachable from a const function's body.
                let scope = match user.def_scope.as_ref() {
                    Some((_, scope)) => scope.clone(),
                    None => def_ctx.map_or_else(|| self.name.clone(), |ctx| ctx.name.clone()),
                };
                // What a frame reads is a copy of the defining scope. Frames of
                // one call tree read the *same* copy: a recursion `N` deep used
                // to copy the module `N` times, which is most of what a const
                // call costs and all of what it costs in memory.
                let parent = self
                    .outer
                    .as_ref()
                    .filter(|outer| {
                        def_ctx.is_some_and(|def| {
                            outer.name == def.name && outer.module_path() == def.module_path()
                        })
                    })
                    .cloned()
                    .unwrap_or_else(|| Arc::new(def_ctx.cloned().unwrap_or_else(|| self.clone())));
                let mut subr_ctx = Context::instant_in(
                    Str::from(format!("{scope}::{}", user.name)),
                    def_ctx.map_or_else(|| self.cfg.clone(), |ctx| ctx.cfg.clone()),
                    2,
                    self.shared.clone(),
                    parent,
                );
                let mut pos_args = args.pos_args.into_iter();
                let mut kw_args = args.kw_args;
                for sig in user.params.non_defaults.iter() {
                    let Some(symbol) = sig.inspect() else {
                        let _ = pos_args.next();
                        errs.push(EvalError::feature_error(
                            self.cfg.input.clone(),
                            line!() as usize,
                            loc.loc(),
                            "_",
                            self.caused_by(),
                        ));
                        continue;
                    };
                    let val = pos_args.next().or_else(|| kw_args.remove(symbol));
                    if let Some(arg) = val {
                        subr_ctx
                            .consts
                            .insert(VarName::from_str(symbol.clone()), arg);
                    }
                }
                let var_params = user.params.var_params.as_ref();
                if let Some(var) = var_params {
                    let rest: Vec<_> = pos_args.by_ref().collect();
                    if let Some(symbol) = var.inspect() {
                        subr_ctx
                            .consts
                            .insert(VarName::from_str(symbol.clone()), ValueObj::from(rest));
                    }
                }
                for default in user.params.defaults.iter() {
                    let Some(symbol) = default.inspect() else {
                        continue;
                    };
                    let val = if var_params.is_none() {
                        pos_args.next()
                    } else {
                        None
                    }
                    .or_else(|| kw_args.remove(symbol));
                    let val = match val {
                        Some(v) => v,
                        None => match subr_ctx.eval_const_expr(&default.default_val) {
                            Ok(v) => v,
                            Err((v, es)) => {
                                errs.extend(es);
                                v
                            }
                        },
                    };
                    subr_ctx
                        .consts
                        .insert(VarName::from_str(symbol.clone()), val);
                }
                for (name, arg) in kw_args {
                    subr_ctx.consts.insert(VarName::from_str(name), arg);
                }
                let tp = match subr_ctx.eval_const_block(&user.block()) {
                    Ok(val) => TyParam::Value(val),
                    Err((val, es)) => {
                        errs.extend(es);
                        TyParam::value(val)
                    }
                };
                if errs.is_empty() {
                    // only a call that produced a value: an error depends on the
                    // diagnostics around it, not on the arguments alone
                    if let (Some(key), Some(shared), TyParam::Value(val)) =
                        (key, self.shared.as_ref(), &tp)
                    {
                        shared.const_calls.insert(key, val.clone());
                    }
                    Ok(tp)
                } else {
                    Err((tp, errs))
                }
            }
            ConstSubr::Builtin(builtin) => builtin
                .call(args, self)
                .map_err(|e| self.const_call_error(e, loc.loc())),
            ConstSubr::Gen(gen) => gen
                .call(args, self)
                .map_err(|e| self.const_call_error(e, loc.loc())),
        }
    }

    /// Continue evaluating on a stack of its own.
    ///
    /// Compile-time evaluation recurses on the host stack, so without this the
    /// depth a const function may reach is whatever fits in one stack -- 31
    /// levels of what the user wrote, against CPython's 1000. Each stack is
    /// worth [`erg_common::spawn::CONST_CALL_LIMIT`] of the levels counted by
    /// `CONST_EVAL_DEPTH` and there may be
    /// [`erg_common::spawn::CONST_CALL_STACKS`] of them, which together are
    /// what bounds a runaway recursion.
    fn on_new_stack<T: Send + Default>(
        &self,
        loc: Location,
        f: impl FnOnce() -> Failable<T> + Send,
    ) -> Failable<T> {
        let used = CONST_EVAL_STACKS.with(|s| s.get());
        if used >= erg_common::spawn::CONST_CALL_STACKS {
            return Err((
                T::default(),
                EvalErrors::from(EvalError::recursion_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    loc,
                    self.caused_by(),
                )),
            ));
        }
        erg_common::spawn::run_on_new_stack(move || {
            // a fresh thread starts with a fresh depth; the number of stacks is
            // what carries over
            CONST_EVAL_STACKS.with(|s| s.set(used + 1));
            f()
        })
    }

    /// The scope a const subroutine's body resolves its free names in: the one
    /// it was written in, not the one calling it.
    ///
    /// `None` means the caller's own chain is already the right one -- either
    /// the definition lives in the module being evaluated, or it is synthetic
    /// (a lambda, an `if` branch), which really does read what surrounds the
    /// call. Otherwise the body would look its own module's names up in the
    /// importer's scope and not find them.
    fn const_def_ctx(&self, user: &UserConstSubr) -> Option<&Context> {
        let (module, _scope) = user.def_scope.as_ref()?;
        // The module being lowered is not in the cache yet -- it is the chain
        // `self` sits in. Take the module scope itself and not `self`: `self`
        // may be another frame of the same recursion, and hanging each frame
        // off the last makes the chain grow with the depth, so a call at depth
        // N copies N scopes.
        if module == &NormalizedPathBuf::from(self.module_path()) {
            return self.get_module();
        }
        let shared = self.shared.as_ref()?;
        shared
            .mod_cache
            .raw_ref_ctx(module)
            .or_else(|| shared.py_mod_cache.raw_ref_ctx(module))
            .map(|mod_ctx| &mod_ctx.context)
    }

    /// What identifies this call for [`SharedConstCallCache`], or `None` when it
    /// must not be cached.
    ///
    /// A const function is pure and a const name cannot be shadowed within a
    /// scope, so the scope it was defined in (baked into the subroutine at
    /// registration -- the *evaluating* module may be a different one that
    /// merely imported it), its name and the arguments decide the result --
    /// with one exception: a quantified signature means the body can read type
    /// variables bound by the caller, which are not part of the arguments.
    /// A synthetic subroutine (a lambda, an `if` branch) has no `def_scope`
    /// and is never cached: it reads the scope it was written in.
    fn const_call_key(&self, user: &UserConstSubr, args: &ValueArgs) -> Option<ConstCallKey> {
        // not only `Quantified`: a method of a polymorphic class can mention the
        // class's type variable without being quantified itself, and its result
        // then depends on an instantiation the arguments do not carry
        if matches!(user.sig_t, Type::Quantified(_)) || user.sig_t.has_qvar() {
            return None;
        }
        let (module, scope) = user.def_scope.clone()?;
        let mut kw_args = args
            .kw_args
            .iter()
            .map(|(name, val)| (name.clone(), val.clone()))
            .collect::<Vec<_>>();
        // `Dict` iterates in hash order, which is not the same twice
        kw_args.sort_by(|(l, _), (r, _)| l.cmp(r));
        Some(ConstCallKey {
            module,
            scope,
            name: user.name.clone(),
            pos_args: args.pos_args.clone(),
            kw_args,
        })
    }

    /// A const function's failure, placed at `loc`.
    ///
    /// A call that merely could not be folded gets the diagnostic that says so,
    /// with the way out; every context that does not need a constant has already
    /// dropped it by the time this runs.
    fn const_call_error(&self, mut e: EvalValueError, loc: Location) -> (TyParam, EvalErrors) {
        if e.core.loc.is_unknown() {
            e.core.loc = loc;
        }
        let err = if e.not_foldable {
            EvalError::unfoldable_call(
                self.cfg.input.clone(),
                line!() as usize,
                e.core.loc,
                self.caused_by(),
                e.core.main_message.clone(),
            )
        } else {
            EvalError::new(*e.core, self.cfg.input.clone(), self.caused_by())
        };
        (TyParam::Failure, EvalErrors::from(err))
    }

    fn eval_const_def(&mut self, def: &Def) -> Failable<ValueObj> {
        // A definition in a scope that is being evaluated -- the frame of a
        // const function, a block being folded -- binds a compile-time value
        // whatever its name: `(a, b) = t` desugars to `%v_desugar_1 = t;
        // a = %v_desugar_1[0]; ...`, and `m = n + 1` is a local like any
        // other. Only in a module or type body is a lowercase name a run-time
        // variable, which no constant can be made of.
        if def.is_const() || !(self.kind.is_module() || self.kind.is_type()) {
            let mut errs = EvalErrors::empty();
            let Some(ident) = def.sig.ident() else {
                return Err((
                    ValueObj::None,
                    EvalErrors::from(EvalError::feature_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        def.sig.loc(),
                        "complex pattern const-def",
                        self.caused_by(),
                    )),
                ));
            };
            let __name__ = ident.inspect();
            // A subroutine definition defines a subroutine. Evaluating the body
            // here would run it with its own parameters unbound, which is what
            // a definition inside a const function's body used to do -- even one
            // that captured nothing failed with a `NameError` on its parameter.
            //
            // The subroutine gets no `def_scope` (a body scope is re-entered with
            // different surroundings, so it cannot identify a definition), and
            // that is exactly what makes it read the frame it is called from:
            // `Context::call` falls back to the caller when there is no defining
            // scope to look up. Since the name cannot leave the body, the frame
            // that defined it is always still on the chain.
            if let Signature::Subr(subr) = &def.sig {
                let obj = match self.register_const_subr(subr, &def.body.block, def.def_kind()) {
                    Ok(obj) => obj,
                    Err((obj, es)) => {
                        errs.extend(es);
                        obj
                    }
                };
                if let Err(es) =
                    self.register_gen_const(ident, obj, None, def.def_kind().is_other())
                {
                    errs.extend(es);
                }
                return if errs.is_empty() {
                    Ok(ValueObj::None)
                } else {
                    Err((ValueObj::None, errs))
                };
            }
            let vis = self
                .instantiate_vis_modifier(def.sig.vis())
                .map_err(|es| (ValueObj::None, es))?;
            let tv_cache = match &def.sig {
                Signature::Subr(subr) => {
                    let ty_cache =
                        match self.instantiate_ty_bounds(&subr.bounds, RegistrationMode::Normal) {
                            Ok(ty_cache) => ty_cache,
                            Err((ty_cache, es)) => {
                                errs.extend(es);
                                ty_cache
                            }
                        };
                    Some(ty_cache)
                }
                Signature::Var(_) => None,
            };
            // TODO: set params
            let kind = ContextKind::from(def);
            self.grow(__name__, kind, vis, tv_cache);
            let obj = self.eval_const_block(&def.body.block).inspect_err(|_| {
                self.pop();
            })?;
            let call = if let Some(Expr::Call(call)) = &def.body.block.first() {
                Some(call)
            } else {
                None
            };
            let (_ctx, es) = self.check_decls_and_pop();
            errs.extend(es);
            if let Err(es) = self.register_gen_const(ident, obj, call, def.def_kind().is_other()) {
                errs.extend(es);
            }
            if errs.is_empty() {
                Ok(ValueObj::None)
            } else {
                Err((ValueObj::None, errs))
            }
        } else {
            Err((
                ValueObj::None,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    def.body.block.loc(),
                    self.caused_by(),
                )),
            ))
        }
    }

    pub(crate) fn eval_const_normal_list(&self, lis: &NormalList) -> Failable<ValueObj> {
        let mut errs = EvalErrors::empty();
        let mut elems = vec![];
        for elem in lis.elems.pos_args().iter() {
            match self.eval_const_expr(&elem.expr) {
                Ok(val) => elems.push(val),
                Err((val, es)) => {
                    elems.push(val);
                    errs.extend(es);
                }
            }
        }
        let list = ValueObj::List(ArcArray::from(elems));
        if errs.is_empty() {
            Ok(list)
        } else {
            Err((list, errs))
        }
    }

    fn eval_const_list(&self, lis: &List) -> Failable<ValueObj> {
        match lis {
            List::Normal(lis) => self.eval_const_normal_list(lis),
            List::WithLength(lis) => {
                let mut errs = EvalErrors::empty();
                let elem = match self.eval_const_expr(&lis.elem.expr) {
                    Ok(val) => val,
                    Err((val, es)) => {
                        errs.extend(es);
                        val
                    }
                };
                let list = match lis.len.as_ref() {
                    Expr::Accessor(Accessor::Ident(ident)) if ident.is_discarded() => {
                        ValueObj::UnsizedList(Box::new(elem))
                    }
                    other => {
                        let len = match self.eval_const_expr(other) {
                            Ok(val) => val,
                            Err((val, es)) => {
                                errs.extend(es);
                                val
                            }
                        };
                        let len = match usize::try_from(&len) {
                            Ok(len) => len,
                            Err(_) => {
                                errs.push(EvalError::type_mismatch_error(
                                    self.cfg.input.clone(),
                                    line!() as usize,
                                    other.loc(),
                                    self.caused_by(),
                                    "_",
                                    None,
                                    &Type::Nat,
                                    &len.t(),
                                    self.get_candidates(&len.t()),
                                    None,
                                ));
                                0
                            }
                        };
                        let arr = vec![elem; len];
                        ValueObj::List(ArcArray::from(arr))
                    }
                };
                if errs.is_empty() {
                    Ok(list)
                } else {
                    Err((list, errs))
                }
            }
            _ => Err((
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    lis.loc(),
                    self.caused_by(),
                )),
            )),
        }
    }

    fn eval_const_set(&self, set: &AstSet) -> Failable<ValueObj> {
        let mut errs = EvalErrors::empty();
        let mut elems = vec![];
        match set {
            AstSet::Normal(lis) => {
                for elem in lis.elems.pos_args().iter() {
                    match self.eval_const_expr(&elem.expr) {
                        Ok(val) => elems.push(val),
                        Err((val, es)) => {
                            elems.push(val);
                            errs.extend(es);
                        }
                    }
                }
                let set = ValueObj::Set(Set::from(elems));
                if errs.is_empty() {
                    Ok(set)
                } else {
                    Err((set, errs))
                }
            }
            // set-builder notation: `Odd = {N: Int | N % 2 == 1}` is a type,
            // the same one a type spec in that position would give
            AstSet::Refinement(refine) => {
                let expr = Expr::Set(AstSet::Refinement(refine.clone()));
                match self.expr_to_type(expr) {
                    Ok(t) => Ok(ValueObj::builtin_type(t)),
                    Err((t, es)) => Err((ValueObj::builtin_type(t), es)),
                }
            }
            _ => Err((
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    set.loc(),
                    self.caused_by(),
                )),
            )),
        }
    }

    fn eval_const_dict(&self, dict: &AstDict) -> Failable<ValueObj> {
        let mut errs = EvalErrors::empty();
        let mut elems = dict! {};
        match dict {
            AstDict::Normal(dic) => {
                for elem in dic.kvs.iter() {
                    match (
                        self.eval_const_expr(&elem.key),
                        self.eval_const_expr(&elem.value),
                    ) {
                        (Ok(key), Ok(value)) => {
                            elems.insert(key, value);
                        }
                        (Ok(key), Err((value, es))) => {
                            elems.insert(key, value);
                            errs.extend(es);
                        }
                        (Err((key, es)), Ok(value)) => {
                            elems.insert(key, value);
                            errs.extend(es);
                        }
                        (Err((key, es1)), Err((value, es2))) => {
                            elems.insert(key, value);
                            errs.extend(es1);
                            errs.extend(es2);
                        }
                    }
                }
                let dict = ValueObj::Dict(elems);
                if errs.is_empty() {
                    Ok(dict)
                } else {
                    Err((dict, errs))
                }
            }
            _ => Err((
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    dict.loc(),
                    self.caused_by(),
                )),
            )),
        }
    }

    fn eval_const_tuple(&self, tuple: &Tuple) -> Failable<ValueObj> {
        let mut errs = EvalErrors::empty();
        let mut elems = vec![];
        match tuple {
            Tuple::Normal(lis) => {
                for elem in lis.elems.pos_args().iter() {
                    let elem = match self.eval_const_expr(&elem.expr) {
                        Ok(val) => val,
                        Err((val, es)) => {
                            errs.extend(es);
                            val
                        }
                    };
                    elems.push(elem);
                }
            }
            _ => {
                return Err((
                    ValueObj::Failure,
                    EvalErrors::from(EvalError::not_const_expr(
                        self.cfg.input.clone(),
                        line!() as usize,
                        tuple.loc(),
                        self.caused_by(),
                    )),
                ));
            }
        }
        let tuple = ValueObj::Tuple(ArcArray::from(elems));
        if errs.is_empty() {
            Ok(tuple)
        } else {
            Err((tuple, errs))
        }
    }

    fn eval_const_record(&self, record: &Record) -> Failable<ValueObj> {
        match record {
            Record::Normal(rec) => self.eval_const_normal_record(rec),
            Record::Mixed(mixed) => self.eval_const_normal_record(
                &Desugarer::desugar_shortened_record_inner(mixed.clone()),
            ),
        }
    }

    fn eval_const_normal_record(&self, record: &NormalRecord) -> Failable<ValueObj> {
        let mut errs = EvalErrors::empty();
        let mut attrs = vec![];
        // HACK: should avoid cloning
        let mut record_ctx = Context::instant(
            Str::ever("<unnamed record>"),
            self.cfg.clone(),
            2,
            self.shared.clone(),
            self.clone(),
        );
        for attr in record.attrs.iter() {
            // let name = attr.sig.ident().map(|i| i.inspect());
            let elem = match record_ctx.eval_const_block(&attr.body.block) {
                Ok(val) => val,
                Err((val, es)) => {
                    errs.extend(es);
                    val
                }
            };
            let ident = match &attr.sig {
                Signature::Var(var) => match &var.pat {
                    VarPattern::Ident(ident) => match record_ctx.instantiate_field(ident) {
                        Ok(field) => field,
                        Err((field, es)) => {
                            errs.extend(es);
                            field
                        }
                    },
                    other => {
                        let err = EvalError::feature_error(
                            self.cfg.input.clone(),
                            line!() as usize,
                            other.loc(),
                            &format!("record field: {other}"),
                            self.caused_by(),
                        );
                        errs.push(err);
                        continue;
                    }
                },
                other => {
                    let err = EvalError::feature_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        other.loc(),
                        &format!("record field: {other}"),
                        self.caused_by(),
                    );
                    errs.push(err);
                    continue;
                }
            };
            let name = VarName::from_str(ident.symbol.clone());
            // T = Trait { .Output = Type; ... }
            // -> .Output = Self(<: T).Output
            if self.kind.is_trait() && self.convert_value_into_type(elem.clone()).is_ok() {
                let slf = mono_q("Self", subtypeof(mono(self.name.clone())));
                let t = ValueObj::builtin_type(slf.proj(ident.symbol.clone()));
                record_ctx.consts.insert(name.clone(), t);
            } else {
                record_ctx.consts.insert(name.clone(), elem.clone());
            }
            let t = v_enum(set! { elem.clone() });
            let vis = match record_ctx.instantiate_vis_modifier(attr.sig.vis()) {
                Ok(vis) => vis,
                Err(es) => {
                    errs.extend(es);
                    continue;
                }
            };
            let vis = Visibility::new(vis, record_ctx.name.clone());
            let vi = VarInfo::record_field(t, record_ctx.absolutize(attr.sig.loc()), vis);
            record_ctx.locals.insert(name, vi);
            attrs.push((ident, elem));
        }
        let rec = ValueObj::Record(attrs.into_iter().collect());
        if errs.is_empty() {
            Ok(rec)
        } else {
            Err((rec, errs))
        }
    }

    /// FIXME: grow
    fn eval_const_lambda(&self, lambda: &Lambda) -> Failable<ValueObj> {
        let mut errs = EvalErrors::empty();
        let mut tmp_tv_cache =
            match self.instantiate_ty_bounds(&lambda.sig.bounds, RegistrationMode::Normal) {
                Ok(ty_cache) => ty_cache,
                Err((ty_cache, es)) => {
                    errs.extend(es);
                    ty_cache
                }
            };
        let mut non_default_params = Vec::with_capacity(lambda.sig.params.non_defaults.len());
        for sig in lambda.sig.params.non_defaults.iter() {
            match self.instantiate_param_ty(
                sig,
                None,
                &mut tmp_tv_cache,
                RegistrationMode::Normal,
                ParamKind::NonDefault,
                false,
            ) {
                Ok(pt) => non_default_params.push(pt),
                Err((pt, err)) => {
                    non_default_params.push(pt);
                    errs.extend(err)
                }
            }
        }
        let var_params = if let Some(p) = lambda.sig.params.var_params.as_ref() {
            match self.instantiate_param_ty(
                p,
                None,
                &mut tmp_tv_cache,
                RegistrationMode::Normal,
                ParamKind::VarParams,
                false,
            ) {
                Ok(pt) => Some(pt),
                Err((pt, err)) => {
                    errs.extend(err);
                    Some(pt)
                }
            }
        } else {
            None
        };
        let mut default_params = Vec::with_capacity(lambda.sig.params.defaults.len());
        for sig in lambda.sig.params.defaults.iter() {
            let expr = self.eval_const_expr(&sig.default_val)?;
            match self.instantiate_param_ty(
                &sig.sig,
                None,
                &mut tmp_tv_cache,
                RegistrationMode::Normal,
                ParamKind::Default(expr.t()),
                false,
            ) {
                Ok(pt) => default_params.push(pt),
                Err((pt, err)) => {
                    errs.extend(err);
                    default_params.push(pt)
                }
            }
        }
        let kw_var_params = if let Some(p) = lambda.sig.params.kw_var_params.as_ref() {
            match self.instantiate_param_ty(
                p,
                None,
                &mut tmp_tv_cache,
                RegistrationMode::Normal,
                ParamKind::KwVarParams,
                false,
            ) {
                Ok(pt) => Some(pt),
                Err((pt, err)) => {
                    errs.extend(err);
                    Some(pt)
                }
            }
        } else {
            None
        };
        // The body is evaluated only to give the lambda a precise (singleton)
        // return type. Doing that for a *nested* lambda would run both branches of
        // an inner `if`, so a recursive const function could never reach its base
        // case: `Cnt(N: Nat): Nat = if N <= 1, do 1, do Cnt(N - 1)` used to
        // overflow the compiler's stack. Deeper lambdas fall back to `Obj`; their
        // value is unaffected, since it comes from actually calling them.
        let counter = RecursionCounter::new(&LAMBDA_BODY_DEPTH, 1);
        let return_t = if counter.limit_reached() {
            Type::Obj
        } else {
            // Building this copies the enclosing scope, and a recursive const
            // function makes two of these per level (one per `if` branch), so
            // it is built only when there is a body to evaluate in it.
            let mut lambda_ctx = Context::instant(
                Str::ever("<lambda>"),
                self.cfg.clone(),
                0,
                self.shared.clone(),
                self.clone(),
            );
            for non_default in non_default_params.iter() {
                let name = non_default
                    .name()
                    .map(|name| VarName::from_str(name.clone()));
                let vi = VarInfo::nd_parameter(
                    non_default.typ().clone(),
                    AbsLocation::unknown(),
                    lambda_ctx.name.clone(),
                );
                lambda_ctx.params.push((name, vi));
            }
            if let Some(var_param) = var_params.as_ref() {
                let name = var_param.name().map(|name| VarName::from_str(name.clone()));
                let vi = VarInfo::nd_parameter(
                    var_param.typ().clone(),
                    AbsLocation::unknown(),
                    lambda_ctx.name.clone(),
                );
                lambda_ctx.params.push((name, vi));
            }
            for default in default_params.iter() {
                let name = default.name().map(|name| VarName::from_str(name.clone()));
                let vi = VarInfo::d_parameter(
                    default.typ().clone(),
                    AbsLocation::unknown(),
                    lambda_ctx.name.clone(),
                );
                lambda_ctx.params.push((name, vi));
            }
            match lambda_ctx.eval_const_block(&lambda.body) {
                Ok(val) => v_enum(set! {val}),
                // The body reads its parameters (`(X: Int) -> X + 1`), which
                // have no value until the lambda is called: the only thing
                // this evaluation can trip over is a name. The lambda is a
                // value all the same -- calling it binds the parameters and
                // evaluates the body -- and its return type is what it
                // declares, if anything, as for a named const function.
                // Any other failure is the failure it is. Going on after one
                // would not do: a `do` block that recursed past the limit
                // would be *called* after its failed pre-evaluation, at every
                // level, and a recursion error took exponential time.
                Err((_, es))
                    if !lambda.sig.params.is_empty()
                        && es.iter().all(|e| e.core.kind == ErrorKind::NameError) =>
                {
                    match lambda.sig.return_t_spec.as_deref() {
                        Some(spec) => match self.instantiate_typespec_full(
                            &spec.t_spec,
                            None,
                            &mut tmp_tv_cache,
                            RegistrationMode::Normal,
                            false,
                        ) {
                            Ok(t) => t,
                            Err((t, es)) => {
                                errs.extend(es);
                                t
                            }
                        },
                        None => Type::Obj,
                    }
                }
                Err(err) => return Err(err),
            }
        };
        drop(counter);
        let sig_t = subr_t(
            SubrKind::from(lambda.op.kind),
            non_default_params.clone(),
            var_params,
            default_params.clone(),
            kw_var_params,
            return_t,
        );
        let block = match erg_parser::Parser::validate_const_block(lambda.body.clone()) {
            Ok(block) => block,
            Err(_) => {
                return Err((
                    ValueObj::Failure,
                    EvalErrors::from(EvalError::not_const_expr(
                        self.cfg.input.clone(),
                        line!() as usize,
                        lambda.loc(),
                        self.caused_by(),
                    )),
                ));
            }
        };
        let sig_t = self.generalize_t(sig_t);
        let subr = ConstSubr::User(UserConstSubr::new(
            Str::ever("<lambda>"),
            lambda.sig.params.clone(),
            block,
            sig_t,
            None,
        ));
        let subr = ValueObj::Subr(subr);
        if errs.is_empty() {
            Ok(subr)
        } else {
            Err((subr, errs))
        }
    }

    pub(crate) fn eval_lit(&self, lit: &Literal) -> EvalResult<ValueObj> {
        let t = type_from_token_kind(lit.token.kind);
        let value = ValueObj::from_str(t, lit.token.content.clone());
        // A decimal whose exact rational does not fit falls back to `f64`. Codegen
        // builds an inline literal from its source text, so folding the `f64` here
        // would make `C = 6.62607015e-34; print! C` and `print! 6.62607015e-34`
        // print different values.
        if lit.token.is(TokenKind::RatioLit) && matches!(value, Some(ValueObj::Float(_))) {
            return Err(EvalError::inexact_ratio_literal(
                self.cfg.input.clone(),
                line!() as usize,
                lit.token.loc(),
                self.caused_by(),
                lit.token.content.to_string(),
            )
            .into());
        }
        value.ok_or_else(|| {
            EvalError::invalid_literal(
                self.cfg.input.clone(),
                line!() as usize,
                lit.token.loc(),
                self.caused_by(),
            )
            .into()
        })
    }

    pub(crate) fn eval_const_expr(&self, expr: &Expr) -> Failable<ValueObj> {
        match expr {
            Expr::Literal(lit) => self.eval_lit(lit).map_err(|e| (ValueObj::Failure, e)),
            Expr::Accessor(acc) => self.eval_const_acc(acc),
            Expr::BinOp(bin) => self.eval_const_bin(bin),
            Expr::UnaryOp(unary) => self.eval_const_unary(unary),
            Expr::Call(call) => self.eval_const_call(call),
            Expr::List(lis) => self.eval_const_list(lis),
            Expr::Set(set) => self.eval_const_set(set),
            Expr::Dict(dict) => self.eval_const_dict(dict),
            Expr::Tuple(tuple) => self.eval_const_tuple(tuple),
            Expr::Record(rec) => self.eval_const_record(rec),
            Expr::Lambda(lambda) => self.eval_const_lambda(lambda),
            // FIXME: type check
            Expr::TypeAscription(tasc) => self.eval_const_expr(&tasc.expr),
            other => Err((
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    other.loc(),
                    self.caused_by(),
                )),
            )),
        }
    }

    /// Every class the declared return type of `t` names, across the branches of
    /// an intersection.
    ///
    /// `None` as soon as one branch returns something that is not a nominal
    /// class: a type variable says nothing about the class the program builds,
    /// and any branch may be the one that applied. `x[i]` is an intersection of
    /// `(Nat) -> T` and `(Range) -> List(T)`, so seeing only the `List` would
    /// reject every fold of an ordinary index.
    fn declared_return_classes(t: &Type) -> Option<Vec<Str>> {
        match t {
            Type::Quantified(quant) => Self::declared_return_classes(quant),
            Type::And(tys, _) => {
                let mut all = vec![];
                for t in tys {
                    all.extend(Self::declared_return_classes(t)?);
                }
                Some(all)
            }
            Type::Subr(subr) => match subr.return_t.derefine() {
                Type::Poly { name, .. } => Some(vec![name]),
                _ => None,
            },
            _ => Some(vec![]),
        }
    }

    /// The class `call` builds at run time, when folding it produced a different
    /// one -- `None` when the fold is faithful.
    ///
    /// `reversed [1, 2]` evaluates to the list `[2, 1]`, but the emitted program
    /// builds a `Reversed`. Typing the constant `{[2, 1]}` then has codegen wrap
    /// that stateful iterator in `List(...)`, which drains it: the second use of
    /// the constant sees an empty one. Folding it is still right *inside* a
    /// larger constant -- `sum(map(F, l))`, or the `all(map(P, xs))` of a
    /// refinement predicate -- so only a chunk's own value is checked, not the
    /// nested calls, which go through `eval_const_expr`.
    fn folded_away_class(&self, call: &Call, val: &ValueObj) -> Option<(String, Str, Type)> {
        let callee = match &call.attr_name {
            Some(attr) => self
                .eval_const_expr(&call.obj)
                .ok()
                .and_then(|obj| self.eval_attr(obj, attr).ok()),
            None => self.eval_const_expr(&call.obj).ok(),
        };
        let Some(ValueObj::Subr(subr)) = callee else {
            return None;
        };
        let mut declared = Self::declared_return_classes(subr.sig_t())?;
        if declared.is_empty() {
            return None;
        }
        let class = val.class();
        let built = self
            .get_super_classes_or_self(&class)
            .any(|c| declared.contains(&c.qual_name()));
        (!built).then(|| (subr.to_string(), declared.swap_remove(0), class))
    }

    // Evaluate compile-time expression (just Expr on AST) instead of evaluating ConstExpr
    // Return Err if it cannot be evaluated at compile time
    // ConstExprを評価するのではなく、コンパイル時関数の式(AST上ではただのExpr)を評価する
    // コンパイル時評価できないならNoneを返す
    pub(crate) fn eval_const_chunk(&mut self, expr: &Expr) -> Failable<ValueObj> {
        match expr {
            Expr::Def(def) => self.eval_const_def(def),
            // what a pattern definition desugars to (`(a, b) = t`): definitions
            // of its own, made in this scope like the block's
            Expr::Compound(compound) => {
                let mut last = ValueObj::None;
                for chunk in compound.iter() {
                    last = self.eval_const_chunk(chunk)?;
                }
                Ok(last)
            }
            // a class, a patch or a methods block inside a const function's
            // body is rejected as not a constant expression
            other => self.eval_const_chunk_ref(other),
        }
    }

    /// Every chunk but a definition, which is the only one that needs a scope
    /// to define into. Split out so that a block which defines nothing -- an
    /// `if` branch, say -- can be evaluated in the scope that is already there.
    fn eval_const_chunk_ref(&self, expr: &Expr) -> Failable<ValueObj> {
        match expr {
            Expr::Literal(lit) => self.eval_lit(lit).map_err(|e| (ValueObj::Failure, e)),
            Expr::Accessor(acc) => self.eval_const_acc(acc),
            Expr::BinOp(bin) => self.eval_const_bin(bin),
            Expr::UnaryOp(unary) => self.eval_const_unary(unary),
            Expr::Call(call) => {
                let val = self.eval_const_call(call)?;
                match self.folded_away_class(call, &val) {
                    Some((callee, class, folded)) => Err((
                        ValueObj::Failure,
                        EvalErrors::from(EvalError::fold_changes_class(
                            self.cfg.input.clone(),
                            line!() as usize,
                            call.loc(),
                            self.caused_by(),
                            callee,
                            class.to_string(),
                            folded.to_string(),
                        )),
                    )),
                    None => Ok(val),
                }
            }
            Expr::List(lis) => self.eval_const_list(lis),
            Expr::Set(set) => self.eval_const_set(set),
            Expr::Dict(dict) => self.eval_const_dict(dict),
            Expr::Tuple(tuple) => self.eval_const_tuple(tuple),
            Expr::Record(rec) => self.eval_const_record(rec),
            Expr::Lambda(lambda) => self.eval_const_lambda(lambda),
            Expr::TypeAscription(tasc) => self.eval_const_expr(&tasc.expr),
            other => Err((
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    other.loc(),
                    self.caused_by(),
                )),
            )),
        }
    }

    pub(crate) fn eval_const_block(&mut self, block: &Block) -> Failable<ValueObj> {
        for chunk in block.iter().rev().skip(1).rev() {
            self.eval_const_chunk(chunk)?;
        }
        self.eval_const_chunk(block.last().unwrap())
    }

    /// `match obj, arm, arm, ...`, which is a special form rather than a name.
    ///
    /// Reproduces what the emitted program does rather than deciding by type:
    /// each arm binds the value to its parameter and tests, in order, the
    /// conditions the pattern was desugared into -- the same expressions
    /// codegen emits. The last arm is taken without testing, because at run
    /// time an unmatched `match` falls into it too.
    fn eval_const_match(&self, call: &Call) -> Failable<ValueObj> {
        let unfoldable = || {
            (
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    call.loc(),
                    self.caused_by(),
                )),
            )
        };
        // Arms nest as deeply as the source does, and each is Rust frames like
        // any other, so they are counted against the stack the same way.
        if CONST_EVAL_DEPTH.with(|d| d.get()) >= erg_common::spawn::CONST_CALL_LIMIT {
            return self.on_new_stack(call.loc(), || self.eval_const_match(call));
        }
        let _counter =
            RecursionCounter::new(&CONST_EVAL_DEPTH, erg_common::spawn::CONST_CALL_LIMIT);
        if !call.args.kw_args.is_empty() || call.args.var_args.is_some() {
            return Err(unfoldable());
        }
        let mut pos_args = call.args.pos_args.iter();
        let obj = self.eval_const_expr(&pos_args.next().ok_or_else(unfoldable)?.expr)?;
        let arms = pos_args
            .map(|arg| match &arg.expr {
                Expr::Lambda(lambda) => Some(lambda),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(unfoldable)?;
        let last = arms.len().checked_sub(1).ok_or_else(unfoldable)?;
        let mut arm_ctx = Context::instant_in(
            Str::from(format!("{}::<match>", self.name)),
            self.cfg.clone(),
            1,
            self.shared.clone(),
            Arc::new(self.clone()),
        );
        for (i, arm) in arms.iter().enumerate() {
            let [param] = &arm.sig.params.non_defaults[..] else {
                return Err(unfoldable());
            };
            if !arm.sig.params.defaults.is_empty() || arm.sig.params.var_params.is_some() {
                return Err(unfoldable());
            }
            arm_ctx.consts.clear();
            if let Some(name) = param.inspect() {
                arm_ctx
                    .consts
                    .insert(VarName::from_str(name.clone()), obj.clone());
            }
            let mut matched = true;
            if i != last {
                for guard in arm.sig.params.guards.iter() {
                    // a binding in a pattern would have to define into the arm's
                    // scope, which is not what this reproduces
                    let GuardClause::Condition(cond) = guard else {
                        return Err(unfoldable());
                    };
                    match arm_ctx.eval_const_chunk_ref(cond) {
                        Ok(ValueObj::Bool(true)) => {}
                        Ok(ValueObj::Bool(false)) => {
                            matched = false;
                            break;
                        }
                        // a guard that does not fold to a `Bool` leaves which arm
                        // runs undecided, and a later arm may still be the answer
                        _ => return Err(unfoldable()),
                    }
                }
            }
            if matched {
                return arm_ctx.eval_const_block(&arm.body);
            }
        }
        Err(unfoldable())
    }

    /// A `do` block evaluated in *this* scope, with no frame of its own.
    ///
    /// An `if` branch takes no arguments and reads the scope it was written
    /// in, which is the scope evaluating the `if` -- so a frame for it would
    /// only clone that scope and spend a second level of the recursion limit
    /// per level the user wrote. The block must define no name: see
    /// [`UserConstSubr::defines_name`], which is what the caller checks.
    pub(crate) fn eval_const_do_block(&self, block: &Block) -> Failable<ValueObj> {
        // These are Rust frames like any other, so they are counted, and this
        // stack is replaced when they spend it. Blocks nest only as deeply as
        // the source does, but a source nesting them deeply enough would
        // otherwise walk past the budget a call had left.
        if CONST_EVAL_DEPTH.with(|d| d.get()) >= erg_common::spawn::CONST_CALL_LIMIT {
            return self.on_new_stack(block.loc(), || self.eval_const_do_block(block));
        }
        let _counter =
            RecursionCounter::new(&CONST_EVAL_DEPTH, erg_common::spawn::CONST_CALL_LIMIT);
        let mut last = None;
        for chunk in block.iter() {
            last = Some(self.eval_const_chunk_ref(chunk)?);
        }
        last.ok_or_else(|| {
            (
                ValueObj::Failure,
                EvalErrors::from(EvalError::not_const_expr(
                    self.cfg.input.clone(),
                    line!() as usize,
                    block.loc(),
                    self.caused_by(),
                )),
            )
        })
    }

    /// Inclusion comparison on type objects (`Nat < Int`, `{=} > {x = Int}`, ...).
    fn eval_type_cmp(&self, op: OpKind, lhs: &ValueObj, rhs: &ValueObj) -> Option<ValueObj> {
        let lt = self.convert_value_into_type(lhs.clone()).ok()?;
        let rt = self.convert_value_into_type(rhs.clone()).ok()?;
        let sub = self.subtype_of(&lt, &rt);
        let sup = self.supertype_of(&lt, &rt);
        Some(ValueObj::Bool(match op {
            Lt => sub && !sup,
            Le => sub,
            Gt => sup && !sub,
            Ge => sup,
            _ => return None,
        }))
    }

    /// `container contains elem` (`elem in container` after desugaring).
    fn eval_contains(
        &self,
        container: ValueObj,
        elem: ValueObj,
        loc: Location,
    ) -> EvalResult<ValueObj> {
        match &container {
            ValueObj::List(xs) | ValueObj::Tuple(xs) => {
                return Ok(ValueObj::Bool(xs.iter().any(|v| v == &elem)));
            }
            ValueObj::Set(s) => {
                return Ok(ValueObj::Bool(s.iter().any(|v| v == &elem)));
            }
            ValueObj::Dict(d) => {
                return Ok(ValueObj::Bool(d.linear_get(&elem).is_some()));
            }
            ValueObj::Str(s) => {
                if let Some(sub) = elem.as_str() {
                    return Ok(ValueObj::Bool(s.contains(&sub[..])));
                }
            }
            _ => {}
        }
        match self.convert_value_into_type(container) {
            Ok(ty) => Ok(ValueObj::Bool(self.subtype_of(&elem.t(), &ty))),
            Err(_) => Err(EvalErrors::from(EvalError::not_const_expr(
                self.cfg.input.clone(),
                line!() as usize,
                loc,
                self.caused_by(),
            ))),
        }
    }

    fn get_const_op_subr(&self, ty: &Type, name: &str) -> Option<ConstSubr> {
        for ctx in self.get_nominal_super_type_ctxs(ty)? {
            if let Some(ValueObj::Subr(subr)) = ctx.consts.get(name) {
                return Some(subr.clone());
            }
            for methods in ctx.methods_list.iter() {
                if let Some(ValueObj::Subr(subr)) = methods.consts.get(name) {
                    return Some(subr.clone());
                }
            }
        }
        None
    }

    fn eval_user_binop(&self, op: OpKind, lhs: ValueObj, rhs: ValueObj) -> Option<ValueObj> {
        let name = op_to_name(op);
        let subr = self.get_const_op_subr(&lhs.class(), name)?;
        let args = ValueArgs::pos_only(vec![lhs, rhs]);
        match self.call(subr, args, Location::Unknown) {
            Ok(tp) => self
                .convert_tp_into_value(tp)
                .ok()
                .filter(|v| !matches!(v, ValueObj::Failure)),
            Err(_) => None,
        }
    }

    fn eval_prim_or_user(
        &self,
        op: OpKind,
        lhs: ValueObj,
        rhs: ValueObj,
        prim: Option<ValueObj>,
        loc: Location,
    ) -> EvalResult<ValueObj> {
        if let Some(v) = prim {
            Ok(v)
        } else if let Some(v) = self.eval_user_binop(op, lhs.clone(), rhs.clone()) {
            Ok(v)
        } else {
            Err(EvalErrors::from(EvalError::uncomputable_op(
                self.cfg.input.clone(),
                line!() as usize,
                loc,
                self.caused_by(),
                format!("{lhs} {op} {rhs}"),
            )))
        }
    }

    fn eval_bin(
        &self,
        op: OpKind,
        lhs: ValueObj,
        rhs: ValueObj,
        loc: Location,
    ) -> EvalResult<ValueObj> {
        if matches!(op, Div | FloorDiv | Mod) && rhs.is_zero() {
            return Err(EvalErrors::from(EvalError::zero_division(
                self.cfg.input.clone(),
                line!() as usize,
                loc,
                self.caused_by(),
            )));
        }
        match op {
            Add => {
                let prim = lhs.clone().try_add(rhs.clone());
                self.eval_prim_or_user(Add, lhs, rhs, prim, loc)
            }
            Sub => {
                let prim = lhs.clone().try_sub(rhs.clone());
                self.eval_prim_or_user(Sub, lhs, rhs, prim, loc)
            }
            Mul => {
                let prim = lhs.clone().try_mul(rhs.clone());
                self.eval_prim_or_user(Mul, lhs, rhs, prim, loc)
            }
            Div => {
                let prim = lhs.clone().try_div(rhs.clone());
                self.eval_prim_or_user(Div, lhs, rhs, prim, loc)
            }
            FloorDiv => {
                let prim = lhs.clone().try_floordiv(rhs.clone());
                self.eval_prim_or_user(FloorDiv, lhs, rhs, prim, loc)
            }
            Pow => {
                let prim = lhs.clone().try_pow(rhs.clone());
                self.eval_prim_or_user(Pow, lhs, rhs, prim, loc)
            }
            Mod => {
                let prim = lhs.clone().try_mod(rhs.clone());
                self.eval_prim_or_user(Mod, lhs, rhs, prim, loc)
            }
            Lt | Le | Gt | Ge => {
                if let Some(v) = self.eval_type_cmp(op, &lhs, &rhs) {
                    Ok(v)
                } else {
                    let prim = match op {
                        Gt => lhs.clone().try_gt(rhs.clone()),
                        Ge => lhs.clone().try_ge(rhs.clone()),
                        Lt => lhs.clone().try_lt(rhs.clone()),
                        Le => lhs.clone().try_le(rhs.clone()),
                        _ => None,
                    };
                    self.eval_prim_or_user(op, lhs, rhs, prim, loc)
                }
            }
            Eq => {
                let prim = lhs.clone().try_eq(rhs.clone());
                self.eval_prim_or_user(Eq, lhs, rhs, prim, loc)
            }
            Ne => {
                let prim = lhs.clone().try_ne(rhs.clone());
                self.eval_prim_or_user(Ne, lhs, rhs, prim, loc)
            }
            Or | BitOr => self.eval_or(lhs, rhs),
            And | BitAnd => self.eval_and(lhs, rhs),
            BitXor => match (lhs, rhs) {
                (ValueObj::Bool(l), ValueObj::Bool(r)) => Ok(ValueObj::Bool(l ^ r)),
                (ValueObj::Int(l), ValueObj::Int(r)) => Ok(ValueObj::Int(l ^ r)),
                _ => Err(EvalErrors::from(EvalError::unreachable(
                    self.cfg.input.clone(),
                    fn_name!(),
                    line!(),
                ))),
            },
            Shl => {
                let prim = lhs.clone().try_shl(rhs.clone());
                self.eval_prim_or_user(Shl, lhs, rhs, prim, loc)
            }
            Shr => {
                let prim = lhs.clone().try_shr(rhs.clone());
                self.eval_prim_or_user(Shr, lhs, rhs, prim, loc)
            }
            ClosedRange => Ok(ValueObj::range(lhs, rhs)),
            LeftOpenRange => Ok(ValueObj::interval(IntervalOp::LeftOpen, lhs, rhs)),
            RightOpenRange => Ok(ValueObj::interval(IntervalOp::RightOpen, lhs, rhs)),
            OpenRange => Ok(ValueObj::interval(IntervalOp::Open, lhs, rhs)),
            _other => Err(EvalErrors::from(EvalError::unreachable(
                self.cfg.input.clone(),
                fn_name!(),
                line!(),
            ))),
        }
    }

    fn eval_or(&self, lhs: ValueObj, rhs: ValueObj) -> EvalResult<ValueObj> {
        match (lhs, rhs) {
            (ValueObj::Bool(l), ValueObj::Bool(r)) => Ok(ValueObj::Bool(l || r)),
            (ValueObj::Int(l), ValueObj::Int(r)) => Ok(ValueObj::Int(l | r)),
            (ValueObj::Type(lhs), ValueObj::Type(rhs)) => Ok(self.eval_or_type(lhs, rhs)),
            // `T or None` should be `T or NoneType`.
            // The type checker will report an error, so here it will be interpreted as `T or NoneType`.
            (ValueObj::Type(lhs), ValueObj::None) => {
                Ok(self.eval_or_type(lhs, TypeObj::builtin_type(Type::NoneType)))
            }
            (ValueObj::None, ValueObj::Type(rhs)) => {
                Ok(self.eval_or_type(TypeObj::builtin_type(Type::NoneType), rhs))
            }
            (lhs, rhs) => {
                let lhs = self.convert_value_into_type(lhs).ok();
                let rhs = self.convert_value_into_type(rhs).ok();
                if let Some((l, r)) = lhs.zip(rhs) {
                    self.eval_or(ValueObj::builtin_type(l), ValueObj::builtin_type(r))
                } else {
                    Err(EvalErrors::from(EvalError::unreachable(
                        self.cfg.input.clone(),
                        fn_name!(),
                        line!(),
                    )))
                }
            }
        }
    }

    fn eval_or_type(&self, lhs: TypeObj, rhs: TypeObj) -> ValueObj {
        match (lhs, rhs) {
            (
                TypeObj::Builtin {
                    t: l,
                    meta_t: Type::ClassType,
                },
                TypeObj::Builtin {
                    t: r,
                    meta_t: Type::ClassType,
                },
            ) => ValueObj::builtin_class(self.union(&l, &r)),
            (
                TypeObj::Builtin {
                    t: l,
                    meta_t: Type::TraitType,
                },
                TypeObj::Builtin {
                    t: r,
                    meta_t: Type::TraitType,
                },
            ) => ValueObj::builtin_trait(self.union(&l, &r)),
            (TypeObj::Builtin { t: l, meta_t: _ }, TypeObj::Builtin { t: r, meta_t: _ }) => {
                ValueObj::builtin_type(self.union(&l, &r))
            }
            (lhs, rhs) => ValueObj::gen_t(GenTypeObj::union(
                self.union(lhs.typ(), rhs.typ()),
                lhs,
                rhs,
            )),
        }
    }

    fn eval_and(&self, lhs: ValueObj, rhs: ValueObj) -> EvalResult<ValueObj> {
        match (lhs, rhs) {
            (ValueObj::Bool(l), ValueObj::Bool(r)) => Ok(ValueObj::Bool(l && r)),
            (ValueObj::Int(l), ValueObj::Int(r)) => Ok(ValueObj::Int(l & r)),
            (ValueObj::Type(lhs), ValueObj::Type(rhs)) => Ok(self.eval_and_type(lhs, rhs)),
            (lhs, rhs) => {
                let lhs = self.convert_value_into_type(lhs).ok();
                let rhs = self.convert_value_into_type(rhs).ok();
                if let Some((l, r)) = lhs.zip(rhs) {
                    self.eval_and(ValueObj::builtin_type(l), ValueObj::builtin_type(r))
                } else {
                    Err(EvalErrors::from(EvalError::unreachable(
                        self.cfg.input.clone(),
                        fn_name!(),
                        line!(),
                    )))
                }
            }
        }
    }

    fn eval_and_type(&self, lhs: TypeObj, rhs: TypeObj) -> ValueObj {
        match (lhs, rhs) {
            (
                TypeObj::Builtin {
                    t: l,
                    meta_t: Type::ClassType,
                },
                TypeObj::Builtin {
                    t: r,
                    meta_t: Type::ClassType,
                },
            ) => ValueObj::builtin_class(self.intersection(&l, &r)),
            (
                TypeObj::Builtin {
                    t: l,
                    meta_t: Type::TraitType,
                },
                TypeObj::Builtin {
                    t: r,
                    meta_t: Type::TraitType,
                },
            ) => ValueObj::builtin_trait(self.intersection(&l, &r)),
            (TypeObj::Builtin { t: l, meta_t: _ }, TypeObj::Builtin { t: r, meta_t: _ }) => {
                ValueObj::builtin_type(self.intersection(&l, &r))
            }
            (lhs, rhs) => ValueObj::gen_t(GenTypeObj::intersection(
                self.intersection(lhs.typ(), rhs.typ()),
                lhs,
                rhs,
            )),
        }
    }

    fn eval_not_type(&self, ty: TypeObj) -> ValueObj {
        match ty {
            TypeObj::Builtin {
                t,
                meta_t: Type::ClassType,
            } => ValueObj::builtin_class(self.complement(&t)),
            TypeObj::Builtin {
                t,
                meta_t: Type::TraitType,
            } => ValueObj::builtin_trait(self.complement(&t)),
            TypeObj::Builtin { t, meta_t: _ } => ValueObj::builtin_type(self.complement(&t)),
            // FIXME:
            _ => ValueObj::Failure,
        }
    }

    /// lhs, rhs: may be unevaluated
    pub(crate) fn eval_bin_tp(
        &self,
        op: OpKind,
        lhs: TyParam,
        rhs: TyParam,
    ) -> EvalResult<TyParam> {
        // let lhs = self.eval_tp(lhs).map_err(|(_, es)| es)?;
        // let rhs = self.eval_tp(rhs).map_err(|(_, es)| es)?;
        match (lhs, rhs) {
            (TyParam::Value(lhs), TyParam::Value(rhs)) => self
                .eval_bin(op, lhs, rhs, Location::Unknown)
                .map(TyParam::value),
            (TyParam::Dict(l), TyParam::Dict(r)) if op == OpKind::Add => {
                Ok(TyParam::Dict(l.concat(r)))
            }
            (TyParam::List(l), TyParam::List(r)) if op == OpKind::Add => {
                Ok(TyParam::List([l, r].concat()))
            }
            (TyParam::FreeVar(fv), r) if fv.is_linked() => {
                let t = fv.crack().clone();
                self.eval_bin_tp(op, t, r)
            }
            (TyParam::FreeVar(_), _) if op.is_comparison() => Ok(TyParam::value(true)),
            // _: Nat <= 10 => true
            // TODO: maybe this is wrong, we should do the type-checking of `<=`
            (TyParam::Erased(t), rhs)
                if op.is_comparison()
                    && self.supertype_of(&t, &self.get_tp_t(&rhs).unwrap_or(Type::Obj)) =>
            {
                Ok(TyParam::value(true))
            }
            (l, TyParam::FreeVar(fv)) if fv.is_linked() => {
                let t = fv.crack().clone();
                self.eval_bin_tp(op, l, t)
            }
            (_, TyParam::FreeVar(_)) if op.is_comparison() => Ok(TyParam::value(true)),
            // 10 <= _: Nat => true
            (lhs, TyParam::Erased(t))
                if op.is_comparison()
                    && self.supertype_of(&self.get_tp_t(&lhs).unwrap_or(Type::Obj), &t) =>
            {
                Ok(TyParam::value(true))
            }
            (e @ TyParam::Erased(_), _) | (_, e @ TyParam::Erased(_)) => Ok(e),
            (lhs @ TyParam::FreeVar(_), rhs) => Ok(TyParam::bin(op, lhs, rhs)),
            (lhs, rhs @ TyParam::FreeVar(_)) => Ok(TyParam::bin(op, lhs, rhs)),
            (TyParam::Value(lhs), rhs) => {
                let lhs = match Self::convert_value_into_tp(lhs) {
                    Ok(tp) => tp,
                    Err(lhs) => {
                        return feature_error!(
                            self,
                            Location::Unknown,
                            &format!("{lhs} {op} {rhs}")
                        );
                    }
                };
                self.eval_bin_tp(op, lhs, rhs)
            }
            (lhs, TyParam::Value(rhs)) => {
                let rhs = match Self::convert_value_into_tp(rhs) {
                    Ok(tp) => tp,
                    Err(rhs) => {
                        return feature_error!(
                            self,
                            Location::Unknown,
                            &format!("{lhs} {op} {rhs}")
                        );
                    }
                };
                self.eval_bin_tp(op, lhs, rhs)
            }
            (l, r) => feature_error!(self, Location::Unknown, &format!("{l} {op} {r}")),
        }
    }

    fn eval_unary_val(&self, op: OpKind, val: ValueObj, loc: Location) -> EvalResult<ValueObj> {
        let uncomputable = |val: &ValueObj| {
            EvalErrors::from(EvalError::uncomputable_op(
                self.cfg.input.clone(),
                line!() as usize,
                loc,
                self.caused_by(),
                format!("{op}{val}"),
            ))
        };
        match op {
            Pos => match val {
                ValueObj::Bool(b) => Ok(ValueObj::Nat(b as u64)),
                ValueObj::Nat(_)
                | ValueObj::Int(_)
                | ValueObj::Ratio(..)
                | ValueObj::Float(_)
                | ValueObj::Inf
                | ValueObj::NegInf => Ok(val),
                _ => Err(uncomputable(&val)),
            },
            Neg => match &val {
                ValueObj::Bool(b) => Ok(ValueObj::Int(-(*b as i64))),
                // `Nat` is a `u64` but `Int` an `i32`, so negating can leave the range
                ValueObj::Nat(n) => i64::try_from(*n)
                    .map(|n| ValueObj::Int(-n))
                    .map_err(|_| uncomputable(&val)),
                ValueObj::Int(i) => i
                    .checked_neg()
                    .map(ValueObj::Int)
                    .ok_or_else(|| uncomputable(&val)),
                ValueObj::Ratio(n, d) => n
                    .checked_neg()
                    .map(|n| ValueObj::Ratio(n, *d))
                    .ok_or_else(|| uncomputable(&val)),
                ValueObj::Float(f) => Ok(ValueObj::Float(-*f)),
                ValueObj::Inf => Ok(ValueObj::NegInf),
                ValueObj::NegInf => Ok(ValueObj::Inf),
                _ => Err(uncomputable(&val)),
            },
            // `~` is the bitwise complement (`~x == -(x + 1)`), so `Bool` follows
            // `Int` here (`~True == -2`), just like `UNARY_INVERT` does at runtime.
            Invert => match &val {
                ValueObj::Bool(_) | ValueObj::Int(_) | ValueObj::Nat(_) => {
                    val.clone().try_invert().ok_or_else(|| uncomputable(&val))
                }
                _ => Err(uncomputable(&val)),
            },
            Not => match val {
                ValueObj::Bool(b) => Ok(ValueObj::Bool(!b)),
                ValueObj::Type(lhs) => Ok(self.eval_not_type(lhs)),
                val => Err(uncomputable(&val)),
            },
            _other => unreachable_error!(self),
        }
    }

    /// val: may be unevaluated
    pub(crate) fn eval_unary_tp(&self, op: OpKind, val: TyParam) -> EvalResult<TyParam> {
        // let val = self.eval_tp(val).map_err(|(_, es)| es)?;
        match val {
            TyParam::Value(c) => self
                .eval_unary_val(op, c, Location::Unknown)
                .map(TyParam::Value),
            TyParam::FreeVar(fv) if fv.is_linked() => {
                let t = fv.crack().clone();
                self.eval_unary_tp(op, t)
            }
            e @ TyParam::Erased(_) => Ok(e),
            TyParam::FreeVar(fv) if fv.is_unbound() => {
                feature_error!(self, Location::Unknown, &format!("{op} {fv}"))
            }
            other => feature_error!(self, Location::Unknown, &format!("{op} {other}")),
        }
    }

    /// args: may be unevaluated
    ///
    /// NOTE: This function may not evaluate app and return `TyParam::app(name, args)`.
    /// Be careful with recursive calls.
    pub(crate) fn eval_app(&self, name: Str, args: Vec<TyParam>) -> Failable<TyParam> {
        /*let args = self
        .eval_type_args(args)
        .map_err(|(args, es)| (TyParam::app(name.clone(), args), es))?;*/
        if let Ok(value_args) = args
            .iter()
            .map(|tp| self.convert_tp_into_value(tp.clone()))
            .collect::<Result<Vec<_>, _>>()
        {
            if let Some(ValueObj::Subr(subr)) = self.rec_get_const_obj(&name) {
                let args = ValueArgs::pos_only(value_args);
                self.call(subr.clone(), args, ().loc())
            } else {
                log!(err "eval_app({name}({}))", fmt_vec(&args));
                Ok(TyParam::app(name, args))
            }
        } else {
            log!(err "failed: eval_app({name}({}))", fmt_vec(&args));
            Ok(TyParam::app(name, args))
        }
    }

    /// Evaluate type parameters.
    /// Some type parameters may be withheld from evaluation because they can be evaluated but only result in a `Failure` as is.
    /// Quantified variables, etc. are returned as is.
    /// 量化変数などはそのまま返す
    pub(crate) fn eval_tp(&self, p: TyParam) -> Failable<TyParam> {
        let mut errs = EvalErrors::empty();
        let tp = match p {
            TyParam::FreeVar(fv) if fv.is_linked() => match self.eval_tp(fv.unwrap_linked()) {
                Ok(tp) => tp,
                Err((tp, es)) => {
                    errs.extend(es);
                    tp
                }
            },
            TyParam::FreeVar(_) => p,
            TyParam::Mono(name) => match self
                .rec_get_const_obj(&name)
                .map(|v| TyParam::value(v.clone()))
            {
                Some(tp) => tp,
                None => {
                    errs.push(EvalError::no_var_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        Location::Unknown,
                        self.caused_by(),
                        &name,
                        None,
                    ));
                    TyParam::mono(name)
                }
            },
            TyParam::App { name, args } => match self.eval_app(name, args) {
                Ok(tp) => tp,
                Err((tp, es)) => {
                    errs.extend(es);
                    tp
                }
            },
            TyParam::DataClass { name, fields } => {
                let mut new_fields = dict! {};
                for (name, tp) in fields.into_iter() {
                    match self.eval_tp(tp) {
                        Ok(tp) => {
                            new_fields.insert(name, tp);
                        }
                        Err((tp, es)) => {
                            new_fields.insert(name, tp);
                            errs.extend(es);
                        }
                    }
                }
                TyParam::DataClass {
                    name,
                    fields: new_fields,
                }
            }
            TyParam::BinOp { op, lhs, rhs } => match self.eval_bin_tp(op, *lhs, *rhs) {
                Ok(tp) => tp,
                Err(es) => {
                    errs.extend(es);
                    return Err((TyParam::Failure, errs));
                }
            },
            TyParam::UnaryOp { op, val } => match self.eval_unary_tp(op, *val) {
                Ok(tp) => tp,
                Err(es) => {
                    errs.extend(es);
                    return Err((TyParam::Failure, errs));
                }
            },
            TyParam::List(tps) => {
                let mut new_tps = Vec::with_capacity(tps.len());
                for tp in tps {
                    match self.eval_tp(tp) {
                        Ok(tp) => new_tps.push(tp),
                        Err((tp, es)) => {
                            new_tps.push(tp);
                            errs.extend(es);
                        }
                    }
                }
                TyParam::List(new_tps)
            }
            TyParam::UnsizedList(elem) => {
                let elem = match self.eval_tp(*elem) {
                    Ok(tp) => tp,
                    Err((tp, es)) => {
                        errs.extend(es);
                        tp
                    }
                };
                TyParam::UnsizedList(Box::new(elem))
            }
            TyParam::Tuple(tps) => {
                let mut new_tps = Vec::with_capacity(tps.len());
                for tp in tps {
                    match self.eval_tp(tp) {
                        Ok(tp) => new_tps.push(tp),
                        Err((tp, es)) => {
                            new_tps.push(tp);
                            errs.extend(es);
                        }
                    }
                }
                TyParam::Tuple(new_tps)
            }
            TyParam::Dict(dic) => {
                let mut new_dic = dict! {};
                for (k, v) in dic.into_iter() {
                    match (self.eval_tp(k), self.eval_tp(v)) {
                        (Ok(k), Ok(v)) => {
                            new_dic.insert(k, v);
                        }
                        (Ok(k), Err((v, es))) => {
                            new_dic.insert(k, v);
                            errs.extend(es);
                        }
                        (Err((k, es)), Ok(v)) => {
                            new_dic.insert(k, v);
                            errs.extend(es);
                        }
                        (Err((k, es1)), Err((v, es2))) => {
                            new_dic.insert(k, v);
                            errs.extend(es1);
                            errs.extend(es2);
                        }
                    }
                }
                TyParam::Dict(new_dic)
            }
            TyParam::Set(set) => {
                let mut new_set = set! {};
                for v in set.into_iter() {
                    match self.eval_tp(v) {
                        Ok(v) => {
                            new_set.insert(v);
                        }
                        Err((v, es)) => {
                            new_set.insert(v);
                            errs.extend(es);
                        }
                    }
                }
                TyParam::Set(new_set)
            }
            TyParam::Record(dict) => {
                let mut fields = dict! {};
                for (name, tp) in dict.into_iter() {
                    match self.eval_tp(tp) {
                        Ok(tp) => {
                            fields.insert(name, tp);
                        }
                        Err((tp, es)) => {
                            fields.insert(name, tp);
                            errs.extend(es);
                        }
                    }
                }
                TyParam::Record(fields)
            }
            TyParam::Type(t) => match self.eval_t_params(*t, self.level, &()) {
                Ok(t) => TyParam::t(t),
                Err((t, es)) => {
                    errs.extend(es);
                    TyParam::t(t)
                }
            },
            TyParam::Erased(t) => match self.eval_t_params(*t, self.level, &()) {
                Ok(t) => TyParam::erased(t),
                Err((t, es)) => {
                    errs.extend(es);
                    TyParam::erased(t)
                }
            },
            TyParam::Value(val) => TyParam::Value(val),
            TyParam::ProjCall { obj, attr, args } => {
                match self.eval_proj_call(*obj, attr, args, &()) {
                    Ok(tp) => tp,
                    Err(es) => {
                        errs.extend(es);
                        return Err((TyParam::Failure, errs));
                    }
                }
            }
            TyParam::Proj { obj, attr } => match self.eval_tp_proj(*obj, attr, &()) {
                Ok(tp) => tp,
                Err(es) => {
                    errs.extend(es);
                    return Err((TyParam::Failure, errs));
                }
            },
            TyParam::Lambda(lambda) => TyParam::Lambda(lambda),
            TyParam::Failure => TyParam::Failure,
        };
        if errs.is_empty() {
            Ok(tp)
        } else {
            Err((tp, errs))
        }
    }

    fn eval_tp_into_value(&self, tp: TyParam) -> Failable<ValueObj> {
        let (tp, mut errs) = match self.eval_tp(tp) {
            Ok(tp) => (tp, EvalErrors::empty()),
            Err((tp, errs)) => (tp, errs),
        };
        let val = match self.convert_tp_into_value(tp) {
            Ok(val) => val,
            Err(err) => {
                errs.push(EvalError::feature_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    Location::Unknown,
                    &format!("convert {err} into a value"),
                    self.caused_by(),
                ));
                return Err((ValueObj::Failure, errs));
            }
        };
        if errs.is_empty() {
            Ok(val)
        } else {
            Err((val, errs))
        }
    }

    /// Evaluate `substituted`.
    /// Some types may be withheld from evaluation because they can be evaluated but are only `Failure/Never` as is (e.g. `?T(:> Never).Proj`).
    /// If the evaluation fails, return a harmless type (filled with `Failure`) and errors
    pub(crate) fn eval_t_params(
        &self,
        substituted: Type,
        level: usize,
        t_loc: &impl Locational,
    ) -> Failable<Type> {
        let mut errs = EvalErrors::empty();
        match substituted {
            Type::FreeVar(fv) if fv.is_linked() => {
                self.eval_t_params(fv.unwrap_linked(), level, t_loc)
            }
            Type::FreeVar(fv) if fv.constraint_is_sandwiched() => {
                let (sub, sup) = fv.get_subsup().unwrap();
                let sub = if sub.is_recursive() {
                    sub
                } else {
                    self.eval_t_params(sub, level, t_loc)?
                };
                let sup = if sup.is_recursive() {
                    sup
                } else {
                    self.eval_t_params(sup, level, t_loc)?
                };
                let fv = Type::FreeVar(fv);
                fv.update_tyvar(sub, sup, None, false);
                Ok(fv)
            }
            Type::FreeVar(_) => Ok(substituted),
            Type::Subr(mut subr) => {
                for pt in subr.non_default_params.iter_mut() {
                    *pt.typ_mut() = match self.eval_t_params(mem::take(pt.typ_mut()), level, t_loc)
                    {
                        Ok(t) => t,
                        Err((t, es)) => {
                            errs.extend(es);
                            t
                        }
                    };
                }
                if let Some(var_args) = subr.var_params.as_mut() {
                    *var_args.typ_mut() =
                        match self.eval_t_params(mem::take(var_args.typ_mut()), level, t_loc) {
                            Ok(t) => t,
                            Err((t, es)) => {
                                errs.extend(es);
                                t
                            }
                        };
                }
                for pt in subr.default_params.iter_mut() {
                    *pt.typ_mut() = match self.eval_t_params(mem::take(pt.typ_mut()), level, t_loc)
                    {
                        Ok(t) => t,
                        Err((t, es)) => {
                            errs.extend(es);
                            t
                        }
                    };
                    if let Some(default) = pt.default_typ_mut() {
                        *default = match self.eval_t_params(mem::take(default), level, t_loc) {
                            Ok(t) => t,
                            Err((t, es)) => {
                                errs.extend(es);
                                t
                            }
                        };
                    }
                }
                if let Some(kw_var_args) = subr.kw_var_params.as_mut() {
                    *kw_var_args.typ_mut() =
                        match self.eval_t_params(mem::take(kw_var_args.typ_mut()), level, t_loc) {
                            Ok(t) => t,
                            Err((t, es)) => {
                                errs.extend(es);
                                t
                            }
                        };
                    if let Some(default) = kw_var_args.default_typ_mut() {
                        *default = match self.eval_t_params(mem::take(default), level, t_loc) {
                            Ok(t) => t,
                            Err((t, es)) => {
                                errs.extend(es);
                                t
                            }
                        };
                    }
                }
                let return_t = match self.eval_t_params(*subr.return_t, level, t_loc) {
                    Ok(return_t) => return_t,
                    Err((return_t, es)) => {
                        errs.extend(es);
                        return_t
                    }
                };
                let subr = subr_t(
                    subr.kind,
                    subr.non_default_params,
                    subr.var_params.map(|v| *v),
                    subr.default_params,
                    subr.kw_var_params.map(|v| *v),
                    return_t,
                );
                if errs.is_empty() {
                    Ok(subr)
                } else {
                    Err((subr, errs))
                }
            }
            Type::Callable { param_ts, return_t } => {
                let mut new_param_ts = Vec::with_capacity(param_ts.len());
                for pt in param_ts {
                    let pt = match self.eval_t_params(pt, level, t_loc) {
                        Ok(pt) => pt,
                        Err((pt, es)) => {
                            errs.extend(es);
                            pt
                        }
                    };
                    new_param_ts.push(pt);
                }
                let return_t = match self.eval_t_params(*return_t, level, t_loc) {
                    Ok(return_t) => return_t,
                    Err((return_t, es)) => {
                        errs.extend(es);
                        return_t
                    }
                };
                let callable = callable(new_param_ts, return_t);
                if errs.is_empty() {
                    Ok(callable)
                } else {
                    Err((callable, errs))
                }
            }
            Type::Quantified(quant) => match self.eval_t_params(*quant, level, t_loc) {
                Ok(t) => Ok(t.quantify()),
                Err((t, es)) => Err((t.quantify(), es)),
            },
            Type::Refinement(refine) => {
                if refine.pred.variables().is_empty() {
                    let pred = match self.eval_pred(*refine.pred) {
                        Ok(pred) => pred,
                        Err((pred, es)) => {
                            errs.extend(es);
                            pred
                        }
                    };
                    Ok(refinement(refine.var, *refine.t, pred))
                } else {
                    Ok(Type::Refinement(refine))
                }
            }
            Type::Proj { lhs, rhs } => self
                .eval_proj(*lhs, rhs, level, t_loc)
                .map_err(|errs| (Failure, errs)),
            Type::ProjCall {
                lhs,
                attr_name,
                args,
            } => self
                .eval_proj_call_t(*lhs, attr_name, args, level, t_loc)
                .map_err(|errs| (Failure, errs)),
            Type::Ref(l) => match self.eval_t_params(*l, level, t_loc) {
                Ok(t) => Ok(ref_(t)),
                Err((t, errs)) => Err((ref_(t), errs)),
            },
            Type::RefMut { before, after } => {
                let before = match self.eval_t_params(*before, level, t_loc) {
                    Ok(before) => before,
                    Err((before, es)) => {
                        errs.extend(es);
                        before
                    }
                };
                let after = if let Some(after) = after {
                    let aft = match self.eval_t_params(*after, level, t_loc) {
                        Ok(aft) => aft,
                        Err((aft, es)) => {
                            errs.extend(es);
                            aft
                        }
                    };
                    Some(aft)
                } else {
                    None
                };
                if errs.is_empty() {
                    Ok(ref_mut(before, after))
                } else {
                    Err((ref_mut(before, after), errs))
                }
            }
            Type::Poly { name, mut params } => {
                for p in params.iter_mut() {
                    *p = match self.eval_tp(mem::take(p)) {
                        Ok(p) => p,
                        Err((p, es)) => {
                            errs.extend(es);
                            p
                        }
                    };
                }
                if let Some(ValueObj::Subr(subr)) = self.rec_get_const_obj(&name) {
                    if let Ok(args) = self.convert_args(None, subr, params.clone(), t_loc) {
                        let ret = self.call(subr.clone(), args, t_loc);
                        if let Some(t) = ret.ok().and_then(|tp| self.convert_tp_into_type(tp).ok())
                        {
                            return Ok(t);
                        }
                    }
                }
                let t = poly(name, params);
                if errs.is_empty() {
                    Ok(t)
                } else {
                    Err((t, errs))
                }
            }
            Type::And(ands, _) => {
                let mut new_ands = set! {};
                for and in ands.into_iter() {
                    match self.eval_t_params(and, level, t_loc) {
                        Ok(and) => {
                            new_ands.insert(and);
                        }
                        Err((and, es)) => {
                            new_ands.insert(and);
                            errs.extend(es);
                        }
                    }
                }
                let intersec = new_ands
                    .into_iter()
                    .fold(Type::Obj, |l, r| self.intersection(&l, &r));
                if errs.is_empty() {
                    Ok(intersec)
                } else {
                    Err((intersec, errs))
                }
            }
            Type::Or(ors) => {
                let mut new_ors = set! {};
                for or in ors.into_iter() {
                    match self.eval_t_params(or, level, t_loc) {
                        Ok(or) => {
                            new_ors.insert(or);
                        }
                        Err((or, es)) => {
                            new_ors.insert(or);
                            errs.extend(es);
                        }
                    }
                }
                let union = new_ors.into_iter().fold(Never, |l, r| self.union(&l, &r));
                if errs.is_empty() {
                    Ok(union)
                } else {
                    Err((union, errs))
                }
            }
            Type::Not(ty) => match self.eval_t_params(*ty, level, t_loc) {
                Ok(ty) => Ok(self.complement(&ty)),
                Err((ty, errs)) => Err((self.complement(&ty), errs)),
            },
            Type::Structural(typ) => {
                // TODO: avoid infinite recursion
                set_recursion_limit!(Ok(typ.structuralize()), 128);
                match self.eval_t_params(*typ, level, t_loc) {
                    Ok(typ) => Ok(typ.structuralize()),
                    Err((t, errs)) => Err((t.structuralize(), errs)),
                }
            }
            Type::Record(rec) => {
                let mut fields = dict! {};
                for (name, ty) in rec.into_iter() {
                    match self.eval_t_params(ty, level, t_loc) {
                        Ok(ty) => {
                            fields.insert(name, ty);
                        }
                        Err((tp, es)) => {
                            fields.insert(name, tp);
                            errs.extend(es);
                        }
                    }
                }
                Ok(Type::Record(fields))
            }
            Type::NamedTuple(tuple) => {
                let mut new_tuple = vec![];
                for (name, ty) in tuple.into_iter() {
                    match self.eval_t_params(ty, level, t_loc) {
                        Ok(ty) => new_tuple.push((name, ty)),
                        Err((ty, es)) => {
                            new_tuple.push((name, ty));
                            errs.extend(es);
                        }
                    }
                }
                Ok(Type::NamedTuple(new_tuple))
            }
            Type::Bounded { sub, sup } => {
                let sub = match self.eval_t_params(*sub, level, t_loc) {
                    Ok(sub) => sub,
                    Err((sub, es)) => {
                        errs.extend(es);
                        sub
                    }
                };
                let sup = match self.eval_t_params(*sup, level, t_loc) {
                    Ok(sup) => sup,
                    Err((sup, es)) => {
                        errs.extend(es);
                        sup
                    }
                };
                if errs.is_empty() {
                    Ok(bounded(sub, sup))
                } else {
                    Err((bounded(sub, sup), errs))
                }
            }
            Type::Guard(grd) => match self.eval_t_params(*grd.to, level, t_loc) {
                Ok(to) => Ok(guard(grd.namespace, *grd.target, to)),
                Err((to, es)) => Err((guard(grd.namespace, *grd.target, to), es)),
            },
            mono_type_pattern!() => Ok(substituted),
        }
    }

    /// This may do nothing (be careful with recursive calls).
    /// lhs: mainly class, may be unevaluated
    /// ```erg
    /// [?T; 0].MutType! == ![?T; 0]
    /// ?T(<: Add(?R(:> Int))).Output == ?T(<: Add(?R)).Output
    /// ?T(:> Int, <: Add(?R(:> Int))).Output == Int
    /// ```
    pub(crate) fn eval_proj(
        &self,
        lhs: Type,
        rhs: Str,
        level: usize,
        t_loc: &impl Locational,
    ) -> EvalResult<Type> {
        let lhs = self
            .eval_t_params(lhs, level, t_loc)
            .map_err(|(_, errs)| errs)?;
        if let Never | Failure = lhs {
            return Ok(lhs);
        }
        // Currently Erg does not allow projection-types to be evaluated with type variables included.
        // All type variables will be dereferenced or fail.
        let (sub, opt_sup) = match lhs.clone() {
            Type::FreeVar(fv) if fv.is_linked() => {
                let t = fv.crack().clone();
                return self
                    .eval_t_params(proj(t, rhs), level, t_loc)
                    .map_err(|(_, errs)| errs);
            }
            Type::FreeVar(fv) if fv.get_subsup().is_some() => {
                let (sub, sup) = fv.get_subsup().unwrap();
                (sub, Some(sup))
            }
            other => (other, None),
        };
        // cannot determine at this point
        if sub == Type::Never {
            return Ok(proj(lhs, rhs));
        }
        // in Methods
        if let Some(ctx) = self.get_same_name_context(&sub.qual_name()) {
            match ctx.validate_and_project(&sub, opt_sup.as_ref(), &rhs, self, None, level, t_loc) {
                Triple::Ok(t) => return Ok(t),
                Triple::Err(err) => return Err(err),
                Triple::None => {}
            }
        }
        let ty_ctxs = match self.get_nominal_super_type_ctxs(&sub) {
            Some(ty_ctxs) => ty_ctxs,
            None => {
                // If the type is already poisoned by a previous error (e.g. `X = X + 1`,
                // where `X` is registered as `{Failure}`), do not report a redundant error
                if sub.is_failure() || sub.contains_failure() {
                    return Ok(Failure);
                }
                let errs = EvalErrors::from(EvalError::type_not_found(
                    self.cfg.input.clone(),
                    line!() as usize,
                    t_loc.loc(),
                    self.caused_by(),
                    &sub,
                ));
                return Err(errs);
            }
        };
        for ty_ctx in ty_ctxs {
            match self.validate_and_project(
                &sub,
                opt_sup.as_ref(),
                &rhs,
                ty_ctx,
                Some(&ty_ctx.typ),
                level,
                t_loc,
            ) {
                Triple::Ok(t) => return Ok(t),
                Triple::Err(err) => return Err(err),
                Triple::None => {}
            }
            for methods in ty_ctx.methods_list.iter() {
                match (&methods.typ, &opt_sup) {
                    (ClassDefType::ImplTrait { impl_trait, .. }, Some(sup)) => {
                        if !self.supertype_of(impl_trait, sup) {
                            continue;
                        }
                    }
                    (ClassDefType::ImplTrait { impl_trait, .. }, None)
                        if !self.supertype_of(impl_trait, &sub) =>
                    {
                        continue;
                    }
                    _ => {}
                }
                match self.validate_and_project(
                    &sub,
                    opt_sup.as_ref(),
                    &rhs,
                    methods,
                    Some(&ty_ctx.typ),
                    level,
                    t_loc,
                ) {
                    Triple::Ok(t) => return Ok(t),
                    Triple::Err(err) => return Err(err),
                    Triple::None => {}
                }
            }
        }
        if let Some((sub, sup)) = lhs.as_free().and_then(|fv| fv.get_subsup()) {
            if self.is_trait(&sup) && !self.trait_impl_exists(&sub, &sup) {
                // link to `Never..Obj` to prevent double errors from being reported
                lhs.destructive_link(&bounded(Never, Type::Obj));
                let sub = if cfg!(feature = "debug") {
                    sub
                } else {
                    self.coerce(sub, t_loc)?
                };
                let sup = if cfg!(feature = "debug") {
                    sup
                } else {
                    self.coerce(sup, t_loc)?
                };
                return Err(EvalErrors::from(EvalError::no_trait_impl_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    &sub,
                    &sup,
                    t_loc.loc(),
                    self.caused_by(),
                    self.get_simple_type_mismatch_hint(&sup, &sub),
                )));
            }
        }
        // if the target can't be found in the supertype, the type will be dereferenced.
        // In many cases, it is still better to determine the type variable than if the target is not found.
        let coerced = self.coerce(lhs.clone(), t_loc)?;
        if lhs != coerced {
            let proj = proj(coerced, rhs);
            self.eval_t_params(proj, level, t_loc)
                .map_err(|(_, errs)| errs)
        } else {
            let proj = proj(lhs, rhs);
            let errs = EvalErrors::from(EvalError::no_candidate_error(
                self.cfg.input.clone(),
                line!() as usize,
                &proj,
                t_loc.loc(),
                self.caused_by(),
                self.get_no_candidate_hint(&proj),
            ));
            Err(errs)
        }
    }

    /// lhs: may be unevaluated
    pub(crate) fn eval_tp_proj(
        &self,
        lhs: TyParam,
        rhs: Str,
        t_loc: &impl Locational,
    ) -> EvalResult<TyParam> {
        let lhs = self.eval_tp(lhs).map_err(|(_, errs)| errs)?;
        // in Methods
        if let Some(ctx) = lhs.qual_name().and_then(|name| {
            self.get_same_name_context(&name)
                .or_else(|| self.get_mod(&name))
        }) {
            if let Some(value) = ctx.rec_get_const_obj(&rhs) {
                return Ok(TyParam::value(value.clone()));
            }
        }
        let ty_ctxs = match self
            .get_tp_t(&lhs)
            .ok()
            .and_then(|t| self.get_nominal_super_type_ctxs(&t))
        {
            Some(ty_ctxs) => ty_ctxs,
            None => {
                let errs = EvalErrors::from(EvalError::type_not_found(
                    self.cfg.input.clone(),
                    line!() as usize,
                    t_loc.loc(),
                    self.caused_by(),
                    &Type::Obj,
                ));
                return Err(errs);
            }
        };
        for ty_ctx in ty_ctxs {
            if let Some(value) = ty_ctx.rec_get_const_obj(&rhs) {
                return Ok(TyParam::value(value.clone()));
            }
            for methods in ty_ctx.methods_list.iter() {
                if let Some(value) = methods.rec_get_const_obj(&rhs) {
                    return Ok(TyParam::value(value.clone()));
                }
            }
        }
        Ok(lhs.proj(rhs))
    }

    /// lhs: may be unevaluated
    pub(crate) fn eval_value_proj(
        &self,
        lhs: TyParam,
        rhs: Str,
        t_loc: &impl Locational,
    ) -> EvalResult<Result<ValueObj, TyParam>> {
        match self.eval_tp_proj(lhs, rhs, t_loc) {
            Ok(TyParam::Value(val)) => Ok(Ok(val)),
            Ok(tp) => Ok(Err(tp)),
            Err(errs) => Err(errs),
        }
    }

    pub fn eval_value_proj_call(
        &self,
        lhs: TyParam,
        attr_name: Str,
        args: Vec<TyParam>,
        t_loc: &impl Locational,
    ) -> EvalResult<Result<ValueObj, TyParam>> {
        match self.eval_proj_call(lhs, attr_name, args, t_loc) {
            Ok(TyParam::Value(val)) => Ok(Ok(val)),
            Ok(tp) => Ok(Err(tp)),
            Err(errs) => Err(errs),
        }
    }

    /// ```erg
    /// TyParam::Type(Int) => Int
    /// [{1}, {2}, {3}] => [{1, 2, 3}; 3]
    /// (Int, Str) => Tuple([Int, Str])
    /// {x = Int; y = Int} => Type::Record({x = Int, y = Int})
    /// {Str: Int} => Dict({Str: Int})
    /// {1, 2} => {I: Int | I == 1 or I == 2 } (== {1, 2})
    /// foo.T => Ok(foo.T) if T: Type else Err(foo.T)
    /// ```
    pub(crate) fn convert_tp_into_type(&self, tp: TyParam) -> Result<Type, TyParam> {
        match tp {
            TyParam::Tuple(tps) => {
                let mut ts = vec![];
                for elem_tp in tps {
                    ts.push(self.convert_tp_into_type(elem_tp)?);
                }
                Ok(tuple_t(ts))
            }
            TyParam::List(tps) => {
                let mut union = Type::Never;
                let len = tps.len();
                for tp in tps {
                    union = self.union(&union, &self.convert_tp_into_type(tp)?);
                }
                Ok(list_t(union, TyParam::value(len)))
            }
            TyParam::UnsizedList(elem) => {
                let elem = self.convert_tp_into_type(*elem)?;
                Ok(unknown_len_list_t(elem))
            }
            TyParam::Set(tps) => {
                let mut union = Type::Never;
                for tp in tps.iter() {
                    union = self.union(&union, &self.get_tp_t(tp).unwrap_or(Type::Obj));
                }
                Ok(tp_enum(union, tps))
            }
            TyParam::Record(rec) => {
                let mut fields = dict! {};
                for (name, tp) in rec {
                    fields.insert(name, self.convert_tp_into_type(tp)?);
                }
                Ok(Type::Record(fields))
            }
            TyParam::Dict(dict) => {
                let mut kvs = dict! {};
                for (key, val) in dict {
                    kvs.insert(
                        self.convert_tp_into_type(key)?,
                        self.convert_tp_into_type(val)?,
                    );
                }
                Ok(Type::from(kvs))
            }
            TyParam::FreeVar(fv) if fv.is_linked() => {
                let t = fv.crack().clone();
                self.convert_tp_into_type(t)
            }
            // TyParam(Ts: List(Type)) -> Type(Ts: List(Type))
            // TyParam(?S(: Str)) -> Err(...),
            // TyParam(?D(: GenericDict)) -> Ok(?D(: GenericDict)),
            // FIXME: GenericDict
            TyParam::FreeVar(fv)
                if fv.get_type().is_some_and(|t| {
                    &t.qual_name() == "GenericDict" || self.subtype_of(&t, &Type::Type)
                }) =>
            {
                // FIXME: This procedure is clearly erroneous because it breaks the type variable linkage.
                Ok(named_free_var(
                    fv.unbound_name().unwrap(),
                    fv.level().unwrap(),
                    fv.constraint().unwrap(),
                ))
            }
            TyParam::Type(t) => Ok(t.as_ref().clone()),
            TyParam::Mono(name) => Ok(Type::Mono(name)),
            TyParam::App { name, args } => {
                if self.get_type_ctx(&name).is_none() {
                    Err(TyParam::App { name, args })
                } else {
                    Ok(Type::Poly { name, params: args })
                }
            }
            TyParam::BinOp { op, lhs, rhs } => match op {
                OpKind::And => {
                    let lhs = self.convert_tp_into_type(*lhs)?;
                    let rhs = self.convert_tp_into_type(*rhs)?;
                    Ok(self.intersection(&lhs, &rhs))
                }
                OpKind::Or => {
                    let lhs = self.convert_tp_into_type(*lhs)?;
                    let rhs = self.convert_tp_into_type(*rhs)?;
                    Ok(self.union(&lhs, &rhs))
                }
                _ => Err(TyParam::BinOp { op, lhs, rhs }),
            },
            TyParam::UnaryOp { op, val } => match op {
                OpKind::Not => {
                    let val = self.convert_tp_into_type(*val)?;
                    Ok(self.complement(&val))
                }
                _ => Err(TyParam::UnaryOp { op, val }),
            },
            TyParam::Proj { obj, attr } => {
                let lhs = self.convert_tp_into_type(*obj.clone())?;
                let Some(ty_ctx) = self.get_nominal_type_ctx(&lhs) else {
                    return Err(TyParam::Proj { obj, attr });
                };
                if ty_ctx.rec_get_const_obj(&attr).is_some() {
                    Ok(lhs.proj(attr))
                } else {
                    Err(TyParam::Proj { obj, attr })
                }
            }
            TyParam::ProjCall { obj, attr, args } => {
                let Some(ty_ctx) = self
                    .get_tp_t(&obj)
                    .ok()
                    .and_then(|t| self.get_nominal_type_ctx(&t))
                else {
                    return Err(TyParam::ProjCall { obj, attr, args });
                };
                if ty_ctx.rec_get_const_obj(&attr).is_some() {
                    Ok(proj_call(*obj, attr, args))
                } else {
                    Err(TyParam::ProjCall { obj, attr, args })
                }
            }
            // TyParam::Erased(_t) => Ok(Type::Obj),
            TyParam::Value(v) => self.convert_value_into_type(v).map_err(TyParam::Value),
            TyParam::Erased(t) if t.is_type() => Ok(Type::Obj),
            TyParam::Failure => Ok(Type::Failure),
            other => Err(other),
        }
    }

    pub(crate) fn convert_tp_into_value(&self, tp: TyParam) -> Result<ValueObj, TyParam> {
        match tp {
            TyParam::FreeVar(fv) if fv.is_linked() => {
                let tp = fv.crack().clone();
                self.convert_tp_into_value(tp)
            }
            TyParam::Value(v) => Ok(v),
            TyParam::List(lis) => {
                let mut new = vec![];
                for elem in lis {
                    let elem = self.convert_tp_into_value(elem)?;
                    new.push(elem);
                }
                Ok(ValueObj::List(new.into()))
            }
            TyParam::UnsizedList(elem) => {
                let elem = self.convert_tp_into_value(*elem)?;
                Ok(ValueObj::UnsizedList(Box::new(elem)))
            }
            TyParam::Tuple(tys) => {
                let mut new = vec![];
                for elem in tys {
                    let elem = self.convert_tp_into_value(elem)?;
                    new.push(elem);
                }
                Ok(ValueObj::Tuple(new.into()))
            }
            TyParam::Record(rec) => {
                let mut new = dict! {};
                for (name, elem) in rec {
                    let elem = self.convert_tp_into_value(elem)?;
                    new.insert(name, elem);
                }
                Ok(ValueObj::Record(new))
            }
            TyParam::Dict(tps) => {
                let mut vals = dict! {};
                for (k, v) in tps {
                    vals.insert(
                        self.convert_tp_into_value(k)?,
                        self.convert_tp_into_value(v)?,
                    );
                }
                Ok(ValueObj::Dict(vals))
            }
            TyParam::Set(set) => {
                let mut new = set! {};
                for elem in set {
                    let elem = self.convert_tp_into_value(elem)?;
                    new.insert(elem);
                }
                Ok(ValueObj::Set(new))
            }
            TyParam::App { name, args } => {
                let mut new = vec![];
                for elem in args.iter() {
                    let elem = self.convert_tp_into_value(elem.clone())?;
                    new.push(elem);
                }
                let Some(ValueObj::Subr(subr)) = self.rec_get_const_obj(&name) else {
                    return Err(TyParam::App { name, args });
                };
                let new = ValueArgs::pos_only(new);
                match self.call(subr.clone(), new, Location::Unknown) {
                    Ok(TyParam::Value(val)) => Ok(val),
                    _ => Err(TyParam::App { name, args }),
                }
            }
            TyParam::Lambda(lambda) => {
                let name = Str::from("<lambda>");
                let params = lambda.const_.sig.params;
                let block = lambda.const_.body;
                let sig_t = func(
                    lambda.nd_params,
                    lambda.var_params,
                    lambda.d_params,
                    lambda.kw_var_params,
                    // TODO:
                    Type::Obj,
                );
                Ok(ValueObj::Subr(ConstSubr::User(UserConstSubr::new(
                    name, params, block, sig_t, None,
                ))))
            }
            TyParam::DataClass { name, fields } => {
                let mut new = dict! {};
                for (name, elem) in fields {
                    let elem = self.convert_tp_into_value(elem)?;
                    new.insert(name, elem);
                }
                Ok(ValueObj::DataClass { name, fields: new })
            }
            TyParam::Proj { obj, attr } => {
                match self.eval_value_proj(*obj.clone(), attr.clone(), &()) {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(tp)) => self.convert_tp_into_type(tp).map(ValueObj::builtin_type),
                    Err(_) => Err(obj.proj(attr)),
                }
            }
            TyParam::ProjCall { obj, attr, args } => {
                match self.eval_value_proj_call(*obj.clone(), attr.clone(), args.clone(), &()) {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(tp)) => self.convert_tp_into_type(tp).map(ValueObj::builtin_type),
                    Err(_) => Err(obj.proj(attr)),
                }
            }
            TyParam::Type(t) => Ok(ValueObj::builtin_type(*t)),
            TyParam::Failure => Ok(ValueObj::Failure),
            other => self.convert_tp_into_type(other).map(ValueObj::builtin_type),
        }
    }

    pub(crate) fn convert_singular_type_into_value(&self, typ: Type) -> Result<ValueObj, Type> {
        match typ {
            Type::FreeVar(fv) if fv.is_linked() => {
                let t = fv.crack().clone();
                self.convert_singular_type_into_value(t)
            }
            Type::Refinement(ref refine) => {
                if let Predicate::Equal { rhs, .. } = refine.pred.as_ref() {
                    self.convert_tp_into_value(rhs.clone()).map_err(|_| typ)
                } else {
                    Err(typ)
                }
            }
            Type::Quantified(quant) => self.convert_singular_type_into_value(*quant),
            Type::Subr(subr) => self.convert_singular_type_into_value(*subr.return_t),
            Type::Proj { lhs, rhs } => {
                let old = lhs.clone().proj(rhs.clone());
                let evaled = self.eval_proj(*lhs, rhs, 0, &()).map_err(|_| old.clone())?;
                if old != evaled {
                    self.convert_singular_type_into_value(evaled)
                } else {
                    Err(old)
                }
            }
            Type::ProjCall {
                lhs,
                attr_name,
                args,
            } => {
                let old = Type::ProjCall {
                    lhs: lhs.clone(),
                    attr_name: attr_name.clone(),
                    args: args.clone(),
                };
                let evaled = self
                    .eval_proj_call_t(*lhs, attr_name, args, 0, &())
                    .map_err(|_| old.clone())?;
                if old != evaled {
                    self.convert_singular_type_into_value(evaled)
                } else {
                    Err(old)
                }
            }
            Type::Failure => Ok(ValueObj::Failure),
            _ => Err(typ),
        }
    }

    // FIXME: Failable<Type>
    pub(crate) fn convert_value_into_type(&self, val: ValueObj) -> Result<Type, ValueObj> {
        match val {
            ValueObj::Failure => Ok(Type::Failure),
            ValueObj::Ellipsis => Ok(Type::Ellipsis),
            ValueObj::NotImplemented => Ok(Type::NotImplementedType),
            ValueObj::Inf => Ok(Type::Inf),
            ValueObj::NegInf => Ok(Type::NegInf),
            ValueObj::Type(t) => Ok(t.into_typ()),
            ValueObj::Record(rec) => {
                let mut fields = dict! {};
                for (name, val) in rec.into_iter() {
                    fields.insert(name, self.convert_value_into_type(val)?);
                }
                Ok(Type::Record(fields))
            }
            ValueObj::Tuple(ts) => {
                let mut new_ts = vec![];
                for v in ts.iter() {
                    new_ts.push(self.convert_value_into_type(v.clone())?);
                }
                Ok(tuple_t(new_ts))
            }
            ValueObj::List(lis) => {
                let len = TyParam::value(lis.len());
                let mut union = Type::Never;
                for v in lis.iter().cloned() {
                    union = self.union(&union, &self.convert_value_into_type(v)?);
                }
                Ok(list_t(union, len))
            }
            ValueObj::UnsizedList(elem) => {
                let elem = self.convert_value_into_type(*elem)?;
                Ok(unknown_len_list_t(elem))
            }
            ValueObj::Set(set) => try_v_enum(set).map_err(ValueObj::Set),
            ValueObj::Dict(dic) => {
                let dic = dic
                    .into_iter()
                    .map(|(k, v)| (TyParam::Value(k), TyParam::Value(v)))
                    .collect();
                Ok(dict_t(TyParam::Dict(dic)))
            }
            ValueObj::Subr(subr) => subr.as_type(self).ok_or(ValueObj::Subr(subr)),
            ValueObj::DataClass { name, fields }
                if ValueObj::as_interval_op(&name).is_some()
                    && fields.get("start").is_some()
                    && fields.get("end").is_some() =>
            {
                let op = ValueObj::as_interval_op(&name).unwrap();
                let start = fields["start"].clone();
                let end = fields["end"].clone();
                Ok(interval(op, start.class(), start, end))
            }
            // TODO:
            ValueObj::DataClass { .. }
            | ValueObj::Int(_)
            | ValueObj::Nat(_)
            | ValueObj::Ratio(..)
            | ValueObj::Bool(_)
            | ValueObj::Float(_)
            | ValueObj::Code(_)
            | ValueObj::Str(_)
            | ValueObj::None => Err(val),
        }
    }

    /// * Ok if `value` can be upcast to `TyParam`
    /// * Err if it is simply converted to `TyParam::Value(value)`
    pub(crate) fn convert_value_into_tp(value: ValueObj) -> Result<TyParam, TyParam> {
        match value {
            ValueObj::Type(t) => Ok(TyParam::t(t.into_typ())),
            ValueObj::List(lis) => {
                let mut new_lis = vec![];
                for v in lis.iter().cloned() {
                    let tp = match Self::convert_value_into_tp(v) {
                        Ok(tp) => tp,
                        Err(tp) => tp,
                    };
                    new_lis.push(tp);
                }
                Ok(TyParam::List(new_lis))
            }
            ValueObj::UnsizedList(elem) => {
                let tp = match Self::convert_value_into_tp(*elem) {
                    Ok(tp) => tp,
                    Err(tp) => tp,
                };
                Ok(TyParam::UnsizedList(Box::new(tp)))
            }
            ValueObj::Tuple(vs) => {
                let mut new_ts = vec![];
                for v in vs.iter().cloned() {
                    let tp = match Self::convert_value_into_tp(v) {
                        Ok(tp) => tp,
                        Err(tp) => tp,
                    };
                    new_ts.push(tp);
                }
                Ok(TyParam::Tuple(new_ts))
            }
            ValueObj::Dict(dict) => {
                let mut new_dict = dict! {};
                for (k, v) in dict.into_iter() {
                    let k = match Self::convert_value_into_tp(k) {
                        Ok(tp) => tp,
                        Err(tp) => tp,
                    };
                    let v = match Self::convert_value_into_tp(v) {
                        Ok(tp) => tp,
                        Err(tp) => tp,
                    };
                    new_dict.insert(k, v);
                }
                Ok(TyParam::Dict(new_dict))
            }
            ValueObj::Set(set) => {
                let mut new_set = set! {};
                for v in set.into_iter() {
                    let tp = match Self::convert_value_into_tp(v) {
                        Ok(tp) => tp,
                        Err(tp) => tp,
                    };
                    new_set.insert(tp);
                }
                Ok(TyParam::Set(new_set))
            }
            ValueObj::Record(rec) => {
                let mut new_rec = dict! {};
                for (k, v) in rec.into_iter() {
                    let v = match Self::convert_value_into_tp(v) {
                        Ok(tp) => tp,
                        Err(tp) => tp,
                    };
                    new_rec.insert(k, v);
                }
                Ok(TyParam::Record(new_rec))
            }
            ValueObj::DataClass { name, fields } => {
                let mut new_fields = dict! {};
                for (k, v) in fields.into_iter() {
                    let v = match Self::convert_value_into_tp(v) {
                        Ok(tp) => tp,
                        Err(tp) => tp,
                    };
                    new_fields.insert(k, v);
                }
                Ok(TyParam::DataClass {
                    name,
                    fields: new_fields,
                })
            }
            ValueObj::Failure => Ok(TyParam::Failure),
            ValueObj::Subr(_) => Err(TyParam::Value(value)),
            mono_value_pattern!(-Failure) => Err(TyParam::Value(value)),
        }
    }

    pub(crate) fn convert_type_to_dict_type(&self, ty: Type) -> Result<Dict<Type, Type>, ()> {
        match ty {
            Type::FreeVar(fv) if fv.is_linked() => {
                let t = fv.crack().clone();
                self.convert_type_to_dict_type(t)
            }
            Type::Refinement(refine) => self.convert_type_to_dict_type(*refine.t),
            Type::Poly { params, .. } if ty.is_dict() || ty.is_dict_mut() => {
                let dict = Dict::try_from(params[0].clone())?;
                let mut new_dict = dict! {};
                for (k, v) in dict.into_iter() {
                    let k = self.convert_tp_into_type(k).map_err(|_| ())?;
                    let v = self.convert_tp_into_type(v).map_err(|_| ())?;
                    new_dict.insert(k, v);
                }
                Ok(new_dict)
            }
            Type::Proj { lhs, rhs } => {
                let old = proj(*lhs.clone(), rhs.clone());
                let eval = self.eval_proj(*lhs, rhs, self.level, &()).map_err(|_| ())?;
                if eval != old {
                    self.convert_type_to_dict_type(eval)
                } else {
                    Err(())
                }
            }
            Type::ProjCall {
                lhs,
                attr_name,
                args,
            } => {
                let old = proj_call(*lhs.clone(), attr_name.clone(), args.clone());
                let eval = self
                    .eval_proj_call_t(*lhs, attr_name, args, self.level, &())
                    .map_err(|_| ())?;
                if eval != old {
                    self.convert_type_to_dict_type(eval)
                } else {
                    Err(())
                }
            }
            Type::Failure => Ok(dict! { Failure => Failure }),
            _ => Err(()),
        }
    }

    pub(crate) fn convert_value_to_dict(
        &self,
        val: &ValueObj,
    ) -> Result<Dict<ValueObj, ValueObj>, ()> {
        match val {
            ValueObj::Dict(dic) => Ok(dic.clone()),
            ValueObj::Type(ty) => {
                let Ok(dict) = self
                    .convert_type_to_dict_type(ty.typ().clone())
                    .map(|dict| {
                        dict.into_iter()
                            .map(|(k, v)| (ValueObj::builtin_type(k), ValueObj::builtin_type(v)))
                            .collect()
                    })
                else {
                    return Err(());
                };
                Ok(dict)
            }
            ValueObj::Failure => Ok(dict! { ValueObj::Failure => ValueObj::Failure }),
            _ => Err(()),
        }
    }

    pub(crate) fn convert_type_to_tuple_type(&self, ty: Type) -> Result<Vec<Type>, ()> {
        match ty {
            Type::FreeVar(fv) if fv.is_linked() => {
                let t = fv.crack().clone();
                self.convert_type_to_tuple_type(t)
            }
            Type::Refinement(refine) => self.convert_type_to_tuple_type(*refine.t),
            Type::Poly { name, params } if &name[..] == "Tuple" => {
                let tps = Vec::try_from(params[0].clone())?;
                let mut tys = vec![];
                for elem in tps.into_iter() {
                    let elem = self.convert_tp_into_type(elem).map_err(|_| ())?;
                    tys.push(elem);
                }
                Ok(tys)
            }
            Type::NamedTuple(tuple) => Ok(tuple.into_iter().map(|(_, ty)| ty).collect()),
            Type::Proj { lhs, rhs } => {
                let old = proj(*lhs.clone(), rhs.clone());
                let eval = self.eval_proj(*lhs, rhs, self.level, &()).map_err(|_| ())?;
                if eval != old {
                    self.convert_type_to_tuple_type(eval)
                } else {
                    Err(())
                }
            }
            Type::ProjCall {
                lhs,
                attr_name,
                args,
            } => {
                let old = proj_call(*lhs.clone(), attr_name.clone(), args.clone());
                let eval = self
                    .eval_proj_call_t(*lhs, attr_name, args, self.level, &())
                    .map_err(|_| ())?;
                if eval != old {
                    self.convert_type_to_tuple_type(eval)
                } else {
                    Err(())
                }
            }
            // FIXME: unsized tuple
            Type::Failure => Ok(vec![Type::Failure; 100]),
            _ => Err(()),
        }
    }

    pub(crate) fn convert_type_to_list(&self, ty: Type) -> Result<Vec<ValueObj>, Type> {
        match ty {
            Type::FreeVar(fv) if fv.is_linked() => {
                let t = fv.crack().clone();
                self.convert_type_to_list(t)
            }
            Type::Refinement(refine) => self.convert_type_to_list(*refine.t),
            Type::Poly { name, params } if &name[..] == "List" || &name[..] == "List!" => {
                let Ok(t) = self.convert_tp_into_type(params[0].clone()) else {
                    log!(err "cannot convert to type: {}", params[0]);
                    return Err(poly(name, params));
                };
                let Ok(len) = usize::try_from(&params[1]) else {
                    log!(err "cannot convert to usize: {}", params[1]);
                    if DEBUG_MODE {
                        panic!("cannot convert to usize: {}", params[1]);
                    }
                    return Err(poly(name, params));
                };
                Ok(vec![ValueObj::builtin_type(t); len])
            }
            Type::Proj { lhs, rhs } => {
                let old = proj(*lhs.clone(), rhs.clone());
                match self.eval_proj(*lhs, rhs, self.level, &()) {
                    Ok(eval) if eval != old => self.convert_type_to_list(eval),
                    _ => Err(old),
                }
            }
            Type::ProjCall {
                lhs,
                attr_name,
                args,
            } => {
                let old = proj_call(*lhs.clone(), attr_name.clone(), args.clone());
                match self.eval_proj_call_t(*lhs, attr_name, args, self.level, &()) {
                    Ok(eval) if eval != old => self.convert_type_to_list(eval),
                    _ => Err(old),
                }
            }
            // FIXME: unsized list
            Type::Failure => Ok(vec![ValueObj::Failure; 100]),
            _ => Err(ty),
        }
    }

    pub(crate) fn convert_value_into_list(&self, val: ValueObj) -> Result<Vec<ValueObj>, ValueObj> {
        match val {
            ValueObj::List(lis) => Ok(lis.to_vec()),
            ValueObj::Tuple(ts) => Ok(ts.to_vec()),
            ValueObj::Type(t) => self
                .convert_type_to_list(t.into_typ())
                .map_err(ValueObj::builtin_type),
            // FIXME: unsized list
            ValueObj::Failure => Ok(vec![ValueObj::Failure; 100]),
            _ => Err(val),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_and_project(
        &self,
        sub: &Type,
        opt_sup: Option<&Type>,
        rhs: &str,
        methods: &Context,
        methods_type: Option<&Type>,
        level: usize,
        t_loc: &impl Locational,
    ) -> Triple<Type, EvalErrors> {
        // e.g. sub: Int, opt_sup: Add(?T), rhs: Output, methods: Int.methods
        //      sub: [Int; 4], opt_sup: Add([Int; 2]), rhs: Output, methods: [T; N].methods
        if let Ok(obj) = methods.get_const_local(&Token::symbol(rhs), &self.name) {
            #[allow(clippy::single_match)]
            // opt_sup: Add(?T), methods.impl_of(): Add(Int)
            // opt_sup: Add([Int; 2]), methods.impl_of(): Add([T; M])
            match (&opt_sup, methods.impl_of()) {
                (Some(sup), Some(trait_)) if !self.supertype_of(&trait_, sup) => {
                    return Triple::None;
                }
                _ => {}
            }
            // obj: Int|<: Add(Int)|.Output == ValueObj::Type(<type Int>)
            // obj: [T; N]|<: Add([T; M])|.Output == ValueObj::Type(<type [T; M+N]>)
            if let ValueObj::Type(quant_projected_t) = obj {
                let projected_t = quant_projected_t.into_typ();
                let quant_sub = self.get_type_ctx(&sub.qual_name()).map(|ctx| &ctx.typ);
                let _sup_subs = if let Some((sup, quant_sup)) = opt_sup.zip(methods.impl_of()) {
                    // T -> Int, M -> 2
                    match Substituter::substitute_typarams(self, &quant_sup, sup) {
                        Ok(sub_subs) => sub_subs,
                        Err(errs) => {
                            return Triple::Err(errs);
                        }
                    }
                } else {
                    None
                };
                // T -> Int, N -> 4
                /*let _sub_subs = if quant_sub.has_undoable_linked_var() {
                    Substituter::overwrite_typarams(self, quant_sub, sub).ok()?
                } else {
                    Substituter::substitute_typarams(self, quant_sub, sub).ok()?
                };*/
                let _sub_subs =
                    match quant_sub.map(|qsub| Substituter::substitute_typarams(self, qsub, sub)) {
                        Some(Ok(sub_subs)) => sub_subs,
                        Some(Err(errs)) => {
                            return Triple::Err(errs);
                        }
                        None => None,
                    };
                let _met_t_subs = match methods_type
                    .map(|met_t| Substituter::substitute_typarams(self, met_t, sub))
                {
                    Some(Ok(subs)) => subs,
                    Some(Err(errs)) => {
                        return Triple::Err(errs);
                    }
                    None => None,
                };
                // [T; M+N] -> [Int; 4+2] -> [Int; 6]
                let res = self.eval_t_params(projected_t, level, t_loc).ok();
                if let Some(t) = res {
                    let mut tv_cache = TyVarCache::new(self.level, self);
                    let t = self.detach(t, &mut tv_cache);
                    // Int -> T, 2 -> M, 4 -> N
                    return Triple::Ok(t);
                }
            } else {
                log!(err "{obj}");
                if DEBUG_MODE {
                    todo!()
                }
            }
        }
        Triple::None
    }

    /// e.g.
    /// F((Int), 3) => F(Int, 3)
    /// F(?T, ?T) => F(?1, ?1)
    pub(crate) fn detach(&self, ty: Type, tv_cache: &mut TyVarCache) -> Type {
        match ty {
            Type::FreeVar(fv) if fv.is_linked() => fv.crack().clone(), // self.detach(fv.crack().clone(), tv_cache),
            Type::FreeVar(fv) => {
                let new_fv = fv.detach();
                let name = new_fv.unbound_name().unwrap();
                if let Some(t) = tv_cache.get_tyvar(&name) {
                    t.clone()
                } else {
                    let tv = Type::FreeVar(new_fv);
                    let varname = VarName::from_str(name.clone());
                    tv_cache.dummy_push_or_init_tyvar(&varname, &tv, self);
                    tv
                }
            }
            Type::Poly { name, params } => {
                let mut new_params = vec![];
                for param in params {
                    new_params.push(self.detach_tp(param, tv_cache));
                }
                poly(name, new_params)
            }
            Type::Refinement(refine) => {
                let t = self.detach(*refine.t, tv_cache);
                refinement(refine.var, t, *refine.pred)
            }
            Type::Proj { lhs, rhs } => proj(self.detach(*lhs, tv_cache), rhs),
            Type::ProjCall {
                lhs,
                attr_name,
                args,
            } => {
                let mut new_args = vec![];
                for arg in args {
                    new_args.push(self.detach_tp(arg, tv_cache));
                }
                proj_call(self.detach_tp(*lhs, tv_cache), attr_name, new_args)
            }
            Type::Ref(t) => ref_(self.detach(*t, tv_cache)),
            Type::RefMut { before, after } => {
                let before = self.detach(*before, tv_cache);
                let after = after.map(|t| self.detach(*t, tv_cache));
                ref_mut(before, after)
            }
            Type::Record(rec) => {
                let mut new_rec = dict! {};
                for (name, t) in rec {
                    new_rec.insert(name, self.detach(t, tv_cache));
                }
                Type::Record(new_rec)
            }
            Type::NamedTuple(tup) => {
                let mut new_tup = vec![];
                for (name, t) in tup {
                    new_tup.push((name, self.detach(t, tv_cache)));
                }
                Type::NamedTuple(new_tup)
            }
            Type::Structural(ty) => self.detach(*ty, tv_cache).structuralize(),
            Type::Guard(grd) => {
                let to = self.detach(*grd.to, tv_cache);
                guard(grd.namespace, *grd.target, to)
            }
            Type::Subr(subr) => {
                let mut new_nd = vec![];
                for pt in subr.non_default_params {
                    new_nd.push(pt.map_type(&mut |t| self.detach(t, tv_cache)));
                }
                let new_var = subr
                    .var_params
                    .map(|t| t.map_type(&mut |t| self.detach(t, tv_cache)));
                let mut new_d = vec![];
                for pt in subr.default_params {
                    let pt = pt
                        .map_type(&mut |t| self.detach(t, tv_cache))
                        .map_default_type(&mut |t| self.detach(t, tv_cache));
                    new_d.push(pt);
                }
                let new_kv = subr.kw_var_params.map(|t| {
                    t.map_type(&mut |t| self.detach(t, tv_cache))
                        .map_default_type(&mut |t| self.detach(t, tv_cache))
                });
                let return_t = self.detach(*subr.return_t, tv_cache);
                let new_subr = SubrType::new(subr.kind, new_nd, new_var, new_d, new_kv, return_t);
                Type::Subr(new_subr)
            }
            Type::And(tys, idx) => {
                let mut new_tys = vec![];
                for t in tys {
                    new_tys.push(self.detach(t, tv_cache));
                }
                Type::checked_and(new_tys, idx)
            }
            Type::Or(tys) => {
                let mut new_tys = set! {};
                for t in tys {
                    new_tys.insert(self.detach(t, tv_cache));
                }
                Type::checked_or(new_tys)
            }
            Type::Not(t) => !self.detach(*t, tv_cache),
            Type::Bounded { sub, sup } => {
                let sub = self.detach(*sub, tv_cache);
                let sup = self.detach(*sup, tv_cache);
                bounded(sub, sup)
            }
            Type::Callable { param_ts, return_t } => {
                let mut new_param_ts = vec![];
                for t in param_ts {
                    new_param_ts.push(self.detach(t, tv_cache));
                }
                let return_t = self.detach(*return_t, tv_cache);
                callable(new_param_ts, return_t)
            }
            Type::Quantified(_) => ty,
            mono_type_pattern!() => ty,
        }
    }

    pub(crate) fn detach_tp(&self, tp: TyParam, tv_cache: &mut TyVarCache) -> TyParam {
        match tp {
            TyParam::FreeVar(fv) if fv.is_linked() => {
                let tp = fv.crack().clone();
                self.detach_tp(tp, tv_cache)
            }
            TyParam::FreeVar(fv) => {
                let new_fv = fv.detach();
                let name = new_fv.unbound_name().unwrap();
                if let Some(tp) = tv_cache.get_typaram(&name) {
                    tp.clone()
                } else {
                    let tp = TyParam::FreeVar(new_fv);
                    let varname = VarName::from_str(name.clone());
                    tv_cache.dummy_push_or_init_typaram(&varname, &tp, self);
                    tp
                }
            }
            TyParam::Type(t) => TyParam::t(self.detach(*t, tv_cache)),
            TyParam::Value(v) => TyParam::Value(self.detach_value(v, tv_cache)),
            TyParam::App { name, args } => {
                let mut new_args = vec![];
                for arg in args {
                    new_args.push(self.detach_tp(arg, tv_cache));
                }
                TyParam::App {
                    name,
                    args: new_args,
                }
            }
            TyParam::BinOp { op, lhs, rhs } => {
                let lhs = self.detach_tp(*lhs, tv_cache);
                let rhs = self.detach_tp(*rhs, tv_cache);
                TyParam::bin(op, lhs, rhs)
            }
            TyParam::UnaryOp { op, val } => {
                let val = self.detach_tp(*val, tv_cache);
                TyParam::unary(op, val)
            }
            TyParam::List(lis) => {
                let mut new_lis = vec![];
                for elem in lis {
                    new_lis.push(self.detach_tp(elem, tv_cache));
                }
                TyParam::List(new_lis)
            }
            TyParam::UnsizedList(elem) => {
                let elem = self.detach_tp(*elem, tv_cache);
                TyParam::UnsizedList(Box::new(elem))
            }
            TyParam::Tuple(tys) => {
                let mut new_tys = vec![];
                for elem in tys {
                    new_tys.push(self.detach_tp(elem, tv_cache));
                }
                TyParam::Tuple(new_tys)
            }
            TyParam::Set(set) => {
                let mut new_set = set! {};
                for elem in set {
                    new_set.insert(self.detach_tp(elem, tv_cache));
                }
                TyParam::Set(new_set)
            }
            TyParam::Dict(dic) => {
                let mut new_dic = dict! {};
                for (k, v) in dic.into_iter() {
                    new_dic.insert(self.detach_tp(k, tv_cache), self.detach_tp(v, tv_cache));
                }
                TyParam::Dict(new_dic)
            }
            TyParam::Record(rec) => {
                let mut new_rec = dict! {};
                for (name, elem) in rec {
                    new_rec.insert(name, self.detach_tp(elem, tv_cache));
                }
                TyParam::Record(new_rec)
            }
            TyParam::DataClass { name, fields } => {
                let mut new_fields = dict! {};
                for (k, v) in fields.into_iter() {
                    new_fields.insert(k, self.detach_tp(v, tv_cache));
                }
                TyParam::DataClass {
                    name,
                    fields: new_fields,
                }
            }
            TyParam::Proj { obj, attr } => {
                let obj = self.detach_tp(*obj, tv_cache);
                obj.proj(attr)
            }
            TyParam::ProjCall { obj, attr, args } => {
                let obj = self.detach_tp(*obj, tv_cache);
                let mut new_args = vec![];
                for arg in args {
                    new_args.push(self.detach_tp(arg, tv_cache));
                }
                obj.proj_call(attr, new_args)
            }
            TyParam::Erased(ty) => TyParam::erased(self.detach(*ty, tv_cache)),
            TyParam::Lambda(_) | TyParam::Mono(_) | TyParam::Failure => tp,
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn detach_value(&self, val: ValueObj, tv_cache: &mut TyVarCache) -> ValueObj {
        match val {
            ValueObj::Type(typ) => {
                // TODO:
                // let typ = typ.mapped_t(|t| self.detach(t, tv_cache));
                ValueObj::Type(typ)
            }
            ValueObj::List(vals) => {
                let mut new_vals = vec![];
                for v in vals.iter() {
                    new_vals.push(self.detach_value(v.clone(), tv_cache));
                }
                ValueObj::List(new_vals.into())
            }
            ValueObj::UnsizedList(val) => {
                ValueObj::UnsizedList(Box::new(self.detach_value(*val, tv_cache)))
            }
            ValueObj::Tuple(vals) => {
                let mut new_vals = vec![];
                for v in vals.iter() {
                    new_vals.push(self.detach_value(v.clone(), tv_cache));
                }
                ValueObj::Tuple(new_vals.into())
            }
            ValueObj::Dict(dic) => {
                let mut new_dic = dict! {};
                for (k, v) in dic.into_iter() {
                    new_dic.insert(
                        self.detach_value(k, tv_cache),
                        self.detach_value(v, tv_cache),
                    );
                }
                ValueObj::Dict(new_dic)
            }
            ValueObj::Set(set) => {
                let mut new_set = set! {};
                for v in set.into_iter() {
                    new_set.insert(self.detach_value(v, tv_cache));
                }
                ValueObj::Set(new_set)
            }
            ValueObj::Record(rec) => {
                let mut new_rec = dict! {};
                for (name, v) in rec.into_iter() {
                    new_rec.insert(name, self.detach_value(v, tv_cache));
                }
                ValueObj::Record(new_rec)
            }
            ValueObj::DataClass { name, fields } => {
                let mut new_fields = dict! {};
                for (name, v) in fields.into_iter() {
                    new_fields.insert(name, self.detach_value(v, tv_cache));
                }
                ValueObj::DataClass {
                    name,
                    fields: new_fields,
                }
            }
            ValueObj::Subr(_) => val,
            mono_value_pattern!() => val,
        }
    }

    fn convert_args(
        &self,
        lhs: Option<TyParam>,
        subr: &ConstSubr,
        args: Vec<TyParam>,
        t_loc: &impl Locational,
    ) -> EvalResult<ValueArgs> {
        let mut pos_args = vec![];
        if subr.sig_t().is_method() {
            let Some(lhs) = lhs else {
                return feature_error!(self, t_loc.loc(), "??");
            };
            if let Ok(value) = self.convert_tp_into_value(lhs.clone()) {
                pos_args.push(value);
            } else if let Ok(value) = self.eval_tp_into_value(lhs.clone()) {
                pos_args.push(value);
            } else {
                return feature_error!(self, t_loc.loc(), &format!("convert {lhs} to value"));
            }
        }
        for pos_arg in args.into_iter() {
            if let Ok(value) = self.convert_tp_into_value(pos_arg.clone()) {
                pos_args.push(value);
            } else if let Ok(value) = self.eval_tp_into_value(pos_arg.clone()) {
                pos_args.push(value);
            } else {
                return feature_error!(self, t_loc.loc(), &format!("convert {pos_arg} to value"));
            }
        }
        Ok(ValueArgs::new(pos_args, dict! {}))
    }

    /// lhs, args: should be evaluated
    fn do_proj_call(
        &self,
        obj: ValueObj,
        lhs: TyParam,
        args: Vec<TyParam>,
        t_loc: &impl Locational,
    ) -> EvalResult<TyParam> {
        if let ValueObj::Subr(subr) = obj {
            let args = self.convert_args(Some(lhs), &subr, args, t_loc)?;
            let tp = self.call(subr, args, t_loc.loc()).map_err(|(_, e)| e)?;
            Ok(tp)
        } else {
            feature_error!(self, t_loc.loc(), "do_proj_call: ??")
        }
    }

    /// lhs, args: should be evaluated
    fn do_proj_call_t(
        &self,
        obj: ValueObj,
        lhs: TyParam,
        args: Vec<TyParam>,
        t_loc: &impl Locational,
    ) -> EvalResult<Type> {
        let tp = self.do_proj_call(obj, lhs, args, t_loc)?;
        self.convert_tp_into_type(tp).map_err(|e| {
            EvalError::feature_error(
                self.cfg.input.clone(),
                line!() as usize,
                t_loc.loc(),
                &format!("converting {e} to a type"),
                self.caused_by(),
            )
            .into()
        })
    }

    fn eval_type_args(&self, args: Vec<TyParam>) -> Failable<Vec<TyParam>> {
        let mut errs = EvalErrors::empty();
        let mut new_args = vec![];
        for arg in args {
            let arg = match self.eval_tp(arg) {
                Ok(tp) => tp,
                Err((tp, es)) => {
                    errs.extend(es);
                    tp
                }
            };
            new_args.push(arg);
        }
        if errs.is_empty() {
            Ok(new_args)
        } else {
            Err((new_args, errs))
        }
    }

    /// lhs, args: may be unevaluated
    /// This may do nothing (be careful with recursive calls)
    pub(crate) fn eval_proj_call_t(
        &self,
        lhs: TyParam,
        attr_name: Str,
        args: Vec<TyParam>,
        level: usize,
        t_loc: &impl Locational,
    ) -> EvalResult<Type> {
        let lhs = self.eval_tp(lhs).map_err(|(_, errs)| errs)?;
        let args = self.eval_type_args(args).map_err(|(_, errs)| errs)?;
        let t = self.get_tp_t(&lhs)?;
        for ty_ctx in self.get_nominal_super_type_ctxs(&t).ok_or_else(|| {
            EvalError::type_not_found(
                self.cfg.input.clone(),
                line!() as usize,
                t_loc.loc(),
                self.caused_by(),
                &t,
            )
        })? {
            if let Ok(obj) = ty_ctx.get_const_local(&Token::symbol(&attr_name), &self.name) {
                return self.do_proj_call_t(obj, lhs, args, t_loc);
            }
            for methods in ty_ctx.methods_list.iter() {
                if let Ok(obj) = methods.get_const_local(&Token::symbol(&attr_name), &self.name) {
                    return self.do_proj_call_t(obj, lhs, args, t_loc);
                }
            }
        }
        if let TyParam::FreeVar(fv) = &lhs {
            if let Some((sub, sup)) = fv.get_subsup() {
                if self.is_trait(&sup) && !self.trait_impl_exists(&sub, &sup) {
                    // to prevent double error reporting
                    lhs.destructive_link(&TyParam::t(Never));
                    let sub = if cfg!(feature = "debug") {
                        sub
                    } else {
                        self.readable_type(sub)
                    };
                    let sup = if cfg!(feature = "debug") {
                        sup
                    } else {
                        self.readable_type(sup)
                    };
                    return Err(EvalErrors::from(EvalError::no_trait_impl_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        &sub,
                        &sup,
                        t_loc.loc(),
                        self.caused_by(),
                        self.get_simple_type_mismatch_hint(&sup, &sub),
                    )));
                }
            }
        }
        // if the target can't be found in the supertype, the type will be dereferenced.
        // In many cases, it is still better to determine the type variable than if the target is not found.
        let coerced = self.coerce_tp(lhs.clone(), t_loc)?;
        if lhs != coerced {
            let proj = proj_call(coerced.clone(), attr_name, args);
            self.eval_t_params(proj, level, t_loc)
                .inspect(|_t| lhs.destructive_link(&coerced))
                .map_err(|(_, errs)| errs)
        } else {
            let proj = proj_call(lhs, attr_name, args);
            Err(EvalErrors::from(EvalError::no_candidate_error(
                self.cfg.input.clone(),
                line!() as usize,
                &proj,
                t_loc.loc(),
                self.caused_by(),
                self.get_no_candidate_hint(&proj),
            )))
        }
    }

    /// lhs, args: may be unevaluated
    pub(crate) fn eval_proj_call(
        &self,
        lhs: TyParam,
        attr_name: Str,
        args: Vec<TyParam>,
        t_loc: &impl Locational,
    ) -> EvalResult<TyParam> {
        let lhs = self.eval_tp(lhs).map_err(|(_, errs)| errs)?;
        let args = self.eval_type_args(args).map_err(|(_, errs)| errs)?;
        let t = self.get_tp_t(&lhs)?;
        for ty_ctx in self.get_nominal_super_type_ctxs(&t).ok_or_else(|| {
            EvalError::type_not_found(
                self.cfg.input.clone(),
                line!() as usize,
                t_loc.loc(),
                self.caused_by(),
                &t,
            )
        })? {
            if let Ok(obj) = ty_ctx.get_const_local(&Token::symbol(&attr_name), &self.name) {
                return self.do_proj_call(obj, lhs, args, t_loc);
            }
            for methods in ty_ctx.methods_list.iter() {
                if let Ok(obj) = methods.get_const_local(&Token::symbol(&attr_name), &self.name) {
                    return self.do_proj_call(obj, lhs, args, t_loc);
                }
            }
        }
        if let TyParam::FreeVar(fv) = &lhs {
            if let Some((sub, sup)) = fv.get_subsup() {
                if self.is_trait(&sup) && !self.trait_impl_exists(&sub, &sup) {
                    // to prevent double error reporting
                    lhs.destructive_link(&TyParam::t(Never));
                    let sub = if cfg!(feature = "debug") {
                        sub
                    } else {
                        self.readable_type(sub)
                    };
                    let sup = if cfg!(feature = "debug") {
                        sup
                    } else {
                        self.readable_type(sup)
                    };
                    return Err(EvalErrors::from(EvalError::no_trait_impl_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        &sub,
                        &sup,
                        t_loc.loc(),
                        self.caused_by(),
                        self.get_simple_type_mismatch_hint(&sup, &sub),
                    )));
                }
            }
        }
        // if the target can't be found in the supertype, the type will be dereferenced.
        // In many cases, it is still better to determine the type variable than if the target is not found.
        let coerced = self.coerce_tp(lhs.clone(), t_loc)?;
        if lhs != coerced {
            self.eval_proj_call(coerced, attr_name, args, t_loc)
        } else {
            let proj = proj_call(lhs, attr_name, args);
            Err(EvalErrors::from(EvalError::no_candidate_error(
                self.cfg.input.clone(),
                line!() as usize,
                &proj,
                t_loc.loc(),
                self.caused_by(),
                self.get_no_candidate_hint(&proj),
            )))
        }
    }

    pub(crate) fn eval_call(
        &self,
        lhs: TyParam,
        args: Vec<TyParam>,
        t_loc: &impl Locational,
    ) -> EvalResult<TyParam> {
        match lhs {
            /*TyParam::Lambda(lambda) => {
                todo!("{lambda}")
            }*/
            TyParam::Value(ValueObj::Subr(subr)) => {
                let args = self.convert_args(None, &subr, args, t_loc)?;
                self.call(subr, args, t_loc.loc()).map_err(|(_, e)| e)
            }
            other => Err(EvalErrors::from(EvalError::type_mismatch_error(
                self.cfg.input.clone(),
                line!() as usize,
                t_loc.loc(),
                self.caused_by(),
                &other.qual_name().unwrap_or(Str::from("_")),
                None,
                &mono("Callable"),
                &self.get_tp_t(&other).ok().unwrap_or(Type::Obj),
                None,
                None,
            ))),
        }
    }

    pub(crate) fn bool_eval_pred(&self, p: Predicate) -> Failable<bool> {
        match self.eval_pred(p) {
            Ok(evaled) => Ok(matches!(evaled, Predicate::Value(ValueObj::Bool(true)))),
            Err((evaled, errs)) => Err((
                matches!(evaled, Predicate::Value(ValueObj::Bool(true))),
                errs,
            )),
        }
    }

    pub(crate) fn eval_pred(&self, p: Predicate) -> Failable<Predicate> {
        let mut errs = EvalErrors::empty();
        let pred = match p {
            Predicate::Value(_) | Predicate::Const(_) | Predicate::Failure => p,
            Predicate::Call {
                receiver,
                name,
                args,
            } => {
                let receiver = match self.eval_tp(receiver) {
                    Ok(tp) => tp,
                    Err((tp, es)) => {
                        errs.extend(es);
                        tp
                    }
                };
                let mut new_args = vec![];
                for arg in args {
                    match self.eval_tp(arg) {
                        Ok(tp) => new_args.push(tp),
                        Err((tp, es)) => {
                            errs.extend(es);
                            new_args.push(tp);
                        }
                    }
                }
                let res = if let Some(name) = name {
                    self.eval_proj_call(receiver, name, new_args, &())
                } else {
                    self.eval_call(receiver, new_args, &())
                };
                let tp = match res {
                    Ok(tp) => tp,
                    Err(es) => {
                        errs.extend(es);
                        return Err((Predicate::Failure, errs));
                    }
                };
                if let Ok(v) = self.convert_tp_into_value(tp) {
                    Predicate::Value(v)
                } else {
                    errs.push(EvalError::feature_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        Location::Unknown,
                        "eval_pred: Predicate::Call",
                        self.caused_by(),
                    ));
                    Predicate::Failure
                }
            }
            Predicate::Attr { receiver, name } => {
                if let Ok(TyParam::Value(val)) =
                    self.eval_tp_proj(receiver.clone(), name.clone(), &())
                {
                    Predicate::Value(val)
                } else {
                    let receiver = match self.eval_tp(receiver) {
                        Ok(tp) => tp,
                        Err((tp, es)) => {
                            errs.extend(es);
                            tp
                        }
                    };
                    Predicate::attr(receiver, name)
                }
            }
            Predicate::GeneralEqual { lhs, rhs } => {
                match (self.eval_pred(*lhs)?, self.eval_pred(*rhs)?) {
                    (Predicate::Value(lhs), Predicate::Value(rhs)) => {
                        Predicate::Value(ValueObj::Bool(lhs == rhs))
                    }
                    (lhs, rhs) => Predicate::general_eq(lhs, rhs),
                }
            }
            Predicate::GeneralNotEqual { lhs, rhs } => {
                match (self.eval_pred(*lhs)?, self.eval_pred(*rhs)?) {
                    (Predicate::Value(lhs), Predicate::Value(rhs)) => {
                        Predicate::Value(ValueObj::Bool(lhs != rhs))
                    }
                    (lhs, rhs) => Predicate::general_ne(lhs, rhs),
                }
            }
            Predicate::GeneralGreaterEqual { lhs, rhs } => {
                match (self.eval_pred(*lhs)?, self.eval_pred(*rhs)?) {
                    (Predicate::Value(lhs), Predicate::Value(rhs)) => {
                        let Some(ValueObj::Bool(res)) = lhs.try_ge(rhs) else {
                            errs.push(EvalError::feature_error(
                                self.cfg.input.clone(),
                                line!() as usize,
                                Location::Unknown,
                                "evaluating >=",
                                self.caused_by(),
                            ));
                            return Err((Predicate::Failure, errs));
                        };
                        Predicate::Value(ValueObj::Bool(res))
                    }
                    (lhs, rhs) => Predicate::general_ge(lhs, rhs),
                }
            }
            Predicate::GeneralLessEqual { lhs, rhs } => {
                match (self.eval_pred(*lhs)?, self.eval_pred(*rhs)?) {
                    (Predicate::Value(lhs), Predicate::Value(rhs)) => {
                        let Some(ValueObj::Bool(res)) = lhs.try_le(rhs) else {
                            errs.push(EvalError::feature_error(
                                self.cfg.input.clone(),
                                line!() as usize,
                                Location::Unknown,
                                "evaluating <=",
                                self.caused_by(),
                            ));
                            return Err((Predicate::Failure, errs));
                        };
                        Predicate::Value(ValueObj::Bool(res))
                    }
                    (lhs, rhs) => Predicate::general_le(lhs, rhs),
                }
            }
            Predicate::Equal { lhs, rhs } => {
                let rhs = match self.eval_tp(rhs) {
                    Ok(tp) => tp,
                    Err((tp, es)) => {
                        errs.extend(es);
                        tp
                    }
                };
                Predicate::eq(lhs, rhs)
            }
            Predicate::NotEqual { lhs, rhs } => {
                let rhs = match self.eval_tp(rhs) {
                    Ok(tp) => tp,
                    Err((tp, es)) => {
                        errs.extend(es);
                        tp
                    }
                };
                Predicate::ne(lhs, rhs)
            }
            Predicate::LessEqual { lhs, rhs } => {
                let rhs = match self.eval_tp(rhs) {
                    Ok(tp) => tp,
                    Err((tp, es)) => {
                        errs.extend(es);
                        tp
                    }
                };
                Predicate::le(lhs, rhs)
            }
            Predicate::GreaterEqual { lhs, rhs } => {
                let rhs = match self.eval_tp(rhs) {
                    Ok(tp) => tp,
                    Err((tp, es)) => {
                        errs.extend(es);
                        tp
                    }
                };
                Predicate::ge(lhs, rhs)
            }
            Predicate::And(l, r) => {
                let lhs = match self.eval_pred(*l) {
                    Ok(pred) => pred,
                    Err((pred, es)) => {
                        errs.extend(es);
                        pred
                    }
                };
                let rhs = match self.eval_pred(*r) {
                    Ok(pred) => pred,
                    Err((pred, es)) => {
                        errs.extend(es);
                        pred
                    }
                };
                lhs & rhs
            }
            Predicate::Or(preds) => {
                let mut new_preds = Set::with_capacity(preds.len());
                for pred in preds {
                    let pred = match self.eval_pred(pred) {
                        Ok(pred) => pred,
                        Err((pred, es)) => {
                            errs.extend(es);
                            pred
                        }
                    };
                    new_preds.insert(pred);
                }
                Predicate::Or(new_preds)
            }
            Predicate::Not(pred) => {
                let pred = match self.eval_pred(*pred) {
                    Ok(pred) => pred,
                    Err((pred, es)) => {
                        errs.extend(es);
                        pred
                    }
                };
                !pred
            }
        };
        if errs.is_empty() {
            Ok(pred)
        } else {
            Err((pred, errs))
        }
    }

    pub(crate) fn get_tp_t(&self, p: &TyParam) -> EvalResult<Type> {
        let p = self
            .eval_tp(p.clone())
            .inspect_err(|(tp, errs)| log!(err "{tp} / {errs}"))
            .unwrap_or(p.clone());
        match p {
            TyParam::Value(v) => Ok(v_enum(set![v])),
            TyParam::Erased(t) => Ok((*t).clone()),
            TyParam::FreeVar(fv) if fv.is_linked() => self.get_tp_t(&fv.unwrap_linked()),
            TyParam::FreeVar(fv) => {
                if let Some(t) = fv.get_type() {
                    Ok(t)
                } else {
                    feature_error!(self, Location::Unknown, "??")
                }
            }
            TyParam::Type(typ) => Ok(self.meta_type(&typ)),
            TyParam::Mono(name) => self
                .rec_get_const_obj(&name)
                .map(|v| v_enum(set![v.clone()]))
                .ok_or_else(|| {
                    EvalErrors::from(EvalError::unreachable(
                        self.cfg.input.clone(),
                        fn_name!(),
                        line!(),
                    ))
                }),
            TyParam::App { ref name, ref args } => self
                .rec_get_const_obj(name)
                .and_then(|v| {
                    let ty = match self.convert_value_into_type(v.clone()) {
                        Ok(ty) => ty,
                        Err(ValueObj::Subr(subr)) => {
                            // REVIEW: evaluation of polymorphic types
                            return subr.sig_t().return_t().cloned();
                        }
                        Err(_) => {
                            return None;
                        }
                    };
                    let instance = self
                        .instantiate_def_type(&ty)
                        .map_err(|err| {
                            log!(err "{err}");
                            err
                        })
                        .ok()?;
                    for (param, arg) in instance.typarams().iter().zip(args.iter()) {
                        self.sub_unify_tp(arg, param, None, &(), false).ok()?;
                    }
                    let ty_obj = if self.is_class(&instance) {
                        ValueObj::builtin_class(instance)
                    } else if self.is_trait(&instance) {
                        ValueObj::builtin_trait(instance)
                    } else {
                        ValueObj::builtin_type(instance)
                    };
                    Some(v_enum(set![ty_obj]))
                })
                .or_else(|| {
                    let namespace = p.namespace();
                    if let Some(namespace) = self.get_namespace(&namespace) {
                        if namespace.name != self.name {
                            if let Some(typ) = p.local_name().and_then(|name| {
                                namespace.get_tp_t(&TyParam::app(name, args.clone())).ok()
                            }) {
                                return Some(typ);
                            }
                        }
                    }
                    None
                })
                .ok_or_else(|| {
                    EvalErrors::from(EvalError::unreachable(
                        self.cfg.input.clone(),
                        fn_name!(),
                        line!(),
                    ))
                }),
            TyParam::List(tps) => {
                let tp_t = if let Some(fst) = tps.first() {
                    self.get_tp_t(fst)?
                } else {
                    Never
                };
                let t = list_t(tp_t, TyParam::value(tps.len()));
                Ok(t)
            }
            TyParam::UnsizedList(tp) => {
                let tp_t = self.get_tp_t(&tp)?;
                Ok(unsized_list_t(tp_t))
            }
            TyParam::Tuple(tps) => {
                let mut tps_t = vec![];
                for tp in tps {
                    tps_t.push(self.get_tp_t(&tp)?);
                }
                Ok(tuple_t(tps_t))
            }
            TyParam::Set(tps) => {
                let len = TyParam::value(tps.len());
                let mut union = Type::Never;
                for tp in tps {
                    let tp_t = self.get_tp_t(&tp)?;
                    union = self.union(&union, &tp_t);
                }
                Ok(set_t(union, len))
            }
            TyParam::Record(dict) => {
                let mut fields = dict! {};
                for (name, tp) in dict.into_iter() {
                    let tp_t = self.get_tp_t(&tp)?;
                    fields.insert(name, tp_t);
                }
                Ok(Type::Record(fields))
            }
            dict @ TyParam::Dict(_) => Ok(dict_t(dict)),
            TyParam::BinOp { op, lhs, rhs } => match op {
                OpKind::Or | OpKind::And => {
                    let lhs = self.get_tp_t(&lhs)?;
                    let rhs = self.get_tp_t(&rhs)?;
                    if self.subtype_of(&lhs, &Type::Bool) && self.subtype_of(&rhs, &Type::Bool) {
                        Ok(Type::Bool)
                    } else if self.subtype_of(&lhs, &Type::Type)
                        && self.subtype_of(&rhs, &Type::Type)
                    {
                        Ok(Type::Type)
                    } else {
                        let op_name = op_to_name(op);
                        feature_error!(
                            self,
                            Location::Unknown,
                            &format!("get type: {op_name}({lhs}, {rhs})")
                        )
                    }
                }
                _ => {
                    let op_name = op_to_name(op);
                    feature_error!(
                        self,
                        Location::Unknown,
                        &format!("get type: {op_name}({lhs}, {rhs})")
                    )
                }
            },
            TyParam::Proj { obj, attr } => self.get_proj_t(&obj, &attr),
            TyParam::ProjCall { obj, attr, args } => {
                let Ok(tp) = self.eval_proj_call(*obj.clone(), attr.clone(), args, &()) else {
                    let Some(obj_ctx) = self.get_nominal_type_ctx(&self.get_tp_t(&obj)?) else {
                        return Ok(Type::Obj);
                    };
                    let value = obj_ctx.get_const_local(&Token::symbol(&attr), &self.name)?;
                    match value {
                        ValueObj::Subr(subr) => {
                            if let Some(ret_t) = subr.sig_t().return_t() {
                                return Ok(ret_t.clone());
                            } else {
                                return Err(EvalErrors::from(EvalError::unreachable(
                                    self.cfg.input.clone(),
                                    fn_name!(),
                                    line!(),
                                )));
                            }
                        }
                        _ => {
                            return Ok(Type::Obj);
                        }
                    }
                };
                let ty = self.get_tp_t(&tp).unwrap_or(Type::Obj).derefine();
                Ok(tp_enum(ty, set![tp]))
            }
            TyParam::Lambda(lambda) => lambda
                .body
                .last()
                .map_or(Ok(Type::Obj), |tp| self.get_tp_t(tp)),
            TyParam::Failure => Ok(Type::Failure),
            other @ (TyParam::DataClass { .. } | TyParam::UnaryOp { .. }) => {
                feature_error!(
                    self,
                    Location::Unknown,
                    &format!("getting the type of {other}")
                )
            }
        }
    }

    pub(crate) fn get_proj_t(&self, obj: &TyParam, attr: &str) -> EvalResult<Type> {
        let obj_t = self.get_tp_t(obj)?;
        let obj_ctx = self.get_nominal_type_ctx(&obj_t).ok_or_else(|| {
            EvalErrors::from(EvalError::type_not_found(
                self.cfg.input.clone(),
                line!() as usize,
                Location::Unknown,
                self.caused_by(),
                &obj_t,
            ))
        })?;
        if let Some((_, vi)) = obj_ctx.get_var_info(attr) {
            Ok(vi.t.clone())
        } else {
            Ok(Type::Obj)
        }
    }

    pub(crate) fn _get_tp_class(&self, p: &TyParam) -> EvalResult<Type> {
        let p = match self.eval_tp(p.clone()) {
            Ok(tp) => tp,
            // TODO: handle errs
            Err((tp, errs)) => {
                log!(err "{tp} / {errs}");
                tp
            }
        };
        match p {
            TyParam::Value(v) => Ok(v.class()),
            TyParam::Erased(t) => Ok((*t).clone()),
            TyParam::FreeVar(fv) => {
                if let Some(t) = fv.get_type() {
                    Ok(t)
                } else {
                    feature_error!(self, Location::Unknown, "??")
                }
            }
            TyParam::Type(_) => Ok(Type::Type),
            TyParam::Mono(name) => {
                self.rec_get_const_obj(&name)
                    .map(|v| v.class())
                    .ok_or_else(|| {
                        EvalErrors::from(EvalError::unreachable(
                            self.cfg.input.clone(),
                            fn_name!(),
                            line!(),
                        ))
                    })
            }
            other => feature_error!(
                self,
                Location::Unknown,
                &format!("getting the class of {other}")
            ),
        }
    }

    /// NOTE: If l and r are types, the Context is used to determine the type.
    /// NOTE: lとrが型の場合はContextの方で判定する
    pub(crate) fn shallow_eq_tp(&self, lhs: &TyParam, rhs: &TyParam) -> bool {
        match (lhs, rhs) {
            (TyParam::Type(l), _) if l.is_unbound_var() => {
                let Ok(rhs) = self.get_tp_t(rhs) else {
                    log!(err "rhs: {rhs}");
                    return false;
                };
                self.subtype_of(&rhs, &Type::Type)
            }
            (_, TyParam::Type(r)) if r.is_unbound_var() => {
                let Ok(lhs) = self.get_tp_t(lhs) else {
                    log!(err "lhs: {lhs}");
                    return false;
                };
                self.subtype_of(&lhs, &Type::Type)
            }
            (TyParam::Type(l), TyParam::Type(r)) => l == r,
            (TyParam::Value(l), TyParam::Value(r)) => l == r,
            (TyParam::Erased(l), TyParam::Erased(r)) => l == r,
            (TyParam::List(l), TyParam::List(r)) => l == r,
            (TyParam::Tuple(l), TyParam::Tuple(r)) => l == r,
            (TyParam::Set(l), TyParam::Set(r)) => l.linear_eq(r),
            (TyParam::Dict(l), TyParam::Dict(r)) => l.linear_eq(r),
            (TyParam::Lambda(l), TyParam::Lambda(r)) => l == r,
            (TyParam::FreeVar { .. }, TyParam::FreeVar { .. }) => true,
            (TyParam::Mono(l), TyParam::Mono(r)) => {
                if l == r {
                    true
                } else if let (Some(l), Some(r)) =
                    (self.rec_get_const_obj(l), self.rec_get_const_obj(r))
                {
                    l == r
                } else {
                    // lとrが型の場合は...
                    false
                }
            }
            (TyParam::Mono(m), TyParam::Value(l)) | (TyParam::Value(l), TyParam::Mono(m)) => {
                if let Some(o) = self.rec_get_const_obj(m) {
                    o == l
                } else {
                    true
                }
            }
            (TyParam::Erased(t), _) => Some(t.as_ref()) == self.get_tp_t(rhs).ok().as_ref(),
            (_, TyParam::Erased(t)) => Some(t.as_ref()) == self.get_tp_t(lhs).ok().as_ref(),
            (TyParam::Value(v), _) => {
                if let Ok(tp) = Self::convert_value_into_tp(v.clone()) {
                    self.shallow_eq_tp(&tp, rhs)
                } else {
                    false
                }
            }
            (_, TyParam::Value(v)) => {
                if let Ok(tp) = Self::convert_value_into_tp(v.clone()) {
                    self.shallow_eq_tp(lhs, &tp)
                } else {
                    false
                }
            }
            (l, r) => {
                log!(err "l: {l}, r: {r}");
                l == r
            }
        }
    }
}
