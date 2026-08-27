//! Version-specific opcode set for codegen.
//!
//! Selects the correct opcode constants for the target Python version so that
//! `codegen.rs` can use a single `OpcodeSetVersion` instead of branching on
//! `py_version.minor` everywhere. Add new CPython versions by:
//! 1. Adding `opcodeNNN.rs` (e.g. `opcode312.rs`) with that version's opcodes.
//! 2. Adding a variant here and in `from_python_version`.
//! 3. Implementing the dispatch in each method below.

use crate::opcode308::Opcode308;
use crate::opcode309::Opcode309;
use crate::opcode310::Opcode310;
use crate::opcode311::Opcode311;
use crate::opcode312::Opcode312;
use crate::opcode313::Opcode313;
use crate::opcode314::Opcode314;
use crate::python_util::PythonVersion;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OpcodeSetVersion {
    V308,
    V309,
    V310,
    #[default]
    V311,
    V312,
    V313,
    V314,
}

impl OpcodeSetVersion {
    pub fn from_python_version(v: PythonVersion) -> Self {
        match v.minor {
            Some(7) | Some(8) => Self::V308,
            Some(9) => Self::V309,
            Some(10) => Self::V310,
            Some(11) => Self::V311,
            Some(12) => Self::V312,
            Some(13) => Self::V313,
            Some(14) => Self::V314,
            _ if v.minor.map(|m| m >= 14).unwrap_or(false) => Self::V314,
            _ if v.minor.map(|m| m >= 12).unwrap_or(false) => Self::V312,
            _ if v.minor.map(|m| m >= 11).unwrap_or(false) => Self::V311,
            _ if v.minor.map(|m| m >= 10).unwrap_or(false) => Self::V310,
            _ => Self::V311, // default to 3.11
        }
    }

    pub fn minor(&self) -> Option<u8> {
        match self {
            Self::V308 => Some(8),
            Self::V309 => Some(9),
            Self::V310 => Some(10),
            Self::V311 => Some(11),
            Self::V312 => Some(12),
            Self::V313 => Some(13),
            Self::V314 => Some(14),
        }
    }

    /// True for 3.9+ (IS_OP, etc.).
    pub fn is_3_9_plus(&self) -> bool {
        self.minor().map(|m| m >= 9).unwrap_or(false)
    }

    /// True for 3.10+ (LOAD_ASSERTION_ERROR, BEFORE_WITH, word-sized jump args, etc.).
    pub fn is_3_10_plus(&self) -> bool {
        self.minor().map(|m| m >= 10).unwrap_or(false)
    }

    /// True for 3.11+ (CALL/PRECALL, COPY, RESUME, etc.).
    pub fn is_3_11_plus(&self) -> bool {
        self.minor().map(|m| m >= 11).unwrap_or(false)
    }

    /// True for 3.12+ (PRECALL removed in 3.12).
    pub fn is_3_12_plus(&self) -> bool {
        self.minor().map(|m| m >= 12).unwrap_or(false)
    }

    /// True if this version emits PRECALL before CALL (3.11 only; 3.12+ use CALL only).
    pub fn has_precall(&self) -> bool {
        self.minor() == Some(11)
    }

    /// Jump argument divisor: 3.10+ uses word-sized (2-byte) instruction units,
    /// so jump targets are divided by 2; 3.9 and below use byte offsets directly.
    pub fn jump_unit_size(&self) -> usize {
        if self.is_3_10_plus() {
            2
        } else {
            1
        }
    }

    /// True for 3.13+ (renumbered opcodes, new cache entries for jumps, etc.).
    pub fn is_3_13_plus(&self) -> bool {
        self.minor().map(|m| m >= 13).unwrap_or(false)
    }

    /// True for 3.14+ (BINARY_SUBSCR removed, SET_FUNCTION_ATTRIBUTE, POP_ITER, etc.).
    pub fn is_3_14_plus(&self) -> bool {
        self.minor().map(|m| m >= 14).unwrap_or(false)
    }

    /// CALL (3.11+) or CALL_FUNCTION (3.10 and below).
    pub fn call(&self) -> u8 {
        match self {
            Self::V308 | Self::V309 | Self::V310 => Opcode310::CALL_FUNCTION as u8,
            Self::V311 | Self::V312 => Opcode311::CALL as u8,
            Self::V313 => Opcode313::CALL as u8,
            Self::V314 => Opcode314::CALL as u8,
        }
    }

    /// PRECALL (3.11 only); None for 3.12+.
    pub fn precall(&self) -> Option<u8> {
        if self.has_precall() {
            Some(Opcode311::PRECALL as u8)
        } else {
            None
        }
    }

    /// COPY (3.11+) or DUP_TOP (3.10 and below).
    pub fn copy(&self) -> u8 {
        match self {
            Self::V308 | Self::V309 | Self::V310 => Opcode310::DUP_TOP as u8,
            Self::V311 | Self::V312 => Opcode311::COPY as u8,
            Self::V313 => Opcode313::COPY as u8,
            Self::V314 => Opcode314::COPY as u8,
        }
    }

    /// SWAP (3.11+) or ROT_TWO (3.10 and below).
    pub fn swap(&self) -> u8 {
        match self {
            Self::V308 | Self::V309 | Self::V310 => Opcode310::ROT_TWO as u8,
            Self::V311 | Self::V312 => Opcode311::SWAP as u8,
            Self::V313 => Opcode313::SWAP as u8,
            Self::V314 => Opcode314::SWAP as u8,
        }
    }

    pub fn load_deref(&self) -> u8 {
        match self {
            Self::V308 | Self::V309 | Self::V310 => Opcode310::LOAD_DEREF as u8,
            Self::V311 => Opcode311::LOAD_DEREF as u8,
            Self::V312 => Opcode312::LOAD_DEREF as u8,
            Self::V313 => Opcode313::LOAD_DEREF as u8,
            Self::V314 => Opcode314::LOAD_DEREF as u8,
        }
    }

    pub fn store_deref(&self) -> u8 {
        match self {
            Self::V308 | Self::V309 | Self::V310 => Opcode310::STORE_DEREF as u8,
            Self::V311 => Opcode311::STORE_DEREF as u8,
            Self::V312 => Opcode312::STORE_DEREF as u8,
            Self::V313 => Opcode313::STORE_DEREF as u8,
            Self::V314 => Opcode314::STORE_DEREF as u8,
        }
    }

    pub fn load_closure(&self) -> u8 {
        match self {
            Self::V308 | Self::V309 | Self::V310 => Opcode310::LOAD_CLOSURE as u8,
            Self::V311 => Opcode311::LOAD_CLOSURE as u8,
            Self::V312 => Opcode312::LOAD_CLOSURE as u8,
            // 3.13+: LOAD_CLOSURE is a pseudo-op; maps to LOAD_FAST in bytecode
            Self::V313 => Opcode313::LOAD_FAST as u8,
            // 3.14: LOAD_CLOSURE is a pseudo-op; maps to LOAD_FAST in bytecode
            Self::V314 => Opcode314::LOAD_FAST as u8,
        }
    }

    pub fn resume(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::RESUME as u8,
            Self::V312 => Opcode312::RESUME as u8,
            Self::V313 => Opcode313::RESUME as u8,
            Self::V314 => Opcode314::RESUME as u8,
            _ => Opcode310::NOP as u8, // no RESUME before 3.11
        }
    }

    pub fn copy_free_vars(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::COPY_FREE_VARS as u8,
            Self::V312 => Opcode312::COPY_FREE_VARS as u8,
            Self::V313 => Opcode313::COPY_FREE_VARS as u8,
            Self::V314 => Opcode314::COPY_FREE_VARS as u8,
            _ => Opcode310::NOP as u8,
        }
    }

    /// MAKE_CELL (3.11+). Not used for 3.10 and below; returns 0.
    pub fn make_cell(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::MAKE_CELL as u8,
            Self::V312 => Opcode312::MAKE_CELL as u8,
            Self::V313 => Opcode313::MAKE_CELL as u8,
            Self::V314 => Opcode314::MAKE_CELL as u8,
            _ => 0,
        }
    }

    /// Whether `byte` is one of this version's jump opcodes.
    ///
    /// A sanity check for the places that patch a jump argument after the fact:
    /// `CommonOpcode::is_jump_op` reads the 3.7-3.12 numbering, which 3.13
    /// renumbered out from under it.
    pub fn is_jump_op(&self, byte: u8) -> bool {
        [
            self.jump_forward(),
            self.jump_backward(),
            self.pop_jump_if_false(),
            self.pop_jump_if_true(),
            self.pop_jump_forward_if_false(),
            self.jump_if_true_or_pop(),
            self.jump_if_false_or_pop(),
        ]
        .contains(&byte)
    }

    pub fn jump_forward(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::JUMP_FORWARD as u8,
            Self::V309 => Opcode309::JUMP_FORWARD as u8,
            Self::V310 => Opcode310::JUMP_FORWARD as u8,
            Self::V311 => Opcode311::JUMP_FORWARD as u8,
            Self::V312 => Opcode312::JUMP_FORWARD as u8,
            Self::V313 => Opcode313::JUMP_FORWARD as u8,
            Self::V314 => Opcode314::JUMP_FORWARD as u8,
        }
    }

    pub fn jump_backward(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::JUMP_BACKWARD as u8,
            Self::V312 => Opcode312::JUMP_BACKWARD as u8,
            Self::V313 => Opcode313::JUMP_BACKWARD as u8,
            Self::V314 => Opcode314::JUMP_BACKWARD as u8,
            _ => Opcode309::JUMP_ABSOLUTE as u8,
        }
    }

    pub fn pop_jump_forward_if_false(&self) -> u8 {
        match self {
            Self::V310 => Opcode310::POP_JUMP_IF_FALSE as u8,
            Self::V311 => Opcode311::POP_JUMP_FORWARD_IF_FALSE as u8,
            Self::V312 => Opcode312::POP_JUMP_IF_FALSE as u8,
            // 3.13+: only POP_JUMP_IF_FALSE exists (forward-only, like 3.12)
            Self::V313 => Opcode313::POP_JUMP_IF_FALSE as u8,
            // 3.14: only POP_JUMP_IF_FALSE exists (forward-only, like 3.12/3.13)
            Self::V314 => Opcode314::POP_JUMP_IF_FALSE as u8,
            _ => Opcode310::POP_JUMP_IF_FALSE as u8,
        }
    }

    pub fn pop_jump_forward_if_true(&self) -> u8 {
        match self {
            Self::V310 => Opcode310::POP_JUMP_IF_TRUE as u8,
            Self::V311 => Opcode311::POP_JUMP_FORWARD_IF_TRUE as u8,
            Self::V312 => Opcode312::POP_JUMP_IF_TRUE as u8,
            // 3.13+: only POP_JUMP_IF_TRUE exists (forward-only, like 3.12)
            Self::V313 => Opcode313::POP_JUMP_IF_TRUE as u8,
            // 3.14: only POP_JUMP_IF_TRUE exists (forward-only, like 3.12/3.13)
            Self::V314 => Opcode314::POP_JUMP_IF_TRUE as u8,
            _ => Opcode310::POP_JUMP_IF_TRUE as u8,
        }
    }

    pub fn pop_jump_backward_if_false(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::POP_JUMP_BACKWARD_IF_FALSE as u8,
            Self::V312 => Opcode312::POP_JUMP_IF_FALSE as u8,
            // 3.13+: no backward conditional jumps; use forward + JUMP_BACKWARD
            Self::V313 => Opcode313::POP_JUMP_IF_FALSE as u8,
            // 3.14: no backward conditional jumps; use POP_JUMP_IF_FALSE
            Self::V314 => Opcode314::POP_JUMP_IF_FALSE as u8,
            _ => Opcode310::POP_JUMP_IF_FALSE as u8,
        }
    }

    pub fn pop_jump_backward_if_true(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::POP_JUMP_BACKWARD_IF_TRUE as u8,
            Self::V312 => Opcode312::POP_JUMP_IF_TRUE as u8,
            // 3.13+: no backward conditional jumps; use forward + JUMP_BACKWARD
            Self::V313 => Opcode313::POP_JUMP_IF_TRUE as u8,
            // 3.14: no backward conditional jumps; use POP_JUMP_IF_TRUE
            Self::V314 => Opcode314::POP_JUMP_IF_TRUE as u8,
            _ => Opcode310::POP_JUMP_IF_TRUE as u8,
        }
    }

    pub fn push_null(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::PUSH_NULL as u8,
            Self::V312 => Opcode312::PUSH_NULL as u8,
            Self::V313 => Opcode313::PUSH_NULL as u8,
            Self::V314 => Opcode314::PUSH_NULL as u8,
            _ => Opcode310::NOP as u8,
        }
    }

    pub fn kw_names(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::KW_NAMES as u8,
            Self::V312 => Opcode312::KW_NAMES as u8,
            // 3.13 removed KW_NAMES; use CALL_KW instead
            Self::V313 => 0,
            // 3.14: KW_NAMES removed; use CALL_KW instead
            Self::V314 => 0,
            _ => 0, // not used before 3.11
        }
    }

    /// CALL_KW (3.13+). Replaces KW_NAMES + CALL pattern.
    pub fn call_kw(&self) -> u8 {
        match self {
            Self::V313 => Opcode313::CALL_KW as u8,
            Self::V314 => Opcode314::CALL_KW as u8,
            _ => 0,
        }
    }

    pub fn before_with(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::SETUP_WITH as u8,
            Self::V309 => Opcode309::SETUP_WITH as u8,
            Self::V310 => Opcode310::BEFORE_WITH as u8,
            Self::V311 => Opcode311::BEFORE_WITH as u8,
            Self::V312 => Opcode312::BEFORE_WITH as u8,
            Self::V313 => Opcode313::BEFORE_WITH as u8,
            // 3.14: BEFORE_WITH removed; use LOAD_SPECIAL pattern
            Self::V314 => 0,
        }
    }

    /// `LOAD_SPECIAL` (3.14+), which replaced `BEFORE_WITH`: it looks a dunder up
    /// on the type and pushes it with its receiver, the way `LOAD_ATTR` does for
    /// a method. Oparg 0 is `__enter__`, 1 is `__exit__`.
    pub fn load_special(&self) -> u8 {
        match self {
            Self::V314 => Opcode314::LOAD_SPECIAL as u8,
            _ => 0,
        }
    }

    pub fn dict_merge(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::DICT_MERGE as u8,
            Self::V312 => Opcode312::DICT_MERGE as u8,
            Self::V313 => Opcode313::DICT_MERGE as u8,
            Self::V314 => Opcode314::DICT_MERGE as u8,
            _ => Opcode310::DICT_MERGE as u8,
        }
    }

    pub fn binary_subscr(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::BINARY_SUBSCR as u8,
            Self::V312 => Opcode312::BINARY_SUBSCR as u8,
            Self::V313 => Opcode313::BINARY_SUBSCR as u8,
            // 3.14: BINARY_SUBSCR removed; use BINARY_OP with NB_SUBSCR=26
            Self::V314 => 0,
            _ => Opcode310::BINARY_SUBSCR as u8,
        }
    }

    /// PRINT_EXPR (3.11 and below); 3.12+ uses CALL_INTRINSIC_1(PrintExpr=1).
    pub fn print_expr(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::PRINT_EXPR as u8,
            Self::V312 | Self::V313 | Self::V314 => 0, // removed in 3.12; use call_intrinsic_1()
            _ => Opcode310::PRINT_EXPR as u8,
        }
    }

    pub fn compare_op(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::COMPARE_OP as u8,
            Self::V312 => Opcode312::COMPARE_OP as u8,
            Self::V313 => Opcode313::COMPARE_OP as u8,
            Self::V314 => Opcode314::COMPARE_OP as u8,
            _ => Opcode310::COMPARE_OP as u8,
        }
    }

    /// CALL_METHOD (3.10 and below); not used in 3.11+ (use CALL instead).
    pub fn call_method(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::CALL_METHOD as u8,
            Self::V309 => Opcode309::CALL_METHOD as u8,
            Self::V310 => Opcode310::CALL_METHOD as u8,
            _ => 0, // not used in 3.11+
        }
    }

    /// CALL_FUNCTION (3.10 and below); 3.11+ uses CALL.
    pub fn call_function(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::CALL_FUNCTION as u8,
            Self::V309 => Opcode309::CALL_FUNCTION as u8,
            Self::V310 => Opcode310::CALL_FUNCTION as u8,
            _ => self.call(), // 3.11+ uses CALL
        }
    }

    /// CALL_FUNCTION_KW (3.10 and below); not used in 3.11+ (use KW_NAMES + CALL).
    pub fn call_function_kw(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::CALL_FUNCTION_KW as u8,
            Self::V309 => Opcode309::CALL_FUNCTION_KW as u8,
            Self::V310 => Opcode310::CALL_FUNCTION_KW as u8,
            _ => 0, // not used in 3.11+
        }
    }

    /// JUMP_ABSOLUTE (3.10 and below); 3.11+ uses JUMP_BACKWARD.
    pub fn jump_absolute(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::JUMP_ABSOLUTE as u8,
            Self::V309 => Opcode309::JUMP_ABSOLUTE as u8,
            Self::V310 => Opcode310::JUMP_ABSOLUTE as u8,
            _ => 0, // not used in 3.11+; use jump_backward() instead
        }
    }

    /// JUMP_IF_TRUE_OR_POP (3.11 and below).
    /// 3.12+ removed this; use COPY(1) + POP_JUMP_IF_TRUE + POP_TOP instead.
    pub fn jump_if_true_or_pop(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::JUMP_IF_TRUE_OR_POP as u8,
            Self::V309 => Opcode309::JUMP_IF_TRUE_OR_POP as u8,
            Self::V310 => Opcode310::JUMP_IF_TRUE_OR_POP as u8,
            Self::V311 => Opcode311::JUMP_IF_TRUE_OR_POP as u8,
            Self::V312 | Self::V313 | Self::V314 => 0, // removed in 3.12
        }
    }

    /// JUMP_IF_FALSE_OR_POP (3.11 and below).
    /// 3.12+ removed this; use COPY(1) + POP_JUMP_IF_FALSE + POP_TOP instead.
    pub fn jump_if_false_or_pop(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::JUMP_IF_FALSE_OR_POP as u8,
            Self::V309 => Opcode309::JUMP_IF_FALSE_OR_POP as u8,
            Self::V310 => Opcode310::JUMP_IF_FALSE_OR_POP as u8,
            Self::V311 => Opcode311::JUMP_IF_FALSE_OR_POP as u8,
            Self::V312 | Self::V313 | Self::V314 => 0, // removed in 3.12
        }
    }

    /// BINARY_OP (3.11+ only); pre-3.11 uses individual BINARY_ADD, etc.
    pub fn binary_op(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::BINARY_OP as u8,
            Self::V312 => Opcode312::BINARY_OP as u8,
            Self::V313 => Opcode313::BINARY_OP as u8,
            Self::V314 => Opcode314::BINARY_OP as u8,
            _ => 0, // not used pre-3.11
        }
    }

    /// IS_OP (3.9+); in 3.8, `is` is handled via COMPARE_OP.
    pub fn is_op(&self) -> u8 {
        match self {
            Self::V309 => Opcode309::IS_OP as u8,
            Self::V310 => Opcode310::IS_OP as u8,
            Self::V311 => Opcode311::IS_OP as u8,
            Self::V312 => Opcode312::IS_OP as u8,
            Self::V313 => Opcode313::IS_OP as u8,
            Self::V314 => Opcode314::IS_OP as u8,
            _ => 0, // 3.8: use COMPARE_OP with IsOp/IsNotOp args
        }
    }

    /// LOAD_ASSERTION_ERROR (3.10+); pre-3.10 loads AssertionError via LOAD_GLOBAL.
    pub fn load_assertion_error(&self) -> u8 {
        match self {
            Self::V310 => Opcode310::LOAD_ASSERTION_ERROR as u8,
            Self::V311 => Opcode311::LOAD_ASSERTION_ERROR as u8,
            Self::V312 => Opcode312::LOAD_ASSERTION_ERROR as u8,
            Self::V313 => Opcode313::LOAD_ASSERTION_ERROR as u8,
            // 3.14: LOAD_ASSERTION_ERROR removed; use LOAD_COMMON_CONSTANT
            Self::V314 => 0,
            _ => 0, // pre-3.10: use LOAD_GLOBAL("AssertionError")
        }
    }

    /// SETUP_WITH (3.7–3.9) / SETUP_WITH (3.10 as BEFORE_WITH for 3.10).
    /// Returns the raw opcode for WITH setup.
    pub fn setup_with(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::SETUP_WITH as u8,
            Self::V309 => Opcode309::SETUP_WITH as u8,
            Self::V310 => Opcode310::SETUP_WITH as u8,
            _ => 0, // 3.11+ uses BEFORE_WITH via before_with()
        }
    }

    /// RERAISE — all versions 3.9+ have this; 3.8 and below don't.
    pub fn reraise(&self) -> u8 {
        match self {
            Self::V309 => Opcode309::RERAISE as u8,
            Self::V310 => Opcode310::RERAISE as u8,
            Self::V311 => Opcode311::RERAISE as u8,
            Self::V312 => Opcode312::RERAISE as u8,
            Self::V313 => Opcode313::RERAISE as u8,
            Self::V314 => Opcode314::RERAISE as u8,
            _ => 0, // 3.8 doesn't have RERAISE
        }
    }

    /// WITH_EXCEPT_START (3.9+).
    pub fn with_except_start(&self) -> u8 {
        match self {
            Self::V309 => Opcode309::WITH_EXCEPT_START as u8,
            Self::V310 => Opcode310::WITH_EXCEPT_START as u8,
            Self::V311 => Opcode311::WITH_EXCEPT_START as u8,
            Self::V312 => Opcode312::WITH_EXCEPT_START as u8,
            Self::V313 => Opcode313::WITH_EXCEPT_START as u8,
            Self::V314 => Opcode314::WITH_EXCEPT_START as u8,
            _ => 0,
        }
    }

    /// POP_EXCEPT — all versions have this.
    pub fn pop_except(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::POP_EXCEPT as u8,
            Self::V309 => Opcode309::POP_EXCEPT as u8,
            Self::V310 => Opcode310::POP_EXCEPT as u8,
            Self::V311 => Opcode311::POP_EXCEPT as u8,
            Self::V312 => Opcode312::POP_EXCEPT as u8,
            Self::V313 => Opcode313::POP_EXCEPT as u8,
            Self::V314 => Opcode314::POP_EXCEPT as u8,
        }
    }

    /// PUSH_EXC_INFO (3.11+).
    pub fn push_exc_info(&self) -> u8 {
        match self {
            Self::V311 => Opcode311::PUSH_EXC_INFO as u8,
            Self::V312 => Opcode312::PUSH_EXC_INFO as u8,
            Self::V313 => Opcode313::PUSH_EXC_INFO as u8,
            Self::V314 => Opcode314::PUSH_EXC_INFO as u8,
            _ => 0,
        }
    }

    /// BUILD_TUPLE_UNPACK_WITH_CALL (3.8–3.9); replaced by LIST_EXTEND + LIST_TO_TUPLE in 3.9+.
    pub fn build_tuple_unpack_with_call(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::BUILD_TUPLE_UNPACK_WITH_CALL as u8,
            Self::V309 => Opcode309::BUILD_TUPLE_UNPACK_WITH_CALL as u8,
            _ => 0,
        }
    }

    /// DUP_TOP (3.10 and below); 3.11+ uses COPY(1).
    pub fn dup_top(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::DUP_TOP as u8,
            Self::V309 => Opcode309::DUP_TOP as u8,
            Self::V310 => Opcode310::DUP_TOP as u8,
            _ => self.copy(), // 3.11+ uses COPY
        }
    }

    /// ROT_TWO (3.10 and below); 3.11+ uses SWAP(2).
    pub fn rot_two(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::ROT_TWO as u8,
            Self::V309 => Opcode309::ROT_TWO as u8,
            Self::V310 => Opcode310::ROT_TWO as u8,
            _ => self.swap(), // 3.11+ uses SWAP
        }
    }

    /// POP_JUMP_IF_FALSE — all versions have this (same opcode number as POP_JUMP_FORWARD_IF_FALSE in 3.11).
    pub fn pop_jump_if_false(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::POP_JUMP_IF_FALSE as u8,
            Self::V309 => Opcode309::POP_JUMP_IF_FALSE as u8,
            Self::V310 => Opcode310::POP_JUMP_IF_FALSE as u8,
            _ => self.pop_jump_forward_if_false(),
        }
    }

    /// POP_JUMP_IF_TRUE — all versions have this (same opcode number as POP_JUMP_FORWARD_IF_TRUE in 3.11).
    pub fn pop_jump_if_true(&self) -> u8 {
        match self {
            Self::V308 => Opcode308::POP_JUMP_IF_TRUE as u8,
            Self::V309 => Opcode309::POP_JUMP_IF_TRUE as u8,
            Self::V310 => Opcode310::POP_JUMP_IF_TRUE as u8,
            _ => self.pop_jump_forward_if_true(),
        }
    }

    /// LIST_EXTEND (3.9+).
    pub fn list_extend(&self) -> u8 {
        match self {
            Self::V309 => Opcode309::LIST_EXTEND as u8,
            Self::V310 => Opcode310::LIST_EXTEND as u8,
            Self::V311 => Opcode311::LIST_EXTEND as u8,
            Self::V312 => Opcode312::LIST_EXTEND as u8,
            Self::V313 => Opcode313::LIST_EXTEND as u8,
            Self::V314 => Opcode314::LIST_EXTEND as u8,
            _ => 0,
        }
    }

    /// LIST_TO_TUPLE (3.9–3.11). 3.12+ uses CALL_INTRINSIC_1(ListToTuple=6).
    pub fn list_to_tuple(&self) -> u8 {
        match self {
            Self::V309 | Self::V310 => Opcode310::LIST_TO_TUPLE as u8,
            Self::V311 => Opcode311::LIST_TO_TUPLE as u8,
            Self::V312 | Self::V313 | Self::V314 => 0, // removed in 3.12; use call_intrinsic_1()
            _ => 0,
        }
    }

    /// CALL_INTRINSIC_1 (3.12+). Replaces PRINT_EXPR, IMPORT_STAR, LIST_TO_TUPLE, etc.
    pub fn call_intrinsic_1(&self) -> u8 {
        match self {
            Self::V312 => Opcode312::CALL_INTRINSIC_1 as u8,
            Self::V313 => Opcode313::CALL_INTRINSIC_1 as u8,
            Self::V314 => Opcode314::CALL_INTRINSIC_1 as u8,
            _ => 0,
        }
    }

    /// CALL_INTRINSIC_2 (3.12+).
    pub fn call_intrinsic_2(&self) -> u8 {
        match self {
            Self::V312 => Opcode312::CALL_INTRINSIC_2 as u8,
            Self::V313 => Opcode313::CALL_INTRINSIC_2 as u8,
            Self::V314 => Opcode314::CALL_INTRINSIC_2 as u8,
            _ => 0,
        }
    }

    /// RETURN_CONST (3.12+). Combines LOAD_CONST + RETURN_VALUE.
    pub fn return_const(&self) -> u8 {
        match self {
            Self::V312 => Opcode312::RETURN_CONST as u8,
            Self::V313 => Opcode313::RETURN_CONST as u8,
            // 3.14: RETURN_CONST removed
            Self::V314 => 0,
            _ => 0,
        }
    }

    /// END_FOR (3.12+). Used after FOR_ITER loops.
    pub fn end_for(&self) -> u8 {
        match self {
            Self::V312 => Opcode312::END_FOR as u8,
            Self::V313 => Opcode313::END_FOR as u8,
            Self::V314 => Opcode314::END_FOR as u8,
            _ => 0,
        }
    }

    /// TO_BOOL (3.13+). Converts TOS to bool before conditional jumps.
    /// Has 3 CACHE entries.
    pub fn to_bool(&self) -> u8 {
        match self {
            Self::V313 => Opcode313::TO_BOOL as u8,
            Self::V314 => Opcode314::TO_BOOL as u8,
            _ => 0,
        }
    }

    /// POP_ITER (3.14+). Pops iterator after END_FOR.
    pub fn pop_iter(&self) -> u8 {
        match self {
            Self::V314 => Opcode314::POP_ITER as u8,
            _ => 0,
        }
    }

    /// SET_FUNCTION_ATTRIBUTE (3.13+). Replaces MAKE_FUNCTION flags.
    /// Arg: 1=defaults, 2=kwdefaults, 4=annotations, 8=closure.
    pub fn set_function_attribute(&self) -> u8 {
        match self {
            Self::V313 => Opcode313::SET_FUNCTION_ATTRIBUTE as u8,
            Self::V314 => Opcode314::SET_FUNCTION_ATTRIBUTE as u8,
            _ => 0,
        }
    }

    /// LOAD_COMMON_CONSTANT (3.14+). Replaces LOAD_ASSERTION_ERROR.
    /// Arg: 0=AssertionError, 1=NotImplementedError.
    pub fn load_common_constant(&self) -> u8 {
        match self {
            Self::V314 => Opcode314::LOAD_COMMON_CONSTANT as u8,
            _ => 0,
        }
    }

    /// POP_JUMP_IF_NOT_NONE (3.12+).
    pub fn pop_jump_if_not_none(&self) -> u8 {
        match self {
            Self::V312 => Opcode312::POP_JUMP_IF_NOT_NONE as u8,
            Self::V313 => Opcode313::POP_JUMP_IF_NOT_NONE as u8,
            Self::V314 => Opcode314::POP_JUMP_IF_NOT_NONE as u8,
            _ => 0,
        }
    }

    /// POP_JUMP_IF_NONE (3.12+).
    pub fn pop_jump_if_none(&self) -> u8 {
        match self {
            Self::V312 => Opcode312::POP_JUMP_IF_NONE as u8,
            Self::V313 => Opcode313::POP_JUMP_IF_NONE as u8,
            Self::V314 => Opcode314::POP_JUMP_IF_NONE as u8,
            _ => 0,
        }
    }

    /// True if JUMP_IF_TRUE_OR_POP / JUMP_IF_FALSE_OR_POP exist in this version.
    /// 3.12+ removed them.
    pub fn has_jump_if_or_pop(&self) -> bool {
        !self.is_3_12_plus()
    }

    /// True if LOAD_METHOD exists as a separate opcode (3.11 and below).
    /// 3.12+ merged LOAD_METHOD into LOAD_ATTR (namei & 1 flag).
    pub fn has_load_method(&self) -> bool {
        !self.is_3_12_plus()
    }

    /// Encode COMPARE_OP argument for the target Python version.
    /// 3.11 and below: raw cmp value (0=LT, 1=LE, 2=EQ, 3=NE, 4=GT, 5=GE).
    /// 3.12: (cmp << 4) | compare_mask, where compare_mask encodes the fast-path.
    /// 3.13+: (cmp << 5) | compare_mask (operator selected via `oparg >> 5`, gh-100923).
    pub fn encode_compare_arg(&self, cmp: usize) -> usize {
        // CPython compare_masks: NOT_EQUALS=1, LESS_THAN=2, GREATER_THAN=4, EQUALS=8
        let mask = match cmp {
            0 => 2,         // LT: LESS_THAN
            1 => 2 | 8,     // LE: LESS_THAN | EQUALS
            2 => 8,         // EQ: EQUALS
            3 => 1 | 2 | 4, // NE: NOT_EQUALS | LESS_THAN | GREATER_THAN
            4 => 4,         // GT: GREATER_THAN
            5 => 4 | 8,     // GE: GREATER_THAN | EQUALS
            _ => 0,
        };
        if self.is_3_13_plus() {
            // 3.13+: (cmp << 5) | mask. In 3.13 the comparison operator moved to
            // `oparg >> 5` (3-bit operator + 4-bit branch mask, bit 4 reserved for
            // the bool-conversion flag). See CPython gh-100923.
            (cmp << 5) | mask
        } else if self.is_3_12_plus() {
            // 3.12: (cmp << 4) | mask
            (cmp << 4) | mask
        } else {
            cmp
        }
    }

    // ── inline cache sizes (number of 2-byte CACHE entries after an instruction) ──

    /// LOAD_GLOBAL cache entries: 3.11=5, 3.12+=4.
    pub fn cache_entries_load_global(&self) -> usize {
        if self.is_3_12_plus() {
            4
        } else {
            5
        }
    }

    /// LOAD_ATTR cache entries: 3.11=4, 3.12+=9.
    pub fn cache_entries_load_attr(&self) -> usize {
        if self.is_3_12_plus() {
            9
        } else {
            4
        }
    }

    /// COMPARE_OP cache entries: 3.11=2, 3.12+=1.
    pub fn cache_entries_compare_op(&self) -> usize {
        if self.is_3_12_plus() {
            1
        } else {
            2
        }
    }

    /// CALL cache entries: 3.11=4, 3.12+=3.
    pub fn cache_entries_call(&self) -> usize {
        if self.is_3_12_plus() {
            3
        } else {
            4
        }
    }

    /// BINARY_SUBSCR cache entries: 3.11=4, 3.12-3.13=1, 3.14+=0 (removed).
    pub fn cache_entries_binary_subscr(&self) -> usize {
        if self.is_3_14_plus() {
            0
        } else if self.is_3_12_plus() {
            1
        } else {
            4
        }
    }

    /// STORE_ATTR cache entries: 3.11=4, 3.12+=4 (unchanged).
    pub fn cache_entries_store_attr(&self) -> usize {
        4
    }

    /// BINARY_OP cache entries: 3.11-3.13=1, 3.14+=5.
    pub fn cache_entries_binary_op(&self) -> usize {
        if self.is_3_14_plus() {
            5
        } else {
            1
        }
    }

    /// FOR_ITER cache entries: 3.11=1, 3.12+=1 (unchanged).
    pub fn cache_entries_for_iter(&self) -> usize {
        1
    }

    /// PRECALL cache entries: 3.11 only=1. 3.12+ removed PRECALL.
    pub fn cache_entries_precall(&self) -> usize {
        if self.has_precall() {
            1
        } else {
            0
        }
    }

    /// CALL_KW cache entries: 3.14+=3, before that=0 (CALL_KW didn't exist or had no cache).
    pub fn cache_entries_call_kw(&self) -> usize {
        if self.is_3_14_plus() {
            3
        } else {
            0
        }
    }

    /// Translate a `CommonOpcode` value (3.7–3.12 numbering) to this version's opcode byte.
    /// For 3.12 and below, this is identity. For 3.13+, opcodes are renumbered.
    pub fn translate_common(&self, common: u8) -> u8 {
        if !self.is_3_13_plus() {
            return common;
        }
        if self.is_3_14_plus() {
            // Map CommonOpcode values → Python 3.14 opcode values
            return match common {
                1 => Opcode314::POP_TOP as u8,              // POP_TOP: 1→31
                9 => Opcode314::NOP as u8,                  // NOP: 9→27
                10 => Opcode314::NOP as u8,                 // UNARY_POSITIVE: removed
                11 => Opcode314::UNARY_NEGATIVE as u8,      // UNARY_NEGATIVE: 11→41
                12 => Opcode314::UNARY_NOT as u8,           // UNARY_NOT: 12→42
                15 => Opcode314::UNARY_INVERT as u8,        // UNARY_INVERT: 15→40
                60 => Opcode314::STORE_SUBSCR as u8,        // STORE_SUBSCR: 60→38
                68 => Opcode314::GET_ITER as u8,            // GET_ITER: 68→16
                69 => Opcode314::GET_YIELD_FROM_ITER as u8, // 69→19
                71 => Opcode314::LOAD_BUILD_CLASS as u8,    // LOAD_BUILD_CLASS: 71→21
                83 => Opcode314::RETURN_VALUE as u8,        // RETURN_VALUE: 83→35
                84 => Opcode314::NOP as u8,                 // IMPORT_STAR: removed
                86 => Opcode314::YIELD_VALUE as u8,         // YIELD_VALUE: 86→120
                87 => Opcode314::NOP as u8,                 // POP_BLOCK: removed
                89 => Opcode314::POP_EXCEPT as u8,          // POP_EXCEPT: 89→29
                90 => Opcode314::STORE_NAME as u8,          // STORE_NAME: 90→116
                91 => Opcode314::DELETE_NAME as u8,         // DELETE_NAME: 91→65
                93 => Opcode314::FOR_ITER as u8,            // FOR_ITER: 93→70
                94 => Opcode314::UNPACK_EX as u8,           // UNPACK_EX: 94→118
                95 => Opcode314::STORE_ATTR as u8,          // STORE_ATTR: 95→110
                97 => Opcode314::STORE_GLOBAL as u8,        // STORE_GLOBAL: 97→115
                100 => Opcode314::LOAD_CONST as u8,         // LOAD_CONST: 100→82
                101 => Opcode314::LOAD_NAME as u8,          // LOAD_NAME: 101→93
                102 => Opcode314::BUILD_TUPLE as u8,        // BUILD_TUPLE: 102→51
                103 => Opcode314::BUILD_LIST as u8,         // BUILD_LIST: 103→46
                104 => Opcode314::BUILD_SET as u8,          // BUILD_SET: 104→48
                105 => Opcode314::BUILD_MAP as u8,          // BUILD_MAP: 105→47
                106 => Opcode314::LOAD_ATTR as u8,          // LOAD_ATTR: 106→80
                107 => Opcode314::COMPARE_OP as u8,         // COMPARE_OP: 107→56
                108 => Opcode314::IMPORT_NAME as u8,        // IMPORT_NAME: 108→73
                109 => Opcode314::IMPORT_FROM as u8,        // IMPORT_FROM: 109→72
                116 => Opcode314::LOAD_GLOBAL as u8,        // LOAD_GLOBAL: 116→92
                118 => Opcode314::CONTAINS_OP as u8,        // CONTAINS_OP: 118→57
                124 => Opcode314::LOAD_FAST as u8,          // LOAD_FAST: 124→84
                125 => Opcode314::STORE_FAST as u8,         // STORE_FAST: 125→112
                126 => Opcode314::DELETE_FAST as u8,        // DELETE_FAST: 126→63
                130 => Opcode314::RAISE_VARARGS as u8,      // RAISE_VARARGS: 130→104
                132 => Opcode314::MAKE_FUNCTION as u8,      // MAKE_FUNCTION: 132→23
                142 => Opcode314::CALL_FUNCTION_EX as u8,   // CALL_FUNCTION_EX: 142→4
                144 => Opcode314::EXTENDED_ARG as u8,       // EXTENDED_ARG: 144→69
                156 => Opcode314::NOP as u8,                // BUILD_CONST_KEY_MAP: removed in 3.14
                157 => Opcode314::BUILD_STRING as u8,       // BUILD_STRING: 157→50
                160 => Opcode314::LOAD_ATTR as u8,          // LOAD_METHOD: merged into LOAD_ATTR
                other => other,
            };
        }
        // Map CommonOpcode values (3.11-era numbering) → Python 3.13 opcode values
        match common {
            1 => Opcode313::POP_TOP as u8,               // POP_TOP: 1→32
            9 => Opcode313::NOP as u8,                   // NOP: 9→30
            10 => 30, // UNARY_POSITIVE: removed in 3.13, map to NOP
            11 => Opcode313::UNARY_NEGATIVE as u8, // UNARY_NEGATIVE: 11→42
            12 => Opcode313::UNARY_NOT as u8, // UNARY_NOT: 12→43
            15 => Opcode313::UNARY_INVERT as u8, // UNARY_INVERT: 15→41
            60 => Opcode313::STORE_SUBSCR as u8, // STORE_SUBSCR: 60→39
            68 => Opcode313::GET_ITER as u8, // GET_ITER: 68→19
            69 => Opcode313::GET_YIELD_FROM_ITER as u8, // 69→21
            71 => Opcode313::LOAD_BUILD_CLASS as u8, // LOAD_BUILD_CLASS: 71→24
            83 => Opcode313::RETURN_VALUE as u8, // RETURN_VALUE: 83→36
            84 => 30, // IMPORT_STAR: removed, use CALL_INTRINSIC_1
            86 => Opcode313::YIELD_VALUE as u8, // YIELD_VALUE: 86→118
            87 => Opcode313::NOP as u8, // POP_BLOCK: removed
            89 => Opcode313::POP_EXCEPT as u8, // POP_EXCEPT: 89→31
            90 => Opcode313::STORE_NAME as u8, // STORE_NAME: 90→114
            91 => Opcode313::DELETE_NAME as u8, // DELETE_NAME: 91→67
            93 => Opcode313::FOR_ITER as u8, // FOR_ITER: 93→72
            94 => Opcode313::UNPACK_EX as u8, // UNPACK_EX: 94→116
            95 => Opcode313::STORE_ATTR as u8, // STORE_ATTR: 95→108
            97 => Opcode313::STORE_GLOBAL as u8, // STORE_GLOBAL: 97→113
            100 => Opcode313::LOAD_CONST as u8, // LOAD_CONST: 100→83
            101 => Opcode313::LOAD_NAME as u8, // LOAD_NAME: 101→92
            102 => Opcode313::BUILD_TUPLE as u8, // BUILD_TUPLE: 102→52
            103 => Opcode313::BUILD_LIST as u8, // BUILD_LIST: 103→47
            104 => Opcode313::BUILD_SET as u8, // BUILD_SET: 104→49
            105 => Opcode313::BUILD_MAP as u8, // BUILD_MAP: 105→48
            106 => Opcode313::LOAD_ATTR as u8, // LOAD_ATTR: 106→82
            107 => Opcode313::COMPARE_OP as u8, // COMPARE_OP: 107→58
            108 => Opcode313::IMPORT_NAME as u8, // IMPORT_NAME: 108→75
            109 => Opcode313::IMPORT_FROM as u8, // IMPORT_FROM: 109→74
            116 => Opcode313::LOAD_GLOBAL as u8, // LOAD_GLOBAL: 116→91
            118 => Opcode313::CONTAINS_OP as u8, // CONTAINS_OP: 118→59
            124 => Opcode313::LOAD_FAST as u8, // LOAD_FAST: 124→85
            125 => Opcode313::STORE_FAST as u8, // STORE_FAST: 125→110
            126 => Opcode313::DELETE_FAST as u8, // DELETE_FAST: 126→65
            130 => Opcode313::RAISE_VARARGS as u8, // RAISE_VARARGS: 130→101
            132 => Opcode313::MAKE_FUNCTION as u8, // MAKE_FUNCTION: 132→26
            142 => Opcode313::CALL_FUNCTION_EX as u8, // CALL_FUNCTION_EX: 142→54
            144 => Opcode313::EXTENDED_ARG as u8, // EXTENDED_ARG: 144→71
            156 => Opcode313::BUILD_CONST_KEY_MAP as u8, // 156→46
            157 => Opcode313::BUILD_STRING as u8, // BUILD_STRING: 157→51
            160 => Opcode313::LOAD_ATTR as u8, // LOAD_METHOD: 160→82 (merged into LOAD_ATTR)
            other => other, // fallback for unknown opcodes
        }
    }

    /// POP_JUMP_IF_FALSE cache entries: 3.13+=1, before that=0.
    pub fn cache_entries_pop_jump_if_false(&self) -> usize {
        if self.is_3_13_plus() {
            1
        } else {
            0
        }
    }

    /// POP_JUMP_IF_TRUE cache entries: 3.13+=1, before that=0.
    pub fn cache_entries_pop_jump_if_true(&self) -> usize {
        if self.is_3_13_plus() {
            1
        } else {
            0
        }
    }

    /// JUMP_BACKWARD cache entries: 3.13+=1, before that=0.
    pub fn cache_entries_jump_backward(&self) -> usize {
        if self.is_3_13_plus() {
            1
        } else {
            0
        }
    }

    /// CONTAINS_OP cache entries: 3.13+=1, before that=0.
    pub fn cache_entries_contains_op(&self) -> usize {
        if self.is_3_13_plus() {
            1
        } else {
            0
        }
    }

    /// TO_BOOL cache entries: 3.13+=3, before that=0.
    pub fn cache_entries_to_bool(&self) -> usize {
        if self.is_3_13_plus() {
            3
        } else {
            0
        }
    }
}
