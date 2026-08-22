# Result T, E

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Result.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Result.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

```python,checker_ignore
Result T     == T or Error
Result(T, E) == T or E
```

與`Option`一樣表示「可能失敗的值」，但失敗帶有上下文：`Error`具有`.msg`、`.kind`、透過`.context`附加的提示，以及它被返回時途經的子程序`.stack`

它不是獨立的類型而是別名。錯誤類型預設為[`Error`](./Error.md)；若要使用其他類型(例如Python例外)，請傳入第二個參數

```python,checker_ignore
half(x: Int): Result Int = if x % 2 == 0, do x // 2, do Error "not even"

parse(s: Str): Result(Int, ValueError) = ...
```

[`?`](../../special.md)運算符同時識別兩者，因此對`Result T`應用`?`會得到`T`，否則外圍子程序返回該錯誤

```python,checker_ignore
quarter(x: Int): Result Int =
    y = half(x)?
    half(y)?
```

`Result(T, E)`與[`Either(T, E)`](./Either.md)是同一個 Or 類型；`Result`是標明哪一側是錯誤的寫法
