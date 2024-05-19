# `ergc` 的架构

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/compiler/architecture.md%26commit_hash%3Deb5b9c4946152acaecc977f47062958ce4e774a2)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/compiler/architecture.md&commit_hash=eb5b9c4946152acaecc977f47062958ce4e774a2)

## 1. 扫描 Erg 脚本 (.er) 并生成 `TokenStream` (parser/lex.rs)

* parser/lexer/Lexer 生成`TokenStream`(这是一个Token的迭代器，TokenStream可以通过lexer.collect()生成)
  * `Lexer` 由 `Lexer::new` 或 `Lexer::from_str` 构造，其中 `Lexer::new` 从文件或命令选项中读取代码
  * `Lexer` 可以作为迭代器按顺序生成令牌；如果您想一次获得 `TokenStream`，请使用 `Lexer::lex`
  * `Lexer` 输出 `LexError` 为错误，但 `LexError` 没有足够的信息显示自己。如果要显示错误，请使用 `LexerRunner` 转换错误
  * 如果你想单独使用 `Lexer`，也可以使用 `LexerRunner`；`Lexer` 只是一个迭代器，并没有实现 `Runnable` 特性
    * `Runnable` 由 `LexerRunner`、`ParserRunner`、`Compiler` 和 `VirtualMachine` 实现

## 2. 转换 `TokenStream` -> `AST` (parser/parse.rs)

* `Parser` 和 `Lexer` 一样，有两个构造函数，`Parser::new` 和 `Parser::from_str`，而 `Parser::parse` 会给出 `AST`
* `AST` 是 `Vec<Expr>` 的包装器类型

### 2.5 脱糖 `AST`

* 扩展嵌套变量 (`Desugarer::desugar_nest_vars_pattern`)
* desugar 多模式定义语法(`Desugarer::desugar_multiple_pattern_def`)

## 3. 类型检查和推断，转换 `AST` -> `HIR` (compiler/lower.rs)

(main) src: [erg_compiler/lower.rs](../../../crates/erg_compiler/lower.rs)

### 3.1 Name Resolution

In the current implementation it is done during type checking.

* All ASTs (including imported modules) are scanned for name resolution before type inference.
* In addition to performing cycle checking and reordering, a context is created for type inference (however, most of the information on variables registered in this context is not yet finalized).

### 3.2 import resolution

* When `import` is called, a new thread is created for analysis.
* `JoinHandle` is stored in `SharedCompilerResource` and is joined when the module is needed.
* Unused modules may not be joined, but currently all such modules are also analyzed.

### 3.3 Type checking & inference

* `HIR` 有每个变量的类型信息。它是用于"高级中间表示"的
* `HIR` 只保存变量的类型，但这已经足够了。在极端情况下，这是因为 Erg 只有转换(或运算符)应用程序的参数对象
* `ASTLower` 可以用与`Parser` 和`Lexer` 相同的方式构造
* 如果没有错误发生，`ASTLowerer::lower` 将输出 `HIR` 和 `CompileWarnings` 的元组
* `ASTLowerer`归`Compiler`所有。与传统结构不同，`ASTLowerer`处理代码上下文并且不是一次性的
* 如果类型推断的结果不完整(如果存在未知类型变量)，名称解析时会出错

## 4. 检查副作用(compiler/effectcheck.rs)

## 4. 检查所有权(compiler/memcheck.rs)

## 5. 从`HIR`(compiler/codegen.rs)生成字节码(`CodeObj`)

* 根据表达式的类型信息，将执行量化子程序的名称解析

## (6.(未来计划)转换字节码 -> LLVM IR)

* 字节码是基于堆栈的，而 LLVM IR 是基于寄存器的
  这个转换过程会多出几层中间过程。