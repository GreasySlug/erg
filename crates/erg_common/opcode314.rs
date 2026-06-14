//! Defines `Opcode314` — Python 3.14 bytecode opcodes.
//!
//! Python 3.14 continues the renumbering started in 3.13, with further shifts
//! and several opcodes removed/added vs 3.13:
//!   - RETURN_CONST, BINARY_SUBSCR, BUILD_CONST_KEY_MAP, LOAD_ASSERTION_ERROR,
//!     BEFORE_WITH, BEFORE_ASYNC_WITH removed
//!   - POP_ITER, SET_FUNCTION_ATTRIBUTE, LOAD_COMMON_CONSTANT, LOAD_SMALL_INT,
//!     NOT_TAKEN, LOAD_SPECIAL, BUILD_TEMPLATE, BUILD_INTERPOLATION added
//!   - MAKE_FUNCTION no longer takes a flags argument; use SET_FUNCTION_ATTRIBUTE
//!   - BINARY_SUBSCR merged into BINARY_OP (sub-opcode NB_SUBSCR=26)
//!   - BINARY_OP cache: 1 → 5 entries
//!   - Magic number: 3627, HAVE_ARGUMENT: 43
//!
//! Values obtained from CPython 3.14.0 via `import opcode; opcode.opmap`.

#![allow(dead_code)]
#![allow(non_camel_case_types)]

use crate::impl_u8_enum;

impl_u8_enum! {Opcode314;
    // ── pre-HAVE_ARGUMENT opcodes (0–42) ──
    CACHE = 0,
    BINARY_SLICE = 1,
    BUILD_TEMPLATE = 2,
    // 3 is unused
    CALL_FUNCTION_EX = 4,
    CHECK_EG_MATCH = 5,
    CHECK_EXC_MATCH = 6,
    CLEANUP_THROW = 7,
    DELETE_SUBSCR = 8,
    END_FOR = 9,
    END_SEND = 10,
    EXIT_INIT_CHECK = 11,
    FORMAT_SIMPLE = 12,
    FORMAT_WITH_SPEC = 13,
    GET_AITER = 14,
    GET_ANEXT = 15,
    GET_ITER = 16,
    RESERVED = 17,
    GET_LEN = 18,
    GET_YIELD_FROM_ITER = 19,
    INTERPRETER_EXIT = 20,
    LOAD_BUILD_CLASS = 21,
    LOAD_LOCALS = 22,
    MAKE_FUNCTION = 23,       // no flags argument in 3.14!
    MATCH_KEYS = 24,
    MATCH_MAPPING = 25,
    MATCH_SEQUENCE = 26,
    NOP = 27,
    NOT_TAKEN = 28,           // NEW: hint after conditional jump fall-through
    POP_EXCEPT = 29,
    POP_ITER = 30,            // NEW: pops iterator after END_FOR
    POP_TOP = 31,
    PUSH_EXC_INFO = 32,
    PUSH_NULL = 33,
    RETURN_GENERATOR = 34,
    RETURN_VALUE = 35,
    SETUP_ANNOTATIONS = 36,
    STORE_SLICE = 37,
    STORE_SUBSCR = 38,
    TO_BOOL = 39,
    UNARY_INVERT = 40,
    UNARY_NEGATIVE = 41,
    UNARY_NOT = 42,
    WITH_EXCEPT_START = 43,

    // ── HAVE_ARGUMENT boundary = 43; opcodes ≥ 44 take an argument ──
    BINARY_OP = 44,           // cache: 5 entries (was 1 in 3.13)
    BUILD_INTERPOLATION = 45, // NEW: t-string interpolation
    BUILD_LIST = 46,
    BUILD_MAP = 47,
    BUILD_SET = 48,
    BUILD_SLICE = 49,
    BUILD_STRING = 50,
    BUILD_TUPLE = 51,
    CALL = 52,
    CALL_INTRINSIC_1 = 53,
    CALL_INTRINSIC_2 = 54,
    CALL_KW = 55,
    COMPARE_OP = 56,
    CONTAINS_OP = 57,
    CONVERT_VALUE = 58,
    COPY = 59,
    COPY_FREE_VARS = 60,
    DELETE_ATTR = 61,
    DELETE_DEREF = 62,
    DELETE_FAST = 63,
    DELETE_GLOBAL = 64,
    DELETE_NAME = 65,
    DICT_MERGE = 66,
    DICT_UPDATE = 67,
    END_ASYNC_FOR = 68,
    EXTENDED_ARG = 69,
    FOR_ITER = 70,
    GET_AWAITABLE = 71,
    IMPORT_FROM = 72,
    IMPORT_NAME = 73,
    IS_OP = 74,
    JUMP_BACKWARD = 75,
    JUMP_BACKWARD_NO_INTERRUPT = 76,
    JUMP_FORWARD = 77,
    LIST_APPEND = 78,
    LIST_EXTEND = 79,
    LOAD_ATTR = 80,
    LOAD_COMMON_CONSTANT = 81, // NEW: replaces LOAD_ASSERTION_ERROR
    LOAD_CONST = 82,
    LOAD_DEREF = 83,
    LOAD_FAST = 84,
    LOAD_FAST_AND_CLEAR = 85,
    LOAD_FAST_BORROW = 86,    // NEW: optimization
    LOAD_FAST_BORROW_LOAD_FAST_BORROW = 87, // NEW: superinstruction
    LOAD_FAST_CHECK = 88,
    LOAD_FAST_LOAD_FAST = 89,
    LOAD_FROM_DICT_OR_DEREF = 90,
    LOAD_FROM_DICT_OR_GLOBALS = 91,
    LOAD_GLOBAL = 92,
    LOAD_NAME = 93,
    LOAD_SMALL_INT = 94,      // NEW: small integer constant
    LOAD_SPECIAL = 95,        // NEW: __enter__/__exit__ for with statements
    LOAD_SUPER_ATTR = 96,
    MAKE_CELL = 97,
    MAP_ADD = 98,
    MATCH_CLASS = 99,
    POP_JUMP_IF_FALSE = 100,
    POP_JUMP_IF_NONE = 101,
    POP_JUMP_IF_NOT_NONE = 102,
    POP_JUMP_IF_TRUE = 103,
    RAISE_VARARGS = 104,
    RERAISE = 105,
    SEND = 106,
    SET_ADD = 107,
    SET_FUNCTION_ATTRIBUTE = 108, // NEW: replaces MAKE_FUNCTION flags
    SET_UPDATE = 109,
    STORE_ATTR = 110,
    STORE_DEREF = 111,
    STORE_FAST = 112,
    STORE_FAST_LOAD_FAST = 113,
    STORE_FAST_STORE_FAST = 114,
    STORE_GLOBAL = 115,
    STORE_NAME = 116,
    SWAP = 117,
    UNPACK_EX = 118,
    UNPACK_SEQUENCE = 119,
    YIELD_VALUE = 120,

    // gap: 121-127 unused

    RESUME = 128,

    // gap: 129-195 unused

    // ── Erg-specific opcodes (196–231) ──
    ERG_POP_NTH = 196,
    ERG_PEEK_NTH = 197,
    ERG_INC = 198,
    ERG_DEC = 199,
    ERG_LOAD_FAST_IMMUT = 200,
    ERG_STORE_FAST_IMMUT = 201,
    ERG_MOVE_FAST = 202,
    ERG_CLONE_FAST = 203,
    ERG_COPY_FAST = 204,
    ERG_REF_FAST = 205,
    ERG_REF_MUT_FAST = 206,
    ERG_MOVE_OUTER = 207,
    ERG_CLONE_OUTER = 208,
    ERG_COPY_OUTER = 209,
    ERG_REF_OUTER = 210,
    ERG_REF_MUT_OUTER = 211,
    ERG_LESS_THAN = 212,
    ERG_LESS_EQUAL = 213,
    ERG_EQUAL = 214,
    ERG_NOT_EQUAL = 215,
    ERG_MAKE_SLOT = 216,
    ERG_MAKE_TYPE = 217,
    ERG_MAKE_PURE_FUNCTION = 218,
    ERG_CALL_PURE_FUNCTION = 219,
    ERG_LOAD_EMPTY_SLOT = 220,
    ERG_LOAD_EMPTY_STR = 221,
    ERG_LOAD_1_NAT = 222,
    ERG_LOAD_1_INT = 223,
    ERG_LOAD_1_REAL = 224,
    ERG_LOAD_NONE = 225,
    ERG_MUTATE = 226,
    ERG_STORE_SUBSCR = 227,
    ERG_BINARY_SUBSCR = 228,
    ERG_BINARY_RANGE = 229,
    ERG_TRY_BINARY_DIVIDE = 230,
    ERG_BINARY_TRUE_DIVIDE = 231,

    // gap: 232-233 unused
    // INSTRUMENTED_* at 234-255 (CPython internal, do not use)

    NOT_IMPLEMENTED = 255,
}

impl_u8_enum! {BinOpCode314;
    Add = 0, And = 1, FloorDiv = 2, LShift = 3, MatrixMultiply = 4,
    Multiply = 5, Remainder = 6, Or = 7, Power = 8, RShift = 9,
    Subtract = 10, TrueDivide = 11, Xor = 12,
    InplaceAdd = 13, InplaceAnd = 14, InplaceFloorDiv = 15, InplaceLShift = 16,
    InplaceMatrixMultiply = 17, InplaceMultiply = 18, InplaceRemainder = 19,
    InplaceOr = 20, InplacePower = 21, InplaceRShift = 22, InplaceSubtract = 23,
    InplaceTrueDivide = 24, InplaceXor = 25,
    Subscr = 26,             // NEW: replaces BINARY_SUBSCR opcode
}
