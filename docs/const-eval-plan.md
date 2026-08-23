# コンパイル時評価 (CTFE) 改善計画

## 概要

`<<`/`>>`/開区間/`~` のオペレータと `int`/`nat`/`float`/`round`/`ord`/`chr`/`hex`/`oct`/`bin`/`pow`/`divmod`/`sorted` の組み込み関数を
コンパイル時評価できるようにした（commits `7a1884f7`, `337bf9a1`, `7a59c473`, `d79fa7ad`）後の、
残ったギャップと改善方針をまとめる。

すべてのギャップはこのツリーで再現を確認済み。各項目に再現コードを添えている。

関連ドキュメント: [const-functions-implementation.md](const-functions-implementation.md)（ユーザー定義 const 関数の実装解説）

---

## 設計原則

他言語の CTFE 実装から得られる原則と、erg の現状との対応。

### 原則 1: ホストスタックで再帰しない

予算は「呼び出し回数」ではなく、実際に枯渇する資源に付ける。

Zig は comptime を 1000 backwards branches で打ち切るが、セルフホスト版コンパイラが comptime 呼び出しを
ホストのスタック上で行っていたため、クォータエラーではなく segfault になるバグを踏んでいる
([ziglang/zig#13724](https://github.com/ziglang/zig/issues/13724))。
Rust は CTFE を MIR インタプリタ (miri) として実装し、評価フレームをヒープに持つ VM にした上で
`const_eval_limit`（既定 1,000,000 ステップ）を課している
([rust-lang/rust#67217](https://github.com/rust-lang/rust/issues/67217))。

**erg**: `Context::call` ([eval.rs:903](../crates/erg_compiler/context/eval.rs#L903)) の
`set_recursion_limit!(..., 128)` は呼び出し回数で数えるが、1 フレームが `Context::instant(... self.clone())` +
本体の lowering という巨大なスタックを消費するため、ガードが効く前にホストスタックが尽きる。→ フェーズ 1

### 原則 2: 「必ず畳み込む文脈」と「投機的な畳み込み」を分ける

C++ は manifestly constant-evaluated な文脈（定数文脈・`consteval`）と、最適化としての constant folding を
明確に分けている。前者は成功かエラーの二択、後者は失敗しても黙って実行時に委ねる
([expr.const](https://eel.is/c++draft/expr.const))。

**erg**: const 関数が `Err` を返すと必ずハードエラーになり、「畳み込めないので実行時に任せる」が表現できない。
`int(_, base)` や `str(<Float>)` の扱いで実際に詰まった。→ フェーズ 4

### 原則 3: コンパイル時の結果は実行時と一致必須 — ホストの算術を使わない

GCC のマニュアルは「constant folding はターゲットの算術をエミュレートしなければならない（さもなくば一切やってはならない）」
と明記し、ホストとターゲットの浮動小数点形式が同一でも常にエミュレーションを使う
([GCC Internals: Cross Compilation and Floating Point](https://gcc.gnu.org/onlinedocs/gccint/target-macros/cross-compilation-and-floating-point.html))。
Ralf Jung も CTFE の決定性は健全性の要件、compile-time と runtime の一致は追求すべき性質とした上で、
浮動小数点が最も難しいと名指ししている
([Thoughts on CTFE and Type Systems](https://www.ralfj.de/blog/2018/07/19/const.html))。

**erg** にとってのターゲット算術は CPython。現状 `0.1 + 0.2` は const で f64 の `0.30000000000000004`、
実行時は `Fraction(3, 10)` = `3/10` になる。→ フェーズ 3

### 原則 4: 決定性と純粋性を型システム側で保証する

Ralf Jung の「const type system」は定数文脈から非 const 関数の呼び出しを型検査で弾き、
"well-typed programs are const-safe" を保証する。

**erg**: `effectcheck` と `should_err/const_nonconst_call.er` で概ね達成済み。
ただし `if` を const 関数化した結果、引数の `do` ブロックが const か否かの検査が呼び出し時まで遅延する。

### 原則 5: AST 直接評価は早晩行き詰まる

CTFE の実装方式は「AST を再帰的に評価」か「IR/バイトコードに落として解釈」の二択
([Compile-Time Function Evaluation](https://jsfpdn.github.io/posts/compile-time-function-evaluation/))。
D は前者で始めて破綻した（式ごとに AST ノードを確保するためメモリを食い、どのノードを解放してよいか分からず、
GC を有効にするとコンパイラ全体が遅くなる）ので、専用 ISA のバイトコードインタプリタに作り直している
([Project Highlight: The New CTFE Engine](https://dlang.org/blog/2016/11/18/project-highlight-the-new-ctfe-engine/))。

**erg**: 完全に AST 直接評価で、1 呼び出しごとに `Context` を丸ごと clone している
（コード中にも `// HACK: should avoid cloning`）。ループを入れない範囲なら現方式で問題ないが、
`for`/`while` の const 化はこの限界に正面からぶつかる。→ フェーズ 6

### 原則 6: 実行時と実装を共有できないなら差分テストを仕組みにする

Zig の comptime は実行時と同じ Sema を使うので原理的にずれない。
**erg は実行時が CPython なので、この共有が構造的に不可能**。残る手段は差分テストのみ。→ フェーズ 0

---

## フェーズ 0: 差分テストの自動化 (最優先)

原則 6。前回の作業で `str` / `//` / `%` / `~` / `Nat` シリアライズの 5 件の不一致を手作業で発見した。
この突き合わせを仕組み化する。

### 現状

`tests/should_ok/const_func.er` などで、以下のように「const 畳み込み値」と「実行時値」を人手で比較している。

```erg
IntTrunc = int 3.7      # 大文字束縛 → const 評価され型は {3}
assert IntTrunc == 3    # 実行時に int(3.7) を評価して比較
```

これは「たまたま書いた式」しか検査しない。

### 実装方針

`tests/` に差分テストハーネスを追加する。

1. 対象ファイル中の `Name = expr`（`Name` は大文字始まり）を集める
2. `--mode check` の HIR 出力から `::Name(: {v})` の畳み込み値を取る
3. 同じ `expr` を小文字束縛にしたプログラムを生成して実行し、出力と `v` を比較

既知の未検出例（フェーズ 3 の Ratio、`reversed`/`map` の戻り値型）が自動で引っかかることを確認する。

### 完了条件

- 新規テストが `reversed [1,2]` の不一致（const は `[2, 1]`、実行時は `Reversed` オブジェクト）を検出する
- `tests/should_ok/const_*.er` 全体が差分テストの対象になっている

---

## フェーズ 1: 再帰ガードの修正 (コンパイラ crash)

原則 1。**コンパイラが落ちるので実害が最も大きい。**

### 再現

```erg
Cnt(N: Nat): Nat = if N <= 1, do 1, do Cnt(N - 1)
X = Cnt 2   # fatal runtime error: stack overflow
```

`F n = F n` 形はきちんと `RecursionError` になるので、`if` 経由の再入をガードが捕捉できていない。
定義だけでは落ちず、**const 位置で呼んだときだけ**落ちることを確認済み。
`2678df93` の worktree でも再現するため、既存バグ（`if` の const 関数化と同時に入った）。

### 問題点

[eval.rs:903](../crates/erg_compiler/context/eval.rs#L903) の `set_recursion_limit!(..., 128)` には 3 つの問題がある。

1. **`ConstSubr::User` 分岐にしか無い** — `ConstSubr::Builtin`（`if_func` など）を経由する再入を数えない
2. **予算の単位が呼び出し回数** — 1 フレームのスタック消費が大きいため、128 回に達する前にホストスタックが尽きる
3. **カウンタがプロセス全体で共有** — [macros.rs:606](../crates/erg_common/macros.rs#L606) の `RecursionCounter` は
   `static AtomicU32`。`parallel` 有効時に複数スレッドが同時に境界を跨ぐと `fetch_sub` が `0` → `u32::MAX` に
   ラップし、`limit_reached()` は `== 0` しか見ないためガードが無効化される

```rust
// macros.rs:617-626 — 現状
impl RecursionCounter {
    pub fn new(count: &'static AtomicU32) -> Self {
        count.fetch_sub(1, Ordering::Relaxed);   // 0 のとき u32::MAX にラップする
        Self { count }
    }
    pub fn limit_reached(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0  // ラップ後は二度と true にならない
    }
}
```

### 実装方針

1. カウンタを `thread_local!` にする（または `Context` / `SharedCompilerResource` に深さを持たせる）
2. `fetch_sub` を `fetch_update` にしてアンダーフローを防ぐ
3. `Context::call` の `ConstSubr::Builtin` 分岐にも同じガードを入れる
4. 限度を「呼び出し回数」から下げる（`Context::instant` + lowering のスタック消費を考えると 128 は多すぎる）か、
   `stacker` などでスタック残量を見る

### 完了条件

- 上記 `Cnt` が `RecursionError` になる（crash しない）
- `tests/should_err/recursive_const.er` は引き続き通る
- 再帰的だが有限で停止する const 関数（`Fact 5` など）は成功する

---

## フェーズ 2: 値レシーバのメソッド呼び出しと添字

「実装済みだが到達できない」ものを繋ぐ、費用対効果の最も高い作業。

### 2.1 値レシーバの const メソッド

[eval.rs:656](../crates/erg_compiler/context/eval.rs#L656) の `eval_attr` は、レシーバが `ValueObj::Type` の
ときしかクラス context の const を探さない。

```erg
X = "abc".replace("a", "z")   # AttributeError: no attribute `replace`
```

そのため const_func.rs に揃っている以下の const メソッドがすべて到達不能:

`str_replace` / `str_join` / `str_find` / `str_startswith` / `str_endswith` / `str_isalpha` /
`str_isascii` / `str_isdecimal` / `list_reversed` / `list_insert_at` / `list_remove_at` /
`list_remove_all` / `list_sum` / `list_prod` / `dict_keys` / `dict_values` / `dict_items` /
`dict_concat` / `dict_diff` / `int_abs` / `__list_getitem__` / `__dict_getitem__` / `__range_getitem__`

**正しいパターンは同ファイル内にある**。`eval_user_binop` が使う `get_const_op_subr`
([eval.rs:1669](../crates/erg_compiler/context/eval.rs#L1669)) は `lhs.class()` から
`get_nominal_super_type_ctxs` を辿って const を引いている。`eval_attr` の非 `Type` 経路でも同じことをすればよい。

```rust
// eval_attr に追加する分岐（イメージ）
if let Some(subr) = self.get_const_op_subr(&obj.class(), ident.inspect()) {
    return Ok(ValueObj::Subr(subr));
}
```

注意: メソッドとして呼ぶ場合、`tp_eval_const_call` が `Self` を先頭引数に渡す既存の経路と整合させること。

### 2.2 添字・タプル属性

[eval.rs:581](../crates/erg_compiler/context/eval.rs#L581) の `eval_const_acc` は `Ident` と `Attr` のみ。
`Accessor::Subscr` / `TupleAttr` / `TypeApp` が未対応。

```erg
X = [1, 2, 3][0]    # AttributeError: no attribute `__getitem__`
X = (1, 2)[0]
X = {"a": 1}["a"]
X = "abc"[0]
T = (1, 2)
X = T.0
```

2.1 が入れば `__list_getitem__` / `__dict_getitem__` / `__range_getitem__` に繋がるので、
`Subscr` を `__getitem__` の呼び出しに落とすだけで済むはず。

### 完了条件

- 上記の再現コードがすべて畳み込まれる
- フェーズ 0 の差分テストで、畳み込み値が実行時値と一致することを確認

---

## フェーズ 3: Ratio / Float と整数範囲

原則 3。**型が実行時と食い違うので、放置すると誤ったコードが通る。**

### 3.1 Ratio

```erg
X = 0.1 + 0.2   # const: {0.30000000000000004}、実行時: Fraction(3, 10) = 3/10
```

erg の float リテラルは実行時 `Fraction` だが、`ValueObj` には Ratio 表現が無く f64 で畳み込んでいる。
GCC の原則どおり「厳密にエミュレートする」か「一切畳み込まない」の二択で、
**現状の f64 近似が最悪**。

- 案 A: `ValueObj::Ratio(num, den)` を追加し、Ratio リテラルを厳密に畳み込む
- 案 B: Ratio 型に解決される値は畳み込まない（`uncomputable_op` を返す）

案 A が望ましいが `ValueObj` の変更は広範囲に及ぶ。案 B を先に入れて誤りを止めるのも可。

### 3.2 整数範囲

`ValueObj::Int` が `i32`、`Nat` が `u64` なので非対称。正の側は `u64::MAX` まで畳み込めるが、
負の側は `i32::MIN` までしか表現できない。

```erg
X = 1 - 10000000000   # NotConstExpr（畳み込めない）
x = -3000000000       # SyntaxError: invalid literal（レキサ）
```

`ValueObj::Int` を `i64`（あるいは任意精度）に広げるのが筋。`ValueObj::Int(` は 131 箇所で参照されているため、
単独のコミットとして行う。`d79fa7ad` で `TYPE_LONG` のシリアライズ・デシリアライズは実装済みなので、
バイトコード側の対応は不要。

---

## フェーズ 4: 「畳み込み不可」をエラーと区別する

原則 2。

### 現状

const 関数は `EvalValueResult<TyParam>` を返し、`Err` は必ずハードエラーになる。
「この呼び出しは畳み込めないので実行時に委ねる」が表現できないため、部分的にしか対応できない
組み込み関数（`str` の Float、`int` の巨大値など）で過剰に厳しくなる。

なお非 const 位置（小文字束縛）では const 評価自体が走らないため、実害は const 位置に限られる。

### 実装方針

`EvalValueResult` に第三の状態（`Deferred` / `NotFoldable`）を追加し、
`eval_const_call` ([eval.rs:744](../crates/erg_compiler/context/eval.rs#L744) 付近) で

- 大文字束縛など「必ず定数でなければならない」文脈 → 診断を出す
- それ以外 → 畳み込みを諦めて実行時呼び出しに委ねる

と分岐する。`todo(...)` を返している箇所（`int (out of range)`、`str(<Float>)`、`chr (surrogate)` など）を
この新しい状態に移す。

---

## フェーズ 5: 残りのオペレータと組み込み関数

フェーズ 1〜4 と独立に進められる。

### オペレータ

| 未対応 | 例 | 対応箇所 |
| --- | --- | --- |
| 非数値の比較 | `"a" < "b"`, `[1] < [2]`, `(1,) < (2,)`, `[1] == [1]` | `try_gt` / `try_lt` / `try_eq` (value.rs) |
| `Bool` の算術 | `True + True` | `try_add` に Bool アームを追加 |
| Dict マージ / Set 差 | `{"a": 1} + {"b": 2}`, `{1,2} and {2}` | 型検査で弾かれている |
| `is!` / `isnot!` | `1 is! 1` | `try_get_op_kind_from_token` |
| 連鎖比較 | `1 < 2 < 3` | パーサ／脱糖 |

### 組み込み関数

未 const: `repr` `ascii` `hash` `isinstance` `issubclass` `bool` `format` `list` `set` `dict` `tuple`
`frozenset` `range` `enumerate`

- `isinstance` / `issubclass` は `subtype_of` があるので実装しやすく、型レベルの分岐に効く（優先）
- `repr` / `format` は表現が実行時と一致するか要検討（`str` で踏んだのと同じ罠。Python の `repr("a")` は
  シングルクォート `'a'`）

### const 関数の戻り値型の不一致

`reversed` / `filter` / `map` / `zip` は const では `List` を返すが、実行時はイテレータオブジェクト。

```erg
D = reversed [1, 2]
assert D == [2, 1]   # 型は {[2, 1]} だが実行時は False
```

型を合わせるか、これらを const から外すかの判断が要る。

### その他の評価機構のギャップ

- 無名ラムダが const 化できない（小文字パラメータは非 const、大文字は `NameError`）。
  結果 `map` / `filter` には名前付き const 関数しか渡せない
- 複数チャンクの const 関数本体で仮引数が見えない（`Tmp = N + 1` の行で `N is not defined`）
- 可変長引数の const 関数は `FeatureError: const parameters`
  ([eval.rs:636](../crates/erg_compiler/context/eval.rs#L636))
- `eval_const_chunk` ([eval.rs:1584](../crates/erg_compiler/context/eval.rs#L1584)) に
  `ClassDef` / `PatchDef` / `Methods` / `ReDef` / `Compound` が無い（コード中に TODO あり）

---

## フェーズ 6: 評価器の再設計（要検討）

原則 5。`for` / `while` / `match` の const 化はここに含まれる。

現状は AST 直接評価 + 1 呼び出しごとの `Context` clone。ループを導入するとメモリと速度の両方で行き詰まる
（D が同じ道を通った）。着手する場合は、

- HIR あるいは専用バイトコードへ落として明示フレームのインタプリタで実行する
- 予算をステップ数で数える（原則 1 の解決も同時に得られる）

という形になる。**フェーズ 1〜5 とは規模が違うので、独立した設計判断として扱う。**

---

## 優先順位まとめ

| 順 | フェーズ | 理由 |
| --- | --- | --- |
| 1 | フェーズ 0（差分テスト） | 以降の作業の安全網。これ無しに 3 を触るのは危険 |
| 2 | フェーズ 1（再帰ガード） | コンパイラが落ちる |
| 3 | フェーズ 2（メソッド・添字） | 実装済みの資産が繋がる。現アーキテクチャのままで安全 |
| 4 | フェーズ 3（Ratio / 整数範囲） | 型が実行時と食い違う |
| 5 | フェーズ 4（畳み込み不可の区別） | 3・5 の前提になると楽 |
| 6 | フェーズ 5（残りのオペレータ・関数） | 独立して進められる |
| 7 | フェーズ 6（評価器再設計） | 設計判断が要る |

フェーズ 1〜3 は現アーキテクチャのまま取れる。フェーズ 6 だけが別格。

---

## 参考文献

- [design flaw: self-hosted compiler uses its own stack for comptime function calls — ziglang/zig#13724](https://github.com/ziglang/zig/issues/13724)
- [Tracking issue for const_eval_limit — rust-lang/rust#67217](https://github.com/rust-lang/rust/issues/67217)
- [Long-running const-eval (loops) unusable in practice — rust-lang/rust#93481](https://github.com/rust-lang/rust/issues/93481)
- [Thoughts on Compile-Time Function Evaluation and Type Systems — Ralf Jung](https://www.ralfj.de/blog/2018/07/19/const.html)
- [Constant evaluation — The Rust Reference](https://doc.rust-lang.org/reference/const_eval.html)
- [Cross Compilation and Floating Point — GCC Internals](https://gcc.gnu.org/onlinedocs/gccint/target-macros/cross-compilation-and-floating-point.html)
- [expr.const — C++ draft standard](https://eel.is/c++draft/expr.const)
- [Project Highlight: The New CTFE Engine — The D Blog](https://dlang.org/blog/2016/11/18/project-highlight-the-new-ctfe-engine/)
- [Compile-Time Function Evaluation — jsfpdn](https://jsfpdn.github.io/posts/compile-time-function-evaluation/)
- [Zig Programming Language Blurs the Line Between Compile-Time and Run-Time — Andrew Kelley](https://andrewkelley.me/post/zig-programming-language-blurs-line-compile-time-run-time.html)
