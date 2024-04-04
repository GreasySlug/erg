# Type erasure

Type erasure is the process of setting a type argument to `_` and deliberately discarding its information. Type erasure is a feature of many polymorphic languages, but in the context of Erg's syntax, it is more accurate to call it type argument erasure.

The most common example of a type that has been type-erased is `[T, _]`. Lists are not always known their length at compile-time. For example, `sys.argv`, which refers to command line arguments, is of type `[Str, _]`. Since Erg's compiler has no way of knowing the length of command line arguments, information about their length must be given up.
However, a type that has been type-erased becomes a supertype of a type that has not been (e.g. `[T; N] <: [T; _]`), so it can accept more objects.
Objects of type `[T; N]` can of course use methods of type `[T; _]`, but the `N` information is erased after use. If the length does not change, then it is possible to use `[T; N]` in the signature. If the length remains the same, it must be indicated by a signature.

```python
# Functions that are guaranteed to not change the length of the array (e.g., sort)
f: [T; N] -> [T; N] # functions that do not (f: [T; N])
# functions that do not (e.g. filter)
g: [T; n] -> [T; _]
```

If you use `_` in the type specification itself, the type is upcast to `Object`.
For non-type type arguments (Int, Bool, etc.), the parameter with `_` will be undefined.

```python
i: _ # i: Object
[_; _] == [Object; _] == List
```

Type erasure is not the same as omitting a type specification. Once the type argument information has been erased, it will not be returned unless you assert it again.

```python
implicit = (1..5).iter().map(i -> i * 2).to_arr()
explicit = (1..5).iter().map(i -> i * 2).into(List(Nat))
```

In Rust, this corresponds to the following code.

```rust
let partial = (1..6).iter().map(|i| i * 2).collect::<Vec<_>>();
```

Erg does not allow partial omission of types, but uses higher-order kind polymorphism instead.

```python
# collect is a higher-order Kind method that takes Kind
hk = (1..5).iter().map(i -> i * 2).collect(List)
hk: List(Int)
```
