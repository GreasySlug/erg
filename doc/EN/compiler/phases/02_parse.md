# Parsing

The `Parser` defined in `erg_parser/parse.rs` performs the parsing. It is also a disposable structure, primarily used wrapped in `ParserRunner`.
The `Parser` performs recursive descent parsing. To avoid stack overflow, on Windows where the default stack size is small, it is executed on a separate thread with a manually specified stack size.

A distinctive feature of Erg's syntax is that it is case-sensitive, and in the worst case, the syntax cannot be determined no matter how much it is pre-read.

For example, consider the following syntax.

```python
a, b, c, d, e, (...)
```

If an '=' appears within (...), it is determined to be a tuple destructuring assignment. If it does not appear, it is simply a tuple.
However, there is no upper limit to the number of tokens needed to determine which it is.

Therefore, in such cases, the `Parser` first assumes it is a tuple and proceeds with parsing.
If an '=', '->', or '=>' appears before a newline, it is determined to be a destructuring assignment, and the previously parsed tuple is converted to a left-hand value.
Other patterns, such as function definitions, are also parsed in a similar manner.
This is possible because for every left-hand value, there exists a syntactically dual right-hand value (although not every right-hand value has a dual left-hand value).

## Expression parsing engine (precedence climbing)

Expressions are parsed by a single engine, `try_reduce_expr_prec(min_prec, ctx)`.
It implements precedence climbing, combining binary operators according to the precedence table returned by `TokenKind::precedence()` in `token.rs`.

> Historical note: statement-level and expression-level parsing used to be two shift-reduce stack machines over `Vec<ExprOrOp>`, with nearly identical logic duplicated in both. Both are now the single entry point `try_reduce_expr(ctx)` over `try_reduce_expr_prec` (this addressed the "Replace `Parser`" TODO item).

### Structure

```text
try_reduce_expr(ctx)                       # the entry point; ExprCtx::CHUNK allows definitions
        └──→ try_reduce_expr_prec(0, ctx)  # the shared engine
                ├── try_reduce_bin_lhs()   # operands (literals, accessors, containers, lambdas, ...)
                └── loop {  dispatch on the following token  }
```

The parsing context is carried around in the `ExprCtx` struct (Copy). Callers start
from one of its constants and override only what differs, e.g.
`self.try_reduce_expr(ExprCtx { winding: true, ..ExprCtx::EXPR })`.

| flag | meaning |
| ---- | ------- |
| `chunk` | statement level: definitions (`=`), method-definition blocks (`::`/`.` + newline) and `expr args` calls are allowed |
| `winding` | paren-less tuples (`1, 2, 3`) and default parameters (`x := 1`) are allowed |
| `in_type_args` | inside type arguments `T\|...\|` |
| `in_brace` | inside `{}`: `:` is a key-value separator, not a type ascription |
| `line_break` | the expression may span multiple lines (inside parentheses) |

### Combining binary operators

After reading one operand, whenever the precedence `op_prec` of the following binary operator is at least `min_prec`,
the engine recurses with `try_reduce_expr_prec(op_prec + 1, ctx)` to parse the right operand (the `+ 1` makes every operator left-associative).
An operator below `min_prec` is left unconsumed and the loop breaks, handing it to the outer call.
Since there is no explicit operator stack, no per-expression `Vec` allocation or stack rescanning takes place.

### Postfix-like constructs

Accessors (`.attr`/`::attr`/`[index]`), lambdas (`->`/`=>`), type ascriptions (`: T`/`<:`/`:>`/`as`)
and paren-less tuples (`,`) bind to the nearest operand:
`a + b.c` parses as `a + (b.c)` and `x == y -> y` as `x == (y -> y)`.
Thanks to the recursive structure, applying them to the current recursion level's `lhs` yields the correct binding, so they are handled regardless of `min_prec`.

### Outermost-level-only constructs

The following are only valid when no binary operator is pending (i.e. `min_prec == 0`):

- definitions (`=`): if an operator remains, as in `a + b = ...`, an "extra expression or operator remains" error is reported
- method-definition blocks (`C::` / `C.` + newline)
- the stream operator `|>`: it binds more loosely than every binary operator (`a + b |> f` is `f(a + b)`). Inner recursion levels break on `|>`, so the fully combined expression is passed as the first argument
