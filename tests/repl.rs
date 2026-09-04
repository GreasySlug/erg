mod common;

use common::expect_repl_failure;
use common::expect_repl_success;
use erg_common::python_util::exec_py;

#[test]
#[ignore]
fn exec_repl_helloworld() -> Result<(), ()> {
    expect_repl_success(
        "repl_hello",
        ["print! \"hello, world\"", "exit()"]
            .into_iter()
            .map(|x| x.to_string())
            .collect(),
    )
}

#[test]
#[ignore]
fn exec_repl_def_func() -> Result<(), ()> {
    expect_repl_success(
        "repl_def",
        ["f i =", "i + 1", "", "x = f 2", "assert x == 3", "exit()"]
            .into_iter()
            .map(|x| x.to_string())
            .collect(),
    )
}

#[test]
#[ignore]
fn exec_repl_for_loop() -> Result<(), ()> {
    expect_repl_success(
        "repl_for",
        ["for! 0..1, i =>", "print! i", "", "exit()"]
            .into_iter()
            .map(|line| line.to_string())
            .collect(),
    )
}

#[test]
#[ignore]
fn exec_repl_auto_indent_dedent_check() -> Result<(), ()> {
    expect_repl_success(
        "repl_auto_indent_dedent",
        [
            "for! 0..0, i =>",
            "for! 0..0, j =>",
            "for! 0..0, k =>",
            "for! 0..0, l =>",
            "print! \"hi\"",
            "# l indent",
            "", // dedent l
            "# k indent",
            "", // dedent k
            "# j indent",
            "", // dedent j
            "# i indent and `for!` loop finished",
            "",
            "# main",
            "exit()",
        ]
        .into_iter()
        .map(|line| line.to_string())
        .collect(),
    )
}

#[test]
#[ignore]
fn exec_repl_class_def() -> Result<(), ()> {
    // The method blocks of a class have to be in the same cell as its
    // definition, so the cell stays open after `C.`'s block ends (another
    // block may follow); the empty line at the outermost level evaluates it.
    expect_repl_success(
        "repl_auto_indent_dedent",
        [
            "C = Class()",
            "C.",
            "attr = 1",
            "", // closes the method block
            "", // evaluates the class cell
            "print! C.attr",
            ":exit",
        ]
        .into_iter()
        .map(|line| line.to_string())
        .collect(),
    )
}

#[test]
#[ignore]
fn exec_repl_class_def_with_deco() -> Result<(), ()> {
    expect_repl_success(
        "repl_auto_indent_dedent",
        [
            "@Inheritable",
            "C = Class{ x = Int }",
            "C.",
            "attr = 1",
            "", // closes the method block
            "", // evaluates the class cell
            "print! C.attr",
            ":exit",
        ]
        .into_iter()
        .map(|line| line.to_string())
        .collect(),
    )
}

#[test]
#[ignore]
fn exec_invalid_class_inheritable() -> Result<(), ()> {
    // `examples/class.er` entered line by line. Both classes, with all their
    // method blocks, form one cell: an empty line after a method block only
    // closes the block, the empty line at the outermost level evaluates.
    expect_repl_success(
        "repl_auto_indent_dedent",
        [
            "@Inheritable",
            "Point2d = Class{ ::[<: Self]x = Int; ::[<: Self]y = Int }",
            "Point2d::",
            "one = 1",
            "", // closes `Point2d::`
            "Point2d.",
            "zero = Point2d::one - 1",
            "", // closes `Point2d.`
            "Point3d = Inherit Point2d, Additional := { z = Int }",
            "Point3d.",
            "@Override",
            "new(x, y, z) =",
            "Point3d {x; y; z}",
            "", // closes `new`'s body
            "norm self = self::x**2 + self::y**2 + self::z**2",
            "", // closes `Point3d.`
            "", // evaluates the cell
            "p = Point3d.new 1, 2, 3",
            "print! p.norm()",
            ":exit",
        ]
        .into_iter()
        .map(|line| line.to_string())
        .collect(),
    )
}

#[test]
#[ignore]
fn exec_invalid_class_def() -> Result<(), ()> {
    // `C = a Class()` is a call to `a`, not a class definition, so it is
    // evaluated on its own line: NotConstExpr and NameError (`a`). The `C.`
    // block then has no class definition in its cell: NameError (`C`).
    // `print! C.attr` compiles against the failed `C` and fails at run time,
    // which is not a compile error.
    expect_repl_failure(
        "repl_auto_indent_dedent",
        [
            "C = a Class() # Invalid but pass the expect block",
            "C.",
            "attr = 1",
            "", // closes the method block
            "", // evaluates the cell
            "print! C.attr",
            ":exit",
        ]
        .into_iter()
        .map(|line| line.to_string())
        .collect(),
        3,
    )
}

#[test]
#[ignore]
fn exec_repl_invalid_indent() -> Result<(), ()> {
    // The first block is valid: auto-indentation follows the previous line,
    // so `2` lands at the 8 columns `1` was typed at, and the empty line
    // returns to the enclosing level (0) and evaluates. In the second block
    // the over-indented `print!` is an invalid indent; the parser reports two
    // errors for that cell.
    expect_repl_failure(
        "repl_invalid_indent",
        [
            "a =",
            "    1",
            "2",
            "",
            "x =>",
            "1",
            "    print! \"hi\"",
            "",
            "exit()",
        ]
        .into_iter()
        .map(|x| x.to_string())
        .collect(),
        2,
    )
}

#[test]
#[ignore]
fn exec_repl_invalid_def_after_the_at_sign() -> Result<(), ()> {
    expect_repl_failure(
        "repl_invalid_indent",
        ["@decorator", "a = 1", "", "exit()"]
            .into_iter()
            .map(|x| x.to_string())
            .collect(),
        1,
    )
}

#[test]
#[ignore]
fn exec_repl_server_mock_test() -> Result<(), ()> {
    assert_eq!(
        exec_py("src/scripts/repl_server_test.py", &[])
            .ok()
            .and_then(|s| s.code()),
        Some(0)
    );
    Ok(())
}
