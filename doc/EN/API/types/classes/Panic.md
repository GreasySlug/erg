# Panic

Abnormal termination. Like [`Never`](./Never.md) it has no instance, but `Never` means normal termination or a deliberate infinite loop, while `Panic` means the program gave up. `Never <: Panic`.

```python,checker_ignore
always_fails(): Panic = panic "boom"
always_exits(): Panic = exit 0
```

## `T or Panic`

Write `T or Panic` for a subroutine that returns a `T` but may terminate instead. Unlike `T or Never`, which collapses to `T`, this survives in the signature — that is the point of writing it.

```python
positive(x: Int): Int or Panic = if x > 0, do x, do panic "must be positive"
```

A value of that type is still usable as a plain `T`, because `Panic` has no instance: if it were one, the program would already have stopped.

```python,checker_ignore
positive(3) * 2      # OK, no narrowing needed
takes_int(positive(3))  # OK
```

A **bare** `Panic` is not a `T`, though: a subroutine declared to return `Panic` never returns normally, so there is no value to use. Write `T or Panic` when the subroutine may return.

```python,checker_ignore
f(): Int = always_fails() # TypeError
```

## What is not implemented

`panic`, `todo` and `unreachable` return `Never`, not `Panic`, so that a stub like `f(): Int = todo()` keeps type-checking.

Operations that may terminate are not typed with `Panic` yet: the `Div` trait is specified as `.'/' = (self: Self, R) -> Self.Output or Panic`, but division is still typed without it.
