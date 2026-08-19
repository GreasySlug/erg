# Command line arguments

## subcommands

Each name below is also accepted by `--mode`, and the aliases in parentheses
are interchangeable with it.

### lex (lexer)

prints the result of lexical analysis.

### parse (parser)

Print parsing results.

### desugar (desugarer)

Print the AST after desugaring, with nested variables expanded and patterns
rewritten.

### typecheck (lower, tc)

Print type checking results.

### check (fullcheck, checker)

Run every check -- typing, side effects and ownership -- without generating
code.

### compile (comp, compiler)

Execute compilation.

### transpile (trans, transpiler)

Convert to Python script.

### run (exec, execute)

Display the result of execution. This is the default, so the name may be
omitted.

### read (byteread, reader, dis)

Deserialize a `.pyc` file and dump its code object.

### server (language-server)

Starts the language server.

### lint (linter)

Lint the program.

### fmt (format, formatter)

Reformat source code. See [tools/fmt.md](./tools/fmt.md).

### pack (package)

Run the package manager. See [tools/pack.md](./tools/pack.md).

### pydecl (py-decl)

Generate Erg declarations (`.d.er`) from Python sources and type stubs.

## options

Options go *before* the file path. Anything after it is treated as an argument
to the script.

### --build-features

Display features enabled at compiler build time.

### --check

`erg fmt` only. Report which files would change and write none; exit 1 if any
would. See [tools/fmt.md](./tools/fmt.md).

### -c, --code

Specify code to execute.

### --decls-from-py

Generate Erg declarations from annotated `.py` sources when no `.d.er` or
`.pyi` stub is found. Requires the `pydecl` build feature.

### --exclude

`erg fmt` only. Skip paths containing this substring. May be given more than
once.

### -? , -h, --help

Display help.

### --hex-py-magic-num, --hex-python-magic-number

Specify the target Python bytecode magic number as its first two bytes, in
hexadecimal.

### --indent

`erg fmt` only. Spaces per level of nesting, 1 or more. Default is 4.

### --max-blank-lines

`erg fmt` only. Longest run of blank lines kept. Default is 2.

### --max-width

`erg fmt` only. Column the formatter tries to stay within. Default is 100.

### --mode

Specify a subcommand.

### -m, --module

Specify a module to run.

### --no-std

Compile without Erg standard library.

### -o, --opt-level, --optimization-level

Specify the optimization level, from 0 to 3.

### --output-dir, --dest, --dist, --dest-dir, --dist-dir

Specify the output directory for the compiled output.

### --ping

Print `pong` and exit, to check that the executable runs.

### --ps1

The REPL's prompt. Default is `>>> `.

### --ps2

The REPL's prompt for a continued line. Default is `... `.

### --py-command, --python-command

Specifies the Python interpreter to use. Default is `python3` on Unix and `python` on Windows.

### --py-magic-num, --python-magic-number

Specify the target Python bytecode magic number, as a 32-bit unsigned integer.

### --py-server-timeout

Specifies timeout for REPL execution. Default is 10 seconds.

### -q, --quiet-startup, --quiet-repl

Stop displaying processor information at REPL startup.

### -t, --show-type

Show type information with REPL execution results.

### --stdout

`erg fmt` only. Write the result to standard output instead of back to the
file.

### --target-version

Specify the version of the pyc file to output. The version follows semantic versioning.

### --transpile-target, --target

What `erg transpile` emits: `python` (`py`), `json` or `toml`.

### --use-local-package

Add a package from a local path. Takes its name, the name to import it as, a
version, and the path.

### --use-package

Add a package. Takes its name, the name to import it as, and a version.

### --use-pylyzer

Use the external pylyzer to generate declarations for Python modules that have
no type information.

### -v, --verbose

Controls the verbosity of the compiler output, which can be from 0 to 2.
Note that warnings cannot be turned off, even if this is set to 0.

### -V, --version

Display the version.

### --

Specifies runtime arguments.
