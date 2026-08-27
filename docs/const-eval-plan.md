# コンパイル時評価 (CTFE) 改善計画

## 概要

`<<`/`>>`/開区間/`~` のオペレータと `int`/`nat`/`float`/`round`/`ord`/`chr`/`hex`/`oct`/`bin`/`pow`/`divmod`/`sorted` の組み込み関数を
コンパイル時評価できるようにした（commits `7a1884f7`, `337bf9a1`, `7a59c473`, `d79fa7ad`）後の、
残ったギャップと改善方針をまとめる。

すべてのギャップはこのツリーで再現を確認済み。各項目に再現コードを添えている。

**進捗**: フェーズ 0〜5 は実装済み。フェーズ 6 は実測とメモ化のみ着手、本体は未着手。

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

- 無名ラムダが const 化できない（小文字パラメータは非 const、大文字は `NameError`）。
  結果 `map` / `filter` には名前付き const 関数しか渡せない
- 複数チャンクの const 関数本体で仮引数が見えない（`Tmp = N + 1` の行で `N is not defined`）
- 可変長引数の const 関数は `FeatureError: const parameters`
  ([eval.rs:636](../crates/erg_compiler/context/eval.rs#L636))
- `eval_const_chunk` ([eval.rs:1584](../crates/erg_compiler/context/eval.rs#L1584)) に
  `ClassDef` / `PatchDef` / `Methods` / `ReDef` / `Compound` が無い（コード中に TODO あり）

---

## フェーズ 6: 評価器の再設計（着手・大部分は未着手）

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
const 関数は純粋（原則 4）で、**const 名はシャドウイングできない**（`G` を二重に束縛すると
`AssignError`）ので、「定義モジュール + 名前 + 引数」が結果を決める。

キャッシュしない条件:

- **`sig_t` が `Quantified`** — 本体が呼び出し元の型変数を読むので、引数だけでは決まらない
- **合成名（`<lambda>` など）** — `if` の分岐は引数を取らず、書かれた場所のスコープを読む。
  最初これを見落とし、`Fib(3)` の分岐の値が `Fib(4)` の分岐に返ってきた（差分テストではなく
  `should_ok` の singleton 型注釈が捕らえた）
- **エラーになった呼び出し** — 診断は周囲の文脈に依存する

ELS のためにモジュール再コンパイル時は `SharedCompilerResource::clear` でそのモジュール分を捨てる。

### 残り（未着手・要判断）

**ステップ予算は今入れても意味がない**。ホストスタック上で再帰している限り、
深い再帰を止めるのは依然として深さガードであり、ステップ数では守れない。
「予算をステップ数で数える」は**明示フレームのインタプリタができた後**に初めて成立する。
順序は「フレームのヒープ化 → ステップ予算 → ループ」であって、逆順には入れられない。

`Context` clone の除去も同じところに繋がる。呼び出しごとに親を deep copy しているのは
`outer: Option<Box<Context>>` が所有権を持つためで、共有参照にするか、
本体が参照する自由変数だけを捕獲する形にするかは、インタプリタの設計と一体で決めるべき。

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
| 7 | フェーズ 6（評価器再設計） | 着手（メモ化のみ）・本体は設計判断が要る |

### 残っている既知の不一致

`tests/const_diff.rs` の `KNOWN_DIVERGENT` が現状の負債そのもの。

| 式 | 内容 | フェーズ |
| --- | --- | --- |
| `{"a": 1, "b": 2}` | `ValueObj::Dict` がハッシュマップなので挿入順を保たない | 5 |

`reversed` / `zip` / `map` / `filter` / `.keys()` / `.values()` / `.items()` は
定数として畳み込めなくなった（`tests/should_err/stateful_const.er`）ので一覧から外した。
Dict の順序は**表示だけ**の差（dict の等価比較は順序非依存で、順序が漏れるビューは畳み込まない）。
直すには `erg_common::Dict` を挿入順保持にする必要があり、コンパイラ全体が使う基盤データ構造なので
独立した判断として扱う。

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
