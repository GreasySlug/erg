# Option! T

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Option!.md%26commit_hash%3Dd15cbbf7b33df0f78a575cff9679d84c36ea3ab1)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Option!.md&commit_hash=d15cbbf7b33df0f78a575cff9679d84c36ea3ab1)

中身を入れ替えられる[`Option T`](./Option.md)のセル。

`Option`は`T or NoneType`の別名でしかないため、通常の方法では可変版を作れない。Or型は名前的型ではないので`Mutizable`を実装する先がなく、`!`でも作れないのである。そのため`Option!`は独立したクラスであり、`Option`を可変化するのではなく呼び出して生成する。

```python
name = Option!("Jane Doe")
print! name.get() # Jane Doe

name.set! "John Doe"
print! name.get() # John Doe

name.clear!()
print! name.get() # None
```

## methods

* `__call__`(value := T) -> Option! T

`value`を入れたセルを作る。値を渡さなければ空のセルになる。

* get(self: Ref(Option! T)) -> T or NoneType

現在セルに入っている値。ただの[`Option T`](./Option.md)なので、[`?`](../../special.md)・`.unwrap`・narrowingがそのまま使える。

```python
c: Option! Int = Option!(1)
assert c.get().unwrap() == 1

incr(opt: Option! Int): Int or NoneType =
    x = opt.get()?
    x + 1

print! incr(c) # 2
```

* set!(self: RefMut(Option! T), value: T or NoneType) => NoneType

セルに`value`を入れる。`None`を渡せば空になる。

* clear!(self: RefMut(Option! T)) => NoneType

セルを空にする。`o.clear!()`は`o.set!(None)`と同じ。

`.update!`は無い。`(T or NoneType) -> (T or NoneType)`を受け取ることになるが、型変数を含むOr型を引数に持つラムダはまだ推論できないためである。読んで、適用して、書き戻すこと。

```python
counter: Option! Int = Option!(0)
counter.set!(counter.get().unwrap_or(0) + 1)
print! counter.get() # 1
```

## 要素型について

`T`はセルを作るときの値から決まり、型注釈で広がることはない。`List!`と同様`Option!`は`T`について共変なので、`Option!({1})`は既に`Option! Int`を満たしてしまう。同じ型の別の値を書き込むのは問題ない。

```python
o = Option!("a")
o.set! "b"
print! o.get() # b
```

値なしで作った`Option!()`には`T`の決め手がなく、型注釈でも与えられない。`T`は最初の`.set!`で確定する。それまでは`T`に依存しない操作(`print!`など)しか使えない。

```python
empty = Option!()
print! empty.get() # None
empty.set! "now a Str"
print! empty.get() # now a Str
```
