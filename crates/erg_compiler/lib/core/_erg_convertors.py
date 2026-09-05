from _erg_bool import Bool
from _erg_float import Float
from _erg_int import Int
from _erg_nat import Nat
from _erg_str import Str


def int__(i, base=None):
    # `int(x, base)` only accepts strings, so `base` must not be forwarded unless given
    if base is None:
        return Int(i)
    return Int(i, base)


def nat__(i):
    return Nat(i)


def bool__(b=False):
    # `Bool` is an `int` subclass, so the truth value has to be taken first:
    # `Bool(5)` would be a `Bool` that is 5
    return Bool(bool(b))


def float__(f):
    return Float(f)


def str__(s):
    return Str(s)
