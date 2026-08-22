# Never

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Never.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Never.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

它是所有类型的子类型。它是一个`Class`，因为它拥有所有的方法，当然还有 `.new`。但是，它没有实例，并且Erg会在即将创建的那一刻停止
还有一种叫做`Panic`的类型没有实例，但是`Never`用于正常终止或故意无限循环，`Panic`用于异常终止

```python,checker_ignore
# Never <: Panic
f(): Panic = exit 0 # OK
g(): Panic = panic "..." # OK

may_fail(): Int or Panic = ...
h(): Never = may_fail() # TypeError: `Panic`不是`Never`
```

`panic`、`todo`和`unreachable`返回`Never`而不是`Panic`，以便`f(): Int = todo()`这样的桩仍能通过类型检查

`T or Never`就是`T`：`Never`在语义上是一个从不出现的选项(如果出现了，程序会立即停止)，因此在构造 OR 类型时即被吸收
`T or Panic`不会被吸收——它保留在签名中，因为它告诉读者程序可能会终止——但出于同样的原因，该类型的值仍可作为`T`使用。参见[Panic](./Panic.md)