# Option T = T or NoneType

A type that represents "may fail". It is an alias, not a type of its own: `Option Int` **is** `Int or NoneType`, and the two are interchangeable wherever a type is expected.

```python
x: Option Int = 1
y: Option Int = None

first(l: List(Int, 3)): Option Int = l.get(0)
```

The [`?`](../../special.md) operator recognizes `NoneType` as the error alternative, so `?` on an `Option T` gives a `T` and returns `None` from the enclosing subroutine otherwise.

```python
sum_two(l: List(Int, 3)): Option Int =
    a = l.get(0)?
    b = l.get(1)?
    a + b
```

## methods

These resolve the failure here and now, where `?` hands it to the caller instead.
They are available on any `T or E` whose `E` is an error alternative, so a
[`Result`](./Result.md) has exactly the same four.

* unwrap(self, msg := Str) -> T or [Panic](./Panic.md)

Extract it expecting the contents to be `T` type. If it is `None`, output `msg` and panic.

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap() == 1
```

```python,checker_ignore
x: Option Int = None
x.unwrap() # UnwrappingError: unwrapped a None value
x.unwrap("failed to convert from string to number") # UnwrappingError: failed to convert from string to number
```

The hints and the `?` frames the value collected on its way up are printed with it,
so the trace survives being unwrapped.

* unwrap_or(self, else: T) -> T

Extract it, or evaluate to `else` if it is `None`. `else` is evaluated either way.

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap_or(0) == 1
```

* unwrap_or_exec(self, f: () -> T) -> T

The same, except that `f` is called only when there is nothing to extract.

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap_or_exec(() -> 0) == 1
```

* unwrap_or_exec!(self, p!: () => T) -> T

`unwrap_or_exec` for a fallback that has a side effect. Being a procedure, it can
only be called from one.

```python
log! = !0
recompute!() =
    log!.inc!()
    0

main!() =
    first(l: List(Int, 3)): Option Int = l.get(0)
    assert first([1, 2, 3]).unwrap_or_exec!(recompute!) == 1
    assert log! == 0

main!()
```
