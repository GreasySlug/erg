# fmtサブコマンド

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/tools/fmt.md%26commit_hash%3D90def16ad652012b938b96236ac63a034a393acf)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/tools/fmt.md&commit_hash=90def16ad652012b938b96236ac63a034a393acf)

ergコマンドにはfmtというサブコマンドがあり、ソースコードを整形する。

```console
$ erg fmt src/          # src/以下の.erファイルをすべて書き換える
$ erg fmt --check src/  # 変わるファイル名を出すだけで、何も書き換えない
$ erg fmt --stdout a.er # 結果を標準出力に出す
```

`erg fmt`は`erg --mode fmt`と書いても同じである。オプションはergコマンド全体の規則どおり、パスより*前*に置く。
パイプ入力と`-c`は書き戻す先がないため、常に標準出力に出る。

ディレクトリを渡すと、その下の`.er`ファイルをすべて整形する。
シンボリックリンクと、`.`で始まる名前のディレクトリには入らない。
`.git`や`.venv`は自分が整形してよいコードではなく、祖先を指すリンクは循環するためである。

## 整形はプログラムを変えない

整形結果は再度字句解析され、入力とトークン単位で比較される。
1つでも違えば——あるいは結果が字句解析できなければ——ファイルは元のまま残され、標準エラーに1行出る。
したがってフォーマッタのバグが奪えるのは整形だけであり、コードは奪えない。

同じ理由から、そもそも字句解析できないファイルは`--check`の失敗にはせず、報告して飛ばす。
`erg fmt`では直せない以上、CIを止めても`erg check`がもっと的確に言うことを繰り返すだけになる。

## 整形を止める

```python
# fmt: off
matrix = [
    1, 0, 0,
    0, 1, 0,
    0, 0, 1,
]
# fmt: on

x = compute(a,b)  # fmt: skip
```

| 書き方 | 効果 |
| ------ | ---- |
| `# fmt: off` | この行以降を整形しない |
| `# fmt: on` | 再開する(`off`のまま閉じなければファイル末尾まで) |
| `# fmt: skip` | その行を整形しない(括弧で複数行に分かれていればその論理行全体) |

綴りは厳密に一致する必要がある。`#fmt:off`や`# FMT: OFF`はただのコメントである。
効いているように見えて効かないディレクティブは、明らかに何もしないものより悪い。
末尾の空白だけは無視する。エディタでは見えないためである。

## オプション

| オプション | 既定値 | 意味 |
| ---------- | ------ | ---- |
| `--indent <n>` | `4` | ネスト1段あたりの空白数(1以上) |
| `--max-blank-lines <n>` | `2` | 残す空行の連なりの上限 |
| `--max-width <n>` | `100` | フォーマッタが収めようとする桁 |
| `--check` | | 何も書き換えず、変わるファイルがあれば終了コード1 |
| `--stdout` | | ファイルに書き戻さず標準出力へ出す |
| `--exclude <substr>` | | この文字列を含むパスを飛ばす(複数指定可) |

設定ファイルはなく、オプションはこの6つで全部である。
フォーマッタが役に立つのはレイアウトの議論を終わらせるからであり、
プロジェクトごとに違う答えを出せるならそれができない。

## エディタでの利用

言語サーバー(`erg server`)が`textDocument/formatting`を実装しているので、
保存時整形は追加の設定なしで動く。段数は`--indent`ではなくエディタ側の設定を使う。
