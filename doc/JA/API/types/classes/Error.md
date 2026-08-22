# Error

Erg独自の回復可能なエラー。`T or Error`を返すことで、サブルーチンは呼び出し側に処理を委ねる失敗を報告する。プログラムを終了させる`Panic`とは対照的である。

```python
half(x: Int): Int or Error =
    if x % 2 == 0:
        do x // 2
        do Error("not even", kind := "ValueError")
```

## 属性

* `.msg: Str` — 何が起きたか
* `.kind: Str` — エラーの種類。指定しなければ`"Error"`(`Error(msg, kind := ...)`)
* `.stack: List(ErrorFrame, _)` — [`?`](../../special.md)によってこのエラーが`return`されたサブルーチンの列。内側の呼び出しが先頭

`.msg`と`.kind`はコンテキストではない。作られた時点でエラーに属するものであり、上書きされることはない。

## メソッド

### `.context(self, msg: Str) -> Error`

エラーを消費し、ヒントを1つ追加した新しいエラーを返す。チェイン可能なので、1つのエラーが複数のコンテキストを持てる。

```python
not_ready(): Error =
    e = Error "not implemented yet"
    e.context("to be implemented in ver 1.2").context("and more hints ...")
```

## スタックトレース

`Error`が`?`によって`return`されるたびに、その呼び出し位置が`.stack`に積まれる。`return`できない位置 — トップレベル — で`?`に到達すると、集まった軌跡が表示されプログラムは終了する。

```python,checker_ignore
try_some(x: Int): Int or Error = ...

f x =
    y = try_some(x)?
    ...

g x =
    y = f(x)?
    ...

i = g(1)?
# Traceback (most recent call first):
#   f, line 5, file "foo.er"
#   5 | y = try_some(x)?
#   g, line 8, file "foo.er"
#   8 | y = f(x)?
#   <module>, line 11, file "foo.er"
#   11 | i = g(1)?
# Error: ...
```

`ErrorFrame`はこの軌跡の1要素で、`.name: Str`・`.line: Nat`・`.file: Str`を持つ。
