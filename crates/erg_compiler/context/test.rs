//! test module for `Context`
use erg_common::traits::StructuralEq;
use erg_common::Str;

use crate::ty::constructors::{func1, mono, mono_q, poly, refinement, ty_tp};
use crate::ty::free::Constraint;
use crate::ty::typaram::TyParam;
use crate::ty::{Predicate, Type};
use Type::*;

use crate::context::Context;

impl Context {
    pub fn assert_var_type(&self, varname: &str, ty: &Type) -> Result<(), ()> {
        let Some((_, vi)) = self.get_var_info(varname) else {
            panic!("variable not found: {varname}");
        };
        println!("{varname}: {}", vi.t);
        if vi.t.structural_eq(ty) {
            Ok(())
        } else {
            println!("{varname} is not the type of {ty}");
            Err(())
        }
    }

    pub fn assert_attr_type(&self, receiver_t: &Type, attr: &str, ty: &Type) -> Result<(), ()> {
        let Some(ctx) = self.get_nominal_type_ctx(receiver_t) else {
            panic!("type not found: {receiver_t}");
        };
        let Some((_, vi)) = ctx.get_method_kv(attr) else {
            panic!("attribute not found: {attr}");
        };
        println!("{attr}: {}", vi.t);
        if vi.t.structural_eq(ty) {
            Ok(())
        } else {
            println!("{attr} is not the type of {ty}");
            Err(())
        }
    }

    pub fn test_refinement_subtyping(&self) -> Result<(), ()> {
        // Nat :> {I: Int | I >= 1} ?
        let lhs = Nat;
        let var = Str::ever("I");
        let rhs = refinement(
            var.clone(),
            Type::Int,
            Predicate::eq(var, TyParam::value(1)),
        );
        if self.supertype_of(&lhs, &rhs) {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn test_quant_subtyping(&self) -> Result<(), ()> {
        let t = crate::ty::constructors::type_q("T");
        let quant = func1(t.clone(), t).quantify();
        let subr = func1(Obj, Never);
        assert!(!self.subtype_of(&quant, &subr));
        assert!(self.subtype_of(&subr, &quant));
        Ok(())
    }

    pub fn test_resolve_trait_inner1(&self) -> Result<(), ()> {
        let name = Str::ever("Add");
        let params = vec![TyParam::t(Nat)];
        let maybe_trait = poly(name, params);
        let mut min = Type::Obj;
        for pair in self.get_trait_impls(&maybe_trait) {
            if self.supertype_of(&pair.sup_trait, &maybe_trait) {
                min = self.min(&min, &pair.sub_type).unwrap_or(&min).clone();
            }
        }
        if min == Nat {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn test_instantiation_and_generalization(&self) -> Result<(), ()> {
        use crate::ty::free::HasLevel;
        let t = mono_q("T", Constraint::new_subtype_of(mono("Eq")));
        let unbound = func1(t.clone(), t);
        let quantified = unbound.clone().quantify();
        println!("quantified      : {quantified}");
        let inst = self.instantiate_def_type(&unbound).map_err(|_| ())?;
        println!("inst: {inst}");
        inst.lift();
        let quantified_again = self.generalize_t(inst);
        println!("quantified_again: {quantified_again}");
        assert!(quantified.structural_eq(&quantified_again));
        Ok(())
    }

    pub fn test_patch(&self) -> Result<(), ()> {
        use crate::ty::constructors::or;
        assert!(self.subtype_of(&Int, &mono("Eq")));
        assert!(self.subtype_of(&or(Int, NoneType), &mono("Eq")));
        assert!(!self.subtype_of(&or(Int, Float), &mono("Eq")));
        assert!(self.subtype_of(&Int, &poly("Sub", vec![ty_tp(Int)])));
        assert!(self.subtype_of(&Nat, &poly("Sub", vec![ty_tp(Nat)])));
        Ok(())
    }

    pub fn test_intersection(&self) -> Result<(), ()> {
        assert!(self.subtype_of(&Code, &(Int | Str | Code | NoneType)));
        assert!(self.subtype_of(&(Int | Str), &(Int | Str | Code | NoneType)));
        Ok(())
    }
}
