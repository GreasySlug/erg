from _erg_range import Range
from _erg_result import is_ok
from _erg_type import StructuralType
from _erg_type import UnionType
from _erg_type import _isinstance
from _erg_type import _is_type_record
from _erg_type import _record_fields
from _erg_type import is_type

def _structurally_contains(type_record, elem) -> bool:
    fields = _record_fields(type_record)
    if fields is None:
        return False
    for name, ty in fields.items():
        if hasattr(elem, name):
            val = getattr(elem, name)
        elif hasattr(elem, "::" + name):
            # Erg private instance attributes are stored as `::name`
            val = getattr(elem, "::" + name)
        else:
            return False
        if not contains_operator(ty, val):
            return False
    return True


# (elem in y) == contains_operator(y, elem)
def contains_operator(y, elem) -> bool:
    if hasattr(elem, "type_check"):
        return elem.type_check(y)
    elif isinstance(y, StructuralType):
        base = y.base
        if _is_type_record(base):
            return _structurally_contains(base, elem)
        if is_type(base) and _isinstance(elem, base):
            return True
        return False
    elif isinstance(y, UnionType):
        return any([contains_operator(t, elem) for t in y.__args__])
    # 1 in Int
    elif is_type(y):
        if _isinstance(elem, y):
            return True
        elif hasattr(y, "generic_try_new"):
            return is_ok(y.generic_try_new(elem, y))
        elif hasattr(y, "try_new") and is_ok(y.try_new(elem)):
            return True
        elif hasattr(y, "__origin__") and hasattr(y.__origin__, "type_check"):
            return y.__origin__.type_check(elem, y)
        # TODO: trait check
        return False
    # record width: extra fields on the value are allowed
    elif _is_type_record(y) and type(elem).__name__ == "Record":
        return _structurally_contains(y, elem)
    # [1] in [Int]
    elif (
        _isinstance(y, list)
        and _isinstance(elem, list)
        and (len(y) == 0 or is_type(y[0]) or _isinstance(y[0], Range))
    ):
        type_check = all(map(lambda x: contains_operator(x[0], x[1]), zip(y, elem)))
        len_check = len(elem) <= len(y)
        return type_check and len_check
    # (1, 2) in (Int, Int)
    elif (
        _isinstance(y, tuple)
        and _isinstance(elem, tuple)
        and (len(y) == 0 or is_type(y[0]) or _isinstance(y[0], Range))
    ):
        if not hasattr(elem, "__iter__"):
            return False
        type_check = all(map(lambda x: contains_operator(x[0], x[1]), zip(y, elem)))
        len_check = len(elem) <= len(y)
        return type_check and len_check
    # {1: 2} in {Int: Int}
    elif (
        _isinstance(y, dict)
        and _isinstance(elem, dict)
        and (len(y) == 0 or is_type(next(iter(y.keys()))))
    ):
        if len(y) == 1:
            key = next(iter(y.keys()))
            key_check = all([contains_operator(key, el) for el in elem.keys()])
            value = next(iter(y.values()))
            value_check = all([contains_operator(value, el) for el in elem.values()])
            return key_check and value_check
        type_check = True  # TODO:
        len_check = True  # It can be True even if either elem or y has the larger number of elems
        return type_check and len_check
    elif _isinstance(elem, list):
        from _erg_list import List

        return contains_operator(y, List(elem))
    elif callable(elem):
        # TODO:
        return callable(y)
    else:
        return elem in y
