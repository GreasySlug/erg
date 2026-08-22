# Option! T

A cell holding an [`Option T`](./Option.md) whose content can be replaced.

`Option` is only an alias for `T or NoneType`, so it cannot have a mutable counterpart
the usual way: a union is not a nominal type, so there is nothing for `Mutizable` to be
implemented on and `!` cannot produce one. `Option!` is therefore a class of its own,
and its instances are made by calling it rather than by mutating an `Option`.

```python
name = Option!("Jane Doe")
print! name.get() # Jane Doe

name.set! "John Doe"
print! name.get() # John Doe

name.clear!()
print! name.get() # None
```

## methods

* `__call__`(value := T) -> Option! T

Make a cell holding `value`, or an empty one if no value is given.

* get(self: Ref(Option! T)) -> T or NoneType

The value in the cell right now. It is an ordinary [`Option T`](./Option.md), so
[`?`](../../special.md), `.unwrap` and narrowing all read it as they read any other one.

```python
c: Option! Int = Option!(1)
assert c.get().unwrap() == 1

incr(opt: Option! Int): Int or NoneType =
    x = opt.get()?
    x + 1

print! incr(c) # 2
```

* set!(self: RefMut(Option! T), value: T or NoneType) => NoneType

Put `value` in the cell. Passing `None` empties it.

* clear!(self: RefMut(Option! T)) => NoneType

Empty the cell. `o.clear!()` is `o.set!(None)`.

There is no `.update!`. It would have to take a `(T or NoneType) -> (T or NoneType)`,
and a lambda whose parameter is a union holding a type variable cannot be inferred yet.
Read, apply and write back instead:

```python
counter: Option! Int = Option!(0)
counter.set!(counter.get().unwrap_or(0) + 1)
print! counter.get() # 1
```

## The element type

`T` comes from the value the cell is made with, and a type annotation does not widen it
— `Option!` is covariant in `T`, as `List!` is, so `Option!({1})` already satisfies
`Option! Int`. Writing another value of the same type is still fine:

```python
o = Option!("a")
o.set! "b"
print! o.get() # b
```

An `Option!()` made with no value has nothing to take `T` from, and an annotation
cannot supply it either; `T` is settled by the first `.set!`. Until then, only
operations that do not depend on `T` (such as `print!`) are available.

```python
empty = Option!()
print! empty.get() # None
empty.set! "now a Str"
print! empty.get() # now a Str
```
