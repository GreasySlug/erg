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

    pub fn zero_division(input: Input, errno: usize, loc: Location, caused_by: String) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => "ゼロで除算しています",
                    "simplified_chinese" => "除以零",
                    "traditional_chinese" => "除以零",
                    "english" => "division by zero",
                ),
                errno,
                ZeroDivisionError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    /// The operator itself is fine, but this particular application cannot be
    /// folded at compile time (e.g. the result overflows `Int`/`Nat`).
    pub fn uncomputable_op(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        expr: String,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("`{expr}`をコンパイル時に評価できません"),
                    "simplified_chinese" => format!("无法在编译时求值`{expr}`"),
                    "traditional_chinese" => format!("無法在編譯時求值`{expr}`"),
                    "english" => format!("`{expr}` cannot be evaluated at compile time"),
                ),
                errno,
                NotConstExpr,
                loc,
            ),
            input,
            caused_by,
        )
    }

    /// A decimal literal whose exact value does not fit `ValueObj::Ratio`
    /// (`6.62607015e-34` needs a denominator of 10^42). Folding it as the nearest
    /// `f64` would make the constant differ from the same literal written inline,
    /// which is built exactly from its source text.
    pub fn inexact_ratio_literal(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        lit: String,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![],
                    Some(switch_lang!(
                        "japanese" => "実行時の値と一致しなくなるため畳み込めません。小文字の名前(実行時変数)に束縛してください".to_string(),
                        "simplified_chinese" => "无法折叠，因为它与运行时的值不一致。请绑定到小写名称(运行时变量)".to_string(),
                        "traditional_chinese" => "無法摺疊，因為它與執行時的值不一致。請繫結到小寫名稱(執行時變數)".to_string(),
                        "english" => "folding it would not match the run-time value; bind it to a lowercase name (a run-time variable) instead".to_string(),
                    )),
                )],
                switch_lang!(
                    "japanese" => format!("`{lit}`はコンパイル時に正確に表現できません"),
                    "simplified_chinese" => format!("`{lit}`无法在编译时精确表示"),
                    "traditional_chinese" => format!("`{lit}`無法在編譯時精確表示"),
                    "english" => format!("`{lit}` cannot be represented exactly at compile time"),
                ),
                errno,
                NotConstExpr,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn index_out_of_range(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        len: usize,
        index: String,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("要素数は{len}ですが、{index}番目の要素にアクセスしようとしています"),
                    "simplified_chinese" => format!("有{len}个元素，但试图访问第{index}个元素"),
                    "traditional_chinese" => format!("有{len}個元素，但試圖訪問第{index}個元素"),
                    "english" => format!("has {len} elements, but tried to access the element at {index}"),
                ),
                errno,
                IndexError,
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
