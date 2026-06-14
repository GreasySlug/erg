f = lambda x: x + 1
assert f(5) == 6

g = lambda x, y: x * y
assert g(3, 4) == 12

h = lambda x: "pos" if x > 0 else "non-pos"
assert h(1) == "pos"
assert h(0) == "non-pos"
