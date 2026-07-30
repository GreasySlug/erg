---
name: error-msg
description: Find, explain, or edit compiler error messages across all languages
argument-hint: "<error message text or ErrorKind>"
context: fork
agent: Explore
---

# Error Message Explorer

Search for and analyze Erg compiler error messages related to "$ARGUMENTS".

## Investigation Steps

### 1. Find the error definition
Search for the error in these locations:
- `crates/erg_compiler/error/` — Compiler errors (type errors, name errors, etc.)
- `crates/erg_parser/error/` — Parser errors (syntax errors)
- `crates/erg_common/error/` — Common error types

### 2. Identify the ErrorKind
Find the `ErrorKind` enum variant that produces this error.

### 3. Find where it's raised
Search for where this error is constructed and returned in the compiler pipeline:
- Parser errors: `crates/erg_parser/parse.rs`, `crates/erg_parser/lex.rs`
- Type errors: `crates/erg_compiler/context/`, `crates/erg_compiler/lower.rs`
- Effect errors: `crates/erg_compiler/effectcheck.rs`
- Ownership errors: `crates/erg_compiler/ownercheck.rs`

### 4. Check localized messages
Erg supports multiple languages for error messages. Check:
- English (default)
- Japanese (`--features japanese`)
- Simplified Chinese (`--features simplified_chinese`)
- Traditional Chinese (`--features traditional_chinese`)

Reference: `doc/EN/dev_guide/i18n_messages.md` for i18n conventions.

### 5. Find related test cases
Search `tests/should_err/` for tests that trigger this error.

## Output Format

1. ErrorKind variant name
2. Source location(s) where the error is raised
3. The error message in all available languages
4. Test case(s) that trigger this error
5. Conditions that cause this error
