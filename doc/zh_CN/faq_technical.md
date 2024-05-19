# 技术常见问题

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/faq_technical.md%26commit_hash%3Dc6eb78a44de48735213413b2a28569fdc10466d0)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/faq_technical.md&commit_hash=c6eb78a44de48735213413b2a28569fdc10466d0)

本节回答有关使用 Erg 语言的技术问题。换句话说，它包含以 What 或 Which 开头的问题，以及可以用 Yes/No 回答的问题

有关如何确定语法的更多信息，请参阅 [此处](./faq_syntax.md) 了解基础语法决策，以及 [此处](./faq_general.md)

## Erg 中有异常机制吗?

答: 不会。Erg 使用 `Result` 类型代替。请参阅 [此处](./faq_syntax.md) 了解 Erg 没有异常机制的原因

## Erg 是否有与 TypeScript 的 `Any` 等价的类型?

答: 不，没有。所有对象都至少属于 `Object` 类，但是这种类型只提供了一组最小的属性，所以你不能像使用 Any 那样对它做任何你想做的事情
`Object` 类通过`match` 等动态检查转换为所需的类型。它与Java 和其他语言中的`Object` 是同一种
在 Erg 世界中，没有像 TypeScript 那样的混乱和绝望，其中 API 定义是"Any"

## Never、{}、None、()、NotImplemented 和 Ellipsis 有什么区别?

A: `Never` 是一种"不可能"的类型。产生运行时错误的子例程将"Never"(或"Never"的合并类型)作为其返回类型。该程序将在检测到这一点后立即停止。尽管 `Never` 类型在定义上也是所有类型的子类，但 `Never` 类型的对象永远不会出现在 Erg 代码中，也永远不会被创建。`{}` 等价于 `Never`
`Ellipsis` 是一个表示省略号的对象，来自 Python
`NotImplemented` 也来自 Python。它被用作未实现的标记，但 Erg 更喜欢产生错误的 `todo` 函数
`None` 是 `NoneType` 的一个实例。它通常与 `Option` 类型一起使用
`()` 是一个单元类型和它自己的一个实例。当您想要返回"无意义的值"(例如过程的返回值)时使用它

## 为什么 `x = p!()` 有效但 `f() = p!()` 会导致 EffectError?

`!` 不是副作用产品的标记，而是可能导致副作用的对象
过程 `p!` 和可变类型 `T!` 会引起副作用，但如果 `p!()` 的返回值是 `Int` 类型，它本身就不再引起副作用

## 当我尝试使用 Python API 时，对于在 Python 中有效的代码，我在 Erg 中收到类型错误。这是什么意思?

A: Erg API 的类型尽可能接近 Python API 规范，但有些情况无法完全表达
此外，根据规范有效但被认为不合需要的输入(例如，在应该输入 int 时输入浮点数)可能会被 Erg 开发团队酌情视为类型错误。

## Why doesn't Tuple have a constructor (`__call__`)?

Erg tuples must have a compile-time length. Therefore, a tuple is constructed almost only by a tuple literal.
If the length is not known until runtime, an immutable array (`List`) can be used instead.

```erg
arr = List map(int, input!().split " ")
```

## I got runtime errors in Erg that I did not get in Python. What could be the cause?

The following script is an example of a strange error that can occur in Erg.

```erg
{main!; TestCase!} = pyimport "unittest"

Test! = Inherit TestCase!
Test!
    test_one self =
        self.assertEqual 1, 1

main!()
```

This is a basic use of unittest, and at first glance it looks correct, but when executed, it produces the following error:

```console
AttributeError: 'Test!' object has no attribute '_testMethodName'.
```

The error is caused by the way `TestCase` is executed.
When `TestCase` (a class that extends `TestCase`) is executed, the test method to be executed must begin with `test_`.
`test_one` seems to follow this, but Erg performs mangling on variable names.
This is what makes the test method unrecognizable.
To avoid mangling, you need to enclose the name in ''.

```erg
{main!; TestCase!} = pyimport "unittest"

Test! = Inherit TestCase!
Test!
    'test_one' self =
        self.assertEqual 1, 1

main!()
```

This time it works.

If you get Erg-specific errors, you can suspect the side-effects of mangling, etc.

## All Python APIs used from Erg have type declarations based on the latest Python version, does this mean that older versions of Python are not supported?

No. Erg is compatible with Python versions from 3.7 to the latest.
Erg uses its own code generator and libraries to absorb the differences between versions of the Python API.
If it does not, it is a bug and please report it to [issues](https://github.com/erg-lang/erg/issues/new) on GitHub.
