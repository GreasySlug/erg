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

## 推奨される次の一手

1. **呼び出し引数チェックの drill-down**（最有力）
   `lower_call` → `inquire.rs` の `substitute_call` / `get_call_t` / 引数の `sub_unify` を計装し、
   1引数 50µs の内訳（callable シグネチャの再 instantiate・clone・variadic の union 蓄積など）を特定。
   超線形性（args 40→80 で 2.6×）が O(n²) なら、可変長引数まわりに明確な改善余地がある可能性。
2. **現実的なベンチコーパスの用意**
   `long.er` は病的入力。実際の Erg プログラム（標準ライブラリの `.er`、ELS のセルフチェック等）で
   同じ段階別計測を取り、call-arg 仮説が一般に成り立つか確認してから最適化に着手する。
3. （任意）`ERG_TIMING` 段階タイマーを正式な opt-in 診断として整備すれば、以後の measure-first が容易。

> いずれも本ドキュメントの計装パターンをそのまま再適用すれば再現できる。型推論はコンパイラの正しさの
> 中核なので、最適化は必ず計測 → 仮説 → 局所修正 → 回帰テストの順で、別タスク（要計画）として進めること。
