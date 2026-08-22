# Option T = T or NoneType

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Option.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Option.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

表示「可能失敗」的類型。它不是獨立的類型而是別名：`Option Int`**就是**`Int or NoneType`，在期望類型的位置兩者可以互換

```python
x: Option Int = 1
y: Option Int = None

first(l: List(Int, 3)): Option Int = l.get(0)
```

[`?`](../../special.md)運算符將`NoneType`視為錯誤一側，因此對`Option T`應用`?`會得到`T`，否則外圍子程序返回`None`

```python
sum_two(l: List(Int, 3)): Option Int =
    a = l.get(0)?
    b = l.get(1)?
    a + b
```

## methods

尚未實現。以下為計劃

* unwrap(self, msg = "unwrapped a None value") -> T or Panic

期望內容為`T`類型並將其取出。如果是`None`，則輸出`msg`並panic

```python,checker_ignore
x = "...".parse(Int).into(Option Int)
x.unwrap() # UnwrappingError: unwrapped a None value
x.unwrap("failed to convert from string to number") # UnwrappingError: failed to convert from string to number
```

* unwrap_or(self, else: T) -> T

* unwrap_or_exec(self, f: () -> T) -> T

* unwrap_or_exec!(self, p!: () => T) -> T
