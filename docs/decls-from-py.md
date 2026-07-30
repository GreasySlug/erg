# アノテーション付き .py からの型宣言生成(`--decls-from-py` / `erg pydecl`)

`.d.er` も `.pyi` も無い Python モジュールを `pyimport` するとき、`.py` ソース内の
型アノテーション(PEP 484)から Erg 宣言をその場で生成し、**型付きモジュール**として
扱う機能です。型推論は行いません(アノテーションの構文変換のみ)。

2つの入口があります:

| 入口 | 用途 |
| ---- | ---- |
| `--decls-from-py` フラグ | コンパイル時にインメモリで自動変換(ファイルは作らない) |
| `erg pydecl` サブコマンド | `.py`/`.pyi` から `.d.er` を明示的に生成(stdout またはファイル) |

- どちらも **opt-in** で、`pydecl` feature 付きビルドが必要
- 実装計画・経緯: [py-to-decl-plan.md](py-to-decl-plan.md)

---

## 1. 使い方

### 1.1 ビルド

```bash
# pydecl feature が必要(無しだとフラグは実質 no-op)
cargo build --features pydecl

# ローカルインストールする場合
cargo install --path . --features "els pydecl"
```

### 1.2 実行(フラグ: インメモリ変換)

```bash
# 型チェックのみ
cargo run --features pydecl -- --mode check --decls-from-py main.er

# コンパイル + 実行
cargo run --features pydecl -- --decls-from-py main.er
```

オプションは**ファイルパスの前**に置くこと(後ろに置くと runtime 引数扱いでエラー)。

### 1.2b サブコマンド: `.d.er` ファイルの生成

```bash
# stdout に出力
erg pydecl mylib.py

# .pyi スタブも変換可能
erg pydecl stub.pyi

# ファイルに書き出す(<dir>/<stem>.d.er)
erg pydecl --output-dir . mylib.py   # → ./mylib.d.er

# パイプ入力(.py として解釈)
cat mylib.py | erg pydecl
```

`mylib.py` と同じ場所に `mylib.d.er` を生成しておけば、以後は**フラグ無しでも**
`.d.er` が最優先で解決されます(§2 の順位 1)。生成した `.d.er` は手で加筆修正
できるので、「自動生成をベースに人手で精度を上げる」運用ができます。
変換できないモジュール(§3.1)の場合は exit 1 でエラーメッセージを出します。

`erg --mode pydecl file.py` という書き方も等価です。

### 1.3 例

`mylib.py`(スタブ無しの普通の Python ソース):

```python
import os

LIMIT = 8

def add(a: int, b: int) -> int:
    return a + b

def helper(x):
    return x * 2

class Point:
    def __init__(self, x: int, y: int) -> None:
        self.x: int = x
        self.y: int = y

    def norm(self) -> float:
        return float(self.x + self.y)
```

`main.er`:

```erg
mod = pyimport "mylib"
print! mod.add(1, 2)     # add: (a: Int, b: Int) -> Int として解決
p = mod.Point(1, 2)      # __call__: (x: Int, y: Int) -> Point
print! p.norm()          # norm: (self: Point) -> Float
print! mod.LIMIT         # LIMIT: Int
print! mod.helper(21)    # helper: (x: Obj) -> Obj(無アノテーションは Obj)
```

フラグありなら `mod.add("a", "b")` は **TypeError**(expected Int)になります。
フラグ無しの場合は従来どおりの挙動です(ローカル `.py` は属性解決不可、
stdlib 等は型なし pyimport の `Obj` フォールバック)。

---

## 2. モジュール解決の順位

`pyimport "mod"` の宣言解決(`resolve_decl_path`)は次の順で試行されます。

| 順位 | ソース | 条件 |
|------|--------|------|
| 1 | `.d.er` | 常時 |
| 2 | `.pyi`(スタブ) | `respect_pyi`(デフォルト有効) |
| 3 | **`.py`(本機能)** | `--decls-from-py` 指定時(または PYTHON_MODE) |
| 4 | 型なし pyimport | 上記すべて失敗時(属性は常に `Obj`) |

---

## 3. 変換ルール

アノテーションの型変換(`int` → `Int`、`Optional[T]` → `T or NoneType` 等)は
`.pyi` 変換と共通です。`.py` モード固有のルール:

| Python コード | 生成される宣言 |
|---------------|----------------|
| `def f(a: int) -> str:` + 本体 | `f: (a: Int) -> Str`(本体は無視) |
| `def f(x):` で本体に `return 値`/`yield` が無い | `f: (x: Obj) -> NoneType` |
| `def f(x):` で値を返す/ジェネレータ | `f: (x: Obj) -> Obj` |
| `X = 100`(リテラル代入) | `X: Int` |
| `X = compute()`(非リテラル代入) | `X: Obj` |
| `a, b = ...` / `a = b = ...` | 各名前を `Obj` |
| `import os` / `import numpy as np` / `from m import f` | `os: Obj` / `np: Obj` / `f: Obj`(末尾に発行) |
| `__init__` 内の `self.x: int = ...` | メンバー `x: Int` |
| `__init__` 内の `self.y = <リテラル>` / `<その他>` | `y: <リテラル型>` / `y: Obj` |
| `try:`/`except:` 内の定義・import | 走査対象(import フォールバックパターン対応) |
| 先頭 `_` の名前 | スキップ(非公開) |

**設計上のポイント**: 公開名は必ず何かしら宣言されます(最低でも `Obj`)。
部分的な宣言だとモジュールが「型付き」になった時点で未宣言属性が
AttributeError になるため、網羅性が正しさの条件です。

### 3.1 変換を中止するケース(untyped フォールバック)

以下を含むモジュールは静的に公開名を列挙できないため、変換を**中止**して
従来の型なし pyimport(空コンテキスト → 属性は `Obj`)に落ちます。

- `from x import *`(スター import)
- モジュールレベルの `def __getattr__(name):`(動的属性)
- `.py` のパースエラー

---

## 4. 制限事項

- **型推論はしない**: 無アノテーション部分は `Obj`。より正確な型が欲しい場合は
  `.d.er` / `.pyi` を書くか、`--use-pylyzer`(外部 pylyzer による推論)を使う。
- **デコレータはシグネチャに反映されない**: `@contextmanager` 等で実際の型が
  変わっても、書かれたアノテーションをそのまま信頼する。
- **型エイリアスは値としては `Obj`**: `Num = Union[int, float]` はアノテーション内では
  インライン展開されるが、`mod.Num` 自体は `Obj` 宣言になる。
- **TypeVar 付き変数宣言は `Obj`**(変数宣言は量化できないため)。
- **`@dataclass` の自動 `__init__` は未対応**: `__call__: (*args: Obj) -> Cls`
  フォールバックになる(v2 候補)。
- 純粋性は検証されない: 生成される関数宣言はすべて純粋(`->`)として扱われる
  (`.d.er`/`.pyi` と同じ信頼モデル)。

---

## 5. メンテナンス

### 5.1 関連コード

| 役割 | 場所 |
|------|------|
| 変換本体(`convert_py_to_decl`, `SrcKind::Py`) | `crates/erg_pydecl/src/lib.rs` |
| フラグ定義・CLI(`decls_from_py`)・`ErgMode::PyDecl` | `crates/erg_common/config.rs` |
| `.py` の宣言ソース解決 | `crates/erg_common/io.rs`(`resolve_decl_path`) |
| 変換フック・declare モード判定 | `crates/erg_compiler/build_package.rs`(`convert_py`, `parse()`) |
| `.py` 宣言シンボルの全公開化 | `crates/erg_compiler/context/instantiate_spec.rs`(`instantiate_vis_modifier`) |
| `erg pydecl` サブコマンド本体 | `src/pydecl.rs`(`erg::pydecl::run`、main.rs からディスパッチ) |
| ヘルプ(COMMAND 一覧) | `crates/erg_common/help_messages.rs` |

### 5.2 テスト

```bash
# 変換の単体テスト(py モードは `py_` プレフィックス)
cargo test -p erg_pydecl

# 手動確認
cargo run --features pydecl -- --mode check --decls-from-py path/to/main.er
```

### 5.3 変更時の注意

1. **網羅性を壊さない**: `.py` モードで新しい構文を「スキップ」する変更は、
   その名前が AttributeError になることを意味する。迷ったら `Obj` で宣言する。
2. **bail 条件**: 動的に名前を作る構文を新たに見つけたら、`has_dynamic_exports`
   に追加して untyped フォールバックへ逃がす。
3. **`.pyi` モードとの分岐**: 挙動差はすべて `Converter::is_py()` でゲートする。
   `.pyi` の出力はゴールデンテストで固定されているため無断で変えない。
