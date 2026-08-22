# Error

Erg自己的可恢复错误。返回`T or Error`是子例程报告"调用方应当处理的失败"的方式，与终止程序的`Panic`相对

```python
half(x: Int): Int or Error =
    if x % 2 == 0:
        do x // 2
        do Error("not even", kind := "ValueError")
```

## 属性

* `.msg: Str` — 出了什么问题
* `.kind: Str` — 错误的种类，未指定时为`"Error"`(`Error(msg, kind := ...)`)
* `.stack: List(ErrorFrame, _)` — 该错误经由[`?`](../../special.md)返回时所途经的子例程，最内层的调用在前

`.msg`和`.kind`不是上下文：它们在错误创建时就属于该错误，永远不会被覆盖

## 方法

### `.context(self, msg: Str) -> Error`

消费该错误并返回一个多带一条提示的新错误。调用可以链式书写，因此一个错误可以持有多个上下文

```python
not_ready(): Error =
    e = Error "not implemented yet"
    e.context("to be implemented in ver 1.2").context("and more hints ...")
```

## 堆栈跟踪

每当`Error`被`?`返回时，该调用位置都会被压入`.stack`。当`?`到达无法`return`的位置(顶层)时，收集到的轨迹会被打印，程序随即退出

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

`ErrorFrame`是该轨迹的一个条目，具有`.name: Str`、`.line: Nat`和`.file: Str`
