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

### --build-features

Display features enabled at compiler build time.

### -c, --code

Specify code to execute.

### --dump-as-pyc

Output compile results as a `.pyc` file.

### -? , -h, --help

Display help.

### --mode

Specify a subcommand.

### -m, --module

Specify a module to run.

### --no-std

Compile without Erg standard library.

### -o, --opt-level

Specify the optimization level, from 0 to 3.

### --output-dir, --dest

Specify the output directory for the compiled output.

### -p, --python-version

Specify the Python version. The version number is a 32-bit unsigned integer and should be selected from [this list](https://github.com/google/pytype/blob/main/pytype/pyc/magic.py).

### --py-command, --python-command

Specifies the Python interpreter to use. Default is `python3` on Unix and `python` on Windows.

### --py-server-timeout

Specifies timeout for REPL execution. Default is 10 seconds.

### --quiet-startup, --quiet-repl

Stop displaying processor information at REPL startup.

### -t, --show-type

Show type information with REPL execution results.

### --target-version

Specify the version of the pyc file to output. The version follows semantic versioning.

### -V, --version

Display the version.

### --verbose

Controls the verbosity of the compiler output, which can be from 0 to 2.
Note that warnings cannot be turned off, even if this is set to 0.

### --

Specifies runtime arguments.
