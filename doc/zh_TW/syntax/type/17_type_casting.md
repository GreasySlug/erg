# 類型轉換

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/syntax/type/17_type_casting.md%26commit_hash%3D7d7849b4932909197c185c1737dcc1f63cce701c)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/syntax/type/17_type_casting.md&commit_hash=7d7849b4932909197c185c1737dcc1f63cce701c)

## 向上轉換

因為 Python 是一種使用鴨子類型的語言，所以沒有強制轉換的概念。沒有必要向上轉換，本質上也沒有向下轉換
但是，Erg 是靜態類型的，因此有時必須進行強制轉換
一個簡單的例子是 `1 + 2.0`: `+`(Int, Ratio) 或 Int(<: Add(Ratio, Ratio)) 操作在 Erg 語言規范中沒有定義。這是因為 `Int <: Ratio`，所以 1 向上轉換為 1.0，即 Ratio 的一個實例

~~ Erg擴展字節碼在BINARY_ADD中增加了類型信息，此時類型信息為Ratio-Ratio。在這種情況下，BINARY_ADD 指令執行 Int 的轉換，因此沒有插入指定轉換的特殊指令。因此，例如，即使您在子類中重寫了某個方法，如果您將父類指定為類型，則會執行類型強制，并在父類的方法中執行該方法(在編譯時執行名稱修改以引用父母的方法)。編譯器只執行類型強制驗證和名稱修改。運行時不強制轉換對象(當前。可以實現強制轉換指令以優化執行)。~~

```python
@Inheritable
Parent = Class()
Parent.
    greet!() = print! "Hello from Parent"

Child = Inherit Parent
Child.
    # Override 需要 Override 裝飾器
    @Override
    greet!() = print! "Hello from Child"

greet! p: Parent = p.greet!()

parent = Parent.new()
child = Child.new()

parent # 來自Parent的問候！
child #  來自child的問候！
```

此行為不會造成與 Python 的不兼容。首先，Python 沒有指定變量的類型，所以可以這么說，所有的變量都是類型變量。由于類型變量會選擇它們可以適應的最小類型，因此如果您沒有在 Erg 中指定類型，則可以實現與 Python 中相同的行為

```python
@Inheritable
Parent = Class()
Parent.
    greet!() = print! "Hello from Parent"

Child = Inherit Parent
Child.
    greet!() = print! "Hello from Child" Child.

greet! some = some.greet!()

parent = Parent.new()
child = Child.new()

parent # 來自Parent的問候！
child  # 來自child的問候！
```

您還可以使用 `.from` 和 `.into`，它們會為相互繼承的類型自動實現

```python
assert 1 == 1.0
assert Ratio.from(1) == 1.0
assert 1.into<Ratio>() == 1.0
```

## Forced upcasting

In many cases, upcasting of objects is automatic, depending on the function or operator that is called.
However, there are cases when you want to force upcasting. In that case, you can use `as`.

```python,compile_fail
n = 1
n.times! do: print!
    print! "Hello"
i = n as Int
i.times! do: # ERR
    "Hello"
s = n as Str # ERR
```

You cannot cast to unrelated types or subtypes with ``as``.

## Forced casting

You can use `typing.cast` to force casting. This can convert the target to any type.
In Python, `typing.cast` does nothing at runtime, but in Erg the conversion will be performed by the constructor if object's type is built-in[<sup id="f1">1</sup>](#1).
For non-built-in types, the safety is not guaranteed at all.

```python
typing = pyimport "typing"

C = Class { .x = Int }

s = typing.cast Str, 1

assert s == "1"
print! s + "a" # 1a

c = typing.cast C, 1
print! c.x # AttributeError: 'int' object has no attribute 'x'
```

## 向下轉換

由于向下轉換通常是不安全的并且轉換方法很重要，我們改為實現`TryFrom.try_from`

```python
IntTryFromFloat = Patch Int
IntTryFromFloat.
    try_from r: Float =
        if r.ceil() == r:
            then: r.ceil()
            else: Error "conversion failed".
```

---

<span id="1" style="font-size:x-small"><sup>1</sup> This conversion is a byproduct of the current implementation and will be removed in the future. [↩](#f1) </span>

<p align='center'>
    <a href='./16_subtyping.md'>上一頁</a> | <a href='./18_mut.md'>下一頁</a>
</p>