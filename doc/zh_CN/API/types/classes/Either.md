# Either L, R = L or R

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Either.md%26commit_hash%3Dd15cbbf7b33df0f78a575cff9679d84c36ea3ab1)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Either.md&commit_hash=d15cbbf7b33df0f78a575cff9679d84c36ea3ab1)

表示"L 或 R"的类型。可以将其视为 Or 类型的二元限定形式

它不是独立的类型而是别名：`Either(Int, Str)`**就是**`Int or Str`，在期望类型的位置两者可以互换。[`Option`](./Option.md)和[`Result`](./Result.md)是固定了一侧的同一套机制

```python
pick(x: Int): Either(Int, Str) = if x > 0, do x, do "negative"
```

由于它就是素的 Or 类型，因此是对称的：`Either`不携带表明值来自哪一侧的标签，所以两侧通过 narrowing 区分，而`Either(Int, Int)`就是`Int`

```python,checker_ignore
picked = pick(-1)
assert picked in Str
picked.upper()
```

## methods

尚未实现。以下为计划

* orl
* orr
* andl
* andr
* mapl
* mapr

它们需要在运行时知道值位于哪一侧，而当`L`和`R`重叠时，无标签的 Or 类型无法回答这一点
