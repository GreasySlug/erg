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


def float__(f):
    return Float(f)


def str__(s):
    return Str(s)
