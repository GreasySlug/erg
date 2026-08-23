from fractions import Fraction

from _erg_ratio import RatioMut


def mutate_operator(x):
    if hasattr(x, "mutate"):
        return x.mutate()
    # An Erg `Ratio` *is* a `Fraction` at run time, so there is no class of ours
    # to hang `mutate` on -- without this, `!0.5` silently stayed immutable.
    elif isinstance(x, Fraction):
        return RatioMut(x)
    else:
        return x
