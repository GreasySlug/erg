# キャスト

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/type/17_type_casting.md%26commit_hash%3D7d7849b4932909197c185c1737dcc1f63cce701c)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/type/17_type_casting.md&commit_hash=7d7849b4932909197c185c1737dcc1f63cce701c)

## アップキャスト

Pythonはダックタイピングを採用する言語のため、キャストという概念はありません。アップキャストはする必要がなく、ダウンキャストも基本的にはありません。
しかしErgは静的に型付けされるため、キャストを行わなければいけない場合があります。
簡単な例では、`1 + 2.0`が挙げられます。Ergの言語仕様上は`+`(Int, Ratio)、すなわちInt(<: Add(Ratio, Ratio))の演算は定義されていません。というのも、`Int <: Ratio`であるため、1はRatioのインスタンスである1.0にアップキャストされるからです。

~~Erg拡張バイトコードはBINARY_ADDに型情報を加えますが、この際の型情報はRatio-Ratioとなります。この場合はBINARY_ADD命令がIntのキャストを行うため、キャストを指定する特別な命令は挿入されません。なので、例えば子クラスでメソッドをオーバーライドしても、親を型に指定すれば型強制(type coercion)が行われ、親のメソッドで実行されます(コンパイル時に親のメソッドを参照するように名前修飾が行われます)。コンパイラが行うのは型強制の妥当性検証と名前修飾のみです。ランタイムがオブジェクトをキャストすることはありません(現在のところ。実行最適化のためにキャスト命令が実装される可能性はあります)。~~

```python
@Inheritable
Parent = Class()
Parent.
    greet!() = print! "Hello from Parent"

Child = Inherit Parent
Child.
    # オーバーライドする際にはOverrideデコレータが必要
    @Override
    greet!() = print! "Hello from Child"

greet! p: Parent = p.greet!()

parent = Parent.new()
child = Child.new()

greet! parent # "Hello from Parent"
greet! child # "Hello from Parent"
```

この挙動はPythonとの非互換性を生むことはありません。そもそもPythonでは変数に型が指定されないので、いわば全ての変数が型変数で型付けされている状態となります。型変数は適合する最小の型を選ぶので、Ergで型を指定しなければPythonと同じ挙動が達成されます。

```python
@Inheritable
Parent = Class()
Parent.
    greet!() = print! "Hello from Parent"

Child = Inherit Parent
Child.
    greet!() = print! "Hello from Child"

greet! some = some.greet!()

parent = Parent.new()
child = Child.new()

greet! parent # "Hello from Parent"
greet! child # "Hello from Child"
```

継承関係にある型同士では`.from`, `.into`が自動実装されるので、それを使うこともできます。

```python
assert 1 == 1.0
assert Ratio.from(1) == 1.0
assert 1.into(Ratio) == 1.0
```

## 強制アップキャスト

多くの場合、オブジェクトのアップキャストは呼び出す関数や演算子に応じて自動で行われます。
しかし、アップキャストを強制したい場合もあります。その場合は`as`を使います。

```python,compile_fail
n = 1
n.times! do:
    print! "Hello"

i = n as Int
i.times! do: # ERR
    print! "Hello"

s = n as Str # ERR
```

`as`では関係のない型や、部分型にキャストすることはできません。

## 強制キャスト

`typing.cast`を使って、型を強制的にキャストすることができます。
これは対象をどんな型にでも変換出来ます。
Pythonの`typing.cast`はランタイムに何も行わない関数ですが、Ergでは組み込み型の場合コンストラクタによる変換が入ります[<sup id="f1">1</sup>](#1)。
組み込み型でない場合、変換は入らず、安全性の保証は全くありません。

```python
typing = pyimport "typing"

C = Class { .x = Int }

s = typing.cast Str, 1

assert s == "1"
print! s + "a" # 1a

c = typing.cast C, 1
print! c.x # AttributeError: 'int' object has no attribute 'x'
```

## ダウンキャスト

ダウンキャストは一般に安全ではなく、変換方法も自明ではないため、代わりに`TryFrom.try_from`の実装で実現します。

```python
IntTryFromFloat = Patch Int
IntTryFromFloat.
    try_from r: Float =
        if r.ceil() == r:
            then: r.ceil()
            else: Error "conversion failed"
```

---

<span id="1" style="font-size:x-small"><sup>1</sup> この変換は現状の実装による副産物であり、将来的には除去される。[↩](#f1) </span>

<p align='center'>
    <a href='./16_subtyping.md'>Previous</a> | <a href='./18_mut.md'>Next</a>
</p>
