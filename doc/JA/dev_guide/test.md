# テスト

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/dev_guide/test.md%26commit_hash%3D307087f6b5acf173f72ff8d8b8871a73b96605b7)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/dev_guide/test.md&commit_hash=307087f6b5acf173f72ff8d8b8871a73b96605b7)

テストは、コードの品質を保証するための重要な要素です。

テストは以下のコマンドから実行してください。

```sh
cargo test --features large_thread
```

cargoはテスト実行用のスレッドを小さく取るため、`large_thread`フラグを付けてスタックオーバーフローを回避します。

## テストの配置場所

実装した機能に合わせて配置してください。パーサーに対するテストは`erg_parser/tests`以下に、コンパイラ(型検査器等)のテストは`erg_compiler/tests`以下に、ユーザーが直接使える言語機能のテストは`erg/tests`以下に配置してください(ただし現在はテストの整備中で、必ずしもこの規則に従ってテストが配置されているわけではありません)。

## テストの書き方

テストは大きく分けて2種類あります。positive(正常系)テストとnegative(異常系)テストです。
正常系テストは、コンパイラが意図したように動作するか確かめるためのテストで、異常系テストは不正な入力に対してコンパイラが適切にエラーを出力するか確かめるためのテストです。
プログラミング言語処理系はその性質上、あらゆるソフトウェアの中でも特に不正な入力を受けやすく、かつエラーを常にユーザーに提示しなくてはならないので、後者にも気を配る必要があります。

あなたが言語に新しい機能を追加するならば、少なくとも1つの正常系テストを書く必要があります。異常系テストも出来れば書いてください。

## ignore属性

Erg開発チームはpre-commitの導入を推奨しています。
これにより、コミット前に毎回テストが走るためバグの混入を未然に防ぐことができますが、幾つかのテストは時間がかかるためコミットが遅くなってしまいます。

そこで、重いテストないし失敗する蓋然性が低いテストには`#[ignore]`属性を付けています。
`#[ignore]`属性を付けたテストは`cargo test`では実行されませんが、`cargo test -- --include-ignored`で実行することができます。
これらのテストはCIで実行されるため、ローカルPCで実行する必要はありません。
