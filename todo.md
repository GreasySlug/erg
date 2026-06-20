# ELS (Erg Language Server) 未実装・未完成 TODO

`crates/els/` の調査(2026-06-20)で判明した未実装・部分実装箇所。優先度順。
進めるごとにチェックを更新する。

## P0: 影響が広く局所的(最優先)

- [x] **内包表記で HIR Visitor が機能しない** — `hir_visitor.rs` の
  `get_expr_from_list/dict/set`・`get_list/dict/set_info` が `List/Dict/Set::Normal`
  以外(`Comprehension`/`WithLength`)で `_ => None`。hover・定義ジャンプ・参照検索などが
  内包表記内で全滅する。(lines 466, 492, 526, 782, 798, 814)
  → **完了 (2026-06-20)**: 6関数すべてで `WithLength`(elem/len)・`Comprehension`
    (elem/guard, key/value/guard)へ降りるよう実装。`[m; n]`(`ListWithLength`)が
    実際に到達する経路。`Comprehension`/`Dict::Comprehension` は現状コンパイラが
    構築しない(生成器内包表記は `list(map(..))` に脱糖)が防御的に対応。
    test: `crates/els/tests/with_length.er` + `test_goto_definition_in_list_with_length`。

## P1: capabilities 宣言と実装の乖離(体感バグ)

- [x] **`textDocument/didClose` ハンドラ欠落** — `open_close: Some(true)` 宣言済みだが
  notification ハンドラが無く、閉じてもファイルキャッシュが解放されない。
  → **完了 (6153cc64)**: キャッシュ削除 + 診断クリア。uri は spec/bare 両形式を許容。test_did_close。
- [x] **`prepareRename` 未実装** — `rename_provider = Some(OneOf::Left(true))`(bool)で
  prepareRename ハンドラ無し。
  → **完了**: `rename_provider` を `RenameOptions{prepare_provider:true}` に。`prepare_rename`
    でシンボルの range/placeholder を返し、builtin/std/未定義は null。test_prepare_rename。
- [ ] **Go to Implementation の誤実装** — `implementation.rs` は「定義の定義」を返す
  リダイレクトで、トレイト実装クラス一覧を返す本来の動作でない。
- [ ] **Call Hierarchy `from_ranges` 常に空** — `call_hierarchy.rs:66,124,134`。
- [ ] **`executeCommand: eliminate_unused_vars` 未処理** — `command.rs` に arm が無く `Ok(None)`。
  → **保留**: 機能自体はコードアクション(quickfix)で動作済み。コマンド経由実装には
    `workspace/applyEdit`(server→client)基盤の新規追加が必要(ELS に未実装)。費用対効果から後回し。
    あるいは形骸化した登録を capabilities から外すのも選択肢。
- [x] **Code Lens 継承数 `send_class_inherits_lens` 空実装** — `code_lens.rs`。
  → **完了**: `gen_show_class_refs_command(loc, noun, hide_when_empty)` に共通化し、各 `ClassDef`
    の上に「N subclasses」レンズ(サブクラス0件は非表示)。test_code_lens_inherits。

## P2: 完全未実装の LSP 機能

- [ ] フォーマット系: `formatting` / `rangeFormatting` / `onTypeFormatting`
- [ ] `textDocument/declaration`
- [ ] 型階層: `prepareTypeHierarchy` / supertypes / subtypes
- [ ] `workspace/didChangeWatchedFiles`, `workspace/didChangeConfiguration`
- [ ] Pull diagnostics, `linkedEditingRange`, `moniker`

## P3: 部分実装の改善

- [ ] Folding: import のみ → 関数/クラス/ブロック対応 (`folding_range.rs`)
- [ ] 補完: site-packages モジュール非対応 (`completion.rs:454`)、複数行コメント抑制(573)
- [ ] Rename: multi-path import 非対応 (`rename.rs:231`)
- [ ] Hover: `StrInterpMid` 非対応 (`hover.rs:98`)
- [ ] Workspace Symbol: `container_name` 常に None
- [ ] Signature Help: `VBar`(型適用)トリガーで None

## P4: 内部品質・性能

- [ ] 実行中タスクの cancel (`scheduler.rs:202`)
- [ ] mutable dependent type のリセット (`diagnostics.rs:261`, `server.rs:1077`)
- [ ] health checker スレッドの再起動時 kill (`diagnostics.rs:568`)
- [ ] `get_checker` のキャッシュ再利用 (`server.rs:1132`)
- [ ] `loc_to_pos` の列オフセット回避策の根本解決 (`util.rs:104`)
