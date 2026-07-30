# アノテーション付き .py → .d.er 宣言生成 実装計画

ブランチ: `feature/py-to-decl`

## ゴール

`.d.er` / `.pyi` スタブのない Python モジュールでも、`.py` ソース内の型アノテーション
(PEP 484/561 インライン型)から Erg 宣言を生成し、型付き pyimport として扱えるようにする。

- 型推論はしない(pylyzer の領分)。アノテーションの**構文変換**のみ。
- 無アノテーションのシンボルは `Obj` で網羅的に宣言する
  (部分的な宣言はモジュールを「型付き」にしてしまい、未宣言属性がエラーになるため)。
- 動的エクスポート(`from x import *`、モジュールレベル `__getattr__`)を含む
  モジュールは変換を**中止**し、従来の型なし pyimport(空コンテキスト → Obj フォールバック)に落とす。

## 現状の経路(変更前)

```text
pyimport "mod"
  └─ resolve_decl_path (io.rs:740)
       1. .d.er
       2. .pyi (respect_pyi, default true) → parse() で convert_pyi → declare モード
       3. .py (PYTHON_MODE のみ = pylyzer バックエンド用)
  └─ 解決失敗 → resolve_py → 空 ModuleContext 登録 (register.rs) → 属性は常に Obj
```

## 変更点

### 1. erg_pydecl: `convert_py_to_decl` 追加(本体)

`Converter` に `SrcKind { Pyi, Py }` を持たせ、既存ロジックを共有。Py モードの差分:

| 項目 | Pyi モード(現状維持) | Py モード |
| ---- | ---- | ---- |
| 戻り値アノテーションなし | `NoneType` | 本体を走査: `return <expr>`/`yield` が無ければ `NoneType`、あれば `Obj` |
| 無アノテーション代入(非リテラル) | スキップ | `name: Obj` を発行 |
| import で束縛される公開名 | スキップ | 末尾で `name: Obj` を発行(`import os.path`→`os`、`as` 対応) |
| `__init__` 内の `self.x = ...` | (stub に存在しない) | AnnAssign→型変換、リテラル→リテラル型、他→`Obj` でメンバー化 |
| `try:` ブロック | 走査しない | body/handlers/orelse/finalbody を走査(import フォールバック対策) |
| `from x import *` / `__getattr__` | — | `ConvertError` で中止(呼び出し側が型なしへフォールバック) |
| タプル/多重代入ターゲット | スキップ | 各 Name に `Obj` を発行 |

### 2. ErgConfig: `decls_from_py` フラグ(default: **false**、opt-in)

- CLI: `--decls-from-py`
- default off の理由: 既存の型なし pyimport テスト・挙動を変えない。実地評価後に default on を検討。

### 3. io.rs `resolve_decl_path`

`.py` 解決の条件を `PYTHON_MODE` → `PYTHON_MODE || cfg.decls_from_py` に拡張。

### 4. build_package.rs

- `parse()`: 拡張子 `py` かつ `decls_from_py` かつ非 PYTHON_MODE なら `convert_py_to_decl`。
  変換失敗(bail)時は空宣言 → 空コンテキスト → 従来どおり Obj フォールバックが効く。
- declare モード判定(~L1118)に同条件で `.py` を追加。
- `pydecl` feature 無効時は変換せず(フラグは実質 no-op、警告を出す)。

### 5. テスト

- erg_pydecl 単体テスト: 上表の各ポリシー + bail 条件。
- 手動確認: フィクスチャ `.py` + `--decls-from-py` で型エラーが正しく出る/出ないこと。
- 既存テスト(untyped_pyimport 等)がフラグ off で無変化なこと。

## 実装結果 (2026-07-29)

計画どおり実装完了 + 計画時に見えていなかった修正が1件:

- **可視性**: `instantiate_vis_modifier` (instantiate_spec.rs) が `.pyi` のみ全公開に
  していたため、`.py` 宣言モジュールのシンボルが private 扱いになった。
  `ext == "py" && cfg.decls_from_py` を条件に追加して解決。
- **変更ファイル**: erg_pydecl/src/lib.rs(+py モードテスト11件)、erg_common/config.rs、
  erg_common/io.rs、erg_compiler/build_package.rs、erg_compiler/context/instantiate_spec.rs
- **検証済み**: フィクスチャ .py に対し `--decls-from-py` で
  型付き解決(`helper: (x: Obj) -> Obj` 等)・誤引数の TypeError 検出・実行成功を確認。
  フラグ無しの挙動はコード上完全にゲートされており無変化
  (ローカル .py の pyimport が AttributeError になるのは変更前からの既存挙動。
  stdlib の untyped_pyimport.er は従来どおり成功)。
- **既知の無関係な既存問題**: `exec_pyimport`(examples/pyimport.er, exit 111 期待)は
  変更前のコードでも失敗する環境依存の既存失敗。erg_common/error.rs 等の clippy 警告も
  ローカルの新しい clippy (1.97) による既存 lint。

## 非目標(明示的にやらないこと)

- 無アノテーションコードの型推論(pylyzer 委譲を維持)
- `@dataclass` からの `__init__` 合成、`__all__` の尊重(v2 候補)
- `.d.er` ファイルのディスク書き出し(コンパイラ経路はインメモリ。
  ファイル生成は将来 `erg_pydecl` に薄い bin を足して対応)
