mod common;
use common::{
    expect_compile_failure, expect_compile_success, expect_end_with, expect_error_location_and_msg,
    expect_success,
};
use erg_common::error::Location;
use erg_common::python_util::env_python_version;

#[test]
fn exec_addition_ok() -> Result<(), ()> {
    expect_success("tests/should_ok/addition.er", 0)
}

#[test]
fn exec_advanced_type_spec() -> Result<(), ()> {
    expect_success("tests/should_ok/advanced_type_spec.er", 5)
}

#[test]
fn exec_and() -> Result<(), ()> {
    expect_success("tests/should_ok/and.er", 0)
}

#[test]
fn exec_args_expansion() -> Result<(), ()> {
    expect_success("tests/should_ok/args_expansion.er", 0)
}

#[test]
fn exec_list_test() -> Result<(), ()> {
    expect_success("tests/should_ok/list.er", 0)
}

#[test]
fn exec_list_member() -> Result<(), ()> {
    expect_success("tests/should_ok/list_member.er", 0)
}

#[test]
fn exec_assert_cast_ok() -> Result<(), ()> {
    expect_success("tests/should_ok/assert_cast.er", 0)
}

#[test]
fn exec_associated_types() -> Result<(), ()> {
    expect_success("tests/should_ok/associated_types.er", 0)
}

#[test]
fn exec_bump_version() -> Result<(), ()> {
    expect_compile_success("bump_version.er", 0)
}

#[test]
fn exec_class() -> Result<(), ()> {
    expect_success("examples/class.er", 0)
}

#[test]
fn exec_class_test() -> Result<(), ()> {
    expect_success("tests/should_ok/class.er", 0)
}

#[test]
fn exec_class_attr() -> Result<(), ()> {
    expect_success("tests/should_ok/class_attr.er", 2)
}

#[test]
fn exec_closure() -> Result<(), ()> {
    expect_success("tests/should_ok/closure.er", 0)
}

#[test]
fn exec_coercion() -> Result<(), ()> {
    expect_success("tests/should_ok/coercion.er", 0)
}

#[test]
fn exec_context_manager() -> Result<(), ()> {
    expect_success("tests/should_ok/context_manager.er", 1)
}

#[test]
fn exec_collection() -> Result<(), ()> {
    expect_success("tests/should_ok/collection.er", 0)
}

#[test]
fn exec_comment() -> Result<(), ()> {
    expect_success("tests/should_ok/comment.er", 0)
}

#[test]
fn exec_comprehension() -> Result<(), ()> {
    expect_success("tests/should_ok/comprehension.er", 0)
}

#[test]
fn exec_comptime() -> Result<(), ()> {
    expect_success("tests/should_ok/comptime.er", 12)
}

#[test]
fn exec_poly_class() -> Result<(), ()> {
    expect_success("tests/should_ok/poly_class.er", 2)
}

#[test]
fn exec_poly_class_full() -> Result<(), ()> {
    expect_success("tests/should_ok/poly_class_full.er", 1)
}

#[test]
fn exec_poly_trait() -> Result<(), ()> {
    expect_success("tests/should_ok/poly_trait.er", 1)
}

#[test]
fn exec_container_class() -> Result<(), ()> {
    expect_success("tests/should_ok/container_class.er", 0)
}

#[test]
fn exec_control() -> Result<(), ()> {
    expect_success("examples/control.er", 2)
}

#[test]
fn exec_control_expr() -> Result<(), ()> {
    expect_success("tests/should_ok/control_expr.er", 3)
}

#[test]
fn exec_decimal() -> Result<(), ()> {
    expect_success("tests/should_ok/decimal.er", 0)
}

#[test]
fn exec_decl() -> Result<(), ()> {
    expect_success("tests/should_ok/decl.er", 1)
}

#[test]
fn exec_default_param() -> Result<(), ()> {
    expect_success("tests/should_ok/default_param.er", 0)
}

#[test]
fn exec_dependent() -> Result<(), ()> {
    expect_success("tests/should_ok/dependent.er", 0)
}

#[test]
fn exec_dependent_refinement() -> Result<(), ()> {
    expect_compile_success("tests/should_ok/dependent_refinement.er", 0)
}

#[test]
fn exec_dict() -> Result<(), ()> {
    expect_success("examples/dict.er", 0)
}

#[test]
fn exec_dict_test() -> Result<(), ()> {
    expect_success("tests/should_ok/dict.er", 0)
}

#[test]
fn exec_dunder() -> Result<(), ()> {
    expect_success("tests/should_ok/dunder.er", 0)
}

#[test]
fn exec_empty_check() -> Result<(), ()> {
    expect_success("tests/should_ok/dyn_type_check.er", 0)
}

#[test]
fn exec_use_ansicolor() -> Result<(), ()> {
    expect_success("examples/use_ansicolor.er", 0)
}

#[test]
fn exec_use_exception() -> Result<(), ()> {
    expect_success("examples/use_exception.er", 0)
}

#[test]
fn exec_fast_value() -> Result<(), ()> {
    expect_success("tests/should_ok/fast_value.er", 0)
}

#[test]
fn exec_fib() -> Result<(), ()> {
    expect_success("examples/fib.er", 0)
}

#[test]
fn exec_glue_patch() -> Result<(), ()> {
    // TODO: `expect_success` once trait-polymorphic calls (e.g. `f|T <: Reverse| x: T = x.rev()`)
    // can dispatch to glue patch methods at runtime
    expect_compile_success("tests/should_ok/glue_patch.er", 0)
}

#[test]
fn exec_helloworld() -> Result<(), ()> {
    // HACK: When running the test with Windows, the exit code is 1 (the cause is unknown)
    if cfg!(windows) && env_python_version().unwrap().minor >= Some(8) {
        expect_end_with("examples/helloworld.er", 0, 1)
    } else {
        expect_success("examples/helloworld.er", 0)
    }
}

#[test]
fn exec_if() -> Result<(), ()> {
    expect_success("tests/should_ok/if.er", 0)
}

#[test]
fn exec_impl() -> Result<(), ()> {
    expect_success("examples/impl.er", 0)
}

#[test]
fn exec_import() -> Result<(), ()> {
    // 2 warns: a11y
    expect_success("examples/import.er", 2)
}

#[test]
fn exec_import_cyclic() -> Result<(), ()> {
    expect_success("tests/should_ok/cyclic/import.er", 0)
}

#[test]
fn exec_import_sugar() -> Result<(), ()> {
    expect_success("tests/should_ok/import_sugar/import_sugar.er", 0)
}

#[test]
fn exec_index() -> Result<(), ()> {
    expect_success("tests/should_ok/index.er", 0)
}

#[test]
fn exec_inherit() -> Result<(), ()> {
    expect_success("tests/should_ok/inherit.er", 0)
}

#[test]
fn exec_infer_class() -> Result<(), ()> {
    expect_success("tests/should_ok/infer_class.er", 0)
}

#[test]
fn exec_infer_method() -> Result<(), ()> {
    expect_success("tests/should_ok/infer_method.er", 0)
}

#[test]
fn exec_infer_trait() -> Result<(), ()> {
    expect_success("tests/should_ok/infer_trait.er", 0)
}

#[test]
fn exec_init_del() -> Result<(), ()> {
    expect_success("examples/init_del.er", 0)
}

#[test]
fn exec_int() -> Result<(), ()> {
    expect_success("tests/should_ok/int.er", 0)
}

#[test]
fn exec_interpolation() -> Result<(), ()> {
    expect_success("tests/should_ok/interpolation.er", 0)
}

#[test]
fn exec_iterator() -> Result<(), ()> {
    expect_success("examples/iterator.er", 0)
}

#[test]
fn exec_iterator_test() -> Result<(), ()> {
    expect_success("tests/should_ok/iterator.er", 0)
}

#[test]
fn exec_long() -> Result<(), ()> {
    expect_success("tests/should_ok/long.er", 257)
}

#[test]
fn exec_magic() -> Result<(), ()> {
    expect_success("examples/magic.er", 0)
}

#[test]
fn exec_mangling() -> Result<(), ()> {
    expect_success("tests/should_ok/mangling.er", 0)
}

#[test]
fn exec_many_import() -> Result<(), ()> {
    expect_success("tests/should_ok/many_import/many_import.er", 0)
}

#[test]
fn exec_map() -> Result<(), ()> {
    expect_success("tests/should_ok/map.er", 0)
}

#[test]
fn exec_match() -> Result<(), ()> {
    expect_success("tests/should_ok/match.er", 0)
}

#[test]
fn exec_method() -> Result<(), ()> {
    expect_success("tests/should_ok/method.er", 0)
}

#[test]
fn exec_move() -> Result<(), ()> {
    expect_success("tests/should_ok/move.er", 0)
}

#[test]
fn exec_mut() -> Result<(), ()> {
    expect_success("examples/mut.er", 0)
}

#[test]
fn exec_mut_type() -> Result<(), ()> {
    expect_success("tests/should_ok/mut_type.er", 0)
}

#[test]
fn exec_cycle_ref() -> Result<(), ()> {
    expect_success("tests/should_ok/cycle_ref.er", 0)
}

#[test]
fn exec_mutizable() -> Result<(), ()> {
    expect_success("tests/should_ok/mutizable.er", 0)
}

#[test]
fn exec_mut_list() -> Result<(), ()> {
    expect_success("tests/should_ok/mut_list.er", 0)
}

#[test]
fn exec_mut_dict() -> Result<(), ()> {
    expect_success("tests/should_ok/mut_dict.er", 0)
}

#[test]
fn exec_mylist() -> Result<(), ()> {
    expect_success("examples/mylist.er", 0)
}

#[test]
fn exec_nested() -> Result<(), ()> {
    expect_success("tests/should_ok/nested.er", 3)
}

#[test]
fn exec_never() -> Result<(), ()> {
    expect_success("tests/should_ok/never.er", 0)
}

#[test]
fn exec_operators() -> Result<(), ()> {
    expect_success("tests/should_ok/operators.er", 0)
}

#[test]
fn exec_either() -> Result<(), ()> {
    expect_success("tests/should_ok/either.er", 0)
}

#[test]
fn exec_option_mut() -> Result<(), ()> {
    expect_success("tests/should_ok/option_mut.er", 0)
}

#[test]
fn exec_option_result() -> Result<(), ()> {
    expect_success("tests/should_ok/option_result.er", 0)
}

#[test]
fn exec_or() -> Result<(), ()> {
    expect_success("tests/should_ok/or.er", 0)
}

#[test]
fn exec_panic_type() -> Result<(), ()> {
    expect_success("tests/should_ok/panic_type.er", 0)
}

#[test]
fn exec_patch() -> Result<(), ()> {
    expect_success("examples/patch.er", 0)
}

#[test]
fn exec_pattern() -> Result<(), ()> {
    expect_success("tests/should_ok/pattern.er", 0)
}

#[test]
fn exec_poly_type_spec() -> Result<(), ()> {
    expect_success("tests/should_ok/poly_type_spec.er", 0)
}

#[test]
fn exec_comptime_type_func() -> Result<(), ()> {
    expect_success("tests/should_ok/comptime_type_func.er", 0)
}

#[test]
fn exec_const_func() -> Result<(), ()> {
    expect_success("tests/should_ok/const_func.er", 0)
}

#[test]
fn exec_const_op() -> Result<(), ()> {
    expect_success("tests/should_ok/const_op.er", 0)
}

#[test]
fn exec_const_recursive() -> Result<(), ()> {
    expect_success("tests/should_ok/const_recursive.er", 0)
}

#[test]
fn exec_const_method() -> Result<(), ()> {
    expect_success("tests/should_ok/const_method.er", 0)
}

#[test]
fn exec_const_ratio() -> Result<(), ()> {
    expect_success("tests/should_ok/const_ratio.er", 0)
}

#[test]
fn exec_const_kw_args() -> Result<(), ()> {
    // not executed: CPython's `abs`/`len`/... reject keyword arguments
    expect_compile_success("tests/should_ok/const_kw_args.er", 17)
}

#[test]
fn exec_const_wide_int() -> Result<(), ()> {
    expect_success("tests/should_ok/const_wide_int.er", 0)
}

#[test]
fn exec_recursive_multi_arm() -> Result<(), ()> {
    expect_success("tests/should_ok/recursive_multi_arm.er", 0)
}

#[test]
fn exec_refinement_arith() -> Result<(), ()> {
    expect_success("tests/should_ok/refinement_arith.er", 0)
}

#[test]
fn exec_pyimport_test() -> Result<(), ()> {
    // HACK: When running the test with Windows, the exit code is 1 (the cause is unknown)
    if cfg!(windows) && env_python_version().unwrap().minor < Some(8) {
        expect_end_with("tests/should_ok/pyimport.er", 2, 1)
    } else {
        expect_success("tests/should_ok/pyimport.er", 2)
    }
}

/// Type-checks a representative sample of the `lib/pystd` declaration files.
/// Compile-only: the point is to validate the `.d.er` declarations themselves,
/// not to run the Python modules they describe.
#[test]
fn exec_pystd_decls() -> Result<(), ()> {
    expect_compile_success("tests/should_ok/pystd_decls.er", 0)
}

#[test]
fn exec_builtin_modules() -> Result<(), ()> {
    expect_success("tests/should_ok/builtin_modules.er", 0)
}

#[test]
fn exec_quantified() -> Result<(), ()> {
    expect_success("examples/quantified.er", 1)
}

#[test]
fn exec_type_app() -> Result<(), ()> {
    expect_success("tests/should_ok/type_app.er", 0)
}

#[test]
fn exec_raw_ident() -> Result<(), ()> {
    expect_success("examples/raw_ident.er", 1)
}

#[test]
fn exec_float() -> Result<(), ()> {
    expect_success("tests/should_ok/float.er", 0)
}

#[test]
fn exec_num_compare() -> Result<(), ()> {
    expect_success("tests/should_ok/num_compare.er", 0)
}

#[test]
fn exec_ratio() -> Result<(), ()> {
    expect_success("tests/should_ok/ratio.er", 0)
}

#[test]
fn exec_rec() -> Result<(), ()> {
    expect_success("tests/should_ok/rec.er", 0)
}

#[test]
fn exec_record() -> Result<(), ()> {
    expect_success("examples/record.er", 0)
}

#[test]
fn exec_record_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/record.er", 0, 2)
}

#[test]
fn exec_recursive_class() -> Result<(), ()> {
    expect_success("tests/should_ok/recursive_class.er", 0)
}

#[test]
fn exec_refinement() -> Result<(), ()> {
    expect_success("tests/should_ok/refinement.er", 0)
}

#[test]
fn exec_refinement_class() -> Result<(), ()> {
    expect_success("tests/should_ok/refinement_class.er", 0)
}

#[test]
fn exec_return() -> Result<(), ()> {
    expect_success("tests/should_ok/return.er", 0)
}

#[test]
fn exec_self_reference() -> Result<(), ()> {
    expect_success("tests/should_ok/self_reference.er", 0)
}

#[test]
fn exec_self_type() -> Result<(), ()> {
    expect_success("tests/should_ok/self_type.er", 0)
}

#[test]
fn exec_set_type() -> Result<(), ()> {
    expect_success("tests/should_ok/set_type.er", 0)
}

#[test]
fn exec_slice() -> Result<(), ()> {
    expect_success("tests/should_ok/slice.er", 0)
}

#[test]
fn exec_star_expr() -> Result<(), ()> {
    expect_success("tests/should_ok/star_expr.er", 0)
}

#[test]
fn exec_structural_example() -> Result<(), ()> {
    expect_success("examples/structural.er", 0)
}

#[test]
fn exec_structural() -> Result<(), ()> {
    expect_success("tests/should_ok/structural.er", 0)
}

#[test]
fn exec_structural_alias() -> Result<(), ()> {
    expect_success("tests/should_ok/structural_alias.er", 0)
}

#[test]
fn exec_structural_disambiguation() -> Result<(), ()> {
    expect_success("tests/should_ok/structural_disambiguation.er", 0)
}

#[test]
fn exec_structural_trait() -> Result<(), ()> {
    expect_success("tests/should_ok/structural_trait.er", 0)
}

#[test]
fn exec_structural_subtyping() -> Result<(), ()> {
    expect_success("tests/should_ok/structural_subtyping.er", 0)
}

#[test]
fn exec_trait_op_requirement() -> Result<(), ()> {
    expect_success("tests/should_ok/trait_op_requirement.er", 0)
}

#[test]
fn exec_subtyping() -> Result<(), ()> {
    expect_success("tests/should_ok/subtyping.er", 0)
}

#[test]
fn exec_sym_op() -> Result<(), ()> {
    expect_success("tests/should_ok/sym_op.er", 0)
}

#[test]
fn exec_trait() -> Result<(), ()> {
    expect_success("examples/trait.er", 0)
}

#[test]
fn exec_tuple() -> Result<(), ()> {
    expect_success("examples/tuple.er", 0)
}

#[test]
fn exec_tuple_test() -> Result<(), ()> {
    expect_success("tests/should_ok/tuple.er", 0)
}

#[test]
fn exec_try_op() -> Result<(), ()> {
    expect_success("tests/should_ok/try_op.er", 1)
}

#[test]
fn exec_try_op_panic() -> Result<(), ()> {
    expect_end_with("tests/should_ok/try_op_panic.er", 0, 1)
}

#[test]
fn exec_unwrap() -> Result<(), ()> {
    expect_success("tests/should_ok/unwrap.er", 0)
}

#[test]
fn exec_unwrap_panic() -> Result<(), ()> {
    expect_end_with("tests/should_ok/unwrap_panic.er", 0, 1)
}

#[test]
fn exec_use_unit() -> Result<(), ()> {
    expect_success("examples/use_unit.er", 0)
}

#[test]
fn exec_unit_test() -> Result<(), ()> {
    expect_success("examples/unit_test.er", 0)
}

#[test]
fn exec_unpack() -> Result<(), ()> {
    expect_success("examples/unpack.er", 0)
}

#[test]
fn exec_unused_import() -> Result<(), ()> {
    expect_success("tests/should_ok/many_import/unused_import.er", 2)
}

#[test]
fn exec_use_itertools() -> Result<(), ()> {
    expect_success("tests/should_ok/use_itertools.er", 0)
}

#[test]
fn exec_use_py() -> Result<(), ()> {
    expect_success("examples/use_py.er", 0)
}

#[test]
fn exec_var_args() -> Result<(), ()> {
    expect_success("tests/should_ok/var_args.er", 0)
}

#[test]
fn exec_var_kwargs() -> Result<(), ()> {
    expect_success("tests/should_ok/var_kwargs.er", 0)
}

#[test]
fn exec_with() -> Result<(), ()> {
    expect_success("examples/with.er", 0)
}

#[test]
fn exec_addition_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/addition.er", 3, 9)
}

#[test]
fn exec_advanced_type_spec_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/advanced_type_spec.er", 0, 1)
}

#[test]
fn exec_either_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/either.er", 0, 5)
}

#[test]
fn exec_option_mut_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/option_mut.er", 0, 3)
}

#[test]
fn exec_option_result_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/option_result.er", 0, 4)
}

#[test]
fn exec_panic_type_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/panic_type.er", 0, 3)
}

#[test]
fn exec_args() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/args.er", 0, 20)
}

#[test]
fn exec_args_expansion_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/args_expansion.er", 0, 4)
}

#[test]
fn exec_list_err() -> Result<(), ()> {
    expect_compile_failure("examples/list.er", 0, 1)
}

#[test]
fn exec_list_member_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/list_member.er", 0, 3)
}

#[test]
fn exec_and_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/and.er", 0, 1)
}

#[test]
fn exec_as() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/as.er", 0, 6)
}

#[test]
fn exec_assert_cast() -> Result<(), ()> {
    expect_compile_failure("examples/assert_cast.er", 0, 3)
}

#[test]
fn exec_assert_cast_err() -> Result<(), ()> {
    expect_end_with("tests/should_err/assert_cast.er", 0, 1)
}

#[test]
fn exec_class_attr_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/class_attr.er", 1, 2)
}

#[test]
fn exec_coercion_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/coercion.er", 0, 1)
}

#[test]
fn exec_collection_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/collection.er", 0, 5)
}

#[test]
fn exec_cyclic_type_def_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/cyclic_type_def.er", 0, 5)
}

#[test]
fn exec_decl_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/decl.er", 1, 2)
}

#[test]
fn exec_default_param_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/default_param.er", 0, 8)
}

#[test]
fn exec_dependent_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/dependent.er", 0, 5)
}

#[test]
fn exec_dict_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/dict.er", 0, 5)
}

#[test]
fn exec_err_import() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/err_import.er", 0, 9)
}

#[test]
fn exec_glue_patch_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/glue_patch.er", 0, 2)
}

/// This file compiles successfully, but causes a run-time error due to incomplete method dispatching
#[test]
fn exec_tests_impl() -> Result<(), ()> {
    expect_end_with("tests/should_ok/impl.er", 0, 1)
}

#[test]
fn exec_impl_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/impl.er", 2, 2)
}

#[test]
fn exec_import_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/import.er", 0, 2)
}

#[test]
fn exec_import_cyclic_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/cyclic/import.er", 0, 1)
}

#[test]
fn exec_incomplete_typespec() -> Result<(), ()> {
    // TODO: errs: 2
    expect_compile_failure("tests/should_err/incomplete_typespec.er", 0, 3)
}

#[test]
fn exec_infer_fn_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/infer_fn.er", 2, 5)
}

#[test]
fn exec_infer_union_array() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/infer_union_array.er", 2, 1)
}

#[test]
fn exec_inherit_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/inherit.er", 0, 1)
}

#[test]
fn exec_init_del_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/init_del.er", 0, 1)
}

#[test]
fn exec_invalid_interpol() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/invalid_interpol.er", 0, 2)
}

#[test]
fn exec_invalid_param() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/invalid_param.er", 0, 3)
}

#[test]
fn exec_move_check() -> Result<(), ()> {
    expect_compile_failure("examples/move_check.er", 1, 1)
}

#[test]
fn exec_or_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/or.er", 0, 1)
}

#[test]
fn exec_poly_trait_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/poly_trait.er", 0, 3)
}

#[test]
fn exec_poly_type_spec_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/poly_type_spec.er", 0, 3)
}

#[test]
fn exec_pyimport() -> Result<(), ()> {
    if cfg!(unix) {
        expect_end_with("examples/pyimport.er", 8, 111)
    } else {
        expect_compile_failure("examples/pyimport.er", 8, 1)
    }
}

#[test]
fn exec_pyimport_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/pyimport.er", 0, 2)
}

#[test]
fn exec_recursive_const_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/recursive_const.er", 0, 3)
}

#[test]
fn exec_const_nonconst_call_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/const_nonconst_call.er", 0, 1)
}

#[test]
fn exec_const_op_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/const_op.er", 0, 4)
}

#[test]
fn exec_set() -> Result<(), ()> {
    expect_compile_failure("examples/set.er", 3, 1)
}

#[test]
fn exec_set_type_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/set_type.er", 0, 3)
}

#[test]
fn exec_side_effect() -> Result<(), ()> {
    expect_compile_failure("examples/side_effect.er", 5, 4)
}

#[test]
fn exec_side_effect_test() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/side_effect.er", 6, 6)
}

#[test]
fn exec_structural_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/structural.er", 1, 11)
}

#[test]
fn exec_structural_ambiguity_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/structural_ambiguity.er", 0, 1)
}

#[test]
fn exec_structural_trait_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/structural_trait.er", 0, 5)
}

#[test]
fn exec_structural_subtyping_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/structural_subtyping.er", 0, 3)
}

#[test]
fn exec_subtyping_err() -> Result<(), ()> {
    // NOTE: The content of some errors is semantically redundant and can be reduced.
    expect_compile_failure("tests/should_err/subtyping.er", 3, 15)
}

#[test]
fn exec_tuple_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/tuple.er", 0, 2)
}

#[test]
fn exec_trait_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/trait.er", 1, 1)
}

#[test]
fn exec_try_op_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/try_op.er", 0, 4)
}

#[test]
fn exec_unwrap_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/unwrap.er", 0, 5)
}

#[test]
fn exec_callable() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/callable.er", 0, 5)
}

#[test]
fn exec_method_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/method.er", 0, 4)
}

#[test]
fn exec_move_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/move.er", 1, 2)
}

#[test]
fn exec_multiline_invalid_next() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/multi_line_invalid_nest.er", 0, 1)
}

#[test]
fn exec_mut_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/mut.er", 0, 1)
}

#[test]
fn exec_mut_type_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/mut_type.er", 0, 3)
}

#[test]
fn exec_mut_class_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/mut_class.er", 0, 4)
}

#[test]
fn exec_cycle_ref_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/cycle_ref.er", 0, 2)
}

#[test]
fn exec_mut_list_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/mut_list.er", 0, 7)
}

#[test]
fn exec_mut_dict_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/mut_dict.er", 1, 5)
}

#[test]
fn exec_quantified_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/quantified.er", 0, 3)
}

#[test]
fn exec_type_app_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/type_app.er", 0, 7)
}

#[test]
fn exec_recursive_fn_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/recursive_fn.er", 0, 2)
}

#[test]
fn exec_refinement_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/refinement.er", 0, 13)
}

#[test]
fn exec_refinement_class_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/refinement_class.er", 0, 2)
}

#[test]
fn exec_use_itertools_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/use_itertools.er", 0, 1)
}

#[test]
fn exec_use_unit_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/use_unit.er", 0, 4)
}

#[test]
fn exec_var_args_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/var_args.er", 0, 4)
}

#[test]
fn exec_var_kwargs_err() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/var_kwargs.er", 0, 2)
}

#[test]
fn exec_visibility() -> Result<(), ()> {
    expect_compile_failure("tests/should_err/visibility.er", 2, 7)
}

#[test]
fn exec_err_loc() -> Result<(), ()> {
    expect_error_location_and_msg(
        "tests/should_err/err_loc.er",
        vec![
            (Location::range(2, 11, 2, 16), None),
            (Location::range(7, 11, 7, 12), None),
            (
                Location::range(13, 21, 13, 27),
                Some("Int object has no attribute method"),
            ),
            (Location::range(10, 11, 10, 16), None),
        ],
    )
}

#[test]
fn test_semver() -> Result<(), ()> {
    expect_success("crates/erg_compiler/lib/std/semver.er", 0)
}
