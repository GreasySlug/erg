from fractions import Fraction

from _erg_type import MutType


def _unwrap(other):
    return other.value if isinstance(other, MutType) else other


class RatioMut(MutType):  # inherits Ratio
    """`Ratio!`: a mutable cell holding a `fractions.Fraction`.

    `Ratio` itself has no wrapper class -- an Erg `Ratio` *is* a `Fraction` at
    run time -- so, unlike `FloatMut`, this is the only class in this module.
    """

    value: Fraction

    def __init__(self, i):
        self.value = Fraction(i)

    def __repr__(self):
        return self.value.__repr__()

    # `print!` goes through `str`, and `str(Fraction(3, 4))` is "3/4" where its
    # `repr` is "Fraction(3, 4)"; an immutable `Ratio` prints the former.
    def __str__(self):
        return self.value.__str__()

    def __hash__(self):
        return self.value.__hash__()

    def __deref__(self):
        return self.value

    def __float__(self):
        return self.value.__float__()

    def __int__(self):
        return self.value.__int__()

    def __eq__(self, other):
        return self.value == _unwrap(other)

    def __ne__(self, other):
        return self.value != _unwrap(other)

    def __le__(self, other):
        return self.value <= _unwrap(other)

    def __ge__(self, other):
        return self.value >= _unwrap(other)

    def __lt__(self, other):
        return self.value < _unwrap(other)

    def __gt__(self, other):
        return self.value > _unwrap(other)

    def __add__(self, other):
        return RatioMut(self.value + _unwrap(other))

    def __sub__(self, other):
        return RatioMut(self.value - _unwrap(other))

    def __mul__(self, other):
        return RatioMut(self.value * _unwrap(other))

    def __floordiv__(self, other):
        return RatioMut(self.value // _unwrap(other))

    def __truediv__(self, other):
        return RatioMut(self.value / _unwrap(other))

    def __pow__(self, other):
        return RatioMut(self.value ** _unwrap(other))

    # Reflected forms, so `1.0 + m` works and not just `m + 1.0`: Python looks
    # these up on the type, so `MutType.__getattr__` cannot stand in for them.
    def __radd__(self, other):
        return RatioMut(_unwrap(other) + self.value)

    def __rsub__(self, other):
        return RatioMut(_unwrap(other) - self.value)

    def __rmul__(self, other):
        return RatioMut(_unwrap(other) * self.value)

    def __rfloordiv__(self, other):
        return RatioMut(_unwrap(other) // self.value)

    def __rtruediv__(self, other):
        return RatioMut(_unwrap(other) / self.value)

    def __rpow__(self, other):
        return RatioMut(_unwrap(other) ** self.value)

    def __pos__(self):
        return self

    def __neg__(self):
        return RatioMut(-self.value)

    def __abs__(self):
        return RatioMut(abs(self.value))

    def update(self, f):
        self.value = Fraction(f(self.value))

    def inc(self, value=1):
        self.value = Fraction(self.value + value)

    def dec(self, value=1):
        self.value = Fraction(self.value - value)

    def copy(self):
        return RatioMut(self.value)
