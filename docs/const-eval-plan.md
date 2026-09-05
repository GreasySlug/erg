# コンパイル時評価 (CTFE) 改善計画

## 概要

`<<`/`>>`/開区間/`~` のオペレータと `int`/`nat`/`float`/`round`/`ord`/`chr`/`hex`/`oct`/`bin`/`pow`/`divmod`/`sorted` の組み込み関数を
コンパイル時評価できるようにした（commits `7a1884f7`, `337bf9a1`, `7a59c473`, `d79fa7ad`）後の、
残ったギャップと改善方針をまとめる。

すべてのギャップはこのツリーで再現を確認済み。各項目に再現コードを添えている。

**進捗**: フェーズ 0〜6 実装済み。段 3（`for` / `while`）だけは**前提が誤っていた**ため対象外
（Erg に純粋な `for` / `while` は無く、`!` 版は手続きなので const 評価は拒否すべきもの）。
同じ枠にあった `match` は畳めるようにした。

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

## フェーズ 0: 差分テストの自動化 ✅ 完了 (`41c7c096`)

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

### 実装結果

`tests/const_diff.rs`。約 110 個の式それぞれについて、`--mode check` の HIR から畳み込み値を取り、
同じ式を実行した出力と突き合わせる。`CASES` は一致必須、`KNOWN_DIVERGENT` は**一致してはいけない**
（直したらテストが落ちてエントリ削除を促す）。

導入時点で新規に 1 件検出: `7 / 2` が f64 の 3.5 に畳み込まれるが実行時は `Fraction(7, 2)`。
以降のフェーズでも `", ".join(["a", "b"])` → `"a, b,"`、const 束縛した Ratio が float として
焼き込まれる問題を検出した。

---

## フェーズ 1: 再帰ガードの修正 ✅ 完了 (`af970054`)

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

### 実装結果

根本原因は上記 3 点ではなく **`eval_const_lambda` がラムダ本体を先行評価していたこと**だった。
`if` の分岐は `do` ブロックなので、呼ばれない側の分岐も毎回評価され、再帰が基底ケースに到達できない。
ネストしたラムダは戻り値型を `Obj` にフォールバックさせた（値は実際に呼んだ結果から来るので影響なし）。

結果として **再帰的 const 関数が動くようになった**（`Fact 10`、`Fib 10`、相互再帰）。
併せて予算を `STACK_SIZE` から導出し（`CONST_CALL_LIMIT`）、カウンタをスレッドローカル化した。
`tests/should_ok/const_recursive.er` / `tests/should_err/recursive_const.er`。

---

## フェーズ 2: 値レシーバのメソッド呼び出しと添字 ✅ 完了 (`2ade62bd`)

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

### 実装結果

`eval_attr` に `get_const_op_subr` によるクラス const の探索を追加しただけで、
約 20 個の const メソッドが一斉に到達可能になった。`Tuple`/`Str` は `__getitem__` を
const subr ではなく型付きメソッドとして持つため `tp_eval_const_call` で直接処理し、
`Range` 値は poly クラスを返すようにして束縛済み range も添字可能にした。
`tests/should_ok/const_method.er`。

差分テストで `", ".join(["a", "b"])` が `"a, b,"` になるバグを検出・修正（区切り文字を各要素の後に
付けて 1 文字だけ pop していた）。dict の `.keys()/.values()/.items()` は List を返すうえ
`ValueObj::Dict` が挿入順を保たないため、`KNOWN_DIVERGENT` に記録した。

---

## フェーズ 3: Ratio / Float と整数範囲 ✅ 完了 (`01dd5a28`, `78217f57`)

原則 3。**型が実行時と食い違うので、放置すると誤ったコードが通る。**

### 3.1 Ratio

```erg
X = 0.1 + 0.2   # const: {0.30000000000000004}、実行時: Fraction(3, 10) = 3/10
```

erg の float リテラルは実行時 `Fraction` だが、`ValueObj` には Ratio 表現が無く f64 で畳み込んでいる。
GCC の原則どおり「厳密にエミュレートする」か「一切畳み込まない」の二択で、
**現状の f64 近似が最悪**。

**採用: 案 A**（`ValueObj::Ratio(i128, u128)` を追加）。`ValueObj` への variant 追加は
非網羅 match 27 箇所で済み、`mono_value_pattern!` にも追加することで大半が解消した。

- `i128`/`u128` なのは SI 接頭辞リテラルが `1e+30`/`1e-30` に達するため（`lib/std/unit.er`）
- それでも収まらないリテラル（`6.62607015e-34` は 10^42 の分母が要る）は従来どおり `Float` にフォールバック
- 畳み込まれた有理数はリテラル文字列を持たず `Fraction` は marshal 定数でもないので、
  codegen で `Fraction(n/d)` として再構築する（これが無いと `A = 0.1 + 0.2; print! A` が 0.3 を出す）
- `class()` は `Ratio` を返す。`Float` のままだと codegen が `Float(...)` へキャストして
  実行時に float 化されてしまう。副作用として `random!` の stub の `0.0..1.0`（有理数区間になり
  float が住めない）を `Float` に修正した

### 3.2 整数範囲

`ValueObj::Int` が `i32`、`Nat` が `u64` なので非対称。正の側は `u64::MAX` まで畳み込めるが、
負の側は `i32::MIN` までしか表現できない。

```erg
X = 1 - 10000000000   # NotConstExpr（畳み込めない）
x = -3000000000       # SyntaxError: invalid literal（レキサ）
```

**採用: `ValueObj::Int` を `i64` 化**（`78217f57`）。コンパイルエラーは 29 箇所で、
ほぼ機械的な型変換だった。シリアライズは `Nat` と同じ規則（`i32` に収まれば `TYPE_INT`、
それ以外は `TYPE_LONG`）。

広げた副産物として既存バグが 2 件出た: `Int(i) == Nat(n)` が `i as u64` で比較していたため
`Int(-1) == Nat(u64::MAX)` が成立していたこと、`Int`/`Nat` の混在比較が `as i32` で切り捨てていたこと。

---

## フェーズ 4: 「畳み込み不可」をエラーと区別する ✅

原則 2。

### 現状（実装前）

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

### 実装結果

**投機的な畳み込みの経路は既にエラーを捨てていた**（`eval_t_params` は `ret.ok()`、
`TyParam::App` の評価は `_ =>` で握り潰す）。const 呼び出しは const 位置でしか走らないので、
「実行時に委ねる」側は最初から成立していた。残っていたのは**診断の質**で、

```erg
X = pow(2, 10000)
# FeatureError: pow(2, 10000) is not supported yet in const context
```

は嘘だった（未実装ではなく、値が `ValueObj` に収まらないだけ）。

`EvalValueError` に `not_foldable: bool` を追加し（`EvalValueResult` の型を 3 状態に変えるのは
const 関数 100 箇所超の機械的な書き換えになるため、振る舞いだけを分けた）、
`Context::call` の Builtin/Gen アームで `EvalError::unfoldable_call` に振り替える:

```erg
X = pow(2, 10000)
# NotConstExpr: cannot fold `pow(2, 10000)`: the result does not fit an `Int`
#   hint: bind it to a lowercase name (a run-time variable) and it is computed at run time
```

`todo(...)` は本当に未実装の箇所（`str` の `encoding` 引数）にだけ残した。
テスト: `tests/should_err/unfoldable_const.er`。

---

## フェーズ 5: 残りのオペレータと組み込み関数 ✅（一部は意図的に非対応）

フェーズ 1〜4 と独立に進められる。

### オペレータ

| 項目 | 例 | 結果 |
| --- | --- | --- |
| 非数値の比較 | `"a" < "b"`, `[1] == [1]`, `(1,) == (1,)` | ✅ |
| `Bool` の算術 | `True + True` | ✅ |
| 連鎖比較 | `1 < 2 < 3` | ✅（**意味自体が間違っていた**、下記） |
| List / Tuple の順序 | `[1] < [2]` | 未対応（型検査が弾く。`PartialOrd` の追加が要る） |
| Dict マージ / Set 差 | `{"a": 1} + {"b": 2}`, `{1,2} and {2}` | **やらない**（下記） |
| `is!` / `isnot!` | `1 is! 1` | **やらない**（下記） |

**連鎖比較は畳み込み以前に意味が違っていた**。左結合で `1 < 3 < 2` が `(1 < 3) < 2` =
`True < 2` = `1 < 2` = `True` になり、Python（と数学）の `False` と食い違っていた。
`a < b` の後に `< c` が来たら `a < b and b < c` に組み替える。`and` は比較より結合が緩いので、
ここで見える `AndOp` は必ず自分が組んだもの。
中央の被演算子は両隣と比較するため二回書く必要があり、式の中に一時変数を作る手段がないので、
**変数・リテラル・属性以外は構文エラーにした**（CPython は一回しか評価しないので、
二回評価して黙って食い違うより拒否する）。

**`is!` は畳み込まない**。`1 is! 1` は実行時に `False` になる — codegen の型ラップが
`Nat(1) is Nat(1)` と別々のオブジェクトを二つ作るため。同一性はここではコンパイル時の性質ではない。
（調査中に、トランスパイラが `is!` をそのまま出して無効な Python になっていたのを発見・修正）

**Dict マージ / Set 差はそもそも Python にない**（`dict + dict` はエラー、集合は `&`/`-`）。
畳み込みを足すには先に「実行時にどう動くか」を決める必要があり、CTFE の範囲ではなく言語機能の追加。

### 組み込み関数

`isinstance` / `issubclass` を const 化（**優先項目**）。オブジェクトの class は
実行時に構築されるクラスそのものなので、`isinstance(-1, Nat)` は
コンパイル時も実行時も `False`（`_erg_nat.py` でも `Int` は `Nat` を継承しない）。
タプル形式 `isinstance(x, (A, B))` にも対応。

畳むのは classinfo が**素のクラス**（組み込みスカラーか単相のユーザークラス）のときだけ。
`List(Int)` のようなパラメータ化ジェネリクスは実行時の `isinstance` が
`TypeError` を投げるので、静的に True/False を作ったら「プログラムが実際に
産む値」ではなくなる。該当時は unfoldable として報告する。

以下は**やらない判断**:

- `hash`: 文字列のハッシュは `PYTHONHASHSEED` で実行ごとに変わる。畳み込めば必ず嘘になる
- `range` / `enumerate`: 遅延イテレータを返すのでフェーズ「畳み込みでクラスを変えない」に反する
- `repr` / `ascii` / `format`: Python の文字列 repr（引用符の選択とエスケープ）を完全に再現する
  必要があり、リスクに対して const 位置での需要が薄い。`str` は既に const
- `bool`: そもそも名前が定義されていない（const のギャップではなく未実装の組み込み）

### const 関数の戻り値型の不一致 ✅

`reversed` / `filter` / `map` / `zip` は const では `List` を返すが、実行時はイテレータオブジェクト。
`.keys()` / `.values()` / `.items()` も同様（実行時はビュー）。

```erg
D = reversed [1, 2]
print! D             # [2, 1]
print! D             # []  ← 同じ定数が二度目は空
```

codegen は `D` の静的型 `{[2, 1]}`（クラスは `List`）に合わせて使用箇所ごとに `List(...)` で
包むが、`D` の実体は遅延イテレータなので最初の包みで枯れる。`abs` / `**` と同じ
「嘘の戻り値型が誤った値になる」系統。

**採った解**: 畳み込み自体は残し、**チャンク自身の値**になるときだけクラス不一致を弾く
(`eval_const_chunk` の Call アーム → `folded_away_class`)。ネストした呼び出しは
`eval_const_expr` を通るので `sum(map(F, l))` や篩型述語の `all(map(P, xs))` は今まで通り畳み込める。
`tests/should_ok/dependent_refinement.er` が const `map` に依存しているので、const から外す案は取れない。

交差型の一部の枝しか nominal でない場合（`x[i]` は `(Nat) -> T` と `(Range) -> List(T)`）は
判定を放棄する。ここを忘れると全ての添字アクセスが弾かれる。

### その他の評価機構のギャップ

- ~~無名ラムダが const 化できない（小文字パラメータは非 const、大文字は `NameError`）~~ → 修正済み（2026-09-04）。
  `eval_const_lambda` は戻り値型を精密にするために本体を定義時に評価していて、本体が自分の仮引数を
  読むと `NameError` で**ラムダ全体が失敗**していた。評価に失敗したら戻り値型は宣言（無ければ `Obj`）に
  落とし、値は `UserConstSubr` として残す（呼ばれたときに本体は仮引数を束縛して評価される）。
  `Inc = (X: Int) -> X + 1; Inc(1)` と `Apply((X: Int) -> X + 1, 2)` が畳める。
  ただし**型の中**（篩型の述語 `all(map((X -> X > 1), A))`）のラムダは値にせず記号形 `TyParam::Lambda` のまま
  （`instantiate_const_expr` で仮引数持ちラムダは評価をスキップ）。述語は仮引数を代入して評価されるので値では困る
- ~~複数チャンクの const 関数本体で仮引数が見えない~~ → 修正済み（2026-08-27）。
  lowering が本体の定義を先行評価していた。const 引数がスコープにあるときだけ先行評価の失敗を捨て、
  通常の lowering に型付けさせる（非 const 関数では従来どおりエラー）
- ~~可変長引数の const 関数は `FeatureError: const parameters`~~ → 修正済み（2026-09-04）。
  引数ゼロの const 関数 `F() = 1` も同じ根で、登録時に「パラメータが無ければ本体を即評価して
  値にする」分岐に落ちていた。`F` の型が `{1}` になる一方 codegen は関数を束縛するので、
  **検査を通ってから実行時に `NameError`** になっていた。可変長は `Context::call` が元から
  残りの引数を List に束縛していて、実行時の `*args` も erg は List にするので一致する
- ~~`eval_const_chunk` に `ClassDef` / `PatchDef` / `Methods` / `ReDef` / `Compound` が無い~~ → 整理済み（2026-09-04）。
  本当のギャップは別で、`eval_const_def` が**小文字の定義を弾いていた**こと。const 関数の本体で
  `m = n + 1` も、`(a, b) = t` の脱糖結果 `%v_desugar_1 = t; a = ...` も `NotConstExpr` になっていた。
  評価中のスコープ（呼び出しフレーム・畳み込み中のブロック）では名前に関わらずコンパイル時の束縛なので、
  module / 型本体でだけ小文字を弾く。`Compound` も評価する。クラス・パッチ・メソッド定義は
  const 関数本体では「定数式でない」として弾く（設計どおり）
- ~~クラス本体の順序: `C.Tripled = C.Base * 3` が `Base` を見つけられない~~ → 修正済み（2026-09-04）。
  メソッドブロックのコンテキストはブロックが終わるまでクラスに接ぎ木されないので、
  `eval_attr_and_recv` は型の nominal 検索に失敗したら**今いるスコープ鎖の MethodDefs**（名前で
  クラスを同定）の `consts` も見る（`const_of_enclosing_methods`）
- ~~篩型を名前に束縛できない（`Odd = {N: Int | N % 2 == 1}` が `NotConstExpr` +
  `filter::iterable` の型エラー）~~ → 修正済み（2026-09-05）。ドキュメントの書き方
  （`12_refinement.md`・`quick_tour.md`）なのに、値の位置では内包表記として
  `set(filter(...))` に脱糖されていた。`{n <- xs | n > 1}` とは形が同じで後から区別できない
  （`1..10` も型なので `{i <- 1..10 | i <= 5}` は篩型とも読める）ため、**どちらを書いたかを
  パーサーが記録する**（`Set::Refinement`）。脱糖はそのまま通し、`eval_const_set` が
  型注釈位置と同じ型を返す。実行時に述語を持てるものは無いので、束縛が出すのは基底型
  （`Odd = Int` と同じ）
- ~~述語の中の算術（`N % 2 == 1`）は畳めず `<failure> == 1` になる~~ → 修正済み（2026-09-05）。
  篩型の述語を CTFE に繋いだ。`Predicate` の葉は値と名前しかなく、算術は
  `instantiate_pred_from_expr` が feature error にして（それも握り潰されて）いた。
  **項** `Predicate::Tp(TyParam)` を足し、命題を作る演算子（比較・`and`/`or`/`not`）以外は
  項として記号形のまま持つ。判定側の仕組み（`structural_supertype_of` が篩変数を候補値に
  束縛して `bool_eval_pred` を呼ぶ）は元からあった。
  併せて 2 件:
  - `Predicate::mentions` が呼び出しの中の篩変数を見ておらず、`{N: Int | IsOdd(N)}` が
    **基底型そのもの**として扱われていた（述語が篩変数に言及しない篩型は基底型なので）。
    これが「const 関数を述語にしても検査されない」の正体
  - `eval_bin_tp` は被演算子が値でないと畳まず、`(X * 2) + 1` のような入れ子項が
    feature error になっていた。畳めなかったら被演算子を評価して一度だけ再試行する

---

## フェーズ 6: 評価器の再設計（段 0・D3・スタック継ぎ足しまで実装済み）

原則 5。`for` / `while` / `match` の const 化はここに含まれる。

現状は AST 直接評価 + 1 呼び出しごとの `Context` clone。ループを導入するとメモリと速度の両方で行き詰まる
（D が同じ道を通った）。着手する場合は、

- HIR あるいは専用バイトコードへ落として明示フレームのインタプリタで実行する
- 予算をステップ数で数える（原則 1 の解決も同時に得られる）

という形になる。**フェーズ 1〜5 とは規模が違うので、独立した設計判断として扱う。**

### 実測（2026-08-27、release ビルド）

判断の根拠になる数字を取った。`Fib(N) = if N <= 1, do N, do Fib(N-1) + Fib(N-2)`:

| | 実装前 | メモ化後 |
| --- | --- | --- |
| `Fib(14)` | 1,055 ms | 13 ms |
| `Fib(18)` | 10,682 ms | 15 ms |
| `Fib(20)` | 120 s 超（打ち切り） | 16 ms |
| `Fib(25)` | — | 19 ms |

`Fib(18)` は約 8,000 呼び出しなので **1 呼び出し約 1.3 ms**。しかもこれは
**モジュールの規模に比例する**: 同じ `Fib(14)` に無関係なモジュールレベル束縛を 300 個足すだけで
1,055 ms → 2,744 ms（2.6 倍）になった。`Context::instant(..., self.clone())` が
呼び出し元コンテキスト（`locals` / `consts` / `mono_types` / `poly_types` …）を
まるごと deep copy しているため。

### 着手した部分: 純粋な const 呼び出しのメモ化

再設計を待たずに効き、再設計後も残る部分から入れた（`SharedConstCallCache`）。
const 関数は純粋（原則 4）で、**const 名はスコープ内でシャドウイングできない**（`G` を
二重に束縛すると `AssignError`）ので、「**定義スコープ**（モジュールパス + スコープ名、
登録時に `UserConstSubr` へ焼き込む）+ 名前 + 引数」が結果を決める。
評価中のモジュールをキーにしてはいけない: `x = import "mod_x"` の後の `x.F(3)` は
main のコンテキストで評価されるので、main 自身の `F(3)` と衝突する（実際にした）。

キャッシュしない条件:

- **`sig_t` が `Quantified` または型変数を含む** — 本体が呼び出し元の型変数を読むので、
  引数だけでは決まらない
- **def_scope を持たない定義** — ラムダ・`if` の分岐（合成名 `<lambda>`）と、評価中の
  本体の中の定義。どちらも「たまたま周囲にあるもの」を読む。最初 `<lambda>` を見落とし、
  `Fib(3)` の分岐の値が `Fib(4)` の分岐に返ってきた（差分テストではなく
  `should_ok` の singleton 型注釈が捕らえた）
- **エラーになった呼び出し** — 診断は周囲の文脈に依存する

ELS のためにモジュール再コンパイル時は `SharedCompilerResource::clear` でそのモジュール分を捨てる。

### 残り: 設計計画（2026-08-27 策定）

#### 現状の実測

**深さ**: `Fib(32)` は通り、`Fib(33)` は `RecursionError`。`CONST_CALL_LIMIT` は
`STACK_SIZE / 128KiB` = **64**（既定ビルドの 8 MiB）だが、**再帰 1 段が const 呼び出しを
2 つ使う**（関数本体 + `if` の分岐。分岐は `<lambda>` という `UserConstSubr` である）ので、
書く側から見える深さは **32 段**。CPython の既定再帰上限は 1000。

**コスト**: 1 呼び出し約 1.3 ms、しかもモジュール規模に比例（無関係な束縛 300 個で 2.6 倍）。

**正しさ**（この計画を立てる過程で発見）: 別モジュールの const 関数が
自分のモジュールの名前を読むと落ちる。

```erg
# a3.er
.K = 10
.F(N: Int): Int = N + .K
# main.er
a3 = import "a3"
X = a3.F(3)   # NameError: K is not defined
```

フレームの親が**呼び出し元**であって**定義元**ではないため。メモ化のキーを
`def_scope` に直したのと同じ根っこで、キーは直したが**本体の名前解決はまだ呼び出し元を見ている**。

同じ領域の既存バグ: ユーザークラスのユーザー定義 const メソッドはそもそも畳めない
（`C.Twice(N: Int): Int = N * 2` を呼ぶと `N` が `<module>::C` に解決され
`NotConstExpr: <module>::C * 2` になる）。
→ **原因はフレームではなく引数だった**（2026-08-27 修正）。属性がどの探索で見つかったかに関わらず
レシーバを第 1 引数に挿していたため、クラス自身が `N` に束縛されていた。
値のクラス経由（`get_const_op_subr`）で見つかったときだけ self を取る。
ただし**クラス自身の const を読む**メソッド（`C.Base + N`）は依然として畳めない。
フレームの親がモジュールスコープなのでクラススコープが見えない — こちらは段 1 の領域。

#### 原因は 1 行

`eval.rs:1023` の

```rust
let mut subr_ctx = Context::instant(user.name.clone(), self.cfg.clone(), 2,
                                    self.shared.clone(), self.clone());
```

フレームの親が呼び出し元の deep clone である。`Context` は `outer: Option<Box<Context>>` と
`mono_types` / `poly_types: Dict<VarName, TypeContext>`（`TypeContext` は `Context` を含む）を
持つので、`derive(Clone)` は**モジュール内の全クラスの全メソッドコンテキストまで再帰的に複製する**。
深さ・速度・スコープの誤りは 3 つともここから出ている。

構造上の追い風が 2 つある。

- 評価器はほぼ全部 `&self` である（`eval_const_expr` / `_acc` / `_call` / `_list` …）。
  `&mut self` は `eval_const_def`（本体内のローカル const 定義）と、それを呼ぶ
  `eval_const_chunk` / `eval_const_block` だけ。つまり**フレームが自前で持つ可変状態は
  引数束縛とローカル定義だけ**で、残りは読むだけである。
- `outer` の所有権が要るのは**低下時のスコープスタック**（`self.outer = Some(Box::new(mem::take(self)))`
  で push、`lower.rs` が `outer.as_mut()` で親に代入する）であって、const フレームとは別の用途である。

#### 段取り

計画の順序は「フレームのヒープ化 → ステップ予算 → ループ」だったが、
**その前に「スコープを直す」を独立の段として置く**。インタプリタなしで実装でき、
単体で価値があり（上の `NameError` が直る）、かつ「clone がなぜ在るのか」を消すので
後段の設計が素直になる。

##### 段 0: フレームの親を定義スコープにする（正しさ・小）

`UserConstSubr::def_scope`（`(NormalizedPathBuf, Str)`、メモ化のときに登録時点で焼き込み済み）を
使って、フレームの親を**定義元のスコープ**にする。

- `def_scope.module` が評価中のモジュールと同じなら、今までどおり呼び出し元の鎖を使う
  （同一モジュール内では const 名がシャドウイングできないので、これは正しい）
- 違うなら `shared.mod_cache` からそのモジュールのコンテキストを引く
- `def_scope` が `None`（ラムダ・`if` の分岐・本体内の定義）は今までどおり呼び出し元

**この段だけで clone をなくすことはできない**（親を所有せずに持つ手段がまだない）ので、
`self.clone()` は残る。速度は変わらず、正しさだけが直る。

- 検証: `tests/should_ok/const_cache/` に「別モジュールの const 関数が自モジュールの
  定数を読む」ケースを足す。上の `a3.F(3)` がそのまま回帰テストになる。
- risk: 初回低下中は自モジュールがまだ `mod_cache` に入っていない。上の分岐で自然に避けられるが、
  循環 import では「相手はまだ低下中」がありうる。その場合は畳まずに実行時へ回す。

##### 段 1: フレームのヒープ化（本体）※実装は下記「実装結果」を参照

呼び出しごとの `Context` clone をやめ、明示フレームのスタックにする。

```rust
struct ConstFrame {
    /// 引数束縛と本体内のローカル const 定義。フレームが持つ可変状態はこれだけ
    locals: Dict<VarName, ValueObj>,
    /// 自由名を解決する先（段 0 で決めた定義スコープ）
    scope: ScopeRef,
}
```

名前解決は「上のフレームの `locals` → `scope` の鎖 → builtins」。
`Context` は 1 つも複製されない。**`outer` の型は変えない**（低下時のスコープスタックのままにする）。

深さについては 2 択がある（D4）。トランポリン方式（呼び出しだけ反復にし、式の評価は
今の Rust 再帰のまま）を推す。深さ制限は const 呼び出しの入れ子で決まっており、
式の入れ子は書かれた AST の深さで自然に小さいからである。

##### 段 2: ステップ予算

フレームがヒープに載って初めて意味を持つ。

- 深さ: `frames.len()` で数える。ホストスタックから外れるので、上限は 32 ではなく
  CPython 並み（1000）に置ける
- 総量: ステップ数を数え、`F(N) = F(N-1) + F(N-1)` のようにメモ化が効かない指数爆発を止める
- `CONST_CALL_LIMIT`（= `STACK_SIZE / 128KiB`）はここで役目を終える

##### 段 3: ループ → **対象外**（2026-08-28）

`for` / `while` の const 化を予定していたが、**前提が誤っていた**。Erg に純粋な
`for` / `while` は存在せず（`for` / `while` という名前は未定義）、あるのは `for!` / `while!` の
手続きだけである。手続きの畳み込みは副作用検査が拒否すべきものなので、ここには作業がない。

同じ枠に挙げていた `match` は畳めるようにした（下記）。

#### 決めるべき分岐

| | 論点 | 選択肢 | 推し |
| --- | --- | --- | --- |
| D1 | フレームはスコープをどう指すか | (a) `(module, scope名)` のカーソルを都度解決 / (b) `Arc<Context>` / (c) `&'c Context` を評価器が借りる | **(a)**。lifetime も `Arc` 化も要らず、解決結果は呼び出しごとに 1 回メモすれば済む。(c) は別モジュール参照で `RwLock` のガードを評価中ずっと持つことになる |
| D2 | 何を評価するか | (a) 今のまま `user.block()` で `ConstBlock` → `Block` に downgrade して汎用 `Expr` を歩く / (b) `ConstExpr`（13 variant の閉じた文法）を直接歩く | **(b)**。明示フレーム機械は文法が閉じている方が書ける。downgrade の clone も 1 呼び出しぶん消える |
| D3 | `if` の分岐 | (a) 今のまま `<lambda>` の const 呼び出し / (b) `if` を特別扱いしてインライン評価 | **(b)**。フレーム数が半分になり、`def_scope` を持てないという理由でメモ化から外れていた分岐が畳めるようになる。ただし段 1 が終わってからでよい |
| D4 | 段 1 の深さ | (a) トランポリン（呼び出しだけ反復） / (b) 完全な明示継続機械（CEK 風） | **(a)**。(b) は eval.rs の const 部分（約 1,000 行）の書き直しになる。(a) で深さ制限と clone 除去という目的は両方達成できる |

#### やらないこと

- **`outer: Option<Box<Context>>` の型を全体で変えない**。あれは低下時のスコープスタックで、
  `lower.rs` が親に書き戻す用途がある。const フレームとは別の仕組みとして足す
- **専用バイトコード VM を作らない**。`ConstExpr` は 13 variant しかなく、木を歩く機械で足りる。
  速度はメモ化がすでに 700 倍取っており、ここで欲しいのは深さと正しさである
- **ステップ予算を先に入れない**（段 2 の位置は動かさない）

#### 実装結果（2026-08-27）

段 0 と D3 はそのまま実装した。段 1・2 は**計画とは違うやり方**になったので、
何がどう違ったかを残す。

**D4(a)「トランポリン」は目的を達成できない**。実測すると、1 段あたりのホストスタックは
**約 89 KiB**（限界を外して `Countdown` を深くしていくと 8 MiB で約 46 段 = 92 tick で
オーバーフロー）。これは呼び出しの瞬間ではなく、`call` → `eval_const_block` →
`eval_const_chunk` → `eval_const_call` → 引数評価 → … → `if_func` → 分岐 → 次の `call`
という**呼び出しと呼び出しの間の式評価の鎖**で消費されている。呼び出しだけを反復にしても
この鎖は残るので、深さは変わらない。実際、D3 で `if` の分岐をインライン化したら
tick が半分になっただけで、その分深く潜れてしまい**スタックオーバーフローした**
（ガードを 1 本の共有カウンタにして解決）。

**代わりに入れたもの: スタックを継ぎ足す**。1 つのスタックを使い切ったら、
評価の続きを**新しいスタック**に渡す（`erg_common::spawn::run_on_new_stack`、
scoped thread + `Builder::stack_size`）。依存は増えない。`Str` は `Arc` ベースで
`Context` は `Sync` なので、scoped thread は評価器が持っている `Context` をそのまま借りられる。

これで「深さ = 1 つのスタックに何段収まるか」が「深さ = こちらが決めた予算」になった。
予算は `CONST_CALL_LIMIT`（1 スタックあたりの tick 数）× `CONST_CALL_STACKS`（スタック数）で、
**ユーザーから見える再帰は 31 段 → 約 1000 段**。超過はクラッシュではなくエラーとして
報告される。これは段 2 が欲しかったもの（予算をこちらで決める）そのものである。

測り方に注意: `print!` して実行すると **CPython 側の `RecursionError`**（既定上限 1000）が
出るので、コンパイル時の畳み込み深さを測っていることにならない。`Deep: {0} = Countdown(N)`
のように singleton 型注釈を付けて `--mode check` する（畳めなければ型エラーになる）。
この方法で測ると変更前は `Countdown(31)` 可 / `Countdown(32)` で `RecursionError`、
変更後は `Countdown(1000)` 可 / `Countdown(1100)` で `RecursionError`
（予算 = `CONST_CALL_LIMIT` 64 × `CONST_CALL_STACKS` 32 = 2048 tick、1 段 2 tick）。

**その過程で見つかったもの: フレームが呼び出し元フレームにぶら下がっていた**。
同一モジュールの呼び出しではフレームの親が `self`（= 直前のフレーム）だったので、
**深さ N の呼び出しは N 個のスコープを複製していた**（O(N²)）。深さ 31 では見えなかったが、
1000 段にすると即座に破綻する。親をモジュールスコープにして解消。
これは段 1 が狙っていた clone 削減の一部でもあり、意味論的にも段 0 の続きである
（const 関数の本体が見るべきなのは定義スコープであって、呼び出し元のフレームではない）。

**速度**（300 束縛のモジュールで `F(N) = if N <= 0, do 0, do N + F(N - 1)` を 30 段、
const 評価部分のみ、release）:

| | 時間 |
| --- | --- |
| 段 0 のみ | 33 ms |
| + D3（`if` 分岐のインライン化） | 19 ms |
| + フレームの親をモジュールスコープに | 16 ms |
| + ラムダのスコープを本体を評価するときだけ作る | 12 ms |

最後の 1 行はレビューで見つけたもの。`eval_const_lambda` はスコープ（= 囲みスコープの clone）を
無条件に作っていたが、本体を評価するのは `LAMBDA_BODY_DEPTH` が 1 以下のときだけなので、
再帰の中では毎回捨てていた。`if` は分岐 2 つでラムダを 2 つ作るので、再帰 1 段につき 2 個。

#### 段 1 の続き（2026-08-28）— これも計画とは違うやり方になった

残っていたのは「1 呼び出しにつきモジュールコンテキストの clone が 1 回」で、計画では
`eval_const_*` 約 20 関数にフレーム引数を通して**借りる**ことにしていた。実際には
**`Context::outer` を `Box` から `Arc` に変える**方が小さく、同じ効果が出た。
フレームは呼び出し先とコピーを**共有**する（呼び出しツリーごとに 1 コピー）。
書き込みは全て `get_mut_outer`（= `Arc::make_mut`、copy-on-write）を通るので、
親が共有されない低下時のスコープスタックは挙動が変わらない。直接 `.outer` に触る箇所は 18 個しかなかった。

| モジュール | 深さ | 前 | 後 |
| --- | --- | --- | --- |
| 300 束縛 | 30 | 6.3 ms | 3.5 ms |
| 300 束縛 | 100 | 29.9 ms | 14.1 ms |
| 1000 束縛 | 30 | 14.0 ms | 2.7 ms |

（`--mode check` の release、同じファイルから const 呼び出しを除いたものとの差、11 回の最小値）
最後の行が要点で、**const 評価の時間がモジュール規模に比例しなくなった**。

**正しさの方も 1 行だった**: フレームの名前が `F` のように**名前空間を持っていなかった**。
`get_mono_type` は「今のスコープが型の名前空間の中か」を `self.name.starts_with(...)` で見るので、
`F` という名前のフレームからはモジュールのどのクラスも辿れず、`C.Base` が
`AttributeError` になっていた。`def_scope`（メモ化キーが既に使っている）から
`<scope>::<subr>` と名付けて解消。エラー表示も `Add` から `<module>::C::Add` になった。

#### `match` の const 化（2026-08-28）

`match` は名前ではなく**特殊形式**である（低下が magic な型を与え、`get_call_t` が横取りする）ので、
const 評価も同じ位置で横取りする（`eval_const_call` の呼び出し先解決の前）。

畳み方は**型からの判断ではなく、生成されるプログラムがすることの再現**にした。各アームは
値を自分の仮引数に束縛し、パターンが脱糖された条件式（codegen が出すのと同じ式）を順に評価する。
最後のアームは条件を評価せずに取る — 実行時もマッチしなければそこへ落ちるからである。

条件が `Bool` に畳めないアームがあると、どのアームが走るか決まらないので**match 全体を畳まない**。
`match True: 1 -> ...` はこれで拒否される（決めるには CPython の `1 == True` を再現することになり、
ここで間違えると値ではなく**型**が嘘をつく）。

#### 未解決

- ~~本体の中で定義された const 関数~~ → 実装済み（2026-09-03、`f5baca0b`）。
  「クロージャか禁止か」という問いが誤りだった: **捕捉ゼロの `Inner(M) = M + 1` でも
  同じ `NameError`** になっていて、原因は二つとも機構側にあった。

  1. const 本体は `ConstBlock` として保存され、その `ConstDef` が識別子しか持たなかった。
     `Inner(M: Int) = M + N` は往復で `Inner = M + N` になる — **パラメータはパーサが
     捨てていた**（const 評価に届く前）。`ConstDef` に `SubrSignature` を持たせて解消
  2. `eval_const_def` が const 定義を全部「値」として扱い、`grow` した直後に本体を
     評価していた。`K = N + 1` には正しく、サブルーチンには正反対（引数が来る前に
     本体を走らせてはいけない）。`ConstSubr` を作るように変更

  **読む先は呼び出し元フレーム**（= 定義したフレーム）。名前は本体の外へ出られないので、
  呼ばれる時点でそのフレームは必ず鎖に乗っている。値に環境を持たせる案は採らなかった:
  `UserConstSubr` は `Eq`/`Hash` を derive していて畳んだ値は `{v}` という型になるため、
  比較で環境を無視すると `Adder(3)` と `Adder(5)` が同じ値になり不変条件が壊れる。
  これは `def_scope: None`（本体スコープは元から `None`）から自動的に従う。

  本体の外へ返す形（`Adder(N) = ... Add`）は従来どおり拒否する。理由も従来と同じで、
  const 呼び出しの結果を呼ぶ経路が未実装。
- ユーザークラスの const メソッドは畳めるようになった（引数側のバグ + フレームの名前空間）。
  修飾なしの `Base`（`C.Base` ではなく）は今も見つからないが、**これは低下側も拒否する**
  ので言語の仕様どおりであり、ギャップではない

---

## 優先順位まとめ

| 順 | フェーズ | 状態 |
| --- | --- | --- |
| 1 | フェーズ 0（差分テスト） | ✅ `41c7c096` |
| 2 | フェーズ 1（再帰ガード） | ✅ `af970054` |
| 3 | フェーズ 2（メソッド・添字） | ✅ `2ade62bd` |
| 4 | フェーズ 3（Ratio / 整数範囲） | ✅ `01dd5a28`, `78217f57` |
| 5 | フェーズ 4（畳み込み不可の区別） | ✅ |
| 6 | フェーズ 5（残りのオペレータ・関数） | ✅ |
| 7 | フェーズ 6（評価器再設計） | ✅ 段 0・D3・メモ化・スタック継ぎ足し（段 2 相当）・段 1（`Arc` 共有）・`match`。段 3 は前提が誤りで対象外 |

フェーズ 6 の段取りは「段 0: スコープを直す → 段 1: フレームのヒープ化 → 段 2: ステップ予算 →
段 3: ループ」を予定していたが、段 1 は `Arc` 共有、段 2 はスタック継ぎ足しという別の形になり、
段 3 は前提が誤っていて対象外になった。詳細は各節の「実装結果」を参照。

### 残っている既知の不一致

`tests/const_diff.rs` の `KNOWN_DIVERGENT` が現状の負債そのもの。**現在は空**。

`reversed` / `zip` / `map` / `filter` / `.keys()` / `.values()` / `.items()` は
定数として畳み込めなくなった（`tests/should_err/stateful_const.er`）ので一覧から外した。
最後まで残っていた Dict の挿入順は `erg_common::Dict` の背骨を `FxHashMap` から
insertion-order 保持の `indexmap`（同じ FxHasher）に替えて解消。等価とハッシュは
順序非依存のまま、`remove` は順序を守るため shift（`O(n)`）。表示・畳み込み・
レコードのフィールド順・エラー一覧の順序が決定的になった。

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
