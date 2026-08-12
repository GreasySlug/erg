# `erg fmt` 設計書

Erg 向けコードフォーマッタ (`cargo fmt` 相当) の設計。

> 一言まとめ: **Lexer にコメントを emit させ、トークン列から空白・インデント・空行だけを正規化する。
> Erg は空白が意味論的に有意な言語なので、整形結果を再 lex して元とトークン列が一致することを検証し、
> 一致しなければ整形を破棄する「自己検証型フォーマッタ」にする。**

- **方式**: トークンストリーム整形（AST/CST は使わない）
- **設定**: なし（gofmt 方式、単一スタイルに固定）
- **スコープ**: フォーマッタのみ。linter (`erg_linter`) の拡張は本書の対象外

---

## 1. 目的と非目標

### 目的

| 項目 | 内容 |
| ---- | ---- |
| `erg fmt <file>` | ファイルをその場で整形 |
| `erg fmt --check` | 差分があれば非ゼロ終了（CI 用）。書き換えない |
| `erg fmt --stdout` | 整形結果を標準出力へ |
| ELS `textDocument/formatting` | エディタの保存時整形 |

### 非目標

- **行の折り返し（reflow）を行わない。** 既存の改行位置は尊重する。長い式を自動で複数行に割る／逆に1行にまとめることはしない
- **設定ファイルを持たない。** インデント幅も含め一切の設定項目を作らない
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

それでも見落としは起こりうるので、§6.5 の検証を必須とする。

---

## 3. 全体アーキテクチャ

```text
                     ┌──────────────── Phase 1 ────────────────┐
source ──> Lexer (keep_comments = true) ──> Vec<Token> (raw 付き)
                                                  │
                     ┌──────────────── Phase 2 ───┼─────────────┐
                     │                            v             │
                     │   1. 論理行への分割    LogicalLine 列     │
                     │   2. 物理行の復元 (lineno 差分)           │
                     │   3. 空白正規化 (kind ペア規則)           │
                     │   4. インデント/空行の正規化              │
                     │                            │             │
                     │                            v             │
                     │                        String            │
                     │                            │             │
                     │   5. 検証: 再 lex してトークン列比較 ─── NG ──> 元のソースを返す
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
    pub raw: Option<Str>,
    pub lineno: u32,
    pub col_begin: u32,
    pub col_end: u32,
}

impl Token {
    /// 整形出力に使うべきテキスト
    pub fn raw_text(&self) -> &str {
        self.raw.as_deref().unwrap_or(&self.content)
    }
}
```

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
- 構築箇所は**約18箇所・5ファイル**（`token.rs` 8、[desugar.rs](../crates/erg_parser/desugar.rs) 6、
  [ast.rs](../crates/erg_parser/ast.rs) 2、[parse.rs](../crates/erg_parser/parse.rs) 1、
  [codegen.rs](../crates/erg_compiler/codegen.rs) 1）。`Token::new` を経由せず構造体リテラルで
  直接組み立てている箇所が `token.rs` の外に10箇所あるので、そこにも `raw: None` を足す必要がある。
  ただし構造体リテラルは網羅性が要求されるため**コンパイラが漏れなく列挙してくれる**。純粋に機械的な作業

> **代替案**: `Token` を触らず、`lex_for_fmt()` が
> `(TokenStream, Vec<Option<Str>>)` を返す並行配列方式。侵襲は小さいが
> 添字ずれのバグを生みやすく、ELS からの再利用もしにくい。**採用しない。**

### 4.4 Phase 1 の完了条件

- `cargo test --features large_thread` が全て通る（既存挙動が不変であることの確認）
- `Lexer::from_str(src).keep_comments().lex()` が、コメントを含むソースについて
  `raw_text()` を連結すると**元のソースに完全一致する**こと（トークン間の空白を除く）

この「連結すると原文に戻る」性質が Phase 2 全体の土台になる。ここを Phase 1 のテストで固定する。

---

## 5. Phase 2: `erg_fmt` クレート

```text
crates/erg_fmt/
├── Cargo.toml
├── lib.rs        Formatter (Runnable 実装), format_str()
├── line.rs       LogicalLine への分割
├── spacing.rs    トークン間空白の規則
├── render.rs     文字列生成
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
2. **物理行の復元**: 各 `LogicalLine` 内で `tokens[i].lineno != tokens[i-1].lineno` の位置に改行を入れる
   （§2.4）。継続行のインデントは `depth + 1` 相当に正規化する
3. **空白正規化** (`spacing.rs`): 同一物理行内の隣接トークン対に `space_between` を適用
4. **描画** (`render.rs`): `depth * 4` の空白 + トークン列 + 行末コメント。空行は上限2に丸める
5. **検証** (`verify.rs`): §6.5

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

- **開き括弧の直前**: `f(x)` と `f (x)` は同じトークン列になるため §6.5 の検証で差を検出できない。
  かつ Erg では `f (1, 2)`（タプル1個を渡す）と `f(1, 2)`（引数2個）で意味が変わりうる。
  → **`LParen`/`LSqBr`/`LBrace` の直前の空白の有無は原文どおりにする**
- **行頭の `Dot`**: `.x = 1` は公開属性の定義であって属性アクセスではない。
  `Dot` が物理行の先頭にある場合は「前後0個」規則を適用せず、後続に空白を入れない
- **文字列補間**: `StrInterpLeft` / `Mid` / `Right` の内側は
  1つの式として扱いつつ、`\{` `}` の直前直後には空白を入れない

### 5.4 インデントと空行

| 項目 | 規則 |
| ---- | ---- |
| インデント幅 | ネスト1段あたり空白4個（固定・設定不可） |
| タブ | レキサが `Illegal` にする（[lex.rs:1492](../crates/erg_parser/lex.rs#L1492)）ので考慮不要 |
| 連続する空行 | 最大2行に丸める |
| ファイル先頭の空行 | 削除 |
| ファイル末尾 | 改行1個で終端。末尾の空行は削除 |
| 行末の空白 | 削除 |
| 括弧内の継続行 | `depth + 1` 段でインデント |

### 5.5 冪等性

`format(format(src)) == format(src)` を不変条件とする。
`blank_lines_before` の丸めや継続行のインデントで違反しやすいので、
全てのテストフィクスチャに対して自動的に二重適用チェックを回す（§7）。

---

## 6. 検証（自己検証型フォーマッタ）

**本設計の中核。** Erg は空白が意味論的に有意（§2.6）なので、整形は必ず検証する。

```rust
pub fn format_str(src: &str) -> String {
    let Ok(before) = Lexer::from_str(src.into()).keep_comments().lex() else {
        return src.to_string();   // lex できないなら何もしない
    };
    let formatted = render(&before);
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

`token_streams_equivalent` は `Newline` / `Indent` / `Dedent` を除外したうえで
`(kind, content)` の列を比較する。`Token` の `PartialEq` が既に
`kind` と `content` のみを見る実装（[token.rs:379-384](../crates/erg_parser/token.rs#L379-L384)）なので、
フィルタしてから `Iterator::eq` で足りる。

**この設計の価値**: 空白規則にバグがあっても、**ユーザーのコードが壊れることは絶対にない**。
最悪「整形されない」で済む。`debug_assert!` により、デバッグビルドとテストでは
バグが即座に顕在化する。

`--check` モードでは検証失敗時に「整形不能」として警告を出し、差分ありとは扱わない。

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

CLI フラグは `ErgConfig` に `fmt_check: bool` / `fmt_stdout: bool` を追加。
`erg fmt` に加えて `erg --mode fmt <file>` でも動く（既存モードと同じ扱い）。

ディレクトリを渡された場合は `**/*.er` を再帰的に整形する。

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

`erg_linter/tests/lint.rs` と同じく `exec_new_thread` で包む。

---

## 10. リスクと未決事項

| # | リスク | 対策 / 状態 |
| - | ------ | ----------- |
| R1 | `Comment` トークンが `prev_token` を汚染し `op_fix_of` の判定を変える | §4.1 の通り `prev_token` を更新しない。Phase 1 のテストで固定 |
| R2 | 文字列リテラルのエスケープ破壊 | §4.3 の `raw` フィールドで解決。専用回帰テスト |
| R3 | `f(x)` ↔ `f (x)` の差を検証で検出できない | §5.3 の通り開き括弧直前の空白は原文保存。**保守的に倒す** |
| R4 | 括弧内の `lineno` が信用できないケースがある | 複数行文字列リテラルを含む行が要注意（`emit_multiline_token` は `lines().count()` から逆算する）。Phase 2 着手時に実挙動を確認する **【未検証】** |
| R5 | 文字列補間 `\{...}` 内のトークンの扱い | `interpol_stack` により入れ子になる。整形対象にするか素通しにするか **【未決 → 初版は素通し】** |
| R6 | `#[ ... ]#` の再インデント | 複数行コメントの中身をどう扱うか。**初版は原文のまま出力し、一切触らない** |
| R7 | ELS のセマンティックトークンが `Comment` を期待しはじめる | 現状 ELS は自前でコメント範囲を計算している可能性がある。Phase 4 で `keep_comments` に統一できるか検討（本設計の必須要件ではない） |

---

## 11. 実装順序

各ステップ末尾で `cargo test --features large_thread` と
`cargo clippy --all --all-targets -- -D warnings` を通し、機能単位でコミットする。

1. **`Token::raw` の追加** — フィールド追加と全構築箇所の更新のみ。挙動は変わらない
2. **文字列レキサの `raw` 対応** — `lex_single_str` 等で原文を並行構築。§4.4 の連結テストを追加
3. **`TokenKind::Comment` + `keep_comments`** — R1 に注意。トークン化テストを追加
4. **`erg_fmt` クレートの骨格** — `format_str` が「何もせず原文を返す」状態で、検証部 (§6) だけ先に実装
5. **論理行分割と描画** — インデント・空行の正規化まで。ここでゴールデンテストを開始
6. **空白規則** — §5.3 の表を1行ずつ実装。各行に対応するテストを書く
7. **括弧内の継続行** — §2.4 の `lineno` 差分処理
8. **CLI 配線** — `ErgMode::Fmt` と `Formatter`
9. **ELS 連携** — `textDocument/formatting`
10. **`examples/` 全体での非破壊性テスト** — 最終関門

ステップ4で検証部を先に作るのが要点。以降の全ステップが「壊れたら原文に戻る」という
安全網の下で進むので、空白規則を大胆に書ける。
