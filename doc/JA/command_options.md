# コマンドライン引数

## サブコマンド

以下の名前はいずれも`--mode`でも指定できます。括弧内は別名で、どれを使っても同じです。

### lex (lexer)

字句解析結果を表示します。

### parse (parser)

構文解析結果を表示します。

### desugar (desugarer)

脱糖後のASTを表示します。ネストした変数の展開やパターンの書き換えが済んだ状態です。

### typecheck (lower, tc)

型検査結果を表示します。

### check (fullcheck, checker)

型・副作用・所有権のすべての検査を行い、コード生成は行いません。

### compile (comp, compiler)

コンパイルを実行します。

### transpile (trans, transpiler)

Pythonスクリプトへ変換します。

### run (exec, execute)

実行結果を表示します。既定のモードなので、名前を省略できます。

### read (byteread, reader, dis)

`.pyc`ファイルを逆シリアライズし、コードオブジェクトを表示します。

### server (language-server)

ランゲージサーバーを起動します。

### lint (linter)

プログラムをlintします。

### fmt (format, formatter)

ソースコードを整形します。[tools/fmt.md](./tools/fmt.md)を参照してください。

### pack (package)

パッケージマネージャを起動します。[tools/pack.md](./tools/pack.md)を参照してください。

### pydecl (py-decl)

Pythonのソースや型スタブからErgの宣言(`.d.er`)を生成します。

## オプション

オプションはファイルパスより*前*に置きます。パスより後ろはスクリプトへの引数として扱われます。

### --build-features

コンパイラのビルド時に有効化された機能を表示します。

### --check

`erg fmt`専用。変わるファイルを報告するだけで何も書き換えず、変わるものがあれば終了コード1になります。
[tools/fmt.md](./tools/fmt.md)を参照してください。

### -c, --code

実行するコードを指定します。

### --decls-from-py

`.d.er`や`.pyi`スタブが見つからない場合に、型注釈付きの`.py`ソースからErgの宣言を生成します。
ビルド機能`pydecl`が必要です。

### --exclude

`erg fmt`専用。この文字列を含むパスを飛ばします。複数回指定できます。

### -? , -h, --help

ヘルプを表示します。

### --hex-py-magic-num, --hex-python-magic-number

対象とするPythonバイトコードのマジックナンバーを、先頭2バイトの16進数で指定します。

### --indent

`erg fmt`専用。ネスト1段あたりの空白数を指定します。1以上で、既定値は4です。

### --max-blank-lines

`erg fmt`専用。残す空行の連なりの上限です。既定値は2です。

### --max-width

`erg fmt`専用。フォーマッタが収めようとする桁数です。既定値は100です。

### --mode

サブコマンドを指定します。

### -m, --module

実行するモジュールを指定します。

### --no-std

Erg標準ライブラリなしでコンパイルします。

### -o, --opt-level, --optimization-level

最適化レベルを0から3で指定します。

### --output-dir, --dest, --dist, --dest-dir, --dist-dir

コンパイル結果の出力先ディレクトリを指定します。

### --ping

`pong`と表示して終了します。実行ファイルが動くことの確認用です。

### --ps1

REPLのプロンプトを指定します。既定値は`>>> `です。

### --ps2

REPLの継続行のプロンプトを指定します。既定値は`... `です。

### --py-command, --python-command

使用するPythonインタプリタを指定します。既定値はUnixでは`python3`、Windowsでは`python`です。

### --py-magic-num, --python-magic-number

対象とするPythonバイトコードのマジックナンバーを、32ビット符号なし整数で指定します。

### --py-server-timeout

REPL実行のタイムアウト時間を指定します。既定値は10秒です。

### -q, --quiet-startup, --quiet-repl

REPL起動時のプロセッサ情報の表示を止めます。

### -t, --show-type

REPLの実行結果に型情報を付けて表示します。

### --stdout

`erg fmt`専用。ファイルに書き戻さず標準出力へ出します。

### --target-version

出力するpycファイルのバージョンを指定します。バージョンはセマンティックバージョニングに従います。

### --transpile-target, --target

`erg transpile`が出力する形式を指定します。`python`(`py`)、`json`、`toml`が指定できます。

### --use-local-package

ローカルのパスからパッケージを追加します。名前、インポート時の名前、バージョン、パスを取ります。

### --use-package

パッケージを追加します。名前、インポート時の名前、バージョンを取ります。

### --use-pylyzer

型情報のないPythonモジュールの宣言生成に、外部のpylyzerを使用します。

### -v, --verbose

コンパイラ出力の冗長性を0から2で指定します。
0を指定しても警告は止められないことに注意してください。

### -V, --version

バージョンを表示します。

### --

ランタイム引数を指定します。
