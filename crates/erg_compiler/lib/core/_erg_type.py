try:
    from typing import Union
except ImportError:
    import warnings

    warnings.warn("`typing.Union` is not available. Please use Python 3.8+.")

    class Union:
        pass


class UnionType:
    __origin__ = Union
    __args__: list  # list[type]

    def __init__(self, *args):
        self.__args__ = args

    def __str__(self):
        s = "UnionType[" + ", ".join(str(arg) for arg in self.__args__) + "]"
        return s

    def __repr__(self):
        return self.__str__()


class FakeGenericAlias:
    __name__: str
    __origin__: type
    __args__: list  # list[type]

    def __init__(self, origin, *args):
        self.__name__ = origin.__name__
        self.__origin__ = origin
        self.__args__ = args


try:
    from types import GenericAlias
except ImportError:
    GenericAlias = FakeGenericAlias


class StructuralType:
    """Runtime wrapper for `Structural T`. Compile-time structural types are
    otherwise erased; keeping the wrapper lets `in` / `<` distinguish
    `Structural {i = Int}` from the record class `{i = Int}`.
    """

    def __init__(self, base):
        self.base = base

    def __repr__(self):
        return f"Structural({self.base!r})"

    def __eq__(self, other):
        if isinstance(other, StructuralType):
            return self.base == other.base
        return False

    def __ne__(self, other):
        return not self.__eq__(other)

    def __hash__(self):
        return hash(("Structural", self.base))


def is_type(x) -> bool:
    return isinstance(
        x, (type, FakeGenericAlias, GenericAlias, UnionType, StructuralType)
    )


def _record_fields(obj):
    if hasattr(obj, "_asdict"):
        try:
            return obj._asdict()
        except TypeError:
            return None
    if hasattr(obj, "_fields"):
        try:
            return {n: getattr(obj, n) for n in obj._fields}
        except AttributeError:
            return None
    return None


def _is_type_record(obj):
    fields = _record_fields(obj)
    if fields is None:
        return False
    if not fields:
        return type(obj).__name__ == "Record"
    return all(is_type(v) for v in fields.values())


def _unwrap_structural(obj):
    return obj.base if isinstance(obj, StructuralType) else obj


def is_subtype(lhs, rhs):
    """`lhs <: rhs` (non-proper; equal types are subtypes)."""
    lhs = _unwrap_structural(lhs)
    rhs = _unwrap_structural(rhs)
    if lhs is rhs or lhs == rhs:
        return True
    try:
        if isinstance(lhs, type) and isinstance(rhs, type) and issubclass(lhs, rhs):
            return True
    except TypeError:
        pass
    if isinstance(lhs, (set, frozenset)) and isinstance(rhs, (set, frozenset)):
        return lhs.issubset(rhs)
    lf = _record_fields(lhs)
    rf = _record_fields(rhs)
    if (
        lf is not None
        and rf is not None
        and _is_type_record(lhs)
        and _is_type_record(rhs)
    ):
        for name, rty in rf.items():
            if name not in lf:
                return False
            if not is_subtype(lf[name], rty):
                return False
        return True
    if isinstance(rhs, UnionType):
        return any(is_subtype(lhs, t) for t in rhs.__args__)
    if isinstance(lhs, UnionType):
        return all(is_subtype(t, rhs) for t in lhs.__args__)
    return False


def is_lt(lhs, rhs):
    return is_subtype(lhs, rhs) and not is_subtype(rhs, lhs)


def is_le(lhs, rhs):
    return is_subtype(lhs, rhs)


def is_gt(lhs, rhs):
    return is_lt(rhs, lhs)


def is_ge(lhs, rhs):
    return is_le(rhs, lhs)


# The behavior of `builtins.isinstance` depends on the Python version.
def _isinstance(obj, classinfo) -> bool:
    if isinstance(classinfo, (FakeGenericAlias, GenericAlias, UnionType)):
        if classinfo.__origin__ == Union:
            return any(_isinstance(obj, t) for t in classinfo.__args__)
        else:
            return isinstance(obj, classinfo.__origin__)
    else:
        try:
            return isinstance(obj, classinfo)
        except TypeError:
            # `classinfo` is not a class or a tuple of them
            return False


class MutType:
    value: object

    # This method is a fallback to implement pseudo-inheritance.
    def __getattr__(self, name):
        return object.__getattribute__(self.value, name)


def _unwrap_mut(other):
    return other.value if isinstance(other, MutType) else other


class Cell(MutType):
    """`Cell! T`: a box holding a `T` that can be replaced."""

    def __init__(self, value):
        self.value = value

    def __repr__(self):
        return f"Cell!({self.value!r})"

    def __str__(self):
        return str(self.value)

    def __hash__(self):
        return hash(self.value)

    def __eq__(self, other):
        return self.value == _unwrap_mut(other)

    def __ne__(self, other):
        return self.value != _unwrap_mut(other)

    def __lt__(self, other):
        return self.value < _unwrap_mut(other)

    def __le__(self, other):
        return self.value <= _unwrap_mut(other)

    def __gt__(self, other):
        return self.value > _unwrap_mut(other)

    def __ge__(self, other):
        return self.value >= _unwrap_mut(other)

    def __add__(self, other):
        return self.value + _unwrap_mut(other)

    def __radd__(self, other):
        return other + self.value

    def __sub__(self, other):
        return self.value - _unwrap_mut(other)

    def __rsub__(self, other):
        return other - self.value

    def __mul__(self, other):
        return self.value * _unwrap_mut(other)

    def __rmul__(self, other):
        return other * self.value

    def __bool__(self):
        return bool(self.value)

    def get(self):
        return self.value

    def set(self, value):
        self.value = value

    def update(self, f):
        self.value = f(self.value)

    def copy(self):
        return Cell(self.value)
