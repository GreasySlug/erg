# 类型推断算法

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/compiler/inference.md%26commit_hash%3Dcac2c51cd4405b0166fcd2be35c23be6412c4028)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/compiler/inference.md&commit_hash=cac2c51cd4405b0166fcd2be35c23be6412c4028)

> __Warning__: 此部分正在编辑中，可能包含一些错误

显示了下面使用的符号

```python
Free type variables (type, unbound): ?T, ?U, ...
Free-type variables (values, unbound): ?a, ?b, ...
type environment (Γ): { x: T, ... }
Type assignment rule (S): { ?T --> T, ... }
Type argument evaluation environment (E): { e -> e', ... }
```

我们以下面的代码为例:

```python
v = ![]
v.push! 1
print! v
```

Erg 的类型推断主要使用 Hindley-Milner 类型推断算法(尽管已经进行了各种扩展)。具体而言，类型推断是通过以下过程执行的。术语将在后面解释

* Calling polymorphic functions (or classes)
* Defining polymorphic functions (or classes)
* Attribute resolution
* Subtype checking

In Erg, control flow such as `if` and `for!` are just (polymorphic) functions, and operators can also be regarded as (polymorphic) functions with one or two arguments.
For monomorphic functions, only subtype determination is sufficient.
The [attribute_resolution](./attribute_resolution.md) and [subtyping](./subtyping.md) are described in a separate section.
This section describes the type inference mechanism for function calls and definitions.

Specifically, type inference is performed in the following steps. Explanation of terminology and other details are described below.

1. 推断右值的类型(搜索)
2. 实例化结果类型
3. 如果是调用，执行类型替换(substitute)
4. 解决已经单态化的Trait
5. 如果有类型变量值，求值/归约(eval)
6. 删除链接类型变量(deref)
7. 传播可变依赖方法的变化
8. 如果有左值并且是Callable，则泛化参数类型(generalize)
9. 如果有左值，对(返回值)类型进行泛化(generalize)
10. 如果是赋值，则在符号表(`Context`)中注册类型信息(更新)

具体操作如下

第 1 行。Def{sig: v, block: ![]}
    获取块类型:
        获取 UnaryOp 类型:
            getList 类型: `['T; 0]`
            实例化: `[?T; 0]`
            (替代，评估被省略)
    更新: `Γ: {v: [?T; 0]！}`
    表达式 返回`NoneType`: OK

第 2 行 CallMethod {obj: v, name: push!, args: [1]}
    获取 obj 类型: `List!(?T, 0)`
        搜索: `Γ List!(?T, 0).push!({1})`
        得到: `= List!('T ~> 'T, 'N ~> 'N+1).push!('T) => NoneType`
        实例化: `List!(?T, ?N).push!(?T) => NoneType`
        替代(`S: {?T --> Nat, ?N --> 0}`): `List!(Nat ~> Nat, 0 ~> 0+1).push!(Nat) => NoneType`
        评估: `List!(Nat, 0 ~> 1).push!({1}) => NoneType`
        更新: `Γ: {v: [Nat; 1]！}`
    表达式 返回`NoneType`: OK

第 3 行。调用 {obj: print!, args: [v]}
    获取参数类型: `[[Nat; 1]!]`
    获取 obj 类型:
        搜索: `Γ print!([Nat; 1]!)`
        得到: `= print!(*Obj) => NoneType`
    表达式 返回`NoneType`: OK

## 类型变量的实现

类型变量最初在 [ty.rs](../../../crates/erg_compiler/ty/mod.rs) 的 `Type` 中表示如下。它现在以不同的方式实现，但本质上是相同的想法，所以我将以更天真的方式考虑这种实现
`RcCell<T>` 是 `Rc<RefCell<T>>` 的包装类型

```rust
pub enum Type {
    ...
    Var(RcCell<Option<Type>>), // a reference to the type of other expression, see docs/compiler/inference.md
    ...
}
```

类型变量可以通过将实体类型保存在外部字典中来实现，并且类型变量本身只有它的键。但是，据说使用 `RcCell` 的实现通常更有效(需要验证，[来源](https://mobile.twitter.com/bd_gfngfn/status/1296719625086877696?s=21))

类型变量首先被初始化为 `Type::Var(RcCell::new(None))`
当分析代码并确定类型时，会重写此类型变量
如果内容直到最后都保持为 None ，它将是一个无法确定为具体类型的类型变量(当场)。例如，具有 `id x = x` 的 `x` 类型
我将这种状态下的类型变量称为 __Unbound 类型变量__(我不知道确切的术语)。另一方面，我们将分配了某种具体类型的变量称为 __Linked 类型变量__

两者都是自由类型变量(该术语显然以"自由变量"命名)。这些是编译器用于推理的类型变量。它之所以有这样一个特殊的名字，是因为它不同于程序员指定类型的类型变量，例如 `id: 'T -> 'T` 中的 `'T`

未绑定类型变量表示为`?T`、`?U`。在类型论的上下文中，经常使用 α 和 β，但这一种是用来简化输入的
请注意，这是出于一般讨论目的而采用的表示法，实际上并未使用字符串标识符实现

进入类型环境时，未绑定的类型变量 `Type::Var` 被替换为 `Type::MonoQuantVar`。这称为 __quantified 类型变量__。这类似于程序员指定的类型变量，例如"T"。内容只是一个字符串，并没有像自由类型变量那样链接到具体类型的工具

用量化类型变量替换未绑定类型变量的操作称为__generalization__(或泛化)。如果将其保留为未绑定类型变量，则类型将通过一次调用固定(例如，调用 `id True` 后，`id 1` 的返回类型将是 `Bool`)，所以它必须是概括的
以这种方式，在类型环境中注册了包含量化类型变量的通用定义

## 概括、类型方案、具体化

让我们将未绑定类型变量 `?T` 泛化为 `gen` 的操作表示。令生成的广义类型变量为 `|T: Type| T`
在类型论中，量化类型，例如多相关类型 `α->α`，通过在它们前面加上 `?α.` 来区分(像 ? 这样的符号称为(通用)量词。)
这样的表示(例如`?α.α->α`)称为类型方案。Erg 中的类型方案表示为 `|T: Type| T -> T`
类型方案通常不被认为是一流的类型。以这种方式配置类型系统可以防止类型推断起作用。但是，在Erg中，在一定条件下可以算是一流的类型。有关详细信息，请参阅 [rank2 类型](../syntax/type/advanced/_rank2type.md)

现在，当在使用它的类型推断(例如，`id 1`，`id True`)中使用获得的类型方案(例如`'T -> 'T(id's type scheme)`)时，必须释放generalize。这种逆变换称为 __instantiation__。我们将调用操作`inst`

```python
gen ?T = 'T
inst 'T = ?T (?T ? Γ)
```

重要的是，这两个操作都替换了所有出现的类型变量。例如，如果你实例化 `'T -> 'T`，你会得到 `?T -> ?T`
实例化需要替换 dict，但为了泛化，只需将 `?T` 与 `'T` 链接以替换它

之后，给出参数的类型以获取目标类型。此操作称为类型替换，将用 `subst` 表示
此外，如果表达式是调用，则获取返回类型的操作表示为 `subst_call_ret`。第一个参数是参数类型列表，第二个参数是要分配的类型

类型替换规则 `{?T --> X}` 意味着将 `?T` 和 `X` 重写为相同类型。此操作称为 __Unification__。`X` 也可以是类型变量
[单独部分] 中描述了详细的统一算法。我们将统一操作表示为"统一"

```python
unify(?T, Int) == Ok(()) # ?T == (Int)

# S为类型分配规则，T为适用类型
subst(S: {?T --> X}, T: ?T -> ?T) == X -> X
# Type assignment rules are {?T --> X, ?U --> T}
subst_call_ret([X, Y], (?T, ?U) -> ?U) == Y
```

## 半统一(semi-unification)

统一的一种变体称为半统一(__Semi-unification__)。这是更新类型变量约束以满足子类型关系的操作
在某些情况下，类型变量可能是统一的，也可能不是统一的，因此称为"半"统一

例如，在参数分配期间会发生半统一
因为实际参数的类型必须是形式参数类型的子类型
如果参数类型是类型变量，我们需要更新子类型关系以满足它

```python
# 如果形参类型是T
f(x: T): T = ...

a: U
# 必须为 U <: T，否则类型错误
f(a)
```

## 泛化

泛化不是一项简单的任务。当涉及多个作用域时，类型变量的"级别管理"就变得很有必要了
为了看到层级管理的必要性，我们首先确认没有层级管理的类型推断会导致问题
推断以下匿名函数的类型

```python
x ->
    y = x
    y
```

首先，Erg 分配类型变量如下:
y 的类型也是未知的，但暂时未分配

```python
x(: ?T) ->
    y = x
    y
```

首先要确定的是右值 x 的类型。右值是一种"用途"，因此我们将其具体化
但是 x 的类型 `?T` 已经被实例化了，因为它是一个自由变量。Yo`?T` 成为右值的类型

```python
x(: ?T) ->
    y = x (: inst ?T)
    y
```

注册为左值 y 的类型时进行泛化。然而，正如我们稍后将看到的，这种概括是不完善的，并且会产生错误的结果

```python
x(: ?T) ->
    y(:gen?T) = x(:?T)
    y
```

```python
x(: ?T) ->
    y(: 'T) = x
    y
```

y 的类型现在是一个量化类型变量"T"。在下一行中，立即使用 `y`。具体的

```python
x: ?T ->
    y(: 'T) = x
    y(: inst 'T)
```

请注意，实例化必须创建一个与任何已经存在的(自由)类型变量不同的(自由)类型变量(概括类似)。这样的类型变量称为新类型变量

```python
x: ?T ->
    y = x
    y(: ?U)
```

并查看生成的整个表达式的类型。`?T -> ?U`
但显然这个表达式应该是`?T -> ?T`，所以我们知道推理有问题
发生这种情况是因为我们没有"级别管理"类型变量

所以我们用下面的符号来介绍类型变量的层次。级别表示为自然数

```python
# 普通类型变量
?T<1>, ?T<2>, ...
# 具有子类型约束的类型变量
?T<1>(<:U) or ?T(<:U)<1>, ...
```

让我们再尝试一次:

```python
x ->
    y = x
    y
```

首先，按如下方式分配一个 leveled 类型变量:  toplevel 级别为 1。随着范围的加深，级别增加
函数参数属于内部范围，因此它们比函数本身高一级

```python
# level 1
x (: ?T<2>) ->
    # level 2
    y = x
    y
```

首先，实例化右值`x`。和以前一样，没有任何改变

```python
x (: ?T<2>) ->
    y = x (: inst ?T<2>)
    y
```

这是关键。这是分配给左值`y`的类型时的概括
早些时候，这里的结果很奇怪，所以我们将改变泛化算法
如果类型变量的级别小于或等于当前范围的级别，则泛化使其保持不变

```python
gen ?T<n> = if n <= current_level, then= ?T<n>, else= 'T
```

```python
x (: ?T<2>) ->
    # current_level = 2
    y(: gen ?T<2>) = x(: ?T<2>)
    y
```

That is, the lvalue `y` has type `?T<2>`.

```python
x (: ?T<2>) ->
    # ↓ 不包括
    y(: ?T<2>) = x
    y
```

y 的类型现在是一个未绑定的类型变量 `?T<2>`。具体如下几行: 但是 `y` 的类型没有被概括，所以什么也没有发生

```python
x (: ?T<2>) ->
    y(: ?T<2>) = x
    y (: inst ?T<2>)
```

```python
x (: ?T<2>) ->
    y = x
    y (: ?T<2>)
```

我们成功获得了正确的类型`?T<2> -> ?T<2>`

让我们看另一个例子。这是更一般的情况，具有函数/运算符应用程序和前向引用

```python
fx, y = id(x) + y
id x = x

f10,1
```

让我们逐行浏览它

在 `f` 的推断过程中，会引用后面定义的函数常量 `id`
在这种情况下，在 `f` 之前插入一个假设的 `id` 声明，并为其分配一个自由类型变量
注意此时类型变量的级别是`current_level`。这是为了避免在其他函数中泛化

```python
id: ?T<1> -> ?U<1>
f x (: ?V<2>), y (: ?W<2>) =
    id(x) (: subst_call_ret([inst ?V<2>], inst ?T<1> -> ?U<1>)) + y
```

类型变量之间的统一将高级类型变量替换为低级类型变量
如果级别相同，则无所谓

类型变量之间的半统一有点不同
不同级别的类型变量不得相互施加类型约束

```python
# BAD
f x (: ?V<2>), y (: ?W<2>) =
    # ?V<2>(<: ?T<1>)
    # ?T<1>(:> ?V<2>)
    id(x) (: ?U<1>) + y (: ?W<2>)
```

这使得无法确定在何处实例化类型变量
对于 Type 类型变量，执行正常统一而不是半统一
也就是说，统一到下层

```python
# OK
f x (: ?V<2>), y (: ?W<2>) =
    # ?V<2> --> ?T<1>
    id(x) (: ?U<1>) + y (: ?W<2>)
```

```python
f x (: ?T<1>), y (: ?W<2>) =
    (id(x) + x): subst_call_ret([inst ?U<1>, inst ?W<2>], inst |'L <: Add('R)| ('L, 'R) -> 'L .AddO)
```

```python
f x (: ?T<1>), y (: ?W<2>) =
    (id(x) + x): subst_call_ret([inst ?U<1>, inst ?W<2>], (?L(<: Add(?R<2>))<2>, ?R<2 >) -> ?L<2>.AddO)
```

```python
id: ?T<1> -> ?U<1>
f x (: ?T<1>), y (: ?W<2>) =
    # ?U<1>(<: Add(?W<2>)) # Inherit the constraints of ?L
    # ?L<2> --> ?U<1>
    # ?R<2> --> ?W<2> (not ?R(:> ?W), ?W(<: ?R))
    (id(x) + x) (: ?U<1>.AddO)
```

```python
# current_level = 1
f(x, y) (: gen ?T<1>, gen ?W<2> -> gen ?U<1>.AddO) =
    id(x) + x
```

```python
id: ?T<1> -> ?U<1>
f(x, y) (: |'W: Type| (?T<1>, 'W) -> gen ?U<1>(<: Add(?W<2>)).AddO) =
    id(x) + x
```

```python
f(x, y) (: |'W: Type| (?T<1>, 'W) -> ?U<1>(<: Add(?W<2>)).AddO) =
    id(x) + x
```

定义时，提高层次，使其可以泛化

```python
# ?T<1 -> 2>
# ?U<1 -> 2>
id x (: ?T<2>) -> ?U<2> = x (: inst ?T<2>)
```

如果已经分配了返回类型，则与结果类型统一(`?U<2> --> ?T<2>`)

```python
# ?U<2> --> ?T<2>
f(x, y) (: |'W: Type| (?T<2>, 'W) -> ?T<2>(<: Add(?W<2>)).AddO) =
    id(x) + x
# current_level = 1
id(x) (: gen ?T<2> -> gen ?T<2>) = x (: ?T<2>)
```

如果类型变量已经被实例化为一个简单的类型变量，
依赖于它的类型变量也将是一个 Type 类型变量
广义类型变量对于每个函数都是独立的

```python
f(x, y) (: |'W: Type, 'T <: Add('W)| ('T, 'W) -> 'T.AddO) =
    id(x) + x
id(x) (: |'T: Type| 'T -> gen 'T) = x
```

```python
f x, y (: |'W: Type, 'T <: Add('W)| ('T, 'W) -> 'T.AddO) =
    id(x) + y
id(x) (: 'T -> 'T) = x

f(10, 1) (: subst_call_ret([inst {10}, inst {1}], inst |'W: Type, 'T <: Add('W)| ('T, 'W) -> 'T .AddO)
```

```python
f(10, 1) (: subst_call_ret([inst {10}, inst {1}], (?T<1>(<: Add(?W<1>)), ?W<1>) -> ? T<1>.AddO))
```

类型变量绑定到具有实现的最小类型

```python
# ?T(:> {10} <: Add(?W<1>))<1>
# ?W(:> {1})<1>
# ?W(:> {1})<1> <: ?T<1> (:> {10}, <: Add(?W(:> {1})<1>))
# serialize
# {1} <: ?W<1> or {10} <: ?T<1> <: Add({1}) <: Add(?W<1>)
# Add(?W)(:> ?V) 的最小实现Trait是 Add(Nat) == Nat，因为 Add 相对于第一个参数是协变的
# {10} <: ?W<1> or {1} <: ?T<1> <: Add(?W<1>) <: Add(Nat) == Nat
# ?T(:> ?W(:> {10}) or {1}, <: Nat).AddO == Nat # 如果只有一个候选人，完成评估
f(10, 1) (: (?W(:> {10}, <: Nat), ?W(:> {1})) -> Nat)
# 程序到此结束，所以去掉类型变量
f(10, 1) (: ({10}, {1}) -> Nat)
```

整个程序的结果类型是:

```python
f|W: Type, T <: Add(W)|(x: T, y: W): T.AddO = id(x) + y
id|T: Type|(x: T): T = x

f(10, 1): Nat
```

我还重印了原始的、未明确键入的程序

```python
fx, y = id(x) + y
id x = x

f(10, 1)
```
