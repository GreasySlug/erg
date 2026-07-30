# erg_proc_macros における syn の使用方法

## 概要

`erg_proc_macros` クレートは Erg コンパイラ用の手続きマクロを提供するクレートです。`syn` クレートは Rust のトークンストリームを解析して構文木 (AST) に変換するために使用されています。

## 依存関係

```toml
[dependencies]
syn = { version = "2.0", features = ["full"] }
quote = "1.0"
```

- `syn`: Rust コードの解析（バージョン 2.0）
- `quote`: Rust コードの生成

## syn 1.0 から 2.0 への移行

### syn 2.0 の主要な変更点

syn 2.0 では多くの破壊的変更がありますが、本クレートで使用している機能は互換性を維持しています。

#### 主要な変更点（参考）

| 変更点 | 詳細 |
|--------|------|
| `AttributeArgs` の削除 | `Punctuated<Meta, Token![,]>` を代わりに使用 |
| `GenericArgument` の `#[non_exhaustive]` 化 | パターンマッチで `_` が必要になる可能性 |
| トークン名の変更 | `Add` → `Plus`, `Sub` → `Minus`, `Colon2` → `PathSep` など |
| `Stmt::Expr` と `Stmt::Semi` の統合 | セミコロンが `Option<Token![;]>` に |
| MSRV の引き上げ | Rust 1.31 → Rust 1.56 |

#### 本クレートへの影響

現在のコードは syn 2.0 と**完全に互換性があります**。使用している API（`parse_macro_input!`、`parse_quote!`、`Punctuated`、型関連の構造体）はすべて syn 2.0 でも同様に動作します。

### 移行で変更が不要だった理由

1. **`AttributeArgs` 未使用**: 属性引数のパースには独自のロジックを使用
2. **トークン型の直接使用なし**: `Token![,]` などのマクロ経由でのみ使用
3. **`Stmt` の直接操作なし**: ブロック全体を `parse_quote!` で生成
4. **型パターンマッチの互換性維持**: `Type::Path`、`Type::Reference` などは変更なし

## インポートされている syn の型

```rust
use syn::{
    punctuated::Punctuated,
    AngleBracketedGenericArguments,
    GenericArgument,
    PathArguments,
    ReturnType,
    Type,
    TypePath,
    TypeReference,
    TypeSlice,
};
```

## 主要なマクロと syn の使用箇所

### 1. `#[exec_new_thread]`

関数を新しいスレッドで実行するように変換するマクロです。

#### syn の使用方法

1. **`syn::parse_macro_input!`**: トークンストリームを `syn::ItemFn`（関数定義）としてパース

   ```rust
   let mut item_fn = syn::parse_macro_input!(item as syn::ItemFn);
   ```

2. **`ReturnType` の解析**: 戻り値の型を取得

   ```rust
   let ReturnType::Type(_, out) = &item_fn.sig.output else { ... };
   ```

3. **`Type::Path` の解析**: 型パスから Result の型引数を抽出

   ```rust
   let Type::Path(TypePath { path, .. }) = out.as_ref() else { ... };
   let PathArguments::AngleBracketed(args) = &result_t.arguments else { ... };
   ```

4. **`syn::LitStr::new`**: 文字列リテラルを生成

   ```rust
   let name = syn::LitStr::new(&name, item_fn.sig.ident.span());
   ```

5. **`syn::parse_quote!`**: 新しいコードブロックを生成

   ```rust
   let block = syn::parse_quote! {{
       fn error(msg: impl Into<String>) -> std::io::Error { ... }
       fn _f() -> Result<#t, Box<dyn std::error::Error>> { #block }
       ...
   }};
   ```

#### 変換例

```rust
// 変換前
#[exec_new_thread]
fn foo() -> Result<isize, Box<dyn std::error::Error>> {
    ...
}

// 変換後
fn foo() -> Result<isize, Box<dyn std::error::Error>> {
    fn error(msg: impl Into<String>) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::Other, msg.into())
    }
    fn _f() -> Result<isize, Box<dyn std::error::Error>> { ... }
    fn f() -> Result<isize, Box<dyn std::error::Error + Send>> {
        _f().map_err(|e| Box::new(error(e.to_string())) as _)
    }
    erg_common::spawn::exec_new_thread(f, "foo").map_err(|e| e as _)
}
```

### 2. `#[to_owned]`

参照を返す関数を所有型を返すように変換するマクロです。

#### syn の使用方法

1. **関数のパース**:

   ```rust
   let mut item_fn = syn::parse_macro_input!(item as syn::ItemFn);
   ```

2. **戻り値の型変換**: `type_to_owned` 関数で参照型を所有型に変換

   - `&str` → `String`
   - `&[T]` → `Vec<T>`
   - `&T` → `T`

3. **新しいコードブロック生成**:

   ```rust
   let block = syn::parse_quote! {{
       let r = #block;
       r.to_owned()
   }};
   ```

#### 変換例

```rust
// 変換前
#[to_owned]
fn foo(s: &str) -> &str { s }

// 変換後
fn foo(s: &str) -> String { let r = s; r.to_owned() }
```

### 3. ダミー属性マクロ

以下のマクロは pyo3 の属性を模倣するダミー実装で、トークンストリームをそのまま返します：

- `#[pyo3]`
- `#[pyclass]`
- `#[pymethods]`
- `#[staticmethod]`
- `#[classmethod]`
- `#[getter]`
- `#[setter]`
- `#[new]`

これらは `syn` を使用していません（入力をそのまま返すだけ）。

## ヘルパー関数

### `type_to_owned(t: &Type) -> Type`

型を参照型から所有型に変換します。

- `Type::Reference` の場合:
  - `&[T]` → `Vec<T>`（`syn::parse_quote! { Vec<#elem> }`）
  - `&str` → `String`（`syn::parse_quote! { String }`）
  - その他 → 内部の型をそのまま返す

- `Type::Path` の場合:
  - ジェネリック引数を再帰的に変換

### `args_to_owned(args: &PathArguments) -> PathArguments`

ジェネリック引数内の型を再帰的に所有型に変換します。

`Punctuated` を使用して新しい引数リストを構築：

```rust
let mut punc = Punctuated::new();
punc.extend(res);
```

## まとめ

| syn の機能 | 使用箇所 | 目的 |
|-----------|---------|------|
| `parse_macro_input!` | `exec_new_thread`, `to_owned` | トークンストリームを AST にパース |
| `ItemFn` | 全マクロ | 関数定義の表現 |
| `ReturnType` | `exec_new_thread`, `to_owned` | 戻り値の型の解析 |
| `Type`, `TypePath`, `TypeReference`, `TypeSlice` | `type_to_owned` | 型の解析と変換 |
| `PathArguments`, `AngleBracketedGenericArguments` | `args_to_owned` | ジェネリック引数の解析 |
| `Punctuated` | `args_to_owned`, `type_to_owned` | 句読点区切りのリスト操作 |
| `parse_quote!` | `exec_new_thread`, `to_owned` | 新しい Rust コードの生成 |
| `LitStr` | `exec_new_thread` | 文字列リテラルの生成 |
