---
name: pipeline
description: Trace how a language feature flows through the Erg compilation pipeline
argument-hint: "<feature or construct to trace>"
context: fork
agent: Explore
---

# Compilation Pipeline Tracer

Trace how "$ARGUMENTS" is handled through each stage of the Erg compiler.

## Investigation Steps

For each stage, find the relevant code and explain how it processes the given feature:

### 1. Lexing (crates/erg_parser/lex.rs)
How is it tokenized? What token types are produced?

### 2. Parsing (crates/erg_parser/parse.rs)
What AST node is generated? Check `crates/erg_parser/ast.rs` for the node definition.

### 3. Desugaring (crates/erg_parser/desugar.rs)
Is there any syntactic desugaring? What does it transform into?

### 4. AST Linking (crates/erg_compiler/link_ast.rs)
Is any linking needed (e.g., methods to class definitions)?

### 5. Lowering / Type Checking (crates/erg_compiler/lower.rs)
How is it lowered to HIR? What type inference or checking happens?
Check `crates/erg_compiler/context/` for type context operations.

### 6. Effect Check (crates/erg_compiler/effectcheck.rs)
Are there side-effect constraints?

### 7. Ownership Check (crates/erg_compiler/ownercheck.rs)
Are there ownership/move semantics involved?

### 8. Code Generation (crates/erg_compiler/codegen.rs)
What Python bytecode is generated? Reference `doc/EN/python/bytecode_instructions.md`.

## Reference Documentation

- Compiler overview: `doc/EN/compiler/overview.md`
- Phase details: `doc/EN/compiler/phases/01_lex.md` through `10_codegen.md`
- Type inference: `doc/EN/compiler/inference.md`
- Subtyping: `doc/EN/compiler/subtyping.md`
- HIR structure: `doc/EN/compiler/hir.md`

## Output Format

For each stage, provide:
1. The relevant source file and function
2. A brief explanation of the processing
3. Key code snippets showing the handling
