# 特殊形式

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/API/special.md%26commit_hash%3D8673a0ce564fd282d0ca586642fa7f002e8a3c50)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/API/special.md&commit_hash=8673a0ce564fd282d0ca586642fa7f002e8a3c50)

特殊形式是不能在 Erg 类型系统中表达的运算符、子程序(等等)。它被`包围，但实际上无法捕获
此外，为方便起见，还出现了"Pattern"、"Body"和"Conv"等类型，但不存在此类类型。它的含义也取决于上下文

## `=`(pat: Pattern, body: Body) -> NoneType

将 body 分配给 pat 作为变量。如果变量已存在于同一范围内或与 pat 不匹配，则引发错误
它还用于记录属性定义和默认参数

```python
record = {i = 1; j = 2}
f(x: Int, y = 2) = ...
```

当主体是类型或函数时，`=` 具有特殊行为
左侧的变量名嵌入到右侧的对象中

```python
print! Class() # <class <lambda>>
print! x: Int -> x + 1 # <function <lambda>>
C = Class()
print! c # <class C>
f = x: Int -> x + 1
print! f # <function f>
g x: Int = x + 1
print! g # <function g>
K X: Int = Class(...)
print! K # <kind K>
L = X: Int -> Class(...)
print! L # <kind L>
```

`=` 运算符的返回值为"未定义"
函数中的多个赋值和 `=` 会导致语法错误

```python
i = j = 1 # SyntaxError: 不允许多次赋值
print!(x=1) # SyntaxError: cannot use `=` in function arguments
# 提示: 您的意思是关键字参数(`x: 1`)吗?
if True, do:
    i = 0 # SyntaxError: 块不能被赋值表达式终止
```

## `->`(pat: Pattern, body: Body) -> Func

生成匿名函数，函数类型

## `=>`(pat: Pattern, body: Body) -> Proc

生成匿名过程，过程类型

## `:`(x, T)

声明对象 `x` 属于 `T` 类型。如果 `x` 不是 `T` 的子类型，则会出错

它可用于变量声明或表达式的右侧值

```erg
# 两者都没问题
x: Int = 1
y = x： Int
```

## `as`(x, T)

强制将对象 `x` 转换为 `T` 类型。如果 `x` 不是 `T` 的子类型，则会出错

与 `:` 不同的是 `(x: T)： U` 当 `x： U; U <： T`，而 `(x as T)： T`

## `.`(obj, attr)

读取obj的属性
`x.[y, z]` 将 x 的 y 和 z 属性作为数组返回

## `|>`(obj, c: Callable)

执行`c(obj)`。`x + y |>.foo()` 与 `(x + y).foo()` 相同

### (x: Option T)`?` -> T

后缀运算符。如果出现错误，请立即调用 `x.unwrap()` 和 `return`

## match(obj, *lambdas: Lambda)

对于 obj，执行与模式匹配的 lambda

```python
match [1, 2, 3]:
  (l: Int) -> log "this is type of Int"
  [[a], b] -> log a, b
  [*a] -> log a
# (1, 2, 3)
```

## Del(*x: T) -> NoneType

删除变量"x"。但是，无法删除内置对象

```python
a = 1
Del a # OK

Del True # SyntaxError: cannot delete a built-in object
```

## do(body: Body) -> Func

生成一个不带参数的匿名函数。`() ->` 的语法糖

## do!(body: Body) -> Proc

生成不带参数的匿名过程。`() =>` 的语法糖

## 集合运算符

### `[]`(*objs)

从参数创建一个数组或从可选参数创建一个字典

### `{}`(*objs)

从参数创建一个集合

### `{}`(*fields: ((Field, Value); N))

生成记录

### `{}`(layout, *names, *preds)

生成筛型

### `*`

展开嵌套集合。它也可以用于模式匹配

```python
[x, *y] = [1, 2, 3]
assert x == 1 and y == [2, 3]
assert [x, *y] == [1, 2, 3]
assert [*y, x] == [2, 3, 1]
{x; *yz} = {x = 1; y = 2; z = 3}
assert x == 1 and yz == {y = 2; z = 3}
assert {x; *yz} == {x = 1; y = 2; z = 3}
```

## 虚拟运算符

用户不能直接使用的运算符

### ref(x: T) -> Ref T

返回对对象的不可变引用

### ref!(x: T!) -> Ref! T!

返回对可变对象的可变引用。
