---
name: add-test
description: Add a new Erg test case (should_ok or should_err)
argument-hint: "<ok|err> <feature-name>"
---

# Add Erg Test Case

Add a new test case for the Erg compiler.

## Arguments

- `$ARGUMENTS[0]` — `ok` (should compile) or `err` (should produce error)
- `$ARGUMENTS[1]` — Feature name (used as filename)

## Steps

1. **Check existing tests** for similar patterns:
   - `tests/should_ok/` for passing tests
   - `tests/should_err/` for error tests

2. **Create test file**: `tests/should_$0/$1.er`
   - Reference `doc/EN/syntax/` for correct Erg syntax
   - Reference `examples/` for idiomatic patterns
   - For `should_err` tests, include a comment explaining the expected error

3. **Register the test** in `tests/eval_tests.rs`:
   - For `should_ok`: Add an `exec_test!` entry
   - For `should_err`: Add an `expect_failure_test!` entry
   - For native compiler: Add test in `crates/erg_native/tests/`

4. **Run the test** to verify:

   ```bash
   cargo test --features large_thread -- <test_name>
   # For native compiler tests:
   cargo test -p erg_native -- <test_name>
   ```

## Erg Syntax Quick Reference

- Variables: `x = 1` (immutable), `x = !1` (mutable)
- Functions: `f x = x + 1` or `f(x) = x + 1`
- Procedures (side effects): `p! x = print! x`
- Types: `x: Int = 1`, `f(x: Int): Int = x + 1`
- Classes: `C = Class {attr: Int}`
- Traits: `T = Trait {method: Self.() -> Int}`

## Reference Files

- Language syntax: `doc/EN/syntax/00_basic.md` onwards
- Type system: `doc/EN/syntax/type/01_type_system.md` onwards
- Existing test patterns: `tests/should_ok/`, `tests/should_err/`
- Example code: `examples/`
