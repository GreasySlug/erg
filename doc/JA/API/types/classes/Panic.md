# Panic

異常終了。[`Never`](./Never.md)と同じくインスタンスを持たないが、`Never`が正常終了や意図的な無限ループを表すのに対し、`Panic`はプログラムが処理を諦めたことを表す。`Never <: Panic`。

```python,checker_ignore
always_fails(): Panic = panic "boom"
always_exits(): Panic = exit 0
```

## `T or Panic`

`T`を返すが終了する可能性もあるサブルーチンには`T or Panic`と書く。`T`に潰れる`T or Never`と違い、これはシグネチャに残る。残すことこそが目的だからである。

```python
positive(x: Int): Int or Panic = if x > 0, do x, do panic "must be positive"
```

その型の値はそのまま`T`として使用できる。`Panic`はインスタンスを持たないため、もしそうなっていたならプログラムは既に停止しているからである。

```python,checker_ignore
positive(3) * 2         # OK、narrowing 不要
takes_int(positive(3))  # OK
```

ただし**裸の**`Panic`は`T`ではない。`Panic`を返すと宣言されたサブルーチンは正常に返らないので、使える値が存在しない。返る可能性があるなら`T or Panic`と書くこと。

```python,checker_ignore
f(): Int = always_fails() # TypeError
```

## 未実装の部分

`panic`・`todo`・`unreachable`は`Panic`ではなく`Never`を返す。これは`f(): Int = todo()`のようなスタブが型検査を通るようにするためである。

終了しうる演算はまだ`Panic`で型付けされていない。`Div`トレイトは`.'/' = (self: Self, R) -> Self.Output or Panic`と規定されているが、除算は現在それなしで型付けされている。
