# Either L, R = L or R

A type that represents "either L or R". You can think of it as a two-limited form of the Or type.

It is an alias, not a type of its own: `Either(Int, Str)` **is** `Int or Str`, and the two are interchangeable wherever a type is expected. [`Option`](./Option.md) and [`Result`](./Result.md) are the same machinery with one side fixed.

```python
pick(x: Int): Either(Int, Str) = if x > 0, do x, do "negative"
```

Because it is the plain Or type, it is symmetric: `Either` carries no tag saying which side a value came from, so the sides are told apart by narrowing, and `Either(Int, Int)` is simply `Int`.

```python,checker_ignore
picked = pick(-1)
assert picked in Str
picked.upper()
```

## methods

Not implemented. The plan:

* orl
* orr
* andl
* andr
* mapl
* mapr

These need to know at runtime which side a value is on, which an untagged union cannot answer when `L` and `R` overlap.
