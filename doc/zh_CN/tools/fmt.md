# fmt 子命令

[![badge](https://img.shields.io/endpoint.svg?url=https%3A%2F%2Fgezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com%2Fdefault%2Fsource_up_to_date%3Fowner%3Derg-lang%26repos%3Derg%26ref%3Dmain%26path%3Ddoc/EN/tools/fmt.md%26commit_hash%3D90def16ad652012b938b96236ac63a034a393acf)](https://gezf7g7pd5.execute-api.ap-northeast-1.amazonaws.com/default/source_up_to_date?owner=erg-lang&repos=erg&ref=main&path=doc/EN/tools/fmt.md&commit_hash=90def16ad652012b938b96236ac63a034a393acf)

erg 命令有一个名为 fmt 的子命令，用于格式化源代码

```console
$ erg fmt src/          # 重写 src/ 下的所有 .er 文件
$ erg fmt --check src/  # 只列出会发生变化的文件，不写入任何内容
$ erg fmt --stdout a.er # 将结果写到标准输出
```

`erg fmt` 与 `erg --mode fmt` 等价。与 erg 命令的其他部分一样，选项要写在路径*之前*
从管道或 `-c` 读取时总是写到标准输出，因为没有可写回的文件

传入目录时，其下的所有 `.er` 文件都会被格式化
符号链接和以 `.` 开头的目录不会被进入: `.git` 和 `.venv` 中的代码不该由你来重排，
而指向祖先目录的符号链接会形成循环

## 格式化不会改变你的程序

格式化后的输出会被重新词法分析，并与输入逐个 token 比较
只要有任何不同——或者输出无法再被词法分析——文件就会原样保留，并在标准错误输出一行
因此格式化工具的缺陷只会让你失去格式，而不会失去代码

出于同样的原因，完全无法词法分析的文件会被报告并跳过，而不是让 `--check` 失败
运行 `erg fmt` 无法修复它，因此让 CI 中断只是把 `erg check` 已经说得更清楚的话重复一遍

## 关闭格式化

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
| `# fmt: off` | 从这一行起停止格式化 |
| `# fmt: on` | 恢复格式化(若 `off` 未被关闭，则一直到文件末尾) |
| `# fmt: skip` | 不格式化这一行(若括号使其跨越多行，则为整个逻辑行) |

拼写必须完全一致; `#fmt:off` 和 `# FMT: OFF` 只是普通注释
一个看起来生效实际却不生效的指令，比一个明显什么都不做的指令更糟
唯一被忽略的是行尾空白，因为编辑器并不显示它

## 选项

| 选项 | 默认值 | 含义 |
| ---- | ------ | ---- |
| `--indent <n>` | `4` | 每层缩进的空格数(1 以上) |
| `--max-blank-lines <n>` | `2` | 保留的连续空行上限 |
| `--max-width <n>` | `100` | 格式化工具尽量不超过的列数 |
| `--check` | | 不写入任何内容; 若有文件会变化则以 1 退出 |
| `--stdout` | | 写到标准输出而不是写回文件 |
| `--exclude <substr>` | | 跳过包含该子串的路径(可重复指定) |

没有配置文件，选项就这六个
格式化工具的价值在于终结关于排版的争论，
而如果每个项目都能给出不同的答案，它就做不到这一点

## 在编辑器中使用

语言服务器(`erg server`)实现了 `textDocument/formatting`，
因此保存时格式化无需额外配置即可工作。缩进宽度使用编辑器自身的设置，而非 `--indent`
