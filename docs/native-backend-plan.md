# Erg ネイティブバイナリ生成 実装計画

## 概要

ErgにLLVMバックエンドを追加し、Pythonランタイムなしで実行可能なネイティブバイナリを生成する。

**選択されたアプローチ:**
- バックエンド: LLVM (via inkwell)
- メモリ管理: 精密GC
- Python相互運用: 段階的に実装

---

## 現在のアーキテクチャ

```
Source → Parser → AST → Lowering → HIR → PyCodeGenerator → Python bytecode
                              ↓
                    型チェック・所有権チェック・副作用チェック完了
```

**重要な点**: HIRは型情報を完全に持つ安定した中間表現。新しいバックエンドはHIRから生成する。

---

## 新しいアーキテクチャ

```
                                    ┌─→ PyCodeGenerator → Python bytecode
Source → Parser → AST → HIR ────────┤
                                    └─→ LLVMCodeGenerator → LLVM IR → Native binary
```

---

## フェーズ1: 基盤構築 (4週間)

### 1.1 新しいクレート作成

**新規ファイル:**
```
crates/erg_native/
├── Cargo.toml
├── lib.rs
├── backend.rs          # Backend trait定義
├── llvm/
│   ├── mod.rs
│   ├── codegen.rs      # LLVM IR生成
│   ├── types.rs        # 型マッピング
│   └── intrinsics.rs   # 組み込み関数
└── runtime/
    ├── mod.rs
    ├── gc.rs           # 精密GC実装
    └── builtins.rs     # 組み込み型の実装
```

### 1.2 Cargo.toml設定

```toml
[package]
name = "erg_native"

[features]
default = ["llvm"]
llvm = ["dep:inkwell"]
python-ffi = ["dep:pyo3"]

[dependencies]
erg_common = { workspace = true }
erg_compiler = { workspace = true }
inkwell = { version = "0.4", features = ["llvm17-0"], optional = true }
pyo3 = { version = "0.20", optional = true }
```

### 1.3 Backend trait定義

```rust
// crates/erg_native/backend.rs
pub trait NativeBackend {
    fn emit(&mut self, hir: HIR) -> Result<(), NativeError>;
    fn emit_object(&self, path: &Path) -> Result<(), NativeError>;
    fn emit_executable(&self, path: &Path) -> Result<(), NativeError>;
}
```

### 1.4 設定の拡張

**変更ファイル:** `crates/erg_common/config.rs`

- `ErgMode::NativeCompile` を追加
- `NativeTarget` enum を追加 (x86_64, aarch64, wasm32)
- CLI引数 `--native` を追加

---

## フェーズ2: 型マッピング (4週間)

### 2.1 Erg型 → LLVM型のマッピング

**参照ファイル:** `crates/erg_compiler/ty/mod.rs` (Type enum定義)

| Erg型 | LLVM型 | メモリレイアウト |
|-------|--------|------------------|
| Int | i64 | 64-bit整数 |
| Nat | i64 | 64-bit整数 (非負) |
| Float | double | 64-bit浮動小数点 |
| Bool | i1 | 1-bit |
| Str | %ErgStr* | { ptr, len, cap } |
| List[T] | %ErgList* | { ptr, len, cap, elem_type } |
| Record | %RecordName | 構造体 |
| Subr | %Closure* | { fn_ptr, env_ptr } |
| Ref[T] | T* | ポインタ |

### 2.2 オブジェクトヘッダ (GC用)

```llvm
%ObjectHeader = type {
  i64,           ; GC mark word
  i64,           ; type tag
  i64            ; size
}

%ErgStr = type {
  %ObjectHeader,
  i8*,           ; data pointer
  i64,           ; length
  i64            ; capacity
}
```

---

## フェーズ3: コード生成 (8週間)

### 3.1 HIR式のコード生成

**参照ファイル:**
- `crates/erg_compiler/hir.rs` (HIR Expr定義)
- `crates/erg_compiler/codegen.rs` (PyCodeGenerator参考)

**実装順序:**
1. リテラル (Int, Float, Bool, Str)
2. 二項演算 (BinOp)
3. 単項演算 (UnaryOp)
4. 変数アクセス (Accessor)
5. 関数呼び出し (Call)
6. 関数定義 (Def, Lambda)
7. 制御フロー (if式、match式)
8. コレクション (List, Tuple, Dict, Set)
9. クラス定義 (ClassDef)
10. クロージャ

### 3.2 関数コンパイル

```rust
fn emit_def(&mut self, def: &Def) -> Result<FunctionValue, NativeError> {
    // 1. 関数シグネチャからLLVM関数型を作成
    // 2. 関数本体のコードを生成
    // 3. クロージャの場合は環境キャプチャを処理
    // 4. GCセーフポイントを挿入
}
```

### 3.3 制御フロー

- if式: LLVM basic blockとphi nodeを使用
- match式: switch命令またはネストしたbranchに変換
- ループ: while/forをLLVMループ構造に変換

---

## フェーズ4: 精密GC実装 (6週間)

### 4.1 スタックマップ生成

- 各GCセーフポイントでルート変数の位置を記録
- LLVM Statepoint intrinsicsを使用

```rust
fn emit_gc_safepoint(&mut self) {
    // @llvm.experimental.gc.statepoint を使用
    // スタック上のGCルートを記録
}
```

### 4.2 GCランタイム

**新規ファイル:** `crates/erg_native/runtime/gc.rs`

```rust
pub struct GC {
    heap: Vec<u8>,
    roots: Vec<*mut ObjectHeader>,
    // ...
}

impl GC {
    pub fn allocate(&mut self, size: usize, type_tag: u64) -> *mut ObjectHeader;
    pub fn collect(&mut self);
    fn mark(&mut self);
    fn sweep(&mut self);
}
```

### 4.3 オブジェクトタイプ情報

- 各型に対してGCがフィールドをトレースできる型情報を生成
- 型タグからポインタフィールドの位置を特定

---

## フェーズ5: ランタイムライブラリ (4週間)

### 5.1 組み込み型の実装

**新規ファイル:** `crates/erg_native/runtime/builtins.rs`

```rust
// Str操作
pub extern "C" fn erg_str_concat(a: *const ErgStr, b: *const ErgStr) -> *mut ErgStr;
pub extern "C" fn erg_str_len(s: *const ErgStr) -> i64;

// List操作
pub extern "C" fn erg_list_new(cap: i64) -> *mut ErgList;
pub extern "C" fn erg_list_push(list: *mut ErgList, elem: ErgValue);
pub extern "C" fn erg_list_get(list: *const ErgList, idx: i64) -> ErgValue;

// I/O
pub extern "C" fn erg_print(val: ErgValue);
pub extern "C" fn erg_input() -> *mut ErgStr;
```

### 5.2 panic/エラーハンドリング

- Ergのassertエラーをネイティブpanic に変換
- スタックトレースの生成

---

## フェーズ6: Python相互運用 (4週間)

### 6.1 Python FFIレイヤー

**新規ファイル:** `crates/erg_native/python_ffi/mod.rs`

```rust
use pyo3::prelude::*;

pub struct PythonBridge {
    py: Python<'static>,
}

impl PythonBridge {
    pub fn call(&self, module: &str, func: &str, args: Vec<ErgValue>) -> ErgValue;
    pub fn erg_to_py(&self, val: ErgValue) -> PyObject;
    pub fn py_to_erg(&self, obj: PyObject) -> ErgValue;
}
```

### 6.2 ハイブリッドコンパイル

- Pythonライブラリを使用するコードパスを検出
- 必要な場合のみPythonランタイムを初期化
- 値変換のオーバーヘッドを最小化

---

## フェーズ7: 統合とテスト (4週間)

### 7.1 CLI統合

**変更ファイル:** `src/main.rs`

```rust
ErgMode::NativeCompile => {
    let compiler = NativeCompiler::new(&cfg);
    let hir = compiler.build_hir()?;
    let mut backend = LLVMBackend::new(&cfg);
    backend.emit(hir)?;
    backend.emit_executable(&output_path)?;
}
```

### 7.2 テスト戦略

1. **ユニットテスト**: 各HIR式の正しいLLVM IR生成
2. **統合テスト**: サンプルプログラムのコンパイル・実行
3. **比較テスト**: Pythonバイトコード版との出力比較
4. **ベンチマーク**: 実行速度の測定

---

## 変更対象ファイル一覧

### 新規作成

| ファイル | 目的 |
|----------|------|
| `crates/erg_native/Cargo.toml` | 新クレート設定 |
| `crates/erg_native/lib.rs` | モジュールエクスポート |
| `crates/erg_native/backend.rs` | Backend trait |
| `crates/erg_native/llvm/mod.rs` | LLVMバックエンドモジュール |
| `crates/erg_native/llvm/codegen.rs` | LLVM IR生成 |
| `crates/erg_native/llvm/types.rs` | 型マッピング |
| `crates/erg_native/llvm/intrinsics.rs` | 組み込み関数 |
| `crates/erg_native/runtime/mod.rs` | ランタイムモジュール |
| `crates/erg_native/runtime/gc.rs` | 精密GC |
| `crates/erg_native/runtime/builtins.rs` | 組み込み型実装 |
| `crates/erg_native/python_ffi/mod.rs` | Python FFI |

### 変更

| ファイル | 変更内容 |
|----------|----------|
| `Cargo.toml` | ワークスペースにerg_native追加 |
| `crates/erg_common/config.rs` | NativeCompileモード追加 |
| `src/main.rs` | NativeCompileモードのハンドリング |

---

## 検証方法

### フェーズごとの検証

1. **フェーズ1完了**: `cargo build -p erg_native` が成功
2. **フェーズ2完了**: 型マッピングのユニットテストがパス
3. **フェーズ3完了**: 基本的なHello Worldプログラムがコンパイル・実行可能
4. **フェーズ4完了**: GCを使うプログラムがメモリリークなく動作
5. **フェーズ5完了**: 文字列・リスト操作を含むプログラムが動作
6. **フェーズ6完了**: Pythonライブラリを呼び出すプログラムが動作
7. **フェーズ7完了**: 全テストスイートがパス

### 最終検証

```bash
# ネイティブコンパイル
cargo run -- --native examples/hello_world.er -o hello

# 実行 (Pythonランタイム不要)
./hello
```

---

## リスクと対策

| リスク | 対策 |
|--------|------|
| LLVMビルドの複雑さ | inkwellのプリビルドバイナリを使用、feature flagで分離 |
| 精密GCの複雑さ | 最初は保守的GCで実装、後で精密GCに移行も検討 |
| クロージャの難しさ | 単純な関数から実装、段階的にクロージャ対応 |
| Python FFIのオーバーヘッド | キャッシュ、遅延初期化で最適化 |

---

## 推定スケジュール

| フェーズ | 期間 | マイルストーン |
|---------|------|---------------|
| 1. 基盤構築 | 4週間 | インフラ完成 |
| 2. 型マッピング | 4週間 | 全型がマッピング完了 |
| 3. コード生成 | 8週間 | 基本プログラムがコンパイル可能 |
| 4. 精密GC | 6週間 | GC動作 |
| 5. ランタイム | 4週間 | 組み込み型サポート |
| 6. Python FFI | 4週間 | Python相互運用 |
| 7. 統合・テスト | 4週間 | 本番利用可能 |
| **合計** | **約34週間** | |
