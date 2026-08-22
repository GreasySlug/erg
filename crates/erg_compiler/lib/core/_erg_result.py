from _erg_type import MutType


class ErrorFrame:
    """A subroutine an `Error` was propagated out of by the `?` operator.

    `Error.stack` is a list of these, innermost call first.
    """

    def __init__(self, name, line, file):
        self.name = name
        self.line = line
        self.file = file

    def __repr__(self):
        return '{}, line {}, file "{}"'.format(self.name, self.line, self.file)


class Error:
    def __init__(self, msg, kind="Error"):
        self.msg = msg
        self.kind = kind
        # call sites `?` propagated this error through, innermost first
        self.stack = []
        # hints attached with `.context`, oldest first
        self.contexts = []

    # kept for compatibility with code that predates `.msg`
    @property
    def message(self):
        return self.msg

    def context(self, msg):
        """Return a copy of this error carrying one more hint.

        `.msg` and `.kind` are not context and are never overridden.
        """
        new = Error(self.msg, self.kind)
        new.stack = list(self.stack)
        new.contexts = self.contexts + [msg]
        return new

    def __repr__(self):
        return "{}: {}".format(self.kind, self.msg)



class _TypeAlias:
    """A type-level alias, usable at runtime as both `Alias(T)` and `Alias[T]`.

    Erg emits a type application as a subscript and a plain call as a call, and an
    alias can appear either way, so both are accepted.
    """

    def __init__(self, name, expand):
        self.__name__ = name
        self._expand = expand

    def __call__(self, *args):
        return self._expand(*args)

    def __getitem__(self, args):
        if isinstance(args, tuple):
            return self._expand(*args)
        return self._expand(args)

    def __repr__(self):
        return self.__name__


def _option(t):
    from _erg_type import UnionType

    return UnionType(t, type(None))


def _result(t, e=Error):
    from _erg_type import UnionType

    return UnionType(t, e)


def _either(l, r):
    from _erg_type import UnionType

    return UnionType(l, r)


# `Option T == T or NoneType`, `Result T == T or Error`, `Either(L, R) == L or R`
Option = _TypeAlias("Option", _option)
Result = _TypeAlias("Result", _result)
Either = _TypeAlias("Either", _either)


class OptionMut(MutType):
    """`Option! T`: a cell holding a `T or NoneType` that can be refilled.

    `Option T` is only an alias for `T or NoneType`, so a mutable option cannot be
    the `MutType!` of anything; this is a class of its own.
    """

    value: object

    def __init__(self, value=None):
        self.value = value

    def __repr__(self):
        return "Option!({})".format(repr(self.value))

    def __eq__(self, other):
        if isinstance(other, MutType):
            return self.value == other.value
        else:
            return self.value == other

    def __ne__(self, other):
        return not self.__eq__(other)

    def get(self):
        return self.value

    def set(self, value):
        self.value = value

    def clear(self):
        self.value = None


def is_ok(obj) -> bool:
    return not isinstance(obj, Error)


# The error alternative of a `T or E` value, as recognized by the `?` operator.
# The compiler only lets `?` through when `E` is `NoneType`, `Error` and/or a
# subtype of `BaseException`, and never when the success type is one of those,
# so this check picks exactly the error alternative.
def is_err(obj) -> bool:
    return obj is None or isinstance(obj, (Error, BaseException))


def push_err_frame(err, name, line, file):
    """Record a `?` propagation site on `err`, then hand it back.

    Only `Error` carries a stack; `None` and exceptions are returned untouched.
    """
    if isinstance(err, Error):
        err.stack.append(ErrorFrame(name, line, file))
    return err


def _source_line(file, line):
    try:
        import linecache

        return linecache.getline(file, line).strip()
    except Exception:
        return ""


def format_traceback(err, stack=None) -> str:
    """Render an error and the `?` chain it travelled, most recent call first.

    `stack` overrides `err.stack`, for errors that cannot carry one themselves.
    """
    lines = []
    if stack is None:
        stack = getattr(err, "stack", [])
    if stack:
        lines.append("Traceback (most recent call first):")
        for frame in stack:
            lines.append("  {}".format(frame))
            src = _source_line(frame.file, frame.line)
            if src:
                lines.append("  {} | {}".format(frame.line, src))
    for context in getattr(err, "contexts", []):
        lines.append("hint: {}".format(context))
    if isinstance(err, Error):
        lines.append("{}: {}".format(err.kind, err.msg))
    elif isinstance(err, BaseException):
        lines.append("{}: {}".format(type(err).__name__, err))
    else:
        lines.append("Error: {!r}".format(err))
    return "\n".join(lines)


def panic_err(err, name, line, file):
    """`?` reached an error where no `return` is possible: report and abort."""
    import sys

    push_err_frame(err, name, line, file)
    # `None` and exceptions cannot carry a stack, so at least report this site
    stack = getattr(err, "stack", None) or [ErrorFrame(name, line, file)]
    print(format_traceback(err, stack), file=sys.stderr)
    sys.exit(1)


def result_unwrap(obj, msg=None):
    """`x.unwrap()`: the success value of `x`, or a panic carrying its trace."""
    import sys

    if not is_err(obj):
        return obj
    if msg is None:
        if obj is None:
            msg = "unwrapped a None value"
        elif isinstance(obj, Error):
            msg = "unwrapped an error value ({}: {})".format(obj.kind, obj.msg)
        else:
            msg = "unwrapped an error value ({}: {})".format(type(obj).__name__, obj)
    err = Error(msg, "UnwrappingError")
    # the `?` frames and hints the unwrapped error collected still apply
    err.stack = list(getattr(obj, "stack", []))
    err.contexts = list(getattr(obj, "contexts", []))
    print(format_traceback(err), file=sys.stderr)
    sys.exit(1)


def result_unwrap_or(obj, default):
    """`x.unwrap_or(default)`: the success value of `x`, or `default`."""
    return default if is_err(obj) else obj


def result_unwrap_or_exec(obj, f):
    """`x.unwrap_or_exec(f)`: the success value of `x`, or the result of `f()`.

    Also backs `unwrap_or_exec!`, which differs only in taking a procedure.
    """
    return f() if is_err(obj) else obj
