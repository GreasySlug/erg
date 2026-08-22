# Error

Erg's own recoverable error. Returning `T or Error` is how a subroutine reports a failure the caller is expected to handle, as opposed to `Panic`, which ends the program.

```python
half(x: Int): Int or Error =
    if x % 2 == 0:
        do x // 2
        do Error("not even", kind := "ValueError")
```

## Attributes

* `.msg: Str` — what went wrong
* `.kind: Str` — the kind of error, `"Error"` unless given (`Error(msg, kind := ...)`)
* `.stack: List(ErrorFrame, _)` — the subroutines the error was returned out of by [`?`](../../special.md), innermost call first

`.msg` and `.kind` are not context: they belong to the error as it was created and are never overridden.

## Methods

### `.context(self, msg: Str) -> Error`

Consumes the error and returns a new one carrying one more hint. Calls are chainable, so an error can hold several contexts.

```python
not_ready(): Error =
    e = Error "not implemented yet"
    e.context("to be implemented in ver 1.2").context("and more hints ...")
```

## Stack trace

Each time an `Error` is returned by `?`, that call site is pushed onto `.stack`. When a `?` is reached where no `return` is possible — at the top level — the collected trace is printed and the program exits.

```python,checker_ignore
try_some(x: Int): Int or Error = ...

f x =
    y = try_some(x)?
    ...

g x =
    y = f(x)?
    ...

i = g(1)?
# Traceback (most recent call first):
#   f, line 5, file "foo.er"
#   5 | y = try_some(x)?
#   g, line 8, file "foo.er"
#   8 | y = f(x)?
#   <module>, line 11, file "foo.er"
#   11 | i = g(1)?
# Error: ...
```

`ErrorFrame` is one entry of that trace, with `.name: Str`, `.line: Nat` and `.file: Str`.
