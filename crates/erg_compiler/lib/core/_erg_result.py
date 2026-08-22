# from typing import TypeVar, Union, _SpecialForm, _type_check


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


# T = TypeVar("T")
# @_SpecialForm
# def Result(self, parameters):
#    """Result type.
#
#    Result[T] is equivalent to Union[T, Error].
#    """
#    arg = _type_check(parameters, f"{self} requires a single type.")
#    return [arg, Error]


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
