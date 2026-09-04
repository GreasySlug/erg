//! Version-specific code generation for Python 3.11+.
//!
//! This module contains instruction sequences that differ structurally from
//! earlier Python versions. Simple opcode substitutions are handled by
//! [`erg_common::opcode_set::OpcodeSetVersion`] instead.

use erg_common::error::ErrorDisplay;
use erg_common::traits::{Locational, Stream};
use erg_common::{fn_name, log};

use erg_common::opcode311::BinOpCode;
use erg_parser::token::{Token, TokenKind};

use crate::codegen::PyCodeGenerator;
use crate::compile::AccessKind;
use crate::context::ControlKind;
use crate::error::CompileError;
use crate::hir::{Args, Expr, Identifier};
use crate::ty::value::ValueObj;
use crate::ty::TypePair;

impl PyCodeGenerator {
    pub(crate) fn emit_with_instr_311(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if !matches!(args.get(1).unwrap(), Expr::Lambda(_)) {
            return self.deopt_instr(ControlKind::With, args);
        }
        let expr = args.remove(0);
        let Expr::Lambda(lambda) = args.remove(0) else {
            unreachable!()
        };
        self.emit_expr(expr);
        if self.opcode_set.is_3_14_plus() {
            // 3.14 dropped BEFORE_WITH. Its replacement spells the same thing out:
            // `LOAD_SPECIAL` pushes a dunder together with its receiver, so the
            // two SWAPs bury the manager under `__exit__` before `__enter__` is
            // called on it.
            self.write_instr(self.opcode_set.copy());
            self.write_arg(1);
            self.write_instr(self.opcode_set.load_special());
            self.write_arg(1); // __exit__
            self.write_instr(self.opcode_set.swap());
            self.write_arg(2);
            self.write_instr(self.opcode_set.swap());
            self.write_arg(3);
            self.write_instr(self.opcode_set.load_special());
            self.write_arg(0); // __enter__
                               // three above the manager at the peak, back to one net -- the two
                               // `stack_inc_n(2)` below covers, as it does for `BEFORE_WITH`
            self.stack_inc_n(3);
            self.emit_precall_and_call(0);
            self.stack_dec_n(2);
        } else {
            self.write_instr(self.opcode_set.before_with());
            self.write_arg(0);
        }
        // push __exit__, __enter__() to the stack
        self.stack_inc_n(2);
        let lambda_line = lambda.body.last().unwrap().ln_begin().unwrap_or(0);
        self.emit_with_block(lambda.body, &lambda.params);
        let stash = Identifier::private_with_line(self.fresh_gen.fresh_varname(), lambda_line);
        self.emit_store_instr(stash.clone(), AccessKind::Name);
        self.emit_load_const(ValueObj::None);
        self.emit_load_const(ValueObj::None);
        self.emit_load_const(ValueObj::None);
        // `BEFORE_WITH` leaves a bound `__exit__`, so the first `None` stands in
        // for the receiver; `LOAD_SPECIAL` leaves the receiver itself, and all
        // three `None`s are arguments.
        let exit_argc = if self.opcode_set.is_3_14_plus() { 3 } else { 2 };
        self.emit_precall_and_call(exit_argc);
        self.emit_pop_top();
        let idx_jump_forward = self.lasti();
        self.write_instr(self.opcode_set.jump_forward());
        self.write_arg(0);
        self.write_instr(self.opcode_set.push_exc_info());
        self.write_arg(0);
        self.write_instr(self.opcode_set.with_except_start());
        self.write_arg(0);
        self.emit_to_bool();
        self.write_instr(self.opcode_set.pop_jump_if_true());
        // skip the four instructions below; the jump counts from after the
        // cache, which is why the argument does not change when there is one
        self.write_arg(4);
        self.emit_pop_jump_cache();
        self.write_instr(self.opcode_set.reraise());
        self.write_arg(0);
        self.write_instr(self.opcode_set.copy());
        self.write_arg(3);
        self.write_instr(self.opcode_set.pop_except());
        self.write_arg(0);
        self.write_instr(self.opcode_set.reraise());
        self.write_arg(1);
        self.emit_pop_top();
        self.write_instr(self.opcode_set.pop_except());
        self.write_arg(0);
        self.emit_pop_top();
        self.emit_pop_top();
        self.calc_edit_jump(idx_jump_forward + 1, self.lasti() - idx_jump_forward - 2);
        self.emit_load_name_instr(stash);
    }

    /// Binary operation for Python 3.11+ (unified BINARY_OP, IS_OP, COMPARE_OP with CACHE).
    ///
    /// Uses `self.opcode_set.*()` for version-dispatched opcode values so this
    /// works correctly for 3.11, 3.12, 3.13, and 3.14+.
    pub(crate) fn emit_binop_instr_311(&mut self, binop: Token, type_pair: TypePair) {
        // Track the instruction kind for cache emission below.
        #[derive(PartialEq)]
        enum InstrKind {
            BinaryOp,
            IsOp,
            CompareOp,
            Call,
            NotImpl,
        }
        let kind = match &binop.kind {
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::FloorDiv
            | TokenKind::Pow
            | TokenKind::Mod
            | TokenKind::AndOp
            | TokenKind::OrOp
            | TokenKind::BitAnd
            | TokenKind::BitOr
            | TokenKind::BitXor
            | TokenKind::Shl
            | TokenKind::Shr => InstrKind::BinaryOp,
            TokenKind::IsOp | TokenKind::IsNotOp => InstrKind::IsOp,
            TokenKind::Less
            | TokenKind::LessEq
            | TokenKind::DblEq
            | TokenKind::NotEq
            | TokenKind::Gre
            | TokenKind::GreEq => InstrKind::CompareOp,
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Closed
            | TokenKind::Open
            | TokenKind::ContainsOp => {
                if self.opcode_set.has_precall() {
                    if let Some(precall) = self.opcode_set.precall() {
                        self.write_instr(precall);
                        self.write_arg(2);
                        self.write_arg(0);
                        self.write_arg(0);
                    }
                }
                InstrKind::Call
            }
            _ => {
                CompileError::feature_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    binop.loc(),
                    &binop.inspect().clone(),
                    String::from(binop.content),
                )
                .write_to_stderr();
                InstrKind::NotImpl
            }
        };
        let instr: u8 = match kind {
            InstrKind::BinaryOp => self.opcode_set.binary_op(),
            InstrKind::IsOp => self.opcode_set.is_op(),
            InstrKind::CompareOp => self.opcode_set.compare_op(),
            InstrKind::Call => self.opcode_set.call(),
            InstrKind::NotImpl => 0,
        };
        let arg = match &binop.kind {
            TokenKind::Plus => BinOpCode::Add as usize,
            TokenKind::Minus => BinOpCode::Subtract as usize,
            TokenKind::Star => BinOpCode::Multiply as usize,
            TokenKind::Slash => BinOpCode::TrueDivide as usize,
            TokenKind::FloorDiv => BinOpCode::FloorDiv as usize,
            TokenKind::Pow => BinOpCode::Power as usize,
            TokenKind::Mod => BinOpCode::Remainder as usize,
            TokenKind::AndOp | TokenKind::BitAnd => BinOpCode::And as usize,
            TokenKind::OrOp | TokenKind::BitOr => BinOpCode::Or as usize,
            TokenKind::BitXor => BinOpCode::Xor as usize,
            TokenKind::Shl => BinOpCode::LShift as usize,
            TokenKind::Shr => BinOpCode::RShift as usize,
            TokenKind::Less => self.opcode_set.encode_compare_arg(0),
            TokenKind::LessEq => self.opcode_set.encode_compare_arg(1),
            TokenKind::DblEq => self.opcode_set.encode_compare_arg(2),
            TokenKind::NotEq => self.opcode_set.encode_compare_arg(3),
            TokenKind::Gre => self.opcode_set.encode_compare_arg(4),
            TokenKind::GreEq => self.opcode_set.encode_compare_arg(5),
            TokenKind::IsOp => 0,
            TokenKind::IsNotOp => 1,
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Closed
            | TokenKind::Open
            | TokenKind::ContainsOp => 2,
            _ => type_pair as usize,
        };
        self.write_instr(instr);
        self.write_arg(arg);
        match kind {
            InstrKind::Call => {
                let cache = self.opcode_set.cache_entries_call() * 2;
                self.write_bytes(&vec![0; cache]);
            }
            InstrKind::BinaryOp => {
                let cache = self.opcode_set.cache_entries_binary_op() * 2;
                self.write_bytes(&vec![0; cache]);
            }
            InstrKind::CompareOp => {
                let cache = self.opcode_set.cache_entries_compare_op() * 2;
                self.write_bytes(&vec![0; cache]);
            }
            _ => {}
        }
        self.stack_dec();
        match &binop.kind {
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Open
            | TokenKind::Closed
            | TokenKind::ContainsOp => {
                self.stack_dec();
                if self.opcode_set.is_3_11_plus() {
                    self.stack_dec();
                }
            }
            _ => {}
        }
    }
}
