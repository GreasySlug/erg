# `erg_parser` クレット アーキテクチャ

`crates/erg_parser/` は Erg コンパイラのフロントエンド（字句解析 → 構文解析 → 脱糖）を担う。
ソース文字列を受け取り、型情報を持たない `AST`（`Vec<Expr>`）を生成するところまでが責務。
下流の `erg_compiler` がこの `AST` を HIR に lowering する。

> 一言まとめ: **Lexer（Iterator）→ precedence-climbing Parser → Desugarer（6パス）** という3段構成。
> パーサーは「まず汎用的に `Expr` として読み、後から `convert.rs` / `typespec.rs` で意味づけし直す」設計。

---

## 1. ファイル構成

| ファイル | 行数 | 役割 |
| -------- | ---: | ---- |
| `lib.rs` | 112 | クレートのエントリ。モジュール宣言と PyO3 バインディング（`pylib`） |
| `main.rs` | 30 | スタンドアロン実行バイナリ（`Lex`/`Parse`/`Desugar` モード） |
| `token.rs` | 569 | `TokenKind` / `TokenCategory` / `Token` / `TokenStream`、**演算子優先順位表** |
| `lex.rs` | 1682 | `Lexer`（`Iterator<Item = Result<Token, _>>`）と `LexerRunner` |
| `ast.rs` | 7448 | `Expr` を頂点とする全 AST ノード定義 + 表示/位置/変換マクロ |
| `parse.rs` | 4174 | `Parser` / `ParserRunner` / `SimpleParser`、precedence-climbing エンジン |
| `convert.rs` | 924 | `impl Parser`。`Expr`/`Args` → シグネチャ・パラメータへの再解釈 |
| `typespec.rs` | 596 | `impl Parser`。`Expr` → `TypeSpec`（型注釈位置の意味解釈） |
| `desugar.rs` | 1955 | `Desugarer`。AST → AST の脱糖パス群 |
| `build_ast.rs` | 134 | `ASTBuilder`：パース + 脱糖をまとめる `Runnable` |
| `visitor.rs` | 65 | `ASTVisitor`：完成した AST の読み取り専用走査ヘルパ |

データの流れ:

```text
String
  │  Lexer (lex.rs)           ── Iterator<Token>
  ▼
TokenStream (token.rs)
  │  Parser::parse (parse.rs) ── precedence climbing + convert.rs/typespec.rs
  ▼
AST (型情報なし, ast.rs)
  │  Desugarer::desugar (desugar.rs) ── 6 passes
  ▼
AST (脱糖済み)  →  erg_compiler へ
```

`ASTBuilder::build`（`build_ast.rs`）がパースと脱糖を連結するトップレベル API。
パース失敗時も `IncompleteArtifact` を脱糖して返す（エラー回復のため）。

---

## 2. 字句解析 (`lex.rs` + `token.rs`)

### `Token` / `TokenKind`（token.rs）

- `TokenKind`（`#[repr(u8)]`、~110 種）: `Symbol` / 各種リテラル（`NatLit`/`IntLit`/`RatioLit`/`StrLit`…）/ 演算子 / 囲み記号 / `Indent`・`Dedent` など。
- `TokenCategory`: `TokenKind` を粗くグループ化（`BinOp`/`UnaryOp`/`SpecialBinOp`/`DefOp`/`LambdaOp` 等）。パーサーの分岐に使う。
- `Token { kind, content: Str, lineno, col_begin, col_end }`。`content` は `Str`（参照カウント文字列）で、`CacheSet` により重複排除されている。
- **演算子優先順位は `TokenKind::precedence() -> Option<usize>`（token.rs:273）に一元化**。precedence-climbing エンジンが唯一の参照元とする。

優先順位表（抜粋、高いほど強い）:

| prec | 演算子 |
| ---: | ------ |
| 200 | `.` `::` |
| 190 | `**` |
| 180 | 単項 `+ - ~ ref ref!` |
| 170 | `* / // %` |
| 160 | `+ -` |
| 150 | `<< >>` |
| 140/130/120 | `&&` / `^^` / `\|\|` |
| 100 | 範囲演算子 `..` `..<` `<..` `<..<` |
| 90 | 比較 `< > <= >= == != in notin contains is isnot` |
| 80/70 | `and` / `or` |
| 60 | `-> => <-` |
| 50 | `: :> <: as` |
| 40 | `,` |
| 20 | `= :=` |
| 10 | `\n` `;` |
| 0 | `( { [ Indent` |

### `Lexer`（lex.rs）

`Iterator` 実装で1トークンずつ遅延生成する。主な状態:

- `chars: Vec<char>` … **正規化済みソース全体を `char` 配列で保持**（`cursor` で添字アクセス）
- `indent_stack: Vec<usize>` … インデント幅スタック（INDENT/DEDENT 生成）
- `enclosure_level: usize` … `() [] {}` の深さ（囲み内では改行を抑制）
- `interpol_stack: Vec<Interpolation>` … 文字列補間のネスト状態
- `prev_token: Token` … 直前トークン（`+`/`-`/`*` の前置/中置判定 `op_fix()` に使用）

`next()` の処理順: EOF 判定 → `lex_space_indent_dedent()`（インデント/改行）→ コメントスキップ → 1文字 `consume()` して種別ごとに分岐（囲み記号 / 演算子 / 文字列 / 数値 `lex_num()` / 記号 `lex_symbol()`）。

- **数値**: `lex_num()` が整数/`0b`/`0o`/`0x`/小数(`lex_ratio`)/指数(`lex_exponent`) を判定。`1..`（範囲）や `1.foo`（メソッド）と小数の曖昧性は `lex_num_dot()` で解決。`f64`/`f32` サフィックスは字句段階では別記号として残し、**パーサー側で隣接判定して脱糖**する（[float-suffix] 参照）。
- **文字列補間** `"...\{ expr }..."`: `\{` で `StrInterpLeft` を出し `interpol_stack` に push。以後は通常の Erg コードを字句解析し、`}` を `lex_interpolation_mid()` で受けて `StrInterpMid` / `StrInterpRight` を出す。ネスト可。
- **セキュリティ**: 文字列/コメント中の双方向制御文字（CVE-2021-42574）を `is_bidi()` で検出してエラー化。
- インデント上限は 100 スペース（CPython 準拠）。

`LexerRunner` は `Runnable` 実装の使い捨てラッパで CLI/REPL から使う。

---

## 3. 構文解析 (`parse.rs` + `convert.rs` + `typespec.rs`)

### 主要型

- **`Parser`**（parse.rs:360）: `counter: DefId`（定義ID採番）/ `level`（デバッグ用再帰深度）/ `tokens: TokenStream` / `warns` / `errs` を保持する再帰下降パーサー。
- **`ParserRunner`**: `ErgConfig` を持ち `Runnable` を実装する高レベルドライバ。`parse(src)` がエントリ。
- **`SimpleParser`**: 字句解析〜パースを一括で行うステートレスな便宜ラッパ（`Parsable` 実装）。
- **`ExprCtx`**（parse.rs:268）: 式パースの文脈フラグ。`chunk`（文レベル＝定義/`expr args` 呼び出しを許可）/ `winding`（括弧なしタプル）/ `in_type_args`（`|...|` 内）/ `in_brace`（`{}` 内で `:` を key-value 区切り扱い）/ `line_break`（括弧内で複数行可）。

### precedence-climbing エンジン

式解析は **`try_reduce_expr_prec(min_prec, ctx)`**（parse.rs:2168）に集約された左結合 precedence climbing。
（旧 `ExprOrOp` shift-reduce 2本立てを置換したもの。[parser-engine] 参照）

1. `try_reduce_bin_lhs()` で最小単位の LHS（リテラル/呼び出し/単項/ラムダ/括弧）を読む。
2. ループで現在トークンを見て:
   - **後置構文**（任意の prec で直近オペランドに結合）: `.`/`::` アクセサ連鎖、`[...]` 添字、`,` タプル（`winding` 時）、`:=` デフォルト引数。
   - **二項演算子**: `op_prec < min_prec` なら停止（外側に委ねる）。そうでなければ RHS を `try_reduce_expr_prec(op_prec + 1, ctx)` で読む（左結合）。
   - **`min_prec == 0` 限定の特殊演算子**: `=`（定義）、`->`/`=>`（ラムダ）、`:`/`<:`/`:>`/`as`（型注釈）、`|>`（パイプ）、`expr args`（文レベル呼び出し）。
3. 予約トークン・EOF・文脈不一致で停止。

`try_reduce_chunk`（parse.rs:1936）/ `try_reduce_expr`（parse.rs:2144）は `ExprCtx` を組み立てる薄いラッパ。

### 「後から再解釈」する設計（convert.rs / typespec.rs）

パーサーは関数定義もタプルもまず汎用 `Expr` として読み、確定後に専用形へ変換する:

- **`convert.rs`** … `Expr`/`Args` → シグネチャ・パラメータ:
  - `convert_rhs_to_sig`（Accessor→`VarSignature`, Call→`SubrSignature`, List/Tuple/Record/DataPack→パターン）
  - `convert_args_to_params` / `convert_rhs_to_param`（`Args` → `Params`）
  - `convert_rhs_to_lambda_sig`（ラムダ引数の特殊扱い）
  - `convert_type_args_to_bounds`（`|...|` → 型境界）
  - 変換不能な式に当たると `ParseError::simple_syntax_error` を `self.errs` に積み `Err(())` を返す。
- **`typespec.rs`** … 型注釈位置の `Expr` → `TypeSpec`:
  - `expr_to_type_spec`（種別で分岐するエントリ）
  - `accessor_to_type_spec` / `call_to_predecl_type_spec` / `lambda_to_subr_type_spec` / `list/dict/set/record/tuple_to_*_type_spec`
  - `BinOp` の範囲演算子→区間型、`Or`/`And`→合併/交差型、`Set::Comprehension`（単一ジェネレータ+ガード）→リファインメント型。
  - `validate_const_expr`：コンパイル時評価可能性を検査して `ConstExpr` 化。

### エラー回復・補完判定

- エラーは致命的でなく `errs` に蓄積して継続（最大限のエラー報告）。`next_expr()` / `next_line()` / `until_dedent()` でスキップ。
- バックトラックは持たず、消費済みトークンは戻さない（誤りは `restore(token)` で1個押し戻す程度）。`counter`(DefId) は採番のみでロールバック不要。
- **REPL 補完**: `check_code_completeness(src)`（parse.rs:133）。まず `has_open_delimiters` で括弧/文字列/コメントの開きを軽量走査し、必要なら `SimpleParser::parse` を試して `ExpectNextLine` を継続入力と判定。

---

## 4. AST (`ast.rs`)

`Expr`（ast.rs:6925）が頂点の列挙型（22 バリアント）。`#![allow(clippy::large_enum_variant)]` 下で重い内部は `Box` 化されている。

| 区分 | バリアント |
| ---- | ---------- |
| 値/参照 | `Literal` / `Accessor`（Ident/Attr/TupleAttr/Subscr/TypeApp） |
| 演算 | `BinOp`（`[Box<Expr>; 2]`）/ `UnaryOp` / `Call`（`obj: Box<Expr>`, `attr_name`, `args`） |
| コンテナ | `List` / `Tuple` / `Set` / `Dict`（各 Normal / Comprehension / WithLength）/ `Record`（Normal/Mixed）/ `DataPack` |
| 関数 | `Lambda`（`sig`, `op`, `body: Block`, `id`） |
| 定義 | `Def`（`sig: Signature`, `body: DefBody`）/ `Methods` / `ClassDef` / `PatchDef` / `ReDef` |
| 型 | `TypeAscription` |
| その他 | `Compound` / `InlineModule` / `Dummy`（プレースホルダ） |

補助グループ:

- **シグネチャ**: `Signature`（Var/Subr）、`VarSignature{ pat: VarPattern, t_spec, bounds }`、`SubrSignature{ ident, params, bounds, decorators, return_t_spec }`。
- **パラメータ**: `Params{ non_defaults, var_params, defaults, kw_var_params, guards, parens }`、`NonDefaultParamSignature`、`DefaultParamSignature`。
- **型注釈**: `TypeSpecWithOp{ op, t_spec: TypeSpec, t_spec_as_expr: Box<Expr> }` … **意味形（TypeSpec）と構文形（Expr）の両方を保持**するのが特徴（脱糖や再解釈のため）。
- トップレベル: `Module`（`Block` ラッパ）と `AST{ name: Str, module: Module }`。

表示・位置・変換は宣言的マクロで生成: `impl_display_from_nested!` / `impl_locational!(_for_enum)` / `impl_nested_display_for(_chunk)_enum!` / `impl_stream!` / `impl_traversable_for_enum!` / `impl_(into|from)_py_for_enum!`（PyO3）。
`visitor.rs` の `ASTVisitor` は完成 AST に対する読み取り専用走査（変数値検索・クラス列挙）を提供。

---

## 5. 脱糖 (`desugar.rs`)

`Desugarer{ var_gen: FreshNameGenerator }`。`var_gen` は `%desugar_N` のような新鮮名を採番する。
`desugar(module)` が以下6パスを**この順で**適用する（順序に依存関係あり）:

| # | パス | 変換例 |
| - | ---- | ------ |
| 1 | `desugar_multiple_pattern_def` | `f 0 = ..; f 1 = ..` → `f n = match n, (0 -> ..), (1 -> ..)` |
| 2 | `desugar_pattern_in_module` | `[i, j] = x` → `i = x[0]; j = x[1]`（分配束縛、ガード生成） |
| 3 | `desugar_shortened_record` | `{x; y}` → `{x = x; y = y}` |
| 4 | `desugar_acc` | `x[y]`→`x.__getitem__(y)`、`` x.`+` y ``→`x.__add__(y)` |
| 5 | `desugar_operator` | `l in r` → `r contains l` |
| 6 | `desugar_comprehension` | `[y \| x <- xs]` → `list(map(x -> y, xs))` |

中核は汎用の構造的再帰 **`perform_desugar(closure, expr)`**（desugar.rs:147）。
各パスは `desugar_all_chunks(module, |e| rec_desugar_X(e))` の形で木全体を走査し、未対応ノードは `perform_desugar` に委ねる。
`perform_desugar` はノードを**ムーブで分解・再構築**する（クローンしない）方針。

補助: `gen_match_call` / `gen_buf_name_and_sig`（分配用バッファ）/ `len_guard`・`hasattr_guard`・`type_guard`（パターンガード生成）/ `desugar_generators`・`desugar_concat_lists`（内包表記）。

---

## 6. 効率性: 適用済みの修正と今後の候補

### 適用済み（本変更）

**`desugar.rs` の型注釈式クローン除去（3箇所: Def/ReDef/Lambda の各アーム）**

`perform_desugar` はムーブ前提の走査だが、型注釈を持つ定義だけ以下の例外があった:

```rust
// before — Box<Expr> を deep-clone してから上書き（クローンは純粋な無駄）
*t_op.t_spec_as_expr = desugar(*t_op.t_spec_as_expr.clone());

// after — 元を mem::replace でムーブして脱糖し書き戻す（追加アロケーションなし）
let t_spec_as_expr = std::mem::replace(
    t_op.t_spec_as_expr.as_mut(),
    Expr::Dummy(Dummy::new(None, Vec::new())),
);
*t_op.t_spec_as_expr = desugar(t_spec_as_expr);
```

このアームは複数パスから同じ型注釈付き定義に対して実行されるため、型注釈式（任意に複雑になりうる）の deep-clone が
パスごとに繰り返されていた。修正により走査が完全にムーブベースになる。挙動は不変（脱糖対象の式は同一、元は直後に破棄される）。
parser テスト 37件 / clippy `-D warnings` グリーンで確認済み。

### 今後の候補（影響大／リスクありで本変更では見送り）

| 箇所 | 内容 | 見送り理由 |
| ---- | ---- | ---------- |
| `lex.rs:193,209` | ソース全体を `Vec<char>` で保持 | `&str` バイト添字化は `cursor`/列計算（char 単位）全面改修になり高リスク。効果は大 |
| `desugar.rs` 全体 | 6パスがそれぞれ木を全走査（≈6×O(n) の再構築） | パス統合は順序依存があり脱糖の正しさに直結。要慎重設計 |
| `ast.rs` の getter | `Call::get_args`/`BinOp::lhs/rhs`/`Attribute::obj` 等が clone を返す | 大半は PyO3 FFI 用（`#[to_owned]` 生成）。Rust 側ホットパスは HIR 側のため影響限定 |
| `convert.rs:858` ほか | `expr_to_type_spec(call.clone())` 等、変換前に clone | `TypeSpecWithOp` が TypeSpec と元 Expr の両方を保持する設計上の必然。借用化は `validate_const_expr` 等の広範な改修が必要 |
| `lex.rs:240` | 複数行トークンで `cont.lines().count()` 再走査 | 出現頻度が低い（複数行文字列/コメントのみ）ため効果小 |

> 詳細な行単位の所見は本ドキュメント作成時の各ファイル調査メモに基づく。上記候補は別タスクとして個別に検証・PR 化するのが望ましい。

### 計測ベースライン（criterion）

「効果大」は推測ではなくまず計測する方針。`crates/erg_parser/benches/parser_bench.rs` が
lex / parse（脱糖なし）/ desugar / end_to_end の4段を3サイズで計測する。
実行: `cargo bench -p erg_parser --bench parser_bench`。

中央値（release ビルド・`--warm-up-time 1 --measurement-time 3 --sample-size 50`・単一マシン1回。
絶対値は環境依存、**段間の比率**を見ること）:

| 入力 | bytes | lex | parse | desugar | end_to_end |
| ---- | ----: | --: | ----: | ------: | ---------: |
| small (`tests/containers.er`, 50行) | 382 | 14.1 µs | 27.2 µs | 18.0 µs | 40.3 µs |
| medium (`pystd/builtins.d.er`, 242行) | 9,090 | 56.1 µs | 47.7 µs | 48.2 µs | 165 µs |
| large (`should_ok/long.er`, 699行) | 11,690 | 307 µs | 634 µs | **725 µs** | 1.41 ms |

**読み取れること**:

- **large では desugar が最も重い段（725 µs ≒ 3段合計の ~43%、parse 634 µs を上回り lex 307 µs の 2倍超）**。
  small では desugar(18µs) < parse(27µs) だが large で逆転 — 6パス ≈ 6×O(n) の木再構築が規模で効いている。
  → **次の最有力候補は「desugar パス統合」**（このために計測した）。
- **lex は一貫して最安**。→ `Vec<char>` → `&str` 書き換え（最高リスク）は**最も後回し**でよい、という判断材料。
- end_to_end は概ね lex+parse+desugar に近い（large: 合計1.67ms vs 1.41ms。分離計測のセットアップ重複ぶん合計がやや過大）。
- 注意: 型チェック等の下流コストは未計測。パーサ全体がコンパイル時間に占める割合を別途測ってから着手するのが望ましい。

### 下流コストとの比較・結論

`erg --mode {lex,parse,desugar,check}` を同一バイナリ（release）で `long.er`(699行)に対し計測（最速値、起動コストは差分で相殺）:

| mode | long.er | 備考 |
| ---- | ------: | ---- |
| lex | 2 ms | ≒ 起動 + 字句解析 |
| parse | 6 ms | + 構文解析 |
| desugar | 5–6 ms | + 脱糖（parse とほぼ同値=脱糖は ms 未満） |
| **check (FullCheck)** | **62 ms** | + lowering + 型/作用/所有権チェック |

固定費の確認: `check` を 1 行ファイル `x = 1` に対して実行 → **7 ms**（起動 + builtin プレリュード読込 + 自明チェック）。
つまり `long.er` の 62 ms はほぼ全てが **699 行ぶんの型チェック実コスト**（≈ 55 ms）であり、builtin 読込の固定費は小さい。

**結論（データに基づく）**: パーサのフロントエンド（lex+parse+desugar ≈ 5–6 ms）はフルチェック 62 ms の **~10%**、
最適化対象の desugar 段は **総コンパイル時間の ~2–3%** にすぎない。
desugar パス統合を理想的に 2× 高速化しても全体の ~1% 短縮にとどまる一方、
脱糖はパス順序依存があり**正しさへのリスクが高い**。したがって：

- **desugar パス統合・lexer `Vec<char>` 書き換えともに、現時点では着手しない**のが妥当（リスク/リターンが見合わない）。
- 既に適用済みの安全なクローン除去で、低リスクな改善は回収済み。
- コンパイル時間を本気で縮めるなら、ボトルネックは **`erg_compiler` 側の型チェック/lowering**。最適化の主戦場はそちら。

これがまさに「計測ファースト」の成果: 推測で lexer や desugar の大改修に着手していれば、全体の数%しか動かない箇所に高リスクな労力を費やしていた。
