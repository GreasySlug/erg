# Option T = T or NoneType

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Option.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Option.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

「失敗するかもしれない」を表す型。独立した型ではなく別名であり、`Option Int`は`Int or NoneType`**そのもの**である。型が期待される場所ではどちらを書いても同じである。

```python
x: Option Int = 1
y: Option Int = None

first(l: List(Int, 3)): Option Int = l.get(0)
```

[`?`](../../special.md)演算子は`NoneType`をエラー側として扱うため、`Option T`に`?`を適用すると`T`が得られ、そうでなければ囲むサブルーチンから`None`が返る。

```python
sum_two(l: List(Int, 3)): Option Int =
    a = l.get(0)?
    b = l.get(1)?
    a + b
```

## methods

以下のメソッドは失敗をその場で解決する。`?`が呼び出し元に委ねるのとは対照的である。エラー側を持つ`T or E`全般に対して定義されているため、[`Result`](./Result.md)にも同じ4つがある。

* unwrap(self, msg := Str) -> T or [Panic](./Panic.md)

中身が`T`型であると期待して取り出す。`None`であった場合`msg`を出力してパニックする。

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap() == 1
```

```python,checker_ignore
x: Option Int = None
x.unwrap() # UnwrappingError: unwrapped a None value
x.unwrap("failed to convert from string to number") # UnwrappingError: failed to convert from string to number
```

値が拾ってきたヒントと`?`のフレームも併せて出力されるため、取り出してもトレースは失われない。

* unwrap_or(self, else: T) -> T

取り出す。`None`であれば`else`を返す。`else`はどちらの場合も評価される。

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap_or(0) == 1
```

* unwrap_or_exec(self, f: () -> T) -> T

同じだが、`f`は取り出すものがないときにのみ呼ばれる。

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap_or_exec(() -> 0) == 1
```

* unwrap_or_exec!(self, p!: () => T) -> T

副作用のあるフォールバックのための`unwrap_or_exec`。プロシージャなのでプロシージャからしか呼べない。

```python
log! = !0
recompute!() =
    log!.inc!()
    0

main!() =
    first(l: List(Int, 3)): Option Int = l.get(0)
    assert first([1, 2, 3]).unwrap_or_exec!(recompute!) == 1
    assert log! == 0

main!()
```
