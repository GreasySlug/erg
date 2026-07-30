---
name: pyinterop
description: Debug Python interop issues (pyimport, .d.er stubs, module resolution)
argument-hint: "<module-name or issue description>"
context: fork
agent: Explore
---

# Python Interop Debugger

Investigate Python interoperability issues for "$ARGUMENTS".

## Investigation Steps

### 1. Module Resolution
- `.d.er` stub search: `crates/erg_compiler/context/register.rs` → `import_py_mod()` → `get_decl_path()`
- `.py` fallback (untyped): `import_py_mod()` → `resolve_py()` + sys_path search
- Path resolution: `crates/erg_common/io.rs` → `resolve_py()`, `resolve_decl_path()`
- Const function: `crates/erg_compiler/context/initialize/const_func.rs` → `resolve_decl_path_func()`

### 2. Type Stubs (.d.er)
- Standard library: `crates/erg_compiler/lib/pystd/`
- Core library: `crates/erg_compiler/lib/core.d/`

### 3. Untyped Pyimport (Obj propagation)
- Attribute access: `context/inquire.rs` → `get_attr_info()` PyModule fallback → `Obj`
- Chain access: `Obj.x → Obj` via `get_attr_info()` fallback
- Method calls: `search_method_info()` → `(...Obj) -> Obj`
- Direct calls: `search_callee_info()` → Obj handler

### 4. HIR Linking
- `crates/erg_compiler/link_hir.rs` → `replace_py_import()`: `pyimport "x"` → `__import__("a.x").x`

## Common Issues

| Issue | Cause |
|-------|-------|
| `ImportError: module X not found` | No `.d.er` or `.py` on sys_path |
| `PyModule("<failure>")` | `resolve_decl_path_func` fallback failed |
| Attributes not resolved | Empty PyModule context |
| `Obj` instead of proper types | No `.d.er` stub exists |
