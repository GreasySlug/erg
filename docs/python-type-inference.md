# Python の型を推定する機構

Erg では、`pyimport` で取り込んだ Python モジュールについて、次の 2 通りで型を扱います。

1. **型付き**: `.d.er` または `.pyi` スタブがある場合 → 通常の Erg 型解決
2. **型なし（untyped）**: スタブがない場合 → 「Python の型を推定する機構」により `Obj` として扱い、属性アクセス・呼び出しを許可する

ここでは **型なし pyimport** で使われる「Python の型を推定する機構」の動きと、オプションで使える **.pyi スタブによる型推定** のやり方をまとめます。

---

## 1. 型なし pyimport（untyped pyimport）の動き

`.d.er` も `.pyi` もないモジュールを `pyimport "mod"` すると、そのモジュールは **型なし** として登録され、次のルールで型が決まります。

### 1.1 モジュール自体の型

- `pyimport "wave"` などで得た値の型は **PyModule 型**（中身は空のモジュールコンテキスト）として扱われます。

### 1.2 属性アクセス（`module.attr`）

- 型なし PyModule に対して `module.attr` を行うと、**常に `Obj` 型** が返ります。
- 実装上のポイント: 通常の型解決の**後**に、「PyModule かつコンテキストが空なら `Obj` を返す」フォールバックが入っています（Erg の型推論を乱さないため）。

### 1.3 チェーンされた属性アクセス（`module.attr.subattr`）

- `Obj` 型の値に対してさらに属性アクセスすると、やはり **`Obj`** が返ります。
- これにより、型なしモジュールから得た値（例: `wave.Error`）のさらにその先（例: `wave.Error.__name__`）もコンパイル可能になります。

### 1.4 型なし Python 値の「呼び出し」（`obj(...)`）

- 型が `Obj` の式を `obj(...)` のように呼び出すと、**`(..Obj) -> Obj`** のサブルーチンとして扱われます。
- 型なしモジュールの関数・メソッドを「とにかく呼べる」ようにするためのフォールバックです。

### 1.5 型なし PyModule のメソッド呼び出し（`module.method(...)`）

- 型なし PyModule に対して `module.some_method(...)` のようにメソッド呼び出しをすると、戻り値は **`Obj`** として扱われます。

### 1.6 モジュール解決のフォールバック

- `pyimport "mod"` の評価時、`.d.er` が見つからない場合は **`.py` ファイル** を `resolve_py` で直接解決します（標準ライブラリ・site-packages 等を検索）。
- 見つかった `.py` に対応するモジュールは、空のモジュールコンテキストとして登録され、上記の「型なし」のルールが適用されます。

---

## 2. 使い方の例（型なし pyimport）

```erg
# 型スタブ（.d.er / .pyi）のないモジュールの例
wave = pyimport "wave"

# 属性アクセス → Obj
x = wave.Error
print! x

# チェーンアクセス → いずれも Obj
y = wave.Error.__name__
print! y

# 属性を変数に束縛（呼ばない）
f = wave.open
print! f
```

- `wave` は型なし PyModule → `wave.Error` は `Obj`、`wave.Error.__name__` も `Obj`。
- `wave.open` も `Obj`（呼び出し可能として `(..Obj) -> Obj` と扱われる）。

---

## 3. .pyi スタブによる型推定（オプション）

`.d.er` がなくても、**Python の型スタブ（.pyi）** があれば、その内容を Erg の型として利用できます。これも「Python の型を推定する機構」の一環です。

### 3.1 有効化

- 設定 **`respect_pyi`** で制御します（デフォルト: **true**）。
- `true` のとき、`resolve_decl_path` は次の順で解決を試みます:
  1. `.d.er`
  2. **`.pyi`**（`resolve_pyi`）
  3. `.py`（`resolve_py`。PYTHON_MODE 時、または `--decls-from-py` 指定時 —
     後者はアノテーションから宣言を生成する。詳細は [decls-from-py.md](decls-from-py.md)）

### 3.2 .pyi の解決先

- プロジェクト内のローカル `.pyi`
- `site-packages` 内の `{path}.pyi` または `{path}/__init__.pyi`
- PEP 561 の **`{module}-stubs`**（`site-packages/{module}-stubs/__init__.pyi`）

### 3.3 .pyi の変換（preprocess_pyi）

`.pyi` のソースはそのままでは Erg の文法ではないため、読み込み時に **preprocess_pyi** で Erg 互換の型注釈に変換してからパースします。

| .pyi の記述例 | 変換後（Erg 用） |
|---------------|------------------|
| `def func(a: int, b: int) -> int: ...` | `func: (a: int, b: int) -> int` |
| `def proc() -> None: ...` | `proc: () -> NoneType` |
| `x: int = ...` | `x: int` |
| `import`, `from ... import` | スキップ |
| `@decorator` | スキップ |
| `class C: ...` とその本体 | スキップ |
| `...`, `pass` | スキップ |
| 型注釈のない代入（`T = TypeVar(...)` など） | スキップ |

- 戻り値の `None` は `NoneType` に変換されます。
- これにより、.pyi 由来のモジュールは「型付き」として扱われ、通常の Erg の型解決・推論が効きます。

### 3.4 無効にしたい場合

- `respect_pyi: false` にすると、`.pyi` は使われず、.d.er がなければ型なし pyimport の挙動（上記 §1）になります。

---

## 4. まとめ

| 状況 | モジュールの型 | 属性アクセス | メソッド/呼び出し |
|------|----------------|-------------|-------------------|
| .d.er がある | 型付き（.d.er の定義） | 通常の型解決 | 通常の型解決 |
| .pyi のみ（`respect_pyi: true`） | 型付き（.pyi から変換） | 通常の型解決 | 通常の型解決 |
| .py のみ + `--decls-from-py` | 型付き（.py のアノテーションから変換、無注釈は `Obj`） | 通常の型解決 | 通常の型解決 |
| どれもない（型なし pyimport） | PyModule（空） | 常に `Obj` | 戻り値 `Obj` / callable は `(..Obj) -> Obj` |

「Python の型を推定する機構」は、

- **型なし**のとき: モジュールとその属性・呼び出しをすべて `Obj`（および PyModule）にフォールバックし、コンパイルを通す。
- **.pyi あり**のとき: .pyi を Erg 用に変換して型情報として使い、より正確な型推定を行う。

という 2 段構えになっています。

---

## 5. 実行方法

### 5.1 型なし pyimport を使う Erg コードの実行

通常の Erg 実行と同じです。Python インタープリタ（3.7–3.11）が利用可能である必要があります。

```bash
# ビルド（初回または変更後）
cargo build

# 実行（コンパイル＋Python で実行）
cargo run -- tests/should_ok/untyped_pyimport.er

# 型チェックのみ（実行しない）
cargo run -- --mode check tests/should_ok/untyped_pyimport.er

# 特定の Python コマンドを指定する場合
cargo run -- --py-command python3.11 -- tests/should_ok/untyped_pyimport.er
```

- `wave` など標準ライブラリモジュールは、その Python 環境に含まれている必要があります。
- `.d.er` / `.pyi` が無いモジュールは型なしとして扱われ、上記 §1 のルールでコンパイルされます。

### 5.2 .pyi を使う場合

- `respect_pyi` はデフォルトで **true** です（`ErgConfig` の既定値）。
- 現状、`respect_pyi` は CLI オプションでは変更できず、コード上で `ErgConfig` を組み立てる場合にのみ指定できます。
- プロジェクト内や site-packages に `.pyi` を置けば、`.d.er` が無くてもそのモジュールは型付きとして解決されます（§3 参照）。

---

## 6. メンテナンス方法

### 6.1 関連するコードの場所

| 役割 | ファイル | 主な内容 |
|------|----------|----------|
| 型なし PyModule の属性 → Obj | `crates/erg_compiler/context/inquire.rs` | `get_attr_info`: 「PyModule かつコンテキストが空なら Obj」フォールバック（約 849–861 行目）、「obj が Obj なら任意属性で Obj」フォールバック（約 926–941 行目） |
| 型なし Python 値の呼び出し (..Obj)->Obj | 同上 | `search_callee_info` / `search_callee_info_without_args`: Obj を callable として扱う（約 1192–1205, 1255–1266 行目） |
| 型なし PyModule のメソッド呼び出し → Obj | 同上 | 約 1506–1526 行目 |
| 空モジュールコンテキストの登録 | `crates/erg_compiler/context/register.rs` | `resolve_py` で .py を解決したあと、空の `ModuleContext` を登録（約 3142–3160 行目） |
| pyimport 時のモジュール解決 | `crates/erg_common/io.rs` | `resolve_decl_path`: .d.er → .pyi（`respect_pyi`）→ .py（PYTHON_MODE）の順（約 779–792 行目） |
| .py の解決（型なし用） | `crates/erg_common/io.rs` | `resolve_py`（約 525 行目付近） |
| pyimport の eval で .py フォールバック | `crates/erg_compiler/context/initialize/const_func.rs` | pyimport の定数関数で `resolve_decl_path` 失敗時に `resolve_py`（約 1569–1572 行目） |
| 型なし pyimport 時のパス扱い | `crates/erg_compiler/link_hir.rs` | `.d.er` が無いときのモジュールパス処理（約 519–522 行目） |
| .pyi の変換 | `crates/erg_compiler/build_package.rs` | `preprocess_pyi`（約 47–186 行目）、`parse()` で .pyi を変換してからパース（約 955–959 行目） |
| 設定 | `crates/erg_common/config.rs` | `respect_pyi` フィールド（デフォルト true）（約 190, 222 行目） |

### 6.2 テストの実行

```bash
# 全テスト（推奨: large_thread）
cargo test --features large_thread

# pyimport 関連の実行テスト
cargo test exec_pyimport_test exec_pyimport exec_pyimport_err --features large_thread

# 型なし pyimport の例（このファイルは should_ok なのでコンパイル・実行が通る想定）
cargo run -- tests/should_ok/untyped_pyimport.er
```

- `tests/should_ok/untyped_pyimport.er` は型なし pyimport の挙動（属性・チェーン・変数束縛）を確認するためのサンプルです。
- `tests/should_ok/pyimport.er` は型付き（.d.er あり）モジュールも含む統合例です。

### 6.3 .pyi 変換（preprocess_pyi）のテスト

`build_package.rs` 内に `preprocess_pyi` の単体テストがあります。

```bash
cargo test -p erg_compiler preprocess_pyi -- --nocapture
```

- テスト名は `test_preprocess_pyi_*`（例: `test_preprocess_pyi_def_single_line`, `test_preprocess_pyi_skip_untyped_assignments` など）。
- .pyi の変換ルールを変えた場合は、ここで期待値を合わせて更新してください。

### 6.4 変更時の注意点

1. **フォールバックの順序**  
   `inquire.rs` では、型なし用のフォールバックは「通常の型解決の後」に置かれています。ここを前に出すと Erg 側の型推論に影響するため、順序を変える場合は挙動の確認が必要です。

2. **Obj の意図**  
   型なし Python 値はすべて `Obj` で受け流す設計です。より細かい型を付けたい場合は、.d.er または .pyi で型を定義する必要があります。

3. **respect_pyi**  
   `respect_pyi` を false にすると .pyi を参照しません。型なし pyimport のテストや、.pyi 無しの環境を再現したい場合にコード上で設定を変更します。
