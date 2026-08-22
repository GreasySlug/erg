def opt(x):
    return x if x > 0 else None


def defined_first(x):
    y = opt(x)
    if y is None:
        return y
    return y + 1


def defined_second(x):
    y = opt(x)
    if y is None:
        return y
    return y + 2


assert defined_second(1) == 3
assert defined_first(1) == 2


def add_one(x):
    y = opt(x)
    if y is None:
        return y
    return y + 1


assert add_one(1) == 2
assert add_one(-1) is None


def add(a, b):
    x = opt(a)
    if x is None:
        return x
    y = opt(b)
    if y is None:
        return y
    return x + y


assert add(1, 2) == 3
assert add(1, -2) is None


def in_if(x):
    if x != 0:
        y = opt(x)
        if y is None:
            return y
        return y * 10
    return 0


assert in_if(7) == 70
assert in_if(0) == 0
assert in_if(-1) is None
