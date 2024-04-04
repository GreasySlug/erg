# 解析 Erg 語言

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/compiler/parsing.md%26commit_hash%3D51de3c9d5a9074241f55c043b9951b384836b258)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/compiler/parsing.md&commit_hash=51de3c9d5a9074241f55c043b9951b384836b258)

## 空格的處理

Erg 語法的一個特點是它對空格敏感，這是為了彌補因省略`()`而造成的表達力損失。在 Nim 中可以找到類似的語法，它也允許省略 `()`

```python
f +1 == f(+1)
f + 1 == `+`(f, 1)
f (1,) == f((1,))
f(1,) == f(1)
(f () -> ...) == f(() -> ...)
(f() -> ...) == (f() -> ...)
```

## 左值、右值

在 Erg 中，左側的值并不是 `=` 的左側那么簡單。
事實上，`=` 左側有一個右值(非常令人困惑)，而 `=` 右側有一個左值。
右值中甚至可以有左值

```python
# i 是左值，List(Int) 和 [1, 2, 3] 是右值
i: List(Int) = [1, 2, 3]
# `[1, 2, 3].iter().map i -> i + 1`是右值，但是`->`左邊的i是左值
a = [1, 2, 3].iter().map i -> i + 1
# {x = 1; y = 2} 是右值，但 `x`, `y` 是左值
r = {x = 1; y = 2}
```

左值和右值的精確定義是“如果右側的值可以計算，則為右值，否則為左值”。
例如，考慮代碼 ``i = 1; i``，其中第二個 `i` 是右值，因為它是可計算的，但第一個 `i` 是左值。
