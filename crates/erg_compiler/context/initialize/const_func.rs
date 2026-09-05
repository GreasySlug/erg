use std::fmt::Display;
use std::mem;
use std::num::IntErrorKind;
use std::path::Path;

use erg_common::dict::Dict;
use erg_common::env::{python_site_packages, python_sys_path};
#[allow(unused_imports)]
use erg_common::log;
use erg_common::normalize_path;
use erg_common::traits::Stream;
use erg_common::{dict, set, set_recursion_limit};

use crate::context::eval::UndoableLinkedList;
use crate::context::initialize::closed_range;
use crate::context::Context;
use crate::feature_error;
use crate::ty::constructors::{and, dict_mut, list_mut, mono, poly, tuple_t, v_enum};
use crate::ty::value::{EvalValueError, EvalValueResult, GenTypeObj, TypeObj, ValueObj};
use crate::ty::{ConstSubr, Field, TyParam, Type, ValueArgs};
use erg_common::error::{ErrorCore, ErrorKind, Location, SubMessage};
use erg_common::style::{Color, StyledStr, StyledString, THEME};

const ERR: Color = THEME.colors.error;
const WARN: Color = THEME.colors.warning;

fn not_passed(t: impl Display) -> EvalValueError {
    let text = t.to_string();
    let param = StyledStr::new(&text, Some(ERR), None);
    ErrorCore::new(
        vec![SubMessage::only_loc(Location::Unknown)],
        format!("{param} is not passed"),
        line!() as usize,
        ErrorKind::KeyError,
        Location::Unknown,
    )
    .into()
}

fn no_key(slf: impl Display, key: impl Display) -> EvalValueError {
    ErrorCore::new(
        vec![SubMessage::only_loc(Location::Unknown)],
        format!("{slf} has no key {key}"),
        line!() as usize,
        ErrorKind::KeyError,
        Location::Unknown,
    )
    .into()
}

fn type_mismatch(expected: impl Display, got: impl Display, param: &str) -> EvalValueError {
    let got = StyledString::new(format!("{got}"), Some(ERR), None);
    let param = StyledStr::new(param, Some(WARN), None);
    ErrorCore::new(
        vec![SubMessage::only_loc(Location::Unknown)],
        format!("non-{expected} object {got} is passed to {param}"),
        line!() as usize,
        ErrorKind::TypeError,
        Location::Unknown,
    )
    .into()
}

fn too_many_args(name: &str, expected: usize) -> EvalValueError {
    let name = StyledStr::new(name, Some(WARN), None);
    ErrorCore::new(
        vec![SubMessage::only_loc(Location::Unknown)],
        format!("{name} takes {expected} type argument(s), but more are passed"),
        line!() as usize,
        ErrorKind::TypeError,
        Location::Unknown,
    )
    .into()
}

/// The type aliases take a fixed number of arguments; anything left over would
/// otherwise be dropped without a word.
fn reject_extra_args(args: &ValueArgs, name: &str, expected: usize) -> Result<(), EvalValueError> {
    if args.pos_args.is_empty() && args.kw_args.is_empty() {
        Ok(())
    } else {
        Err(too_many_args(name, expected))
    }
}

/// The call is fine, only folding it is not: see `EvalValueError::not_foldable`.
fn not_foldable(what: impl Display, why: impl Display) -> EvalValueError {
    EvalValueError::not_foldable(what, why)
}

/// A genuinely unimplemented corner of a const function, as opposed to one that
/// merely cannot be folded here.
fn todo(msg: &str) -> EvalValueError {
    ErrorCore::new(
        vec![SubMessage::only_loc(Location::Unknown)],
        format!("{msg} is not supported yet in const context"),
        line!() as usize,
        ErrorKind::FeatureError,
        Location::Unknown,
    )
    .into()
}

/// Base := Type or NoneType, Impl := Type -> ClassType
pub(crate) fn class_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let base = args.remove_left_or_key("Base");
    let impls = args.remove_left_or_key("Impl");
    let impls = impls.and_then(|v| v.as_type(ctx));
    // Check if the context has type parameters for polymorphic class
    let t = if let Some(ref tv_cache) = ctx.tv_cache {
        // Create polymorphic type with type parameters from tv_cache
        // (in declaration order, e.g. `|T, U|`)
        let params = tv_cache.ordered_instances();
        if params.is_empty() {
            mono(ctx.name.clone())
        } else {
            poly(ctx.name.clone(), params)
        }
    } else {
        mono(ctx.name.clone())
    };
    match base {
        Some(value) => {
            if let Some(base) = value.as_type(ctx) {
                Ok(ValueObj::gen_t(GenTypeObj::class(t, Some(base), impls, true)).into())
            } else {
                Err(type_mismatch("type", value, "Base"))
            }
        }
        None => Ok(ValueObj::gen_t(GenTypeObj::class(t, None, impls, true)).into()),
    }
}

/// Super: ClassType, Impl := Type, Additional := Type -> ClassType
pub(crate) fn inherit_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let sup = args
        .remove_left_or_key("Super")
        .ok_or_else(|| not_passed("Super"))?;
    let Some(sup) = sup.as_type(ctx) else {
        return Err(type_mismatch("class", sup, "Super"));
    };
    let impls = args.remove_left_or_key("Impl");
    let impls = impls.and_then(|v| v.as_type(ctx));
    let additional = args.remove_left_or_key("Additional");
    let additional = additional.and_then(|v| v.as_type(ctx));
    let t = mono(ctx.name.clone());
    Ok(ValueObj::gen_t(GenTypeObj::inherited(t, sup, impls, additional)).into())
}

/// Class: ClassType -> ClassType (with `InheritableType`)
/// This function is used by the compiler to mark a class as inheritable and does nothing in terms of actual operation.
pub(crate) fn inheritable_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let class = args
        .remove_left_or_key("Class")
        .ok_or_else(|| not_passed("Class"))?;
    match class {
        ValueObj::Type(TypeObj::Generated(mut gen)) => {
            if let Some(typ) = gen.impls_mut() {
                match typ.as_mut().map(|x| x.as_mut()) {
                    Some(TypeObj::Generated(gen)) => {
                        *gen.typ_mut() = and(mem::take(gen.typ_mut()), mono("InheritableType"));
                    }
                    Some(TypeObj::Builtin { t, .. }) => {
                        *t = and(mem::take(t), mono("InheritableType"));
                    }
                    _ => {
                        *typ = Some(Box::new(TypeObj::builtin_trait(mono("InheritableType"))));
                    }
                }
            }
            Ok(ValueObj::Type(TypeObj::Generated(gen)).into())
        }
        other => feature_error!(
            EvalValueError,
            _ctx,
            Location::Unknown,
            &format!("Inheritable {other}")
        ),
    }
}

pub(crate) fn override_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let func = args
        .remove_left_or_key("func")
        .ok_or_else(|| not_passed("func"))?;
    Ok(func.into())
}

/// Base: Type, Impl := Type -> TraitType
pub(crate) fn trait_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let req = args
        .remove_left_or_key("Requirement")
        .ok_or_else(|| not_passed("Requirement"))?;
    let Some(req) = req.as_type(ctx) else {
        return Err(type_mismatch("type", req, "Requirement"));
    };
    let impls = args.remove_left_or_key("Impl");
    let impls = impls.and_then(|v| v.as_type(ctx));
    // Check if the context has type parameters for polymorphic trait
    let t = if let Some(ref tv_cache) = ctx.tv_cache {
        // in declaration order, e.g. `|T, U|`
        let params = tv_cache.ordered_instances();
        if params.is_empty() {
            mono(ctx.name.clone())
        } else {
            poly(ctx.name.clone(), params)
        }
    } else {
        mono(ctx.name.clone())
    };
    Ok(ValueObj::gen_t(GenTypeObj::trait_(t, req, impls, true)).into())
}

/// Base: Type, Impl := Type -> Patch
pub(crate) fn patch_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let base = args
        .remove_left_or_key("Base")
        .ok_or_else(|| not_passed("Base"))?;
    let Some(base) = base.as_type(ctx) else {
        return Err(type_mismatch("type", base, "Base"));
    };
    let impls = args.remove_left_or_key("Impl");
    let impls = impls.and_then(|v| v.as_type(ctx));
    let t = mono(ctx.name.clone());
    Ok(ValueObj::gen_t(GenTypeObj::patch(t, base, impls)).into())
}

/// Super: TraitType, Impl := Type, Additional := Type -> TraitType
pub(crate) fn subsume_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let sup = args
        .remove_left_or_key("Super")
        .ok_or_else(|| not_passed("Super"))?;
    let Some(sup) = sup.as_type(ctx) else {
        return Err(type_mismatch("trait", sup, "Super"));
    };
    let impls = args.remove_left_or_key("Impl");
    let impls = impls.and_then(|v| v.as_type(ctx));
    let additional = args.remove_left_or_key("Additional");
    let additional = additional.and_then(|v| v.as_type(ctx));
    let t = mono(ctx.name.clone());
    Ok(ValueObj::gen_t(GenTypeObj::subsumed(t, sup, impls, additional)).into())
}

/// `Option T == T or NoneType`
///
/// An alias, not a type of its own: the result is the plain union, which is what
/// the standard declarations, the `?` operator and the `OptionEq` patch all speak.
pub(crate) fn option_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let t = args
        .remove_left_or_key("T")
        .ok_or_else(|| not_passed("T"))?;
    let Some(t) = t.as_type(ctx) else {
        return Err(type_mismatch("type", t, "T"));
    };
    reject_extra_args(&args, "Option", 1)?;
    Ok(ValueObj::builtin_type(ctx.union(t.typ(), &Type::NoneType)).into())
}

/// `Result T == T or Error`, or `Result(T, E) == T or E` for another error type.
///
/// An alias, like [`option_func`].
pub(crate) fn result_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let t = args
        .remove_left_or_key("T")
        .ok_or_else(|| not_passed("T"))?;
    let Some(t) = t.as_type(ctx) else {
        return Err(type_mismatch("type", t, "T"));
    };
    let err_t = match args.remove_left_or_key("E") {
        Some(e) => match e.as_type(ctx) {
            Some(e) => e.typ().clone(),
            None => return Err(type_mismatch("type", e, "E")),
        },
        None => Type::Error,
    };
    reject_extra_args(&args, "Result", 2)?;
    Ok(ValueObj::builtin_type(ctx.union(t.typ(), &err_t)).into())
}

/// `Either(L, R) == L or R`
///
/// An alias, like [`option_func`]. Note that the union is symmetric: unlike a
/// tagged sum, `Either` cannot tell the two sides apart when they overlap.
pub(crate) fn either_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let l = args
        .remove_left_or_key("L")
        .ok_or_else(|| not_passed("L"))?;
    let Some(l) = l.as_type(ctx) else {
        return Err(type_mismatch("type", l, "L"));
    };
    let r = args
        .remove_left_or_key("R")
        .ok_or_else(|| not_passed("R"))?;
    let Some(r) = r.as_type(ctx) else {
        return Err(type_mismatch("type", r, "R"));
    };
    reject_extra_args(&args, "Either", 2)?;
    Ok(ValueObj::builtin_type(ctx.union(l.typ(), r.typ())).into())
}

pub(crate) fn structural_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let type_ = args
        .remove_left_or_key("Type")
        .ok_or_else(|| not_passed("Type"))?;
    let Some(base) = type_.as_type(ctx) else {
        return Err(type_mismatch("type", type_, "Type"));
    };
    let t = base.typ().clone().structuralize();
    Ok(ValueObj::gen_t(GenTypeObj::structural(t, base)).into())
}

pub(crate) fn __list_getitem__(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let slf = match ctx.convert_value_into_list(slf) {
        Ok(slf) => slf,
        Err(val) => {
            return Err(type_mismatch("List", val, "Self"));
        }
    };
    let index = args
        .remove_left_or_key("Index")
        .ok_or_else(|| not_passed("Index"))?;
    let Ok(index) = usize::try_from(&index) else {
        return Err(type_mismatch("Nat", index, "Index"));
    };
    if let Some(v) = slf.get(index) {
        Ok(v.clone().into())
    } else {
        Err(ErrorCore::new(
            vec![SubMessage::only_loc(Location::Unknown)],
            format!(
                "[{}] has {} elements, but accessed {}th element",
                erg_common::fmt_vec(&slf),
                slf.len(),
                index
            ),
            line!() as usize,
            ErrorKind::IndexError,
            Location::Unknown,
        )
        .into())
    }
}

pub(crate) fn sub_vdict_get<'d>(
    dict: &'d Dict<ValueObj, ValueObj>,
    key: &ValueObj,
    ctx: &Context,
) -> Option<&'d ValueObj> {
    set_recursion_limit!(None, 64);
    let mut matches = vec![];
    for (k, v) in dict.iter() {
        if key == k {
            return Some(v);
        }
        match (
            ctx.convert_value_into_type(key.clone()),
            ctx.convert_value_into_type(k.clone()),
        ) {
            (Ok(idx), Ok(kt))
                if dict.len() == 1 || ctx.subtype_of(&idx.lower_bounded(), &kt.lower_bounded()) =>
            {
                matches.push((idx, kt, v));
            }
            _ => {}
        }
    }
    for (idx, kt, v) in matches.into_iter() {
        let list = UndoableLinkedList::new();
        match ctx.undoable_sub_unify(&idx, &kt, &(), &list, None) {
            Ok(_) => {
                return Some(v);
            }
            Err(_err) => {
                erg_common::log!(err "{idx} <!: {kt} => {v}");
            }
        }
    }
    None
}

pub(crate) fn sub_tpdict_get<'d>(
    dict: &'d Dict<TyParam, TyParam>,
    key: &TyParam,
    ctx: &Context,
) -> Option<&'d TyParam> {
    let mut matches = vec![];
    for (k, v) in dict.iter() {
        if key == k {
            return Some(v);
        }
        match (
            ctx.convert_tp_into_type(key.clone()),
            ctx.convert_tp_into_type(k.clone()),
        ) {
            (Ok(idx), Ok(kt))
                if dict.len() == 1 || ctx.subtype_of(&idx.lower_bounded(), &kt.lower_bounded()) =>
            {
                matches.push((idx, kt, v));
            }
            _ => {}
        }
    }
    for (idx, kt, v) in matches.into_iter() {
        let list = UndoableLinkedList::new();
        match ctx.undoable_sub_unify(&idx, &kt, &(), &list, None) {
            Ok(_) => {
                return Some(v);
            }
            Err(_err) => {
                erg_common::log!(err "{idx} <!: {kt} => {v}");
            }
        }
    }
    None
}

/// `{{"a"}: Int, {"b"}: Float} ==> {{"a", "b"}: Float}`
fn homogenize_dict_type(dict: &Dict<Type, Type>, ctx: &Context) -> Dict<Type, Type> {
    let mut union_key = Type::Never;
    let mut union_value = Type::Never;
    for (k, v) in dict.iter() {
        union_key = ctx.union(&union_key, k);
        union_value = ctx.union(&union_value, v);
    }
    dict! { union_key => union_value }
}

/// see `homogenize_dict_type`
fn homogenize_dict(dict: &Dict<ValueObj, ValueObj>, ctx: &Context) -> Dict<ValueObj, ValueObj> {
    let mut type_dict = Dict::new();
    for (k, v) in dict.iter() {
        match (k, v) {
            (ValueObj::Type(k), ValueObj::Type(v)) => {
                type_dict.insert(k.typ().clone(), v.typ().clone());
            }
            _ => {
                return dict.clone();
            }
        }
    }
    let dict_t = homogenize_dict_type(&type_dict, ctx);
    let mut value_dict = Dict::new();
    for (k, v) in dict_t.iter() {
        let k = ValueObj::builtin_type(k.clone());
        let v = ValueObj::builtin_type(v.clone());
        value_dict.insert(k, v);
    }
    value_dict
}

pub(crate) fn __dict_getitem__(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let Ok(slf) = ctx.convert_value_to_dict(&slf) else {
        return Err(type_mismatch("Dict", slf, "Self"));
    };
    let index = args
        .remove_left_or_key("Index")
        .ok_or_else(|| not_passed("Index"))?;
    if let Some(v) = slf
        .linear_get(&index)
        .or_else(|| sub_vdict_get(&slf, &index, ctx))
    {
        Ok(v.clone().into())
    } else if let Some(v) = sub_vdict_get(&homogenize_dict(&slf, ctx), &index, ctx).cloned() {
        Ok(v.into())
    } else {
        let index = if let ValueObj::Type(t) = &index {
            ValueObj::builtin_type(ctx.readable_type(t.typ().clone()))
        } else {
            index
        };
        Err(no_key(slf, index))
    }
}

/// ```erg
/// {"a": 1, "b": 2}.keys() == ["a", "b"]
/// {Str: Int, Int: Float}.keys() == Str or Int
/// ```
pub(crate) fn dict_keys(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let Ok(slf) = ctx.convert_value_to_dict(&slf) else {
        return Err(type_mismatch("Dict", slf, "Self"));
    };
    let dict_type = slf
        .iter()
        .map(|(k, v)| {
            let k = ctx.convert_value_into_type(k.clone())?;
            let v = ctx.convert_value_into_type(v.clone())?;
            Ok((k, v))
        })
        .collect::<Result<Dict<_, _>, ValueObj>>();
    if let Ok(slf) = dict_type {
        let union = slf
            .keys()
            .fold(Type::Never, |union, t| ctx.union(&union, t));
        // let keys = poly(DICT_KEYS, vec![ty_tp(union)]);
        Ok(ValueObj::builtin_type(union).into())
    } else {
        Ok(ValueObj::List(slf.into_keys().collect::<Vec<_>>().into()).into())
    }
}

/// ```erg
/// {"a": 1, "b": 2}.values() == [1, 2]
/// {Str: Int, Int: Float}.values() == Int or Float
/// ```
pub(crate) fn dict_values(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let Ok(slf) = ctx.convert_value_to_dict(&slf) else {
        return Err(type_mismatch("Dict", slf, "Self"));
    };
    let dict_type = slf
        .iter()
        .map(|(k, v)| {
            let k = ctx.convert_value_into_type(k.clone())?;
            let v = ctx.convert_value_into_type(v.clone())?;
            Ok((k, v))
        })
        .collect::<Result<Dict<_, _>, ValueObj>>();
    if let Ok(slf) = dict_type {
        let union = slf
            .values()
            .fold(Type::Never, |union, t| ctx.union(&union, t));
        // let values = poly(DICT_VALUES, vec![ty_tp(union)]);
        Ok(ValueObj::builtin_type(union).into())
    } else {
        Ok(ValueObj::List(slf.into_values().collect::<Vec<_>>().into()).into())
    }
}

/// ```erg
/// {"a": 1, "b": 2}.items() == [("a", 1), ("b", 2)]
/// {Str: Int, Int: Float}.items() == (Str, Int) or (Int, Float)
/// ```
pub(crate) fn dict_items(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let Ok(slf) = ctx.convert_value_to_dict(&slf) else {
        return Err(type_mismatch("Dict", slf, "Self"));
    };
    let dict_type = slf
        .iter()
        .map(|(k, v)| {
            let k = ctx.convert_value_into_type(k.clone())?;
            let v = ctx.convert_value_into_type(v.clone())?;
            Ok((k, v))
        })
        .collect::<Result<Dict<_, _>, ValueObj>>();
    if let Ok(slf) = dict_type {
        let union = slf.into_iter().fold(Type::Never, |union, (k, v)| {
            ctx.union(&union, &tuple_t(vec![k, v]))
        });
        // let items = poly(DICT_ITEMS, vec![ty_tp(union)]);
        Ok(ValueObj::builtin_type(union).into())
    } else {
        Ok(ValueObj::List(
            slf.into_iter()
                .map(|(k, v)| ValueObj::Tuple(vec![k, v].into()))
                .collect::<Vec<_>>()
                .into(),
        )
        .into())
    }
}

/// If the key is duplicated, the value of the right dict is used.
/// `{Str: Int, Int: Float}.concat({Int: Str, Float: Bool}) == {Str: Int, Int: Str, Float: Bool}`
pub(crate) fn dict_concat(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let Ok(slf) = ctx.convert_value_to_dict(&slf) else {
        return Err(type_mismatch("Dict", slf, "Self"));
    };
    let other = args
        .remove_left_or_key("Other")
        .ok_or_else(|| not_passed("Other"))?;
    let ValueObj::Dict(other) = other else {
        return Err(type_mismatch("Dict", other, "Other"));
    };
    Ok(ValueObj::Dict(slf.concat(other)).into())
}

pub(crate) fn dict_diff(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let Ok(slf) = ctx.convert_value_to_dict(&slf) else {
        return Err(type_mismatch("Dict", slf, "Self"));
    };
    let other = args
        .remove_left_or_key("Other")
        .ok_or_else(|| not_passed("Other"))?;
    let ValueObj::Dict(other) = other else {
        return Err(type_mismatch("Dict", other, "Other"));
    };
    Ok(ValueObj::Dict(slf.diff(&other)).into())
}

pub(crate) fn list_constructor(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let _cls = args
        .remove_left_or_key("Cls")
        .ok_or_else(|| not_passed("Cls"))?;
    let elem = args
        .remove_left_or_key("elem")
        .ok_or_else(|| not_passed("elem"))?;
    let Ok(elem_t) = ctx.convert_value_into_type(elem.clone()) else {
        return Err(type_mismatch("Type", elem, "elem"));
    };
    let len = args
        .remove_left_or_key("len")
        .map(TyParam::value)
        .unwrap_or(TyParam::erased(Type::Nat));
    Ok(ValueObj::builtin_class(list_mut(elem_t, len)).into())
}

pub(crate) fn dict_constructor(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let _cls = args
        .remove_left_or_key("Cls")
        .ok_or_else(|| not_passed("Cls"))?;
    let key_value = args
        .remove_left_or_key("key_value")
        .ok_or_else(|| not_passed("key_value"))?;
    let (key_t, value_t) = match key_value {
        ValueObj::Tuple(ts) | ValueObj::List(ts) => {
            let key = ts.first().ok_or_else(|| not_passed("key"))?;
            let value = ts.get(1).ok_or_else(|| not_passed("value"))?;
            let Ok(key_t) = ctx.convert_value_into_type(key.clone()) else {
                return Err(type_mismatch("Type", key, "key"));
            };
            let Ok(value_t) = ctx.convert_value_into_type(value.clone()) else {
                return Err(type_mismatch("Type", value, "value"));
            };
            (key_t, value_t)
        }
        _ => return Err(type_mismatch("Tuple", key_value, "key_value")),
    };
    Ok(ValueObj::builtin_class(dict_mut(dict! { key_t => value_t }.into())).into())
}

/// `[Int, Str].union() == Int or Str`
pub(crate) fn list_union(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let ValueObj::List(slf) = slf else {
        return Err(type_mismatch("List", slf, "Self"));
    };
    let slf = slf
        .iter()
        .flat_map(|t| ctx.convert_value_into_type(t.clone()))
        .collect::<Vec<_>>();
    // args must already be evaluated
    if slf.iter().any(|t| t.has_proj() || t.has_proj_call()) {
        return Ok(TyParam::t(Type::Obj));
    }
    let union = slf
        .iter()
        .fold(Type::Never, |union, t| ctx.union(&union, t));
    Ok(ValueObj::builtin_type(union).into())
}

fn _lis_shape(arr: ValueObj, ctx: &Context) -> Result<Vec<TyParam>, String> {
    let mut shape = vec![];
    let mut arr = arr;
    loop {
        match arr {
            ValueObj::List(a) => {
                shape.push(ValueObj::from(a.len()).into());
                match a.first() {
                    Some(arr_ @ (ValueObj::List(_) | ValueObj::Type(_))) => {
                        arr = arr_.clone();
                    }
                    _ => {
                        break;
                    }
                }
            }
            ValueObj::Type(ref t) if &t.typ().qual_name()[..] == "List" => {
                let mut tps = t.typ().typarams();
                let elem = match ctx.convert_tp_into_type(tps.remove(0)) {
                    Ok(elem) => elem,
                    Err(err) => {
                        return Err(err.to_string());
                    }
                };
                let len = tps.remove(0);
                shape.push(len);
                arr = ValueObj::builtin_type(elem);
            }
            _ => {
                break;
            }
        }
    }
    Ok(shape)
}

/// ```erg
/// List(Int, 2).shape() == [2,]
/// List(List(Int, 2), N).shape() == [N, 2]
/// [1, 2].shape() == [2,]
/// [[1, 2], [3, 4], [5, 6]].shape() == [3, 2]
/// ```
pub(crate) fn list_shape(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let res = _lis_shape(val, ctx).unwrap();
    let lis = TyParam::List(res);
    Ok(lis)
}

fn _list_scalar_type(mut typ: Type, ctx: &Context) -> Result<Type, String> {
    loop {
        if matches!(&typ.qual_name()[..], "List" | "List!" | "UnsizedList") {
            let tp = typ.typarams().remove(0);
            match ctx.convert_tp_into_type(tp) {
                Ok(typ_) => {
                    typ = typ_;
                }
                Err(err) => {
                    return Err(format!("Cannot convert {err} into type"));
                }
            }
        } else {
            return Ok(typ);
        }
    }
}

/// `classinfo` as the list of classes it stands for: one class, or a tuple of
/// them, as CPython's `isinstance`/`issubclass` accept.
fn class_infos(value: ValueObj, ctx: &Context) -> Option<Vec<Type>> {
    match value {
        ValueObj::Tuple(ts) => ts
            .iter()
            .map(|t| ctx.convert_value_into_type(t.clone()).ok())
            .collect(),
        other => ctx.convert_value_into_type(other).ok().map(|t| vec![t]),
    }
}

/// Whether `t` is something the emitted program can hand to `isinstance` /
/// `issubclass`: a plain class, existing at run time under its own name.
/// A parameterized generic (`List(Int)`) raises `TypeError` there, and a
/// refinement or a union is not a runtime class object at all, so folding
/// those would invent an answer the program never produces.
fn is_runtime_class(t: &Type, ctx: &Context) -> bool {
    match t {
        Type::Int | Type::Nat | Type::Ratio | Type::Float | Type::Bool | Type::Str => true,
        Type::Mono(_) => ctx
            .get_nominal_type_ctx(t)
            .is_some_and(|tc| tc.ctx.kind.is_class()),
        _ => false,
    }
}

/// `isinstance(object, classinfo)`.
///
/// The object's class is the class the emitted program builds, so this asks the
/// same question CPython will: `isinstance(-1, Nat)` is false both here (`Int`
/// is not a subtype of `Nat`) and there (`Int` does not inherit `Nat` in
/// `_erg_nat.py`).
pub(crate) fn isinstance_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let object = args
        .remove_left_or_key("object")
        .ok_or_else(|| not_passed("object"))?;
    let classinfo = args
        .remove_left_or_key("classinfo")
        .ok_or_else(|| not_passed("classinfo"))?;
    let Some(classes) = class_infos(classinfo.clone(), ctx) else {
        return Err(type_mismatch("ClassType", classinfo, "classinfo"));
    };
    if let Some(t) = classes.iter().find(|t| !is_runtime_class(t, ctx)) {
        return Err(not_foldable(
            format!("isinstance(..., {t})"),
            "only a plain class decides the same way at compile time and at run time",
        ));
    }
    let class = object.class();
    let is = classes.iter().any(|t| ctx.subtype_of(&class, t));
    Ok(ValueObj::Bool(is).into())
}

/// `issubclass(subclass, classinfo)`.
pub(crate) fn issubclass_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let subclass = args
        .remove_left_or_key("subclass")
        .ok_or_else(|| not_passed("subclass"))?;
    let classinfo = args
        .remove_left_or_key("classinfo")
        .ok_or_else(|| not_passed("classinfo"))?;
    let Ok(sub) = ctx.convert_value_into_type(subclass.clone()) else {
        return Err(type_mismatch("ClassType", subclass, "subclass"));
    };
    let Some(classes) = class_infos(classinfo.clone(), ctx) else {
        return Err(type_mismatch("ClassType", classinfo, "classinfo"));
    };
    if let Some(t) = classes
        .iter()
        .chain([&sub])
        .find(|t| !is_runtime_class(t, ctx))
    {
        return Err(not_foldable(
            format!("issubclass(..., {t})"),
            "only a plain class decides the same way at compile time and at run time",
        ));
    }
    let is = classes.iter().any(|t| ctx.subtype_of(&sub, t));
    Ok(ValueObj::Bool(is).into())
}

pub(crate) fn list_scalar_type(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let Ok(slf) = ctx.convert_value_into_type(slf.clone()) else {
        return Err(type_mismatch("Type", slf, "Self"));
    };
    let res = _list_scalar_type(slf, ctx).unwrap();
    Ok(TyParam::t(res))
}

fn _scalar_type(mut value: ValueObj, _ctx: &Context) -> Result<Type, String> {
    loop {
        match value {
            ValueObj::List(a) => match a.first() {
                Some(elem) => {
                    value = elem.clone();
                }
                None => {
                    return Ok(Type::Never);
                }
            },
            ValueObj::Set(s) => match s.iter().next() {
                Some(elem) => {
                    value = elem.clone();
                }
                None => {
                    return Ok(Type::Never);
                }
            },
            ValueObj::Tuple(t) => match t.first() {
                Some(elem) => {
                    value = elem.clone();
                }
                None => {
                    return Ok(Type::Never);
                }
            },
            ValueObj::UnsizedList(a) => {
                value = *a.clone();
            }
            other => {
                return Ok(other.class());
            }
        }
    }
}

/// ```erg
/// [1, 2].scalar_type() == Nat
/// [[1, 2], [3, 4], [5, 6]].scalar_type() == Nat
/// ```
#[allow(unused)]
pub(crate) fn scalar_type(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let res = _scalar_type(val, ctx).unwrap();
    let lis = TyParam::t(res);
    Ok(lis)
}

fn _list_sum(arr: ValueObj, _ctx: &Context) -> Result<ValueObj, String> {
    match arr {
        ValueObj::List(a) => {
            let mut sum = 0f64;
            for v in a.iter() {
                match v {
                    ValueObj::Nat(n) => {
                        sum += *n as f64;
                    }
                    ValueObj::Int(n) => {
                        sum += *n as f64;
                    }
                    ValueObj::Float(n) => {
                        sum += **n;
                    }
                    ValueObj::Inf => {
                        return Ok(ValueObj::Inf);
                    }
                    ValueObj::NegInf => {
                        return Ok(ValueObj::NegInf);
                    }
                    _ => {
                        return Err(format!("Cannot sum {v}"));
                    }
                }
            }
            if sum.round() == sum && sum >= 0.0 {
                Ok(ValueObj::Nat(sum as u64))
            } else if sum.round() == sum {
                Ok(ValueObj::Int(sum as i64))
            } else {
                Ok(ValueObj::from(sum))
            }
        }
        _ => Err(format!("Cannot sum {arr}")),
    }
}

/// ```erg
/// [1, 2].sum() == [3,]
/// ```
pub(crate) fn list_sum(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let res = _list_sum(val, ctx).unwrap();
    let lis = TyParam::Value(res);
    Ok(lis)
}

fn _list_prod(lis: ValueObj, _ctx: &Context) -> Result<ValueObj, String> {
    match lis {
        ValueObj::List(a) => {
            let mut prod = 1f64;
            for v in a.iter() {
                match v {
                    ValueObj::Nat(n) => {
                        prod *= *n as f64;
                    }
                    ValueObj::Int(n) => {
                        prod *= *n as f64;
                    }
                    ValueObj::Float(n) => {
                        prod *= **n;
                    }
                    ValueObj::Inf => {
                        return Ok(ValueObj::Inf);
                    }
                    ValueObj::NegInf => {
                        return Ok(ValueObj::NegInf);
                    }
                    _ => {
                        return Err(format!("Cannot prod {v}"));
                    }
                }
            }
            if prod.round() == prod && prod >= 0.0 {
                Ok(ValueObj::Nat(prod as u64))
            } else if prod.round() == prod {
                Ok(ValueObj::Int(prod as i64))
            } else {
                Ok(ValueObj::from(prod))
            }
        }
        _ => Err(format!("Cannot prod {lis}")),
    }
}

/// ```erg
/// [1, 2].prod() == [2,]
/// ```
pub(crate) fn list_prod(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let res = _list_prod(val, ctx).unwrap();
    let lis = TyParam::Value(res);
    Ok(lis)
}

fn _list_reversed(lis: ValueObj, _ctx: &Context) -> Result<ValueObj, String> {
    match lis {
        ValueObj::List(a) => {
            let mut vec = a.to_vec();
            vec.reverse();
            Ok(ValueObj::List(vec.into()))
        }
        _ => Err(format!("Cannot reverse {lis}")),
    }
}

pub(crate) fn list_reversed(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let res = _list_reversed(val, ctx).unwrap();
    let lis = TyParam::Value(res);
    Ok(lis)
}

fn _list_insert_at(
    lis: ValueObj,
    index: usize,
    value: ValueObj,
    _ctx: &Context,
) -> Result<ValueObj, String> {
    match lis {
        ValueObj::List(a) => {
            let mut a = a.to_vec();
            if index > a.len() {
                return Err(format!("Index out of range: {index}"));
            }
            a.insert(index, value);
            Ok(ValueObj::List(a.into()))
        }
        _ => Err(format!("Cannot insert into {lis}")),
    }
}

pub(crate) fn list_insert_at(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let lis = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let index = args
        .remove_left_or_key("Index")
        .ok_or_else(|| not_passed("Index"))?;
    let value = args
        .remove_left_or_key("Value")
        .ok_or_else(|| not_passed("Value"))?;
    let Ok(index) = usize::try_from(&index) else {
        return Err(type_mismatch("Nat", index, "Index"));
    };
    let res = _list_insert_at(lis, index, value, ctx).unwrap();
    let lis = TyParam::Value(res);
    Ok(lis)
}

fn _list_remove_at(lis: ValueObj, index: usize, _ctx: &Context) -> Result<ValueObj, String> {
    match lis {
        ValueObj::List(a) => {
            let mut a = a.to_vec();
            if index >= a.len() {
                return Err(format!("Index out of range: {index}"));
            }
            a.remove(index);
            Ok(ValueObj::List(a.into()))
        }
        _ => Err(format!("Cannot remove from {lis}")),
    }
}

pub(crate) fn list_remove_at(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let index = args
        .remove_left_or_key("Index")
        .ok_or_else(|| not_passed("Index"))?;
    let Ok(index) = usize::try_from(&index) else {
        return Err(type_mismatch("Nat", index, "Index"));
    };
    let res = _list_remove_at(val, index, ctx).unwrap();
    let lis = TyParam::Value(res);
    Ok(lis)
}

fn _list_remove_all(lis: ValueObj, value: ValueObj, _ctx: &Context) -> Result<ValueObj, String> {
    match lis {
        ValueObj::List(a) => {
            let mut a = a.to_vec();
            a.retain(|v| v != &value);
            Ok(ValueObj::List(a.into()))
        }
        _ => Err(format!("Cannot remove from {lis}")),
    }
}

pub(crate) fn list_remove_all(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let value = args
        .remove_left_or_key("Value")
        .ok_or_else(|| not_passed("Value"))?;
    let res = _list_remove_all(val, value, ctx).unwrap();
    let lis = TyParam::Value(res);
    Ok(lis)
}

pub(crate) fn __range_getitem__(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let ValueObj::DataClass { name: _, fields } = slf else {
        return Err(type_mismatch("Range", slf, "Self"));
    };
    let index = args
        .remove_left_or_key("Index")
        .ok_or_else(|| not_passed("Index"))?;
    let Ok(index) = usize::try_from(&index) else {
        return Err(type_mismatch("Nat", index, "Index"));
    };
    let start = fields
        .get("start")
        .ok_or_else(|| no_key(&fields, "start"))?;
    let Ok(start) = usize::try_from(start) else {
        return Err(type_mismatch("Nat", start, "start"));
    };
    let end = fields.get("end").ok_or_else(|| no_key(&fields, "end"))?;
    let Ok(end) = usize::try_from(end) else {
        return Err(type_mismatch("Nat", end, "end"));
    };
    // FIXME <= if inclusive
    if start + index < end {
        Ok(ValueObj::Nat((start + index) as u64).into())
    } else {
        Err(ErrorCore::new(
            vec![SubMessage::only_loc(Location::Unknown)],
            format!("Index out of range: {index}"),
            line!() as usize,
            ErrorKind::IndexError,
            Location::Unknown,
        )
        .into())
    }
}

pub(crate) fn __named_tuple_getitem__(
    mut args: ValueArgs,
    ctx: &Context,
) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let fields = match ctx.convert_value_into_type(slf) {
        Ok(Type::NamedTuple(fields)) => fields,
        Ok(other) => {
            return Err(type_mismatch("NamedTuple", other, "Self"));
        }
        Err(val) => {
            return Err(type_mismatch("NamedTuple", val, "Self"));
        }
    };
    let index = args
        .remove_left_or_key("Index")
        .ok_or_else(|| not_passed("Index"))?;
    let Ok(index) = usize::try_from(&index) else {
        return Err(type_mismatch("Nat", index, "Index"));
    };
    if let Some((_, t)) = fields.get(index) {
        Ok(TyParam::t(t.clone()))
    } else {
        Err(no_key(Type::NamedTuple(fields), index))
    }
}

/// `NamedTuple({ .x = Int; .y = Str }).union() == Int or Str`
/// `GenericNamedTuple.union() == Obj`
pub(crate) fn named_tuple_union(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let fields = match ctx.convert_value_into_type(slf) {
        Ok(Type::NamedTuple(fields)) => fields,
        Ok(Type::Mono(n)) if &n == "GenericNamedTuple" => {
            return Ok(ValueObj::builtin_type(Type::Obj).into());
        }
        Ok(other) => {
            return Err(type_mismatch("NamedTuple", other, "Self"));
        }
        Err(val) => {
            return Err(type_mismatch("NamedTuple", val, "Self"));
        }
    };
    let union = fields
        .iter()
        .fold(Type::Never, |union, (_, t)| ctx.union(&union, t));
    Ok(ValueObj::builtin_type(union).into())
}

/// `{ .x = Int; .y = Str }.as_dict() == { "x": Int, "y": Str }`
/// `Record.as_dict() == { Obj: Obj }`
pub(crate) fn as_dict(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let fields = match ctx.convert_value_into_type(slf) {
        Ok(Type::Record(fields)) => fields,
        Ok(Type::Mono(n)) if &n == "Record" => {
            let dict = dict! { Type::Obj => Type::Obj };
            return Ok(ValueObj::builtin_type(Type::from(dict)).into());
        }
        Ok(other) => {
            return Err(type_mismatch("Record", other, "Self"));
        }
        Err(val) => {
            return Err(type_mismatch("Record", val, "Self"));
        }
    };
    let dict = fields
        .into_iter()
        .map(|(k, v)| (v_enum(set! {  k.symbol.into() }), v))
        .collect::<Dict<_, _>>();
    Ok(ValueObj::builtin_type(Type::from(dict)).into())
}

/// `{ {"x"}: Int, {"y"}: Str }.as_record() == { .x = Int, .y = Str }`
pub(crate) fn as_record(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("Self")
        .ok_or_else(|| not_passed("Self"))?;
    let fields = match ctx.convert_value_into_type(slf) {
        Ok(Type::Poly { name, params }) if &name == "Dict" => {
            Dict::try_from(params[0].clone()).unwrap()
        }
        Ok(other) => {
            return Err(type_mismatch("Dict", other, "Self"));
        }
        Err(val) => {
            return Err(type_mismatch("Dict", val, "Self"));
        }
    };
    let mut dict = Dict::new();
    for (k, v) in fields {
        match (ctx.convert_tp_into_type(k), ctx.convert_tp_into_type(v)) {
            (Ok(k_t), Ok(v_t)) => {
                if let Some(values) = k_t.refinement_values() {
                    for value in values {
                        if let TyParam::Value(ValueObj::Str(field)) = value {
                            dict.insert(Field::public(field.clone()), v_t.clone());
                        } else {
                            return Err(type_mismatch("Str", value, "Key"));
                        }
                    }
                } else {
                    return Err(type_mismatch("Str refinement type", k_t, "Key"));
                }
            }
            (Ok(_), Err(err)) | (Err(err), Ok(_)) => {
                return Err(type_mismatch("Type", err, "Self"));
            }
            (Err(k), Err(_v)) => {
                return Err(type_mismatch("Type", k, "Self"));
            }
        };
    }
    Ok(ValueObj::builtin_type(Type::Record(dict)).into())
}

pub(crate) fn int_abs(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let Some(slf) = slf.as_int() else {
        return Err(type_mismatch("Int", slf, "self"));
    };
    Ok(ValueObj::Int(i64::from(slf.abs())).into())
}

pub(crate) fn str_endswith(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let suffix = args
        .remove_left_or_key("suffix")
        .ok_or_else(|| not_passed("suffix"))?;
    let Some(slf) = slf.as_str() else {
        return Err(type_mismatch("Str", slf, "self"));
    };
    let Some(suffix) = suffix.as_str() else {
        return Err(type_mismatch("Str", suffix, "suffix"));
    };
    Ok(ValueObj::Bool(slf.ends_with(&suffix[..])).into())
}

pub(crate) fn str_find(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let sub = args
        .remove_left_or_key("sub")
        .ok_or_else(|| not_passed("sub"))?;
    let Some(slf) = slf.as_str() else {
        return Err(type_mismatch("Str", slf, "self"));
    };
    let Some(sub) = sub.as_str() else {
        return Err(type_mismatch("Str", sub, "sub"));
    };
    Ok(ValueObj::Int(slf.find(&sub[..]).map_or(-1, |i| i as i64)).into())
}

pub(crate) fn str_isalpha(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let Some(slf) = slf.as_str() else {
        return Err(type_mismatch("Str", slf, "self"));
    };
    Ok(ValueObj::Bool(slf.chars().all(|c| c.is_alphabetic())).into())
}

pub(crate) fn str_isascii(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let Some(slf) = slf.as_str() else {
        return Err(type_mismatch("Str", slf, "self"));
    };
    Ok(ValueObj::Bool(slf.is_ascii()).into())
}

pub(crate) fn str_isdecimal(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let Some(slf) = slf.as_str() else {
        return Err(type_mismatch("Str", slf, "self"));
    };
    Ok(ValueObj::Bool(slf.chars().all(|c| c.is_ascii_digit())).into())
}

pub(crate) fn str_join(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let Some(slf) = slf.as_str() else {
        return Err(type_mismatch("Str", slf, "self"));
    };
    let arr = match iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        ValueObj::Dict(d) => d.into_keys().collect(),
        _ => {
            return Err(type_mismatch("Iterable(Str)", iterable, "iterable"));
        }
    };
    // NOTE: the separator goes *between* the elements; appending it after each
    // one and popping a single char left a trailing separator behind whenever it
    // was longer than one character (`", ".join(["a", "b"]) == "a, b,"`).
    let mut parts = Vec::with_capacity(arr.len());
    for v in arr.iter() {
        let Some(v) = v.as_str() else {
            return Err(type_mismatch("Str", v, "arr.next()"));
        };
        parts.push(&v[..]);
    }
    Ok(ValueObj::Str(parts.join(&slf[..]).into()).into())
}

pub(crate) fn str_replace(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let old = args
        .remove_left_or_key("old")
        .ok_or_else(|| not_passed("old"))?;
    let new = args
        .remove_left_or_key("new")
        .ok_or_else(|| not_passed("new"))?;
    let Some(slf) = slf.as_str() else {
        return Err(type_mismatch("Str", slf, "self"));
    };
    let Some(old) = old.as_str() else {
        return Err(type_mismatch("Str", old, "old"));
    };
    let Some(new) = new.as_str() else {
        return Err(type_mismatch("Str", new, "new"));
    };
    Ok(ValueObj::Str(slf.replace(&old[..], new).into()).into())
}

pub(crate) fn str_startswith(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    let prefix = args
        .remove_left_or_key("prefix")
        .ok_or_else(|| not_passed("prefix"))?;
    let Some(slf) = slf.as_str() else {
        return Err(type_mismatch("Str", slf, "self"));
    };
    let Some(prefix) = prefix.as_str() else {
        return Err(type_mismatch("Str", prefix, "prefix"));
    };
    Ok(ValueObj::Bool(slf.starts_with(&prefix[..])).into())
}

pub(crate) fn abs_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let num = args
        .remove_left_or_key("n")
        .ok_or_else(|| not_passed("n"))?;
    abs_of(num, "n")
}

/// `x.abs()` on a `Ratio` or a `Float`. `Int.abs` (`int_abs`) narrows to `Nat`;
/// these keep the receiver's class, so the declared return type matches what
/// CPython's `__abs__` actually returns.
pub(crate) fn num_abs(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let slf = args
        .remove_left_or_key("self")
        .ok_or_else(|| not_passed("self"))?;
    abs_of(slf, "self")
}

fn abs_of(num: ValueObj, param: &str) -> EvalValueResult<TyParam> {
    match num {
        ValueObj::Nat(n) => Ok(ValueObj::Nat(n).into()),
        ValueObj::Int(n) => Ok(ValueObj::Nat(n.unsigned_abs()).into()),
        ValueObj::Bool(b) => Ok(ValueObj::Nat(b as u64).into()),
        ValueObj::Ratio(n, d) => n
            .checked_abs()
            .map(|n| ValueObj::Ratio(n, d).into())
            .ok_or_else(|| not_foldable("abs", "the result does not fit an `Int`")),
        ValueObj::Float(n) => Ok(ValueObj::from(n.abs()).into()),
        ValueObj::Inf | ValueObj::NegInf => Ok(ValueObj::Inf.into()),
        _ => Err(type_mismatch("Num", num, param)),
    }
}

pub(crate) fn if_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let cond = args
        .remove_left_or_key("cond")
        .ok_or_else(|| not_passed("cond"))?;
    let then = args
        .remove_left_or_key("then")
        .ok_or_else(|| not_passed("then"))?;
    let else_ = args.remove_left_or_key("else");
    let take_then = match cond {
        ValueObj::Bool(b) => b,
        other => return Err(type_mismatch("Bool", other, "cond")),
    };
    let branch = if take_then {
        then
    } else {
        else_.unwrap_or(ValueObj::None)
    };
    match branch {
        // A branch is a `do` block: no arguments, no definition scope of its own,
        // and it reads the scope the `if` is being evaluated in. Calling it would
        // clone that scope into a frame and spend a second level of the recursion
        // limit for every level the user wrote -- which is why a const function
        // could only recurse 31 deep out of a limit of 64.
        //
        // Anything else falls through to the call below: a named subroutine
        // resolves its names where it was defined rather than here, and a block
        // that defines a name needs a scope of its own to define into.
        ValueObj::Subr(ConstSubr::User(user))
            if user.params.is_empty() && user.def_scope.is_none() && !user.defines_name() =>
        {
            match ctx.eval_const_do_block(&user.block()) {
                Ok(val) => Ok(val.into()),
                Err((_val, mut errs)) => Err(EvalValueError::from(*errs.remove(0).core)),
            }
        }
        ValueObj::Subr(subr) => match ctx.call(subr, ValueArgs::empty(), Location::Unknown) {
            Ok(tp) => Ok(tp),
            Err((_tp, mut err)) => Err(EvalValueError::from(*err.remove(0).core)),
        },
        other => Ok(other.into()),
    }
}

pub(crate) fn all_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let arr = match iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        _ => {
            return Err(type_mismatch("Iterable(Bool)", iterable, "iterable"));
        }
    };
    let mut all = true;
    for v in arr.iter() {
        match v {
            ValueObj::Bool(b) => {
                all &= *b;
            }
            _ => {
                return Err(type_mismatch("Bool", v, "iterable.next()"));
            }
        }
    }
    Ok(ValueObj::Bool(all).into())
}

pub(crate) fn any_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let arr = match iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        _ => {
            return Err(type_mismatch("Iterable(Bool)", iterable, "iterable"));
        }
    };
    let mut any = false;
    for v in arr.iter() {
        match v {
            ValueObj::Bool(b) => {
                any |= *b;
            }
            _ => {
                return Err(type_mismatch("Bool", v, "iterable.next()"));
            }
        }
    }
    Ok(ValueObj::Bool(any).into())
}

pub(crate) fn filter_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let func = args
        .remove_left_or_key("func")
        .ok_or_else(|| not_passed("func"))?;
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let arr = match iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        _ => {
            return Err(type_mismatch("Iterable(T)", iterable, "iterable"));
        }
    };
    let subr = match func {
        ValueObj::Subr(f) => f,
        _ => {
            return Err(type_mismatch("Subr", func, "func"));
        }
    };
    let mut filtered = vec![];
    for v in arr.into_iter() {
        let args = ValueArgs::pos_only(vec![v.clone()]);
        match ctx.call(subr.clone(), args, Location::Unknown) {
            Ok(res) => match ctx.convert_tp_into_value(res) {
                Ok(res) => {
                    if res.is_true() {
                        filtered.push(v);
                    }
                }
                Err(tp) => {
                    return Err(type_mismatch("Bool", tp, "func"));
                }
            },
            Err((_res, mut err)) => {
                return Err(EvalValueError::from(*err.remove(0).core));
            }
        }
    }
    Ok(TyParam::Value(ValueObj::List(filtered.into())))
}

pub(crate) fn len_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let container = args
        .remove_left_or_key("s")
        .ok_or_else(|| not_passed("s"))?;
    let len = match container {
        ValueObj::List(a) => a.len(),
        ValueObj::Tuple(t) => t.len(),
        ValueObj::Set(s) => s.len(),
        ValueObj::Dict(d) => d.len(),
        ValueObj::Record(r) => r.len(),
        ValueObj::Str(s) => s.len(),
        _ => {
            return Err(type_mismatch("Container", container, "container"));
        }
    };
    Ok(ValueObj::Nat(len as u64).into())
}

pub(crate) fn map_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let func = args
        .remove_left_or_key("func")
        .ok_or_else(|| not_passed("func"))?;
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let arr = match iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        _ => {
            return Err(type_mismatch("Iterable(Bool)", iterable, "iterable"));
        }
    };
    let subr = match func {
        ValueObj::Subr(f) => f,
        _ => {
            return Err(type_mismatch("Subr", func, "func"));
        }
    };
    let mut mapped = vec![];
    for v in arr.into_iter() {
        let args = ValueArgs::pos_only(vec![v]);
        match ctx.call(subr.clone(), args, Location::Unknown) {
            Ok(res) => {
                mapped.push(res);
            }
            Err((_res, mut err)) => {
                return Err(EvalValueError::from(*err.remove(0).core));
            }
        }
    }
    Ok(TyParam::List(mapped))
}

pub(crate) fn max_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let arr = match iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        _ => {
            return Err(type_mismatch("Iterable(Ord)", iterable, "iterable"));
        }
    };
    let mut max = ValueObj::NegInf;
    if arr.is_empty() {
        return Err(ErrorCore::new(
            vec![SubMessage::only_loc(Location::Unknown)],
            "max() arg is an empty sequence",
            line!() as usize,
            ErrorKind::ValueError,
            Location::Unknown,
        )
        .into());
    }
    for v in arr.into_iter() {
        if v.is_num() {
            if max.clone().try_lt(v.clone()).is_some_and(|b| b.is_true()) {
                max = v;
            }
        } else {
            return Err(type_mismatch("Ord", v, "iterable.next()"));
        }
    }
    Ok(max.into())
}

pub(crate) fn min_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let arr = match iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        _ => {
            return Err(type_mismatch("Iterable(Ord)", iterable, "iterable"));
        }
    };
    let mut min = ValueObj::Inf;
    if arr.is_empty() {
        return Err(ErrorCore::new(
            vec![SubMessage::only_loc(Location::Unknown)],
            "min() arg is an empty sequence",
            line!() as usize,
            ErrorKind::ValueError,
            Location::Unknown,
        )
        .into());
    }
    for v in arr.into_iter() {
        if v.is_num() {
            if min.clone().try_gt(v.clone()).is_some_and(|b| b.is_true()) {
                min = v;
            }
        } else {
            return Err(type_mismatch("Ord", v, "iterable.next()"));
        }
    }
    Ok(min.into())
}

pub(crate) fn not_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("b")
        .ok_or_else(|| not_passed("b"))?;
    match val {
        ValueObj::Bool(b) => Ok(ValueObj::Bool(!b).into()),
        _ => Err(type_mismatch("Bool", val, "b")),
    }
}

pub(crate) fn reversed_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let reversible = args
        .remove_left_or_key("seq")
        .ok_or_else(|| not_passed("seq"))?;
    let arr = match reversible {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        _ => {
            return Err(type_mismatch("Reversible", reversible, "seq"));
        }
    };
    let mut reversed = vec![];
    for v in arr.into_iter().rev() {
        reversed.push(v);
    }
    Ok(TyParam::Value(ValueObj::List(reversed.into())))
}

pub(crate) fn str_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("object")
        .ok_or_else(|| not_passed("object"))?;
    if let Some(_encoding) = args.remove_left_or_key("encoding") {
        return Err(todo("encoding"));
    }
    // NOTE: `ValueObj`'s `Display` is repr-like (`Str` comes out quoted), so it
    // must not be used here -- the result has to equal what `str` returns at run time.
    match py_str(&val) {
        Some(s) => Ok(ValueObj::Str(s.into()).into()),
        None => Err(not_foldable(
            format!("str({val})"),
            "its text at run time is not reproducible here",
        )),
    }
}

pub(crate) fn sum_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let arr = match iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        ValueObj::Dict(d) => d.into_keys().collect(),
        ValueObj::Record(r) => r.into_values().collect(),
        _ => {
            return Err(type_mismatch("Iterable(Add)", iterable, "iterable"));
        }
    };
    let mut sum = ValueObj::Nat(0);
    for v in arr.into_iter() {
        if v.is_num() {
            sum = sum.try_add(v).unwrap();
        } else {
            return Err(type_mismatch("Add", v, "iterable.next()"));
        }
    }
    Ok(sum.into())
}

/// Python's `str()`, restricted to the values whose textual form is guaranteed
/// to be the same at compile time and at run time.
///
/// `Float` is deliberately excluded: an inexact literal keeps `Float`'s own
/// shortest-repr rounding, which the compile-time representation does not
/// reproduce. A `Ratio` prints through `Display`, which is `Ratio.__str__`.
fn py_str(val: &ValueObj) -> Option<String> {
    match val {
        ValueObj::Str(s) => Some(s.to_string()),
        ValueObj::Int(i) => Some(i.to_string()),
        ValueObj::Nat(n) => Some(n.to_string()),
        ValueObj::Bool(b) => Some(if *b { "True" } else { "False" }.to_string()),
        ValueObj::None => Some("None".to_string()),
        ValueObj::Ratio(..) => Some(val.to_string()),
        _ => None,
    }
}

/// The integral value of `val` under Python's `int()`: `Float`s truncate towards
/// zero and `Str`s are parsed as decimal. `None` if `int(val)` would raise.
fn py_int(val: &ValueObj) -> Option<i128> {
    match val {
        ValueObj::Int(i) => Some(*i as i128),
        ValueObj::Nat(n) => Some(*n as i128),
        ValueObj::Bool(b) => Some(*b as i128),
        // `int(Fraction(37, 10)) == 3`: truncate towards zero
        ValueObj::Ratio(n, d) => Some(*n / i128::try_from(*d).ok()?),
        ValueObj::Float(f) if f.is_finite() => {
            let t = f.trunc();
            // `i64::MAX as f64` rounds up to 2**63, which `as i64` would saturate
            (-9223372036854775808.0..9223372036854775808.0)
                .contains(&t)
                .then_some(t as i128)
        }
        ValueObj::Str(s) => match parse_py_int(s, 10) {
            IntParse::Parsed(i) => Some(i),
            _ => None,
        },
        _ => None,
    }
}

/// Remove Python's `_` digit separators, or `None` if they are misplaced
/// (Python only allows one between two digits).
fn strip_py_separators(s: &str) -> Option<String> {
    if !s.contains('_') {
        return Some(s.to_string());
    }
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'_'
            && !(i > 0
                && bytes[i - 1].is_ascii_digit()
                && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            return None;
        }
    }
    Some(s.replace('_', ""))
}

/// Outcome of parsing a string the way Python's `int(s, base)` does.
enum IntParse {
    Parsed(i128),
    /// The digits are valid, but the value does not fit `i128`.
    TooLarge,
    /// `int()` would raise `ValueError`.
    Invalid,
}

/// Python's `int(s, base)`. `base` 0 means "detect from the `0x`/`0o`/`0b` prefix".
fn parse_py_int(src: &str, base: u32) -> IntParse {
    if base != 0 && !(2..=36).contains(&base) {
        return IntParse::Invalid;
    }
    let src = src.trim();
    let (neg, body) = match src.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, src.strip_prefix('+').unwrap_or(src)),
    };
    let lower = body.to_ascii_lowercase();
    let (base, digits, prefixed) = match base {
        16 | 0 if lower.starts_with("0x") => (16, &body[2..], true),
        8 | 0 if lower.starts_with("0o") => (8, &body[2..], true),
        2 | 0 if lower.starts_with("0b") => (2, &body[2..], true),
        // without a prefix, base 0 means base 10 -- but then redundant leading
        // zeros are rejected (`int("010", 0)` raises, `int("00", 0)` is 0)
        0 => {
            if body.starts_with('0') && !body.trim_start_matches(['0', '_']).is_empty() {
                return IntParse::Invalid;
            }
            (10, body, false)
        }
        b => (b, body, false),
    };
    // `_` may only separate digits (Python also allows one right after a prefix)
    let mut prev_underscore = !prefixed;
    for c in digits.chars() {
        if c == '_' {
            if prev_underscore {
                return IntParse::Invalid;
            }
            prev_underscore = true;
        } else {
            prev_underscore = false;
        }
    }
    if prev_underscore {
        return IntParse::Invalid;
    }
    let digits = digits.replace('_', "");
    // `from_str_radix` accepts a sign, which would make `int("--1")` parse
    if digits.starts_with(['+', '-']) {
        return IntParse::Invalid;
    }
    match i128::from_str_radix(&digits, base) {
        Ok(i) => IntParse::Parsed(if neg { -i } else { i }),
        Err(e)
            if matches!(
                e.kind(),
                IntErrorKind::PosOverflow | IntErrorKind::NegOverflow
            ) =>
        {
            IntParse::TooLarge
        }
        Err(_) => IntParse::Invalid,
    }
}

fn value_error(msg: String) -> EvalValueError {
    ErrorCore::new(
        vec![SubMessage::only_loc(Location::Unknown)],
        msg,
        line!() as usize,
        ErrorKind::ValueError,
        Location::Unknown,
    )
    .into()
}

/// `int obj` / `int(obj, base)`, following Python's `int()`.
pub(crate) fn int_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let obj = args
        .remove_left_or_key("obj")
        .ok_or_else(|| not_passed("obj"))?;
    let parsed = match args.remove_left_or_key("base") {
        // `int(s, base)` only accepts a string, exactly as in Python
        Some(base) => {
            let Some(radix) = py_int(&base).and_then(|b| u32::try_from(b).ok()) else {
                return Err(type_mismatch("0 or 2..36", base, "base"));
            };
            if radix != 0 && !(2..=36).contains(&radix) {
                return Err(value_error(
                    "int() base must be >= 2 and <= 36, or 0".to_string(),
                ));
            }
            let Some(s) = obj.as_str() else {
                return Err(type_mismatch("Str", obj, "obj"));
            };
            match parse_py_int(s, radix) {
                IntParse::Parsed(i) => i,
                IntParse::TooLarge => {
                    return Err(not_foldable("int", "the result does not fit an `Int`"))
                }
                IntParse::Invalid => {
                    return Err(value_error(format!(
                        "invalid literal for int() with base {radix}: {s}"
                    )))
                }
            }
        }
        None => match py_int(&obj) {
            Some(i) => i,
            None => return Err(type_mismatch("Int-convertible", obj, "obj")),
        },
    };
    Ok(ValueObj::from_i128(parsed)
        .ok_or_else(|| not_foldable("int", "the result does not fit an `Int`"))?
        .into())
}

/// `nat obj`. `Nat(_)` raises at run time for negative inputs, so this rejects them too.
pub(crate) fn nat_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let obj = args
        .remove_left_or_key("obj")
        .ok_or_else(|| not_passed("obj"))?;
    match py_int(&obj) {
        Some(i) if i < 0 => Err(value_error(format!("Nat can't be negative: {i}"))),
        Some(i) => Ok(u64::try_from(i)
            .map(ValueObj::Nat)
            .map_err(|_| not_foldable("nat", "the result does not fit a `Nat`"))?
            .into()),
        None => Err(type_mismatch("Nat-convertible", obj, "obj")),
    }
}

/// `float obj`
pub(crate) fn float_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    // `obj` is a default parameter: `float() == 0.0`
    let Some(obj) = args.remove_left_or_key("obj") else {
        return Ok(ValueObj::from(0.0).into());
    };
    let f = match &obj {
        ValueObj::Float(f) => Some(**f),
        ValueObj::Ratio(n, d) => Some(*n as f64 / *d as f64),
        ValueObj::Int(i) => Some(*i as f64),
        ValueObj::Nat(n) => Some(*n as f64),
        ValueObj::Bool(b) => Some(*b as u8 as f64),
        ValueObj::Str(s) => strip_py_separators(s.trim()).and_then(|s| s.parse::<f64>().ok()),
        _ => None,
    };
    match f {
        Some(f) => Ok(ValueObj::from(f).into()),
        None => Err(type_mismatch("Float-convertible", obj, "obj")),
    }
}

/// `bool obj`. The truth value CPython gives the object the program will
/// actually hold: what is empty or zero is false, and what cannot be decided
/// here (a type, a subroutine, a record) is left to run time.
pub(crate) fn bool_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    // `obj` is a default parameter: `bool() == False`
    let Some(obj) = args.remove_left_or_key("obj") else {
        return Ok(ValueObj::Bool(false).into());
    };
    let b = match &obj {
        ValueObj::Bool(b) => Some(*b),
        ValueObj::Int(i) => Some(*i != 0),
        ValueObj::Nat(n) => Some(*n != 0),
        // kept in lowest terms with a positive denominator, so the numerator decides
        ValueObj::Ratio(n, _) => Some(*n != 0),
        ValueObj::Float(f) => Some(**f != 0.0),
        ValueObj::Str(s) => Some(!s.is_empty()),
        ValueObj::List(l) | ValueObj::Tuple(l) => Some(!l.is_empty()),
        ValueObj::Set(s) => Some(!s.is_empty()),
        ValueObj::Dict(d) => Some(!d.is_empty()),
        ValueObj::None => Some(false),
        ValueObj::Ellipsis => Some(true),
        _ => None,
    };
    match b {
        Some(b) => Ok(ValueObj::Bool(b).into()),
        None => Err(not_foldable(
            format!("bool({obj})"),
            "what the object is at run time decides this",
        )),
    }
}

/// `round number`. Ties round to even, as in Python (`round 2.5 == 2`).
pub(crate) fn round_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let number = args
        .remove_left_or_key("number")
        .ok_or_else(|| not_passed("number"))?;
    // A `Ratio` is a `Fraction` at run time, which rounds exactly -- going
    // through `f64` would round the wrong way for a fraction that is only just
    // off the halfway point.
    if let Some((num, den)) = number.as_ratio() {
        let (Some(q), Some(rest)) = (num.checked_div(den), num.checked_rem(den)) else {
            return Err(not_foldable("round", "the result does not fit an `Int`"));
        };
        // `floor` the quotient, so `rest` is the non-negative distance above it
        let (q, rest) = if rest < 0 {
            (q - 1, rest + den)
        } else {
            (q, rest)
        };
        // `rest` vs `den / 2` rather than `rest * 2` vs `den`, which overflows
        // once the denominator passes i128::MAX / 2. An odd denominator can
        // never land exactly halfway.
        let half = den / 2;
        let round_up = rest > half || (den % 2 == 0 && rest == half && q % 2 != 0);
        let rounded = if round_up { q + 1 } else { q };
        return match i64::try_from(rounded) {
            Ok(i) => Ok(ValueObj::Int(i).into()),
            Err(_) => Err(not_foldable("round", "the result does not fit an `Int`")),
        };
    }
    let Some(f) = number.as_float().filter(|f| f.is_finite()) else {
        // `round` of NaN/inf raises at run time, so it must not fold to a value
        return Err(type_mismatch("finite Float", number, "number"));
    };
    let lower = f.floor();
    let diff = f - lower;
    // exactly halfway: round to the even neighbour
    let round_up = diff > 0.5 || (diff == 0.5 && (lower / 2.0).fract() != 0.0);
    let rounded = if round_up { lower + 1.0 } else { lower };
    if rounded.is_finite() {
        Ok(ValueObj::Int(rounded as i64).into())
    } else {
        Err(not_foldable("round", "the result does not fit an `Int`"))
    }
}

/// `ord c`
pub(crate) fn ord_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let c = args
        .remove_left_or_key("c")
        .ok_or_else(|| not_passed("c"))?;
    let Some(s) = c.as_str() else {
        return Err(type_mismatch("Str", c, "c"));
    };
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(ValueObj::Nat(c as u64).into()),
        _ => Err(value_error(format!(
            "ord() expected a character, but string of length {} found",
            s.chars().count()
        ))),
    }
}

/// `chr i`
pub(crate) fn chr_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let i = args
        .remove_left_or_key("i")
        .ok_or_else(|| not_passed("i"))?;
    let Some(n) = py_int(&i).and_then(|n| u32::try_from(n).ok()) else {
        return Err(type_mismatch("0..1114111", i, "i"));
    };
    match char::from_u32(n) {
        Some(c) => Ok(ValueObj::Str(c.to_string().into()).into()),
        // lone surrogates are valid for Python's `chr` but have no `char`
        None if n <= 0x10FFFF => Err(not_foldable("chr", "a lone surrogate is not a `Str` here")),
        None => Err(value_error("chr() arg not in range(0x110000)".to_string())),
    }
}

fn radix_repr(val: &ValueObj, param: &str, prefix: &str, radix: u32) -> EvalValueResult<TyParam> {
    let Some(i) = py_int(val).filter(|_| !matches!(val, ValueObj::Float(_) | ValueObj::Str(_)))
    else {
        return Err(type_mismatch("Int", val, param));
    };
    let digits = match radix {
        2 => format!("{:b}", i.unsigned_abs()),
        8 => format!("{:o}", i.unsigned_abs()),
        _ => format!("{:x}", i.unsigned_abs()),
    };
    let sign = if i < 0 { "-" } else { "" };
    Ok(ValueObj::Str(format!("{sign}{prefix}{digits}").into()).into())
}

/// `bin n` (e.g. `bin 5 == "0b101"`)
pub(crate) fn bin_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let n = args
        .remove_left_or_key("n")
        .ok_or_else(|| not_passed("n"))?;
    radix_repr(&n, "n", "0b", 2)
}

/// `oct X` (e.g. `oct 8 == "0o10"`)
pub(crate) fn oct_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let x = args
        .remove_left_or_key("X")
        .ok_or_else(|| not_passed("X"))?;
    radix_repr(&x, "X", "0o", 8)
}

/// `hex n` (e.g. `hex 255 == "0xff"`)
pub(crate) fn hex_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let n = args
        .remove_left_or_key("n")
        .ok_or_else(|| not_passed("n"))?;
    radix_repr(&n, "n", "0x", 16)
}

/// `pow base, exp` == `base ** exp`
pub(crate) fn pow_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let base = args
        .remove_left_or_key("base")
        .ok_or_else(|| not_passed("base"))?;
    let exp = args
        .remove_left_or_key("exp")
        .ok_or_else(|| not_passed("exp"))?;
    match base.clone().try_pow(exp.clone()) {
        Some(v) => Ok(v.into()),
        None => Err(not_foldable(
            format!("pow({base}, {exp})"),
            "the result does not fit an `Int`",
        )),
    }
}

/// `divmod a, b` == `(a // b, a % b)`
pub(crate) fn divmod_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let a = args
        .remove_left_or_key("a")
        .ok_or_else(|| not_passed("a"))?;
    let b = args
        .remove_left_or_key("b")
        .ok_or_else(|| not_passed("b"))?;
    if b.is_zero() {
        return Err(value_error(
            "integer division or modulo by zero".to_string(),
        ));
    }
    let (Some(div), Some(rem)) = (
        a.clone().try_floordiv(b.clone()),
        a.clone().try_mod(b.clone()),
    ) else {
        return Err(not_foldable(
            format!("divmod({a}, {b})"),
            "the result does not fit an `Int`",
        ));
    };
    Ok(ValueObj::Tuple(vec![div, rem].into()).into())
}

/// `sorted iterable`
pub(crate) fn sorted_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let iterable = args
        .remove_left_or_key("iterable")
        .ok_or_else(|| not_passed("iterable"))?;
    let mut elems = match &iterable {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.iter().cloned().collect(),
        _ => return Err(type_mismatch("Iterable(Ord)", iterable, "iterable")),
    };
    // every pair must be comparable, otherwise the run-time `sorted` would raise
    for w in elems.windows(2) {
        if w[0].try_cmp(&w[1]).is_none() {
            return Err(type_mismatch("Ord", &w[0], "iterable.next()"));
        }
    }
    let mut failed = false;
    elems.sort_by(|l, r| {
        l.try_cmp(r).unwrap_or_else(|| {
            failed = true;
            std::cmp::Ordering::Equal
        })
    });
    if failed {
        return Err(type_mismatch("Ord", iterable, "iterable"));
    }
    Ok(ValueObj::List(elems.into()).into())
}

pub(crate) fn resolve_path_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let path = args
        .remove_left_or_key("Path")
        .ok_or_else(|| not_passed("Path"))?;
    let path = match &path {
        ValueObj::Str(s) => Path::new(&s[..]),
        other => {
            return Err(type_mismatch("Str", other, "Path"));
        }
    };
    let Some(path) = ctx.cfg.input.resolve_path(path, &ctx.cfg) else {
        return Err(ErrorCore::new(
            vec![SubMessage::only_loc(Location::Unknown)],
            format!("Path {} is not found", path.display()),
            line!() as usize,
            ErrorKind::IoError,
            Location::Unknown,
        )
        .into());
    };
    Ok(ValueObj::Str(path.to_string_lossy().into()).into())
}

pub(crate) fn resolve_decl_path_func(
    mut args: ValueArgs,
    ctx: &Context,
) -> EvalValueResult<TyParam> {
    let path = args
        .remove_left_or_key("Path")
        .ok_or_else(|| not_passed("Path"))?;
    let path = match &path {
        ValueObj::Str(s) => Path::new(&s[..]),
        other => {
            return Err(type_mismatch("Str", other, "Path"));
        }
    };
    if let Some(path) = ctx.cfg.input.resolve_decl_path(path, &ctx.cfg) {
        return Ok(ValueObj::Str(path.to_string_lossy().into()).into());
    }
    // Fallback: resolve .py file directly for untyped pyimport
    if let Ok(resolved) = ctx.cfg.input.resolve_py(path) {
        return Ok(ValueObj::Str(resolved.to_string_lossy().into()).into());
    }
    for sys_path in python_sys_path() {
        let mut dir = sys_path.clone();
        dir.push(path);
        dir.set_extension("py");
        if dir.exists() {
            return Ok(ValueObj::Str(normalize_path(dir).to_string_lossy().into()).into());
        }
        let mut dir = sys_path.clone();
        dir.push(path);
        dir.push("__init__.py");
        if dir.exists() {
            return Ok(ValueObj::Str(normalize_path(dir).to_string_lossy().into()).into());
        }
    }
    for pkgs_path in python_site_packages() {
        let mut dir = pkgs_path.clone();
        dir.push(path);
        dir.set_extension("py");
        if dir.exists() {
            return Ok(ValueObj::Str(normalize_path(dir).to_string_lossy().into()).into());
        }
        let mut dir = pkgs_path.clone();
        dir.push(path);
        dir.push("__init__.py");
        if dir.exists() {
            return Ok(ValueObj::Str(normalize_path(dir).to_string_lossy().into()).into());
        }
    }
    Err(ErrorCore::new(
        vec![SubMessage::only_loc(Location::Unknown)],
        format!("Path {} is not found", path.display()),
        line!() as usize,
        ErrorKind::IoError,
        Location::Unknown,
    )
    .into())
}

pub(crate) fn succ_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("n")
        .ok_or_else(|| not_passed("n"))?;
    let val = match &val {
        ValueObj::Bool(b) => ValueObj::Nat(*b as u64 + 1),
        ValueObj::Nat(n) => ValueObj::Nat(n + 1),
        ValueObj::Int(n) => ValueObj::Int(n + 1),
        ValueObj::Float(n) => ValueObj::from(**n + f64::EPSILON),
        v @ (ValueObj::Inf | ValueObj::NegInf) => v.clone(),
        _ => {
            return Err(type_mismatch("Number", val, "n"));
        }
    };
    Ok(val.into())
}

pub(crate) fn pred_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("n")
        .ok_or_else(|| not_passed("n"))?;
    let val = match &val {
        ValueObj::Bool(b) => ValueObj::Nat((*b as u64).saturating_sub(1)),
        ValueObj::Nat(n) => ValueObj::Nat(n.saturating_sub(1)),
        ValueObj::Int(n) => ValueObj::Int(n - 1),
        ValueObj::Float(n) => ValueObj::from(**n - f64::EPSILON),
        v @ (ValueObj::Inf | ValueObj::NegInf) => v.clone(),
        _ => {
            return Err(type_mismatch("Number", val, "n"));
        }
    };
    Ok(val.into())
}

// TODO: varargs
pub(crate) fn zip_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let iterable1 = args
        .remove_left_or_key("iterable1")
        .ok_or_else(|| not_passed("iterable1"))?;
    let iterable2 = args
        .remove_left_or_key("iterable2")
        .ok_or_else(|| not_passed("iterable2"))?;
    let iterable1 = match iterable1 {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        _ => {
            return Err(type_mismatch("Iterable(T)", iterable1, "iterable1"));
        }
    };
    let iterable2 = match iterable2 {
        ValueObj::List(a) => a.to_vec(),
        ValueObj::Tuple(t) => t.to_vec(),
        ValueObj::Set(s) => s.into_iter().collect(),
        _ => {
            return Err(type_mismatch("Iterable(T)", iterable2, "iterable2"));
        }
    };
    let mut zipped = vec![];
    for (v1, v2) in iterable1.into_iter().zip(iterable2) {
        zipped.push(ValueObj::Tuple(vec![v1, v2].into()));
    }
    Ok(TyParam::Value(ValueObj::List(zipped.into())))
}

/// ```erg
/// derefine({X: T | ...}) == T
/// derefine({1}) == Nat
/// derefine(List!({1, 2}, 2)) == List!(Nat, 2)
/// ```
pub(crate) fn derefine_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("T")
        .ok_or_else(|| not_passed("T"))?;
    let t = match ctx.convert_value_into_type(val) {
        Ok(t) => t.derefine(),
        Err(val) => {
            return Err(type_mismatch("Type", val, "T"));
        }
    };
    Ok(TyParam::t(t))
}

/// ```erg
/// fill_ord({1, 4}) == {1, 2, 3, 4}
/// fill_ord({"a", "c"}) == {"a", "b", "c"}
/// ```
pub(crate) fn fill_ord_func(mut args: ValueArgs, ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("T")
        .ok_or_else(|| not_passed("T"))?;
    let t = match ctx.convert_value_into_type(val) {
        Ok(t) => {
            let coerced = ctx.coerce(t.clone(), &()).unwrap_or(t);
            let inf = ctx.inf(&coerced);
            let sup = ctx.sup(&coerced);
            let der = coerced.derefine();
            match (inf, sup) {
                (Some(inf), Some(sup)) => closed_range(der, inf, sup),
                _ => coerced,
            }
        }
        Err(val) => {
            return Err(type_mismatch("Type", val, "T"));
        }
    };
    Ok(TyParam::t(t))
}

/// TODO: accept non-const expr
/// ```erg
/// classof(1) == Nat
/// ```
pub(crate) fn classof_func(mut args: ValueArgs, _ctx: &Context) -> EvalValueResult<TyParam> {
    let val = args
        .remove_left_or_key("obj")
        .ok_or_else(|| not_passed("obj"))?;
    Ok(TyParam::t(val.class()))
}

#[cfg(test)]
mod tests {
    use erg_common::config::ErgConfig;

    use crate::module::SharedCompilerResource;
    use crate::ty::constructors::*;

    use super::*;

    #[test]
    fn test_dict_items() {
        let cfg = ErgConfig::default();
        let shared = SharedCompilerResource::default();
        Context::init_builtins(cfg, shared.clone());
        let ctx = &shared.raw_ref_builtins_ctx().unwrap().context;

        let a = singleton(Type::Str, TyParam::value("a"));
        let b = singleton(Type::Str, TyParam::value("b"));
        let c = singleton(Type::Str, TyParam::value("c"));
        // FIXME: order dependency
        let k_union = ctx.union(&ctx.union(&c, &a), &b);
        let g_dic = singleton(Type::Type, TyParam::t(mono("GenericDict")));
        let g_lis = singleton(Type::Type, TyParam::t(mono("GenericList")));
        let sub = func0(singleton(Type::NoneType, TyParam::value(ValueObj::None)));
        let v_union = ctx.union(&ctx.union(&g_lis, &sub), &g_dic);
        let dic = dict! {
            a.clone() => sub.clone(),
            b.clone() => g_dic.clone(),
            c.clone() => g_lis.clone(),
        };
        match ctx.eval_proj_call_t(dic.clone().into(), "items".into(), vec![], 1, &()) {
            Ok(t) => {
                let items = tuple_t(vec![k_union, v_union]);
                assert_eq!(t, items, "{t} != {items}");
            }
            Err(e) => {
                panic!("ERR: {e}");
            }
        }
    }
}
