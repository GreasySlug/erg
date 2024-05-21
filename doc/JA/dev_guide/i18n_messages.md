# メッセージの多言語化

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/dev_guide/i18n_messages.md%26commit_hash%3Da76da9feb2491b58a7d02476edfbd29a266e1033)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/dev_guide/i18n_messages.md&commit_hash=a76da9feb2491b58a7d02476edfbd29a266e1033)

Ergはメッセージ(スタート、オプション、ドキュメント、ヒント、警告、エラーメッセージなど)の多言語化を進めています。
このプロジェクトは、RustやErgの詳しい知識がなくても参加することができます。ぜひ協力をお願いします。

以下に、多言語化の方法を説明します。

## `switch_lang!`を探す

Ergのソースコードの中で、`switch_lang!`という項目を探します(grepやエディタの検索機能を使ってください)。
以下のようなものが見つかるはずです。

```rust
switch_lang!(
    "japanese" => format!("この機能({name})はまだ正式に提供されていません"),
    "english" => format!("this feature({name}) is not implemented yet"),
),
```

このメッセージは現在、日本語と英語のみでサポートされています。試しに簡体字のメッセージを追加してみましょう。

## メッセージを追加する

他の言語の内容を見ながら、翻訳されたメッセージを追加してください。最後にカンマ(`,`)を忘れないでください。

```rust
switch_lang!(
    "japanese" => format!("この機能({name})はまだ正式に提供されていません"),
    "simplified_chinese" => format!("该功能({name})还没有正式提供"),
    "english" => format!("this feature({name}) is not implemented yet"),
),
```

なお、英語はデフォルトであり、必ず最後に来るようにします。
`{name}` の部分は Rust のフォーマット機能で、変数の内容 (`name`) を文字列に埋め込むことができます。

## ビルド

では、`--features simplified_chinese` オプションを付けてビルドしてみましょう。

<img src="https://raw.githubusercontent.com/erg-lang/erg/main/assets/screenshot_i18n_messages.png" alt='screenshot_i18n_messages'>

やりましたね!

## FAQ

Q: `{RED}{foo}{RESET}` のような指定は何を意味するのでしょうか?
A: {RED}以降が赤色で表示されます。{RESET}で色を元に戻します。

Q: 自分の言語を追加したい場合、`"simplified_chinese" =>`の部分はどのように置き換えればよいですか?
A: 現在、以下の言語がサポートされています。

* "english" (デフォルト)
* "japanese" (日本語)
* "simplified_chinese" (簡体字中国語)
* "traditional_chinese"(繁体字中国語)

これら以外の言語を追加したい場合は、リクエストしてください。
