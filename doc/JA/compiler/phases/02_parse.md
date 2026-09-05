# Parsing (構文解析)

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/compiler/phases/02_parse.md%26commit_hash%3D19bab4ae63af9415da20ebd7499c668144da5ea6)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/compiler/phases/02_prarse.md&commit_hash=19bab4ae63af9415da20ebd7499c668144da5ea6)

構文解析を行うのは`erg_parser/parse.rs`に定義される`Parser`である。これも使い捨ての構造体であり、主に`ParserRunner`でラップして使う。
`Parser`は再帰下降構文解析を行う。スタックオーバーフローを避けるため、デフォルトのスタックが小さいWindowsでは、手動でスタックサイズが指定された別スレッド上で実行される。

Ergの文法の特徴的な点は、case-sensitiveであること、また最悪の場合いくら先読みしても文法が確定しないことである。

例えば、以下の構文を考える。

```python
a, b, c, d, e, (...)
```

(...)の中で=が現れればこれはタプルの分割代入と判明する。現れなければ単なるタプルである。
しかし、どちらであるかを決定するのに必要なトークンの数は上限がない。

そこで`Parser`は上のような場合、まずタプルであると決め打ちして解析を進める。
改行が来る前に=または->, =>が来たら、これは分割代入であると判明し、今まで解析したタプルを左辺値に変換する。
その他のパターン、関数定義もこれと同様の方法で解析される。
このようなことが可能なのは、すべての左辺値に対して構文的に双対となる右辺値が存在するからである(しかし、すべての右辺値に対して双対となる左辺値があるわけでない)。

## 式解析エンジン (precedence climbing)

式の解析は単一のエンジン`try_reduce_expr_prec(min_prec, ctx)`が行う。
これは演算子優先順位法(precedence climbing)による実装で、`token.rs`の`TokenKind::precedence()`が返す優先順位表に基づいて二項演算子を結合する。

> 歴史的経緯: かつては文レベルと式レベルの解析がそれぞれ`Vec<ExprOrOp>`を用いたshift-reduce方式のスタックマシンとして実装されており、ほぼ同一のロジックが2箇所に重複していた。現在は`try_reduce_expr_prec`の上に単一のエントリポイント`try_reduce_expr(ctx)`があるだけである(TODO「Replace `Parser`」対応)。

### 構造

```text
try_reduce_expr(ctx)                       # エントリポイント。ExprCtx::CHUNK なら定義を許可
        └──→ try_reduce_expr_prec(0, ctx)  # 共通エンジン
                ├── try_reduce_bin_lhs()   # オペランド(リテラル、アクセサ、コンテナ、lambda等)
                └── loop {  後続トークンで分岐  }
```

解析コンテキストは`ExprCtx`構造体(Copy)で持ち回る。呼び出し側は定数から始めて差分だけ上書きする:
`self.try_reduce_expr(ExprCtx { winding: true, ..ExprCtx::EXPR })`。

| フラグ | 意味 |
| ------ | ---- |
| `chunk` | 文レベル。定義(`=`)・メソッド定義ブロック(`::`/`.` + 改行)・`式 引数`形式の呼び出しを許可 |
| `winding` | 括弧なしタプル(`1, 2, 3`)・デフォルト引数(`x := 1`)を許可 |
| `in_type_args` | 型引数`T\|...\|`の内部 |
| `in_brace` | `{}`内部。`:`を型指定ではなくkey-valueの区切りとして扱う |
| `line_break` | `()`内部で複数行に渡る式を許可 |

### 二項演算子の結合

エンジンはオペランドを1つ読んだ後、後続の二項演算子の優先順位`op_prec`が`min_prec`以上であれば
`try_reduce_expr_prec(op_prec + 1, ctx)`を再帰呼び出しして右オペランドを解析する(`+ 1`によりすべて左結合になる)。
`min_prec`未満の演算子は消費せずにbreakし、外側の呼び出しに委ねる。
明示的な演算子スタックを持たないため、式ごとのVec確保やスタックの再走査は発生しない。

### 後置的構文

アクセサ(`.attr`/`::attr`/`[index]`)、lambda(`->`/`=>`)、型指定(`: T`/`<:`/`:>`/`as`)、
括弧なしタプル(`,`)は「直近のオペランド」に結合する。
すなわち`a + b.c`は`a + (b.c)`、`x == y -> y`は`x == (y -> y)`となる。
再帰構造上、これらは現在の再帰レベルの`lhs`に適用するだけで正しい結合が得られるため、`min_prec`に関係なく常に処理される。

### 最外レベル限定の構文

以下は係留中の二項演算子が存在しない(`min_prec == 0`の)場合のみ有効である。

- 定義`=`: `a + b = ...`のように演算子が残っている場合は「余分な式・演算子が残っています」エラー
- メソッド定義ブロック(`C::` / `C.` + 改行)
- ストリーム演算子`|>`: すべての二項演算子より弱く結合する(`a + b |> f`は`f(a + b)`)。内側の再帰は`|>`でbreakし、完全に結合された式が第一引数として渡される
