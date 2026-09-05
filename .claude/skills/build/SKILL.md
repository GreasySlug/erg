---
name: build
description: Build Erg with various feature flag combinations
argument-hint: "[feature-set: full|els|debug|native|release]"
disable-model-invocation: true
allowed-tools: Bash(cargo build:*), Bash(cargo install:*)
---

# Erg Build Tool

Build the Erg compiler with specific feature combinations.

## Usage

- `/build` — Default debug build: `cargo build`
- `/build full` — All user-facing features: `cargo build --features full`
- `/build els` — With language server: `cargo build --features els`
- `/build debug` — With debug logging: `cargo build --features debug`
- `/build native` — With native compiler (Galois): `cargo build --features gal`
- `/build release` — Optimized release build: `cargo build --release --features full`
- `/build install` — Install locally: `cargo install --path . --features full`

## Feature Flags Reference

| Flag | Description |
|------|-------------|
| `parallel` | Compiler parallelization (default, always on) |
| `els` | Language Server Protocol support |
| `debug` | Debug logging output |
| `full` | All user-facing features (els + full-repl + unicode + pretty) |
| `full-repl` | Full REPL with history and completion |
| `unicode` | Unicode symbol support in output |
| `pretty` | Pretty-printed output |
| `large_thread` | Larger stack for tests (not for builds) |
| `gal` | Native compiler via LLVM (only on `main` / `feat-native`; `experimental` has the flag, not the crate) |
| `japanese` | Japanese error messages |
| `simplified_chinese` | Simplified Chinese error messages |
| `traditional_chinese` | Traditional Chinese error messages |

## Cargo Aliases

- `cargo b_full_re` — Full release build

## If Build Fails

1. Check that Python 3.7-3.14 is available (`python3 --version`)
2. For native compiler: ensure LLVM 14+ is installed (`llvm-config --version`)
3. Check feature flag compatibility
4. Show the full error output for diagnosis
