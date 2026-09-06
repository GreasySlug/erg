use std::mem;

use erg_common::config::ErgConfig;
use erg_common::dict::Dict;
use erg_common::error::Location;
use erg_common::set::Set;
use erg_common::style::colors::DEBUG_MAIN;
use erg_common::traits::{Locational, Stream};
use erg_common::Str;
use erg_common::{impl_display_from_debug, log};
use erg_parser::ast::{ParamPattern, VarName};

use crate::ty::{HasType, Ownership, Type, Visibility};

use crate::error::{OwnershipError, OwnershipErrors};
use crate::hir::{self, Accessor, Block, Call, Def, Expr, Identifier, List, Signature, Tuple, HIR};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WrapperKind {
    Ref,
    Rc,
    Box,
}

#[derive(Debug, Default)]
struct LocalVars {
    alive_vars: Set<Str>,
    dropped_vars: Dict<Str, Location>,
}

impl_display_from_debug!(LocalVars);

/// Check code ownership.
/// for example:
/// * Check if moved variables are not used again.
/// * Checks whether a mutable reference method is called in an immutable reference method.
#[derive(Debug)]
pub struct OwnershipChecker {
    cfg: ErgConfig,
    path_stack: Vec<Visibility>,
    dict: Dict<Str, LocalVars>,
    /// Directed graph of named mutable objects that may store one another.
    /// Key: unique id of the source variable; value: (dest id, call site).
    stores: Dict<Str, Vec<(Str, Location)>>,
    store_names: Dict<Str, Str>,
    /// The mutable variables each named closure captures (key: `ident_key` of the
    /// closure). A closure reads its captures when it is called, so using the closure
    /// after one of them was moved is using the moved variable. Each is paired with the
    /// path of the scope that owns it: a name alone would match an unrelated variable
    /// of the same name in whatever scope the closure is called from.
    captures: Dict<Str, Vec<(Str, Identifier)>>,
    /// One frame per entry of `path_stack` but the module's: the mutable outer
    /// variables the scope being checked has accessed so far.
    capture_frames: Vec<Vec<(Str, Identifier)>>,
    /// The `path_stack` depth of each function-like scope being checked (a subroutine
    /// definition or a lambda). A variable owned outside the innermost of them is
    /// captured by it, not moved into it: the closure runs as many times as it is
    /// called, and the scope that owns the variable goes on using it afterwards.
    fn_scopes: Vec<usize>,
    errs: OwnershipErrors,
}

impl OwnershipChecker {
    pub fn new(cfg: ErgConfig) -> Self {
        OwnershipChecker {
            cfg,
            path_stack: vec![],
            dict: Dict::new(),
            stores: Dict::new(),
            store_names: Dict::new(),
            captures: Dict::new(),
            capture_frames: vec![],
            fn_scopes: vec![],
            errs: OwnershipErrors::empty(),
        }
    }

    fn full_path(&self) -> String {
        self.path_stack.iter().fold(String::new(), |acc, vis| {
            if vis.is_public() {
                acc + "." + &vis.def_namespace[..]
            } else {
                acc + "::" + &vis.def_namespace[..]
            }
        })
    }

    // moveされた後の変数が使用されていないかチェックする
    // ProceduralでないメソッドでRefMutが使われているかはSideEffectCheckerでチェックする
    pub fn check(&mut self, hir: HIR) -> Result<HIR, (HIR, OwnershipErrors)> {
        log!(info "the ownership checking process has started.{RESET}");
        if self.full_path() != ("::".to_string() + &hir.name[..]) {
            self.path_stack.push(Visibility::private(hir.name.clone()));
            self.dict
                .insert(Str::from(self.full_path()), LocalVars::default());
        }
        for chunk in hir.module.iter() {
            self.check_expr(chunk, Ownership::Owned, true);
        }
        self.emit_cycle_errors();
        log!(
            "{DEBUG_MAIN}[DEBUG] the ownership checking process has completed, found errors: {}{RESET}",
            self.errs.len()
        );
        if self.errs.is_empty() {
            Ok(hir)
        } else {
            Err((hir, mem::take(&mut self.errs)))
        }
    }

    fn check_block(&mut self, block: &Block) {
        if block.len() == 1 {
            return self.check_expr(block.first().unwrap(), Ownership::Owned, false);
        }
        for chunk in block.iter() {
            self.check_expr(chunk, Ownership::Owned, true);
        }
    }

    fn check_expr(&mut self, expr: &Expr, ownership: Ownership, chunk: bool) {
        match expr {
            Expr::Def(def) => {
                self.define(def);
                let name = match &def.sig {
                    Signature::Var(var) => var.inspect().clone(),
                    Signature::Subr(subr) => subr.ident.inspect().clone(),
                    Signature::Glob(_) => Str::from("*"),
                };
                self.path_stack
                    .push(Visibility::new(def.sig.vis().clone(), name));
                self.capture_frames.push(vec![]);
                // a variable definition's body runs here and once; a subroutine's runs
                // where it is called, as many times as it is called
                let is_subr = def.sig.is_subr();
                if is_subr {
                    self.fn_scopes.push(self.path_stack.len());
                }
                self.dict
                    .insert(Str::from(self.full_path()), LocalVars::default());
                if let Signature::Subr(subr) = &def.sig {
                    let (nd_params, var_params, d_params, kw_var, _) =
                        subr.params.ref_deconstruct();
                    for param in nd_params {
                        if let ParamPattern::VarName(name) = &param.raw.pat {
                            self.define_param(name);
                        }
                    }
                    if let Some(var) = var_params {
                        if let ParamPattern::VarName(name) = &var.raw.pat {
                            self.define_param(name);
                        }
                    }
                    for param in d_params {
                        if let ParamPattern::VarName(name) = &param.sig.raw.pat {
                            self.define_param(name);
                        }
                    }
                    if let Some(kw_var) = kw_var {
                        if let ParamPattern::VarName(name) = &kw_var.raw.pat {
                            self.define_param(name);
                        }
                    }
                }
                self.check_block(&def.body.block);
                if is_subr {
                    self.fn_scopes.pop();
                }
                let captured = self.capture_frames.pop().unwrap_or_default();
                self.path_stack.pop();
                self.record_captures(def, captured);
            }
            Expr::ClassDef(class_def) => {
                if let Some(req_sup) = &class_def.require_or_sup {
                    self.check_expr(req_sup, Ownership::Owned, false);
                }
                for def in class_def.all_methods() {
                    self.check_expr(def, Ownership::Owned, true);
                }
            }
            Expr::PatchDef(patch_def) => {
                self.check_expr(&patch_def.base, Ownership::Owned, false);
                for def in patch_def.methods.iter() {
                    self.check_expr(def, Ownership::Owned, true);
                }
            }
            // Access in chunks does not drop variables (e.g., access in REPL)
            Expr::Accessor(acc) => self.check_acc(acc, ownership, chunk),
            Expr::Call(call) => {
                self.record_mut_store(call);
                // The callee, and the receiver of a method call, have to be alive. The
                // receiver used to go unchecked, so `y = x; x.push! 1` passed.
                // It is not moved by the call even when the method takes `self` by value:
                // the builtin and `.d.er` declarations write `self` by value for methods
                // that only borrow it (`Int!.inc!`, every Python method), so taking that
                // literally would move `i` on `i.inc!()`.
                self.check_expr(&call.obj, Ownership::Ref, false);
                let Some(sig_t) = call.signature_t() else {
                    return;
                };
                if !sig_t.is_subr() {
                    return;
                }
                let args_owns = sig_t.args_ownership();
                let non_defaults_len = if call.is_method_call() {
                    args_owns.non_defaults.len() - 1
                } else {
                    args_owns.non_defaults.len()
                };
                let defaults_len = args_owns.defaults.len();
                let (non_default_args, var_args) = call
                    .args
                    .pos_args
                    .split_at(usize::min(non_defaults_len, call.args.pos_args.len()));
                for (nd_arg, (_, ownership)) in
                    non_default_args.iter().zip(args_owns.non_defaults.iter())
                {
                    self.check_expr(&nd_arg.expr, *ownership, false);
                }
                if let Some((_, ownership)) = args_owns.var_params.as_ref() {
                    for var_arg in var_args.iter() {
                        self.check_expr(&var_arg.expr, *ownership, false);
                    }
                } else {
                    let kw_args = var_args;
                    for (arg, (_, ownership)) in kw_args.iter().zip(args_owns.defaults.iter()) {
                        self.check_expr(&arg.expr, *ownership, false);
                    }
                }
                let (default_args, kw_var_args) = call
                    .args
                    .kw_args
                    .split_at(usize::min(defaults_len, call.args.kw_args.len()));
                for kw_arg in default_args.iter() {
                    if let Some((_, ownership)) = args_owns
                        .defaults
                        .iter()
                        .find(|(k, _)| k == kw_arg.keyword.inspect())
                    {
                        self.check_expr(&kw_arg.expr, *ownership, false);
                    } else if let Some((_, ownership)) = args_owns
                        .non_defaults
                        .iter()
                        .find(|(k, _)| k.as_ref() == Some(kw_arg.keyword.inspect()))
                    {
                        self.check_expr(&kw_arg.expr, *ownership, false);
                    } else if let Some((_, ownership)) = args_owns.kw_var_params.as_ref() {
                        self.check_expr(&kw_arg.expr, *ownership, false);
                    } else {
                        todo!()
                    }
                }
                if let Some((_, ownership)) = args_owns.kw_var_params.as_ref() {
                    for var_arg in kw_var_args.iter() {
                        self.check_expr(&var_arg.expr, *ownership, false);
                    }
                } else {
                    let kw_args = kw_var_args;
                    for (arg, (_, ownership)) in kw_args.iter().zip(args_owns.defaults.iter()) {
                        self.check_expr(&arg.expr, *ownership, false);
                    }
                }
            }
            Expr::BinOp(binop) => {
                self.check_expr(&binop.lhs, Ownership::Ref, false);
                self.check_expr(&binop.rhs, Ownership::Ref, false);
            }
            Expr::UnaryOp(unary) => {
                self.check_expr(&unary.expr, Ownership::Ref, false);
            }
            Expr::List(list) => match list {
                List::Normal(lis) => {
                    for a in lis.elems.pos_args.iter() {
                        self.check_expr(&a.expr, ownership, false);
                    }
                }
                List::WithLength(lis) => {
                    self.check_expr(&lis.elem, ownership, false);
                    if let Some(len) = &lis.len {
                        self.check_expr(len, ownership, false);
                    }
                }
                _ => todo!(),
            },
            Expr::Tuple(tuple) => match tuple {
                Tuple::Normal(lis) => {
                    for a in lis.elems.pos_args.iter() {
                        self.check_expr(&a.expr, ownership, false);
                    }
                }
            },
            Expr::Dict(dict) => match dict {
                hir::Dict::Normal(dic) => {
                    for kv in dic.kvs.iter() {
                        self.check_expr(&kv.key, ownership, false);
                        self.check_expr(&kv.value, ownership, false);
                    }
                }
                other => todo!("{other}"),
            },
            Expr::Record(rec) => {
                for def in rec.attrs.iter() {
                    for chunk in def.body.block.iter() {
                        self.check_expr(chunk, ownership, false);
                    }
                }
            }
            Expr::Set(set) => match set {
                hir::Set::Normal(st) => {
                    for a in st.elems.pos_args.iter() {
                        self.check_expr(&a.expr, ownership, false);
                    }
                }
                hir::Set::WithLength(st) => {
                    self.check_expr(&st.elem, ownership, false);
                    self.check_expr(&st.len, ownership, false);
                }
            },
            Expr::Lambda(lambda) => {
                let name_and_vis =
                    Visibility::private(Str::from(format!("<lambda_{}>", lambda.id)));
                self.path_stack.push(name_and_vis);
                self.capture_frames.push(vec![]);
                self.fn_scopes.push(self.path_stack.len());
                self.dict
                    .insert(Str::from(self.full_path()), LocalVars::default());
                self.check_block(&lambda.body);
                self.fn_scopes.pop();
                // what an anonymous lambda captures is recorded on the closures around it
                self.capture_frames.pop();
                self.path_stack.pop();
            }
            Expr::TypeAsc(asc) => {
                self.check_expr(&asc.expr, ownership, chunk);
            }
            _ => {}
        }
    }

    fn check_acc(&mut self, acc: &Accessor, ownership: Ownership, chunk: bool) {
        match acc {
            Accessor::Ident(ident) => {
                if let Err(e) = self.check_if_dropped(ident.inspect(), ident) {
                    self.errs.push(e);
                    return;
                }
                if let Err(e) = self.check_captures_alive(ident) {
                    self.errs.push(e);
                    return;
                }
                if acc.ref_t().is_mut_type() && ownership.is_owned() && !chunk {
                    self.drop(ident);
                } else if acc.ref_t().is_mut_type() {
                    self.note_capture(ident);
                }
            }
            Accessor::Attr(attr) => {
                // `x.y` reads `x`, it does not take it. What a move takes here is the
                // attribute's value, and a whole variable is all this checker tracks.
                // The receiver's own ownership used to be passed down, so `root.tag`
                // moved `root` and the next `root.findall "b"` was a use of a moved
                // variable (`should_err/move.er` marked this as maybe too strict).
                self.check_expr(&attr.obj, Ownership::Ref, false)
            }
        }
    }

    /// TODO: このメソッドを呼ぶとき、スコープを再帰的に検索する
    #[inline]
    fn current_scope(&mut self) -> &mut LocalVars {
        self.dict.get_mut(&self.full_path()[..]).unwrap()
    }

    /// The path of the scope `n` entries above the one being checked (`0` is it).
    fn nth_outer_path(&self, n: usize) -> String {
        self.path_stack
            .iter()
            .take(self.path_stack.len() - n)
            .fold(String::new(), |acc, vis| {
                if vis.is_public() {
                    acc + "." + &vis.def_namespace[..]
                } else {
                    acc + "::" + &vis.def_namespace[..]
                }
            })
    }

    #[inline]
    fn nth_outer_scope(&mut self, n: usize) -> &mut LocalVars {
        let path = self.nth_outer_path(n);
        self.dict.get_mut(&path[..]).unwrap()
    }

    fn define(&mut self, def: &Def) {
        log!(info "define: {}", def.sig);
        match &def.sig {
            Signature::Var(sig) => {
                self.current_scope()
                    .alive_vars
                    .insert(sig.inspect().clone());
            }
            Signature::Subr(sig) => {
                self.current_scope()
                    .alive_vars
                    .insert(sig.ident.inspect().clone());
            }
            Signature::Glob(_) => {}
        }
    }

    fn define_param(&mut self, name: &VarName) {
        log!(info "define: {}", name);
        self.current_scope()
            .alive_vars
            .insert(name.inspect().clone());
    }

    fn drop(&mut self, ident: &Identifier) {
        log!("drop: {ident} (in {})", ident.ln_begin().unwrap_or(0));
        let innermost_fn = self.fn_scopes.last().copied().unwrap_or(0);
        let len = self.path_stack.len();
        for n in 0..len {
            if !self.nth_outer_scope(n).alive_vars.contains(ident.inspect()) {
                continue;
            }
            // A closure cannot take a variable of a scope outside it: it may run more
            // than once, and that scope goes on using the variable. `while! do! flg, ...`
            // reads `flg` on every iteration, and `flg.invert!()` follows the loop.
            // Only the closure's own body reads it that way, though: a definition
            // inside it (`y = x`) does take the variable, and that is still a move.
            if len == innermost_fn && len - n < innermost_fn {
                return self.note_capture(ident);
            }
            self.nth_outer_scope(n).alive_vars.remove(ident.inspect());
            self.nth_outer_scope(n)
                .dropped_vars
                .insert(ident.inspect().clone(), ident.loc());
            return;
        }
        panic!("variable not found: {ident}");
    }

    /// `ident` (a mutable variable that this access does not move) is being read in the
    /// scope being checked. If it is a variable of an enclosing scope, every closure
    /// between that scope and here captures it.
    fn note_capture(&mut self, ident: &Identifier) {
        let name = ident.inspect();
        if self.current_scope().alive_vars.contains(name) {
            return;
        }
        // the depth (index into `path_stack`) of the scope owning the variable
        let len = self.path_stack.len();
        let Some(owner_depth) = (1..len).map(|n| len - 1 - n).find(|&depth| {
            self.nth_outer_scope(len - 1 - depth)
                .alive_vars
                .contains(name)
        }) else {
            return;
        };
        let owner = Str::from(self.nth_outer_path(len - 1 - owner_depth));
        // `capture_frames[i]` is the frame of `path_stack[i + 1]`
        for frame in self.capture_frames[owner_depth..].iter_mut() {
            if !frame.iter().any(|(_, captured)| captured.inspect() == name) {
                frame.push((owner.clone(), ident.clone()));
            }
        }
    }

    /// Remember the mutable outer variables a closure captures (`f! = () => x.push! 1`,
    /// `f!() = x.push! 1`), so that using `f!` after `x` was moved is reported.
    /// A variable definition (`y = x`) is not a closure: it takes `x`, it does not read it later.
    fn record_captures(&mut self, def: &Def, captured: Vec<(Str, Identifier)>) {
        let is_closure = match &def.sig {
            Signature::Subr(_) => true,
            Signature::Var(_) => {
                def.body.block.len() == 1 && matches!(def.body.block.first(), Some(Expr::Lambda(_)))
            }
            Signature::Glob(_) => false,
        };
        if is_closure && !captured.is_empty() {
            self.captures
                .insert(Self::ident_key(def.sig.ident()), captured);
        }
    }

    /// A closure is used: every mutable variable it captured has to be alive.
    fn check_captures_alive(&mut self, closure: &Identifier) -> Result<(), OwnershipError> {
        // `ident_key` allocates, and all but a handful of the identifiers in a program
        // are not closures
        if self.captures.is_empty() {
            return Ok(());
        }
        let Some(captured) = self.captures.get(&Self::ident_key(closure)) else {
            return Ok(());
        };
        // the variable is looked up in the scope that owns it: a same-named variable of
        // the scope the closure is called from is a different variable
        for (owner, ident) in captured.clone() {
            let Some(moved_loc) = self
                .dict
                .get(&owner[..])
                .and_then(|vars| vars.dropped_vars.get(ident.inspect()))
                .copied()
            else {
                continue;
            };
            return Err(OwnershipError::move_error(
                self.cfg.input.clone(),
                line!() as usize,
                ident.inspect(),
                closure.loc(),
                moved_loc,
                self.full_path(),
            ));
        }
        Ok(())
    }

    fn check_if_dropped(
        &mut self,
        name: &Str,
        loc: &impl Locational,
    ) -> Result<(), OwnershipError> {
        for n in 0..self.path_stack.len() {
            if let Some(moved_loc) = self.nth_outer_scope(n).dropped_vars.get(name) {
                let moved_loc = *moved_loc;
                return Err(OwnershipError::move_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    name,
                    loc.loc(),
                    moved_loc,
                    self.full_path(),
                ));
            }
        }
        Ok(())
    }

    fn record_mut_store(&mut self, call: &Call) {
        let Some(attr) = call.attr_name.as_ref() else {
            return;
        };
        if !attr.is_procedural() {
            return;
        }
        let Some(from) = Self::cyclic_container_ident(&call.obj) else {
            return;
        };
        for arg in call.args.pos_args.iter() {
            if let Some(to) = Self::cyclic_container_ident(&arg.expr) {
                self.add_store_edge(from, to, call.loc());
            }
        }
        for arg in call.args.kw_args.iter() {
            if let Some(to) = Self::cyclic_container_ident(&arg.expr) {
                self.add_store_edge(from, to, call.loc());
            }
        }
    }

    fn cyclic_container_ident(expr: &Expr) -> Option<&Identifier> {
        match expr {
            Expr::Accessor(Accessor::Ident(ident)) if Self::can_form_ref_cycle(ident.ref_t()) => {
                Some(ident)
            }
            Expr::TypeAsc(tasc) => Self::cyclic_container_ident(&tasc.expr),
            _ => None,
        }
    }

    /// Mutable types that can hold other objects (not scalar cells such as `Int!`).
    fn can_form_ref_cycle(t: &Type) -> bool {
        if !t.is_mut_type() {
            return false;
        }
        !matches!(
            &t.local_name()[..],
            "Int!" | "Nat!" | "Bool!" | "Float!" | "Ratio!" | "Str!" | "Complex!"
        )
    }

    fn ident_key(ident: &Identifier) -> Str {
        Str::from(format!("{}#{}", ident.vi.def_loc, ident.inspect()))
    }

    fn add_store_edge(&mut self, from: &Identifier, to: &Identifier, loc: Location) {
        let from_key = Self::ident_key(from);
        let to_key = Self::ident_key(to);
        self.store_names
            .insert(from_key.clone(), from.inspect().clone());
        self.store_names
            .insert(to_key.clone(), to.inspect().clone());
        self.stores.entry(from_key).or_default().push((to_key, loc));
    }

    fn emit_cycle_errors(&mut self) {
        let graph = self.stores.clone();
        let names = self.store_names.clone();
        let mut color: Dict<Str, u8> = Dict::new();
        let mut stack: Vec<Str> = vec![];
        let mut reported: Set<Str> = Set::new();
        let mut errors = vec![];
        for start in graph.keys() {
            if color.get(start).copied().unwrap_or(0) == 0 {
                Self::dfs_cycles(
                    &graph,
                    &names,
                    start,
                    &mut color,
                    &mut stack,
                    &mut reported,
                    &mut errors,
                );
            }
        }
        for (loc, cycle) in errors {
            self.errs.push(OwnershipError::cycle_error(
                self.cfg.input.clone(),
                line!() as usize,
                loc,
                &cycle,
                self.full_path(),
            ));
        }
    }

    fn dfs_cycles(
        graph: &Dict<Str, Vec<(Str, Location)>>,
        names: &Dict<Str, Str>,
        node: &Str,
        color: &mut Dict<Str, u8>,
        stack: &mut Vec<Str>,
        reported: &mut Set<Str>,
        errors: &mut Vec<(Location, String)>,
    ) {
        color.insert(node.clone(), 1);
        stack.push(node.clone());
        if let Some(edges) = graph.get(node) {
            for (dest, loc) in edges.iter() {
                let dest_color = color.get(dest).copied().unwrap_or(0);
                if dest_color == 1 {
                    if let Some((key, cycle)) = Self::cycle_from_stack(stack, names, dest) {
                        if reported.insert(key) {
                            errors.push((*loc, cycle));
                        }
                    }
                } else if dest_color == 0 {
                    Self::dfs_cycles(graph, names, dest, color, stack, reported, errors);
                }
            }
        }
        stack.pop();
        color.insert(node.clone(), 2);
    }

    fn cycle_from_stack(
        stack: &[Str],
        names: &Dict<Str, Str>,
        dest: &Str,
    ) -> Option<(Str, String)> {
        let start = stack.iter().position(|n| n == dest)?;
        let nodes = &stack[start..];
        let mut key_parts: Vec<&str> = nodes.iter().map(|s| &s[..]).collect();
        key_parts.sort_unstable();
        let key = Str::from(key_parts.join("|"));
        let mut path: Vec<&str> = nodes
            .iter()
            .map(|k| names.get(k).map(|s| &s[..]).unwrap_or(&k[..]))
            .collect();
        path.push(names.get(dest).map(|s| &s[..]).unwrap_or(&dest[..]));
        Some((key, path.join(" → ")))
    }
}

impl Default for OwnershipChecker {
    fn default() -> Self {
        Self::new(ErgConfig::default())
    }
}
