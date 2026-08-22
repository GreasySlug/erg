# Panic

异常终止。与[`Never`](./Never.md)一样没有实例，但`Never`表示正常终止或故意的无限循环，而`Panic`表示程序放弃了处理。`Never <: Panic`

```python,checker_ignore
always_fails(): Panic = panic "boom"
always_exits(): Panic = exit 0
```

## `T or Panic`

对于返回`T`但也可能终止的子例程，写作`T or Panic`。与会塌缩为`T`的`T or Never`不同，它保留在签名中——保留正是书写它的目的

```python
positive(x: Int): Int or Panic = if x > 0, do x, do panic "must be positive"
```

该类型的值仍可直接作为`T`使用，因为`Panic`没有实例：如果真的是它，程序早已停止

```python,checker_ignore
positive(3) * 2         # OK，无需 narrowing
takes_int(positive(3))  # OK
```

但**裸的**`Panic`不是`T`：声明返回`Panic`的子例程不会正常返回，因此没有可用的值。如果可能返回，请写`T or Panic`

```python,checker_ignore
f(): Int = always_fails() # TypeError
```

## 尚未实现的部分

`panic`、`todo`和`unreachable`返回`Never`而不是`Panic`，以便`f(): Int = todo()`这样的桩仍能通过类型检查

可能终止的运算尚未用`Panic`定型：`Div`trait被规定为`.'/' = (self: Self, R) -> Self.Output or Panic`，但除法目前仍在没有它的情况下定型
