# User-Defined Const Functions Implementation

## Overview

This document describes the implementation of user-defined const functions (compile-time functions) in Erg. Const functions allow computation at compile time with the following characteristics:

- Function name begins with an uppercase letter (e.g., `Add`, `MyDouble`)
- All arguments must be constants with explicit type annotations
- Only constant expressions can be used in the function body
- Computation is done at compile time

### Example Usage

```python
Add(X, Y: Nat): Nat = X + Y
assert Add(1, 2) == 3

Double(X: Nat): Nat = X * 2
assert Double(5) == 10
```

## Module: erg_compiler

### File: crates/erg_compiler/context/register.rs

#### Key Changes

##### 1. Added Imports

```rust
use erg_parser::Parser;  // For validate_const_block
use crate::ty::constructors::subr_t;  // For building function signature types
use crate::ty::{ConstSubr, SubrKind, UserConstSubr};  // Const subroutine types
```

##### 2. Modified `register_def` (around line 1508)

The key insight: const functions with parameters should create a `UserConstSubr` object instead of evaluating the body immediately.

**Before:** All const subroutines were evaluated immediately, which failed when the body referenced parameters.

**After:** Check if the signature has parameters:
- **With parameters (const function):** Create `UserConstSubr` via `register_const_subr`
- **Without parameters (const value):** Evaluate immediately (existing behavior)

```rust
ast::Signature::Subr(sig) => {
    if sig.is_const() {
        if !sig.params.is_empty() {
            // Const function: create UserConstSubr
            let obj = match self.register_const_subr(sig, &def.body.block, def.def_kind()) {
                Ok(obj) => obj,
                Err((obj, es)) => { errs.extend(es); obj }
            };
            if let Err(es) = self.register_gen_const(...) { errs.extend(es); }
        } else {
            // Const value: evaluate immediately (existing code)
        }
    }
}
```

##### 3. New Method: `register_const_subr` (lines 1908-2062)

Creates a `UserConstSubr` for const functions:

1. **Type bounds instantiation:** Process type bounds from signature
2. **Parameter type instantiation:** Instantiate non-default params, var params, default params, kw_var_params
3. **Return type instantiation:** Extract from `sig.return_t_spec` (required for const functions)
4. **Const block validation:** Use `Parser::validate_const_block()` to convert AST to const-evaluable form
5. **Signature type construction:** Build `Type::Subr` using `subr_t()`
6. **Create UserConstSubr:** Wrap in `ValueObj::Subr(ConstSubr::User(...))`

```rust
fn register_const_subr(
    &mut self,
    sig: &ast::SubrSignature,
    block: &ast::Block,
    _def_kind: ast::DefKind,
) -> Failable<ValueObj> {
    // ... type instantiation code ...

    let const_block = match Parser::validate_const_block(block.clone()) {
        Ok(block) => block,
        Err(err) => {
            let err = TyCheckError::feature_error(...);
            return Err((ValueObj::None, TyCheckErrors::from(err)));
        }
    };

    let sig_t = subr_t(
        SubrKind::Func,
        non_default_params,
        var_params,
        default_params,
        kw_var_params,
        return_t,
    );

    let user_subr = UserConstSubr::new(
        sig.ident.inspect().clone(),
        sig.params.clone(),
        const_block,
        sig_t,
    );

    Ok(ValueObj::Subr(ConstSubr::User(user_subr)))
}
```

##### 4. Modified `register_gen_const` for `ValueObj::Subr`

When registering a const subroutine, use the actual function signature type and store in `locals`:

```rust
ValueObj::Subr(ref subr) => {
    let id = DefId(get_hash(ident));
    let sig_t = subr.sig_t().clone();  // Use actual signature type
    let vi = VarInfo::new(sig_t, Const, ...);
    self.index().register(ident.inspect().clone(), &vi);
    self.locals.insert(ident.name.clone(), vi);  // Store in locals, not decls
    self.consts.insert(ident.name.clone(), obj);
    Ok(())
}
```

##### 5. Modified `assign_subr` for const subroutines

Handle const subroutines that are already registered in `locals`:

```rust
if sig.ident.is_const() {
    // For const subroutines (ValueObj::Subr), already in locals
    if let Some(vi) = self.locals.get(sig.ident.inspect()).cloned() {
        return Ok(vi);
    }
    // For other const values, move from decls to locals
    let vi = self.decls.remove(sig.ident.inspect()).unwrap();
    self.locals.insert(sig.ident.name.clone(), vi.clone());
    return Ok(vi);
}
```

## Module: erg_compiler/context/eval.rs

### Existing Infrastructure Used

The implementation leveraged existing const evaluation infrastructure:

- **`UserConstSubr`:** Struct representing user-defined const subroutines
- **`ConstSubr` enum:** Wraps `BuiltinConstSubr` and `UserConstSubr`
- **`eval_const_call`:** Evaluates const function calls
- **`call()` method on ConstSubr:** Executes the const function with arguments

### Pattern Reference: `eval_const_lambda` (lines 1312-1466)

The `register_const_subr` implementation follows the same pattern as `eval_const_lambda`:

```rust
// From eval_const_lambda (reference pattern)
let block = Parser::validate_const_block(lambda.body.clone())?;
let sig_t = subr_t(
    SubrKind::from(lambda.op.kind),
    non_default_params,
    var_params,
    default_params,
    kw_var_params,
    return_t,
);
let subr = ConstSubr::User(UserConstSubr::new(name, params, block, sig_t));
```

## Key Learnings

### 1. Const vs Non-Const Registration Flow

| Aspect | Const Values | Const Functions | Regular Functions |
|--------|--------------|-----------------|-------------------|
| Storage | `consts` | `consts` + `locals` | `locals` |
| Type | Evaluated value type | `Type::Subr` signature | `Type::Subr` signature |
| Body | Evaluated immediately | Stored as `ConstBlock` | Compiled to bytecode |

### 2. Why `locals` Instead of `decls`?

- `decls`: Forward declarations waiting to be defined
- `locals`: Fully defined local variables/functions
- Const subroutines are fully defined at registration time, so they go directly to `locals`

### 3. Type Instantiation for Const Functions

Const functions require explicit return type annotation because:
- Body is not evaluated at definition time
- Type inference cannot determine return type without evaluation
- `sig.return_t_spec` must be present and is used for type construction

### 4. Avoiding Name Conflicts

Built-in traits like `Add`, `Sub`, `Mul` exist in Erg's standard library. User-defined const functions should use unique names (e.g., `MyAdd`) to avoid conflicts.

## Test File

### tests/should_ok/const_func.er

```python
# Test for user-defined const functions (compile-time functions)

# Basic const function with two parameters
MyAdd(X, Y: Nat): Nat = X + Y
assert MyAdd(1, 2) == 3
assert MyAdd(10, 20) == 30

# Single parameter const function
MyDouble(X: Nat): Nat = X * 2
assert MyDouble(5) == 10
assert MyDouble(0) == 0

# Const function with subtraction
MySub(X, Y: Nat): Nat = X - Y
assert MySub(10, 3) == 7

# Const function returning a computed value
MySquare(X: Nat): Nat = X * X
assert MySquare(4) == 16
assert MySquare(3) == 9
```

## Future Work (Out of Scope)

- Recursive const functions (e.g., `Factorial`)
- Multi-pattern const function definitions
- Type parameter support in const functions

## Related Files

- [crates/erg_compiler/context/register.rs](../crates/erg_compiler/context/register.rs) - Main implementation
- [crates/erg_compiler/context/eval.rs](../crates/erg_compiler/context/eval.rs) - Const evaluation infrastructure
- [crates/erg_compiler/ty/const_subr.rs](../crates/erg_compiler/ty/const_subr.rs) - `UserConstSubr` definition
- [doc/EN/syntax/04_function.md](../doc/EN/syntax/04_function.md) - User documentation
- [tests/should_ok/const_func.er](../tests/should_ok/const_func.er) - Test cases
- [tests/should_ok/comptime.er](../tests/should_ok/comptime.er) - Related compile-time tests
