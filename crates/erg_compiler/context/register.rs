use std::option::Option;
use std::path::{Path, PathBuf};

use erg_common::consts::{ERG_MODE, PYTHON_MODE};
use erg_common::dict::Dict;
use erg_common::env::{is_pystd_main_module, python_site_packages, python_sys_path};
use erg_common::erg_util::BUILTIN_ERG_MODS;
use erg_common::levenshtein::get_similar_name;
use erg_common::normalize_path;
use erg_common::pathutil::{DirKind, FileKind, NormalizedPathBuf};
use erg_common::python_util::BUILTIN_PYTHON_MODS;
use erg_common::set::Set;
use erg_common::traits::{Locational, Stream, StructuralEq};
use erg_common::triple::Triple;
use erg_common::{get_hash, log, set, unique_in_place, Str};

use ast::{
    ConstIdentifier, Decorator, DefId, Identifier, OperationKind, PolyTypeSpec, PreDeclTypeSpec,
    VarName,
};
use erg_parser::ast::{self, ClassAttr, RecordAttrOrIdent, TypeSpecWithOp};
use erg_parser::Parser;

use crate::ty::constructors::{
    func, func0, func1, module, named_free_var, poly, proc, py_module, ref_, ref_mut, str_dict_t,
    subr_t, tp_enum, unknown_len_list_t, v_enum,
};
use crate::ty::free::{Constraint, HasLevel};
use crate::ty::typaram::TyParam;
use crate::ty::value::{GenTypeObj, TypeObj, ValueObj};
use crate::ty::{
    CastTarget, ConstSubr, Field, GuardType, HasType, ParamTy, SubrKind, SubrType, Type,
    UserConstSubr, Visibility, VisibilityModifier,
};

use crate::build_package::CheckStatus;
use crate::context::{
    ClassDefType, Context, ContextKind, DefaultInfo, ModuleContext, RegistrationMode,
};
use crate::error::{concat_result, readable_name, Failable};
use crate::error::{
    CompileError, CompileErrors, CompileResult, TyCheckError, TyCheckErrors, TyCheckResult,
};
use crate::hir::Literal;
use crate::varinfo::{AbsLocation, AliasInfo, Mutability, VarInfo, VarKind};
use crate::{feature_error, hir, unreachable_error};
use Mutability::*;
use RegistrationMode::*;

use super::eval::Substituter;
use super::instantiate::TyVarCache;
use super::instantiate_spec::ParamKind;
use super::{ControlKind, MethodContext, ParamSpec, TraitImpl, TypeContext};

type ClassTrait<'c> = (Type, Option<(Type, &'c TypeSpecWithOp)>);
type ClassTraitErrors<'c> = (
    Option<Type>,
    Option<(Type, &'c TypeSpecWithOp)>,
    TyCheckErrors,
);

pub fn valid_mod_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('/') && name.trim() == name
}

const UBAR: &Str = &Str::ever("_");

impl Context {
    /// If it is a constant that is defined, there must be no variable of the same name defined across all scopes
    pub(crate) fn registered_info(
        &self,
        name: &str,
        is_const: bool,
    ) -> Option<(&VarName, &VarInfo)> {
        if let Some((name, vi)) = self.params.iter().find(|(maybe_name, _)| {
            maybe_name
                .as_ref()
                .map(|n| &n.inspect()[..] == name)
                .unwrap_or(false)
        }) {
            return Some((name.as_ref().unwrap(), vi));
        } else if let Some((name, vi)) = self.locals.get_key_value(name) {
            return Some((name, vi));
        }
        if is_const {
            let outer = self.get_outer_scope_or_builtins()?;
            outer.registered_info(name, is_const)
        } else {
            None
        }
    }

    /// Whether a constant defined here could have a compile-time value at all.
    ///
    /// Only a const subroutine's parameters have one, and they are the ones
    /// spelled in uppercase: `F(N: Int)` binds `N` per call, `f(n: Int)` never
    /// binds `n` at compile time. A `do` block is transparent to this -- a
    /// constant inside an `if` branch of a const function is still per call.
    fn has_const_param(&self) -> bool {
        if self
            .params
            .iter()
            .any(|(name, _)| name.as_ref().is_some_and(|n| n.inspect().is_uppercase()))
        {
            return true;
        }
        self.get_outer()
            .filter(|outer| !outer.kind.is_module())
            .is_some_and(Self::has_const_param)
    }

    /// Whether a name bound in this scope may be bound again by a later
    /// definition. The REPL's top level is one scope for the whole session, so
    /// there a later input may redefine what an earlier one bound, as in GHCi or
    /// the Scala REPL; a file, and any scope inside a REPL input, may not
    /// (doc/EN/syntax/02_name.md).
    pub(crate) fn allows_rebinding(&self) -> bool {
        self.cfg.input.is_repl() && self.kind == ContextKind::Module
    }
    fn declare_var(&mut self, ident: &Identifier, t_spec: &TypeSpecWithOp) -> Failable<()> {
        if self.decls.get(&ident.name).is_some()
            || self
                .future_defined_locals
                .get(&ident.name)
                .is_some_and(|future| future.def_loc.loc < ident.loc())
        {
            return Ok(());
        }
        let mut errs = TyCheckErrors::empty();
        let vis = match self.instantiate_vis_modifier(&ident.vis) {
            Ok(vis) => vis,
            Err(es) => {
                errs.extend(es);
                VisibilityModifier::Public
            }
        };
        let kind = VarKind::Declared;
        let sig_t = match self.instantiate_var_sig_t(Some(&t_spec.t_spec), PreRegister) {
            Ok(t) => t,
            Err((t, _es)) => {
                // errs.extend(es);
                t
            }
        };
        let py_name = if let ContextKind::PatchMethodDefs(_base) = &self.kind {
            Some(Str::from(format!("::{}{}", self.name, ident)))
        } else {
            None
        };
        let vi = VarInfo::new(
            sig_t,
            Mutability::from(&ident.inspect()[..]),
            Visibility::new(vis, self.name.clone()),
            kind,
            None,
            self.kind.clone(),
            py_name,
            self.absolutize(ident.name.loc()),
        );
        // self.index().register(ident.inspect().clone(), &vi);
        self.decls.insert(ident.name.clone(), vi);
        if errs.is_empty() {
            Ok(())
        } else {
            Err(((), errs))
        }
    }

    fn pre_define_var(&mut self, sig: &ast::VarSignature, id: Option<DefId>) -> Failable<()> {
        let mut errs = TyCheckErrors::empty();
        let muty = Mutability::from(&sig.inspect().unwrap_or(UBAR)[..]);
        let ident = match &sig.pat {
            ast::VarPattern::Ident(ident) | ast::VarPattern::Phi(ident) => ident,
            ast::VarPattern::Discard(_) | ast::VarPattern::Glob(_) => {
                return Ok(());
            }
            other => unreachable!("{other}"),
        };
        let vis = match self.instantiate_vis_modifier(&ident.vis) {
            Ok(vis) => vis,
            Err(es) => {
                errs.extend(es);
                VisibilityModifier::Public
            }
        };
        let kind = id.map_or(VarKind::Declared, VarKind::Defined);
        let sig_t = match self
            .instantiate_var_sig_t(sig.t_spec.as_ref().map(|ts| &ts.t_spec), PreRegister)
        {
            Ok(t) => t,
            Err((t, _es)) => {
                // errs.extend(es);
                t
            }
        };
        let py_name = if let ContextKind::PatchMethodDefs(_base) = &self.kind {
            Some(Str::from(format!("::{}{}", self.name, ident)))
        } else {
            None
        };
        if self.decls.get(&ident.name).is_some() {
            let vi = VarInfo::new(
                sig_t,
                muty,
                Visibility::new(vis, self.name.clone()),
                kind,
                None,
                self.kind.clone(),
                py_name,
                self.absolutize(ident.name.loc()),
            );
            self.index().register(ident.inspect().clone(), &vi);
            return Ok(());
        }
        // don't remove at this point
        if !self.allows_rebinding()
            && self
                .get_class_attr(ident.name.inspect())
                .is_some_and(|(_, decl)| !decl.kind.is_auto())
        {
            errs.push(TyCheckError::duplicate_decl_error(
                self.cfg.input.clone(),
                line!() as usize,
                sig.loc(),
                self.caused_by(),
                ident.name.inspect(),
            ));
            Err(((), errs))
        } else {
            let vi = VarInfo::new(
                sig_t,
                muty,
                Visibility::new(vis, self.name.clone()),
                kind,
                None,
                self.kind.clone(),
                py_name,
                self.absolutize(ident.name.loc()),
            );
            self.index().register(ident.inspect().clone(), &vi);
            self.future_defined_locals.insert(ident.name.clone(), vi);
            if errs.is_empty() {
                Ok(())
            } else {
                Err(((), errs))
            }
        }
    }

    pub(crate) fn declare_sub(
        &mut self,
        sig: &ast::SubrSignature,
        id: Option<DefId>,
    ) -> TyCheckResult<()> {
        let name = sig.ident.inspect();
        let vis = self.instantiate_vis_modifier(&sig.ident.vis)?;
        let muty = Mutability::from(&name[..]);
        let kind = id.map_or(VarKind::Declared, VarKind::Defined);
        let comptime_decos = sig
            .decorators
            .iter()
            .filter_map(|deco| match &deco.0 {
                ast::Expr::Accessor(ast::Accessor::Ident(local)) if local.is_const() => {
                    Some(local.inspect().clone())
                }
                _ => None,
            })
            .collect::<Set<_>>();
        let (errs, t) = match self.instantiate_sub_sig_t(sig, PreRegister) {
            Ok(t) => (TyCheckErrors::empty(), t),
            Err((t, errs)) => (errs, t),
        };
        let py_name = if let ContextKind::PatchMethodDefs(_base) = &self.kind {
            Some(Str::from(format!("::{}{}", self.name, sig.ident)))
        } else {
            None
        };
        let vi = VarInfo::new(
            t,
            muty,
            Visibility::new(vis, self.name.clone()),
            kind,
            Some(comptime_decos),
            self.kind.clone(),
            py_name,
            self.absolutize(sig.ident.name.loc()),
        );
        self.index().register(sig.ident.inspect().clone(), &vi);
        self.decls.remove(name);
        if self
            .remove_class_attr(name)
            .is_some_and(|(_, decl)| !decl.kind.is_auto())
            && !self.allows_rebinding()
        {
            Err(TyCheckErrors::from(TyCheckError::duplicate_decl_error(
                self.cfg.input.clone(),
                line!() as usize,
                sig.loc(),
                self.caused_by(),
                name,
            )))
        } else {
            self.decls.insert(sig.ident.name.clone(), vi);
            if errs.is_empty() {
                Ok(())
            } else {
                Err(errs)
            }
        }
    }

    /// already validated
    pub(crate) fn assign_var_sig(
        &mut self,
        sig: &ast::VarSignature,
        body_t: &Type,
        id: DefId,
        expr: Option<&hir::Expr>,
        py_name: Option<Str>,
    ) -> TyCheckResult<VarInfo> {
        let alias_of = if let Some((origin, name)) =
            expr.and_then(|exp| exp.var_info().zip(exp.last_name()))
        {
            Some(AliasInfo::new(
                name.inspect().clone(),
                origin.def_loc.clone(),
            ))
        } else {
            None
        };
        let ident = match &sig.pat {
            ast::VarPattern::Ident(ident) | ast::VarPattern::Phi(ident) => ident,
            ast::VarPattern::Discard(_) => {
                return Ok(VarInfo {
                    t: body_t.clone(),
                    ctx: self.kind.clone(),
                    def_loc: self.absolutize(sig.loc()),
                    py_name,
                    alias_of: alias_of.map(Box::new),
                    ..VarInfo::const_default_private()
                });
            }
            _ => unreachable!(),
        };
        if let Some(py_name) = &py_name {
            self.erg_to_py_names
                .insert(ident.inspect().clone(), py_name.clone());
        }
        let ident = if PYTHON_MODE && py_name.is_some() {
            let mut symbol = ident.name.clone().into_token();
            symbol.content = py_name.clone().unwrap();
            Identifier::new(ident.vis.clone(), VarName::new(symbol))
        } else {
            ident.clone()
        };
        let vis = self.instantiate_vis_modifier(&ident.vis)?;
        // already defined as const
        if sig.is_const() {
            let mut vi = self.decls.remove(ident.inspect()).unwrap_or_else(|| {
                VarInfo::new(
                    body_t.clone(),
                    Mutability::Const,
                    Visibility::new(vis, self.name.clone()),
                    VarKind::Declared,
                    None,
                    self.kind.clone(),
                    py_name.clone(),
                    self.absolutize(ident.name.loc()),
                )
            });
            if vi.py_name.is_none() {
                vi.py_name = py_name;
            }
            if vi.alias_of.is_none() {
                vi.alias_of = alias_of.map(Box::new);
            }
            self.locals.insert(ident.name.clone(), vi.clone());
            if let Ok(value) = self.convert_singular_type_into_value(vi.t.clone()) {
                self.consts.insert(ident.name.clone(), value);
            }
            return Ok(vi);
        }
        let muty = Mutability::from(&ident.inspect()[..]);
        let opt_vi = self
            .decls
            .remove(ident.inspect())
            .or_else(|| self.future_defined_locals.remove(ident.inspect()));
        let py_name = opt_vi
            .as_ref()
            .and_then(|vi| vi.py_name.clone())
            .or(py_name);
        let kind = if id.0 == 0 {
            VarKind::Declared
        } else {
            VarKind::Defined(id)
        };
        let t = sig.t_spec.as_ref().map_or(body_t.clone(), |ts| {
            if ts.ascription_kind().is_force_cast() {
                match self.instantiate_typespec(&ts.t_spec) {
                    Ok(t) => t,
                    Err((t, _)) => t,
                }
            } else {
                body_t.clone()
            }
        });
        let vi = VarInfo::maybe_alias(
            t,
            muty,
            Visibility::new(vis, self.name.clone()),
            kind,
            None,
            self.kind.clone(),
            py_name,
            self.absolutize(ident.name.loc()),
            alias_of,
        );
        log!(info "Registered {}{}: {}", self.name, ident, vi);
        self.locals.insert(ident.name.clone(), vi.clone());
        if let Ok(value) = self.convert_singular_type_into_value(vi.t.clone()) {
            self.consts.insert(ident.name.clone(), value);
        }
        Ok(vi)
    }

    fn type_self_param(
        &self,
        pat: &ast::ParamPattern,
        name: &VarName,
        spec_t: &Type,
        sig_t: Option<&SubrType>,
        errs: &mut TyCheckErrors,
    ) {
        if let Some(self_t) = self.rec_get_self_t() {
            let self_t = match pat {
                ast::ParamPattern::Ref(_) => ref_(self_t),
                ast::ParamPattern::RefMut(_) => ref_mut(self_t, None),
                _ => self_t,
            };
            // spec_t <: self_t
            if let Err(es) = self.sub_unify(spec_t, &self_t, name, Some(name.inspect())) {
                errs.extend(es);
            }
            if let Some(sig_t) = sig_t {
                if sig_t.return_t.has_no_unbound_var() && !sig_t.return_t.contains_type(spec_t) {
                    // spec_t == self_t
                    if let Err(es) = self.sub_unify(&self_t, spec_t, name, Some(name.inspect())) {
                        errs.extend(es);
                    }
                }
            }
        } else {
            log!(err "self_t is None");
        }
    }

    /// TODO: sig should be immutable
    /// 宣言が既にある場合、opt_decl_tに宣言の型を渡す
    fn assign_param(
        &mut self,
        sig: &mut hir::NonDefaultParamSignature,
        opt_decl_t: Option<&ParamTy>,
        sig_t: Option<&SubrType>,
        tmp_tv_cache: &mut TyVarCache,
        kind: ParamKind,
    ) -> TyCheckResult<()> {
        let vis = if PYTHON_MODE {
            Visibility::BUILTIN_PUBLIC
        } else {
            Visibility::private(self.name.clone())
        };
        let default = kind.default_info();
        let is_var_params = kind.is_var_params() || kind.is_kw_var_params();
        match &sig.raw.pat {
            // Literal patterns will be desugared to discard patterns
            ast::ParamPattern::Lit(_) => unreachable!(),
            ast::ParamPattern::Discard(token) => {
                let (spec_t, errs) = match self.instantiate_param_sig_t(
                    &sig.raw,
                    opt_decl_t,
                    tmp_tv_cache,
                    Normal,
                    kind,
                    false,
                ) {
                    Ok(ty) => (ty, TyCheckErrors::empty()),
                    Err((ty, errs)) => (ty, errs),
                };
                let def_id = DefId(get_hash(&(&self.name, "_")));
                let kind = VarKind::parameter(def_id, is_var_params, DefaultInfo::NonDefault);
                let vi = VarInfo::new(
                    spec_t,
                    Immutable,
                    vis,
                    kind,
                    None,
                    self.kind.clone(),
                    None,
                    self.absolutize(token.loc()),
                );
                sig.vi = vi.clone();
                self.params.push((Some(VarName::from_static("_")), vi));
                if errs.is_empty() {
                    Ok(())
                } else {
                    Err(errs)
                }
            }
            ast::ParamPattern::VarName(name) => {
                // A parameter introduces a fresh binding in its own scope and may shadow any
                // outer name, so only a sibling parameter of the same name (a real duplicate,
                // e.g. `f(x, x)`) is a conflict. Pass `is_const = false` to restrict the lookup
                // to the current scope: searching the outer scope (which `is_const = true` does)
                // would wrongly flag e.g. a `match` catch-all branch variable as reassigning the
                // enclosing const function's parameter (recursive const functions with 3+ arms).
                if self.registered_info(name.inspect(), false).is_some()
                    && &name.inspect()[..] != "_"
                {
                    Err(TyCheckErrors::from(TyCheckError::reassign_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        name.loc(),
                        self.caused_by(),
                        name.inspect(),
                    )))
                } else {
                    // ok, not defined
                    let (spec_t, mut errs) = match self.instantiate_param_sig_t(
                        &sig.raw,
                        opt_decl_t,
                        tmp_tv_cache,
                        Normal,
                        kind.clone(),
                        false,
                    ) {
                        Ok(ty) => (ty, TyCheckErrors::empty()),
                        Err((ty, errs)) => (ty, errs),
                    };
                    let spec_t = match kind {
                        ParamKind::VarParams => unknown_len_list_t(spec_t),
                        ParamKind::KwVarParams => str_dict_t(spec_t),
                        _ => spec_t,
                    };
                    if &name.inspect()[..] == "self" {
                        self.type_self_param(&sig.raw.pat, name, &spec_t, sig_t, &mut errs);
                    }
                    let def_id = DefId(get_hash(&(&self.name, name)));
                    let kind = VarKind::parameter(def_id, is_var_params, default);
                    let muty = Mutability::from(&name.inspect()[..]);
                    let vi = VarInfo::new(
                        spec_t,
                        muty,
                        vis,
                        kind,
                        None,
                        self.kind.clone(),
                        None,
                        self.absolutize(name.loc()),
                    );
                    self.index().register(name.inspect().clone(), &vi);
                    sig.vi = vi.clone();
                    self.params.push((Some(name.clone()), vi));
                    if errs.is_empty() {
                        Ok(())
                    } else {
                        Err(errs)
                    }
                }
            }
            ast::ParamPattern::Ref(name) => {
                if self
                    .registered_info(name.inspect(), name.is_const())
                    .is_some()
                {
                    Err(TyCheckErrors::from(TyCheckError::reassign_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        name.loc(),
                        self.caused_by(),
                        name.inspect(),
                    )))
                } else {
                    // ok, not defined
                    let (spec_t, mut errs) = match self.instantiate_param_sig_t(
                        &sig.raw,
                        opt_decl_t,
                        tmp_tv_cache,
                        Normal,
                        kind,
                        false,
                    ) {
                        Ok(ty @ Type::Ref(_)) => (ty, TyCheckErrors::empty()),
                        Ok(ty) => (ty.into_ref(), TyCheckErrors::empty()),
                        Err((ty, errs)) => (ty, errs),
                    };
                    if &name.inspect()[..] == "self" {
                        self.type_self_param(&sig.raw.pat, name, &spec_t, sig_t, &mut errs);
                    }
                    let kind = VarKind::parameter(
                        DefId(get_hash(&(&self.name, name))),
                        is_var_params,
                        default,
                    );
                    let vi = VarInfo::new(
                        spec_t,
                        Immutable,
                        vis,
                        kind,
                        None,
                        self.kind.clone(),
                        None,
                        self.absolutize(name.loc()),
                    );
                    sig.vi = vi.clone();
                    self.params.push((Some(name.clone()), vi));
                    if errs.is_empty() {
                        Ok(())
                    } else {
                        Err(errs)
                    }
                }
            }
            ast::ParamPattern::RefMut(name) => {
                if self
                    .registered_info(name.inspect(), name.is_const())
                    .is_some()
                {
                    Err(TyCheckErrors::from(TyCheckError::reassign_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        name.loc(),
                        self.caused_by(),
                        name.inspect(),
                    )))
                } else {
                    // ok, not defined
                    let (spec_t, mut errs) = match self.instantiate_param_sig_t(
                        &sig.raw,
                        opt_decl_t,
                        tmp_tv_cache,
                        Normal,
                        kind,
                        false,
                    ) {
                        Ok(ty @ Type::RefMut { .. }) => (ty, TyCheckErrors::empty()),
                        Ok(ty) => (ty.into_ref_mut(None), TyCheckErrors::empty()),
                        Err((ty, errs)) => (ty, errs),
                    };
                    if &name.inspect()[..] == "self" {
                        self.type_self_param(&sig.raw.pat, name, &spec_t, sig_t, &mut errs);
                    }
                    let kind = VarKind::parameter(
                        DefId(get_hash(&(&self.name, name))),
                        is_var_params,
                        default,
                    );
                    let vi = VarInfo::new(
                        spec_t,
                        Immutable,
                        vis,
                        kind,
                        None,
                        self.kind.clone(),
                        None,
                        self.absolutize(name.loc()),
                    );
                    sig.vi = vi.clone();
                    self.params.push((Some(name.clone()), vi));
                    if errs.is_empty() {
                        Ok(())
                    } else {
                        Err(errs)
                    }
                }
            }
            other => {
                log!(err "{other}");
                unreachable!("{other}")
            }
        }
    }

    pub(crate) fn assign_params(
        &mut self,
        params: &mut hir::Params,
        tmp_tv_cache: &mut TyVarCache,
        expect: Option<SubrType>,
    ) -> TyCheckResult<()> {
        let mut errs = TyCheckErrors::empty();
        if let Some(subr_t) = expect {
            if params.non_defaults.len() > subr_t.non_default_params.len() {
                let excessive_params = params
                    .non_defaults
                    .iter()
                    .skip(subr_t.non_default_params.len())
                    .collect::<Vec<_>>();
                errs.push(TyCheckError::too_many_args_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    excessive_params.loc(),
                    "<lambda>", // TODO:
                    self.caused_by(),
                    subr_t.non_default_params.len(),
                    params.non_defaults.len(),
                    params.defaults.len(),
                ));
            }
            // debug_assert_eq!(params.defaults.len(), subr_t.default_params.len());
            for (non_default, pt) in params
                .non_defaults
                .iter_mut()
                .zip(subr_t.non_default_params.iter())
            {
                if let Err(es) = self.assign_param(
                    non_default,
                    Some(pt),
                    Some(&subr_t),
                    tmp_tv_cache,
                    ParamKind::NonDefault,
                ) {
                    errs.extend(es);
                }
            }
            if let Some(var_params) = &mut params.var_params {
                if let Some(pt) = &subr_t.var_params {
                    let pt = pt.clone().map_type(&mut unknown_len_list_t);
                    if let Err(es) = self.assign_param(
                        var_params,
                        Some(&pt),
                        Some(&subr_t),
                        tmp_tv_cache,
                        ParamKind::VarParams,
                    ) {
                        errs.extend(es);
                    }
                } else if let Err(es) = self.assign_param(
                    var_params,
                    None,
                    Some(&subr_t),
                    tmp_tv_cache,
                    ParamKind::VarParams,
                ) {
                    errs.extend(es);
                }
            }
            for (default, pt) in params.defaults.iter_mut().zip(subr_t.default_params.iter()) {
                if let Err(es) = self.assign_param(
                    &mut default.sig,
                    Some(pt),
                    Some(&subr_t),
                    tmp_tv_cache,
                    ParamKind::Default(default.default_val.t()),
                ) {
                    errs.extend(es);
                }
            }
            if let Some(kw_var_params) = &mut params.kw_var_params {
                if let Some(pt) = &subr_t.kw_var_params {
                    let pt = pt.clone().map_type(&mut str_dict_t);
                    if let Err(es) = self.assign_param(
                        kw_var_params,
                        Some(&pt),
                        Some(&subr_t),
                        tmp_tv_cache,
                        ParamKind::KwVarParams,
                    ) {
                        errs.extend(es);
                    }
                } else if let Err(es) = self.assign_param(
                    kw_var_params,
                    None,
                    Some(&subr_t),
                    tmp_tv_cache,
                    ParamKind::KwVarParams,
                ) {
                    errs.extend(es);
                }
            }
        } else {
            for non_default in params.non_defaults.iter_mut() {
                if let Err(es) =
                    self.assign_param(non_default, None, None, tmp_tv_cache, ParamKind::NonDefault)
                {
                    errs.extend(es);
                }
            }
            if let Some(var_params) = &mut params.var_params {
                if let Err(es) =
                    self.assign_param(var_params, None, None, tmp_tv_cache, ParamKind::VarParams)
                {
                    errs.extend(es);
                }
            }
            for default in params.defaults.iter_mut() {
                if let Err(es) = self.assign_param(
                    &mut default.sig,
                    None,
                    None,
                    tmp_tv_cache,
                    ParamKind::Default(default.default_val.t()),
                ) {
                    errs.extend(es);
                }
            }
            if let Some(kw_var_params) = &mut params.kw_var_params {
                if let Err(es) = self.assign_param(
                    kw_var_params,
                    None,
                    None,
                    tmp_tv_cache,
                    ParamKind::KwVarParams,
                ) {
                    errs.extend(es);
                }
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }

    fn unify_params_t(
        &self,
        sig: &ast::SubrSignature,
        registered_t: &SubrType,
        params: &hir::Params,
        body_t: &Type,
        body_loc: &impl Locational,
    ) -> TyCheckResult<()> {
        let name = &sig.ident.name;
        let mut errs = TyCheckErrors::empty();
        for (param, pt) in params
            .non_defaults
            .iter()
            .zip(registered_t.non_default_params.iter())
        {
            pt.typ().lower();
            if let Err(es) = self.force_sub_unify(&param.vi.t, pt.typ(), param, None) {
                errs.extend(es);
            }
            pt.typ().lift();
        }
        // TODO: var_params: [Int; _], pt: Int
        /*if let Some((var_params, pt)) = params.var_params.as_deref().zip(registered_t.var_params.as_ref()) {
            pt.typ().lower();
            if let Err(es) = self.force_sub_unify(&var_params.vi.t, pt.typ(), var_params, None) {
                errs.extend(es);
            }
            pt.typ().lift();
        }*/
        for (param, pt) in params
            .defaults
            .iter()
            .zip(registered_t.default_params.iter())
        {
            pt.typ().lower();
            if let Err(es) = self.force_sub_unify(&param.sig.vi.t, pt.typ(), param, None) {
                errs.extend(es);
            }
            pt.typ().lift();
        }
        let spec_ret_t = registered_t.return_t.as_ref();
        // spec_ret_t.lower();
        let unify_return_result = if let Some(t_spec) = sig.return_t_spec.as_deref() {
            self.force_sub_unify(body_t, spec_ret_t, t_spec, None)
        } else {
            self.force_sub_unify(body_t, spec_ret_t, body_loc, None)
        };
        // spec_ret_t.lift();
        if let Err(unify_errs) = unify_return_result {
            let es = TyCheckErrors::new(
                unify_errs
                    .into_iter()
                    .map(|e| {
                        let expect = if cfg!(feature = "debug") {
                            spec_ret_t.clone()
                        } else {
                            self.readable_type(spec_ret_t.clone())
                        };
                        let found = if cfg!(feature = "debug") {
                            body_t.clone()
                        } else {
                            self.readable_type(body_t.clone())
                        };
                        TyCheckError::return_type_error(
                            self.cfg.input.clone(),
                            line!() as usize,
                            e.core.get_loc_with_fallback(),
                            e.caused_by,
                            readable_name(name.inspect()),
                            &expect,
                            &found,
                            // e.core.get_hint().map(|s| s.to_string()),
                        )
                    })
                    .collect(),
            );
            errs.extend(es);
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }

    /// ## Errors
    /// * TypeError: if `return_t` != typeof `body`
    /// * AssignError: if `name` has already been registered
    pub(crate) fn assign_subr(
        &mut self,
        sig: &ast::SubrSignature,
        id: DefId,
        params: &hir::Params,
        body_t: &Type,
        body_loc: &impl Locational,
    ) -> Result<VarInfo, (TyCheckErrors, VarInfo)> {
        let mut errs = TyCheckErrors::empty();
        // already defined as const
        if sig.ident.is_const() {
            // For const subroutines (ValueObj::Subr), already registered in locals
            if let Some(vi) = self.locals.get(sig.ident.inspect()).cloned() {
                return Ok(vi);
            }
            // For other const values, move from decls to locals
            let vi = self.decls.remove(sig.ident.inspect()).unwrap();
            self.locals.insert(sig.ident.name.clone(), vi.clone());
            return Ok(vi);
        }
        let vis = match self.instantiate_vis_modifier(&sig.ident.vis) {
            Ok(vis) => vis,
            Err(es) => {
                errs.extend(es);
                VisibilityModifier::Private
            }
        };
        let muty = if sig.ident.is_const() {
            Mutability::Const
        } else {
            Mutability::Immutable
        };
        let name = &sig.ident.name;
        // FIXME: constでない関数
        let Some(subr_t) = self.get_current_scope_var(name).map(|vi| &vi.t) else {
            let err = unreachable_error!(TyCheckErrors, TyCheckError, self);
            return err.map_err(|e| (e, VarInfo::ILLEGAL));
        };
        let Ok(subr_t) = <&SubrType>::try_from(subr_t) else {
            let err = unreachable_error!(TyCheckErrors, TyCheckError, self);
            return err.map_err(|e| (e, VarInfo::ILLEGAL));
        };
        if let Err(es) = self.unify_params_t(sig, subr_t, params, body_t, body_loc) {
            errs.extend(es);
        }
        // NOTE: not `body_t.clone()` because the body may contain `return`
        let return_t = subr_t.return_t.as_ref().clone();
        let sub_t = if sig.ident.is_procedural() {
            proc(
                subr_t.non_default_params.clone(),
                subr_t.var_params.as_deref().cloned(),
                subr_t.default_params.clone(),
                subr_t.kw_var_params.as_deref().cloned(),
                return_t,
            )
        } else {
            func(
                subr_t.non_default_params.clone(),
                subr_t.var_params.as_deref().cloned(),
                subr_t.default_params.clone(),
                subr_t.kw_var_params.as_deref().cloned(),
                return_t,
            )
        };
        sub_t.lift();
        let found_t = self.generalize_t(sub_t);
        // let found_t = self.eliminate_needless_quant(found_t, crate::context::Variance::Covariant, sig)?;
        let py_name = if let Some(vi) = self.decls.remove(name) {
            if !self.supertype_of(&vi.t, &found_t) {
                let err = TyCheckError::violate_decl_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    sig.ident.loc(),
                    self.caused_by(),
                    name.inspect(),
                    &vi.t,
                    &found_t,
                );
                errs.push(err);
            }
            vi.py_name
        } else {
            None
        };
        let comptime_decos = sig
            .decorators
            .iter()
            .filter_map(|deco| match &deco.0 {
                ast::Expr::Accessor(ast::Accessor::Ident(local)) if local.is_const() => {
                    Some(local.inspect().clone())
                }
                _ => None,
            })
            .collect();
        let vi = VarInfo::new(
            found_t,
            muty,
            Visibility::new(vis, self.name.clone()),
            VarKind::Defined(id),
            Some(comptime_decos),
            self.kind.clone(),
            py_name,
            self.absolutize(name.loc()),
        );
        let vis = if vi.vis.is_private() { "::" } else { "." };
        log!(info "Registered {}{}{name}: {}", self.name, vis, &vi.t);
        self.locals.insert(name.clone(), vi.clone());
        if errs.is_empty() {
            Ok(vi)
        } else {
            Err((errs, vi))
        }
    }

    pub(crate) fn fake_subr_assign(
        &mut self,
        ident: &Identifier,
        decorators: &Set<Decorator>,
        failure_t: Type,
    ) -> TyCheckResult<()> {
        // already defined as const
        if ident.is_const() {
            if let Some(vi) = self.decls.remove(ident.inspect()) {
                // a const declaration was found: preserve it as-is instead of
                // overwriting with a `DoesNotExist`/`failure_t` entry below
                self.locals.insert(ident.name.clone(), vi);
                return Ok(());
            } else {
                log!(err "not found: {}", ident.name);
                return Ok(());
            }
        }
        let vis = self.instantiate_vis_modifier(&ident.vis)?;
        let muty = if ident.is_const() {
            Mutability::Const
        } else {
            Mutability::Immutable
        };
        let name = &ident.name;
        self.decls.remove(name);
        let comptime_decos = decorators
            .iter()
            .filter_map(|deco| match &deco.0 {
                ast::Expr::Accessor(ast::Accessor::Ident(local)) if local.is_const() => {
                    Some(local.inspect().clone())
                }
                _ => None,
            })
            .collect();
        let vi = VarInfo::new(
            failure_t,
            muty,
            Visibility::new(vis, self.name.clone()),
            VarKind::DoesNotExist,
            Some(comptime_decos),
            self.kind.clone(),
            None,
            self.absolutize(name.loc()),
        );
        log!(info "Registered {}::{name}: {}", self.name, &vi.t);
        self.locals.insert(name.clone(), vi);
        Ok(())
    }

    /// Instantiate the class specification of a methods block (e.g. `C` of `C.` / `C(T)` of `C(T).`).
    /// For a polymorphic class spec, undefined type variables (e.g. `T` of `Wrapper(T).`)
    /// are registered into `tv_cache` as fresh type variables, so that the caller can
    /// bind them in the methods context.
    pub(crate) fn get_class_and_impl_trait<'c>(
        &mut self,
        class_spec: &'c ast::TypeSpec,
        tv_cache: &mut TyVarCache,
    ) -> Result<ClassTrait<'c>, ClassTraitErrors<'c>> {
        let mut errs = TyCheckErrors::empty();
        // e.g. `Wrapper(T).`: `T` is not defined at this point,
        // so it must be instantiated as a fresh type variable
        let is_poly_spec = |spec: &ast::TypeSpec| {
            matches!(
                spec,
                ast::TypeSpec::PreDeclTy(ast::PreDeclTypeSpec::Poly(_))
            )
        };
        match class_spec {
            ast::TypeSpec::TypeApp { spec, args } => {
                let not_found_is_qvar = is_poly_spec(spec);
                match &args.args {
                    ast::TypeAppArgsKind::Args(args) => {
                        let (impl_trait, t_spec) = match &args.pos_args().first().unwrap().expr {
                            // TODO: check `tasc.op`
                            ast::Expr::TypeAscription(tasc) => {
                                let t = match self.instantiate_typespec_full(
                                    &tasc.t_spec.t_spec,
                                    None,
                                    tv_cache,
                                    RegistrationMode::Normal,
                                    false,
                                ) {
                                    Ok(t) => t,
                                    Err((t, es)) => {
                                        errs.extend(es);
                                        t
                                    }
                                };
                                (t, &tasc.t_spec)
                            }
                            other => {
                                return Err((
                                    None,
                                    None,
                                    TyCheckErrors::from(TyCheckError::syntax_error(
                                        self.cfg.input.clone(),
                                        line!() as usize,
                                        other.loc(),
                                        self.caused_by(),
                                        format!(
                                            "expected type ascription, but found {}",
                                            other.name()
                                        ),
                                        None,
                                    )),
                                ))
                            }
                        };
                        let class = match self.instantiate_typespec_full(
                            spec,
                            None,
                            tv_cache,
                            RegistrationMode::Normal,
                            not_found_is_qvar,
                        ) {
                            Ok(t) => t,
                            Err((t, es)) => {
                                errs.extend(es);
                                t
                            }
                        };
                        if errs.is_empty() {
                            Ok((class, Some((impl_trait, t_spec))))
                        } else {
                            Err((Some(class), Some((impl_trait, t_spec)), errs))
                        }
                    }
                    ast::TypeAppArgsKind::SubtypeOf(trait_spec) => {
                        // The class spec must be instantiated first so that its type
                        // variables (e.g. `T` of `C(T)|<: Eq C(T)|.`) are available
                        // in the trait spec
                        let class = match self.instantiate_typespec_full(
                            spec,
                            None,
                            tv_cache,
                            RegistrationMode::Normal,
                            not_found_is_qvar,
                        ) {
                            Ok(t) => t,
                            Err((t, es)) => {
                                errs.extend(es);
                                t
                            }
                        };
                        let impl_trait = match self.instantiate_typespec_full(
                            &trait_spec.t_spec,
                            None,
                            tv_cache,
                            RegistrationMode::Normal,
                            false,
                        ) {
                            Ok(t) => t,
                            Err((t, es)) => {
                                if !PYTHON_MODE {
                                    errs.extend(es);
                                }
                                t.replace(&Type::Failure, &Type::Never)
                            }
                        };
                        if errs.is_empty() {
                            Ok((class, Some((impl_trait, trait_spec.as_ref()))))
                        } else {
                            Err((Some(class), Some((impl_trait, trait_spec.as_ref())), errs))
                        }
                    }
                }
            }
            other => {
                // e.g. `Wrapper.` where `Wrapper` is a polymorphic class:
                // instantiate the type parameters as fresh type variables
                // named after the class's own parameter names
                if let ast::TypeSpec::PreDeclTy(ast::PreDeclTypeSpec::Mono(ident)) = other {
                    if let Some(class) = self.instantiate_poly_class_as_mono_spec(ident, tv_cache) {
                        return Ok((class, None));
                    }
                }
                let t = match self.instantiate_typespec_full(
                    other,
                    None,
                    tv_cache,
                    RegistrationMode::Normal,
                    is_poly_spec(other),
                ) {
                    Ok(t) => t,
                    Err((t, es)) => {
                        errs.extend(es);
                        t
                    }
                };
                if errs.is_empty() {
                    Ok((t, None))
                } else {
                    Err((Some(t), None, errs))
                }
            }
        }
    }

    /// `Wrapper.` (where `Wrapper|T| = Class ...`) => `Some(Wrapper(?T))`
    /// with `T ↦ ?T` registered into `tv_cache`.
    /// Returns `None` if `ident` does not name a polymorphic type.
    fn instantiate_poly_class_as_mono_spec(
        &self,
        ident: &Identifier,
        tv_cache: &mut TyVarCache,
    ) -> Option<Type> {
        let type_ctx = self.get_type_ctx(ident.inspect())?;
        if type_ctx.typ.is_monomorphic() {
            return None;
        }
        let qual_name = type_ctx.typ.qual_name();
        let params = type_ctx
            .ctx
            .params
            .iter()
            .map(|(name, vi)| {
                let param_name = name
                    .as_ref()
                    .map_or(Str::ever("_"), |name| name.inspect().clone());
                let varname = VarName::from_str(param_name.clone());
                if vi.t == Type::Type {
                    let tv =
                        named_free_var(param_name, self.level, Constraint::new_type_of(Type::Type));
                    let _ = tv_cache.push_or_init_tyvar(&varname, &tv, self);
                    TyParam::t(tv)
                } else {
                    let tp = TyParam::named_free_var(
                        param_name,
                        self.level,
                        Constraint::new_type_of(vi.t.clone()),
                    );
                    let _ = tv_cache.push_or_init_typaram(&varname, &tp, self);
                    tp
                }
            })
            .collect::<Vec<_>>();
        Some(poly(qual_name, params))
    }

    pub(crate) fn register_trait_impl(
        &mut self,
        class: &Type,
        trait_: &Type,
        trait_loc: &impl Locational,
    ) -> TyCheckResult<()> {
        // TODO: polymorphic trait
        let declared_in = NormalizedPathBuf::from(self.module_path());
        let declared_in = declared_in.exists().then_some(declared_in);
        if let Some(mut impls) = self.trait_impls().get_mut(&trait_.qual_name()) {
            impls.insert(TraitImpl::new(class.clone(), trait_.clone(), declared_in));
        } else {
            self.trait_impls().register(
                trait_.qual_name(),
                set! {TraitImpl::new(class.clone(), trait_.clone(), declared_in)},
            );
        }
        let trait_ctx = if let Some(trait_ctx) = self.get_nominal_type_ctx(trait_) {
            trait_ctx.clone()
        } else {
            // TODO: maybe parameters are wrong
            return Err(TyCheckErrors::from(TyCheckError::no_var_error(
                self.cfg.input.clone(),
                line!() as usize,
                trait_loc.loc(),
                self.caused_by(),
                &trait_.local_name(),
                None,
            )));
        };
        let Some(class_ctx) = self.get_mut_nominal_type_ctx(class) else {
            return Err(TyCheckErrors::from(TyCheckError::type_not_found(
                self.cfg.input.clone(),
                line!() as usize,
                trait_loc.loc(),
                self.caused_by(),
                class,
            )));
        };
        class_ctx.register_supertrait(trait_.clone(), &trait_ctx);
        Ok(())
    }

    /// Registers type definitions of types and constants; unlike `register_const`, this does not evaluate terms.
    pub(crate) fn preregister_consts(&mut self, block: &ast::Block) -> TyCheckResult<()> {
        let mut total_errs = TyCheckErrors::empty();
        for expr in block.iter() {
            match expr {
                ast::Expr::Def(def) => {
                    if let Err(errs) = self.preregister_const_def(def) {
                        total_errs.extend(errs);
                    }
                }
                ast::Expr::ClassDef(class_def) => {
                    if let Err(errs) = self.preregister_const_def(&class_def.def) {
                        total_errs.extend(errs);
                    }
                }
                ast::Expr::PatchDef(patch_def) => {
                    if let Err(errs) = self.preregister_const_def(&patch_def.def) {
                        total_errs.extend(errs);
                    }
                }
                ast::Expr::Dummy(dummy) => {
                    if let Err(errs) = self.preregister_consts(&dummy.exprs) {
                        total_errs.extend(errs);
                    }
                }
                ast::Expr::Call(call) if PYTHON_MODE => {
                    if let Err(errs) = self.preregister_control_consts(call) {
                        total_errs.extend(errs);
                    }
                }
                _ => {}
            }
        }
        if total_errs.is_empty() {
            Ok(())
        } else {
            Err(total_errs)
        }
    }

    fn preregister_control_consts(&mut self, call: &ast::Call) -> TyCheckResult<()> {
        match call
            .obj
            .get_name()
            .and_then(|s| ControlKind::try_from(&s[..]).ok())
        {
            Some(ControlKind::If) => {
                let Some(ast::Expr::Lambda(then)) = call.args.nth_or_key(1, "then") else {
                    return Ok(());
                };
                self.preregister_consts(&then.body)?;
                if let Some(ast::Expr::Lambda(else_)) = call.args.nth_or_key(2, "else") {
                    self.preregister_consts(&else_.body)?;
                }
            }
            Some(ControlKind::For) => {
                let Some(ast::Expr::Lambda(body)) = call.args.nth_or_key(1, "body") else {
                    return Ok(());
                };
                self.preregister_consts(&body.body)?;
            }
            Some(ControlKind::While) => {
                let Some(ast::Expr::Lambda(body)) = call.args.nth_or_key(1, "body") else {
                    return Ok(());
                };
                self.preregister_consts(&body.body)?;
            }
            Some(ControlKind::With) => {
                let Some(ast::Expr::Lambda(body)) = call.args.nth_or_key(1, "body") else {
                    return Ok(());
                };
                self.preregister_consts(&body.body)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn register_defs(&mut self, block: &ast::Block) -> TyCheckResult<()> {
        let mut total_errs = TyCheckErrors::empty();
        for expr in block.iter() {
            match expr {
                ast::Expr::Def(def) => {
                    if let Err(errs) = self.register_def(def) {
                        total_errs.extend(errs);
                    }
                    if def.def_kind().is_import() {
                        if let Err(errs) = self.pre_import(def) {
                            total_errs.extend(errs);
                        }
                    }
                }
                ast::Expr::ClassDef(class_def) => {
                    if let Err(errs) = self.register_def(&class_def.def) {
                        total_errs.extend(errs);
                    }
                    let vis = self
                        .instantiate_vis_modifier(class_def.def.sig.vis())
                        .unwrap_or(VisibilityModifier::Public);
                    for methods in class_def.methods_list.iter() {
                        let mut tv_cache = TyVarCache::new(self.level, self);
                        let (class, impl_trait) =
                            match self.get_class_and_impl_trait(&methods.class, &mut tv_cache) {
                                Ok(x) => x,
                                Err((class, trait_, errs)) => {
                                    total_errs.extend(errs);
                                    (class.unwrap_or(Type::Obj), trait_)
                                }
                            };
                        // assume the class has implemented the trait, regardless of whether the implementation is correct
                        if let Some((trait_, trait_loc)) = &impl_trait {
                            if let Err(errs) = self.register_trait_impl(&class, trait_, *trait_loc)
                            {
                                total_errs.extend(errs);
                            }
                        }
                        let kind = ContextKind::MethodDefs {
                            class: (!class.is_monomorphic()).then(|| class.clone()),
                            impl_trait: impl_trait.as_ref().map(|(t, _)| t.clone()),
                        };
                        // The type variables of the class spec (e.g. `T` of `Wrapper(T).`)
                        // are created at the outer level; raise them below the methods
                        // context's level (== self.level + 1) so that `generalize_t`
                        // (which only generalizes variables deeper than the registering
                        // context) quantifies them into each method's type.
                        // NOTE: `self.level + 2` (not `+ 1`) because a variable appearing
                        // only in the `self` parameter's bound (e.g. `U` of
                        // `Pair(T, U). fst(self): T = ...`) is generalized via
                        // `generalize_constraint` without being lifted beforehand
                        for tv in tv_cache.tyvar_instances.values() {
                            tv.set_level(self.level.saturating_add(2));
                        }
                        for tp in tv_cache.typaram_instances.values() {
                            tp.set_level(self.level.saturating_add(2));
                        }
                        let tv_cache = (!tv_cache.is_empty()).then_some(tv_cache);
                        self.grow(&class.local_name(), kind, vis.clone(), tv_cache);
                        for attr in methods.attrs.iter() {
                            match attr {
                                ClassAttr::Def(def) => {
                                    if let Err(errs) = self.register_def(def) {
                                        total_errs.extend(errs);
                                    }
                                }
                                ClassAttr::Decl(decl) => {
                                    if let Some(ident) = decl.expr.as_ident() {
                                        if let Err((_, errs)) =
                                            self.declare_var(ident, &decl.t_spec)
                                        {
                                            total_errs.extend(errs);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        let ctx = self.pop();
                        let Some(class_root) = self.get_mut_nominal_type_ctx(&class) else {
                            log!(err "class not found: {class}");
                            continue;
                        };
                        let typ = if let Some((impl_trait, _)) = impl_trait {
                            ClassDefType::impl_trait(class, impl_trait)
                        } else {
                            ClassDefType::Simple(class)
                        };
                        class_root
                            .methods_list
                            .push(MethodContext::new(methods.id, typ, ctx));
                    }
                }
                ast::Expr::PatchDef(patch_def) => {
                    if let Err(errs) = self.register_def(&patch_def.def) {
                        total_errs.extend(errs);
                    }
                }
                ast::Expr::Dummy(dummy) => {
                    if let Err(errs) = self.register_defs(&dummy.exprs) {
                        total_errs.extend(errs);
                    }
                }
                ast::Expr::TypeAscription(tasc) => {
                    if !self.kind.is_module() {
                        continue;
                    }
                    if let Some(ident) = tasc.expr.as_ident() {
                        if let Err((_, errs)) = self.declare_var(ident, &tasc.t_spec) {
                            total_errs.extend(errs);
                        }
                    }
                }
                ast::Expr::Call(call) if PYTHON_MODE => {
                    if let Err(errs) = self.register_control_defs(call) {
                        total_errs.extend(errs);
                    }
                }
                _ => {}
            }
        }
        if total_errs.is_empty() {
            Ok(())
        } else {
            Err(total_errs)
        }
    }

    fn register_control_defs(&mut self, call: &ast::Call) -> TyCheckResult<()> {
        match call
            .obj
            .get_name()
            .and_then(|s| ControlKind::try_from(&s[..]).ok())
        {
            Some(ControlKind::If) => {
                let Some(ast::Expr::Lambda(then)) = call.args.nth_or_key(1, "then") else {
                    return Ok(());
                };
                self.register_defs(&then.body)?;
                if let Some(ast::Expr::Lambda(else_)) = call.args.nth_or_key(2, "else") {
                    self.register_defs(&else_.body)?;
                }
            }
            Some(ControlKind::For) => {
                let Some(ast::Expr::Lambda(body)) = call.args.nth_or_key(1, "body") else {
                    return Ok(());
                };
                self.register_defs(&body.body)?;
            }
            Some(ControlKind::While) => {
                let Some(ast::Expr::Lambda(body)) = call.args.nth_or_key(1, "body") else {
                    return Ok(());
                };
                self.register_defs(&body.body)?;
            }
            Some(ControlKind::With) => {
                let Some(ast::Expr::Lambda(body)) = call.args.nth_or_key(1, "body") else {
                    return Ok(());
                };
                self.register_defs(&body.body)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// HACK: The constant expression evaluator can evaluate attributes when the type of the receiver is known.
    /// import/pyimport is not a constant function, but specially assumes that the type of the module is known in the eval phase.
    fn pre_import(&mut self, def: &ast::Def) -> TyCheckResult<()> {
        let Some(ast::Expr::Call(call)) = def.body.block.first() else {
            unreachable!()
        };
        let Some(ast::Expr::Literal(mod_name)) = call.args.get_left_or_key("Path") else {
            return Ok(());
        };
        let Ok(mod_name) = hir::Literal::try_from(mod_name.token.clone()) else {
            return Ok(());
        };
        let path = self.import_mod(call.additional_operation().unwrap(), &mod_name);
        let arg = if let Ok(path) = &path {
            TyParam::Value(ValueObj::Str(path.to_string_lossy().into()))
        } else {
            TyParam::Value(ValueObj::Str(
                mod_name.token.content.replace('\"', "").into(),
            ))
        };
        let res = path.map(|_path| ());
        let typ = if def.def_kind().is_erg_import() {
            module(arg)
        } else {
            py_module(arg)
        };
        let Some(ident) = def.sig.ident() else {
            return res;
        };
        let Some((_, vi)) = self.get_var_info(ident.inspect()) else {
            return res;
        };
        if let Some(_fv) = vi.t.as_free() {
            vi.t.destructive_link(&typ);
        }
        res
    }

    fn preregister_const_def(&mut self, def: &ast::Def) -> TyCheckResult<()> {
        match &def.sig {
            ast::Signature::Var(var) if var.is_const() => {
                let Some(ast::Expr::Call(call)) = def.body.block.first() else {
                    return Ok(());
                };
                self.preregister_type(var, call)
            }
            _ => Ok(()),
        }
    }

    fn preregister_type(&mut self, var: &ast::VarSignature, call: &ast::Call) -> TyCheckResult<()> {
        match call.obj.as_ref() {
            ast::Expr::Accessor(ast::Accessor::Ident(ident)) => match &ident.inspect()[..] {
                "Class" => {
                    let ident = var.ident().unwrap();
                    // Skip preregistration for polymorphic classes - they will be registered in register_def
                    if !var.bounds.is_empty() {
                        return Ok(());
                    }
                    let t = Type::Mono(format!("{}{ident}", self.name).into());
                    let class = GenTypeObj::class(t, None, None, false);
                    let class = ValueObj::Type(TypeObj::Generated(class));
                    self.register_gen_const(ident, class, Some(call), false)
                }
                "Trait" => {
                    let ident = var.ident().unwrap();
                    // Skip preregistration for polymorphic traits - they will be registered in register_def
                    if !var.bounds.is_empty() {
                        return Ok(());
                    }
                    let t = Type::Mono(format!("{}{ident}", self.name).into());
                    let trait_ =
                        GenTypeObj::trait_(t, TypeObj::builtin_type(Type::Failure), None, false);
                    let trait_ = ValueObj::Type(TypeObj::Generated(trait_));
                    self.register_gen_const(ident, trait_, Some(call), false)
                }
                // `T = Structural Trait {...}`: pre-register the inner trait so that `Self`
                // can be resolved while instantiating the requirement record.
                // `Structural` applied to anything else is a mere type alias, so it is skipped.
                "Structural" => {
                    let Some(ast::Expr::Call(inner)) = call.args.get_left_or_key("Type") else {
                        return Ok(());
                    };
                    if inner.obj.get_name().map(|n| &n[..]) != Some("Trait") {
                        return Ok(());
                    }
                    let ident = var.ident().unwrap();
                    // Skip preregistration for polymorphic traits - they will be registered in register_def
                    if !var.bounds.is_empty() {
                        return Ok(());
                    }
                    let t = Type::Mono(format!("{}{ident}", self.name).into());
                    let trait_ =
                        GenTypeObj::trait_(t, TypeObj::builtin_type(Type::Failure), None, false);
                    let structural = GenTypeObj::structural(
                        trait_.typ().clone().structuralize(),
                        TypeObj::Generated(trait_),
                    );
                    let structural = ValueObj::Type(TypeObj::Generated(structural));
                    self.register_gen_const(ident, structural, Some(call), false)
                }
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    /// Detects cyclic class/trait definitions such as `C = Class D; D = Class C`.
    /// Since forward references are only possible for pre-registered types (classes/traits),
    /// it is sufficient to check the requirement (base) chains of the generated types.
    /// Recursion through container types (e.g. `Node = Class { next = Node or NoneType }`)
    /// is not flagged.
    pub(crate) fn check_cyclic_definitions(&self) -> TyCheckErrors {
        let mut errs = TyCheckErrors::empty();
        // node: qual_name of the type, edge: qual_name of its requirement (base) type.
        // Each node has at most one outgoing edge, so cycles can be found
        // by simply following the chains.
        let mut req_edges = Dict::new();
        let mut nodes = vec![];
        for (name, val) in self.consts.iter() {
            let ValueObj::Type(TypeObj::Generated(gen)) = val else {
                continue;
            };
            if !matches!(gen, GenTypeObj::Class(_) | GenTypeObj::Trait(_)) {
                continue;
            }
            let Some(base) = gen.base_or_sup() else {
                continue;
            };
            req_edges.insert(gen.typ().qual_name(), base.typ().qual_name());
            nodes.push((name, gen.typ().qual_name()));
        }
        let local = |qn: &Str| qn.rsplit(&[':', '.'][..]).next().unwrap_or(qn).to_string();
        for (name, start) in nodes.iter() {
            let mut cur = start;
            let mut path = vec![local(start)];
            // The walk is bounded to guard against chains that fall into
            // a cycle not containing `start`
            for _ in 0..=req_edges.len() {
                let Some(next) = req_edges.get(cur) else {
                    break;
                };
                path.push(local(next));
                if next == start {
                    errs.push(TyCheckError::cyclic_definition_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        name.loc(),
                        self.caused_by(),
                        &path.join(" -> "),
                    ));
                    break;
                }
                cur = next;
            }
        }
        errs
    }

    pub(crate) fn register_def(&mut self, def: &ast::Def) -> TyCheckResult<()> {
        let id = Some(def.body.id);
        let __name__ = def.sig.ident().map(|i| i.inspect()).unwrap_or(UBAR);
        let call = if let Some(ast::Expr::Call(call)) = &def.body.block.first() {
            Some(call)
        } else {
            None
        };
        let mut errs = TyCheckErrors::empty();
        match &def.sig {
            ast::Signature::Subr(sig) => {
                if sig.is_const() {
                    // `MyTr T = Trait ...` would silently degrade (`T` decays to `Never`),
                    // so reject it explicitly. Use the (supported) `MyTr|T| = Trait ...`
                    // form instead.
                    if !sig.params.is_empty() && def.def_kind().is_trait() {
                        let name = sig.ident.inspect();
                        return feature_error!(
                            TyCheckErrors,
                            TyCheckError,
                            self,
                            sig.loc(),
                            &format!(
                                "polymorphic trait definition with parameter syntax (use `{name}|...| = Trait ...` instead)"
                            )
                        );
                    }
                    // A const subroutine is a subroutine whatever its arity:
                    // `F() = 1` defines a function, which a call folds -- not
                    // the constant 1. Evaluating the body here (as this used to
                    // for a parameterless one) put the type of a value on a
                    // name that codegen binds to a function, and the program
                    // failed at run time after passing the checker.
                    //
                    // In Python a parameterless function is a function whatever
                    // its name, and its body is not for evaluating here.
                    if PYTHON_MODE && sig.params.is_empty() {
                        if let Err(es) = self.declare_sub(sig, id) {
                            errs.extend(es);
                        }
                    } else {
                        let obj =
                            match self.register_const_subr(sig, &def.body.block, def.def_kind()) {
                                Ok(obj) => obj,
                                Err((obj, es)) => {
                                    errs.extend(es);
                                    obj
                                }
                            };
                        if let Err(es) = self.register_gen_const(
                            def.sig.ident().unwrap(),
                            obj,
                            call,
                            def.def_kind().is_other(),
                        ) {
                            errs.extend(es);
                        }
                    }
                } else if let Err(es) = self.declare_sub(sig, id) {
                    errs.extend(es);
                }
            }
            ast::Signature::Var(sig) => {
                if sig.is_const() {
                    let kind = ContextKind::from(def);
                    let vis = self.instantiate_vis_modifier(sig.vis())?;
                    // Instantiate type bounds for polymorphic class/trait definitions
                    let tv_cache = if sig.bounds.is_empty() {
                        None
                    } else {
                        match self.instantiate_ty_bounds(&sig.bounds, PreRegister) {
                            Ok(tv_cache) => Some(tv_cache),
                            Err((tv_cache, es)) => {
                                errs.extend(es);
                                Some(tv_cache)
                            }
                        }
                    };
                    // before `grow`, after which `self` is the definition's own
                    // scope rather than the one it is being defined in
                    let per_call = self.kind.is_subr() && self.has_const_param();
                    self.grow(__name__, kind, vis, tv_cache);
                    let (obj, const_t) = match self.eval_const_block(&def.body.block) {
                        Ok(obj) => (obj.clone(), v_enum(set! {obj})),
                        Err((obj, es)) => {
                            if PYTHON_MODE {
                                self.pop();
                                if let Err((_, es)) = self.pre_define_var(sig, id) {
                                    errs.extend(es);
                                }
                                if let Some(ident) = sig.ident() {
                                    let _ = self.register_gen_const(
                                        ident,
                                        obj,
                                        call,
                                        def.def_kind().is_other(),
                                    );
                                }
                                if errs.is_empty() {
                                    return Ok(());
                                } else {
                                    return Err(errs);
                                }
                            }
                            // A constant in a const subroutine's body may be built
                            // from the parameters, whose values belong to a call and
                            // not to this definition. Leave it to the ordinary
                            // lowering, which types the binding; a body that is wrong
                            // for some other reason is reported there rather than
                            // twice, and the call reports one that cannot be folded.
                            if per_call {
                                self.pop();
                                if let Err((_, es)) = self.pre_define_var(sig, id) {
                                    errs.extend(es);
                                }
                                return if errs.is_empty() { Ok(()) } else { Err(errs) };
                            }
                            errs.extend(es);
                            (obj.clone(), v_enum(set! {obj}))
                        }
                    };
                    if let Some(spec) = sig.t_spec.as_ref() {
                        let mut dummy_tv_cache = TyVarCache::new(self.level, self);
                        let spec_t = match self.instantiate_typespec_full(
                            &spec.t_spec,
                            None,
                            &mut dummy_tv_cache,
                            PreRegister,
                            false,
                        ) {
                            Ok(ty) => ty,
                            Err((ty, es)) => {
                                errs.extend(es);
                                ty
                            }
                        };
                        if let Err(es) = self.sub_unify(&const_t, &spec_t, &def.body, None) {
                            errs.extend(es);
                        }
                    }
                    self.pop();
                    if let Some(ident) = sig.ident() {
                        if let Err(es) =
                            self.register_gen_const(ident, obj, call, def.def_kind().is_other())
                        {
                            errs.extend(es);
                        }
                    }
                } else if let Err((_, es)) = self.pre_define_var(sig, id) {
                    errs.extend(es);
                }
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }

    /// e.g. .new
    fn register_auto_impl(
        &mut self,
        name: &'static str,
        t: Type,
        muty: Mutability,
        vis: Visibility,
        py_name: Option<Str>,
    ) -> CompileResult<()> {
        let name = VarName::from_static(name);
        if self.locals.get(&name).is_some() {
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                name.loc(),
                self.caused_by(),
                name.inspect(),
            )))
        } else {
            let vi = VarInfo::new(
                t,
                muty,
                vis,
                VarKind::Auto,
                None,
                self.kind.clone(),
                py_name,
                AbsLocation::unknown(),
            );
            self.locals.insert(name, vi);
            Ok(())
        }
    }

    /// e.g. `::__call__`
    ///
    /// NOTE: this is same as `register_auto_impl` if `PYTHON_MODE` is true
    fn register_fixed_auto_impl(
        &mut self,
        name: &'static str,
        t: Type,
        muty: Mutability,
        vis: Visibility,
        py_name: Option<Str>,
    ) -> CompileResult<()> {
        let name = VarName::from_static(name);
        let kind = if PYTHON_MODE {
            VarKind::Auto
        } else {
            VarKind::FixedAuto
        };
        if self.locals.get(&name).is_some() {
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                name.loc(),
                self.caused_by(),
                name.inspect(),
            )))
        } else {
            self.locals.insert(
                name,
                VarInfo::new(
                    t,
                    muty,
                    vis,
                    kind,
                    None,
                    self.kind.clone(),
                    py_name,
                    AbsLocation::unknown(),
                ),
            );
            Ok(())
        }
    }

    fn _register_gen_decl(
        &mut self,
        name: VarName,
        t: Type,
        vis: Visibility,
        kind: ContextKind,
        py_name: Option<Str>,
    ) -> CompileResult<()> {
        if self.decls.get(&name).is_some() {
            Err(CompileErrors::from(CompileError::duplicate_decl_error(
                self.cfg.input.clone(),
                line!() as usize,
                name.loc(),
                self.caused_by(),
                name.inspect(),
            )))
        } else {
            let vi = VarInfo::new(
                t,
                Immutable,
                vis,
                VarKind::Declared,
                None,
                kind,
                py_name,
                self.absolutize(name.loc()),
            );
            self.decls.insert(name, vi);
            Ok(())
        }
    }

    fn _register_gen_impl(
        &mut self,
        name: VarName,
        t: Type,
        muty: Mutability,
        vis: Visibility,
        kind: ContextKind,
        py_name: Option<Str>,
    ) -> CompileResult<()> {
        if self.locals.get(&name).is_some() {
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                name.loc(),
                self.caused_by(),
                name.inspect(),
            )))
        } else {
            let id = DefId(get_hash(&(&self.name, &name)));
            let vi = VarInfo::new(
                t,
                muty,
                vis,
                VarKind::Defined(id),
                None,
                kind,
                py_name,
                self.absolutize(name.loc()),
            );
            self.locals.insert(name, vi);
            Ok(())
        }
    }

    /// If the trait has super-traits, you should call `register_trait` after calling this method.
    pub(crate) fn register_trait_methods(&mut self, class: Type, methods: Self) {
        let trait_ = if let ContextKind::MethodDefs {
            impl_trait: Some(tr),
            ..
        } = &methods.kind
        {
            tr.clone()
        } else {
            unreachable!()
        };
        self.super_traits.push(trait_.clone());
        self.methods_list.push(MethodContext::new(
            DefId(0),
            ClassDefType::impl_trait(class, trait_),
            methods,
        ));
    }

    /// Register that a class implements a trait and its super-traits.
    /// This does not register the class-trait relationship to `shared.trait_impls` (use `register_trait_impl`).
    pub(crate) fn register_trait(&mut self, ctx: &Self, trait_: Type) -> CompileResult<()> {
        let trait_ctx = ctx.get_nominal_type_ctx(&trait_).ok_or_else(|| {
            CompileError::type_not_found(
                self.cfg.input.clone(),
                line!() as usize,
                ().loc(),
                self.caused_by(),
                &trait_,
            )
        })?;
        if trait_ctx.typ.has_qvar() {
            let _substituter = Substituter::substitute_typarams(ctx, &trait_ctx.typ, &trait_)?;
            self.super_traits.push(trait_);
            let mut tv_cache = TyVarCache::new(ctx.level, ctx);
            let traits = trait_ctx.super_classes.iter().cloned().map(|ty| {
                if ty.has_undoable_linked_var() {
                    ctx.detach(ty, &mut tv_cache)
                } else {
                    ty
                }
            });
            self.super_traits.extend(traits);
            let traits = trait_ctx.super_traits.iter().cloned().map(|ty| {
                if ty.has_undoable_linked_var() {
                    ctx.detach(ty, &mut tv_cache)
                } else {
                    ty
                }
            });
            self.super_traits.extend(traits);
        } else {
            self.super_traits.push(trait_);
            let traits = trait_ctx.super_classes.clone();
            self.super_traits.extend(traits);
            let traits = trait_ctx.super_traits.clone();
            self.super_traits.extend(traits);
        }
        unique_in_place(&mut self.super_traits);
        Ok(())
    }

    pub(crate) fn unregister_trait(&mut self, trait_: &Type) {
        self.super_traits.retain(|t| !t.structural_eq(trait_));
        // .retain(|t| !ctx.same_type_of(t, trait_));
    }

    pub(crate) fn register_base_class(&mut self, ctx: &Self, class: Type) -> CompileResult<()> {
        let class_ctx = ctx.get_nominal_type_ctx(&class).ok_or_else(|| {
            CompileError::type_not_found(
                self.cfg.input.clone(),
                line!() as usize,
                ().loc(),
                self.caused_by(),
                &class,
            )
        })?;
        if class_ctx.typ.has_qvar() {
            let _substituter = Substituter::substitute_typarams(ctx, &class_ctx.typ, &class)?;
            self.super_classes.push(class);
            let mut tv_cache = TyVarCache::new(ctx.level, ctx);
            let classes = class_ctx.super_classes.iter().cloned().map(|ty| {
                if ty.has_undoable_linked_var() {
                    ctx.detach(ty, &mut tv_cache)
                } else {
                    ty
                }
            });
            self.super_classes.extend(classes);
            let traits = class_ctx.super_traits.iter().cloned().map(|ty| {
                if ty.has_undoable_linked_var() {
                    ctx.detach(ty, &mut tv_cache)
                } else {
                    ty
                }
            });
            self.super_traits.extend(traits);
        } else {
            self.super_classes.push(class);
            let classes = class_ctx.super_classes.clone();
            self.super_classes.extend(classes);
            let traits = class_ctx.super_traits.clone();
            self.super_traits.extend(traits);
        }
        unique_in_place(&mut self.super_classes);
        Ok(())
    }

    /// Register a user-defined const subroutine (compile-time function with parameters).
    /// Instead of evaluating the body immediately, we create a UserConstSubr that stores
    /// the function definition and evaluates it when called.
    pub(crate) fn register_const_subr(
        &mut self,
        sig: &ast::SubrSignature,
        block: &ast::Block,
        _def_kind: ast::DefKind,
    ) -> Failable<ValueObj> {
        let mut errs = TyCheckErrors::empty();
        let mut tmp_tv_cache = match self.instantiate_ty_bounds(&sig.bounds, PreRegister) {
            Ok(tv_cache) => tv_cache,
            Err((tv_cache, es)) => {
                errs.extend(es);
                tv_cache
            }
        };
        let mut non_default_params = Vec::with_capacity(sig.params.non_defaults.len());
        for param in sig.params.non_defaults.iter() {
            match self.instantiate_param_ty(
                param,
                None,
                &mut tmp_tv_cache,
                PreRegister,
                ParamKind::NonDefault,
                false,
            ) {
                Ok(pt) => non_default_params.push(pt),
                Err((pt, err)) => {
                    non_default_params.push(pt);
                    errs.extend(err);
                }
            }
        }
        let var_params = if let Some(p) = sig.params.var_params.as_ref() {
            match self.instantiate_param_ty(
                p,
                None,
                &mut tmp_tv_cache,
                PreRegister,
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
        let mut default_params = Vec::with_capacity(sig.params.defaults.len());
        for param in sig.params.defaults.iter() {
            let default_t = match self.eval_const_expr(&param.default_val) {
                Ok(val) => val.t(),
                Err((val, es)) => {
                    errs.extend(es);
                    val.t()
                }
            };
            match self.instantiate_param_ty(
                &param.sig,
                None,
                &mut tmp_tv_cache,
                PreRegister,
                ParamKind::Default(default_t),
                false,
            ) {
                Ok(pt) => default_params.push(pt),
                Err((pt, err)) => {
                    errs.extend(err);
                    default_params.push(pt);
                }
            }
        }
        let kw_var_params = if let Some(p) = sig.params.kw_var_params.as_ref() {
            match self.instantiate_param_ty(
                p,
                None,
                &mut tmp_tv_cache,
                PreRegister,
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
        let return_t = if let Some(spec) = sig.return_t_spec.as_ref() {
            match self.instantiate_typespec_full(
                &spec.t_spec,
                None,
                &mut tmp_tv_cache,
                PreRegister,
                false,
            ) {
                Ok(ty) => ty,
                Err((ty, es)) => {
                    errs.extend(es);
                    ty
                }
            }
        } else {
            Type::Obj
        };
        let const_block = match Parser::validate_const_block(block.clone()) {
            Ok(block) => block,
            Err(_) => {
                errs.push(TyCheckError::feature_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    sig.loc(),
                    "const function body",
                    self.caused_by(),
                ));
                return Err((ValueObj::Failure, errs));
            }
        };
        let sig_t = subr_t(
            SubrKind::Func,
            non_default_params,
            var_params,
            default_params,
            kw_var_params,
            return_t,
        );
        let sig_t = self.generalize_t(sig_t);
        // Only a scope that exists once per module can identify a definition
        // for the const-call cache. A body scope (a function being evaluated,
        // a lambda) is re-entered with different surroundings, so a definition
        // there gets no identity and its calls are never cached.
        let def_scope = match self.kind {
            ContextKind::Module
            | ContextKind::Class
            | ContextKind::Trait
            | ContextKind::MethodDefs { .. }
            | ContextKind::PatchMethodDefs(_)
            | ContextKind::Patch(_) => Some((
                NormalizedPathBuf::from(self.module_path()),
                self.name.clone(),
            )),
            _ => None,
        };
        let user_subr = UserConstSubr::new(
            sig.ident.inspect().clone(),
            sig.params.clone(),
            const_block,
            sig_t,
            def_scope,
        );
        let subr = ValueObj::Subr(ConstSubr::User(user_subr));
        if errs.is_empty() {
            Ok(subr)
        } else {
            Err((subr, errs))
        }
    }

    pub(crate) fn register_gen_const(
        &mut self,
        ident: &Identifier,
        obj: ValueObj,
        call: Option<&ast::Call>,
        alias: bool,
    ) -> CompileResult<()> {
        let vis = self.instantiate_vis_modifier(&ident.vis)?;
        let inited = self
            .rec_get_const_obj(ident.inspect())
            .is_some_and(|v| v.is_inited());
        if inited && vis.is_private() {
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
                ident.inspect(),
            )))
        } else {
            match obj {
                ValueObj::Type(t) => match t {
                    TypeObj::Generated(gen) if alias => {
                        let meta_t = gen.meta_type();
                        self.register_type_alias(ident, gen.into_typ(), meta_t)
                    }
                    TypeObj::Generated(gen) => self.register_gen_type(ident, gen, call),
                    TypeObj::Builtin { t, meta_t } => self.register_type_alias(ident, t, meta_t),
                },
                ValueObj::Subr(ref subr) => {
                    let id = DefId(get_hash(ident));
                    let sig_t = subr.sig_t().clone();
                    let vi = VarInfo::new(
                        sig_t,
                        Const,
                        Visibility::new(vis, self.name.clone()),
                        VarKind::Defined(id),
                        None,
                        self.kind.clone(),
                        None,
                        self.absolutize(ident.name.loc()),
                    );
                    self.index().register(ident.inspect().clone(), &vi);
                    self.locals.insert(ident.name.clone(), vi);
                    self.consts.insert(ident.name.clone(), obj);
                    Ok(())
                }
                // TODO: not all value objects are comparable
                other => {
                    let id = DefId(get_hash(ident));
                    let vi = VarInfo::new(
                        v_enum(set! {other.clone()}),
                        Const,
                        Visibility::new(vis, self.name.clone()),
                        VarKind::Defined(id),
                        None,
                        self.kind.clone(),
                        None,
                        self.absolutize(ident.name.loc()),
                    );
                    self.index().register(ident.inspect().clone(), &vi);
                    self.decls.insert(ident.name.clone(), vi);
                    self.consts.insert(ident.name.clone(), other);
                    Ok(())
                }
            }
        }
    }

    pub(crate) fn register_gen_type(
        &mut self,
        ident: &Identifier,
        gen: GenTypeObj,
        call: Option<&ast::Call>,
    ) -> CompileResult<()> {
        match gen {
            GenTypeObj::Class(_) => {
                if gen.typ().is_monomorphic() {
                    // let super_traits = gen.impls.iter().map(|to| to.typ().clone()).collect();
                    let mut ctx = Self::mono_class(
                        gen.typ().qual_name(),
                        self.cfg.clone(),
                        self.shared.clone(),
                        2,
                        self.level,
                    );
                    let res = self.gen_class_new_method(&gen, call, &mut ctx);
                    let res2 = self.register_gen_mono_type(ident, gen, ctx, Const);
                    concat_result(res, res2)
                } else {
                    let params = gen
                        .typ()
                        .typarams()
                        .into_iter()
                        .map(|tp| {
                            let name = tp.qual_name().unwrap_or(Str::ever("_"));
                            ParamSpec::named_nd(name, self.get_tp_t(&tp).unwrap_or(Type::Obj))
                        })
                        .collect();
                    let mut ctx = Self::poly_class(
                        gen.typ().qual_name(),
                        params,
                        self.cfg.clone(),
                        self.shared.clone(),
                        2,
                        self.level,
                    );
                    let res = self.gen_class_new_method(&gen, call, &mut ctx);
                    let res2 = self.register_gen_poly_type(ident, gen, ctx, Const);
                    concat_result(res, res2)
                }
            }
            GenTypeObj::Subclass(_) => self.register_gen_subclass(ident, gen, call),
            GenTypeObj::Trait(_) => {
                if gen.typ().is_monomorphic() {
                    let mut ctx = Self::mono_trait(
                        gen.typ().qual_name(),
                        self.cfg.clone(),
                        self.shared.clone(),
                        2,
                        self.level,
                    );
                    let res = if let Some(TypeObj::Builtin {
                        t: Type::Record(req),
                        ..
                    }) = gen.base_or_sup()
                    {
                        self.register_instance_attrs(&mut ctx, req, call)
                    } else {
                        Ok(())
                    };
                    let res2 = self.register_gen_mono_type(ident, gen, ctx, Const);
                    concat_result(res, res2)
                } else {
                    let params = gen
                        .typ()
                        .typarams()
                        .into_iter()
                        .map(|tp| {
                            let name = tp.qual_name().unwrap_or(Str::ever("_"));
                            ParamSpec::named_nd(name, self.get_tp_t(&tp).unwrap_or(Type::Obj))
                        })
                        .collect();
                    let mut ctx = Self::poly_trait(
                        gen.typ().qual_name(),
                        params,
                        self.cfg.clone(),
                        self.shared.clone(),
                        2,
                        self.level,
                    );
                    let res = if let Some(TypeObj::Builtin {
                        t: Type::Record(req),
                        ..
                    }) = gen.base_or_sup()
                    {
                        self.register_instance_attrs(&mut ctx, req, call)
                    } else {
                        Ok(())
                    };
                    // Generalize the trait's type parameters (in place; the variables are
                    // shared with the requirement attribute types) so that each use of the
                    // trait (e.g. `MyTr(Int)`, `MyTr(Str)`) instantiates fresh type variables
                    gen.typ().lift();
                    let _ = self.generalize_t(gen.typ().clone());
                    let res2 = self.register_gen_poly_type(ident, gen, ctx, Const);
                    concat_result(res, res2)
                }
            }
            // `T = Structural Trait {...}`
            // The requirement record is nested inside the wrapped trait object,
            // and the type itself is `Structural(Mono(T))`, so the context must be
            // registered under the destructuralized (nominal) name in order for
            // `fields(Structural(Mono(T)))` to find the declared attributes.
            GenTypeObj::Structural(_) => {
                if gen.typ().is_monomorphic() {
                    let mut ctx = Self::mono_trait(
                        gen.typ().destructuralize().qual_name(),
                        self.cfg.clone(),
                        self.shared.clone(),
                        2,
                        self.level,
                    );
                    // for better error locations, point at the inner `Trait {...}` call
                    let req_call = call
                        .and_then(|call| match call.args.get_left_or_key("Type") {
                            Some(ast::Expr::Call(inner)) => Some(inner),
                            _ => None,
                        })
                        .or(call);
                    let res = match gen.base_or_sup() {
                        Some(TypeObj::Generated(inner)) => {
                            if let Some(TypeObj::Builtin {
                                t: Type::Record(req),
                                ..
                            }) = inner.base_or_sup()
                            {
                                self.register_instance_attrs(&mut ctx, req, req_call)
                            } else {
                                Ok(())
                            }
                        }
                        Some(TypeObj::Builtin {
                            t: Type::Record(req),
                            ..
                        }) => self.register_instance_attrs(&mut ctx, req, req_call),
                        _ => Ok(()),
                    };
                    let res2 = self.register_gen_mono_type(ident, gen, ctx, Const);
                    concat_result(res, res2)
                } else {
                    feature_error!(
                        CompileErrors,
                        CompileError,
                        self,
                        ident.loc(),
                        "polymorphic structural trait definition"
                    )
                }
            }
            GenTypeObj::Subtrait(_) => {
                if gen.typ().is_monomorphic() {
                    let super_classes = gen.base_or_sup().map_or(vec![], |t| vec![t.typ().clone()]);
                    // let super_traits = gen.impls.iter().map(|to| to.typ().clone()).collect();
                    let mut ctx = Self::mono_trait(
                        gen.typ().qual_name(),
                        self.cfg.clone(),
                        self.shared.clone(),
                        2,
                        self.level,
                    );
                    let additional = if let Some(TypeObj::Builtin {
                        t: Type::Record(additional),
                        ..
                    }) = gen.additional()
                    {
                        Some(additional)
                    } else {
                        None
                    };
                    let res = if let Some(additional) = additional {
                        self.register_instance_attrs(&mut ctx, additional, call)
                    } else {
                        Ok(())
                    };
                    for sup in super_classes.into_iter() {
                        if let Some(sup_ctx) = self.get_nominal_type_ctx(&sup) {
                            ctx.register_supertrait(sup, sup_ctx);
                        } else {
                            log!(err "{sup} not found");
                        }
                    }
                    let res2 = self.register_gen_mono_type(ident, gen, ctx, Const);
                    concat_result(res, res2)
                } else {
                    feature_error!(
                        CompileErrors,
                        CompileError,
                        self,
                        ident.loc(),
                        "polymorphic trait definition"
                    )
                }
            }
            GenTypeObj::Patch(_) => {
                if gen.typ().is_monomorphic() {
                    // The base is whatever `Patch` was applied to, and a user
                    // class is as good a base as a builtin one -- take the type
                    // out of either.
                    let Some(base) = gen.base_or_sup().map(TypeObj::typ) else {
                        return feature_error!(
                            CompileErrors,
                            CompileError,
                            self,
                            ident.loc(),
                            "patch definition with no base type"
                        );
                    };
                    // A patch with `Impl := Trait` is a glue patch,
                    // which retrofits the trait to the base type
                    let ctx = if let Some(impls) = gen.impls() {
                        Self::poly_glue_patch(
                            gen.typ().qual_name(),
                            base.clone(),
                            impls.typ().clone(),
                            vec![],
                            self.cfg.clone(),
                            self.shared.clone(),
                            2,
                            self.level,
                        )
                    } else {
                        Self::mono_patch(
                            gen.typ().qual_name(),
                            base.clone(),
                            self.cfg.clone(),
                            self.shared.clone(),
                            2,
                            self.level,
                        )
                    };
                    self.register_gen_mono_patch(ident, gen, ctx, Const)
                } else {
                    feature_error!(
                        CompileErrors,
                        CompileError,
                        self,
                        ident.loc(),
                        "polymorphic patch definition"
                    )
                }
            }
            other => feature_error!(
                CompileErrors,
                CompileError,
                self,
                ident.loc(),
                &format!("{other} definition")
            ),
        }
    }

    fn register_gen_subclass(
        &mut self,
        ident: &Identifier,
        gen: GenTypeObj,
        call: Option<&ast::Call>,
    ) -> CompileResult<()> {
        let mut errs = CompileErrors::empty();
        if gen.typ().is_monomorphic() {
            let super_classes = gen.base_or_sup().map_or(vec![], |t| vec![t.typ().clone()]);
            // let super_traits = gen.impls.iter().map(|to| to.typ().clone()).collect();
            let mut ctx = Self::mono_class(
                gen.typ().qual_name(),
                self.cfg.clone(),
                self.shared.clone(),
                2,
                self.level,
            );
            for sup in super_classes.into_iter() {
                if sup.is_failure() {
                    continue;
                }
                let sup_ctx = match self.get_nominal_type_ctx(&sup).ok_or_else(|| {
                    TyCheckErrors::from(TyCheckError::type_not_found(
                        self.cfg.input.clone(),
                        line!() as usize,
                        ident.loc(),
                        self.caused_by(),
                        &sup,
                    ))
                }) {
                    Ok(ctx) => ctx,
                    Err(es) => {
                        errs.extend(es);
                        continue;
                    }
                };
                ctx.register_superclass(sup, sup_ctx);
            }
            let mut methods =
                Self::methods(None, self.cfg.clone(), self.shared.clone(), 2, self.level);
            if let Some(sup) = gen.base_or_sup() {
                let param_t = match sup {
                    TypeObj::Builtin { t, .. } => Some(t),
                    TypeObj::Generated(t) => t.base_or_sup().map(|t| t.typ()),
                };
                let invalid_fields = if let Some(TypeObj::Builtin {
                    t: Type::Record(rec),
                    ..
                }) = gen.additional()
                {
                    if let Err((fields, es)) =
                        self.check_subtype_instance_attrs(sup.typ(), rec, call)
                    {
                        errs.extend(es);
                        fields
                    } else {
                        Set::new()
                    }
                } else {
                    Set::new()
                };
                // `Super.Requirement := {x = Int}` and `Self.Additional := {y = Int}`
                // => `Self.Requirement := {x = Int; y = Int}`
                let call_t = {
                    let (nd_params, var_params, d_params, kw_var_params) =
                        if let Some(additional) = gen.additional() {
                            if let TypeObj::Builtin {
                                t: Type::Record(rec),
                                ..
                            } = additional
                            {
                                if let Err(es) = self.register_instance_attrs(&mut ctx, rec, call) {
                                    errs.extend(es);
                                }
                            }
                            let param_t = if let Some(Type::Record(rec)) = param_t {
                                let mut rec = rec.clone();
                                rec.remove_entries(&invalid_fields);
                                Some(Type::Record(rec))
                            } else {
                                param_t.cloned()
                            };
                            let nd_params = param_t
                                .map(|pt| self.intersection(&pt, additional.typ()))
                                .or(Some(additional.typ().clone()))
                                .map_or(vec![], |t| vec![ParamTy::Pos(t)]);
                            (nd_params, None, vec![], None)
                        } else {
                            self.get_nominal_type_ctx(sup.typ())
                                .and_then(|ctx| {
                                    ctx.get_class_member(&VarName::from_static("__call__"), ctx)
                                })
                                .and_then(|vi| {
                                    Some((
                                        vi.t.non_default_params()?.clone(),
                                        vi.t.var_params().cloned(),
                                        vi.t.default_params()?.clone(),
                                        vi.t.kw_var_params().cloned(),
                                    ))
                                })
                                .unwrap_or((vec![], None, vec![], None))
                        };
                    func(
                        nd_params,
                        var_params,
                        d_params,
                        kw_var_params,
                        gen.typ().clone(),
                    )
                };
                let new_t = {
                    let (nd_params, var_params, d_params, kw_var_params) = if let Some(additional) =
                        gen.additional()
                    {
                        let param_t = if let Some(Type::Record(rec)) = param_t {
                            let mut rec = rec.clone();
                            rec.remove_entries(&invalid_fields);
                            Some(Type::Record(rec))
                        } else {
                            param_t.cloned()
                        };
                        let nd_params = param_t
                            .map(|pt| self.intersection(&pt, additional.typ()))
                            .or(Some(additional.typ().clone()))
                            .map_or(vec![], |t| vec![ParamTy::Pos(t)]);
                        (nd_params, None, vec![], None)
                    } else {
                        self.get_nominal_type_ctx(sup.typ())
                            .and_then(|ctx| {
                                ctx.get_class_member(&VarName::from_static("new"), ctx)
                                    .or_else(|| {
                                        ctx.get_class_member(&VarName::from_static("__call__"), ctx)
                                    })
                            })
                            .and_then(|vi| {
                                Some((
                                    vi.t.non_default_params()?.clone(),
                                    vi.t.var_params().cloned(),
                                    vi.t.default_params()?.clone(),
                                    vi.t.kw_var_params().cloned(),
                                ))
                            })
                            .unwrap_or((vec![], None, vec![], None))
                    };
                    func(
                        nd_params,
                        var_params,
                        d_params,
                        kw_var_params,
                        gen.typ().clone(),
                    )
                };
                if PYTHON_MODE {
                    if let Err(es) = methods.register_auto_impl(
                        "__call__",
                        call_t,
                        Immutable,
                        Visibility::private(ctx.name.clone()),
                        None,
                    ) {
                        errs.extend(es);
                    }
                } else {
                    if let Err(es) = methods.register_fixed_auto_impl(
                        "__call__",
                        call_t,
                        Immutable,
                        Visibility::private(ctx.name.clone()),
                        None,
                    ) {
                        errs.extend(es);
                    }
                    // 必要なら、ユーザーが独自に上書きする
                    if let Err(es) = methods.register_auto_impl(
                        "new",
                        new_t,
                        Immutable,
                        Visibility::public(ctx.name.clone()),
                        None,
                    ) {
                        errs.extend(es);
                    }
                }
                ctx.methods_list.push(MethodContext::new(
                    DefId(0),
                    ClassDefType::Simple(gen.typ().clone()),
                    methods,
                ));
                if let Err(es) = self.register_gen_mono_type(ident, gen, ctx, Const) {
                    errs.extend(es);
                }
                if errs.is_empty() {
                    Ok(())
                } else {
                    Err(errs)
                }
            } else {
                let class_name = gen
                    .base_or_sup()
                    .map(|t| t.typ().local_name())
                    .unwrap_or(Str::from("?"));
                Err(CompileErrors::from(CompileError::no_type_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    ident.loc(),
                    self.caused_by(),
                    &class_name,
                    self.get_similar_name(&class_name),
                )))
            }
        } else {
            feature_error!(
                CompileErrors,
                CompileError,
                self,
                ident.loc(),
                "polymorphic class definition"
            )
        }
    }

    fn check_subtype_instance_attrs(
        &self,
        sup: &Type,
        rec: &Dict<Field, Type>,
        call: Option<&ast::Call>,
    ) -> Result<(), (Set<Field>, CompileErrors)> {
        let mut errs = CompileErrors::empty();
        let mut invalid_fields = Set::new();
        let sup_ctx = self.get_nominal_type_ctx(sup);
        let additional = call.and_then(|call| {
            if let Some(ast::Expr::Record(record)) = call.args.get_with_key("Additional") {
                Some(record)
            } else {
                None
            }
        });
        for (field, sub_t) in rec.iter() {
            let loc = additional
                .as_ref()
                .and_then(|record| {
                    record
                        .keys()
                        .iter()
                        .find(|id| id.inspect() == &field.symbol)
                        .map(|name| name.loc())
                })
                .unwrap_or_default();
            let varname = VarName::from_str(field.symbol.clone());
            if let Some(sup_ctx) = sup_ctx {
                if let Some(sup_vi) = sup_ctx.decls.get(&varname) {
                    if !self.subtype_of(sub_t, &sup_vi.t) {
                        invalid_fields.insert(field.clone());
                        errs.push(CompileError::type_mismatch_error(
                            self.cfg.input.clone(),
                            line!() as usize,
                            loc,
                            self.caused_by(),
                            &field.symbol,
                            None,
                            &sup_vi.t,
                            sub_t,
                            None,
                            None,
                        ));
                    }
                }
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err((invalid_fields, errs))
        }
    }

    fn register_instance_attrs(
        &self,
        ctx: &mut Context,
        rec: &Dict<Field, Type>,
        call: Option<&ast::Call>,
    ) -> CompileResult<()> {
        let mut errs = CompileErrors::empty();
        let record = call.and_then(|call| {
            if let Some(ast::Expr::Record(record)) = call
                .args
                .get_left_or_key("Base")
                .or_else(|| call.args.get_left_or_key("Requirement"))
                .or_else(|| call.args.get_left_or_key("Super"))
            {
                Some(record)
            } else {
                None
            }
        });
        for (field, t) in rec.iter() {
            let loc = record
                .as_ref()
                .and_then(|record| {
                    record
                        .keys()
                        .iter()
                        .find(|id| id.inspect() == &field.symbol)
                        .map(|name| self.absolutize(name.loc()))
                })
                .unwrap_or(AbsLocation::unknown());
            let varname = VarName::from_str(field.symbol.clone());
            let vi = VarInfo::instance_attr(
                field.clone(),
                t.clone(),
                self.kind.clone(),
                ctx.name.clone(),
                loc,
            );
            // self.index().register(&vi);
            if let Some(_ent) = ctx.decls.insert(varname.clone(), vi) {
                errs.push(CompileError::duplicate_decl_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    varname.loc(),
                    self.caused_by(),
                    varname.inspect(),
                ));
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }

    fn gen_class_new_method(
        &self,
        gen: &GenTypeObj,
        call: Option<&ast::Call>,
        ctx: &mut Context,
    ) -> CompileResult<()> {
        let mut methods = Self::methods(None, self.cfg.clone(), self.shared.clone(), 2, self.level);
        let new_t = if let Some(base) = gen.base_or_sup() {
            match base {
                TypeObj::Builtin {
                    t: Type::Record(rec),
                    ..
                } => {
                    self.register_instance_attrs(ctx, rec, call)?;
                }
                other => {
                    methods.register_fixed_auto_impl(
                        "base",
                        other.typ().clone(),
                        Immutable,
                        Visibility::BUILTIN_PRIVATE,
                        None,
                    )?;
                }
            }
            func1(base.typ().clone(), gen.typ().clone())
        } else {
            func0(gen.typ().clone())
        };
        let new_t = if gen.typ().is_monomorphic() {
            new_t
        } else {
            // Generalize the class's type parameters (in place, the variables are shared
            // with `gen.typ()` etc.) so that each use of the class instantiates fresh
            // type variables (e.g. `T` of `Box.new {value = 1}` and `Box.new {value = "a"}`)
            new_t.lift();
            gen.typ().lift();
            let new_t = self.generalize_t(new_t);
            let _ = self.generalize_t(gen.typ().clone());
            new_t
        };
        if ERG_MODE {
            methods.register_fixed_auto_impl(
                "__call__",
                new_t.clone(),
                Immutable,
                Visibility::private(ctx.name.clone()),
                Some("__call__".into()),
            )?;
            // users can override this if necessary
            methods.register_auto_impl(
                "new",
                new_t,
                Immutable,
                Visibility::public(ctx.name.clone()),
                None,
            )?;
        } else {
            methods.register_auto_impl(
                "__call__",
                new_t,
                Immutable,
                Visibility::public(ctx.name.clone()),
                Some("__call__".into()),
            )?;
        }
        ctx.methods_list.push(MethodContext::new(
            DefId(0),
            ClassDefType::Simple(gen.typ().clone()),
            methods,
        ));
        Ok(())
    }

    pub(crate) fn register_type_alias(
        &mut self,
        ident: &Identifier,
        t: Type,
        meta_t: Type,
    ) -> CompileResult<()> {
        let vis = self.instantiate_vis_modifier(&ident.vis)?;
        let inited = self
            .rec_get_const_obj(ident.inspect())
            .is_some_and(|v| v.is_inited());
        if inited && vis.is_private() {
            // TODO: display where defined
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
                ident.inspect(),
            )))
        } else {
            let name = &ident.name;
            let muty = Mutability::from(&ident.inspect()[..]);
            let id = DefId(get_hash(&(&self.name, &name)));
            let val = ValueObj::Type(TypeObj::Builtin { t, meta_t });
            let vi = VarInfo::new(
                v_enum(set! { val.clone() }),
                muty,
                Visibility::new(vis, self.name.clone()),
                VarKind::Defined(id),
                None,
                self.kind.clone(),
                None,
                self.absolutize(name.loc()),
            );
            self.index().register(name.inspect().clone(), &vi);
            self.decls.insert(name.clone(), vi);
            self.consts.insert(name.clone(), val);
            Ok(())
        }
    }

    fn register_gen_mono_type(
        &mut self,
        ident: &Identifier,
        gen: GenTypeObj,
        ctx: Self,
        muty: Mutability,
    ) -> CompileResult<()> {
        let vis = self.instantiate_vis_modifier(&ident.vis)?;
        let inited = self
            .rec_get_const_obj(ident.inspect())
            .is_some_and(|v| v.is_inited());
        let vi = self.rec_get_var_info(ident, crate::AccessKind::Name, &self.cfg.input, self);
        if inited && vi.is_ok_and(|vi| vi.def_loc != self.absolutize(ident.loc())) {
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
                ident.inspect(),
            )))
        } else {
            let t = gen.typ().clone();
            let val = ValueObj::Type(TypeObj::Generated(gen));
            let meta_t = v_enum(set! { val.clone() });
            let name = &ident.name;
            let id = DefId(get_hash(&(&self.name, &name)));
            let vi = VarInfo::new(
                meta_t,
                muty,
                Visibility::new(vis, self.name.clone()),
                VarKind::Defined(id),
                None,
                self.kind.clone(),
                None,
                self.absolutize(name.loc()),
            );
            self.index().register(name.inspect().clone(), &vi);
            self.decls.insert(name.clone(), vi);
            self.consts.insert(name.clone(), val);
            self.register_methods(&t, &ctx);
            self.mono_types
                .insert(name.clone(), TypeContext::new(t, ctx));
            Ok(())
        }
    }

    fn register_gen_poly_type(
        &mut self,
        ident: &Identifier,
        gen: GenTypeObj,
        ctx: Self,
        muty: Mutability,
    ) -> CompileResult<()> {
        let vis = self.instantiate_vis_modifier(&ident.vis)?;
        let inited = self
            .rec_get_const_obj(ident.inspect())
            .is_some_and(|v| v.is_inited());
        if inited && vis.is_private() {
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
                ident.inspect(),
            )))
        } else {
            let t = gen.typ().clone();
            let val = ValueObj::Type(TypeObj::Generated(gen));
            let params = t
                .typarams()
                .into_iter()
                .map(|tp| {
                    ParamTy::Pos(tp_enum(
                        self.get_tp_t(&tp).unwrap_or(Type::Obj),
                        set! { tp },
                    ))
                })
                .collect();
            let meta_t = func(params, None, vec![], None, v_enum(set! { val.clone() })).quantify();
            let name = &ident.name;
            let id = DefId(get_hash(&(&self.name, &name)));
            let vi = VarInfo::new(
                meta_t,
                muty,
                Visibility::new(vis, self.name.clone()),
                VarKind::Defined(id),
                None,
                self.kind.clone(),
                None,
                self.absolutize(name.loc()),
            );
            self.index().register(name.inspect().clone(), &vi);
            self.decls.insert(name.clone(), vi);
            self.consts.insert(name.clone(), val);
            self.register_methods(&t, &ctx);
            self.poly_types
                .insert(name.clone(), TypeContext::new(t, ctx));
            Ok(())
        }
    }

    fn register_gen_mono_patch(
        &mut self,
        ident: &Identifier,
        gen: GenTypeObj,
        ctx: Self,
        muty: Mutability,
    ) -> CompileResult<()> {
        let vis = self.instantiate_vis_modifier(&ident.vis)?;
        // FIXME: recursive search
        if self.patches.contains_key(ident.inspect()) {
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
                ident.inspect(),
            )))
        } else if self.rec_get_const_obj(ident.inspect()).is_some() && vis.is_private() {
            Err(CompileErrors::from(CompileError::reassign_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
                ident.inspect(),
            )))
        } else {
            let t = gen.typ().clone();
            let meta_t = gen.meta_type();
            let name = &ident.name;
            let id = DefId(get_hash(&(&self.name, &name)));
            self.decls.insert(
                name.clone(),
                VarInfo::new(
                    meta_t,
                    muty,
                    Visibility::new(vis, self.name.clone()),
                    VarKind::Defined(id),
                    None,
                    self.kind.clone(),
                    None,
                    self.absolutize(name.loc()),
                ),
            );
            self.consts
                .insert(name.clone(), ValueObj::Type(TypeObj::Generated(gen)));
            self.register_methods(&t, &ctx);
            // If this is a glue patch (`P = Patch C, Impl := T`), record that the base type
            // now implements the trait (used by trait impl searches; the subtype judgment
            // itself is made by `Context::find_compatible_glue_patch`)
            if let ContextKind::GluePatch(tr_impl) = &ctx.kind {
                let declared_in = NormalizedPathBuf::from(self.module_path());
                let declared_in = declared_in.exists().then_some(declared_in);
                let tr_impl = TraitImpl::new(
                    tr_impl.sub_type.clone(),
                    tr_impl.sup_trait.clone(),
                    declared_in,
                );
                if let Some(mut impls) = self.trait_impls().get_mut(&tr_impl.sup_trait.qual_name())
                {
                    impls.insert(tr_impl);
                } else {
                    self.trait_impls()
                        .register(tr_impl.sup_trait.qual_name(), set! {tr_impl});
                }
            }
            self.patches.insert(name.clone(), ctx);
            Ok(())
        }
    }

    pub(crate) fn import_mod(
        &mut self,
        kind: OperationKind,
        mod_name: &Literal,
    ) -> CompileResult<PathBuf> {
        let ValueObj::Str(__name__) = &mod_name.value else {
            let name = if kind.is_erg_import() {
                "import"
            } else {
                "pyimport"
            };
            return Err(TyCheckErrors::from(TyCheckError::type_mismatch_error(
                self.cfg.input.clone(),
                line!() as usize,
                mod_name.loc(),
                self.caused_by(),
                name,
                Some(1),
                &Type::Str,
                &mod_name.t(),
                None,
                None,
            )));
        };
        if !valid_mod_name(__name__) {
            return Err(TyCheckErrors::from(TyCheckError::syntax_error(
                self.cfg.input.clone(),
                line!() as usize,
                mod_name.loc(),
                self.caused_by(),
                format!("{__name__} is not a valid module name"),
                None,
            )));
        }
        if kind.is_erg_import() {
            self.import_erg_mod(__name__, mod_name)
        } else {
            self.import_py_mod(__name__, mod_name)
        }
    }

    fn import_err(&self, line: u32, __name__: &Str, loc: &impl Locational) -> TyCheckErrors {
        let mod_cache = self.mod_cache();
        let py_mod_cache = self.py_mod_cache();
        TyCheckErrors::from(TyCheckError::import_error(
            self.cfg.input.clone(),
            line as usize,
            format!("module {__name__} not found"),
            loc.loc(),
            self.caused_by(),
            self.similar_builtin_erg_mod_name(__name__)
                .or_else(|| mod_cache.get_similar_name(__name__)),
            self.similar_builtin_py_mod_name(__name__)
                .or_else(|| py_mod_cache.get_similar_name(__name__)),
        ))
    }

    fn import_erg_mod(&self, __name__: &Str, loc: &impl Locational) -> CompileResult<PathBuf> {
        let path = match self
            .cfg
            .input
            .resolve_real_path(Path::new(&__name__[..]), &self.cfg)
        {
            Some(path) => path,
            None => {
                return Err(self.import_err(line!(), __name__, loc));
            }
        };
        if ERG_MODE {
            self.check_mod_vis(path.as_path(), __name__, loc)?;
        }
        Ok(path)
    }

    /// If the path is like `foo/bar`, check if `bar` is a public module (the definition is in `foo/__init__.er`)
    fn check_mod_vis(
        &self,
        path: &Path,
        __name__: &Str,
        loc: &impl Locational,
    ) -> CompileResult<()> {
        let file_kind = FileKind::from(path);
        let parent = if file_kind.is_init_er() {
            path.parent().and_then(|p| p.parent())
        } else {
            path.parent()
        };
        if let Some(parent) = parent {
            if DirKind::from(parent).is_erg_module() {
                let parent = parent.join("__init__.er");
                // `check_mod_vis` takes `&self`, so the parent `__init__.er` cannot be
                // built here; it is expected to already be in the module cache by the
                // time a submodule is imported. If it isn't cached, the visibility check
                // is skipped (best-effort).
                if let Some(parent_module) = self.get_mod_with_path(&parent) {
                    let import_err = |line| {
                        TyCheckErrors::from(TyCheckError::import_error(
                            self.cfg.input.clone(),
                            line as usize,
                            format!("module `{__name__}` is not public"),
                            loc.loc(),
                            self.caused_by(),
                            None,
                            None,
                        ))
                    };
                    let file_stem = if file_kind.is_init_er() {
                        path.parent().and_then(|p| p.file_stem())
                    } else {
                        path.file_stem()
                    };
                    let mod_name = file_stem.unwrap_or_default().to_string_lossy();
                    if let Some((_, vi)) = parent_module.get_var_info(&mod_name) {
                        if !vi.vis.compatible(&ast::AccessModifier::Public, self) {
                            return Err(import_err(line!()));
                        }
                    } else {
                        return Err(import_err(line!()));
                    }
                }
            }
        }
        Ok(())
    }

    fn similar_builtin_py_mod_name(&self, name: &Str) -> Option<Str> {
        get_similar_name(BUILTIN_PYTHON_MODS.into_iter(), name).map(Str::rc)
    }

    fn similar_builtin_erg_mod_name(&self, name: &Str) -> Option<Str> {
        get_similar_name(BUILTIN_ERG_MODS.into_iter(), name).map(Str::rc)
    }

    fn get_decl_path(&self, __name__: &Str, loc: &impl Locational) -> CompileResult<PathBuf> {
        match self
            .cfg
            .input
            .resolve_decl_path(Path::new(&__name__[..]), &self.cfg)
        {
            Some(path) => {
                if self.cfg.input.decl_file_is(&path) {
                    return Ok(path);
                }
                if is_pystd_main_module(path.as_path())
                    && !BUILTIN_PYTHON_MODS.contains(&&__name__[..])
                {
                    let err = TyCheckError::module_env_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        __name__,
                        loc.loc(),
                        self.caused_by(),
                    );
                    return Err(TyCheckErrors::from(err));
                }
                Ok(path)
            }
            None => {
                let err = TyCheckError::import_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    format!("module {__name__} not found"),
                    loc.loc(),
                    self.caused_by(),
                    self.similar_builtin_erg_mod_name(__name__)
                        .or_else(|| self.mod_cache().get_similar_name(__name__)),
                    self.similar_builtin_py_mod_name(__name__)
                        .or_else(|| self.py_mod_cache().get_similar_name(__name__)),
                );
                Err(TyCheckErrors::from(err))
            }
        }
    }

    fn import_py_mod(&self, __name__: &Str, loc: &impl Locational) -> CompileResult<PathBuf> {
        match self.get_decl_path(__name__, loc) {
            Ok(path) => {
                // module itself
                if self.cfg.input.path() == path.as_path() {
                    return Ok(path);
                }
                if self.py_mod_cache().get(&path).is_some() {
                    return Ok(path);
                }
                Ok(path)
            }
            Err(_) => {
                let path = Path::new(&__name__[..]);
                if let Ok(pyi_path) = self.cfg.input.resolve_pyi(path) {
                    return Ok(pyi_path);
                }
                let py_path = self
                    .cfg
                    .input
                    .resolve_py(path)
                    .or_else(|_| {
                        for sys_path in python_sys_path() {
                            let mut dir = sys_path.clone();
                            dir.push(path);
                            dir.set_extension("py");
                            if dir.exists() {
                                return Ok(normalize_path(dir));
                            }
                            let mut dir = sys_path.clone();
                            dir.push(path);
                            dir.push("__init__.py");
                            if dir.exists() {
                                return Ok(normalize_path(dir));
                            }
                        }
                        for pkgs_path in python_site_packages() {
                            let mut dir = pkgs_path.clone();
                            dir.push(path);
                            dir.set_extension("py");
                            if dir.exists() {
                                return Ok(normalize_path(dir));
                            }
                            let mut dir = pkgs_path.clone();
                            dir.push(path);
                            dir.push("__init__.py");
                            if dir.exists() {
                                return Ok(normalize_path(dir));
                            }
                        }
                        Err(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            format!("module {__name__} not found"),
                        ))
                    })
                    .map_err(|_| {
                        TyCheckErrors::from(TyCheckError::import_error(
                            self.cfg.input.clone(),
                            line!() as usize,
                            format!("module {__name__} not found"),
                            loc.loc(),
                            self.caused_by(),
                            self.similar_builtin_erg_mod_name(__name__)
                                .or_else(|| self.mod_cache().get_similar_name(__name__)),
                            self.similar_builtin_py_mod_name(__name__)
                                .or_else(|| self.py_mod_cache().get_similar_name(__name__)),
                        ))
                    })?;
                if self.cfg.input.path() == py_path.as_path() {
                    return Ok(py_path);
                }
                let norm_path = NormalizedPathBuf::from(py_path.clone());
                if self.py_mod_cache().get(&norm_path).is_some() {
                    return Ok(py_path);
                }
                // Register an empty (untyped) module context
                let cfg = self.cfg.inherit(py_path.clone());
                let ctx = Context::new(
                    Str::rc(&__name__[..]),
                    cfg,
                    ContextKind::Module,
                    vec![],
                    None,
                    self.shared.clone(),
                    Self::TOP_LEVEL,
                );
                let mod_ctx = ModuleContext::new(ctx, erg_common::dict::Dict::new());
                self.py_mod_cache().register(
                    py_path.clone(),
                    None,
                    None,
                    mod_ctx,
                    CheckStatus::Succeed,
                );
                Ok(py_path)
            }
        }
    }

    pub fn del(&mut self, ident: &hir::Identifier) -> CompileResult<()> {
        let is_const = self
            .rec_get_var_info(&ident.raw, crate::AccessKind::Name, &self.cfg.input, self)
            .map_ok_or(false, |vi| vi.muty.is_const());
        let is_builtin = self
            .get_builtins()
            .unwrap()
            .get_var_kv(ident.inspect())
            .is_some();
        if is_const || is_builtin {
            Err(TyCheckErrors::from(TyCheckError::del_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident,
                is_const,
                self.caused_by(),
            )))
        } else if self.locals.get(ident.inspect()).is_some() {
            let vi = self.locals.remove(ident.inspect()).unwrap();
            self.deleted_locals.insert(ident.raw.name.clone(), vi);
            Ok(())
        } else {
            Err(TyCheckErrors::from(TyCheckError::no_var_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.loc(),
                self.caused_by(),
                ident.inspect(),
                self.get_similar_name(ident.inspect()),
            )))
        }
    }

    pub(crate) fn get_casted_type(&self, expr: &ast::Expr) -> Option<Type> {
        for guard in self.rec_get_guards() {
            if !self.name.starts_with(&guard.namespace[..]) {
                continue;
            }
            if let CastTarget::Expr(target) = guard.target.as_ref() {
                if expr == target {
                    return Some(*guard.to.clone());
                }
                // { r.x in Int } =>  { r in Structural { .x = Int } }
                else if let ast::Expr::Accessor(ast::Accessor::Attr(attr)) = target {
                    if attr.obj.as_ref() == expr {
                        let mut rec = Dict::new();
                        let vis = self.instantiate_vis_modifier(&attr.ident.vis).ok()?;
                        let field = Field::new(vis, attr.ident.inspect().clone());
                        rec.insert(field, *guard.to.clone());
                        return Some(Type::Record(rec).structuralize());
                    }
                }
            }
        }
        None
    }

    pub(crate) fn cast(
        &mut self,
        guard: GuardType,
        args: Option<&hir::Args>,
        overwritten: &mut Vec<(VarName, VarInfo)>,
    ) -> TyCheckResult<()> {
        match guard.target.as_ref() {
            CastTarget::Var { name, .. } => {
                if !self.name.starts_with(&guard.namespace[..]) {
                    return Ok(());
                }
                let vi = if let Some((name, vi)) = self.locals.remove_entry(name) {
                    overwritten.push((name, vi.clone()));
                    vi
                } else if let Some((n, vi)) = self.get_var_kv(name) {
                    overwritten.push((n.clone(), vi.clone()));
                    vi.clone()
                } else {
                    VarInfo::nd_parameter(
                        *guard.to.clone(),
                        self.absolutize(().loc()),
                        self.name.clone(),
                    )
                };
                match self.recover_typarams(&vi.t, &guard) {
                    Ok(t) => {
                        self.locals
                            .insert(VarName::from_str(name.clone()), VarInfo { t, ..vi });
                    }
                    Err(errs) => {
                        self.locals.insert(VarName::from_str(name.clone()), vi);
                        return Err(errs);
                    }
                }
                Ok(())
            }
            // ```
            // i: Obj
            // is_int: (x: Obj) -> {x in Int} # change the 0th arg type to Int
            // assert is_int i
            // i: Int
            // ```
            CastTarget::Arg { nth, name, loc } => {
                if let Some(name) = args
                    .and_then(|args| args.get(*nth))
                    .and_then(|ex| ex.local_name())
                {
                    let vi = if let Some((name, vi)) = self.locals.remove_entry(name) {
                        overwritten.push((name, vi.clone()));
                        vi
                    } else if let Some((n, vi)) = self.get_var_kv(name) {
                        overwritten.push((n.clone(), vi.clone()));
                        vi.clone()
                    } else {
                        VarInfo::nd_parameter(
                            *guard.to.clone(),
                            self.absolutize(().loc()),
                            self.name.clone(),
                        )
                    };
                    match self.recover_typarams(&vi.t, &guard) {
                        Ok(t) => {
                            self.locals
                                .insert(VarName::from_str(Str::rc(name)), VarInfo { t, ..vi });
                        }
                        Err(errs) => {
                            self.locals.insert(VarName::from_str(Str::rc(name)), vi);
                            return Err(errs);
                        }
                    }
                    Ok(())
                } else {
                    let target = CastTarget::Var {
                        name: name.clone(),
                        loc: *loc,
                    };
                    let guard = GuardType::new(guard.namespace, target, *guard.to);
                    self.cast(guard, args, overwritten)
                }
            }
            CastTarget::Expr(_) => {
                self.guards.push(guard);
                Ok(())
            }
        }
    }

    pub(crate) fn inc_ref<L: Locational>(
        &self,
        name: &Str,
        vi: &VarInfo,
        loc: &L,
        namespace: &Context,
    ) {
        if let Some(index) = self.opt_index() {
            index.inc_ref(name, vi, namespace.absolutize(loc.loc()));
        }
    }

    pub(crate) fn inc_ref_acc(
        &self,
        acc: &ast::Accessor,
        namespace: &Context,
        tmp_tv_cache: &TyVarCache,
    ) -> bool {
        match acc {
            ast::Accessor::Ident(ident) => self.inc_ref_local(ident, namespace, tmp_tv_cache),
            ast::Accessor::Attr(attr) => {
                self.inc_ref_expr(&attr.obj, namespace, tmp_tv_cache);
                if let Ok(ctxs) = self.get_singular_ctxs(&attr.obj, self) {
                    for ctx in ctxs {
                        if ctx.inc_ref_local(&attr.ident, namespace, tmp_tv_cache) {
                            return true;
                        }
                    }
                }
                false
            }
            other => {
                log!(err "inc_ref_acc: {other}");
                false
            }
        }
    }

    pub(crate) fn inc_ref_predecl_typespec(
        &self,
        predecl: &PreDeclTypeSpec,
        namespace: &Context,
        tmp_tv_cache: &TyVarCache,
    ) -> bool {
        match predecl {
            PreDeclTypeSpec::Mono(mono) => {
                self.inc_ref_mono_typespec(mono, namespace, tmp_tv_cache)
            }
            PreDeclTypeSpec::Poly(poly) => {
                self.inc_ref_poly_typespec(poly, namespace, tmp_tv_cache)
            }
            PreDeclTypeSpec::Attr { namespace: obj, t } => {
                self.inc_ref_expr(obj, namespace, tmp_tv_cache);
                if let Ok(ctxs) = self.get_singular_ctxs(obj, self) {
                    for ctx in ctxs {
                        if ctx.inc_ref_mono_typespec(t, namespace, tmp_tv_cache) {
                            return true;
                        }
                    }
                }
                false
            }
            // TODO:
            PreDeclTypeSpec::Subscr { namespace: ns, .. } => {
                self.inc_ref_expr(ns, namespace, tmp_tv_cache)
            }
        }
    }

    fn inc_ref_mono_typespec(
        &self,
        ident: &Identifier,
        namespace: &Context,
        tmp_tv_cache: &TyVarCache,
    ) -> bool {
        if let Triple::Ok(vi) = self.rec_get_var_info(
            ident,
            crate::compile::AccessKind::Name,
            &self.cfg.input,
            self,
        ) {
            self.inc_ref(ident.inspect(), &vi, &ident.name, namespace);
            true
        } else if let Some(vi) = tmp_tv_cache.var_infos.get(&ident.name) {
            self.inc_ref(ident.inspect(), vi, &ident.name, namespace);
            true
        } else {
            false
        }
    }

    fn inc_ref_poly_typespec(
        &self,
        poly: &PolyTypeSpec,
        namespace: &Context,
        tmp_tv_cache: &TyVarCache,
    ) -> bool {
        for arg in poly.args.pos_args() {
            self.inc_ref_expr(&arg.expr.clone().downgrade(), namespace, tmp_tv_cache);
        }
        if let Some(arg) = poly.args.var_args.as_ref() {
            self.inc_ref_expr(&arg.expr.clone().downgrade(), namespace, tmp_tv_cache);
        }
        for arg in poly.args.kw_args() {
            self.inc_ref_expr(&arg.expr.clone().downgrade(), namespace, tmp_tv_cache);
        }
        if let Some(arg) = poly.args.kw_var.as_ref() {
            self.inc_ref_expr(&arg.expr.clone().downgrade(), namespace, tmp_tv_cache);
        }
        self.inc_ref_acc(&poly.acc.clone().downgrade(), namespace, tmp_tv_cache)
    }

    pub(crate) fn inc_ref_local(
        &self,
        local: &ConstIdentifier,
        namespace: &Context,
        tmp_tv_cache: &TyVarCache,
    ) -> bool {
        if let Triple::Ok(vi) = self.rec_get_var_info(
            local,
            crate::compile::AccessKind::Name,
            &self.cfg.input,
            self,
        ) {
            self.inc_ref(local.inspect(), &vi, &local.name, namespace);
            true
        } else if let Some(vi) = tmp_tv_cache.var_infos.get(&local.name) {
            self.inc_ref(local.inspect(), vi, &local.name, namespace);
            true
        } else {
            &local.inspect()[..] == "module" || &local.inspect()[..] == "global"
        }
    }

    fn inc_ref_block(
        &self,
        block: &ast::Block,
        namespace: &Context,
        tmp_tv_cache: &TyVarCache,
    ) -> bool {
        let mut res = false;
        for expr in block.iter() {
            if self.inc_ref_expr(expr, namespace, tmp_tv_cache) {
                res = true;
            }
        }
        res
    }

    fn inc_ref_params(
        &self,
        params: &ast::Params,
        namespace: &Context,
        tmp_tv_cache: &TyVarCache,
    ) -> bool {
        let mut res = false;
        for param in params.non_defaults.iter() {
            if let Some(expr) = param.t_spec.as_ref().map(|ts| &ts.t_spec_as_expr) {
                if self.inc_ref_expr(expr, namespace, tmp_tv_cache) {
                    res = true;
                }
            }
        }
        if let Some(expr) = params
            .var_params
            .as_ref()
            .and_then(|p| p.t_spec.as_ref().map(|ts| &ts.t_spec_as_expr))
        {
            if self.inc_ref_expr(expr, namespace, tmp_tv_cache) {
                res = true;
            }
        }
        for param in params.defaults.iter() {
            if let Some(expr) = param.sig.t_spec.as_ref().map(|ts| &ts.t_spec_as_expr) {
                if self.inc_ref_expr(expr, namespace, tmp_tv_cache) {
                    res = true;
                }
            }
            if self.inc_ref_expr(&param.default_val, namespace, tmp_tv_cache) {
                res = true;
            }
        }
        if let Some(expr) = params
            .kw_var_params
            .as_ref()
            .and_then(|p| p.t_spec.as_ref().map(|ts| &ts.t_spec_as_expr))
        {
            if self.inc_ref_expr(expr, namespace, tmp_tv_cache) {
                res = true;
            }
        }
        res
    }

    fn inc_ref_expr(
        &self,
        expr: &ast::Expr,
        namespace: &Context,
        tmp_tv_cache: &TyVarCache,
    ) -> bool {
        match expr {
            ast::Expr::Literal(_) => false,
            ast::Expr::Accessor(acc) => self.inc_ref_acc(acc, namespace, tmp_tv_cache),
            ast::Expr::BinOp(bin) => {
                let mut res = false;
                if self.inc_ref_expr(&bin.args[0], namespace, tmp_tv_cache) {
                    res = true;
                }
                if self.inc_ref_expr(&bin.args[1], namespace, tmp_tv_cache) {
                    res = true;
                }
                res
            }
            ast::Expr::UnaryOp(unary) => self.inc_ref_expr(&unary.value(), namespace, tmp_tv_cache),
            ast::Expr::Call(call) => {
                let mut res = self.inc_ref_expr(&call.obj, namespace, tmp_tv_cache);
                for arg in call.args.pos_args() {
                    if self.inc_ref_expr(&arg.expr, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                if let Some(arg) = call.args.var_args() {
                    if self.inc_ref_expr(&arg.expr, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                for arg in call.args.kw_args() {
                    if self.inc_ref_expr(&arg.expr, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::Record(ast::Record::Normal(rec)) => {
                let mut res = false;
                for val in rec.attrs.iter() {
                    if self.inc_ref_block(&val.body.block, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::Record(ast::Record::Mixed(rec)) => {
                let mut res = false;
                for val in rec.attrs.iter() {
                    match val {
                        RecordAttrOrIdent::Attr(attr) => {
                            if self.inc_ref_block(&attr.body.block, namespace, tmp_tv_cache) {
                                res = true;
                            }
                        }
                        RecordAttrOrIdent::Ident(ident) => {
                            if self.inc_ref_local(ident, namespace, tmp_tv_cache) {
                                res = true;
                            }
                        }
                    }
                }
                res
            }
            ast::Expr::List(ast::List::Normal(lis)) => {
                let mut res = false;
                for val in lis.elems.pos_args().iter() {
                    if self.inc_ref_expr(&val.expr, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::Tuple(ast::Tuple::Normal(tup)) => {
                let mut res = false;
                for val in tup.elems.pos_args().iter() {
                    if self.inc_ref_expr(&val.expr, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::Set(ast::Set::Normal(set)) => {
                let mut res = false;
                for val in set.elems.pos_args().iter() {
                    if self.inc_ref_expr(&val.expr, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::Set(ast::Set::Comprehension(comp)) => {
                let mut res = false;
                for (_, gen) in comp.generators.iter() {
                    if self.inc_ref_expr(gen, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                if let Some(guard) = &comp.guard {
                    if self.inc_ref_expr(guard, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::Dict(ast::Dict::Normal(dict)) => {
                let mut res = false;
                for ast::KeyValue { key, value } in dict.kvs.iter() {
                    if self.inc_ref_expr(key, namespace, tmp_tv_cache) {
                        res = true;
                    }
                    if self.inc_ref_expr(value, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::Dict(ast::Dict::Comprehension(comp)) => {
                let mut res = false;
                for (_, gen) in comp.generators.iter() {
                    if self.inc_ref_expr(gen, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                if let Some(guard) = &comp.guard {
                    if self.inc_ref_expr(guard, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::TypeAscription(ascription) => {
                self.inc_ref_expr(&ascription.expr, namespace, tmp_tv_cache)
            }
            ast::Expr::Compound(comp) => {
                let mut res = false;
                for expr in comp.exprs.iter() {
                    if self.inc_ref_expr(expr, namespace, tmp_tv_cache) {
                        res = true;
                    }
                }
                res
            }
            ast::Expr::Lambda(lambda) => {
                let mut res = false;
                // FIXME: assign params
                if self.inc_ref_params(&lambda.sig.params, namespace, tmp_tv_cache) {
                    res = true;
                }
                if self.inc_ref_block(&lambda.body, namespace, tmp_tv_cache) {
                    res = true;
                }
                res
            }
            other => {
                log!(err "inc_ref_expr: {other}");
                false
            }
        }
    }
}
