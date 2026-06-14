//! Integration tests for `erg_linter`'s lint rules.
//!
//! Each test lints an in-memory Erg snippet and counts only the warnings whose
//! message matches the rule under test, so unrelated compiler warnings (e.g.
//! unused-variable) don't affect the assertions. Messages are matched against
//! the default (english) locale, following the repository's testing convention.

use erg_common::config::ErgConfig;
use erg_common::spawn::exec_new_thread;

use erg_linter::Linter;

/// Lints `src` and returns the number of emitted warnings whose main message
/// contains `needle`.
fn count_warns(src: &'static str, needle: &'static str) -> usize {
    exec_new_thread(
        move || {
            let mut linter = Linter::new(ErgConfig::default());
            let warns = linter
                .lint_from_string(src.to_string())
                .unwrap_or_else(|art| panic!("linting failed:\n{}", art.errors));
            warns
                .iter()
                .filter(|w| w.core.main_message.contains(needle))
                .count()
        },
        "lint_test",
    )
}

#[test]
fn contradiction_self_comparison() {
    let src = "\
x = 1
_ = x < x
_ = x > x
_ = x != x
";
    assert_eq!(count_warns(src, "always false"), 3);
}

#[test]
fn tautology_self_comparison() {
    let src = "\
x = 1
_ = x >= x
_ = x <= x
_ = x == x
";
    assert_eq!(count_warns(src, "comparison operator is verbose"), 3);
}

#[test]
fn magic_number_well_known_constants() {
    let src = "\
_ = 3.14
_ = 2.718
_ = 6.28
";
    assert_eq!(count_warns(src, "well-known constant"), 3);
}

#[test]
fn magic_number_ignores_ordinary_floats() {
    let src = "\
_ = 1.5
_ = 2.0
_ = 42.0
";
    assert_eq!(count_warns(src, "well-known constant"), 0);
}

#[test]
fn double_negation_is_redundant() {
    let src = "\
b = True
_ = not (not b)
";
    assert_eq!(count_warns(src, "double negation"), 1);
}

#[test]
fn single_negation_is_fine() {
    let src = "\
b = True
_ = not b
";
    assert_eq!(count_warns(src, "double negation"), 0);
}

#[test]
fn identity_op_is_redundant() {
    // x + 0, 0 + x, x - 0, x * 1, 1 * x
    let src = "\
x = 5
_ = x + 0
_ = 0 + x
_ = x - 0
_ = x * 1
_ = 1 * x
";
    assert_eq!(count_warns(src, "has no effect"), 5);
}

#[test]
fn identity_op_ignores_real_arithmetic() {
    let src = "\
x = 5
_ = x + 1
_ = x * 2
_ = x - 3
";
    assert_eq!(count_warns(src, "has no effect"), 0);
}

#[test]
fn erasing_and_modulo_one() {
    let src = "\
x = 5
_ = x * 0
_ = x % 1
";
    assert_eq!(count_warns(src, "always zero"), 2);
}

#[test]
fn erasing_op_ignores_non_numeric_repetition() {
    // `str * 0` is `""` and `list * 0` is `[]`, not `0`, so must not warn.
    let src = "\
s = \"ab\"
lis = [1, 2]
_ = s * 0
_ = lis * 0
";
    assert_eq!(count_warns(src, "always zero"), 0);
}

#[test]
fn needless_bool() {
    let src = "\
cond = True
_ = if cond, do(True), do(False)
_ = if cond, do(False), do(True)
";
    assert_eq!(count_warns(src, "returns a boolean literal"), 2);
}

#[test]
fn needless_bool_ignores_real_branches() {
    let src = "\
cond = True
_ = if cond, do(1), do(2)
";
    assert_eq!(count_warns(src, "returns a boolean literal"), 0);
}

#[test]
fn redundant_branches_are_identical() {
    let src = "\
cond = True
_ = if cond, do(42), do(42)
";
    assert_eq!(count_warns(src, "both branches are identical"), 1);
}

#[test]
fn absurd_nat_comparison_always_false() {
    // `n < 0` and `0 > n` can never hold for a Nat.
    let src = "\
chk n: Nat =
    _ = n < 0
    _ = 0 > n
    n
_ = chk 1
";
    assert_eq!(count_warns(src, "always false"), 2);
}

#[test]
fn absurd_nat_comparison_always_true() {
    // `n >= 0` and `0 <= n` always hold for a Nat.
    let src = "\
chk n: Nat =
    _ = n >= 0
    _ = 0 <= n
    n
_ = chk 1
";
    assert_eq!(count_warns(src, "always true"), 2);
}

#[test]
fn nat_comparison_ignores_non_absurd() {
    // `n > 0`, `n <= 0`, `n == 0` all depend on the value.
    let src = "\
chk n: Nat =
    _ = n > 0
    _ = n <= 0
    _ = n == 0
    n
_ = chk 1
";
    assert_eq!(count_warns(src, "always true"), 0);
    assert_eq!(count_warns(src, "always false"), 0);
}

#[test]
fn int_comparison_with_zero_is_fine() {
    // `Int` can be negative, so comparing with 0 is not absurd.
    let src = "\
chk i: Int =
    _ = i < 0
    _ = i >= 0
    i
_ = chk 1
";
    assert_eq!(count_warns(src, "always false"), 0);
    assert_eq!(count_warns(src, "always true"), 0);
}

#[test]
fn subr_param_shadows_builtin() {
    let src = "\
f map =
    map
_ = f 1
";
    assert_eq!(count_warns(src, "shadows a built-in"), 1);
}

#[test]
fn lambda_param_shadows_builtin() {
    let src = "\
apply = filter -> filter
_ = apply 5
";
    assert_eq!(count_warns(src, "shadows a built-in"), 1);
}

#[test]
fn ordinary_param_names_are_fine() {
    let src = "\
add x, y =
    x + y
_ = add 1, 2
";
    assert_eq!(count_warns(src, "shadows a built-in"), 0);
}

#[test]
fn module_level_shadowing_is_not_duplicated() {
    // The compiler already reports module-level shadowing as a `NameWarning`;
    // the linter must not add a second warning for it.
    let src = "\
map = 10
_ = map
";
    assert_eq!(count_warns(src, "shadows a built-in"), 0);
}

#[test]
fn unreachable_after_return() {
    let src = "\
g x: Int =
    g::return x
    log \"dead\"
    x + 1
_ = g 1
";
    assert_eq!(count_warns(src, "unreachable code"), 1);
}

#[test]
fn unreachable_after_panic() {
    let src = "\
h x: Int =
    panic \"boom\"
    x
_ = h 1
";
    assert_eq!(count_warns(src, "unreachable code"), 1);
}

#[test]
fn unreachable_at_module_level() {
    let src = "\
panic \"top\"
log \"dead\"
";
    assert_eq!(count_warns(src, "unreachable code"), 1);
}

#[test]
fn no_unreachable_without_divergence() {
    let src = "\
ok x: Int =
    log \"fine\"
    x
_ = ok 1
";
    assert_eq!(count_warns(src, "unreachable code"), 0);
}

#[test]
fn divergence_as_last_chunk_is_fine() {
    // A `return`/`panic` as the final chunk leaves nothing unreachable.
    let src = "\
h x: Int =
    log \"before\"
    h::return x
_ = h 1
";
    assert_eq!(count_warns(src, "unreachable code"), 0);
}

#[test]
fn effect_free_proc_is_flagged() {
    let src = "\
pure! x: Int =
    x + 1
_ = pure! 1
";
    assert_eq!(count_warns(src, "no side effects"), 1);
}

#[test]
fn proc_with_print_is_not_flagged() {
    let src = "\
io! x: Int =
    print! x
    x
_ = io! 1
";
    assert_eq!(count_warns(src, "no side effects"), 0);
}

#[test]
fn proc_calling_another_proc_is_not_flagged() {
    // Calling a procedure is itself an effect, even if that procedure is pure.
    let src = "\
helper! y: Int =
    y
caller! x: Int =
    helper! x
_ = caller! 1
";
    // `helper!` is pure (flagged); `caller!` calls a proc (not flagged).
    assert_eq!(count_warns(src, "no side effects"), 1);
}

#[test]
fn plain_function_is_not_flagged() {
    let src = "\
plain x: Int =
    x + 1
_ = plain 1
";
    assert_eq!(count_warns(src, "no side effects"), 0);
}
