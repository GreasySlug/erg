# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Erg is a statically typed programming language that compiles to Python bytecode. It features Rust-like robustness with Python interoperability. The compiler is written in Rust.

## Essential Commands

```bash
# Build
cargo build                        # Debug build

# Test
cargo test --features large_thread # All tests (use large_thread to prevent stack overflow)
cargo test --package erg_parser    # Test specific crate
cargo test -- --nocapture          # With output

# Lint
cargo clippy --all --all-targets -- -D warnings
cargo fmt --all -- --check

# Run
cargo run -- examples/hello_world.er          # Execute Erg file
cargo run -- --mode check examples/file.er    # Type check only
cargo run --features debug -- file.er         # Debug mode

# Install locally
cargo install --path . --features els
```

### Cargo Aliases (from .cargo/config.toml)

Common shortcuts: `cargo rd` (run with debug), `cargo b_full_re` (full release build), `cargo lex/prs/chk/cmp file.er` (run specific compiler stages).

## Architecture

### Workspace Crates

- **erg_common**: Shared utilities (config, logging, threading)
- **erg_parser**: Lexer and parser (TokenStream → AST)
- **erg_compiler**: Type checker and code generator (AST → HIR → Python bytecode)
- **erg_linter**: Code linting
- **els**: Language Server Protocol implementation
- **erg_proc_macros**: Internal procedural macros
- **erg_pydecl**: Converts Python type stubs (.pyi) to Erg declarations (feature `pydecl`)

### Compilation Pipeline

1. **Lexing** (erg_parser/lex.rs): Source → TokenStream
2. **Parsing** (erg_parser/parse.rs): TokenStream → AST
3. **Desugaring** (erg_parser/desugar.rs): Expand nested vars, desugar patterns
4. **AST Linking** (erg_compiler/link_ast.rs): Link class methods to definitions
5. **Lowering** (erg_compiler/lower.rs): AST → HIR (name resolution + type inference)
6. **Effect Check** (erg_compiler/effectcheck.rs): Verify side-effect safety
7. **Ownership Check** (erg_compiler/ownercheck.rs): Verify ownership rules
8. **Optimization** (erg_compiler/optimize.rs): Dead code elimination
9. **Code Generation** (erg_compiler/codegen.rs): HIR → Python bytecode

### Key Types

- `TokenStream`: Iterator of lexer tokens
- `AST`: Abstract Syntax Tree (Vec<Expr>)
- `HIR`: High-level Intermediate Representation with type information
- `CodeObj`: Python bytecode output

## Feature Flags

- `parallel` (default): Compiler parallelization
- `els`: Language Server support
- `debug`: Debug logging
- `full`: All user-facing features (els + full-repl + unicode + pretty)
- `large_thread`: Larger stack for tests
- `japanese/simplified_chinese/traditional_chinese`: Localized error messages
- `pydecl`: Parser-based .pyi → Erg declaration conversion (rustpython-parser)

## Python Type Stub (.pyi) Handling

When importing a Python module, `.d.er` declarations take priority; if absent, a `.pyi`
stub is resolved (`Input::resolve_pyi`, PEP 561 `-stubs` packages supported, gated by
`respect_pyi`, default on) and converted in-memory to Erg declaration syntax in
`build_package.rs::parse()`, then checked in "declare" mode (all symbols public).

- With feature `pydecl`: full conversion via `erg_pydecl` (rustpython-parser based;
  classes, generics via TypeVar quantifiers, overloads, Literal, Callable, etc.).
  Each generated chunk is validated with `SimpleParser`; invalid chunks are dropped.
- Without the feature (or on conversion failure): falls back to line-based
  `preprocess_pyi` (functions and variable annotations only, classes skipped).
- For untyped `.py`-only modules, `--use-pylyzer` invokes the external pylyzer
  (`pylyzer --dump-decl`) to generate `.d.er` files (unchanged behavior).
- Pitfall: `mod_name()` in erg_common/pathutil.rs must strip `.pyi` like `.py`/`.d.er`,
  otherwise class qual_names become `mod.pyi::Class` and instance types fail to resolve.

## CI Requirements

- Tests pass on Windows, Ubuntu, macOS with Python 3.7-3.11
- No clippy warnings (`-D warnings`)
- ELS tests use nextest with retries due to timing sensitivity

## Runtime Requirement

Python 3.7+ interpreter required. The build script (build.rs) detects Python version at compile time.
Use `--py-command <path>` to target a specific interpreter (e.g. `--py-command python3.13`).

## Python Bytecode Versioning

The codegen targets multiple Python bytecode versions (3.7–3.14). Version-specific logic is split across:

### Key Files

| File | Role |
| ---- | ---- |
| `crates/erg_common/opcodeNNN.rs` | Opcode enums with raw numeric values per Python version |
| `crates/erg_common/opcode.rs` | `CommonOpcode` enum — shared opcode names with 3.7–3.12 numeric values |
| `crates/erg_common/opcode_set.rs` | `OpcodeSetVersion` — dispatch layer: method per opcode, returns correct `u8` for target version |
| `crates/erg_compiler/codegen.rs` | Main codegen — uses `CommonOpcode::*` and `self.opcode_set.*()` |
| `crates/erg_common/serialize.rs` | Magic number, .pyc header format |

### Two Opcode Write Paths (Critical for 3.13+)

Python 3.13 renumbered ALL opcodes. The codegen has two write paths:

```rust
// 1. For CommonOpcode values — translates via opcode_set before writing
self.write_opcode(LOAD_CONST);  // CommonOpcode → translate_common() → version-specific byte

// 2. For opcode_set method results — already version-specific, no translation
self.write_instr(self.opcode_set.call());  // u8, already correct for target version
```

**These cannot be merged**: 3.13 opcode values collide with CommonOpcode values (e.g. `END_FOR=11` vs `UNARY_NEGATIVE=11`).

### Adding a New Python Version

1. Create `crates/erg_common/opcodeNNN.rs` with correct opcode values (get from `python -c "import opcode; ..."`)
2. Add variant to `OpcodeSetVersion` enum and `from_python_version()` in `opcode_set.rs`
3. Add `Self::VNNN =>` arms to every method in `opcode_set.rs` (~50 methods)
4. If opcodes are renumbered (like 3.13): update `translate_common()` match table
5. Check for new/removed/changed cache entries and update `cache_entries_*()` methods
6. Check for semantic changes (stack layout, instruction format, calling convention)
7. Add `tests/bytecodeNNN/` with `.er` test files, `python_ref/*.py`, `run_tests.sh`, `compare_bytecode.sh`

**Common pitfalls when adding versions:**

- `edit_code()` with raw `CommonOpcode::X as usize` bypasses translation — use `translate_common()` first (3.13+)
- `codegen_v311.rs:emit_binop_instr_311()` hardcodes `Opcode311` values — must use `opcode_set.*()` methods
- New opcodes before conditional jumps (e.g., `TO_BOOL` in 3.13+) affect ALL jump offset calculations
- FOR_ITER jump target must account for new post-loop opcodes (e.g., `POP_ITER` in 3.14)

### Version-Specific Differences Summary

| Change | 3.11 | 3.12 | 3.13 | 3.14 |
| ------ | ---- | ---- | ---- | ---- |
| PRECALL | yes | removed | removed | removed |
| LOAD_METHOD | separate opcode | merged into LOAD_ATTR | pseudo-op (=LOAD_ATTR) | pseudo-op |
| LOAD_CLOSURE | separate opcode | separate opcode | pseudo-op (=LOAD_FAST) | pseudo-op |
| KW_NAMES | yes | yes | removed → CALL_KW | removed |
| COMPARE_OP arg | raw cmp | `(cmp<<4)\|mask` | `(cmp<<5)\|mask` | `(cmp<<5)\|mask` |
| Opcode numbering | classic | classic | **completely renumbered** | renumbered (shifted from 3.13) |
| PUSH_NULL order | before callable | before callable | **after callable** | after callable |
| TO_BOOL before conditional jumps | no | no | **required (3 CACHE)** | required (3 CACHE) |
| POP_JUMP_IF_FALSE cache | 0 | 0 | 1 | 1 |
| JUMP_BACKWARD cache | 0 | 0 | 1 | 1 |
| BINARY_OP cache | 1 | 1 | 1 | **5** |
| BINARY_SUBSCR | yes | yes | yes | **removed → BINARY_OP 26** |
| MAKE_FUNCTION | flags arg | flags arg | **no arg → SET_FUNCTION_ATTRIBUTE** | no arg → SET_FUNCTION_ATTRIBUTE |
| FOR_ITER exhaustion | — | END_FOR (pops 2) | **END_FOR (pops 1) + POP_TOP** | END_FOR (pops 1) + POP_ITER |
| LOAD_ASSERTION_ERROR | yes | yes | yes | **removed → LOAD_COMMON_CONSTANT** |

### Closures (cells)

Codegen follows CPython's model; the pieces that have to agree are spread out:

- A variable a nested function reads lives in a **cell owned by the defining frame**,
  made in that frame's prologue (`MAKE_CELL`, before `COPY_FREE_VARS`/`RESUME`) —
  one cell per variable per frame, so closures made in a loop share the loop
  variable (late binding, as in Python and the transpiler). The closure tuple
  carries the cells; a callee never boxes its freevars.
- Which locals need cells comes from `collect_captures` (a pre-scan over the HIR using
  the `captured_names` lowering records) plus a per-function walk at emit time
  (`frame_cells`). Lambdas that codegen inlines (`if` branches, `for!`/`while!` bodies,
  `match` arms) are walked as part of the host frame — their `captured_names` include
  the host's own parameters, which must not become cells. Desugared lambdas
  (comprehensions) share a `Location`, so nothing is keyed by location.
- Slots are numbered as names are met, but `COPY_FREE_VARS` fills the *trailing* slots,
  so `remap_localsplus` renumbers every slot instruction at unit end to
  `CodeObj::localsplus_layout` (locals in order, captured ones marked cells in place,
  freevars last). The serializer uses the same function; they must not disagree.
- Before 3.11, a deref/`LOAD_CLOSURE` indexes `cellvars ++ freevars`, and the closure
  tuple is built in the *callee's* freevars order.
- Variables bound in inlined control blocks (`if` branches, loop bodies, `match` arms,
  `with!` blocks) and the blocks' parameters are fast locals of the host frame
  (`store_kind` / `block_param_kind`, keyed on `UnitKind`). They used to be stored by
  name — and a function frame without `CO_OPTIMIZED` uses `func_globals` as its locals,
  so they leaked into the module's globals and every closure over them read the last
  call's value. `VarInfo::is_fast_value` cannot decide this: it looks at the scope
  lowering gave the variable, which is the block's own.
- Lowering's `current_true_function_ctx` judges a scope by its kind, not by what it is
  lowering at the moment; otherwise captures inside a `for!` body are never recorded.

Regression file: `tests/should_ok/nested_closure.er` (run it on every Python version).

### Bytecode Test Suites

```bash
# Run version-specific tests
bash tests/bytecode312/run_tests.sh [python_path]
bash tests/bytecode313/run_tests.sh [python_path]
bash tests/bytecode314/run_tests.sh [python_path]

# Compare Erg vs CPython bytecode
bash tests/bytecode314/compare_bytecode.sh [python_path] [test_name]
```

## ELS (Erg Language Server) Architecture

### Thread Structure

ELS spawns up to 30 threads at startup:

| Thread | Location | Purpose |
|--------|----------|---------|
| `Server::run` | server.rs | Main message loop |
| `load_modules` | completion.rs | External module loading (auto-terminates) |
| `start_auto_diagnostics` | diagnostics.rs | Periodic diagnostics |
| `start_workspace_diagnostics` | diagnostics.rs | Initial workspace check (auto-terminates) |
| `start_client_health_checker_sender` | diagnostics.rs | Health check requests |
| `start_client_health_checker_receiver` | diagnostics.rs | Health check responses |
| LSP workers (24) | server.rs | One per LSP feature (completion, hover, etc.) |

### Key Components

- **`Server<Checker, Parser>`**: Main LSP server struct, cloned for each worker thread
- **`SendChannels` / `ReceiveChannels`**: MPSC channels for worker communication
- **`WorkerMessage<P>`**: Either `Request(id, params)` or `Kill` for graceful shutdown
- **`BackgroundThreads`**: Manages kill channels for threads that need explicit termination
- **`Scheduler`**: Prioritizes LSP requests, limits concurrent workers to `MAX_WORKERS` (10)
- **`Shared<T>`**: Thread-safe wrapper using `Arc<RwLock<T>>`

### Thread Termination

Background threads use kill channels for graceful shutdown:

```rust
// Setting up a killable thread
let (kill_tx, kill_rx) = mpsc::channel::<()>();
self.background_threads.set_auto_diagnostics_killer(kill_tx);
spawn_new_thread(move || {
    loop {
        if kill_rx.try_recv().is_ok() { break; }  // Non-blocking check
        // ... work ...
    }
}, "thread_name");

// On restart, kill all background threads
self.background_threads.kill_all();
```

### Testing ELS

```rust
// Create fake client for testing
let mut client = Server::bind_fake_client();
client.request_initialize()?;
client.notify_initialized()?;
client.notify_open("tests/file.er")?;
let hover = client.request_hover(uri, line, col)?;
```

Use `#[exec_new_thread]` attribute for tests requiring large stack size.

## Claude Code Skills (Slash Commands)

| Skill | Usage | Description |
| ----- | ----- | ----------- |
| `/test` | `/test [crate or test_name]` | Run tests with `--features large_thread`. ELS uses nextest with retries |
| `/lint` | `/lint` | Run `cargo fmt --check` + `cargo clippy -D warnings`, fix issues |
| `/run-er` | `/run-er [--mode check\|lex\|parse\|native] <file>` | Execute or analyze Erg source files through compiler stages |
| `/ci-check` | `/ci-check` | Run full CI checklist locally (fmt → clippy → build → test) |
| `/build` | `/build [full\|els\|debug\|native\|release]` | Build with specific feature flag combinations |
| `/pipeline` | `/pipeline <feature>` | Trace how a language feature flows through the compilation pipeline |
| `/add-test` | `/add-test <ok\|err> <feature>` | Add a should_ok / should_err test case |
| `/error-msg` | `/error-msg <text or ErrorKind>` | Find, explain, or edit error messages across all languages |
| `/pyinterop` | `/pyinterop <module or issue>` | Debug Python interop issues (pyimport, .d.er stubs, module resolution) |
| `/pyver` | `/pyver <ver> [compile\|run\|test\|dis\|compare\|opcodes] [file]` | Compile and test against a specific Python version (3.10-3.14 via uv) |

## Claude Code Agents (Task Subagents)

| Agent | When to Use |
| ----- | ----------- |
| `erg-type-explorer` | Explore type inference, subtyping, type checking implementation |
| `els-debugger` | Debug ELS (Language Server) issues, LSP features, threading, test failures |
| `ast-analyzer` | Analyze AST/HIR node structures, parsing, and desugaring |
| `native-compiler` | Debug/develop the native compiler (Galois) — LLVM codegen, type mapping, linking |
| `bytecode-codegen` | Debug Python bytecode generation — opcode versioning, cache entries, stack layout, .pyc format |
| `pyinterop-debugger` | Debug Python interop — pyimport, .d.er stubs, module resolution, untyped imports |
| `Explore` | Broad codebase exploration (when 3+ queries are needed) |
| `Plan` | Design implementation plans for new features |

## Native Compiler (Galois)

Experimental LLVM-based native compiler. Crate: `crates/erg_native/`, feature flag: `gal`.

```text
Source → Parser → AST → Lowering → HIR ──┬──→ PyCodeGenerator → .pyc (default)
                                          └──→ LLVMCodeGenerator → LLVM IR → .o → ELF (native)
```

### Currently Supported

- Int/Nat/Float/Bool/Str literals
- Binary operators (+, -, *, /, //, %, <, >, <=, >=, ==, !=)
- Unary operators (-, not)
- Variable definitions and access (alloca/store/load)
- `print!` function (printf-based, type-dispatched format strings)
- `if!`/`if` conditional expressions (LLVM basic block branching with phi nodes)
- `for!`/`for` Range-based loops (counter variable with alloca/store/load, supports `..`, `..<`, `<..`, `<..<`)
- `while!` loops (condition thunk re-evaluation per iteration)
- TypeAsc (type ascriptions are transparently skipped)

### Native Compiler Key Files

| File | Purpose |
| ---- | ------- |
| `src/lib.rs` | `NativeCompiler` (`Runnable` impl), `build_hir()`, `compile_to_native()` |
| `src/codegen.rs` | `LLVMCodeGenerator`: HIR → LLVM IR (emit_expr, emit_binop, emit_if, etc.) |
| `src/types.rs` | Erg Type → LLVM type mapping, `base_type()`, `is_*_type()` |
| `src/builtins.rs` | `print!` → printf (type-based format strings) |
| `src/error.rs` | `NativeCompileError` (Unsupported, Undefined, TypeMismatch, Link, LLVM) |
| `src/linker.rs` | `emit_object_file()` + `link_executable(cc)` |

### Testing

```bash
cargo test -p erg_native  # large_thread via dev-dependency
```

### Key Pitfalls

- Erg types are often wrapped in Refinement types (e.g., `"hello"` → `{"hello"}` refining `Str`) → use `base_type()` to unwrap
- Comparison operators return `Type::Guard` (for type narrowing), not `Bool` → `base_type()` maps Guard → Bool
- Linux requires `RelocMode::PIC` (PIE requirement)
- All inkwell builder methods return `Result` → wrap with `.map_err()`
