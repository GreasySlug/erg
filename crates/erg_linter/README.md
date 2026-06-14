# erg-linter (WIP)

erg-linter (can be used with `erg lint`) is a tool to check the erg file for errors.

## Features

### Implemented

The following codes are warned.

* Redundant comparisons against `True`/`False` (e.g. `x == True`)
* Tautological self-comparisons that are always true (e.g. `x >= x`)
* Contradictory self-comparisons that are always false (e.g. `x != x`)
* Absurd `Nat` comparisons against `0` (e.g. `n < 0` is always false, `n >= 0` is always true)
* Redundant double negation (e.g. `not (not x)`)
* Hardcoded well-known constants (e.g. `3.14` → `math.pi`)
* Identity arithmetic (e.g. `x + 0`, `x * 1`)
* Erasing arithmetic and `% 1` that are always zero (e.g. `x * 0`, `x % 1`)
* `if` expressions that just return a boolean literal (e.g. `if cond, do(True), do(False)` → `cond`)
* `if` expressions whose branches are identical
* Unreachable code (chunks after a `return`/`panic` or any expression of type `Never`)
* Procedures (`f! x = ...`) whose body has no side effects (could be a plain function)
* Parameters that shadow a built-in (e.g. `f map = ...`); the compiler already covers module-level and local shadowing
* Defining a subroutine with too many parameters
* Defining a class with too many fields

### Planned

* Wildcard import
* Variables that can be defined as constants
* Unnecessary `.clone`
* Mutable objects that do not change

### These are warned by the compiler

* Unused variables
* Unused objects that are not `NoneLike`

## Architecture

```text
crates/erg_linter/
  lib.rs         # `Linter` struct, `Runnable`/`New` impls, and `lint()` — the dispatch
                 # list that enumerates the active rules
  traverse.rs    # `check_recursively` / `check_acc_recursively` — the shared HIR walk
  warn.rs        # warning factories (one per message, localized via `switch_lang!`)
  rules/
    mod.rs        # module declarations
    comparison.rs # tautology / contradiction / bool_comparison
    numeric.rs    # magic_number
    redundancy.rs # double_negation
    structure.rs  # too_many_params / too_many_instance_attributes
  tests/lint.rs  # integration tests (lint an in-memory snippet, assert warning counts)
```

Each rule is a `lint_*` method added to `Linter` via an `impl` block in a
`rules/*.rs` file. A rule inspects the node it cares about, pushes a warning when
it matches, and then recurses through `self.check_recursively(&Self::lint_*, expr)`
so it visits every descendant expression. See
[`lint_enhance.md`](./lint_enhance.md) for a catalog of candidate rules inspired
by `rustc`/Clippy.

## Adding a new rule

Take the existing `magic_number` rule as a template (`rules/numeric.rs` +
`magic_number` in `warn.rs`).

1. **Add the warning factory** in `warn.rs`. Localize the message and hint with
   `switch_lang!`, and return a `CompileWarning`:

   ```rust
   pub(crate) fn my_rule(input: Input, errno: usize, caused_by: String, loc: Location) -> CompileWarning {
       let msg = switch_lang!(
           "japanese" => "……",
           "english"  => "……",
           // simplified_chinese / traditional_chinese …
       ).to_string();
       // … build ErrorCore + CompileWarning::new(…)
   }
   ```

2. **Implement the rule** in the matching `rules/*.rs` (or a new file registered
   in `rules/mod.rs`). Make the entry method `pub(crate)` so `lint()` can call it,
   and always recurse at the end:

   ```rust
   impl Linter {
       pub(crate) fn lint_my_rule(&mut self, expr: &Expr) {
           if let Expr::/* node of interest */ = expr {
               self.warns.push(my_rule(self.input(), line!() as usize, self.caused_by(), expr.loc()));
           }
           self.check_recursively(&Self::lint_my_rule, expr);
       }
   }
   ```

   Use `self.input()` / `self.caused_by()` for the warning context. If you need
   type/effect/ownership information, see the table in `lint_enhance.md`.

3. **Register it** in the dispatch loop of `Linter::lint` in `lib.rs`:

   ```rust
   self.lint_my_rule(chunk);
   ```

4. **Add a test** to `tests/lint.rs` (lint an inline snippet and assert the count
   of warnings whose message matches the rule).

5. **Document it** under `### Implemented` above.

Then verify:

```bash
cargo test -p erg_linter --features large_thread --test lint
cargo clippy -p erg_linter --all-targets --features large_thread -- -D warnings
cargo fmt -p erg_linter -- --check
```
