# Erg 代码如何转译成 Python 代码?

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/compiler/transpile.md%26commit_hash%3D96b113c47ec6ca7ad91a6b486d55758de00d557d)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/compiler/transpile.md&commit_hash=96b113c47ec6ca7ad91a6b486d55758de00d557d)

准确地说，Erg 代码是被转译为 Python 字节码。鉴于 Python 字节码几乎可以被重构为 Python 文本代码，因此这里以等效的 Python 代码为例。
顺便说一下，这里展示的示例是低优化级别。更高级的优化会消除不需要实例化的东西。

## 记录，记录类型

它将被转换为一个命名元组（namedtuple）。
对于 namedtuple，请参阅 [此处](https://docs.python.org/zh-cn/3/library/collections.html#collections.namedtuple)。
有一个类似的功能，数据类（dataclass），但由于__eq__和__hash__的自动实现，数据类在性能上略有下降。

```python
employee = Employee.new({.name = "John Smith"; .id = 100})
assert employee.name == "John Smith"
```


```python
employee = NamedTuple(['name', 'id'])('John Smith', 100)
assert employee.name == 'John Smith'
```

如果可以进一步优化，它还将被转换为简单的元组。

## 多态类型

> 在制品

## 即时范围

如果没有发生命名空间冲突，它只会被破坏和扩展
`x::y` 等名称在字节码中使用，不能与 Python 代码关联，但如果强制表示，则会如下所示

```python
x =
    y = 1
    y+1
```

```python
x::y = 1
x = x::y + 1
```

万一发生冲突，定义和使用只能在内部引用的函数

```python
x =
    y = 1
    y+1
```

```python
def _():
    x=1
    y = x
    return y + 1
x = _()
```

## 可见性

它对公共变量没有任何作用，因为它是 Python 的默认值
私有变量由 mangling 处理

```python
x=1
y =
    x = 2
    assert module::x == 2
```

```python
module::x = 1
y::x = 2
assert module::x == 2
y = None
```

## Patch

```python
func b: Bool =
    Invert = Patch Bool
    Invert.
        invert self = not self
    b.invert()
```

```python
def func(b):
    def Invert::invert(self): return not self
    return Invert::invert(b)
```
