# Option! T

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Option!.md%26commit_hash%3Dd15cbbf7b33df0f78a575cff9679d84c36ea3ab1)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Option!.md&commit_hash=d15cbbf7b33df0f78a575cff9679d84c36ea3ab1)

可以替换内容的[`Option T`](./Option.md)单元格

`Option`只是`T or NoneType`的别名，因此无法用通常的方式得到可变版本：Or 类型不是名义类型，没有地方实现`Mutizable`，`!`也无法生成它。所以`Option!`是一个独立的类，其实例通过调用它来创建，而不是通过可变化一个`Option`

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

创建一个装着`value`的单元格。不传值则得到一个空单元格

* get(self: Ref(Option! T)) -> T or NoneType

单元格当前的值。它就是普通的[`Option T`](./Option.md)，因此[`?`](../../special.md)、`.unwrap`和narrowing都可以照常使用

```python
c: Option! Int = Option!(1)
assert c.get().unwrap() == 1

incr(opt: Option! Int): Int or NoneType =
    x = opt.get()?
    x + 1

print! incr(c) # 2
```

* set!(self: RefMut(Option! T), value: T or NoneType) => NoneType

把`value`放进单元格。传`None`则清空它

* clear!(self: RefMut(Option! T)) => NoneType

清空单元格。`o.clear!()`等同于`o.set!(None)`

没有`.update!`。它需要接受`(T or NoneType) -> (T or NoneType)`，而参数为含类型变量的 Or 类型的 lambda 目前还无法推断。请读取、应用、再写回

```python
counter: Option! Int = Option!(0)
counter.set!(counter.get().unwrap_or(0) + 1)
print! counter.get() # 1
```

## 关于元素类型

`T`来自创建单元格时的值，类型注解不会将其放宽。与`List!`一样，`Option!`对`T`是协变的，因此`Option!({1})`已经满足`Option! Int`。写入同一类型的其他值仍然没有问题

```python
o = Option!("a")
o.set! "b"
print! o.get() # b
```

不带值创建的`Option!()`没有确定`T`的依据，类型注解也无法提供。`T`由第一次`.set!`确定。在那之前只能使用不依赖`T`的操作(例如`print!`)

```python
empty = Option!()
print! empty.get() # None
empty.set! "now a Str"
print! empty.get() # now a Str
```
