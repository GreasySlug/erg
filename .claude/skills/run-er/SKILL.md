---
name: run-er
description: Run or type-check an Erg source file
argument-hint: "[--mode check|lex|parse|desugar|lower|compile|native] <file.er>"
disable-model-invocation: true
allowed-tools: Bash(cargo run:*), Bash(cargo test -p erg_native:*)
---

# Erg File Runner

Execute or analyze Erg source files through various compiler stages.

## Usage

- `/run-er <file>` — Execute: `cargo run -- $ARGUMENTS`
- `/run-er --mode check <file>` — Type check only
- `/run-er --mode lex <file>` — Show lexer output
- `/run-er --mode parse <file>` — Show parsed AST
- `/run-er --mode native <file>` — Native compile (LLVM): `cargo run --features gal -- --mode native $FILE`
- `/run-er --debug <file>` — Run with debug output: `cargo run --features debug -- $ARGUMENTS`

## Cargo Aliases (shortcuts)

- `cargo lex <file>` — Lexer stage only
- `cargo prs <file>` — Parse stage only
- `cargo chk <file>` — Type check only
- `cargo cmp <file>` — Full compile
- `cargo rd <file>` — Run with debug features

## Native Compilation (Galois)

To compile to native via LLVM:

```bash
cargo run --features gal -- --mode native <file>
```

This produces an ELF binary via HIR → LLVM IR → object → link.

## Compilation Pipeline

1. **Lex** → TokenStream (crates/erg_parser/lex.rs)
2. **Parse** → AST (crates/erg_parser/parse.rs)
3. **Desugar** → Simplified AST (crates/erg_parser/desugar.rs)
4. **Lower** → HIR with types (crates/erg_compiler/lower.rs)
5. **Effect Check** → Side-effect safety (crates/erg_compiler/effectcheck.rs)
6. **Ownership Check** → Move semantics (crates/erg_compiler/ownercheck.rs)
7. **Codegen** → Python bytecode (crates/erg_compiler/codegen.rs)

## If Errors Occur

Identify which compilation phase produced the error and explain it in the context of Erg's type system and semantics.
