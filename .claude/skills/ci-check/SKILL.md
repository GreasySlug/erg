---
name: ci-check
description: Run all CI checks locally before pushing
disable-model-invocation: true
allowed-tools: Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Bash(cargo build:*)
---

# Local CI Check

Run the full CI checklist locally to catch issues before pushing.

## Steps

Run these checks in order. Stop and report on first failure:

### 1. Format Check
```
cargo fmt --all -- --check
```
If failed: run `cargo fmt --all` to fix, then show diff.

### 2. Clippy
```
cargo clippy --all --all-targets -- -D warnings
```
If failed: show warnings and fix them following `doc/EN/dev_guide/rust_code_guideline.md`.

### 3. Build
```
cargo build
```

### 4. Tests
```
cargo test --features large_thread
```

### 5. Report Summary

```
Format:   PASS/FAIL
Clippy:   PASS/FAIL (N warnings)
Build:    PASS/FAIL
Tests:    PASS/FAIL (N passed, N failed, N ignored)
```

## CI Environment Reference

- Platforms: Windows, Ubuntu, macOS
- Python: 3.7-3.11
- ELS tests use nextest with retries
- Feature flags: see `doc/EN/dev_guide/build_features.md`
