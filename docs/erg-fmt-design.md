# `erg fmt` 設計書

Erg 向けコードフォーマッタ (`cargo fmt` 相当) の設計。

> 一言まとめ: **Lexer にコメントを emit させ、トークン列から空白・インデント・空行を正規化し、
> 長すぎる行を括弧の内側で折り返す。Erg は空白が意味論的に有意な言語なので、
> 整形結果を再 lex して元とトークン列が一致することを検証し、
> 一致しなければ整形を破棄する「自己検証型フォーマッタ」にする。**

- **方式**: トークンストリーム整形（AST/CST は使わない）
- **設定**: CLI フラグ3つのみ（§1.2）。設定ファイルは持たない
- **折り返し**: 括弧の内側でのみ、カンマで割る。演算子は行頭に置く（§5.7）
- **抑止**: `# fmt: off` / `# fmt: on` / `# fmt: skip` と `--exclude`（§5.6）
- **スコープ**: フォーマッタのみ。linter (`erg_linter`) の拡張は本書の対象外

---

## 1. 目的と非目標

### 1.1 目的

| 項目 | 内容 |
| ---- | ---- |
| `erg fmt <file>` | ファイルをその場で整形 |
| `erg fmt --check` | 差分があれば非ゼロ終了（CI 用）。書き換えない |
| `erg fmt --stdout` | 整形結果を標準出力へ |
| ELS `textDocument/formatting` | エディタの保存時整形 |

### 1.2 設定項目

**CLI フラグ3つのみ。設定ファイル (`erg-fmt.toml` 等) は持たない。**

| フラグ | 既定値 | 意味 |
| ------ | ------ | ---- |
| `--indent <n>` | `4` | ネスト1段あたりの空白数 |
| `--max-blank-lines <n>` | `2` | 連続してよい空行の上限 |
| `--max-width <n>` | `100` | 行長の上限。超過した行は括弧内で折り返す（§5.7） |

設定ファイルを持たない理由は §12 を参照。

### 1.3 非目標

- **行を結合しない。** reflow は「長すぎる行を割る」方向にのみ働く。
  すでに複数行に分かれている式を1行にまとめ直すことはしない（§5.7 の理由を参照）
- **括弧の外では改行しない。** 折り返せるのは括弧の内側だけ。実測に基づく安全規則（§5.7.1）
- **設定ファイルを持たない。** 設定は §1.2 の3フラグのみで、リポジトリごとのスタイル分岐を作らない
- **式の並べ替え・書き換えを行わない。** 括弧の付け外し、`if` の `elif` 化などは linter の `--fix` の領分
- **型検査を要求しない。** パースすら通らないコードでも、lex できる範囲で整形する

---

## 2. 前提: 現状のレキサが持つ制約

実装前に把握しておくべき、`crates/erg_parser/lex.rs` の性質。**ここが設計の全てを決めている。**

### 2.1 コメントは完全に捨てられている 【要改修】

`lex_comment` ([lex.rs:385](../crates/erg_parser/lex.rs#L385)) と `lex_multi_line_comment`
([lex.rs:408](../crates/erg_parser/lex.rs#L408)) は、コメント本文を `s` に集めるが
**トークンを一切 emit せず `Ok(())` を返す**。`s` は bidi 文字（双方向オーバーライド）検査のためだけに
組み立てられている。

したがって現状のトークン列からコメントは復元できない。**Phase 1 の改修対象。**

なお呼び出し位置は `Iterator::next()` の先頭1箇所のみ
([lex.rs:1222-1230](../crates/erg_parser/lex.rs#L1222-L1230)) で、
行末コメント (`x = 1 # foo`) も行頭コメントも同じ経路を通る。改修点は局所的。

### 2.2 `Newline` / `Indent` / `Dedent` はトークン化されている

`lex_space_indent_dedent` ([lex.rs:469](../crates/erg_parser/lex.rs#L469)) が
`Newline` / `Indent` / `Dedent` を通常のトークンとして emit する。
ブロック構造はトークン列だけで追跡できる。

- `Indent` の `content` は**実際の空白文字列**そのもの (`" ".repeat(indent_len)`,
  [lex.rs:552](../crates/erg_parser/lex.rs#L552))
- `Dedent` の `content` は空文字列
- `indent_stack` は**相対**インデント幅のスタック。Erg はインデント幅を強制しない

→ 再インデントは「`Indent`/`Dedent` の出現でネスト深さを数え、深さ × 4 の空白を出力する」で済む。

### 2.3 空行は `Newline` の連続として現れる

`enclosure_level == 0` の改行は1行につき `Newline` 1個
([lex.rs:481-487](../crates/erg_parser/lex.rs#L481-L487))。
空行が2行あれば `Newline` が3個連続する。**空行数はトークン列から復元可能。**

### 2.4 括弧の中では改行が消える 【設計上の急所】

`enclosure_level > 0`（`(` `[` `{` の内側）では、改行は
`self.next()` の再帰呼び出しで**握り潰される**([lex.rs:1481-1485](../crates/erg_parser/lex.rs#L1481-L1485))。

```erg
f(
    a,
    b,
)
```

は `f ( a , b , )` というトークン列になり、`Newline` も `Indent` も一切現れない。

**対策**: `Token` は `lineno` を持ち、括弧内でも `lineno_token_starts` は正しく加算されている。
よって**改行の有無は「隣接トークンの `lineno` の差」から復元する**。

これを括弧の内外で統一的に使う。つまりフォーマッタの駆動軸は
`Newline` トークンではなく **`lineno` の差分**とする。`Newline` トークンは
「論理行の終端」を示すマーカーとしてのみ使う。

### 2.5 文字列リテラルの `content` は原文と一致しない 【設計上の最大の急所】

`lex_single_str` ([lex.rs:895](../crates/erg_parser/lex.rs#L895)) は
**エスケープシーケンスを復号した結果**を `content` に格納する。

| ソース | `Token::content` |
| ------ | ---------------- |
| `"a\nb"` | `"a` + 改行文字 + `b"` |
| `"\x41"` | `"A"` |
| `"a\tb"` | `"a    b"`（タブ → 空白4個。**不可逆**） |

したがって **`Token::content` をそのまま出力に使うと文字列リテラルが破壊される。**
`"a\nb"` が「クォート内に実際の改行が入った壊れたソース」になる。

さらに `col_end` も信用できない。`emit_singleline_token` は
`col_token_starts += cont.chars().count()` として**復号後の長さ**で桁を進めるため、
エスケープを含む文字列では `col_end` が原文とずれる
（`token.rs:349-351` のコメント「`col_end - col_start` は必ずしも `content.len()` と等しくない」はこの件）。

→ **`Token` に原文テキストを持たせる必要がある。Phase 1 の改修対象。**

### 2.6 空白が意味論的に有意 【安全性の根拠】

`op_fix_of` ([lex.rs:320-351](../crates/erg_parser/lex.rs#L320-L351)) は
**演算子の前後の空白から前置/中置を決定する**。

```text
x + 1   →  Plus     (中置)  … 加算
x +1    →  PrePlus  (前置)  … x を +1 に適用（関数呼び出し）
x+1     →  Plus     (中置)
x+ 1    →  Plus     (中置)
```

つまり「二項演算子の前後には空白1個」という素朴な規則は、
**`x +1` を `x + 1` に変えて意味を壊す。**

**救い**: レキサは前置と中置を*別の `TokenKind`* として区別する
（`PrePlus` / `Plus`、`PreMinus` / `Minus`、`PreStar` / `Star`、`PreDblStar` / `Pow`、
[token.rs:54-70](../crates/erg_parser/token.rs#L54-L70)）。

よって**空白規則を `TokenKind` ごとに定義すれば fixity は保存される**:

- `Plus`（中置）→ 前後に空白1個 → `x + 1`
- `PrePlus`（前置）→ 前に空白1個、後ろに空白なし → `x +1`

それでも見落としは起こりうるので、§6 の検証を必須とする。

---

## 3. 全体アーキテクチャ

```text
                     ┌──────────────── Phase 1 ────────────────┐
source ──> Lexer (keep_comments = true) ──> Vec<Token> (raw 付き)
                                                  │
                     ┌──────────────── Phase 2 ───┼─────────────┐
                     │                            v             │
                     │   1. 論理行への分割    LogicalLine 列     │
                     │   2. 抑止範囲の確定 (# fmt: off/on/skip)  │
                     │   3. 物理行の復元 (lineno 差分)           │
                     │   4. 空白正規化 (kind ペア規則)           │
                     │   5. 折り返し (括弧内のカンマでのみ)      │
                     │   6. インデント/空行の正規化              │
                     │                            │             │
                     │                            v             │
                     │                        String            │
                     │                            │             │
                     │   7. 検証: 再 lex してトークン列比較 ─── NG ──> 元のソースを返す
                     └────────────────────────────┼─────────────┘
                                                  v OK
                     ┌──────── Phase 3 ───────────┴─── Phase 4 ────────┐
                     │  erg fmt (CLI)                ELS formatting    │
                     └─────────────────────────────────────────────────┘
```

新規クレート `crates/erg_fmt/` を追加し、`erg_parser` にのみ依存させる。
**`erg_compiler` には依存しない**（型検査不要という非目標を構造的に保証する）。

---

## 4. Phase 1: `erg_parser` の改修

既存の挙動を一切変えないことを絶対条件とする。フォーマッタ用の情報は**すべてオプトイン**にする。

> **【Phase 1 完了】** Step 1〜3 とも実装済み。§4.4 の完了条件を満たしている。
> 実装中に既存バグを1件発見し修正した（§10.1）。

### 4.1 `TokenKind::Comment` の追加

```rust
// token.rs
pub enum TokenKind {
    // ...
    /// `# ...` および `#[ ... ]#`
    Comment,
    // ...
}
```

`TokenCategory` には新カテゴリ `Ignorable` を追加し、`Comment` を割り当てる。

> **注意**: `op_fix_of` は `self.prev_token.category()` で分岐する
> ([lex.rs:321](../crates/erg_parser/lex.rs#L321))。`Comment` を `prev_token` に
> 記録してしまうと `x #c\n+1` のような入力で fixity 判定が変わる恐れがある。
> **`Comment` トークンは `self.prev_token` を更新しないこと。**
> `emit_singleline_token` は `prev_token` を書き換える
> ([lex.rs:255](../crates/erg_parser/lex.rs#L255)) ので、専用の emit 関数を用意するか、
> 呼び出し後に `prev_token` を復元する。ここが Phase 1 で最も壊しやすい箇所。

**【実装済み】** `emit_comment` が `prev_token` を保存・復元する方式を採った。
`category()` の最後に `_ => TokenCategory::BinOp` というフォールバックがある
（[token.rs](../crates/erg_parser/token.rs)）ため、`Comment` の arm を**明示的に**
書かないとコメントが二項演算子として分類される。ここも踏みやすい罠だった。
回帰テストは `tests/comment_token_test.rs::a_comment_does_not_change_operator_fixity`。

### 4.2 `Lexer` に `keep_comments` フラグ

```rust
pub struct Lexer {
    // ...
    /// フォーマッタ用。true のときコメントをトークンとして emit する
    keep_comments: bool,
}

impl Lexer {
    /// ビルダー。既定は false なので既存の呼び出し元は影響を受けない
    pub fn keep_comments(mut self) -> Self { self.keep_comments = true; self }
}
```

`lex_comment` / `lex_multi_line_comment` のシグネチャを
`LexResult<Option<Token>>` に変え、`next()` で `Some(tok)` なら即 return する
（`match self.consume()` へ進まない）。次の `next()` 呼び出しで本来のトークンが読まれる。

**既定を `false` にする理由**: `Lexer` の利用者は
[parse.rs:110](../crates/erg_parser/parse.rs#L110) / [parse.rs:371](../crates/erg_parser/parse.rs#L371) /
[parse.rs:658](../crates/erg_parser/parse.rs#L658)、ELS の
[file_cache.rs](../crates/els/file_cache.rs)（セマンティックトークン用に3箇所）、
`tests/tokenize_test.rs`、`benches/parser_bench.rs` と広範囲にある。
既定で `Comment` が流れ出すと全部が壊れる。

`#[ ... ]#` は改行を跨ぐため、`lex_multi_line_comment` 内の
`s.clear()`（[lex.rs:428](../crates/erg_parser/lex.rs#L428)）を `keep_comments` 時は行わず、
`emit_multiline_token(Comment, ...)` で emit する。

### 4.3 `Token` に原文フィールドを追加

§2.5 の問題への対処。

```rust
pub struct Token {
    pub kind: TokenKind,
    pub content: Str,
    /// 原文テキスト。`content` と一致する場合は `None`（＝ほぼ全てのトークン）
    pub raw: Option<Box<Str>>,
    pub lineno: u32,
    pub col_begin: u32,
    pub col_end: u32,
}

impl Token {
    /// 整形出力に使うべきテキスト
    pub fn raw_text(&self) -> &str {
        match &self.raw {
            Some(raw) => raw,
            None => &self.content,
        }
    }

    /// 原文が `content` と一致するなら `None` に正規化する
    /// （エスケープを含まない文字列リテラルで無駄な確保をしないため）
    pub fn with_raw<S: Into<Str>>(mut self, raw: S) -> Self {
        let raw = raw.into();
        self.raw = (raw != self.content).then(|| Box::new(raw));
        self
    }
}
```

**`Box` する理由（実測）**: `raw` は文字列リテラル以外では常に `None` なので、
`Option<Str>` をインラインで持つと**全トークンが 24 バイト太る**。実測値:

| 型 | 変更前 | `Option<Str>` | `Option<Box<Str>>` |
| -- | ------ | ------------- | ------------------ |
| `Token` | 40 | 64 (+24) | **48 (+8)** |
| `Literal` | 40 | 64 | **48** |
| `Expr` | 368 | 416 (+48) | **384 (+16)** |

`Box` 1個分の 8 バイトに抑え、実体はそれを必要とする稀なトークンだけが持つ。
確保が走るのはエスケープを含む文字列リテラルのときのみ。

**`raw` を設定するのは文字列系のみ**:
`StrLit` / `StrInterpLeft` / `StrInterpMid` / `StrInterpRight` / `DocComment`
（`DocComment` は名前に反して文字列レキサ経由で生成される。
[lex.rs:159-160](../crates/erg_parser/lex.rs#L159-L160)）。
`lex_single_str` などでエスケープ復号後の `s` と並行して原文 `raw` を組み立て、
`emit_*_token` に渡す。他のトークン（記号・数値・識別子・`Indent`）は `content` が原文と一致するので `None`。

**互換性への影響**:

- `PartialEq` / `Hash` は手書き実装で `kind` と `content` しか見ていない
  （[token.rs:379-391](../crates/erg_parser/token.rs#L379-L391)）ので、`raw` を無視する挙動が自動的に得られる
- `Token::DUMMY` と `Token::dummy` は `const fn`（[token.rs:409-424](../crates/erg_parser/token.rs#L409-L424)）だが
  `raw: None` は const 文脈で書ける
- 構築箇所は**15箇所・2ファイル**（`token.rs` 9、[desugar.rs](../crates/erg_parser/desugar.rs) 6）。
  構造体リテラルは網羅性が要求されるため**コンパイラが漏れなく列挙してくれる**
- ただし `desugar.rs` の6箇所は `Token { content: "[".into(), kind: LSqBr, ..len.token }` という
  **関数的レコード更新**で、`raw` を追加すると `..len.token` が非 `Copy` フィールドを move してしまい
  「partially moved value」エラーになる。`raw: None` を明示すれば解決する。
  合成された `[` `]` `{` `}` が無関係なリテラルの原文を継承しては困るので、**意味論的にもこれが正しい**

> 【実装済み】上記は Step 1 として実施済み。`Debug` 出力は `raw` が `Some` のときだけ
> 表示するようにしたので、既存のトークン列テストは無改変で通る。
> `check_structs_size` に `Token` を追加して、以後サイズ変化を追えるようにした。
>
> **代替案**: `Token` を触らず、`lex_for_fmt()` が
> `(TokenStream, Vec<Option<Str>>)` を返す並行配列方式。侵襲は小さいが
> 添字ずれのバグを生みやすく、ELS からの再利用もしにくい。**採用しない。**

### 4.4 Phase 1 の完了条件

- `cargo test --features large_thread` が全て通る（既存挙動が不変であることの確認）
- `Lexer::from_str(src).keep_comments().lex()` が、コメントを含むソースについて
  `raw_text()` を連結すると**元のソースに完全一致する**こと（トークン間の空白を除く）

この「連結すると原文に戻る」性質が Phase 2 全体の土台になる。ここを Phase 1 のテストで固定する。

**【達成】** 対応するテスト:

| 完了条件 | テスト |
| -------- | ------ |
| 既存挙動が不変 | `comment_token_test.rs::keeping_comments_does_not_disturb_the_other_tokens`（コメントトークンを除くと既定のレキサの出力と完全一致することを8種のソースで確認）+ 既存スイート |
| 原文への復元（コメントなし） | `raw_text_test.rs::raw_text_round_trips_the_source` |
| 原文への復元（コメントあり） | `comment_token_test.rs::raw_text_round_trips_a_source_with_comments` |

`erg_parser` のテストは 60 件すべて green。ルートパッケージは 172 passed / 11 failed で、
この11件は本ブランチに元からある `exec_*` の失敗（変更前と同一）。

---

## 5. Phase 2: `erg_fmt` クレート

```text
crates/erg_fmt/
├── Cargo.toml
├── lib.rs        Formatter (Runnable 実装), format_str(), FmtOptions
├── line.rs       LogicalLine への分割
├── skip.rs       # fmt: off / on / skip の解釈
├── spacing.rs    トークン間空白の規則
├── reflow.rs     max_width 超過行の折り返し
├── render.rs     文字列生成 (Emit::Format / Emit::Verbatim)
├── verify.rs     再 lex による検証
└── tests/
    ├── fmt.rs
    └── fixtures/  *.er と *.expected.er のペア
```

### 5.1 データモデル

```rust
/// 1つの論理行（Newline で終わる、または括弧内で複数物理行にまたがる単位）
struct LogicalLine {
    /// ネスト深さ（Indent/Dedent から算出）
    depth: usize,
    /// この論理行を構成するトークン。Newline/Indent/Dedent は含まない
    tokens: Vec<Token>,
    /// 行末コメント
    trailing_comment: Option<Token>,
    /// 直前に何行の空行があったか（正規化前）
    blank_lines_before: usize,
}

enum Item {
    Line(LogicalLine),
    /// 行を占有するコメント
    OwnLineComment { depth: usize, token: Token, blank_lines_before: usize },
}
```

コメントの帰属は「そのコメントトークンの `lineno` が、直前の非コメントトークンの `lineno` と同じか」で決める。
同じなら `trailing_comment`、違えば `OwnLineComment`。

### 5.2 パイプライン

1. **分割** (`line.rs`): トークン列を走査し `Indent`/`Dedent` で `depth` を更新、`Newline` で論理行を確定。
   `Newline` の連続数 − 1 を次の行の `blank_lines_before` に記録
2. **抑止範囲の確定** (`skip.rs`): `# fmt: off` / `# fmt: on` / `# fmt: skip` を拾い、
   各 `Item` に「整形するか原文をそのまま出すか」の印を付ける（§5.6）
3. **物理行の復元**: 各 `LogicalLine` 内で `tokens[i].lineno != tokens[i-1].lineno` の位置に改行を入れる
   （§2.4）。継続行のインデントは `depth + 1` 相当に正規化する
4. **空白正規化** (`spacing.rs`): 同一物理行内の隣接トークン対に `space_between` を適用
5. **折り返し** (`reflow.rs`): `--max-width` 超過行を括弧内のカンマで割る（§5.7）
6. **描画** (`render.rs`): `depth * indent` の空白 + トークン列 + 行末コメント。
   空行は `--max-blank-lines` に丸める。抑止された `Item` は原文の行をそのまま出す
7. **検証** (`verify.rs`): §6

### 5.3 空白規則

`space_between(prev: &Token, next: &Token) -> bool`。**`TokenKind` を第一の判断材料**にし、
`TokenCategory` はフォールバックとして使う（§2.6 の fixity 保存のため、
`Plus` と `PrePlus` を category でまとめてはいけない）。

| 状況 | 規則 | 例 |
| ---- | ---- | --- |
| 中置演算子 (`Plus`, `Minus`, `Star`, `Slash`, `Pow`, 比較, `AndOp`, `OrOp` …) | 前後に空白1個 | `x + 1` |
| 前置演算子 (`PrePlus`, `PreMinus`, `PreStar`, `PreDblStar`, `PreBitNot`) | 前に1個、後ろに0個 | `x +1`, `*args` |
| 後置演算子 (`Try` = `?`) | 前後とも0個 | `x?` |
| `Comma`, `Semi` | 前0個、後ろ1個 | `f(a, b)` |
| `Colon` | 前0個、後ろ1個 | `x: Int` |
| `DblColon`, `Dot` | 前後とも0個 | `a.b`, `A::b` |
| `Assign`, `Walrus`, `FuncArrow`, `ProcArrow`, `SubtypeOf`, `SupertypeOf` | 前後に1個 | `x = 1`, `x -> x` |
| 開き括弧の直後 / 閉じ括弧の直前 | 0個 | `f(x)`, `[1, 2]` |
| `Mutate` (`!`) | 前後とも0個 | `x!`, `print!` |
| その他の隣接（識別子とリテラルなど） | 1個 | `f x` |

**保守的に扱う（＝原文の空白を保存する）ケース**:

- **開き括弧の直前**: `f(x)` と `f (x)` は同じトークン列になるため §6 の検証で差を検出できない。
  かつ Erg では `f (1, 2)`（タプル1個を渡す）と `f(1, 2)`（引数2個）で意味が変わりうる。
  → **`LParen`/`LSqBr`/`LBrace` の直前の空白の有無は原文どおりにする**
- **行頭の `Dot`**: `.x = 1` は公開属性の定義であって属性アクセスではない。
  `Dot` が物理行の先頭にある場合は「前後0個」規則を適用せず、後続に空白を入れない
- **文字列補間**: `StrInterpLeft` / `Mid` / `Right` の内側は
  1つの式として扱いつつ、`\{` `}` の直前直後には空白を入れない

### 5.4 インデントと空行

| 項目 | 規則 |
| ---- | ---- |
| インデント幅 | ネスト1段あたり空白 `--indent` 個（既定 4） |
| タブ | レキサが `Illegal` にする（[lex.rs:1492](../crates/erg_parser/lex.rs#L1492)）ので考慮不要 |
| 連続する空行 | `--max-blank-lines` 行に丸める（既定 2） |
| ファイル先頭の空行 | 削除 |
| ファイル末尾 | 改行1個で終端。末尾の空行は削除 |
| 行末の空白 | 削除 |
| 括弧内の継続行 | `depth + 1` 段でインデント |
| `--max-width` 超過行 | 括弧の内側で折り返す（§5.7）。括弧がなければ長いまま出力し `--check` 時に報告 |

### 5.5 冪等性

`format(format(src)) == format(src)` を不変条件とする。
`blank_lines_before` の丸めや継続行のインデントで違反しやすいので、
全てのテストフィクスチャに対して自動的に二重適用チェックを回す（§9）。

### 5.6 整形の抑止 (opt-out)

Erg のコメントは `#` 始まりで Python と同形なので、**black と同じコメントディレクティブ方式**を採る。
`rustfmt::skip` のような属性方式にしないのは、Erg の `@` デコレータが式に付けられる位置を
調べる必要があり、しかもデコレータはパースを要求するため「パースが通らなくても整形する」
という非目標 (§1.3) と衝突するため。**コメント方式なら Phase 1 で得た `Comment` トークンだけで完結する。**

| 書き方 | 効果 |
| ------ | ---- |
| `# fmt: off` | 以降を整形しない |
| `# fmt: on` | 整形を再開する |
| `# fmt: skip`（行末コメント） | **その論理行だけ**整形しない |
| `--exclude <glob>` | 該当ファイルを丸ごと対象外にする（複数指定可） |

- `# fmt: off` に対応する `# fmt: on` がなければ、ファイル末尾まで抑止が続く。
  ファイル先頭に `# fmt: off` を1行置けば「このファイルは整形しない」になる
- `off`/`on` は同じネスト深さになくてよい。行番号の範囲としてのみ扱う
- ディレクティブ自身の行は原文のまま出力する（インデントも変えない）
- 認識は**完全一致**とする。`#fmt:off` や `# FMT: OFF` は普通のコメント扱い。
  揺れを許すと「効いていないのに効いていると思い込む」事故が起きるため

**実装**: 抑止された範囲は「原文の該当行をそのまま出力する」だけで済む。
全トークンが `lineno` を持つので、`Item` から行範囲 `[start_lineno, end_lineno]` を求め、
`src.lines()` から該当行を切り出して出力する。**桁 (`col`) は一切使わない**ので、
§2.5 の `col_end` が信用できない問題を踏まない。

```rust
enum Emit {
    /// 通常どおり整形する
    Format,
    /// 原文の該当行をそのまま出す
    Verbatim { start_lineno: u32, end_lineno: u32 },
}
```

> **設計上の要点**: この `Verbatim` の経路は、§6 の検証に失敗したときのフォールバックと
> **同一のコード**になる。「整形をあきらめて原文を出す」という操作が1箇所に集約されるので、
> `# fmt: off` を実装すると検証フォールバックのテストも同時に効くようになる。
> 実装順序 (§11) でこの2つを隣接させているのはそのため。

### 5.7 折り返し (reflow)

`--max-width`（既定 100）を超えた物理行を、括弧の内側で複数行に割る。

#### 5.7.1 安全規則（実測に基づく）

Erg は空白とインデントが意味論的に有意なので、「どこで改行してよいか」は自明でない。
`erg --mode lex` で実測した結果、以下が確定している。

**規則1: 改行を入れてよいのは括弧の内側だけ。**

括弧の外で改行すると `Newline` トークンが増え、パースが変わる。
一方、括弧の内側 (`enclosure_level > 0`) では改行はレキサに握り潰される（§2.4）ので、
**トークン列は1ビットも変わらない**。

```text
_ = f(1, 2)            [UBar, Assign, Symbol f, LParen, NatLit 1, Comma, NatLit 2, RParen, Newline, EOF]
_ = f(                 [UBar, Assign, Symbol f, LParen, NatLit 1, Comma, NatLit 2, RParen, Newline, EOF]
    1,
    2
)                      ← 完全一致
```

`[ ]` でも同じ結果になることを確認済み。
さらに、括弧の内側では `Indent`/`Dedent` も一切 emit されない（`lex_indent_dedent` に到達しない）ため、
**括弧の中にインデントベースのブロック構造は存在しえない**。これが括弧内 reflow の安全性を構造的に保証している。

**規則2: 二項演算子は行頭に置く。行末に置いてはならない。**

`op_fix_of`（§2.6）は演算子直後の文字を見るため、演算子を行末に置くと直後が `\n` になり、
**中置が前置に化ける**。

```text
_ = (1 + 2)     →  Plus     ✅
_ = (1 +        →  PrePlus  ❌ 意味が変わる（1 を +2 に適用する呼び出しになる）
    2)
_ = (1          →  Plus     ✅ 行頭に置けば安全
    + 2)
```

rustfmt の `binop_separator = "Front"` と同じ結論に、Erg では**安全性の要請から**到達する。

**規則3: trailing comma を追加してはならない。**

閉じ括弧の直前にカンマを足すと `Comma` トークンが1個増え、トークン列が変わる。
パース自体は通る（`--mode check` で確認済み）が、§6 の検証を通らないので整形が破棄される。

→ **`erg fmt` は trailing comma を追加も削除もしない。**
ただし**原文にすでにある trailing comma を読み取ることは安全**なので、
これを「この括弧は展開したままにせよ」という利用者のシグナルとして使う（black の magic trailing comma の
読み取り専用版）。§5.7.3 を参照。

#### 5.7.2 アルゴリズム

初版は**カンマでのみ割る**。演算子での分割は §5.7.4（将来）。

1. 物理行を描画し、幅が `max_width` 以下ならそのまま
2. 超過していれば、その行に含まれる**最も外側の括弧対**を探す
3. その括弧の直下（相対ネスト深さ1）のカンマで割る。
   開き括弧は前の行の末尾に残し、各要素は `depth + 1` 段、閉じ括弧は元の `depth` で独立行に置く。
   **最後の要素にカンマを足さない**（規則3）
4. 生成された各行がまだ超過していれば、その行に対して 2 から再帰
5. **括弧を含まない行が超過していても、何もしない。**
   括弧の外では割れない（規則1）ので、長いまま出力し `--check` 時に一覧表示する

```erg
# before (max_width 超過)
result = compute(alpha, beta, gamma, delta)

# after
result = compute(
    alpha,
    beta,
    gamma,
    delta
)
```

#### 5.7.3 既存の改行を結合しない理由

reflow は「割る」方向にのみ働く。すでに複数行に分かれている式を1行に戻すことはしない。

canonical な出力（black/rustfmt のように一度平坦化してから幅で割り直す）にはしなかった。
平坦化は**既存コードを壊しうる**ためである。原文が

```erg
_ = (1 +
    2)
```

の場合、この `+` は `PrePlus` として lex されている。これを `(1 + 2)` に平坦化すると `Plus` になり、
トークン列が変わる。§6 の検証はこれを検出して**ファイル全体の整形を破棄する**ので安全ではあるが、
「整形できないファイル」が生まれる。割る方向にしか動かなければ、この事故は原理的に起きない。

代わりに、利用者が明示的に展開を維持したい場合は **trailing comma を置く**:

```erg
_ = f(
    1,
    2,     # ← このカンマがあれば、幅に収まっても展開したまま
)
```

`erg fmt` はこのカンマを追加も削除もせず、**存在すれば展開を維持する**シグナルとして読むだけ。
トークン列は変わらないので検証も通る。

#### 5.7.4 将来: 演算子での分割

カンマを含まない長い式（`a + b + c + ...` や長いメソッドチェーン）は初版では割れない。
実装する場合は規則2 に従い、**演算子を継続行の先頭**に置く:

```erg
total = (
    alpha
    + beta
    + gamma
)
```

外側に括弧がない式は括弧を補う必要があるが、それは「式の書き換え」（非目標 §1.3）に当たる。
`erg lint` 側で「長すぎる行は括弧で括って分割せよ」と警告する方が筋がよい。

---

## 6. 検証（自己検証型フォーマッタ）

**本設計の中核。** Erg は空白が意味論的に有意（§2.6）なので、整形は必ず検証する。

```rust
/// §1.2 の3項目だけを持つ。設定ファイルからは読まない
#[derive(Debug, Clone, Copy)]
pub struct FmtOptions {
    pub indent: usize,          // 既定 4
    pub max_blank_lines: usize, // 既定 2
    pub max_width: usize,       // 既定 100（§5.7 の折り返し閾値）
}

pub fn format_str(src: &str, opts: FmtOptions) -> String {
    let Ok(before) = Lexer::from_str(src.into()).keep_comments().lex() else {
        return src.to_string();   // lex できないなら何もしない
    };
    let formatted = render(src, &before, opts);
    let Ok(after) = Lexer::from_str(formatted.clone()).keep_comments().lex() else {
        return src.to_string();   // 整形結果が lex できない = バグ。原文を返す
    };
    if !token_streams_equivalent(&before, &after) {
        // 整形が意味を変えた。原文を返す
        debug_assert!(false, "erg fmt changed the token stream");
        return src.to_string();
    }
    formatted
}
```

`render` が原文 `src` も受け取るのは、§5.6 の `Emit::Verbatim` で行を切り出すため。

**この設計の価値**: 空白規則にバグがあっても、**ユーザーのコードが壊れることは絶対にない**。
最悪「整形されない」で済む。`debug_assert!` により、デバッグビルドとテストでは
バグが即座に顕在化する。

`--check` モードでは検証失敗時に「整形不能」として警告を出し、差分ありとは扱わない。

### 6.1 何を比較するか【実装済み】

当初案は「`Newline` / `Indent` / `Dedent` を除外して `(kind, content)` を比較」だったが、
実装時に2点修正した。

| トークン | 扱い | 理由 |
| -------- | ---- | ---- |
| `Newline` | **比較しない** | 空行の丸めで個数が変わるのは整形の目的そのもの |
| `Indent` / `Dedent` | **種別のみ比較** | 幅（`content`）は `--indent` で変わってよいが、**出現と順序は変わってはならない**。当初案のように丸ごと除外すると「ブロックが消えた」バグを検出できない |
| その他 | `(kind, raw_text())` | 下記 |

**`content` ではなく `raw_text()` を比較する。** これはテストが実際に検出した穴で、
`"a\tb"` と `"a    b"` は `content` が同一（どちらも復号後は空白4個）になる。
`content` で比較すると**フォーマッタが `"a\tb"` を `"a    b"` に書き換えても検証をすり抜ける**。
`raw` を導入した目的そのものを取りこぼすところだった。

実装は `crates/erg_fmt/verify.rs`。`compare()` は最初に食い違ったトークン対を
`Mismatch` として返すので、`debug_assert!` のメッセージがそのままデバッグの手がかりになる。

---

## 7. Phase 3: CLI 配線

```rust
// erg_common/config.rs
pub enum ErgMode {
    // ...
    Fmt,
}

// TryFrom<&str>
"fmt" | "format" | "formatter" => Ok(Self::Fmt),
// From<ErgMode> for &str
ErgMode::Fmt => "fmt",
```

```rust
// src/main.rs
Fmt => Formatter::run(cfg),
```

`Formatter` は `erg_linter::Linter` ([lib.rs:41-94](../crates/erg_linter/lib.rs#L41-L94)) と同じ形で
`New` + `Runnable` を実装する。

| メソッド | 実装 |
| -------- | ---- |
| `NAME` | `"Erg formatter"` |
| `exec()` | 入力を読み `format_str`。`--check` なら差分判定のみ、`--stdout` なら出力、既定はファイル書き戻し |
| `eval(src)` | 整形結果の文字列を返す（REPL から使える） |
| `initialize` / `clear` / `finish` | 状態を持たないので空実装 |
| `completeness_checker` | `erg_parser::parse::check_code_completeness` を返す |

`ErgConfig` に追加するフィールド:

| フィールド | 対応フラグ |
| ---------- | ---------- |
| `fmt_check: bool` | `--check` |
| `fmt_stdout: bool` | `--stdout` |
| `fmt_indent: usize` | `--indent`（既定 4） |
| `fmt_max_blank_lines: usize` | `--max-blank-lines`（既定 2） |
| `fmt_max_width: usize` | `--max-width`（既定 100） |
| `fmt_exclude: Vec<String>` | `--exclude`（複数指定可） |

`erg fmt` に加えて `erg --mode fmt <file>` でも動く（既存モードと同じ扱い）。

ディレクトリを渡された場合は `**/*.er` を再帰的に整形する。
`--exclude` の glob に一致するファイルは読み込まずにスキップする。

`--check` の終了コードは **差分の有無のみ**で決める。
折り返せなかった超過行（括弧を含まない行、§5.7 手順5）と検証失敗 (§6) は
stderr に報告するが、終了コードには影響させない。
どちらも `erg fmt` が直せない類の問題であり、CI を止めるかどうかは利用者の判断に委ねるため。

---

## 8. Phase 4: ELS 連携

ELS は「LSP 機能1つにつきワーカースレッド1本 + mpsc チャネル1本」という構造
（[channels.rs](../crates/els/channels.rs)、`SendChannels` / `ReceiveChannels` / `WorkerMessage<P>`）。
既存24ワーカーと同じパターンで追加する。

1. `channels.rs`: `SendChannels` / `ReceiveChannels` に
   `formatting: mpsc::Sender<WorkerMessage<DocumentFormattingParams>>` を追加
2. `server.rs` の `init_capabilities()`（[server.rs:504](../crates/els/server.rs#L504)）に
   `capabilities.document_formatting_provider = Some(OneOf::Left(true));`
3. `crates/els/formatting.rs` を新設し、`Formatting` リクエストのハンドラを実装。
   ファイル全体を1つの `TextEdit` で置き換える（範囲指定整形は非対応）
4. ソースは `FileCache` から取得する

**型検査を経ないので高速**であり、保存時整形に十分な応答性が出る。
`erg_fmt` が `erg_compiler` に依存しないことがここで効く。

---

## 9. テスト戦略

| 層 | 内容 |
| -- | ---- |
| Phase 1 | `keep_comments` ありで lex し、`raw_text()` を連結すると原文に戻ることを確認（§4.4）。既存の `tokenize_test.rs` が無改変で通ること |
| ゴールデンテスト | `tests/fixtures/*.er` → `*.expected.er`。`examples/` の既存 `.er` を素材にする |
| 冪等性 | 全フィクスチャで `fmt(fmt(x)) == fmt(x)` |
| 非破壊性 | `examples/` と `tests/should_ok/` の**全 `.er` ファイル**について、整形前後でトークン列が一致すること。これが最重要のリグレッションテスト |
| 実行同値性 | 整形後の `examples/*.er` が整形前と同じ出力を出すこと（数本のスモークテストで足りる） |
| エスケープ | `"a\nb"` `"\x41"` `"a\tb"` が**一字一句変わらない**こと（§2.5 の回帰テスト） |
| fixity | `x +1` が `x + 1` にならないこと、`x + 1` が `x +1` にならないこと（§2.6 の回帰テスト） |
| opt-out | `# fmt: off`〜`# fmt: on` の範囲が1バイトも変わらないこと。`# fmt: skip` がその行だけに効くこと。`#fmt:off` は効かないこと（§5.6） |
| 設定 | `--indent 2` / `--max-blank-lines 1` / `--max-width 40` が効くこと。既定値でのゴールデンテストと別に、各フラグ1本ずつ |
| reflow | 折り返し後もトークン列が一致すること（§5.7.1 規則1）。trailing comma を足さないこと（規則3）。既存の trailing comma があれば幅に収まっても展開を維持すること（§5.7.3）。括弧を含まない長い行を割らないこと |

`erg_linter/tests/lint.rs` と同じく `exec_new_thread` で包む。

---

## 10. リスクと未決事項

| # | リスク | 対策 / 状態 |
| - | ------ | ----------- |
| R1 | `Comment` トークンが `prev_token` を汚染し `op_fix_of` の判定を変える | §4.1 の通り `prev_token` を更新しない。Phase 1 のテストで固定 |
| R2 | 文字列リテラルのエスケープ破壊 | §4.3 の `raw` フィールドで解決。専用回帰テスト |
| R3 | `f(x)` ↔ `f (x)` の差を検証で検出できない | §5.3 の通り開き括弧直前の空白は原文保存。**保守的に倒す** |
| R4 | 複数行文字列リテラルの `lineno` が壊れている | **【解消済み】** 既存バグだった。§10.1 を参照。`\n` エスケープを含む `"""..."""` が `lineno = 0`（＝`Location::Unknown`）になっていた。Step 2 の直後に修正済み |
| R5 | 文字列補間 `\{...}` 内のトークンの扱い | `interpol_stack` により入れ子になる。整形対象にするか素通しにするか **【未決 → 初版は素通し】** |
| R6 | `#[ ... ]#` の再インデント | 複数行コメントの中身をどう扱うか。**初版は原文のまま出力し、一切触らない** |
| R7 | ELS のセマンティックトークンが `Comment` を期待しはじめる | 現状 ELS は自前でコメント範囲を計算している可能性がある。Phase 4 で `keep_comments` に統一できるか検討（本設計の必須要件ではない） |
| R8 | 折り返しで行内コメントの位置が壊れる | 括弧内にコメントがある行を割ると、コメントがどの要素に付くか曖昧になる。**初版は「括弧内にコメントトークンを含む論理行は折り返さない」** とする。安全側に倒す |
| R9 | 折り返しが冪等でない | 割った結果がさらに `max_width` を超えて再度割られる、という振動。手順4の再帰は「割れる括弧がなくなるまで」であって幅に依存しないので原理的には起きないが、§9 の二重適用チェックで担保する |

---

### 10.1 R4 の詳細: 複数行文字列リテラルの `lineno` が 0 になる

Step 2 の作業中に実測で確認した、**フォーマッタとは独立に存在する既存バグ**。

`emit_multiline_token` は行番号を「現在行 − 内容の行数」で逆算する:

```rust
let lineno = (self.lineno_token_starts + 2).saturating_sub(cont.lines().count() as u32);
```

ところが `cont` は**エスケープ復号後**の内容なので、`\n` エスケープが実際の改行に化けて
行数が水増しされる。`lineno_token_starts` は実際の改行でしか増えないため、両者がずれる。

| ソース | `lineno_token_starts` | `cont.lines()` | 算出 `lineno` | 正解 |
| ------ | --------------------- | -------------- | ------------- | ---- |
| `"""ab"""` | 0 | 1 | 1 | 1 ✅ |
| `"""a\nb"""`（エスケープ） | 0 | 2 | **0** | 1 ❌ |
| `"""a` 改行 `b"""`（実改行） | 1 | 2 | 1 | 1 ✅ |

`lineno == 0` は `Locational::loc()` が `Location::Unknown` を返す番兵値
（[token.rs](../crates/erg_parser/token.rs) の `impl Locational for Token`）。
つまりこのリテラルは**位置情報を完全に失い**、エラーメッセージがその文字列を指せなくなる。
フォーマッタ以前に、コンパイラのエラー表示の問題である。

**修正【適用済み】**: 行数を復号後の `cont` ではなく**原文**から数えるようにした。
Step 2 で入れた `raw_since_token_start()` がそのデータそのものだったので、
`emit_raw_multiline_token`（原文から高さを取り、ついでに `raw` も載せる）を追加し、
`lex_multi_line_str` の成功パス2箇所をそれに差し替えた。

エラーパスの `emit_multiline_token` は**あえて据え置き**にした。あちらは
`&format!("\\{next_c}")` のような断片を渡すので、原文全体の高さで逆算すると
エラー位置がかえって手前にずれる。不正エスケープは「いま読んでいる行」を指すべき。

回帰テストは `tests/raw_text_test.rs::multi_line_string_lineno_counts_source_lines`。

---

## 11. 実装順序

各ステップ末尾で `cargo test --features large_thread` と
`cargo clippy --all --all-targets -- -D warnings` を通し、機能単位でコミットする。

1. ~~**`Token::raw` の追加**~~ ✅ — 実測の結果 `Option<Box<Str>>` を採用（§4.3）
2. ~~**文字列レキサの `raw` 対応**~~ ✅ — 分岐ごとの並行構築ではなく、
   `token_start_cursor` からのソーススライスで実装
3. ~~**`TokenKind::Comment` + `keep_comments`**~~ ✅ — R1 は `emit_comment` の
   `prev_token` 復元で回避。`category()` の `_ => BinOp` フォールバックにも注意が必要だった

   （ここまでで Phase 1 完了。併せて既存バグ §10.1 を修正）
4. ~~**`erg_fmt` クレートの骨格**~~ ✅ — `render` は恒等関数のまま、検証部 (§6) を先に実装。
   比較対象は実装時に §6.1 のとおり修正した
5. **`Emit::Verbatim` と `# fmt: off` / `on` / `skip`** — §5.6。検証フォールバックと同じ経路なので、
   ステップ4の直後にここを作ると両方まとめてテストできる
6. **論理行分割と描画** — インデント・空行の正規化まで。ここでゴールデンテストを開始
7. **空白規則** — §5.3 の表を1行ずつ実装。各行に対応するテストを書く
8. **括弧内の継続行** — §2.4 の `lineno` 差分処理
9. **折り返し** — §5.7。ステップ8で括弧内の複数行描画ができてからでないと作れない
10. **CLI 配線** — `ErgMode::Fmt`、`Formatter`、§1.2 の3フラグと `--exclude`
11. **ELS 連携** — `textDocument/formatting`
12. **`examples/` 全体での非破壊性テスト** — 最終関門

ステップ4〜5で「整形をあきらめて原文を出す」経路を先に作るのが要点。
以降の全ステップが「壊れたら原文に戻る」という安全網の下で進むので、空白規則を大胆に書ける。

---

## 12. 補記: なぜ設定ファイルを持たないか

rustfmt の設定項目は**82個あり、そのうち stable で使えるのは28個**
（rustfmt 1.9.0-stable で実測）。残る54個は永久に nightly 専用のまま置かれている。
rustfmt チーム自身がオプションを増やしすぎたと繰り返し表明しており、
**このリポジトリも `rustfmt.toml` を持たず、82個を1つも使っていない。**

gofmt はオプションを持たない。black は実質2個（`line-length`, `skip-string-normalization`）。
後発ほど少ない、というのが一貫した傾向である。

Erg はまだ利用者が少なく、**いま単一スタイルに倒しておけばエコシステムの分裂を防げる**。
一方で「行長をいくつにするか」「どこで整形を止めるか」は必ず議論になるので、
その2つだけ §1.2 / §5.6 で先に決めてしまう。これが CLI フラグ3つに絞った理由。

将来スタイルのデフォルト自体を変えたくなった場合は、rustfmt の `style_edition` に相当する
仕組み（既存コードを一斉に書き換えずに移行する手段）が必要になる。
**初版では作らない**が、`FmtOptions` を構造体にしておくのは将来そこに
`style_edition` フィールドを足せるようにするため。
