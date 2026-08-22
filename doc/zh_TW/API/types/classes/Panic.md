# Panic

異常終止。與[`Never`](./Never.md)一樣沒有實例，但`Never`表示正常終止或故意的無限循環，而`Panic`表示程序放棄了處理。`Never <: Panic`

```python,checker_ignore
always_fails(): Panic = panic "boom"
always_exits(): Panic = exit 0
```

## `T or Panic`

對於返回`T`但也可能終止的子程序，寫作`T or Panic`。與會塌縮為`T`的`T or Never`不同，它保留在簽名中——保留正是書寫它的目的

```python
positive(x: Int): Int or Panic = if x > 0, do x, do panic "must be positive"
```

該類型的值仍可直接作為`T`使用，因為`Panic`沒有實例：如果真的是它，程序早已停止

```python,checker_ignore
positive(3) * 2         # OK，無需 narrowing
takes_int(positive(3))  # OK
```

但**裸的**`Panic`不是`T`：宣告返回`Panic`的子程序不會正常返回，因此沒有可用的值。如果可能返回，請寫`T or Panic`

```python,checker_ignore
f(): Int = always_fails() # TypeError
```

## 尚未實現的部分

`panic`、`todo`和`unreachable`返回`Never`而不是`Panic`，以便`f(): Int = todo()`這樣的樁仍能通過類型檢查

可能終止的運算尚未用`Panic`定型：`Div`trait被規定為`.'/' = (self: Self, R) -> Self.Output or Panic`，但除法目前仍在沒有它的情況下定型
