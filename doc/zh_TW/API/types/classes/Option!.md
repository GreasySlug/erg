# Option! T

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Option!.md%26commit_hash%3Dd15cbbf7b33df0f78a575cff9679d84c36ea3ab1)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Option!.md&commit_hash=d15cbbf7b33df0f78a575cff9679d84c36ea3ab1)

可以替換內容的[`Option T`](./Option.md)單元格

`Option`只是`T or NoneType`的別名，因此無法用通常的方式得到可變版本：Or 類型不是名義類型，沒有地方實現`Mutizable`，`!`也無法生成它。所以`Option!`是一個獨立的類，其實例透過呼叫它來建立，而不是透過可變化一個`Option`

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

建立一個裝著`value`的單元格。不傳值則得到一個空單元格

* get(self: Ref(Option! T)) -> T or NoneType

單元格當前的值。它就是普通的[`Option T`](./Option.md)，因此[`?`](../../special.md)、`.unwrap`和narrowing都可以照常使用

```python
c: Option! Int = Option!(1)
assert c.get().unwrap() == 1

incr(opt: Option! Int): Int or NoneType =
    x = opt.get()?
    x + 1

print! incr(c) # 2
```

* set!(self: RefMut(Option! T), value: T or NoneType) => NoneType

把`value`放進單元格。傳`None`則清空它

* clear!(self: RefMut(Option! T)) => NoneType

清空單元格。`o.clear!()`等同於`o.set!(None)`

沒有`.update!`。它需要接受`(T or NoneType) -> (T or NoneType)`，而參數為含類型變數的 Or 類型的 lambda 目前還無法推斷。請讀取、套用、再寫回

```python
counter: Option! Int = Option!(0)
counter.set!(counter.get().unwrap_or(0) + 1)
print! counter.get() # 1
```

## 關於元素類型

`T`來自建立單元格時的值，類型註解不會將其放寬。與`List!`一樣，`Option!`對`T`是協變的，因此`Option!({1})`已經滿足`Option! Int`。寫入同一類型的其他值仍然沒有問題

```python
o = Option!("a")
o.set! "b"
print! o.get() # b
```

不帶值建立的`Option!()`沒有確定`T`的依據，類型註解也無法提供。`T`由第一次`.set!`確定。在那之前只能使用不依賴`T`的操作(例如`print!`)

```python
empty = Option!()
print! empty.get() # None
empty.set! "now a Str"
print! empty.get() # now a Str
```
