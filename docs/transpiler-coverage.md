# Transpiler coverage survey

`erg --mode transpile file.er` writes a Python script that must behave like the
bytecode backend. `tests/transpile_survey.py` runs every program under
`tests/should_ok/` and `examples/` through both backends and classifies the
result; `tests/transpile_diff.rs` is the regression test for the cases that
already agree (add a case there for every group fixed).

```bash
cargo build
python3 tests/transpile_survey.py . /tmp/transpile_survey   # summary.tsv + details/
```

## Totals (156 files)

| Class | 2026-09-05 (start) | after the class rewrite | Meaning |
| ----- | ---: | ---: | ------- |
| OK | 57 | 69 | both backends print the same thing |
| TRANSPILE_FAIL | 40 | 25 | the transpiler panics (`todo!`/`unreachable!`/index) |
| PY_ERROR | 40 | 40 | the generated Python fails to parse or raises |
| MISMATCH | 0 | 3 | the programs print an object's default `repr` (`<... object at 0x...>`), which no two runs agree on; not a transpiler defect |
| BC_FAIL | 19 | 19 | the bytecode backend itself fails (demo programs that are meant to fail, external packages, declaration-only modules); out of scope |

Of the 137 programs the bytecode backend runs, 69 (50 %) transpile correctly.

## Done

- **Classes** (`write_classdef`): `Class()`, `Class {record}`, `Class Int`
  (the value kept as `base__`), `Inherit X` (`class Y(X)`, with
  `super().__init__(*args, **kwargs)` when the constructor takes nothing), a
  user `__init__!` body after the field assignments, `__del__!` as `__del__`,
  `new` with as many parameters as the constructor, `__class_getitem__` for a
  polymorphic class, `Self` as the class's name.
- **Names**: a private (or restricted, `::[<: Self]x`) attribute or method is
  one name for the whole class, `x__`, at its definition, in the record
  literal that fills it and at `self::x`; a parameter keeps its source name
  (as the bytecode backend does, with a `_` after a Python keyword) so a
  keyword argument and a `**kwargs` key match it; `*args` / `**kwargs`
  parameters are written; a raw identifier (`'test_one'`) is spelled exactly
  when Python can, so `unittest` finds the test.
- **Decorators** are written (`@staticmethod`); `Override` and `Inheritable`
  are compile-time and dropped.
- An attribute definition's target is written without the builtin class
  wrapper (`Nat((C).i) = ...` was a `SyntaxError`).

## TRANSPILE_FAIL by cause (25)

| # | Site (`erg_compiler/transpile.rs`) | Cause | Files |
| - | --- | --- | --- |
| 9 | `Expr::Code` (761) | importing user-defined modules | import, magic, use_ansicolor, use_exception, use_unit (examples); closure, context_manager, dunder, infer_method |
| 10 | match arm patterns (1162) | `match` arms typed by a type spec other than a literal set (`Int`, `Str`, a class, `0..1`, `[?; 2]`, tuples, records) | class, control, fib, mylist (examples); control_expr, match, pattern, rec, recursive_class, recursive_multi_arm |
| 4 | `write_unaryop` (892) | the `?` operator (an early `return` inside an expression) | option_mut, option_result, try_op, unwrap |
| 1 | `Expr::List` (693) | list comprehension / non-normal list | advanced_type_spec |
| 1 | `write_if` (1061) | `if` whose then-branch is not a lambda literal | never |

## PY_ERROR by cause (40)

| # | Cause | Files |
| - | --- | --- |
| 8 | `Trait` / `Structural` are not emitted as runtime names | trait (examples); associated_types, poly_trait, structural, trait_op_requirement, structural_alias, structural_disambiguation, structural_trait |
| 6 | **invalid Python emitted** (`SyntaxError`): stray tokens, unterminated strings | record, with (examples); coercion, mut_dict, assert_cast, interpolation |
| 4 | **scoping**: a mangled local (`k_L12_C27`, `i_L2_C24`, `b_L5_C10`, `i_L2`) is referenced where a different mangling is in scope | dict (examples); iterator, use_itertools, decl |
| 4 | **arguments**: `*args` spread (`f(*xs)`), a positional passed after `*args` and again by keyword, keyword arguments to a Python builtin (`abs(x:=1)`), a `**kwargs` key | args_expansion, var_args, const_kw_args, var_kwargs |
| 3 | prelude helpers `iterable_map` / `iterable_filter` / `iterable_reduce` are used but never defined | iterator (examples); lambda_arg, comprehension |
| 3 | a `return` in a lambda is written as an attribute (`.return__`); a type application on a function (`f|T|`) | return, fast_value, poly_class_full |
| 12 | one-offs: patch operator name (`__Invert___zero__`), `Del`, `add` (operator symbol), `pow__`, `list_iterator`, `float.nearly_eq`, `Structural` subtype compare, `method-wrapper[...]`, `dyn_type_check` and `pyimport` assertions, `pystd_decls` import shadowing, `ratio_literal` (timeout) | patch, impl (examples); comment, sym_op, const_func, ratio_num, structural_subtyping, map, dyn_type_check, pyimport, pystd_decls, ratio_literal |

## Order of work

1. Match arms on types (10 files) -- `case` with a class pattern, `isinstance`
   for the builtin types, ranges as guards.
2. Importing user modules (9 files): transpile the imported module too and
   write it next to the output.
3. `Trait` / `Structural` at run time (8 files): a runtime class like the
   bytecode backend's.
4. Emitting invalid Python (6), scoping (4), arguments (4), prelude helpers (3).
5. The `?` operator (4 files) -- needs statement-level lowering of the
   enclosing expression, like the multi-chunk `if` already does.
