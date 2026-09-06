//! Differential test for the transpiler backend.
//!
//! `erg file.er` runs a program through the bytecode backend; `erg --mode
//! transpile file.er` writes a Python script that has to do the same thing.
//! Nothing else checks that the two agree: the transpiler's only tests are
//! one-line stdout comparisons through the embedding API, and CI never runs
//! it at all. Each case below is run both ways and the outputs compared.

use std::env::temp_dir;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use erg_common::python_util::_opt_which_python;

/// The interpreter the compiler itself resolves (a `.venv`, conda, pyenv, ...).
/// A fixed `python3` is a different version from the build-time one wherever a
/// virtualenv is active, and does not exist at all on Windows.
fn python_command() -> &'static str {
    static PYTHON: OnceLock<String> = OnceLock::new();
    PYTHON.get_or_init(|| _opt_which_python().expect("no Python interpreter found"))
}

/// Printed before the program's own output: the compiler writes its warnings
/// to stdout too, and only what the program prints is being compared.
const MARKER: &str = "<<<erg_transpile_diff>>>";

/// Programs whose two backends must print the same thing.
const CASES: &[(&str, &str)] = &[
    (
        "arithmetic",
        "\
print! 1 + 2
print! 7 // 2
print! \"a\" + \"b\"
",
    ),
    (
        "function",
        "\
f(x: Int): Int = x * 2
print! f(3)
",
    ),
    // A subclass without `new` of its own gets the superclass's `new` signature; the
    // generated `new` has to forward to it. The transpiled class is a Python subclass,
    // so it has the superclass's methods.
    (
        "inherit_new",
        "\
@Inheritable
P = Class {.x = Int; .y = Int}
P.
    new x, y := 0 = P {.x; .y}
    sum self = self.x + self.y
@Inheritable
Q = Inherit P
R = Inherit Q
q = Q.new 1
r = R.new 3, 4
print! q.x, q.y, q.sum(), r.sum(), isinstance(r, P)
",
    ),
    // A generated `__init__` that forwards to the superclass has to name the class it
    // belongs to. `super(type(self), self)` is the *instance's* class, so a third class
    // down the chain inherited the frame and called it again until the stack ran out.
    (
        "inherit_chain",
        "\
@Inheritable
P = Class()
@Inheritable
Q = Inherit P
R = Inherit Q
r = R.new()
print! r in R, r in Q, r in P
",
    ),
    // The same chain with a body to run in the middle class: the forwarding `__init__`
    // is generated there, so this is the case the guard cannot skip.
    (
        "inherit_chain_init",
        "\
@Inheritable
P = Class()
@Inheritable
Q = Inherit P
Q.
    __init__! self =
        print! \"q\"
R = Inherit Q
_ = R.new()
",
    ),
    // The user's `__init__!` body goes into the generated `__init__`; it used to be
    // left in the method list and written a second time under its mangled name.
    (
        "user_init",
        "\
C = Class {.x = Int}
C.
    __init__! self =
        print! \"init\"
c = C.new {.x = 1}
print! c.x
",
    ),
    // `*args` comes before the defaults: written the other way round, `f(1, 2, 3)`
    // bound `b = 2` and the sum came out 6 instead of 16.
    (
        "var_args_with_default",
        "\
f(a: Int, *args: Int, b := 10) = a + b + args.sum()
print! f(1, 2, 3)
",
    ),
    // A raw identifier sidesteps the compiler's name checks, so a private one has to be
    // mangled like any other private name: spelled exactly, it overwrote the prelude.
    (
        "raw_ident_shadows_prelude",
        "\
'Int' = 1
print! 'Int' + 1
",
    ),
    // A block local is a local of the call, not of the module. Every
    // definition inside a block used to be written as a module global, so a
    // recursive call overwrote its caller's copy.
    (
        "recursion_block_local",
        "\
f(n: Int): Int =
    m = n
    if n <= 0:
        do: 0
        do: f(n - 1) + m
print! f(3)
",
    ),
    (
        "recursion_two_locals",
        "\
g(n: Int): Int =
    a = n * 10
    b = n
    if n <= 0:
        do: 0
        do:
            rest = g(n - 1)
            a + b + rest
print! g(2)
",
    ),
    // The body of a loop is called once per element, so a binding of its own
    // is fresh on every iteration and a closure made in the body keeps that
    // iteration's value.
    (
        "loop_var_capture",
        "\
adders!() =
    fs = ![]
    for! [1, 2, 3], i =>
        fs.push!((m: Int) -> m + i)
    fs
fs = adders!()
print! fs[0](10)
last = fs.pop!()
print! last(10)
",
    ),
    (
        "loop_body_local_capture",
        "\
mk!() =
    fs = ![]
    for! [1, 2], i =>
        k = i * 10
        fs.push!(() -> k)
    fs
fs = mk!()
print! fs[0]()
last = fs.pop!()
print! last()
",
    ),
    (
        "loop_body_as_variable",
        "\
adders!() =
    fs = ![]
    body! = i => fs.push!((m: Int) -> m + i)
    for! [1, 2, 3], body!
    fs
fs = adders!()
print! fs[0](10)
",
    ),
    (
        "while_body_local_capture",
        "\
counters!() =
    fs = ![]
    n = !0
    while! do! n < 3, do!:
        n.inc!()
        k = n * 2
        fs.push!(() -> k)
    fs
fs = counters!()
print! fs[0]()
last = fs.pop!()
print! last()
",
    ),
    // A nested function reads the frame it was defined in, one copy per call.
    (
        "nested_closure",
        "\
adder(n: Int): (Int -> Int) =
    (m: Int) -> m + n
one = adder 1
two = adder 2
print! one(10)
print! two(10)
",
    ),
    (
        "if_branch_local_capture",
        "\
mk(x: Int): (() -> Int) =
    if x > 0:
        do:
            k = x + 10
            () -> k
        do: () -> 0
print! mk(1)()
print! mk(2)()
",
    ),
    (
        "match_arm_capture",
        "\
mk(x: Int): (() -> Int) =
    match x:
        n -> () -> n
print! mk(1)()
print! mk(2)()
",
    ),
    // `with!` is a block with a value, like every other
    (
        "with_block_value",
        "\
Box = Class {.v = Int}
Box|<: ContextManager|.
    __enter__ self = self
    __exit__ self, _, _, _ = False
mk!(x: Int): (() -> Int) =
    with! Box.new({.v = x}), b =>
        () -> b.v
print! mk!(1)()
print! mk!(2)()
",
    ),
    // the transpiler has to import the convertors module for these
    (
        "convertors",
        "\
print! bool 1
print! bool 0
print! int \"10\"
print! str 1
",
    ),
    (
        "loop_sum",
        "\
total!() =
    acc = !0
    for! [1, 2, 3], i =>
        acc.inc! i
    acc
print! total!()
",
    ),
    (
        "nested_loops",
        "\
pairs!() =
    fs = ![]
    for! [1, 2], i =>
        for! [10, 20], j =>
            fs.push!(() -> i + j)
    fs
fs = pairs!()
print! fs[0]()
last = fs.pop!()
print! last()
",
    ),
    // A parameter named `p!` is registered as `p__erg_proc__`, and a reference
    // to it was spelled `p!`: not a local, so it became a `LOAD_NAME` of a
    // global that does not exist.
    (
        "bang_parameter",
        "\
run!(p!, x) = p! x
run! (x) => print!(x), 1
",
    ),
    // Classes are written as the bytecode backend builds them: `__init__` from
    // the constructor's shape, `new` with as many parameters as it takes.
    (
        "class_record",
        "\
C = Class {.x = Int}
C.
    get self = self.x
c = C.new {.x = 1}
print! c.get()
",
    ),
    (
        "class_empty",
        "\
E = Class()
E.
    hello self = \"hi\"
print! E.new().hello()
",
    ),
    (
        "class_newtype",
        "\
V = Class [Int; _]
V.
    total self = sum(self::base)
print! V.new([1, 2, 3]).total()
",
    ),
    // A private field is one attribute for the whole class (`value__`), as
    // the record literal, the generated `__init__` and `self::value` must
    // agree on its name.
    (
        "class_private_field",
        "\
M = Class {value = Int}
M.
    get self = self::value
print! M.new({value = 2}).get()
",
    ),
    (
        "class_inherit",
        "\
@Inheritable
P = Class {::[<: Self]x = Int}
P.
    norm self = self::x ** 2
Q = Inherit P
Q.
    @Override
    norm self = self::x ** 3
print! P.new({x = 2}).norm()
print! Q.new({x = 2}).norm()
",
    ),
    (
        "class_staticmethod",
        "\
D = Class()
D.
    @staticmethod
    foo x = x + 1
print! D.new().foo(1)
",
    ),
    (
        "class_self",
        "\
D = Class {.y = Int}
D.
    new y = Self {.y;}
    one = Self.new 1
print! D.one.y
print! D.new(2).y
",
    ),
    // A parameter is named as the keyword arguments that pass it are.
    (
        "keyword_args",
        "\
f(a: Int, b := 1) = a + b
print! f(1)
print! f(1, b := 2)
",
    ),
    // A private method or class attribute is one name for the whole class.
    (
        "class_private_members",
        "\
P = Class()
P::
    one = 1
    helper self = 10
P.
    zero = P::one - 1
    go self = self::helper() + P::one
print! P.zero
print! P.new().go()
",
    ),
    // The user's `__init__!` runs after the generated field assignments.
    (
        "class_user_init",
        "\
C = Class {.x = Int}
C.
    __init__! self =
        print! \"init\", self.x
c = C.new {.x = 3}
print! c.x
",
    ),
    // A raw identifier is spelled exactly (`unittest` looks tests up by name).
    (
        "raw_identifier",
        "\
'test_one' x = x + 1
print! 'test_one'(1)
'2t+3' = 5
print! '2t+3'
",
    ),
    // A parameter that is a Python keyword gets a `_`, at the definition and
    // at the keyword argument.
    (
        "keyword_named_param",
        "\
f(def: Int, class: Int) = def * 10 + class
print! f(1, class := 2)
",
    ),
    (
        "var_args",
        "\
first *x = x[0]
print! first(1, 2, 3)
kw_var(**x: Int) = x[\"a\"] + x[\"b\"]
print! kw_var(a := 1, b := 2)
",
    ),
];

/// Programs whose two backends are known *not* to agree.
///
/// Each entry must still diverge: fixing one makes this test fail, which is
/// the reminder to delete the entry.
const KNOWN_DIVERGENT: &[&str] = &[];

/// Unique per call: the tests run in parallel in one process.
fn tmp_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    temp_dir().join(format!(
        "erg_transpile_diff_{name}_{}_{n}.er",
        std::process::id()
    ))
}

/// What the bytecode backend prints, or why it could not be run.
fn bytecode_output(path: &PathBuf) -> Result<String, String> {
    let out = Command::new(env!("CARGO_BIN_EXE_erg"))
        .arg(path)
        .output()
        .expect("failed to run the compiler");
    // the run leaves the compiled module next to the source
    let _ = fs::remove_file(path.with_extension("pyc"));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(after_marker(&stdout))
    } else {
        Err(format!(
            "the bytecode backend failed:\n{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

/// What the program printed: everything after the marker.
fn after_marker(stdout: &str) -> String {
    stdout
        .split_once(MARKER)
        .map(|(_, rest)| rest.trim_start_matches('\n').to_string())
        .unwrap_or_else(|| stdout.to_string())
}

/// What the transpiled Python prints, or why it could not be run.
fn transpiled_output(path: &PathBuf) -> Result<String, String> {
    let out = Command::new(env!("CARGO_BIN_EXE_erg"))
        .args(["--mode", "transpile"])
        .arg(path)
        .output()
        .expect("failed to run the transpiler");
    if !out.status.success() {
        return Err(format!(
            "the transpiler failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let py = path.with_extension("py");
    let py_command = python_command();
    let run = Command::new(py_command).arg(&py).output();
    let run = match run {
        Ok(run) => run,
        Err(e) => return Err(format!("failed to run {py_command}: {e}")),
    };
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let res = if run.status.success() {
        Ok(after_marker(&stdout))
    } else {
        Err(format!(
            "the transpiled program failed:\n{stdout}{}",
            String::from_utf8_lossy(&run.stderr)
        ))
    };
    let _ = fs::remove_file(&py);
    res
}

/// Both backends' output for one program.
fn outputs(name: &str, source: &str) -> (Result<String, String>, Result<String, String>) {
    let path = tmp_path(name);
    let source = format!("print! \"{MARKER}\"\n{source}");
    fs::write(&path, &source).expect("failed to write the program");
    let bytecode = bytecode_output(&path);
    let transpiled = transpiled_output(&path);
    let _ = fs::remove_file(&path);
    (bytecode, transpiled)
}

#[test]
fn transpiled_matches_bytecode() {
    let mut failures = String::new();
    for (name, source) in CASES {
        if KNOWN_DIVERGENT.contains(name) {
            continue;
        }
        let (bytecode, transpiled) = outputs(name, source);
        match (bytecode, transpiled) {
            (Ok(b), Ok(t)) if b == t => {}
            (Ok(b), Ok(t)) => {
                failures += &format!(
                    "\n[{name}] the backends disagree\n  bytecode:   {:?}\n  transpiled: {:?}\n",
                    b, t
                );
            }
            (Err(e), _) | (_, Err(e)) => {
                failures += &format!("\n[{name}] {e}\n");
            }
        }
    }
    assert!(failures.is_empty(), "{failures}");
}

#[test]
fn known_divergences_still_diverge() {
    for name in KNOWN_DIVERGENT {
        let (_, source) = CASES
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} is not a case"));
        let (bytecode, transpiled) = outputs(name, source);
        if let (Ok(b), Ok(t)) = (&bytecode, &transpiled) {
            assert_ne!(
                b, t,
                "[{name}] the backends agree now: remove it from KNOWN_DIVERGENT"
            );
        }
    }
}
