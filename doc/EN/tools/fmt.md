# fmt subcommand

The erg command has a subcommand called fmt, which reformats source code.

```console
$ erg fmt src/          # rewrite every .er file under src/
$ erg fmt --check src/  # name the files that would change, write none
$ erg fmt --stdout a.er # write the result to standard output
```

`erg fmt` is the same as `erg --mode fmt`. Options go *before* the path, as
everywhere else in the erg command. Reading from a pipe or `-c` always writes to
standard output, since there is no file to write back to.

Given a directory, every `.er` file below it is formatted. Symlinks and
directories whose name starts with `.` are not followed: `.git` and `.venv` hold
code that is not yours to reformat, and a symlink to an ancestor is a cycle.

## Formatting never changes your program

The formatted output is lexed again and compared with the input, token for
token. If anything differs -- or if the output no longer lexes -- the file is
left exactly as it was and a line is written to standard error. A bug in the
formatter can therefore cost you formatting, but never code.

For the same reason, a file that does not lex at all is reported and skipped
rather than failing `--check`. Running `erg fmt` cannot fix it, so stopping CI
over it would only repeat, less clearly, what `erg check` already says.

## Turning it off

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

| directive | effect |
| --------- | ------ |
| `# fmt: off` | stop formatting from this line on |
| `# fmt: on` | resume (if `off` is never closed, it runs to end of file) |
| `# fmt: skip` | leave this line alone (the whole logical line, if brackets spread it over several) |

The spelling has to match exactly; `#fmt:off` and `# FMT: OFF` are ordinary
comments. A directive that looks like it works but does not is worse than one
that plainly does nothing. Trailing whitespace is the one thing ignored, since
an editor does not show it.

## Options

| option | default | meaning |
| ------ | ------- | ------- |
| `--indent <n>` | `4` | spaces per level of nesting (1 or more) |
| `--max-blank-lines <n>` | `2` | longest run of blank lines kept |
| `--max-width <n>` | `100` | column the formatter tries to stay within |
| `--check` | | write nothing; exit 1 if any file would change |
| `--stdout` | | write to standard output instead of back to the file |
| `--exclude <substr>` | | skip paths containing this substring (repeatable) |

There is no configuration file, and these six options are all there is. A
formatter earns its keep by ending arguments about layout, which it cannot do
if every project can answer them differently.

## In an editor

The language server (`erg server`) implements `textDocument/formatting`, so
format-on-save works with no further setup. It uses the editor's own indent
width rather than `--indent`.
