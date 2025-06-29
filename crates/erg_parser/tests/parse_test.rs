use erg_common::config::ErgConfig;
use erg_common::consts::DEBUG_MODE;
use erg_common::error::MultiErrorDisplay;
use erg_common::io::Input;
use erg_common::spawn::exec_new_thread;
use erg_common::traits::Stream;

use erg_parser::error::{ErrorArtifact, ParseWarnings, ParserRunnerErrors};
use erg_parser::lex::Lexer;
use erg_parser::ParserRunner;

#[test]
fn parse_args() -> Result<(), ()> {
    expect_success("tests/args.er", 0)
}

#[test]
fn parse_containers() -> Result<(), ()> {
    expect_success("tests/containers.er", 0)
}

#[test]
fn parse_dependent() -> Result<(), ()> {
    expect_success("tests/dependent.er", 0)
}

#[test]
fn parse_fib() -> Result<(), ()> {
    expect_success("tests/fib.er", 0)
}

#[test]
fn parse_hello_world() -> Result<(), ()> {
    expect_success("tests/hello_world.er", 0)
}

#[test]
fn parse_simple_if() -> Result<(), ()> {
    expect_success("tests/simple_if.er", 0)
}

#[test]
fn parse_starless_mul() -> Result<(), ()> {
    expect_success("tests/starless_mul.er", 0)
}

#[test]
fn parse_stream() -> Result<(), ()> {
    expect_success("tests/stream.er", 0)
}

#[test]
fn parse_test1_basic_syntax() -> Result<(), ()> {
    expect_success("tests/test1_basic_syntax.er", 0)
}

#[test]
fn parse_test2_advanced_syntax() -> Result<(), ()> {
    expect_success("tests/test2_advanced_syntax.er", 0)
}

#[test]
fn parse_stack() -> Result<(), ()> {
    expect_failure("tests/stack.er", 0, 2)
}

#[test]
fn parse_str_literal() -> Result<(), ()> {
    expect_failure("tests/failed_str_lit.er", 0, 2)
}

#[test]
fn parse_invalid_chunk() -> Result<(), ()> {
    expect_failure("tests/invalid_chunk.er", 0, 60)
}

#[test]
fn parse_invalid_collections() -> Result<(), ()> {
    expect_failure("tests/invalid_collections.er", 0, 29)
}

#[test]
fn parse_invalid_class_definition() -> Result<(), ()> {
    expect_failure("tests/invalid_class_definition.er", 0, 7)
}

#[test]
fn parse_warns() -> Result<(), ()> {
    expect_success("tests/warns.er", 1)
}

fn _parse_test_from_code(
    file_path: &'static str,
) -> Result<ParseWarnings, ErrorArtifact<ParserRunnerErrors>> {
    let input = Input::file(file_path.into());
    let cfg = ErgConfig {
        input: input.clone(),
        py_server_timeout: 100,
        ..ErgConfig::default()
    };
    let lexer = Lexer::new(input.clone());
    let mut parser = ParserRunner::new(cfg);
    match parser.parse_token_stream(lexer.lex().map_err(|(_, errs)| {
        ErrorArtifact::new(
            ParserRunnerErrors::empty(),
            ParserRunnerErrors::convert(&input, errs),
        )
    })?) {
        Ok(artifact) => {
            if DEBUG_MODE {
                println!("{}", artifact.ast);
            }
            Ok(artifact.warns)
        }
        Err(artifact) => {
            if DEBUG_MODE {
                artifact.warns.write_all_stderr();
                artifact.errors.write_all_stderr();
            }
            Err(ErrorArtifact::new(artifact.warns, artifact.errors))
        }
    }
}

fn parse_test_from_code(
    file_path: &'static str,
) -> Result<ParseWarnings, ErrorArtifact<ParserRunnerErrors>> {
    exec_new_thread(move || _parse_test_from_code(file_path), file_path)
}

fn expect_success(file_path: &'static str, num_warns: usize) -> Result<(), ()> {
    match parse_test_from_code(file_path) {
        Ok(warns) => {
            if warns.len() == num_warns {
                Ok(())
            } else {
                println!(
                    "err: number of warnings is not {num_warns} but {}",
                    warns.len()
                );
                Err(())
            }
        }
        Err(_) => Err(()),
    }
}

fn expect_failure(file_path: &'static str, num_warns: usize, num_errs: usize) -> Result<(), ()> {
    match parse_test_from_code(file_path) {
        Ok(_) => Err(()),
        Err(eart) => match (eart.errors.len() == num_errs, eart.warns.len() == num_warns) {
            (true, true) => Ok(()),
            (true, false) => {
                println!(
                    "err: number of warnings is not {num_warns} but {}",
                    eart.warns.len()
                );
                Err(())
            }
            (false, true) => {
                println!(
                    "err: number of errors is not {num_errs} but {}",
                    eart.errors.len()
                );
                Err(())
            }
            (false, false) => {
                println!(
                    "err: number of warnings is not {num_warns} but {}",
                    eart.warns.len()
                );
                println!(
                    "err: number of errors is not {num_errs} but {}",
                    eart.errors.len()
                );
                Err(())
            }
        },
    }
}

fn parse_test_from_string(code: &str) -> Result<ParseWarnings, ErrorArtifact<ParserRunnerErrors>> {
    let input = Input::str(code.to_string());
    let cfg = ErgConfig {
        input: input.clone(),
        py_server_timeout: 100,
        ..ErgConfig::default()
    };
    let lexer = Lexer::new(input.clone());
    let mut parser = ParserRunner::new(cfg);
    match parser.parse_token_stream(lexer.lex().map_err(|(_, errs)| {
        ErrorArtifact::new(
            ParserRunnerErrors::empty(),
            ParserRunnerErrors::convert(&input, errs),
        )
    })?) {
        Ok(artifact) => {
            if DEBUG_MODE {
                println!("Successfully parsed: {}", artifact.ast);
            }
            Ok(artifact.warns)
        }
        Err(artifact) => {
            if DEBUG_MODE {
                println!("Parse failed for code: {}", code);
                artifact.warns.write_all_stderr();
                artifact.errors.write_all_stderr();
            }
            Err(ErrorArtifact::new(artifact.warns, artifact.errors))
        }
    }
}

fn expect_string_success(code: &str, num_warns: usize) -> Result<(), ()> {
    match parse_test_from_string(code) {
        Ok(warns) => {
            if warns.len() == num_warns {
                Ok(())
            } else {
                println!(
                    "err: number of warnings is not {num_warns} but {} for code: {}",
                    warns.len(),
                    code
                );
                Err(())
            }
        }
        Err(err) => {
            println!("expected success but got error for code: {}", code);
            err.errors.write_all_stderr();
            Err(())
        }
    }
}

fn expect_string_failure(code: &str, num_warns: usize, num_errs: usize) -> Result<(), ()> {
    match parse_test_from_string(code) {
        Ok(_) => {
            println!("expected failure but got success for code: {}", code);
            Err(())
        }
        Err(eart) => match (eart.errors.len() == num_errs, eart.warns.len() == num_warns) {
            (true, true) => Ok(()),
            (true, false) => {
                println!(
                    "err: number of warnings is not {num_warns} but {} for code: {}",
                    eart.warns.len(),
                    code
                );
                Err(())
            }
            (false, true) => {
                println!(
                    "err: number of errors is not {num_errs} but {} for code: {}",
                    eart.errors.len(),
                    code
                );
                Err(())
            }
            (false, false) => {
                println!(
                    "err: number of warnings is not {num_warns} but {} for code: {}",
                    eart.warns.len(),
                    code
                );
                println!(
                    "err: number of errors is not {num_errs} but {} for code: {}",
                    eart.errors.len(),
                    code
                );
                Err(())
            }
        },
    }
}

fn expect_string_error_contains(code: &str, expected_error: &str) -> Result<(), ()> {
    match parse_test_from_string(code) {
        Ok(_) => {
            println!("expected error but got success for code: {}", code);
            Err(())
        }
        Err(eart) => {
            let error_display = format!("{}", eart.errors);
            if error_display.contains(expected_error) {
                Ok(())
            } else {
                println!(
                    "expected error '{}' not found in error output for code: {}\nActual errors: {}",
                    expected_error, code, error_display
                );
                Err(())
            }
        }
    }
}

#[test]
fn test_parentheses_positive_basic_function_call() -> Result<(), ()> {
    expect_string_success("func(a, b)", 0)
}

#[test]
fn test_parentheses_positive_nested_calls() -> Result<(), ()> {
    expect_string_success("func(other(x), y)", 0)
}

#[test]
fn test_parentheses_positive_complex_expression() -> Result<(), ()> {
    expect_string_success("result = func(a + 1, b * 2)", 0)
}

#[test]
fn test_parentheses_positive_no_parentheses_call() -> Result<(), ()> {
    expect_string_success("func a, b", 0)
}

#[test]
fn test_parentheses_positive_empty_parentheses() -> Result<(), ()> {
    expect_string_success("func()", 0)
}

#[test]
fn test_parentheses_positive_mathematical_expression() -> Result<(), ()> {
    expect_string_success("x = (a + b) * (c + d)", 0)
}

#[test]
fn test_parentheses_positive_tuple_creation() -> Result<(), ()> {
    expect_string_success("tuple = (1, 2, 3)", 0)
}

#[test]
fn test_parentheses_positive_type_annotation() -> Result<(), ()> {
    expect_string_success("func(x: Int, y: Str): Bool = True", 0)
}

#[test]
fn test_parentheses_positive_lambda_expression() -> Result<(), ()> {
    expect_string_success("lambda = x -> (x + 1)", 0)
}

#[test]
fn test_parentheses_positive_nested_parentheses() -> Result<(), ()> {
    expect_string_success("result = func((a + b), (c * d))", 0)
}

#[test]
fn test_parentheses_positive_multiple_function_calls() -> Result<(), ()> {
    expect_string_success("a = func1(x)\nb = func2(y)", 0)
}

#[test]
fn test_parentheses_positive_function_definition() -> Result<(), ()> {
    expect_string_success("add(a: Int, b: Int): Int = a + b", 0)
}

// NEGATIVE TESTS - Invalid parentheses usage should produce specific error messages
// Note: Our fix specifically targets unmatched closing parentheses in function call contexts

#[test]
fn test_parentheses_negative_simple_unmatched_closing() -> Result<(), ()> {
    // This is the main case our fix targets
    expect_string_error_contains("func a, b)", "unmatched closing parenthesis")
}

#[test]
fn test_parentheses_negative_assignment_with_unmatched() -> Result<(), ()> {
    // Assignment context - our fix doesn't apply here, expect generic error
    expect_string_error_contains("x = func a, b)", "invalid syntax")
}

#[test]
fn test_parentheses_negative_expression_with_unmatched() -> Result<(), ()> {
    // Mathematical expressions fall back to generic "invalid syntax" - this is expected
    expect_string_error_contains("result = 1 + 2)", "invalid syntax")
}

#[test]
fn test_parentheses_negative_multiple_unmatched() -> Result<(), ()> {
    // Multiple function calls with unmatched parentheses
    expect_string_error_contains("func a, b)\nother x, y)", "unmatched closing parenthesis")
}

#[test]
fn test_parentheses_negative_function_definition_unmatched() -> Result<(), ()> {
    // Function definition with unmatched parentheses
    expect_string_error_contains("add a, b) = a + b", "unmatched closing parenthesis")
}

#[test]
fn test_parentheses_negative_complex_expression_unmatched() -> Result<(), ()> {
    // Unmatched closing parenthesis in complex expression
    expect_string_error_contains("result = (a + 1) * b)", "invalid syntax")
}

#[test]
fn test_parentheses_negative_nested_function_unmatched() -> Result<(), ()> {
    // Nested function call with unmatched closing parenthesis
    expect_string_error_contains("func(other a, b))", "invalid syntax")
}

#[test]
fn test_parentheses_negative_tuple_unmatched() -> Result<(), ()> {
    // Tuple context - our fix targets function calls at statement level
    expect_string_error_contains("tuple = 1, 2, 3)", "invalid syntax")
}

#[test]
fn test_parentheses_negative_lambda_unmatched() -> Result<(), ()> {
    // Lambda expression - different context, expect generic error
    expect_string_error_contains("lambda = x -> x + 1)", "invalid syntax")
}

#[test]
fn test_parentheses_negative_type_annotation_unmatched() -> Result<(), ()> {
    // Function definition with type annotation and unmatched parenthesis
    expect_string_error_contains(
        "func x: Int, y: Str) -> Bool = True",
        "unmatched closing parenthesis",
    )
}

// Error count tests - verifying the exact number of errors produced

#[test]
fn test_parentheses_error_count_single_unmatched() -> Result<(), ()> {
    // Should produce exactly 2 errors: unmatched paren + invalid syntax
    expect_string_failure("func a, b)", 0, 2)
}

#[test]
fn test_parentheses_error_count_multiple_unmatched() -> Result<(), ()> {
    // Should produce 4 errors: 2 unmatched parens + 2 invalid syntax
    expect_string_failure("func a, b)\nother x, y)", 0, 4)
}

#[test]
fn test_parentheses_error_count_no_error_on_valid() -> Result<(), ()> {
    // Should produce no errors
    expect_string_success("func(a, b)", 0)
}
