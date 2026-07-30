use erg_common::error::{ErrorCore, ErrorKind::*, Location, SubMessage};
use erg_common::io::Input;
use erg_common::switch_lang;

use crate::error::*;

pub type EvalError = CompileError;
pub type EvalErrors = CompileErrors;
pub type EvalResult<T> = CompileResult<T>;
pub type SingleEvalResult<T> = SingleCompileResult<T>;

impl EvalError {
    pub fn not_const_expr(input: Input, errno: usize, loc: Location, caused_by: String) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => "定数式ではありません",
                    "simplified_chinese" => "不是常量表达式",
                    "traditional_chinese" => "不是常量表達式",
                    "english" => "not a constant expression",
                ),
                errno,
                NotConstExpr,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn recursion_error(input: Input, errno: usize, loc: Location, caused_by: String) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => "コンパイル時評価の再帰の深さが上限を超えました(定数定義が循環している可能性があります)",
                    "simplified_chinese" => "编译时求值的递归深度超过了上限(常量定义可能存在循环)",
                    "traditional_chinese" => "編譯時求值的遞迴深度超過了上限(常量定義可能存在循環)",
                    "english" => "recursion depth limit exceeded during compile-time evaluation (constant definitions may be cyclic)",
                ),
                errno,
                RecursionError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn cyclic_definition_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        cycle: &str,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("循環定義が検出されました: {cycle}"),
                    "simplified_chinese" => format!("检测到循环定义: {cycle}"),
                    "traditional_chinese" => format!("檢測到循環定義: {cycle}"),
                    "english" => format!("cyclic definition detected: {cycle}"),
                ),
                errno,
                RecursionError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn invalid_literal(input: Input, errno: usize, loc: Location, caused_by: String) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => "リテラルが不正です",
                    "simplified_chinese" => "字面量不合法",
                    "traditional_chinese" => "字面量不合法",
                    "english" => "invalid literal",
                ),
                errno,
                SyntaxError,
                loc,
            ),
            input,
            caused_by,
        )
    }
}
