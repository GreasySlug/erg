use erg_common::error::{ErrorCore, ErrorKind, Location, SubMessage};
use erg_common::io::Input;
use erg_common::switch_lang;
use erg_common::traits::NoTypeDisplay;
use erg_compiler::error::CompileWarning;
use erg_compiler::hir::Expr;

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

pub(crate) fn contradiction(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    expr: Expr,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "この比較は常に偽になります",
            "simplified_chinese" => "此比较始终为假",
            "traditional_chinese" => "此比較始終為假",
            "english" => "this comparison is always false",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => format!("`{}` は自身と等しいので、この条件は決して成立しません", expr.to_string_notype()),
            "simplified_chinese" => format!("`{}` 等于自身, 因此该条件永远不成立", expr.to_string_notype()),
            "traditional_chinese" => format!("`{}` 等於自身, 因此該條件永遠不成立", expr.to_string_notype()),
            "english" => format!("`{}` is equal to itself, so this condition never holds", expr.to_string_notype()),
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

pub(crate) fn magic_number(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    constant_name: &str,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "よく知られた定数がハードコードされています",
            "simplified_chinese" => "硬编码了一个著名常量",
            "traditional_chinese" => "硬編碼了一個著名常數",
            "english" => "a well-known constant is hardcoded",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => format!("`{constant_name}` を使うとより正確で読みやすくなります"),
            "simplified_chinese" => format!("使用 `{constant_name}` 会更精确且易读"),
            "traditional_chinese" => format!("使用 `{constant_name}` 會更精確且易讀"),
            "english" => format!("use `{constant_name}` for more precision and readability"),
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

pub(crate) fn double_negation(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    expr: Expr,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "二重否定は冗長です",
            "simplified_chinese" => "双重否定是多余的",
            "traditional_chinese" => "雙重否定是多餘的",
            "english" => "double negation is redundant",
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

pub(crate) fn effect_free_proc(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    func_name: &str,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "この手続きには副作用がありません",
            "simplified_chinese" => "此过程没有副作用",
            "traditional_chinese" => "此程序沒有副作用",
            "english" => "this procedure has no side effects",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => format!("`!` を外して関数として定義できます: `{func_name}`"),
            "simplified_chinese" => format!("可以去掉 `!` 定义为函数: `{func_name}`"),
            "traditional_chinese" => format!("可以去掉 `!` 定義為函式: `{func_name}`"),
            "english" => format!("drop the `!` and define it as a function: `{func_name}`"),
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

pub(crate) fn unreachable_code(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "到達不能なコードです",
            "simplified_chinese" => "无法到达的代码",
            "traditional_chinese" => "無法到達的程式碼",
            "english" => "unreachable code",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => "直前の式が必ず脱出する（`return`/`panic` など）ため、ここは実行されません",
            "simplified_chinese" => "前一个表达式总会跳出（如 `return`/`panic`）, 因此这里不会执行",
            "traditional_chinese" => "前一個運算式總會跳出（如 `return`/`panic`）, 因此這裡不會執行",
            "english" => "the previous expression always diverges (e.g. `return`/`panic`), so this is never executed",
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

pub(crate) fn builtin_shadowing(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    name: &str,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => format!("引数 `{name}` が組み込みを隠しています"),
            "simplified_chinese" => format!("参数 `{name}` 遮蔽了内置名称"),
            "traditional_chinese" => format!("參數 `{name}` 遮蔽了內建名稱"),
            "english" => format!("the parameter `{name}` shadows a built-in"),
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => format!("組み込みの `{name}` と紛らわしいので、別の名前に変えましょう"),
            "simplified_chinese" => format!("为避免与内置 `{name}` 混淆, 请改用其他名称"),
            "traditional_chinese" => format!("為避免與內建 `{name}` 混淆, 請改用其他名稱"),
            "english" => format!("rename it to avoid confusion with the built-in `{name}`"),
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

pub(crate) fn nat_bound_comparison(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    result: bool,
) -> CompileWarning {
    let msg = if result {
        switch_lang!(
            "japanese" => "この比較は常に真になります",
            "simplified_chinese" => "此比较始终为真",
            "traditional_chinese" => "此比較始終為真",
            "english" => "this comparison is always true",
        )
    } else {
        switch_lang!(
            "japanese" => "この比較は常に偽になります",
            "simplified_chinese" => "此比较始终为假",
            "traditional_chinese" => "此比較始終為假",
            "english" => "this comparison is always false",
        )
    }
    .to_string();
    let hint = switch_lang!(
            "japanese" => "`Nat` は常に 0 以上です",
            "simplified_chinese" => "`Nat` 始终大于等于 0",
            "traditional_chinese" => "`Nat` 始終大於等於 0",
            "english" => "`Nat` values are never negative",
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

pub(crate) fn identity_op(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    suggestion: String,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "この演算は値を変えません",
            "simplified_chinese" => "此运算不会改变值",
            "traditional_chinese" => "此運算不會改變值",
            "english" => "this operation has no effect",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => format!("より簡潔に書きましょう: {suggestion}"),
            "simplified_chinese" => format!("更简洁地写作: {suggestion}"),
            "traditional_chinese" => format!("寫作更簡潔: {suggestion}"),
            "english" => format!("write more succinctly: {suggestion}"),
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

pub(crate) fn erasing_op(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "この演算結果は常に 0 です",
            "simplified_chinese" => "此运算结果始终为 0",
            "traditional_chinese" => "此運算結果始終為 0",
            "english" => "this operation is always zero",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => "`0` と書けば十分です",
            "simplified_chinese" => "写 `0` 就足够了",
            "traditional_chinese" => "寫 `0` 就足夠了",
            "english" => "just write `0`",
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

pub(crate) fn modulo_one(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "`% 1` の結果は常に 0 です",
            "simplified_chinese" => "`% 1` 的结果始终为 0",
            "traditional_chinese" => "`% 1` 的結果始終為 0",
            "english" => "`% 1` is always zero",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => "`0` と書けば十分です",
            "simplified_chinese" => "写 `0` 就足够了",
            "traditional_chinese" => "寫 `0` 就足夠了",
            "english" => "just write `0`",
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

pub(crate) fn needless_bool(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    suggestion: String,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "真偽値リテラルを返すだけの if 式は冗長です",
            "simplified_chinese" => "只返回布尔字面量的 if 表达式是多余的",
            "traditional_chinese" => "只回傳布林字面值的 if 運算式是多餘的",
            "english" => "this if-expression just returns a boolean literal",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => format!("より簡潔に書きましょう: {suggestion}"),
            "simplified_chinese" => format!("更简洁地写作: {suggestion}"),
            "traditional_chinese" => format!("寫作更簡潔: {suggestion}"),
            "english" => format!("write more succinctly: {suggestion}"),
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

pub(crate) fn redundant_branches(
    input: Input,
    errno: usize,
    caused_by: String,
    loc: Location,
    suggestion: String,
) -> CompileWarning {
    let msg = switch_lang!(
            "japanese" => "両方の分岐が同一です",
            "simplified_chinese" => "两个分支完全相同",
            "traditional_chinese" => "兩個分支完全相同",
            "english" => "both branches are identical",
    )
    .to_string();
    let hint = switch_lang!(
            "japanese" => format!("条件分岐は不要です。次のように書けます: {suggestion}"),
            "simplified_chinese" => format!("无需条件分支。可以写作: {suggestion}"),
            "traditional_chinese" => format!("不需要條件分支。可以寫作: {suggestion}"),
            "english" => format!("the condition is unnecessary; just write: {suggestion}"),
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
