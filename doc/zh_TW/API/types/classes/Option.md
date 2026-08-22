# Option T = T or NoneType

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/types/classes/Option.md%26commit_hash%3D06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/types/classes/Option.md&commit_hash=06f8edc9e2c0cee34f6396fd7c64ec834ffb5352)

表示「可能失敗」的類型。它不是獨立的類型而是別名：`Option Int`**就是**`Int or NoneType`，在期望類型的位置兩者可以互換

```python
x: Option Int = 1
y: Option Int = None

first(l: List(Int, 3)): Option Int = l.get(0)
```

[`?`](../../special.md)運算符將`NoneType`視為錯誤一側，因此對`Option T`應用`?`會得到`T`，否則外圍子程序返回`None`

```python
sum_two(l: List(Int, 3)): Option Int =
    a = l.get(0)?
    b = l.get(1)?
    a + b
```

## methods

下面的方法在當場解決失敗，與把失敗交給呼叫者的`?`相對。它們是為帶有錯誤一側的`T or E`定義的，因此[`Result`](./Result.md)也有同樣的四個

* unwrap(self, msg := Str) -> T or [Panic](./Panic.md)

期望內容為`T`類型並將其取出。如果是`None`，則輸出`msg`並panic

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap() == 1
```

```python,checker_ignore
x: Option Int = None
x.unwrap() # UnwrappingError: unwrapped a None value
x.unwrap("failed to convert from string to number") # UnwrappingError: failed to convert from string to number
```

值一路收集的提示和`?`的幀也會一併輸出，因此取出後仍能看到蹤跡

* unwrap_or(self, else: T) -> T

取出它，如果是`None`則返回`else`。無論哪種情況`else`都會被求值

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap_or(0) == 1
```

* unwrap_or_exec(self, f: () -> T) -> T

相同，只是`f`僅在沒有可取出的值時才被呼叫

```python
first(l: List(Int, 3)): Option Int = l.get(0)

assert first([1, 2, 3]).unwrap_or_exec(() -> 0) == 1
```

* unwrap_or_exec!(self, p!: () => T) -> T

用於有副作用的回退的`unwrap_or_exec`。它是程序，因此只能從程序中呼叫

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
