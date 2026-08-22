# Either L, R = L or R

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Either.md%26commit_hash%3Dd15cbbf7b33df0f78a575cff9679d84c36ea3ab1)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Either.md&commit_hash=d15cbbf7b33df0f78a575cff9679d84c36ea3ab1)

「LかRかどちらか」を表す型。Or型の2つ限定形と考えて良い。

独立した型ではなく別名であり、`Either(Int, Str)`は`Int or Str`**そのもの**である。型が期待される場所ではどちらを書いても同じである。[`Option`](./Option.md)と[`Result`](./Result.md)は片側を固定した同じ仕組みである。

```python
pick(x: Int): Either(Int, Str) = if x > 0, do x, do "negative"
```

素のOr型であるため対称である。`Either`は値がどちら側から来たかのタグを持たないので、両者は narrowing で区別する。また`Either(Int, Int)`は単に`Int`である。

```python,checker_ignore
picked = pick(-1)
assert picked in Str
picked.upper()
```

## methods

未実装。以下は予定。

* orl
* orr
* andl
* andr
* mapl
* mapr

これらは値がどちら側かを実行時に知る必要があるが、`L`と`R`が重なる場合、タグを持たないOr型はそれに答えられない。
