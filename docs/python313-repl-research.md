# Python 3.13 REPL (PyREPL) 調査

## 概要

Python 3.13では、[PyPyプロジェクトのPyREPL](https://github.com/pypy/pyrepl)をベースにした新しいREPLが導入された。実装は `Lib/_pyrepl/` ディレクトリにある。

### 主な特徴

- 複数行編集と履歴保持
- シンタックスハイライト（カラー表示）
- ブロック単位の履歴ナビゲーション
- ペーストモード（F3）
- `help`, `exit`, `quit` を関数呼び出しなしで使用可能

---

## ファイル構成

`Lib/_pyrepl/` ディレクトリ内のファイル:

| ファイル | 説明 |
|----------|------|
| `reader.py` | コア行読み取り機能 |
| `commands.py` | コマンド実装（accept, maybe_accept等） |
| `readline.py` | readline互換インターフェース |
| `simple_interact.py` | 簡易インタラクティブシェルモード |
| `historical_reader.py` | コマンド履歴管理 |
| `completing_reader.py` | タブ補完機能 |
| `keymap.py` | キーボード入力からエディタコマンドへのマッピング |
| `unix_console.py` | Unix/Linux固有の実装 |
| `windows_console.py` | Windows固有の実装 |
| `console.py` | コンソールインターフェース抽象化 |

---

## Enter キーの動作 - `maybe_accept` コマンド

### 重要な発見

Enter キーは単純な `accept` ではなく `maybe_accept` コマンドにマッピングされている。

### `maybe_accept` の動作ロジック

```python
class maybe_accept(commands.Command):
    def do(self):
        r = self.reader
        text = r.get_unicode()

        # 1. more_lines コールバックでコードが完全かチェック
        if r.more_lines is not None and r.more_lines(text):
            # コードが不完全 → 新しい行を挿入（自動インデント付き）
            self.insert_newline_with_auto_indent()
            return

        # 2. カーソル後に改行がある場合
        if has_newlines_after_cursor:
            # 新しい行を挿入
            self.insert_newline_with_auto_indent()
            return

        # 3. コードが完全 → 実行
        self.finish = True
```

### 判定フロー

```
Enter キー押下
    │
    ▼
more_lines(text) を呼び出し
    │
    ├─ True (不完全) ──▶ 改行を挿入、自動インデント
    │
    └─ False (完全)
         │
         ▼
    カーソル後に改行がある？
         │
         ├─ Yes ──▶ 改行を挿入
         │
         └─ No ──▶ コード実行 (finish = True)
```

---

## コード完全性の判定 - `more_lines` コールバック

`simple_interact.py` の `_more_lines()` 関数:

```python
def _more_lines(text):
    # Python の compile() を使って完全性をチェック
    try:
        result = console.compile(text, "<stdin>", "single")
        if result is None:
            return True   # 不完全 → 継続入力が必要
        return False      # 完全 → 実行可能
    except (SyntaxError, OverflowError, ValueError):
        # エラーの場合も実行を試みる（エラーメッセージ表示のため）
        return False
```

### compile() の "single" モード

- `None` を返す: 入力が不完全（継続が必要）
- コードオブジェクトを返す: 入力が完全
- 例外を投げる: 構文エラー

### `code.compile_command()` の内部アルゴリズム

```python
# Lib/codeop.py
def compile_command(source, filename="<input>", symbol="single"):
    # 1. まずそのままコンパイル試行
    try:
        code = compile(source, filename, symbol)
        return code  # 成功 → 完全
    except SyntaxError as e:
        pass

    # 2. 改行を追加してコンパイル試行
    try:
        compile(source + "\n", filename, symbol)
        return None  # 成功 → 不完全（もっと入力が必要）
    except SyntaxError:
        pass

    # 3. さらに改行を追加
    try:
        compile(source + "\n\n", filename, symbol)
        return None
    except SyntaxError:
        raise  # 本当のエラー
```

### 判定ロジックのフローチャート

```
入力テキスト
    │
    ▼
compile(text) 試行
    │
    ├─ 成功 ──────────────────▶ 完全（実行可能）
    │
    └─ SyntaxError
           │
           ▼
    compile(text + "\n") 試行
           │
           ├─ 成功 ────────────▶ 不完全（継続入力）
           │
           └─ SyntaxError
                  │
                  ▼
           compile(text + "\n\n") 試行
                  │
                  ├─ 成功 ──────▶ 不完全（継続入力）
                  │
                  └─ SyntaxError ▶ 構文エラー（実行してエラー表示）
```

---

## 自動インデント

### ルール

1. 行末が `:` で終わる場合、次の行は自動的に4スペースインデント
2. 空行が2回連続すると、ブロック終了と判定
3. 最後の2行が全て空白の場合、自動インデントは適用されない

### 実装

```python
def insert_newline_with_auto_indent(self):
    # 現在行のインデントを取得
    current_indent = self.get_current_indent()

    # 行末がコロンなら追加インデント
    if self.current_line.rstrip().endswith(':'):
        current_indent += '    '

    # 改行とインデントを挿入
    self.insert('\n' + current_indent)
```

---

## キーバインディング

| キー | コマンド | 動作 |
|------|----------|------|
| **Enter** | `maybe_accept` | コードが完全なら実行、不完全なら改行追加 |
| **F1** | - | ヘルプブラウジング |
| **F2** | - | 履歴ブラウジング（出力をスキップ） |
| **F3** | - | ペーストモード切替 |
| **Ctrl+R** | - | 逆方向履歴検索 |
| **Ctrl+S** | - | 順方向履歴検索 |
| **PageUp/Down** | - | プレフィックスベース履歴検索 |
| **Up/Down** | - | 複数行内では行移動、端では履歴移動 |

---

## Reader クラス階層

```
Reader (base)
    │
    ├── HistoricalReader (履歴管理)
    │       │
    │       └── CompletingReader (補完機能)
    │               │
    │               └── PythonicReader (Python REPL用)
```

### 特徴

- グローバル変数なし（複数の独立したリーダーを実行可能）
- イベントループへの統合が容易
- Unicode完全対応

---

## 環境変数

| 変数 | 説明 | 値 |
|------|------|-----|
| `PYTHON_BASIC_REPL` | 新REPLを無効化 | `1` で旧REPL使用 |
| `PYTHON_COLORS` | カラー出力制御 | `on`, `off`, `auto` |
| `NO_COLOR` | カラー無効化 | 任意の値で無効 |
| `FORCE_COLOR` | カラー強制有効 | 任意の値で有効 |
| `PYTHON_HISTORY` | 履歴ファイルパス | ファイルパス |

---

## Ergへの適用

### 現状との比較

| 項目 | Python 3.13 | Erg (現状) |
|------|------------|-----------|
| Enter | 自動判定（完全なら実行） | 常に改行追加 |
| 実行 | Enter（自動） | Ctrl+Enter |
| 完全性判定 | `compile()` で判定 | なし（ユーザー判断） |
| 実装言語 | Python | Rust |

### Python方式の利点

- Enter一回で実行できる（単行の場合）
- 従来のREPLユーザーに馴染みやすい
- ブロック終了を自動検出

### Erg方式（Ctrl+Enter）の利点

- 明示的で予測可能
- 複雑なパーサー不要
- Jupyter Notebook スタイルに馴染みやすい

### 将来の検討事項

Python方式を採用する場合、`expect_block()` を拡張して `more_lines` 相当の機能を実装する必要がある。

---

## Erg への実装案

### 案1: パーサーベース（高精度）

```rust
fn more_lines(text: &str) -> bool {
    let lexer = Lexer::new(text.to_string());
    let mut parser = Parser::new(lexer);

    match parser.parse() {
        Ok(_) => false,  // 完全
        Err(e) if e.is_incomplete() => true,  // 不完全
        Err(_) => false,  // 構文エラー（実行してエラー表示）
    }
}
```

### 案2: 既存の `expect_block()` を活用（中精度）

現在の `expect_block()` は `BlockKind` を返す。これを拡張:

```rust
// traits.rs の expect_block を参考に
fn is_code_complete(src: &str) -> bool {
    // 1. 開き括弧の数 == 閉じ括弧の数
    let opens = src.matches(['(', '[', '{']).count();
    let closes = src.matches([')', ']', '}']).count();
    if opens != closes {
        return false;
    }

    // 2. 行末が継続を示すトークンで終わっていない
    let trimmed = src.trim_end();
    let continues = [
        ":", "=>", "do", "do!", "=", ",",
        "(", "[", "{", "\\",
    ];
    if continues.iter().any(|c| trimmed.ends_with(c)) {
        return false;
    }

    // 3. 複数行文字列が閉じている
    let triple_quotes = src.matches("\"\"\"").count();
    if triple_quotes % 2 != 0 {
        return false;
    }

    true
}
```

### 案3: シンプルなヒューリスティック（低精度・高速）

```rust
fn is_code_complete(cell: &Cell) -> bool {
    let text = cell.to_code();
    let trimmed = text.trim();

    // 空なら完全
    if trimmed.is_empty() {
        return true;
    }

    // 最後の行が空行で、前の行がインデントなしなら完全
    if cell.lines.len() >= 2 {
        let last = cell.lines.last().unwrap();
        let prev = &cell.lines[cell.lines.len() - 2];
        if last.trim().is_empty() && !prev.starts_with(' ') {
            return true;
        }
    }

    // 単一行で継続トークンがなければ完全
    if cell.lines.len() == 1 {
        let continues = [":", "=>", "do", "do!", "=", ",", "(", "[", "{"];
        return !continues.iter().any(|c| trimmed.ends_with(c));
    }

    false
}
```

### 実装方式の比較

| 方式 | 精度 | 実装コスト | パフォーマンス |
|------|------|-----------|--------------|
| パーサーベース | 高 | 高 | 中（毎回パース） |
| expect_block拡張 | 中 | 中 | 高 |
| ヒューリスティック | 低 | 低 | 高 |

### 推奨アプローチ

1. **案3のヒューリスティック**から始めて、必要に応じて案2に移行
2. 現在のCtrl+Enter方式も維持（明示的実行のオプション）
3. 環境変数 `ERG_BASIC_REPL` で旧方式に切り替え可能にする

---

## 参考リンク

- [What's New In Python 3.13](https://docs.python.org/3/whatsnew/3.13.html)
- [PEP 762 – REPL-acing the default REPL](https://peps.python.org/pep-0762/)
- [CPython _pyrepl source](https://github.com/python/cpython/tree/v3.13.0/Lib/_pyrepl)
- [PyPy pyrepl](https://github.com/pypy/pyrepl)
- [The new REPL in Python 3.13 - Trey Hunner](https://treyhunner.com/2024/05/my-favorite-python-3-dot-13-feature/)
