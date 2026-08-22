# Never

It is a subtype of all types. It is a `Class` because it has all the methods and of course `.new`. However, it does not have an instance, and the Erg stops the moment it is about to be created.
There is also a type called `Panic` that does not have an instance, but `Never` is used for normal termination or an intentional infinite loop, and `Panic` is used for abnormal termination.

```python,checker_ignore
# Never <: Panic
f(): Panic = exit 0 # OK
g(): Panic = panic "..." # OK

may_fail(): Int or Panic = ...
h(): Never = may_fail() # TypeError: `Panic` is not `Never`
```

`panic`, `todo` and `unreachable` return `Never` rather than `Panic`, so that a stub like `f(): Int = todo()` keeps type-checking.

`T or Never` is `T`: `Never` is a semantically never-occurring option (if it does, the program stops immediately), so it is absorbed as soon as the union is built.
`T or Panic` is not absorbed — it stays in the signature, because it tells the reader the program may terminate — but a value of that type is still usable as a `T`, for the same reason. See [Panic](./Panic.md).