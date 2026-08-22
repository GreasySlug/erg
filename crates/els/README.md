# els (erg-language-server)

ELS is a language server for the [Erg](https://github.com/erg-lang/erg) programming language.

## Features

- [x] Syntax highlighting (by [vscode-erg](https://github.com/erg-lang/vscode-erg))
- [x] Code completion
  - [x] Variable completion
  - [x] Method/attribute completion
  - [x] Smart completion (considering type, parameter names, etc.)
  - [x] Auto-import
- [x] Diagnostics
- [x] Hover
- [x] Go to definition
- [x] Go to declaration
- [x] Go to type definition
- [x] Go to implementation
- [x] Type hierarchy
- [x] Find references
- [x] Renaming
- [x] Inlay hint
- [x] Semantic tokens
- [x] Code actions
  - [x] eliminate unused variables
  - [x] change variable case
  - [x] extract variables/functions
  - [x] inline variables
- [x] Code lens
  - [x] show trait implementations
- [x] Signature help
- [x] Workspace symbol
- [x] Document symbol
- [x] Document highlight
- [x] Document link
- [x] Call hierarchy
- [x] Folding range
  - [x] Folding imports
  - [x] Folding functions, methods, classes, and other blocks
- [x] Selection range
- [x] Formatting (`erg_fmt`; no type checking, so it works on a file mid-edit)
  - [x] Document formatting
  - [x] Range formatting

## Installation

```console
cargo install erg --features els
```
