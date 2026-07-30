# CPython バイトコードのバージョン切り替え

Erg は複数の CPython バージョン（3.8〜3.14）向けにバイトコードを生成するため、オペコード番号や呼び出し規則がバージョンごとに異なります。`codegen.rs` で `if self.py_version.minor >= Some(11)` を大量に書くとメンテナンスが難しくなるため、**必要なバージョンだけに切り替えられる**形でオペコードを扱う仕組みを入れています。

## 構成

### 1. バージョン別オペコード定義（`erg_common`）

| ファイル | 対象 CPython |
|----------|--------------|
| `opcode308.rs` | 3.8 |
| `opcode309.rs` | 3.9 |
| `opcode310.rs` | 3.10 |
| `opcode311.rs` | 3.11 |
| `opcode312.rs` | 3.12 |
| `opcode313.rs` | 3.13 |
| `opcode314.rs` | 3.14 |

各ファイルは `impl_u8_enum! { OpcodeNNN; ... }` でそのバージョンのオペコード番号を定義します。3.12 以降は CPython の `Include/opcode.h` や `Lib/dis.py` が変わったら、対応する `opcodeNNN.rs` の値を更新してください。

### 2. バージョン選択（`erg_common/opcode_set.rs`）

- **`OpcodeSetVersion`**: 現在のターゲット CPython に対応するオペコードセット（V308〜V314）。
- **`OpcodeSetVersion::from_python_version(v)`**: `PythonVersion` から使うオペコードセットを選ぶ。
- **メソッド**（例）: `call()`, `precall()`, `copy()`, `swap()`, `load_deref()`, `store_deref()`, `resume()`, `jump_forward()`, …  
  内部で `match self { V308 => Opcode308::..., V311 => Opcode311::..., ... }` のように、バージョンごとのオペコード番号を返します。

これにより、codegen 側では「バージョン if 分岐」ではなく「`self.opcode_set.call()` などを呼ぶだけ」にできます。

### 3. codegen での利用（`erg_compiler/codegen.rs`）

- **`PyCodeGenerator`** に `opcode_set: OpcodeSetVersion` を追加。
- **`PyCodeGenerator::new(cfg)`** で `py_version` から `OpcodeSetVersion::from_python_version(py_version)` を一度だけ計算し、`opcode_set` に保持。
- オペコード発行は、可能な箇所で `self.opcode_set.xxx()` を使う（例: 呼び出しまわりは `emit_push_null`, `emit_precall_and_call`, `emit_call_instr`, `emit_call_kw_instr` で既に使用）。

残りの `if self.py_version.minor >= Some(11)` などは、順次 `self.opcode_set` のメソッドや `opcode_set.is_3_11_plus()` などに置き換えていくと、codegen の分岐が減り、新バージョン追加時は主に `opcode_set.rs` と `opcodeNNN.rs` の修正で済むようになります。

## 新バージョン（例: 3.15）の追加手順

1. **`erg_common/opcode315.rs` を追加**  
   CPython 3.15 の `Include/opcode.h` / `Lib/dis.py` を参照してオペコード番号を定義。既存の `opcode314.rs` をコピーしてから差分を修正するとよい。

2. **`erg_common/lib.rs` に**  
   `pub mod opcode315;` を追加。

3. **`erg_common/opcode_set.rs` を更新**  
   - `OpcodeSetVersion` に `V315` を追加。  
   - `from_python_version` で `Some(15)` および「15 以上」のフォールバックを `V315` に振る。  
   - 各メソッドの `match self { ... }` に `Self::V315 => Opcode315::...` を追加。

4. **（必要なら）`python_util.rs` の `PythonVersion`**  
   3.15 用の定数や表示文字列があれば追加。

5. **codegen の分岐**  
   既に `opcode_set` 経由にしている部分は変更不要。まだ `py_version.minor` で分岐している箇所があれば、`opcode_set` のメソッドや `is_3_11_plus()` のようなフラグに寄せていく。

## 注意点

- **PRECALL**: 3.11 では PRECALL → CALL の順で発行するが、3.12 では PRECALL が削除され CALL のみ。`opcode_set.has_precall()` と `precall()` が `Option<u8>` でこれを表現している。
- **Erg 専用オペコード**（`ERG_*`）は CPython のバージョンに依存しないため、各 `opcodeNNN.rs` で同じ番号を維持する。
- 新しい CPython でオペコードの追加・削除・番号変更があった場合は、該当する `opcodeNNN.rs` と `opcode_set.rs` の両方を整合するように更新する。
