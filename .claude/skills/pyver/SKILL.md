---
name: pyver
description: Switch Python target version for Erg compilation and testing
argument-hint: "<version> [compile|run|test|dis|compare|opcodes] [file]"
disable-model-invocation: true
allowed-tools: Bash(cargo run:*), Bash(cargo build:*), Bash(*python*:*), Bash(tests/bytecode*/run_tests.sh:*), Bash(bash tests/bytecode*:*), Bash(ls:*), Bash(uv:*)
---

# Python Version Switcher

Compile, run, or test Erg code targeting a specific Python version.

## Available Interpreters (uv)

| Version | Path |
| ------- | ---- |
| 3.10 | `~/.local/share/uv/python/cpython-3.10.19-linux-x86_64-gnu/bin/python3.10` |
| 3.11 | `~/.local/share/uv/python/cpython-3.11.14-linux-x86_64-gnu/bin/python3.11` |
| 3.12 | `~/.local/share/uv/python/cpython-3.12.11-linux-x86_64-gnu/bin/python3.12` |
| 3.13 | `~/.local/share/uv/python/cpython-3.13.12-linux-x86_64-gnu/bin/python3.13` |
| 3.14 | `~/.local/share/uv/python/cpython-3.14.0-linux-x86_64-gnu/bin/python3.14` |

Install new versions: `uv python install 3.XX`

## Usage

### `/pyver 3.12 run <file.er>` — Compile & run

```bash
PY=~/.local/share/uv/python/cpython-3.12.*/bin/python3.12
cargo run -- --py-command "$PY" <file.er>
```

### `/pyver 3.12 compile <file.er>` — Compile only (.pyc)

```bash
PY=~/.local/share/uv/python/cpython-3.12.*/bin/python3.12
cargo run -- --py-command "$PY" --mode compile <file.er>
```

### `/pyver 3.12 test` — Run bytecode test suite

```bash
bash tests/bytecode312/run_tests.sh
```

### `/pyver 3.12 dis <file.er>` — Compile then disassemble

```bash
PY=~/.local/share/uv/python/cpython-3.12.*/bin/python3.12
cargo run -- --py-command "$PY" --mode compile <file.er>
"$PY" -c "
import dis, marshal
with open('<file>.pyc', 'rb') as f:
    f.read(16)
    code = marshal.load(f)
dis.dis(code)
"
```

### `/pyver 3.12 compare [test_name]` — Compare Erg vs CPython bytecode

```bash
bash tests/bytecode312/compare_bytecode.sh [test_name]
```

### `/pyver 3.13 opcodes` — Dump opcode table from CPython

```bash
PY=~/.local/share/uv/python/cpython-3.13.*/bin/python3.13
"$PY" -c "import opcode; print(sorted((v,k) for k,v in opcode.opmap.items()))"
"$PY" -c "import opcode; print('HAVE_ARGUMENT:', opcode.HAVE_ARGUMENT)"
```

### `/pyver list` — Show installed versions

```bash
ls ~/.local/share/uv/python/
```

## Interpreter Resolution

1. Parse version from first argument (e.g. `3.12`)
2. Find `~/.local/share/uv/python/cpython-{version}*/bin/python{major}.{minor}`
3. Pass via `--py-command` flag to Erg

## Key Erg Flags

| Flag | Purpose |
| ---- | ------- |
| `--py-command <path>` | Target interpreter (sets magic number + version) |
| `--target-version <X.Y.Z>` | Set version directly (no interpreter needed) |
| `--mode compile` | Generate .pyc without executing |

## Bytecode Test Suites

| Version | Directory | Status |
| ------- | --------- | ------ |
| 3.12 | `tests/bytecode312/` | 13 pass |
| 3.13 | `tests/bytecode313/` | WIP (segfault — cache + stack layout) |
| 3.14 | `tests/bytecode314/` | 13 pass |

Each suite contains:
- `*.er` — Erg test files
- `python_ref/*.py` — CPython reference implementations
- `run_tests.sh` — Compile + run all tests
- `compare_bytecode.sh` — Disassemble and diff Erg vs CPython bytecode

## Debugging Bytecode Issues

Use the `bytecode-codegen` agent for in-depth debugging. Quick checks:

1. **Segfault (139)**: Wrong opcode values or missing CACHE entries
2. **NameError on preamble vars**: CommonOpcode not translated (write_instr vs write_opcode)
3. **dis works but exec fails**: CACHE entries or stack layout mismatch
4. **Compare bytecode**: `bash tests/bytecodeNNN/compare_bytecode.sh [test_name]`
