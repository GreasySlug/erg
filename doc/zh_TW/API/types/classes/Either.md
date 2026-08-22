# Either L, R = L or R

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Either.md%26commit_hash%3Dd15cbbf7b33df0f78a575cff9679d84c36ea3ab1)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Either.md&commit_hash=d15cbbf7b33df0f78a575cff9679d84c36ea3ab1)

表示「L 或 R」的類型。可以將其視為 Or 類型的二元限定形式

它不是獨立的類型而是別名：`Either(Int, Str)`**就是**`Int or Str`，在期望類型的位置兩者可以互換。[`Option`](./Option.md)和[`Result`](./Result.md)是固定了一側的同一套機制

```python
pick(x: Int): Either(Int, Str) = if x > 0, do x, do "negative"
```

由於它就是素的 Or 類型，因此是對稱的：`Either`不攜帶表明值來自哪一側的標籤，所以兩側透過 narrowing 區分，而`Either(Int, Int)`就是`Int`

```python,checker_ignore
picked = pick(-1)
assert picked in Str
picked.upper()
```

## methods

尚未實現。以下為計劃

* orl
* orr
* andl
* andr
* mapl
* mapr

它們需要在執行時知道值位於哪一側，而當`L`和`R`重疊時，無標籤的 Or 類型無法回答這一點
