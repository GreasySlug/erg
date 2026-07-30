---
name: lint
description: Run clippy and rustfmt checks, then fix any issues found
disable-model-invocation: true
allowed-tools: Bash(cargo clippy:*), Bash(cargo fmt:*)
---

# Lint Checker for Erg

Run all lint checks required by CI.

## Steps

1. Run format check:
   ```
   cargo fmt --all -- --check
   ```
2. Run clippy:
   ```
   cargo clippy --all --all-targets -- -D warnings
   ```
3. Report results summary

## If Issues Found

- **Format issues**: Run `cargo fmt --all` to auto-fix, then show the diff
- **Clippy warnings**: Show each warning with file location and suggested fix. Apply fixes following the Rust code guidelines from `doc/EN/dev_guide/rust_code_guideline.md`

## CI Requirements

- Zero clippy warnings (`-D warnings` treats all warnings as errors)
- All code must be formatted with rustfmt
- Tests must pass on Windows, Ubuntu, macOS
