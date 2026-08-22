# Result T, E

```python,checker_ignore
Result T     == T or Error
Result(T, E) == T or E
```

Like `Option`, it represents "a value that may fail", but the failure carries context: `Error` has a `.msg`, a `.kind`, the hints attached with `.context`, and the `.stack` of subroutines it was returned out of.

It is an alias, not a type of its own. The error type defaults to [`Error`](./Error.md); pass a second argument for another one, such as a Python exception.

```python,checker_ignore
half(x: Int): Result Int = if x % 2 == 0, do x // 2, do Error "not even"

parse(s: Str): Result(Int, ValueError) = ...
```

The [`?`](../../special.md) operator recognizes both, so `?` on a `Result T` gives a `T` and returns the error from the enclosing subroutine otherwise.

```python,checker_ignore
quarter(x: Int): Result Int =
    y = half(x)?
    half(y)?
```

`Result(T, E)` and [`Either(T, E)`](./Either.md) are the same union; `Result` is the spelling that says which side is the error.
