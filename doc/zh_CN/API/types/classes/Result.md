# Result T, E

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Result.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Result.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

```python,checker_ignore
Result T     == T or Error
Result(T, E) == T or E
```

与`Option`一样表示"可能失败的值"，但失败带有上下文：`Error`具有`.msg`、`.kind`、通过`.context`附加的提示，以及它被返回时途经的子例程`.stack`

它不是独立的类型而是别名。错误类型默认为[`Error`](./Error.md)；若要使用其他类型(例如Python异常)，请传入第二个参数

```python,checker_ignore
half(x: Int): Result Int = if x % 2 == 0, do x // 2, do Error "not even"

parse(s: Str): Result(Int, ValueError) = ...
```

[`?`](../../special.md)运算符同时识别两者，因此对`Result T`应用`?`会得到`T`，否则外围子例程返回该错误

```python,checker_ignore
quarter(x: Int): Result Int =
    y = half(x)?
    half(y)?
```

`Result(T, E)`与[`Either(T, E)`](./Either.md)是同一个 Or 类型；`Result`是标明哪一侧是错误的写法


## methods

[`Option`的方法](./Option.md#methods)也就是`Result`的方法。它们不是为某一个别名定义的，而是为带有错误一侧的`T or E`定义的

```python
half(x: Int): Result Int = if x % 2 == 0, do x // 2, do Error "not even"

assert half(8).unwrap() == 4
assert half(3).unwrap_or(0) == 0
assert half(3).unwrap_or_exec(() -> 0) == 0
```

`.unwrap`在panic之前会输出它找到的错误：种别、消息、用`.context`附加的提示，以及`?`经过的帧
