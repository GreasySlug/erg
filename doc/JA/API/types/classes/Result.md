# Result T, E

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Result.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Result.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

```python,checker_ignore
Result T     == T or Error
Result(T, E) == T or E
```

`Option`と同じく「失敗するかもしれない値」を表すが、失敗時のコンテクストを持てる。`Error`は`.msg`・`.kind`・`.context`で付けたヒント・`return`されてきたサブルーチンの`.stack`を持つ。

独立した型ではなく別名である。エラー型の既定は[`Error`](./Error.md)で、Pythonの例外など別の型を使いたい場合は第2引数に渡す。

```python,checker_ignore
half(x: Int): Result Int = if x % 2 == 0, do x // 2, do Error "not even"

parse(s: Str): Result(Int, ValueError) = ...
```

[`?`](../../special.md)演算子はどちらも認識するため、`Result T`に`?`を適用すると`T`が得られ、そうでなければ囲むサブルーチンからエラーが返る。

```python,checker_ignore
quarter(x: Int): Result Int =
    y = half(x)?
    half(y)?
```

かつて`Result`の定義に使われていた`Either T, E`は未実装である。
