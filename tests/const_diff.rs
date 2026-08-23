//! Differential test for compile-time evaluation.
//!
//! The value the compiler folds at compile time becomes a singleton type, so it
//! has to equal the value the emitted program produces at run time -- otherwise
//! the type system states something the program does not do.
//!
//! Erg cannot share its evaluator with the runtime (the runtime is CPython), so
//! this is the only way to catch a divergence. Each expression below is compiled
//! once (to read the folded value out of the HIR) and executed once (to read the
//! run-time value), and the two are compared.

use std::env::temp_dir;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Definitions both generated programs need.
const PRELUDE: &str = "Dbl(N: Nat): Nat = N * 2\nOdd(N: Nat): Bool = N % 2 == 1\n";

/// Printed before the values, so warnings (which the runner also writes to
/// stdout) can be skipped.
const MARKER: &str = "<<<erg_const_diff>>>";

/// Expressions whose folded value must equal their run-time value.
const CASES: &[&str] = &[
    // arithmetic
    "1 + 1",
    "7 - 3",
    "3 * 4",
    "-7 // 2",
    "7 // -2",
    "-7 % 2",
    "7 % -2",
    "2 ** 10",
    // beyond `Int` (an i32), but within `Nat`
    "10000000000 - 1",
    "5000000000 + 5000000000",
    "100000 * 100000",
    "10000000000 // 3",
    "10000000000 % 3",
    "2 ** 32",
    // bitwise
    "1 << 3",
    "1 << 40",
    "16 >> 2",
    "-8 >> 1",
    "~5",
    "~(-1)",
    "~True",
    // unary
    "+3",
    "-3",
    // comparison / logic
    "1 < 2",
    "1 == 1",
    "\"a\" == \"a\"",
    "True and False",
    "True or True",
    "not True",
    // containers
    "\"a\" + \"b\"",
    "\"ab\" * 3",
    "[1] + [2]",
    // `in` (both as an operator and against every interval kind)
    "1 in [1, 2]",
    "1 in {1: 2}",
    "1 in {1, 2}",
    "\"a\" in \"abc\"",
    "5 in 1..5",
    "5 in 1..<5",
    "1 in 1<..5",
    "3 in 1<..<5",
    // `if` is a compile-time function
    "if(True, do 1, do 0)",
    "if(False, do 1, do 0)",
    // conversions
    "int(\"10\")",
    "int(3.7)",
    "int(-3.7)",
    "int(True)",
    "int(\"1_0\")",
    "int(\"10000000000\")",
    "int(\"ff\", base := 16)",
    "int(\"0x10\", base := 0)",
    "nat(5)",
    "nat(\"10000000000\")",
    "float()",
    "float(1)",
    "float(\"2.5\")",
    "float(\"1_0.5\")",
    "str(1)",
    "str(True)",
    "str(\"ab\")",
    // numeric builtins
    "abs(-3)",
    // `abs`/`round`/`str` on a `Ratio` -- `abs` used to fold to a truncated
    // `Nat`, and `round`/`str` did not accept a `Ratio` at all
    "abs(-1.5)",
    "abs(-0.25)",
    "abs(0.1 - 0.3)",
    "round(0.5)",
    "round(1.5)",
    "round(2.5)",
    "round(-0.5)",
    "round(-1.5)",
    "round(2 / 3)",
    "str(1.5)",
    "str(-1.5)",
    "str(3.0)",
    "round(float(2.5))",
    "round(float(1.5))",
    "round(float(2.4))",
    "round(float(-1.5))",
    "pow(2, 10)",
    "divmod(7, 2)",
    "divmod(-7, 2)",
    // text builtins
    "ord(\"a\")",
    "chr(97)",
    "hex(255)",
    "hex(-255)",
    "bin(5)",
    "oct(8)",
    // sequence builtins
    "len([1, 2, 3])",
    "sum([1, 2, 3])",
    "max([1, 2])",
    "min([1, 2])",
    "all([True, True])",
    "any([False, True])",
    "sorted([3, 1, 2])",
    // const methods called on a value (rather than on the type)
    "\"abc\".replace(\"a\", \"z\")",
    "\"abc\".startswith(\"a\")",
    "\"abc\".endswith(\"c\")",
    "\"abc\".isalpha()",
    "\"abc\".isascii()",
    "\"123\".isdecimal()",
    "\"abc\".find(\"b\")",
    "\", \".join([\"a\", \"b\"])",
    "(-3).abs()",
    // subscripts (the desugarer turns these into `__getitem__` calls)
    "[1, 2, 3][0]",
    "[[1, 2], [3, 4]][0][1]",
    "{\"a\": 1}[\"a\"]",
    "(1..5)[0]",
    "[1, 2].reversed()",
    // decimal literals are `Fraction`s at run time, so they are folded exactly
    "0.1",
    "0.1 + 0.2",
    "0.5 + 0.5",
    "1.5 * 2",
    "0.1 * 0.1",
    "1.5 - 0.5",
    "1.5 // 1",
    "1.5 % 1",
    "1.5 ** 2",
    "0.5 ** 2",
    "7 / 2",
    "6 / 2",
    "1.5 > 1",
    "0.5 + 0.5 == 1",
    "int(0.1 + 0.2)",
    "-1.5",
    "1e+3",
    "1e-3",
    // `Int` is an i64, so negative results below i32 fold too
    "1 - 10000000000",
    "-100000 * 100000",
    "-3000000000",
    "-3000000000 + 1",
    "int(\"-9223372036854775808\")",
    "-2147483648 - 1",
];

/// Expressions whose folded value is known *not* to match the run-time value.
///
/// Each entry must still diverge: fixing one makes this test fail, which is the
/// reminder to delete the entry. See docs/const-eval-plan.md (phases 3 and 5).
const KNOWN_DIVERGENT: &[(&str, &str)] = &[
    // phase 5: these fold to a list, but return a lazy iterator at run time
    (
        "reversed([1, 2])",
        "folds to a list, but returns `Reversed`",
    ),
    ("zip([1], [2])", "folds to a list, but returns `zip`"),
    ("map(Dbl, [1, 2])", "folds to a list, but returns `map`"),
    (
        "filter(Odd, [1, 2])",
        "folds to a list, but returns `filter`",
    ),
    // `ValueObj::Dict` is a hash map, so it does not keep insertion order the way
    // Python's dict does -- which also reorders the views below
    (
        "{\"a\": 1, \"b\": 2}",
        "folds unordered, but Python keeps insertion order",
    ),
    (
        "{\"a\": 1, \"b\": 2}.keys()",
        "folds to a list, but returns `dict_keys`",
    ),
    (
        "{\"a\": 1, \"b\": 2}.values()",
        "folds to a list, but returns `dict_values`",
    ),
    (
        "{\"a\": 1}.items()",
        "folds to a list, but returns `dict_items`",
    ),
];

/// Unique per call: the tests run in parallel in one process.
fn tmp_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    temp_dir().join(format!(
        "erg_const_diff_{name}_{}_{n}.er",
        std::process::id()
    ))
}

/// The folded value of every `X{i}` binding, in order.
///
/// `--mode check` prints the HIR to stdout, one binding per line, in either of
/// two shapes:
///   `::X0(: {2}) =`
///   `::X0(: {%v_global_1: Tuple([Nat, Nat]) | %v_global_1 == (3, 1)}) =`
fn folded_values(source: &str) -> Vec<Option<String>> {
    let path = tmp_path("check");
    fs::write(&path, source).expect("failed to write the check program");
    let out = Command::new(env!("CARGO_BIN_EXE_erg"))
        .args(["--mode", "check"])
        .arg(&path)
        .output()
        .expect("failed to run the compiler");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = fs::remove_file(&path);
    let mut values = vec![];
    for i in 0.. {
        let Some(rest) = stdout.split(&format!("::X{i}(: ")).nth(1) else {
            break;
        };
        values.push(balanced_braces(rest).map(|inner| unrefine(&inner)));
    }
    values
}

/// The `{...}` starting at `s`, without the outer braces.
fn balanced_braces(s: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut in_str = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '{' if !in_str => depth += 1,
            '}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(s[1..i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// `%v: T | %v == VALUE` -> `VALUE`; anything else is already the value.
fn unrefine(inner: &str) -> String {
    match inner.split_once(" == ") {
        Some((lhs, rhs)) if lhs.starts_with('%') => rhs.trim().to_string(),
        _ => inner.trim().to_string(),
    }
}

/// One line of `print!` output per expression.
fn runtime_values(source: &str) -> Vec<String> {
    let path = tmp_path("run");
    fs::write(&path, source).expect("failed to write the run program");
    let out = Command::new(env!("CARGO_BIN_EXE_erg"))
        .arg(&path)
        .output()
        .expect("failed to run the program");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = fs::remove_file(&path);
    assert!(
        out.status.success(),
        "the generated program failed to run:\n{stdout}{stderr}"
    );
    let values = stdout
        .split_once(MARKER)
        .map(|(_, rest)| rest)
        .unwrap_or(&stdout);
    values.lines().skip(1).map(str::to_string).collect()
}

/// `ValueObj`'s `Display` quotes strings (it is repr-like), `print!` does not.
fn normalize(folded: &str) -> &str {
    folded
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(folded)
}

fn compare(cases: &[&str]) -> Vec<(String, String, String)> {
    let check_src = PRELUDE.to_string()
        + &cases
            .iter()
            .enumerate()
            .map(|(i, e)| format!("X{i} = {e}\n"))
            .collect::<String>();
    let run_src = format!("{PRELUDE}print! \"{MARKER}\"\n")
        + &cases
            .iter()
            .map(|e| format!("print! ({e})\n"))
            .collect::<String>();
    let folded = folded_values(&check_src);
    let runtime = runtime_values(&run_src);
    assert_eq!(
        folded.len(),
        cases.len(),
        "the compiler did not fold every expression (got {} of {})",
        folded.len(),
        cases.len()
    );
    assert_eq!(runtime.len(), cases.len(), "unexpected run-time output");
    let mut diffs = vec![];
    for ((expr, folded), runtime) in cases.iter().zip(folded).zip(runtime) {
        let folded = folded.unwrap_or_else(|| "<not folded>".to_string());
        if normalize(&folded) != runtime {
            diffs.push((expr.to_string(), folded, runtime));
        }
    }
    diffs
}

#[test]
fn const_fold_matches_runtime() {
    let diffs = compare(CASES);
    assert!(
        diffs.is_empty(),
        "compile-time and run-time values disagree:\n{}",
        diffs
            .iter()
            .map(|(e, c, r)| format!("  {e}\n    compile time: {c}\n    run time:     {r}\n"))
            .collect::<String>()
    );
}

#[test]
fn known_divergences_still_diverge() {
    let exprs = KNOWN_DIVERGENT.iter().map(|(e, _)| *e).collect::<Vec<_>>();
    let diffs = compare(&exprs);
    let diverged = diffs.iter().map(|(e, ..)| e.as_str()).collect::<Vec<_>>();
    let fixed = KNOWN_DIVERGENT
        .iter()
        .filter(|(e, _)| !diverged.contains(e))
        .map(|(e, why)| format!("  {e} ({why})\n"))
        .collect::<String>();
    assert!(
        fixed.is_empty(),
        "these no longer diverge -- remove them from KNOWN_DIVERGENT:\n{fixed}"
    );
}
