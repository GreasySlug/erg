# Option T = T or NoneType

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Option.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Option.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

表示"可能失败"的类型。它不是独立的类型而是别名：`Option Int`**就是**`Int or NoneType`，在期望类型的位置两者可以互换

```python
x: Option Int = 1
y: Option Int = None

first(l: List(Int, 3)): Option Int = l.get(0)
```

[`?`](../../special.md)运算符将`NoneType`视为错误一侧，因此对`Option T`应用`?`会得到`T`，否则外围子例程返回`None`

```python
sum_two(l: List(Int, 3)): Option Int =
    a = l.get(0)?
    b = l.get(1)?
    a + b
```

## methods

下面的方法在当场解决失败，与把失败交给调用者的`?`相对。它们是为带有错误一侧的`T or E`定义的，因此[`Result`](./Result.md)也有同样的四个

* unwrap(self, msg := Str) -> T or [Panic](./Panic.md)

期望内容为`T`类型并将其取出。如果是`None`，则输出`msg`并panic

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap() == 1
```

```python,checker_ignore
x: Option Int = None
x.unwrap() # UnwrappingError: unwrapped a None value
x.unwrap("failed to convert from string to number") # UnwrappingError: failed to convert from string to number
```

值一路收集的提示和`?`的帧也会一并输出，因此取出后仍能看到踪迹

* unwrap_or(self, else: T) -> T

取出它，如果是`None`则返回`else`。无论哪种情况`else`都会被求值

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap_or(0) == 1
```

* unwrap_or_exec(self, f: () -> T) -> T

相同，只是`f`仅在没有可取出的值时才被调用

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap_or_exec(() -> 0) == 1
```

* unwrap_or_exec!(self, p!: () => T) -> T

用于有副作用的回退的`unwrap_or_exec`。它是过程，因此只能从过程中调用

```python
log! = !0
recompute!() =
    log!.inc!()
    0

main!() =
    first(l: List(Int, 3)): Option Int = l.get(0)
    assert first([1, 2, 3]).unwrap_or_exec!(recompute!) == 1
    assert log! == 0

main!()
```
