assert True or False
assert not (False or False)
assert True and True
assert not (True and False)
a = 1
b = a == 1 or a == 2
assert b
c = a == 1 and a != 0
assert c
