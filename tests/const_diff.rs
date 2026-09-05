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
const PRELUDE: &str = concat!(
    "Dbl(N: Nat): Nat = N * 2\n",
    "Odd(N: Nat): Bool = N % 2 == 1\n",
    // a constant *of* a class: the receiver is not one of its arguments
    "Cls = Class {}\n",
    "Cls.\n",
    "    Twice(N: Int): Int = N * 2\n",
    "    Base: Int = 10\n",
    "    Plus(N: Int): Int = Cls.Base + N\n",
    // built from a constant above it in the same block
    "    Tripled: Int = Cls.Base * 3\n",
    // a constant built from the parameters, which only a call can evaluate
    "Local(N: Int): Int =\n",
    "    Tmp = N + 1\n",
    "    Tmp * 2\n",
    // `match`, whose arms are tested with the conditions its patterns were
    // desugared into -- the same ones codegen emits
    "Name(K: Int): Str =\n",
    "    match K:\n",
    "        0 -> \"zero\"\n",
    "        1 -> \"one\"\n",
    "        _ -> \"many\"\n",
    "Bind(K: Int): Int =\n",
    "    match K:\n",
    "        0 -> 100\n",
    "        n -> n * 2\n",
    // a subroutine defined inside a body: it reads the frame it is called
    // from, the way the emitted Python closure reads its enclosing frame
    "Nest(N: Int): Int =\n",
    "    Bump(M: Int): Int = M + N\n",
    "    Bump(1) + Bump(2)\n",
    "Pure(N: Int): Int =\n",
    "    Sq(M: Int): Int = M * M\n",
    "    Sq(4) + N\n",
    // a subroutine of no parameters is a subroutine: a call folds, the name does not
    "Zero(): Int = 1\n",
    // the rest of the arguments -- the list they are at run time, too
    "Count(*Xs: Int): Int = len Xs\n",
    "Rest(X: Int, *Xs: Int): [Int; _] = Xs\n",
    // locals of any name in a body, and what a pattern definition desugars to
    "Sum2(T: (Int, Int)): Int =\n",
    "    (a, b) = T\n",
    "    a + b\n",
    "Low(N: Int): Int =\n",
    "    m = N + 1\n",
    "    m * 2\n",
    // a function taking a function: called with an anonymous lambda below
    "Apply(F, X: Int): Int = F(X)\n",
    "Which(S: Str): Int =\n",
    "    match S:\n",
    "        \"a\" -> 1\n",
    "        \"b\" -> 2\n",
    "        _ -> 3\n",
);

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
    // a negative exponent is an exact rational, not a float: `Int.PowOutput`
    // used to be `Nat`, so this folded to 0 (and `(-2) ** 3` raised at run time)
    "2 ** -1",
    "2 ** -2",
    "(-2) ** 3",
    "(-2) ** 2",
    "pow(2, -1)",
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
    // indexing every interval kind: `start + index` if the range contains it,
    // as the runtime `Range.__getitem__` does (a closed range used to be indexed
    // as if its end were exclusive, so `(1..3)[2]` was a compile-time IndexError)
    "(1..3)[2]",
    "(1..3)[0]",
    "(1..<3)[1]",
    "(1<..3)[1]",
    "(1<..<4)[1]",
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
    "bool()",
    "bool(1)",
    "bool(0)",
    "bool(-1)",
    "bool(0.0)",
    "bool(2.5)",
    "bool(0.1 - 0.1)",
    "bool(\"\")",
    "bool(\"a\")",
    "bool(True)",
    "bool(False)",
    "bool(None)",
    "bool([])",
    "bool([0])",
    "bool({})",
    "bool({0: 1})",
    "bool({0})",
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
    // unary `+`/`-` on a `Ratio`, which had no `Pos`/`Neg` impl
    "+0.1",
    "-(1 / 3)",
    "+(1 / 3)",
    "-(0.1 + 0.2)",
    // exactly representable only once the power of ten is cancelled down
    "2.5e-38",
    "1e-38",
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
    // `Ratio`'s reduced pair, which had no compile-time reading at all
    "1.5.numerator",
    "1.5.denominator",
    "(-1.5).numerator",
    "(1 / 3).denominator",
    "3.numerator",
    "3.denominator",
    "0.1.real",
    "0.1.imag",
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
    // a constant of a class, and a constant method called through the class
    "Cls.Twice(3)",
    // a constant of the class, read from a constant of that class
    "Cls.Plus(3)",
    "Str.replace(\"abc\", \"a\", \"z\")",
    "Str.startswith(\"abc\", \"a\")",
    // a constant function whose body defines a constant from its parameters
    "Local(3)",
    "Local(0)",
    // `match`: a literal pattern, the arm that binds, and the last arm, which is
    // taken without testing because an unmatched `match` falls into it too
    "Name(0)",
    "Name(1)",
    "Name(7)",
    "Bind(0)",
    "Bind(7)",
    "Which(\"a\")",
    "Which(\"b\")",
    "Which(\"z\")",
    "1e-3",
    // phase 5: Bool arithmetic, string ordering, sequence equality
    "True + True",
    "True * 3",
    "False - True",
    "True + 1",
    "\"a\" < \"b\"",
    "\"b\" <= \"a\"",
    "\"a\" > \"a\"",
    "\"abc\" >= \"abd\"",
    "[1] == [1]",
    "[1, 2] == [1, 3]",
    "[1] == [1, 2]",
    "[1] != [2]",
    "(1, \"a\") == (1, \"a\")",
    "(1,) != (2,)",
    "None == None",
    // phase 5: chained comparison
    "1 < 2 < 3",
    "1 < 3 < 2",
    "3 > 2 > 1",
    "1 <= 1 < 2",
    "1 == 1 == 1",
    "1 < 2 < 3 < 4",
    "1 < 2 < 3 < 0",
    // phase 5: type predicates
    "isinstance(1, Int)",
    "isinstance(-1, Nat)",
    "isinstance(\"a\", Int)",
    "isinstance(True, Int)",
    "isinstance(1, (Str, Int))",
    "issubclass(Nat, Int)",
    "issubclass(Int, Nat)",
    "issubclass(Str, (Int, Str))",
    "1e0",
    "245e5",
    "25E5",
    "2.5E-2",
    // `Int` is an i64, so negative results below i32 fold too
    "1 - 10000000000",
    "-100000 * 100000",
    "-3000000000",
    "-3000000000 + 1",
    "int(\"-9223372036854775808\")",
    "-2147483648 - 1",
    // `Dict` keeps insertion order now, the way Python's does (numeric keys:
    // `print!` shows Python's repr, which quotes strings differently)
    "{2: 20, 1: 10}",
    "{3: 1, 1: 3, 2: 2}",
    "{1: 2, 3: 4}",
    // a subroutine defined in a body, whose captures belong to one call
    "Nest(3)",
    "Nest(4)",
    "Nest(0)",
    "Pure(1)",
    "Pure(10)",
    // parameterless and variadic const subroutines
    "Zero()",
    "Count()",
    "Count(1, 2, 3)",
    "Rest(1, 2, 3)",
    "Rest(1)",
    // a constant of a class built from another in the same block
    "Cls.Tripled",
    // locals of any name, a pattern definition, an anonymous lambda
    "Sum2((1, 2))",
    "Low(1)",
    "Apply((X: Int) -> X + 1, 2)",
];

/// Expressions whose folded value is known *not* to match the run-time value.
///
/// Each entry must still diverge: fixing one makes this test fail, which is the
/// reminder to delete the entry. See docs/const-eval-plan.md (phases 3 and 5).
const KNOWN_DIVERGENT: &[(&str, &str)] = &[
    // empty since `Dict` learned to keep insertion order; the shape stays for
    // the next divergence found
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
