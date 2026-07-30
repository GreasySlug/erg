# Smart Enter REPL Implementation

Python 3.13の`maybe_accept`動作をErgのREPLに実装。Enterキーでコードの完全性を自動判定し、完全なら実行、不完全なら改行を挿入する。

## 動作

| キー | 動作 |
|------|------|
| **Enter** | コードが完全なら実行、不完全なら改行挿入 |
| **Ctrl+Enter** | 常に実行（従来通り） |

## 環境変数

- `ERG_BASIC_REPL=1`: 従来動作に戻す（Enter=改行、Ctrl+Enter=実行）

## アーキテクチャ

### 課題
- `erg_common`（stdin.rs）は`erg_parser`に依存できない
- パーサーを使った完全性チェックが必要

### 解決策
コールバック/クロージャパターンを採用。`erg_common`にコールバック型を定義し、上位クレートから注入。

```
erg_common (stdin.rs)          erg_parser (parse.rs)
    │                               │
    │  CompletenessChecker          │  check_code_completeness()
    │  ◄───────────────────────────┤
    │                               │
    ▼                               │
Runnable trait (traits.rs)          │
    │                               │
    │  completeness_checker()       │
    ▼                               │
DummyVM (dummy.rs)                  │
    │                               │
    └──────────────────────────────►┘
         Box::new(check_code_completeness)
```

## 変更ファイル

### crates/erg_common/stdin.rs

```rust
/// コード完全性の判定結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeCompleteness {
    Complete,      // 実行可能
    Incomplete,    // 継続入力が必要
    SyntaxError,   // 構文エラー（実行してエラー表示）
}

/// 完全性チェック用コールバック型
pub type CompletenessChecker = Box<dyn Fn(&str) -> CodeCompleteness + Send + Sync>;
```

- `StdinReader`に`completeness_checker`フィールド追加
- `GlobalStdin`に`set_completeness_checker()`メソッド追加
- Enterキー処理でチェッカーを呼び出し、結果に応じて実行または改行

### crates/erg_parser/parse.rs

```rust
pub fn check_code_completeness(src: &str) -> CodeCompleteness
```

完全性判定ロジック：
1. 括弧のバランスチェック（ヒューリスティック）
2. パーサーによるコード解析
3. `ExpectNextLine`エラーで不完全と判定
4. 構文エラーは実行してエラー表示

### crates/erg_parser/error.rs

- `LexError`に`core()`メソッド追加（エラー種別取得用）

### crates/erg_common/traits.rs

```rust
/// Runnableトレイトに追加
fn completeness_checker(&self) -> Option<CompletenessChecker> {
    None  // デフォルトはNone
}
```

- `run()`メソッドでチェッカーを登録

### src/dummy.rs

```rust
fn completeness_checker(&self) -> Option<CompletenessChecker> {
    if std::env::var("ERG_BASIC_REPL").is_ok() {
        return None;
    }
    Some(Box::new(erg_parser::parse::check_code_completeness))
}
```

## 完全性判定の詳細

### 完全と判定されるケース
- `x = 1` - 単純な代入
- `1 + 2` - 式
- `print! "hello"` - 関数呼び出し
- 空文字列、空白のみ

### 不完全と判定されるケース
- `[1, 2,` - 未閉じ括弧
- `(1 + 2` - 未閉じ括弧
- `{a: 1,` - 未閉じ括弧
- `f x =` - 関数定義の本体なし
- `if True:` - ブロック開始

### 構文エラーと判定されるケース
- `1 + + 2` - 無効な構文
- その他のパースエラー

## テスト

```bash
cargo test --package erg_parser --features large_thread -- code_completeness_tests
```

## 参考
- [Python 3.13 REPL調査](python313-repl-research.md)
