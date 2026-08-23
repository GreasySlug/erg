from fractions import Fraction

from _erg_type import MutType

# The compiler folds rationals into a 128-bit `ValueObj::Ratio`, and renders one
# with the same rule as `_decimal_str` below. Applying its bounds here too keeps
# a folded constant printing exactly like the expression it was folded from.
_I128_MAX = (1 << 127) - 1


def _unwrap(other):
    return other.value if isinstance(other, MutType) else other


def _decimal_str(num, den):
    """The exact decimal expansion of `num`/`den`, or `None` when there is none.

    A rational in lowest terms is a finite decimal exactly when its denominator
    is 2**a * 5**b, and the digits are then `num * 10**k / den` for
    `k = max(a, b)`. That is computed as `num * 2**(k - a) * 5**(k - b)`, so the
    10**k -- which can be far larger than either -- is never formed.
    """
    a = 0
    while den % 2 == 0:
        den //= 2
        a += 1
    b = 0
    while den % 5 == 0:
        den //= 5
        b += 1
    if den != 1:
        return None
    # no forced fractional digit: `Fraction(3)` prints `3`, and the compiler
    # folds an integral rational to a `Nat`/`Int`, which prints the same way
    k = max(a, b)
    if k == 0:
        return str(num)
    scale = 2 ** (k - a) * 5 ** (k - b)
    if scale > _I128_MAX:
        return None
    digits = num * scale
    if abs(digits) > _I128_MAX:
        return None
    sign = "-" if digits < 0 else ""
    digits = str(abs(digits)).rjust(k + 1, "0")
    return f"{sign}{digits[:-k]}.{digits[-k:]}"


def _keep_class(value):
    """`Fraction`'s operators build a `Fraction` even for a subclass."""
    return Ratio(value) if type(value) is Fraction else value


class Ratio(Fraction):
    """`Ratio`: an exact rational.

    A `Fraction` subclass only for how it prints. `0.1234` is what the user
    wrote; `617/5000` is not, and neither is the `Fraction(617, 5000)` that
    would show up inside a list. Every operator is overridden because
    `Fraction`'s return a base `Fraction` even when both operands are
    subclasses -- without them `0.1 + 0.2` would print `3/10`.
    """

    def __str__(self):
        decimal = _decimal_str(self.numerator, self.denominator)
        return Fraction.__str__(self) if decimal is None else decimal

    __repr__ = __str__

    def __add__(self, other):
        return _keep_class(Fraction.__add__(self, other))

    def __radd__(self, other):
        return _keep_class(Fraction.__radd__(self, other))

    def __sub__(self, other):
        return _keep_class(Fraction.__sub__(self, other))

    def __rsub__(self, other):
        return _keep_class(Fraction.__rsub__(self, other))

    def __mul__(self, other):
        return _keep_class(Fraction.__mul__(self, other))

    def __rmul__(self, other):
        return _keep_class(Fraction.__rmul__(self, other))

    def __truediv__(self, other):
        return _keep_class(Fraction.__truediv__(self, other))

    def __rtruediv__(self, other):
        return _keep_class(Fraction.__rtruediv__(self, other))

    def __mod__(self, other):
        return _keep_class(Fraction.__mod__(self, other))

    def __rmod__(self, other):
        return _keep_class(Fraction.__rmod__(self, other))

    def __pow__(self, other):
        return _keep_class(Fraction.__pow__(self, other))

    def __rpow__(self, other):
        return _keep_class(Fraction.__rpow__(self, other))

    def __neg__(self):
        return _keep_class(Fraction.__neg__(self))

    def __pos__(self):
        return self

    def __abs__(self):
        return _keep_class(Fraction.__abs__(self))


class RatioMut(MutType):  # inherits Ratio
    """`Ratio!`: a mutable cell holding a `Ratio`."""

    value: Ratio

    def __init__(self, i):
        self.value = Ratio(i)

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
        self.value = Ratio(f(self.value))

    def inc(self, value=1):
        self.value = Ratio(self.value + value)

    def dec(self, value=1):
        self.value = Ratio(self.value - value)

    def copy(self):
        return RatioMut(self.value)
