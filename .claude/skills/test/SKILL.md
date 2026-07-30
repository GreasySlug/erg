---
name: test
description: Run Erg compiler tests with appropriate flags and options
argument-hint: "[crate-name or test-name]"
disable-model-invocation: true
allowed-tools: Bash(cargo test:*), Bash(cargo nextest run:*)
---

# Test Runner for Erg

Run tests for the Erg compiler project.

## Usage

- `/test` — Run all tests: `cargo test --features large_thread`
- `/test <crate>` — Run specific crate tests: `cargo test --package $ARGUMENTS --features large_thread`
- `/test <test_name>` — Run specific test: `cargo test <test_name> --features large_thread`

## Rules

1. Always use `--features large_thread` to prevent stack overflow
2. For ELS tests, use nextest with retries: `cargo nextest run --package els --features els --retries 3`
3. Use `-- --nocapture` if stdout/stderr output is needed for debugging
4. Report test results clearly: total, passed, failed, ignored

## Crate Names

- `erg_common` — Shared utilities
- `erg_parser` — Lexer and parser
- `erg_compiler` — Type checker and code generator
- `erg_linter` — Linter
- `els` — Language Server (use nextest)
- `erg_proc_macros` — Procedural macros
- `erg_native` — Native compiler (Galois/LLVM). Note: `large_thread` is a dev-dependency so `--features large_thread` is not needed

## Test Structure Reference

- `tests/should_ok/` — Programs that must compile and run successfully
- `tests/should_err/` — Programs that must produce compile errors
- `tests/eval/` — Evaluation/execution tests
- `tests/not_yet/` — Tests for unimplemented features
- `tests/linter/` — Linter rule tests
- `crates/erg_native/tests/` — Native compiler tests

## If Tests Fail

1. Show the failing test name and error output
2. Identify which compiler phase failed (parsing, type checking, codegen, etc.)
3. Suggest relevant source files to investigate based on the error
