# Error

Erg自己的可恢復錯誤。返回`T or Error`是子程序報告「呼叫方應當處理的失敗」的方式，與終止程序的`Panic`相對

```python
half(x: Int): Int or Error =
    if x % 2 == 0:
        do x // 2
        do Error("not even", kind := "ValueError")
```

## 屬性

* `.msg: Str` — 出了什麼問題
* `.kind: Str` — 錯誤的種類，未指定時為`"Error"`(`Error(msg, kind := ...)`)
* `.stack: List(ErrorFrame, _)` — 該錯誤經由[`?`](../../special.md)返回時所途經的子程序，最內層的呼叫在前

`.msg`和`.kind`不是上下文：它們在錯誤建立時就屬於該錯誤，永遠不會被覆蓋

## 方法

### `.context(self, msg: Str) -> Error`

消費該錯誤並返回一個多帶一條提示的新錯誤。呼叫可以鏈式書寫，因此一個錯誤可以持有多個上下文

```python
not_ready(): Error =
    e = Error "not implemented yet"
    e.context("to be implemented in ver 1.2").context("and more hints ...")
```

## 堆疊追蹤

每當`Error`被`?`返回時，該呼叫位置都會被壓入`.stack`。當`?`到達無法`return`的位置(頂層)時，收集到的軌跡會被列印，程序隨即退出

```python,checker_ignore
try_some(x: Int): Int or Error = ...

f x =
    y = try_some(x)?
    ...

g x =
    y = f(x)?
    ...

i = g(1)?
# Traceback (most recent call first):
#   f, line 5, file "foo.er"
#   5 | y = try_some(x)?
#   g, line 8, file "foo.er"
#   8 | y = f(x)?
#   <module>, line 11, file "foo.er"
#   11 | i = g(1)?
# Error: ...
```

`ErrorFrame`是該軌跡的一個條目，具有`.name: Str`、`.line: Nat`和`.file: Str`
