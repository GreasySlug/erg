# `erg` build features

## debug

Enter debug mode. As a result, the behavior inside Erg is sequentially displayed in the log. Also, enable `backtrace_on_stack_overflow`.
Independent of Rust's `debug_assertions` flag.

## backtrace

Enable only `backtrace_on_stack_overflow`.

## japanese

Set the system language to Japanese.
Erg internal options, help (help, copyright, license, etc.) and error display are guaranteed to be Japanese.

## simplified_chinese

Set the system language to Simplified Chinese.
Erg internal options, help (help, copyright, license, etc.) and errors are displayed in Simplified Chinese.

## traditional_chinese

Set the system language to Traditional Chinese.
Erg internal options, help (help, copyright, license, etc.) and errors are displayed in Traditional Chinese.

## unicode/pretty

The compiler makes the display rich.

## large_thread

Increase the thread stack size. Used for Windows execution and test execution.

## els

`--language-server` option becomes available.
`erg --language-server` will start the Erg language server.

## full-repl

Enable the rich REPL: cursor movement, pasting, history, and so on.

## full

Enable every user-facing feature (`els` + `full-repl` + `unicode` + `pretty`).

## pydecl

Convert a Python type stub (`.pyi`) into Erg declarations with `erg_pydecl`, which parses the stub
(classes, generics, overloads, `Literal`, `Callable`, ...).
Without this feature the compiler falls back to a line-based preprocessor that reads only functions
and variable annotations, and skips classes.

## py_compat

Enable Python-compatible mode, which makes parts of the APIs and syntax compatible with Python. Used for [pylyzer](https://github.com/mtshiba/pylyzer).

## experimental

Enable experimental features.

## log-level-error

Only display error logs.

## parallel

Enable compiler parallelization. Default is enabled.
