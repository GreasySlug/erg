# erg_compiler 型チェックの計測（hotspot 調査）

パーサの計測（[erg_parser-architecture.md](erg_parser-architecture.md) §6）で、フルチェック時間の ~88% が
パーサより下流（型チェック）であることが分かった。本ドキュメントはその下流＝`erg_compiler` の
`--mode check`（FullCheck）について、**どこに時間が使われているか**を計測した結果。

> 結論先出し: チェック時間の **~96% が lowering の per-chunk 型推論（`lower_chunk` ループ）**。
> 個別パス（link_ast / register / resolve / effect / ownership）は合計でも数%。
> lowering 内では **呼び出し引数の型チェックが突出して高コスト**（1引数 ≈ 50µs ＝ 自明な変数定義の ~16倍）。

## 計測方法

プロファイラ（perf/samply/flamegraph/valgrind）はこの環境に無く、`perf_event_paranoid=2` で
perf 系も使えない。そこで**一時的な計装**で段階別の実時間を測った（計測後にリバート済み・未コミット）。

計装ポイント（再現する場合はここに `std::time::Instant` + `eprintln!`、`ERG_TIMING` env で gate）:

- `crates/erg_compiler/build_hir.rs` `GenericHIRBuilder::check()` … `lower` / `effect_check` / `ownership_check` の3区間
- `crates/erg_compiler/lower.rs` `GenericASTLowerer::lower()` … `link_ast` / `register_defs` / `lower_chunk(loop)` / `resolve` の4区間

実行: `ERG_TIMING=1 erg --mode check <file>`（release ビルド、best-of-3）。

## 段階別ブレークダウン（`tests/should_ok/long.er`, 699行, check ≈ 55ms）

| 区間 | 時間 | 割合 |
| ---- | ---: | ---: |
| link_ast | ~60 µs | 0.1% |
| register_defs（preregister_consts + register_defs） | ~0.4 ms | 0.7% |
| **lower_chunk(loop)** | **~53 ms** | **~96%** |
| resolve（型変数解決） | ~1.37 ms | 2.5% |
| effect_check | ~0.22 ms | 0.4% |
| ownership_check | ~0.26 ms | 0.5% |

→ **最適化対象は `lower_chunk` → `lower_expr` と、それが呼ぶ型推論機構**
（`context/` の `inquire.rs`・`unify.rs`・`eval.rs`・`instantiate.rs`）一択。
discrete な各パスを速くしても全体は動かない。

## lowering 内のどこが重いか（構文別マイクロベンチ）

`lower_chunk` を構文別に切り分けた（synthetic な `.er` を生成し best-of-3）。

| ワークロード | スケール | 1単位あたり | 計算量の見え方 |
| ------------ | -------- | ----------- | -------------- |
| トップレベルの自明な定義 `x_i = i` | N=200→1600: 1.4→5.2 ms | **~3 µs/def** | ほぼ線形 |
| 関数ボディ N文 | N=100→800: 1.5→4.2 ms | ~5 µs/stmt | ほぼ線形 |
| **1個の `print!` 呼び出しの引数 N個** | args=5→80: 0.3→4.2 ms | **~50 µs/arg** | **超線形**（40→80 で 2.6×） |

`long.er` の 53ms の正体は、bisect すると 298–699 行（`while!`/`if!` ブロック中の
**15引数の `print!`/`log` 呼び出しが ~25本**）に集中していた。1呼び出し ≈ 1–2 ms。

**要点**: 自明な定義は 1個 ~3µs と安いが、**呼び出し1引数の型チェックは ~50µs と ~16倍高い**。
呼び出しは実コードの至る所にあるため、これは合成ファイル固有ではなく**一般的なホットスポット**。
ただし `long.er` 自体は「595個の自明定義＋多引数 print!」という stress test であり、
1引数 50µs／超線形性が実コードでどれだけ効くかは現実的なベンチで再確認すべき（下記）。

## drill-down 結果：call 引数チェックの内訳（計測済み）

`inquire.rs`/`unify.rs`/`instantiate.rs` の主要関数に呼び出しカウンタを仕込み（計測後リバート）、
`print!` に N 引数を渡す合成ファイルで計測した（カウンタは決定的）:

| args | instantiate | sub_unify | get_super_types | get_nominal_type_ctx |
| ---: | ----------: | --------: | --------------: | -------------------: |
| 10 | 2 | 74 | 0 | 183 |
| 20 | 2 | 94 | 0 | 283 |
| 40 | 2 | 134 | 0 | 483 |
| 80 | 2 | 214 | 0 | 883 |

**判明したこと（事前仮説を複数否定）**:

- `instantiate` は **呼び出しあたり定数 2**（引数数に非依存）→ 「引数ごとに再 instantiate」仮説は**誤り**。
- `get_super_types` は **0回**（この経路では未使用）→ 「super_classes/super_traits の Vec clone が per-arg コスト」仮説は**誤り**。
- `sub_unify`（≈2/arg）も `get_nominal_type_ctx`（≈10/arg）も **引数数に対して完全に線形** → `print!` 等の
  具体可変長引数の経路に **O(n²) アルゴリズムバグは無い**（先の「40→80 で 2.6×」は小スケールの計測ノイズだった）。
- リテラル引数 `print! 0,1,…` と変数引数 `print! x_0,x_1,…` はカウンタが**完全に同一**。スコープサイズを
  振っても（50→800）lookup は概ね一定 → **変数 lookup は O(1)**。

**結論**: この経路に「clone を消す／O(n²) を潰す」式の安い勝ち筋は**無い**。1引数 ~50µs は、
引数ごとの実在する型推論作業（≈10回の `get_nominal_type_ctx` ＋ ≈2回の `sub_unify` ＋部分型チェック）の
**定数係数**であり、既に線形。高速化するなら「per-arg の定数作業を削る」しかなく、それは正しさ中枢の
慎重な最適化（要計画＋回帰テスト）になる。推測でのクイック修正はしない、というのが measure-first の結論。

## 今後の最適化候補（要計画・別タスク）

1. **`get_nominal_type_ctx` のメモ化**（最有力）
   1引数につき ~10回呼ばれる nominal-type-context 解決がホット。型→TypeContext は（推論進行で文脈が
   変わるため）素朴なキャッシュは不可だが、call 引数チェック中の同一型に対する重複解決を抑えられれば
   per-arg 定数を直接削れる可能性。無効化条件の設計が肝。
2. **ジェネリック可変長引数の O(n²) 検証**
   `print!`（要素型が具体 `Ref(Obj)`）には無いが、ユーザ定義の `|T| (*args: T) -> ...` では各引数で
   `?T` の下限 union が成長し O(n²) になりうる（unify.rs:1167 付近）。これは別の・狭いが実在しうるバグ。
   合成テスト（generic variadic を多引数で呼ぶ）で再現確認してから対処。
3. **現実的なベンチコーパス**
   `long.er` は病的入力。標準ライブラリ `.er` や ELS セルフチェックで同じ段階別計測を取り、
   call-arg 定数係数が実コードでも支配的か確認してから着手する。

> 計装パターンは本ドキュメントのとおり再適用すれば再現できる。型推論はコンパイラの正しさの中核なので、
> 最適化は必ず 計測 → 仮説 → 局所修正 → 回帰テスト の順で、別タスク（要計画）として進めること。
