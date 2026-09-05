# Transpiler coverage survey (2026-09-05)

`erg --mode transpile file.er` writes a Python script that must behave like the
bytecode backend. `tests/transpile_survey.py` runs every program under
`tests/should_ok/` and `examples/` through both backends and classifies the
result; `tests/transpile_diff.rs` is the regression test for the cases that
already agree.

```bash
cargo build
python3 tests/transpile_survey.py . /tmp/transpile_survey   # summary.tsv + details/
```

## Totals (156 files)

| Class | Count | Meaning |
| ----- | ----- | ------- |
| OK | 57 | both backends print the same thing |
| TRANSPILE_FAIL | 40 | the transpiler panics (`todo!`/`unreachable!`/index) |
| PY_ERROR | 40 | the generated Python fails to parse or raises |
| BC_FAIL | 19 | the bytecode backend itself fails (demo programs that are meant to fail, external packages, declaration-only modules); out of scope |

Of the 137 programs the bytecode backend runs, 57 (42 %) transpile correctly.

## TRANSPILE_FAIL by cause (40)

| # | Site (`erg_compiler/transpile.rs`) | Cause | Files |
| - | --- | --- | --- |
| 10 | `write_classdef` (1427) | assumes every class is `Class {record}` with one positional constructor parameter; `Class()`, `Inherit`, `Class Int` index an empty parameter list | class, init_del, unit_test, associated_types, decl, poly_class, poly_class_full, poly_trait, self_type (should_ok); class (examples) |
| 9 | `Expr::Code` (743) | importing user-defined modules | import, magic, use_ansicolor, use_exception, use_unit (examples); closure, context_manager, dunder, infer_method |
| 7 | match arm patterns (1144) | `match` arms typed by a type spec other than a literal set (`Int`, `Str`, `0..1`, `[?; 2]`, tuples) | control, fib (examples); control_expr, match, pattern, rec, recursive_multi_arm |
| 5 | `write_classdef` (1440) | class whose base is a type expression, not a record (`Class Int or NoneType`, refinement sets, `Dict(...)`, function types) | mylist (examples); const_method, container_class, recursive_class, refinement_class |
| 4 | `write_unaryop` (874) | the `?` operator (an early `return` inside an expression) | option_mut, option_result, try_op, unwrap |
| 3 | `write_params` (1278) | a parameter pattern other than a name or `_` (mutable/type-ascribed patterns) | mut (examples); infer_trait, mut_type |
| 1 | `Expr::List` (675) | list comprehension / non-normal list | advanced_type_spec |
| 1 | `write_if` (1043) | `if` whose then-branch is not a lambda literal | never |

## PY_ERROR by cause (40)

| # | Cause | Files |
| - | --- | --- |
| 11 | **invalid Python emitted** (`SyntaxError`): attribute assignment written as a call (`cannot assign to function call`), stray tokens, unterminated strings | a11y, record, with (examples); class_attr, const_func, inherit, coercion, fast_value, mut_dict, return, assert_cast, interpolation |
| 6 | `Trait` / `Structural` are not emitted as runtime names | trait (examples); structural, trait_op_requirement, structural_alias, structural_disambiguation, structural_trait |
| 5 | **keyword / variadic arguments**: parameters are mangled (`x_L1_C4`, `a__`) but the call site passes the source name; `*args` handler gets positional args | var_args, var_kwargs, default_param, args_expansion, const_kw_args |
| 3 | **scoping**: a mangled local (`k_L11_C27`, `i_L1_C24`, `b_L4_C10`) is referenced where a different mangling is in scope | dict (examples); iterator, use_itertools |
| 3 | prelude helpers `iterable_map` / `iterable_filter` / `iterable_reduce` are used but never defined | iterator (examples); lambda_arg, comprehension |
| 12 | one-offs: patch operator name (`__Invert___zero__`), `Del`, `add` (operator symbol), `float.nearly_eq`, private record field (`x__`), `Structural` subtype compare, `method-wrapper[...]`, `dyn_type_check` assertion, `pyimport` assertion, `pystd_decls` import shadowing, `map`, `ratio_literal` (timeout) | patch, impl (examples); comment, sym_op, ratio_num, structural_subtyping, map, dyn_type_check, pyimport, pystd_decls, ratio_literal |

## Order of work

1. `write_classdef` for every constructor shape (15 files across the two class rows).
2. Emitting invalid Python (11 files) -- each is a small writer bug.
3. Importing user modules (9 files): transpile the imported module too and
   write it next to the output.
4. Match arms on types (7 files), keyword/variadic arguments (5), scoping (3),
   prelude helpers (3).
5. The `?` operator (4 files) -- needs statement-level lowering of the
   enclosing expression, like the multi-chunk `if` already does.
