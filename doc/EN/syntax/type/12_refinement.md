# Refinement Type

Refinement type is a type constrained by a predicate expression. Enumeration types and interval types are syntax sugar of refinement types.

The standard form of a refinement type is `{Elem: Type | (Pred)*}`. This means that the type is a type whose elements are `Elem` satisfying `Pred`.
The type that can be used for the refinement type is [Value type](./08_value.md) only.

```python
Nat = 0.. _
Odd = {N: Int | N % 2 == 1}
Char = StrWithLen 1
# StrWithLen 1 == {_: StrWithLen N | N == 1}
[Int; 3] == {_: List Int, N | N == 3}
List3OrMore == {A: List _, N | N >= 3}
```

When there are multiple preds, they can be separated by `;` or `and` or `or`. `;` and `and` mean the same thing.

The elements of `Odd` are `1, 3, 5, 7, 9, ...`.
It is called a refinement type because it is a type whose elements are part of an existing type as if it were a refinement.

The `Pred` is called a predicate expression. It is checked by running it: whenever a value
is checked against the type, the refinement variable is bound to that value and the predicate
is evaluated at compile time. Arithmetic and calls of compile-time functions may therefore
appear in it.

```python
{X: Int | X**2 - 5 * X + 6 == 0} # 2 and 3 are of this type, 5 is not
IsOdd(N: Int): Bool = N % 2 == 1
_: {N: Int | IsOdd(N)} = 7
```

What the compiler does not do is *solve* the predicate: it cannot tell you that the type
above is `{2, 3}`, only whether a value you hand it belongs.

## Subtyping rules for refinement types

All refinement types are subtypes of the type specified in the `Type` part.

```erg
{I: Int | I <= 0} <: Int
```

Otherwise, the current Erg has a subtyping type rule for integer comparisons.

```erg
{I: Int | I <= 5} <: {I: Int | I <= 0}
```

## Smart Cast

It's nice that you defined `Odd`, but as it is, it doesn't look like it can be used much outside of literals. To promote an odd number in a normal `Int` object to `Odd`, i.e., to downcast an `Int` to `Odd`, you need to pass the constructor of `Odd`.
For refinement types, the normal constructor `.new` may panic, and there is an auxiliary constructor called `.try_new` that returns a `Result` type.

```python
i = Odd.new (0..10).sample!() # i: Odd (or Panic)
```

It can also be used as a type specification in `match`.

```python
# i: 0..10
i = (0..10).sample!
match i:
    o: Odd ->
        log "i: Odd"
    n: Nat -> # 0..10 < Nat
        log "i: Nat"
```

However, Erg cannot currently make sub-decisions such as `Even` because it was not `Odd`, etc.

## Enumerated, Interval and Refinement Types

The enumerative/interval types introduced before are syntax sugar of the refinement type.
`{a, b, ...}` is `{I: Typeof(a) | I == a or I == b or ... }`, and `a..b` is desugarized to `{I: Typeof(a) | I >= a and I <= b}`.

```python
{1, 2} == {I: Int | I == 1 or I == 2}
1..10 == {I: Int | I >= 1 and I <= 10}
1... <10 == {I: Int | I >= 1 and I < 10}
```

## Refinement pattern

Just as `_: {X}` can be rewritten as `X` (constant pattern), `_: {X: T | Pred}` can be rewritten as `X: T | Pred`.

```python
# method `.m` is defined for arrays of length 3 or greater
List(T, N | N >= 3)
    .m(&self) = ...
```

<p align='center'>
    <a href='./11_enum.md'>Previous</a> | <a href='./13_algebraic.md'>Next</a>
</p>
