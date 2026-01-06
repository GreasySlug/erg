use core::f64;

use erg_common::error::{ErrorCore, ErrorKind, Location, SubMessage};
use erg_common::io::Input;
use erg_common::switch_lang;
use erg_common::traits::NoTypeDisplay;
use erg_compiler::error::CompileWarning;
use erg_compiler::hir::Expr;

/// Known mathematical constants that should not be hardcoded
pub(crate) struct KnownConstant {
    pub value: f64,
    pub name: &'static str,
    pub module: &'static str,
    pub tolerance: f64,
}

impl KnownConstant {
    pub const fn new(value: f64, name: &'static str, module: &'static str, tolerance: f64) -> Self {
        Self {
            value,
            name,
            module,
            tolerance,
        }
    }

    pub fn matches(&self, value: f64) -> bool {
        if self.value.is_infinite() {
            return self.value == value;
        }
        (self.value - value).abs() < self.tolerance
    }

    pub fn suggestion(&self) -> String {
        format!("{}.{}", self.module, self.name)
    }
}

/// List of well-known constants that should not be hardcoded.
///
/// The suggestions point to Erg's `math` module equivalents.
/// More specific values (like pi/2) come before less specific ones (like pi)
/// to avoid false matches when tolerance ranges overlap.
pub(crate) const KNOWN_CONSTANTS: &[KnownConstant] = &[
    KnownConstant::new(f64::consts::FRAC_PI_8, "pi / 8", "math", 1e-10),
    KnownConstant::new(f64::consts::FRAC_PI_6, "pi / 6", "math", 1e-10),
    KnownConstant::new(f64::consts::FRAC_PI_4, "pi / 4", "math", 1e-10),
    KnownConstant::new(f64::consts::FRAC_PI_3, "pi / 3", "math", 1e-10),
    KnownConstant::new(f64::consts::FRAC_PI_2, "pi / 2", "math", 1e-10),
    KnownConstant::new(f64::consts::PI, "pi", "math", 1e-10),
    KnownConstant::new(f64::consts::TAU, "tau", "math", 1e-10),
    KnownConstant::new(f64::consts::FRAC_1_PI, "1 / pi", "math", 1e-10),
    KnownConstant::new(f64::consts::FRAC_2_PI, "2 / pi", "math", 1e-10),
    KnownConstant::new(f64::consts::FRAC_2_SQRT_PI, "2 / sqrt(pi)", "math", 1e-10),
    KnownConstant::new(f64::consts::E, "e", "math", 1e-10),
    KnownConstant::new(f64::consts::LOG2_E, "log2(e)", "math", 1e-10),
    KnownConstant::new(f64::consts::LOG10_E, "log10(e)", "math", 1e-10),
    KnownConstant::new(f64::consts::SQRT_2, "sqrt(2)", "math", 1e-10),
    KnownConstant::new(f64::consts::FRAC_1_SQRT_2, "1 / sqrt(2)", "math", 1e-10),
    KnownConstant::new(f64::consts::LN_2, "ln(2)", "math", 1e-10),
    KnownConstant::new(f64::consts::LN_10, "ln(10)", "math", 1e-10),
    KnownConstant::new(f64::INFINITY, "inf", "math", 0.0),
    KnownConstant::new(6.62607015e-34, "h", "consts.physics", 1e-42),
    KnownConstant::new(1.054571817e-34, "ħ", "consts.physics", 1e-42),
    KnownConstant::new(2.99792458e8, "c", "consts.physics", 1e-1),
    KnownConstant::new(8.8541878128e-12, "ε_0", "consts.physics", 1e-20),
    KnownConstant::new(1.25663706212e-6, "μ_0", "consts.physics", 1e-14),
    KnownConstant::new(9.80665, "g", "consts.physics", 1e-5),
    KnownConstant::new(6.67430e-11, "G", "consts.physics", 1e-16),
    KnownConstant::new(7.2973525693e-3, "α", "consts.physics", 1e-12),
    KnownConstant::new(1.380649e-23, "k_B", "consts.physics", 1e-30),
    KnownConstant::new(6.02214076e23, "N_A", "consts.physics", 1e16),
    KnownConstant::new(10973731.568160, "R_inf", "consts.physics", 1e-6),
];

pub(crate) fn too_many_params(input: Input, caused_by: String, loc: Location) -> CompileWarning {
    CompileWarning::new(
        ErrorCore::new(
            vec![],
            "too many parameters".to_string(),
            0,
            ErrorKind::Warning,
            loc,
        ),
        input,
        caused_by,
    )
}

pub(crate) fn tautology(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    expr: Expr,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "比較演算子が冗長です",
            "simplified_chinese" => "比较运算符是多余的",
            "traditional_chinese" => "比較運算符號是多餘的",
            "english" => "comparison operator is verbose",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => format!("より簡潔に書きましょう: {}", expr.to_string_notype()),
            "simplified_chinese" => format!("更简洁地写作: {}", expr.to_string_notype()),
            "traditional_chinese" => format!("寫作更簡潔: {}", expr.to_string_notype()),
            "english" => format!("write more succinctly: {}", expr.to_string_notype()),
    )
    .to_string();
    CompileWarning::new(
        ErrorCore::new(
            vec![SubMessage::ambiguous_new(loc, vec![], Some(hint))],
            msg,
            errno,
            ErrorKind::Warning,
            loc,
        ),
        input,
        caused_by,
    )
}

pub(crate) fn too_many_instance_attributes(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "インスタンス属性が多すぎます",
            "simplified_chinese" => "实例属性过多",
            "traditional_chinese" => "實例屬性過多",
            "english" => "too many instance attributes",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => "サブクラスやデータクラスを活用してください",
            "simplified_chinese" => "利用子类和数据类",
            "traditional_chinese" => "利用子類和資料類",
            "english" => "take advantage of subclasses or data classes",
    )
    .to_string();
    CompileWarning::new(
        ErrorCore::new(
            vec![SubMessage::ambiguous_new(loc, vec![], Some(hint))],
            msg,
            errno,
            ErrorKind::AttributeWarning,
            loc,
        ),
        input,
        caused_by,
    )
}

pub(crate) fn true_comparison(
    expr: &Expr,
    input: Input,
    caused_by: String,
    loc: Location,
) -> CompileWarning {
    CompileWarning::new(
        ErrorCore::new(
            vec![SubMessage::ambiguous_new(
                loc,
                vec![],
                Some(format!("just write: {}", expr.to_string_notype())),
            )],
            "equality checks against True are redundant".to_string(),
            0,
            ErrorKind::Warning,
            loc,
        ),
        input,
        caused_by,
    )
}

pub(crate) fn false_comparison(
    expr: &Expr,
    input: Input,
    caused_by: String,
    loc: Location,
) -> CompileWarning {
    CompileWarning::new(
        ErrorCore::new(
            vec![SubMessage::ambiguous_new(
                loc,
                vec![],
                Some(format!("just write: not {}", expr.to_string_notype())),
            )],
            "equality checks against False are redundant".to_string(),
            0,
            ErrorKind::Warning,
            loc,
        ),
        input,
        caused_by,
    )
}

pub(crate) fn hardcoded_constant(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    suggestion: &str,
) -> CompileWarning {
    let msg = switch_lang!(
        "japanese" => "ハードコーディングされた定数が検出されました",
        "simplified_chinese" => "检测到硬编码的常量",
        "traditional_chinese" => "檢測到硬編碼的常量",
        "english" => "detected hardcoded constant",
    )
    .to_string();
    let hint = switch_lang!(
        "japanese" => format!("代わりに {} を使用してください", suggestion),
        "simplified_chinese" => format!("请使用 {} 代替", suggestion),
        "traditional_chinese" => format!("請使用 {} 代替", suggestion),
        "english" => format!("use {} instead", suggestion),
    )
    .to_string();
    CompileWarning::new(
        ErrorCore::new(
            vec![SubMessage::ambiguous_new(loc, vec![], Some(hint))],
            msg,
            errno,
            ErrorKind::Warning,
            loc,
        ),
        input,
        caused_by,
    )
}
