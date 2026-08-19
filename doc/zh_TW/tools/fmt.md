# fmt 子命令

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/tools/fmt.md%26commit_hash%3D90def16ad652012b938b96236ac63a034a393acf)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/tools/fmt.md&commit_hash=90def16ad652012b938b96236ac63a034a393acf)

erg 命令有一個名為 fmt 的子命令，用於格式化源代碼

```console
$ erg fmt src/          # 重寫 src/ 下的所有 .er 文件
$ erg fmt --check src/  # 只列出會發生變化的文件，不寫入任何內容
$ erg fmt --stdout a.er # 將結果寫到標準輸出
```

`erg fmt` 與 `erg --mode fmt` 等價。與 erg 命令的其他部分一樣，選項要寫在路徑*之前*
從管道或 `-c` 讀取時總是寫到標準輸出，因為沒有可寫回的文件

傳入目錄時，其下的所有 `.er` 文件都會被格式化
符號鏈接和以 `.` 開頭的目錄不會被進入: `.git` 和 `.venv` 中的代碼不該由你來重排，
而指向祖先目錄的符號鏈接會形成循環

## 格式化不會改變你的程序

格式化後的輸出會被重新詞法分析，並與輸入逐個 token 比較
只要有任何不同——或者輸出無法再被詞法分析——文件就會原樣保留，並在標準錯誤輸出一行
因此格式化工具的缺陷只會讓你失去格式，而不會失去代碼

出於同樣的原因，完全無法詞法分析的文件會被報告並跳過，而不是讓 `--check` 失敗
運行 `erg fmt` 無法修復它，因此讓 CI 中斷只是把 `erg check` 已經說得更清楚的話重複一遍

## 關閉格式化

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

| 指令 | 效果 |
| ---- | ---- |
| `# fmt: off` | 從這一行起停止格式化 |
| `# fmt: on` | 恢復格式化(若 `off` 未被關閉，則一直到文件末尾) |
| `# fmt: skip` | 不格式化這一行(若括號使其跨越多行，則為整個邏輯行) |

拼寫必須完全一致; `#fmt:off` 和 `# FMT: OFF` 只是普通註釋
一個看起來生效實際卻不生效的指令，比一個明顯什麼都不做的指令更糟
唯一被忽略的是行尾空白，因為編輯器並不顯示它

## 選項

| 選項 | 默認值 | 含義 |
| ---- | ------ | ---- |
| `--indent <n>` | `4` | 每層縮進的空格數(1 以上) |
| `--max-blank-lines <n>` | `2` | 保留的連續空行上限 |
| `--max-width <n>` | `100` | 格式化工具盡量不超過的列數 |
| `--check` | | 不寫入任何內容; 若有文件會變化則以 1 退出 |
| `--stdout` | | 寫到標準輸出而不是寫回文件 |
| `--exclude <substr>` | | 跳過包含該子串的路徑(可重複指定) |

沒有配置文件，選項就這六個
格式化工具的價值在於終結關於排版的爭論，
而如果每個項目都能給出不同的答案，它就做不到這一點

## 在編輯器中使用

語言服務器(`erg server`)實現了 `textDocument/formatting`，
因此保存時格式化無需額外配置即可工作。縮進寬度使用編輯器自身的設置，而非 `--indent`
