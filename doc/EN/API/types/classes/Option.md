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

Not implemented yet. The plan:

* unwrap(self, msg = "unwrapped a None value") -> T or Panic

Extract it expecting the contents to be `T` type. If it is `None`, output `msg` and panic.

```python,checker_ignore
x = "...".parse(Int).into(Option Int)
x.unwrap() # UnwrappingError: unwrapped a None value
x.unwrap("failed to convert from string to number") # UnwrappingError: failed to convert from string to number
```

* unwrap_or(self, else: T) -> T

* unwrap_or_exec(self, f: () -> T) -> T

* unwrap_or_exec!(self, p!: () => T) -> T
