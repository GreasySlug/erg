//! Version-specific code generation for Python 3.9–3.10.
//!
//! This module contains instruction sequences that differ structurally from
//! other Python version groups.

use erg_common::error::ErrorDisplay;
use erg_common::opcode::CommonOpcode::*;
use erg_common::opcode309::Opcode309;
use erg_common::opcode310::Opcode310;
use erg_common::opcode311::Opcode311;
use erg_common::traits::{Locational, Stream};
use erg_common::{fn_name, log};

use erg_parser::token::{Token, TokenKind};

use crate::codegen::PyCodeGenerator;
use crate::compile::AccessKind;
use crate::context::ControlKind;
use crate::error::CompileError;
use crate::hir::{Args, Expr, Identifier, PosArg};
use crate::ty::value::ValueObj;
use crate::ty::TypePair;

impl PyCodeGenerator {
    /// WITH implementation for Python 3.10 (uses SETUP_WITH + BEFORE_WITH semantics).
    pub(crate) fn emit_with_instr_310(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if !matches!(args.get(1).unwrap(), Expr::Lambda(_)) {
            return self.deopt_instr(ControlKind::With, args);
        }
        let expr = args.remove(0);
        let Expr::Lambda(lambda) = args.remove(0) else {
            unreachable!()
        };
        let params = self.gen_param_names(&lambda.params);
        self.emit_expr(expr);
        let idx_setup_with = self.lasti();
        self.write_instr(Opcode310::SETUP_WITH);
        self.write_arg(0);
        // push __exit__, __enter__() to the stack
        self.stack_inc_n(2);
        let lambda_line = lambda.body.last().unwrap().ln_begin().unwrap_or(0);
        self.emit_with_block(lambda.body, params);
        let stash = Identifier::private_with_line(self.fresh_gen.fresh_varname(), lambda_line);
        self.emit_store_instr(stash.clone(), AccessKind::Name);
        self.write_instr(POP_BLOCK);
        self.write_arg(0);
        self.emit_load_const(ValueObj::None);
        self.write_instr(Opcode310::DUP_TOP);
        self.write_arg(0);
        self.stack_inc();
        self.write_instr(Opcode310::DUP_TOP);
        self.write_arg(0);
        self.stack_inc();
        self.write_instr(Opcode310::CALL_FUNCTION);
        self.write_arg(3);
        self.stack_dec_n((1 + 3) - 1);
        self.emit_pop_top();
        let idx_jump_forward = self.lasti();
        self.write_instr(Opcode310::JUMP_FORWARD);
        self.write_arg(0);
        self.edit_code(idx_setup_with + 1, (self.lasti() - idx_setup_with - 2) / 2);
        self.write_instr(Opcode310::WITH_EXCEPT_START);
        self.write_arg(0);
        let idx_pop_jump_if_true = self.lasti();
        self.write_instr(Opcode310::POP_JUMP_IF_TRUE);
        self.write_arg(0);
        self.write_instr(Opcode310::RERAISE);
        self.write_arg(1);
        self.edit_code(idx_pop_jump_if_true + 1, self.lasti() / 2);
        // self.emit_pop_top();
        // self.emit_pop_top();
        self.emit_pop_top();
        self.write_instr(Opcode310::POP_EXCEPT);
        self.write_arg(0);
        let idx_end = self.lasti();
        self.edit_code(idx_jump_forward + 1, (idx_end - idx_jump_forward - 2) / 2);
        self.emit_load_name_instr(stash);
    }

    /// WITH implementation for Python 3.9 (uses SETUP_WITH + WITH_EXCEPT_START).
    pub(crate) fn emit_with_instr_309(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if !matches!(args.get(1).unwrap(), Expr::Lambda(_)) {
            return self.deopt_instr(ControlKind::With, args);
        }
        let expr = args.remove(0);
        let Expr::Lambda(lambda) = args.remove(0) else {
            unreachable!()
        };
        let params = self.gen_param_names(&lambda.params);
        self.emit_expr(expr);
        let idx_setup_with = self.lasti();
        self.write_instr(Opcode310::SETUP_WITH);
        self.write_arg(0);
        // push __exit__, __enter__() to the stack
        self.stack_inc_n(2);
        let lambda_line = lambda.body.last().unwrap().ln_begin().unwrap_or(0);
        self.emit_with_block(lambda.body, params);
        let stash = Identifier::private_with_line(self.fresh_gen.fresh_varname(), lambda_line);
        self.emit_store_instr(stash.clone(), AccessKind::Name);
        self.write_instr(POP_BLOCK);
        self.write_arg(0);
        self.emit_load_const(ValueObj::None);
        self.write_instr(Opcode310::DUP_TOP);
        self.write_arg(0);
        self.stack_inc();
        self.write_instr(Opcode310::DUP_TOP);
        self.write_arg(0);
        self.stack_inc();
        self.write_instr(Opcode310::CALL_FUNCTION);
        self.write_arg(3);
        self.stack_dec_n((1 + 3) - 1);
        self.emit_pop_top();
        let idx_jump_forward = self.lasti();
        self.write_instr(Opcode311::JUMP_FORWARD);
        self.write_arg(0);
        self.edit_code(idx_setup_with + 1, self.lasti() - idx_setup_with - 2);
        self.write_instr(Opcode310::WITH_EXCEPT_START);
        self.write_arg(0);
        let idx_pop_jump_if_true = self.lasti();
        self.write_instr(Opcode310::POP_JUMP_IF_TRUE);
        self.write_arg(0);
        self.write_instr(Opcode309::RERAISE);
        self.write_arg(1);
        self.edit_code(idx_pop_jump_if_true + 1, self.lasti());
        // self.emit_pop_top();
        // self.emit_pop_top();
        self.emit_pop_top();
        self.write_instr(Opcode310::POP_EXCEPT);
        self.write_arg(0);
        let idx_end = self.lasti();
        self.edit_code(idx_jump_forward + 1, idx_end - idx_jump_forward - 2);
        self.emit_load_name_instr(stash);
    }

    /// Binary operation for Python 3.9–3.10 (individual BINARY_* opcodes, IS_OP separated).
    pub(crate) fn emit_binop_instr_309(&mut self, binop: Token, type_pair: TypePair) {
        let instr = match &binop.kind {
            TokenKind::Plus => Opcode309::BINARY_ADD,
            TokenKind::Minus => Opcode309::BINARY_SUBTRACT,
            TokenKind::Star => Opcode309::BINARY_MULTIPLY,
            TokenKind::Slash => Opcode309::BINARY_TRUE_DIVIDE,
            TokenKind::FloorDiv => Opcode309::BINARY_FLOOR_DIVIDE,
            TokenKind::Pow => Opcode309::BINARY_POWER,
            TokenKind::Mod => Opcode309::BINARY_MODULO,
            TokenKind::AndOp | TokenKind::BitAnd => Opcode309::BINARY_AND,
            TokenKind::OrOp | TokenKind::BitOr => Opcode309::BINARY_OR,
            TokenKind::BitXor => Opcode309::BINARY_XOR,
            TokenKind::IsOp | TokenKind::IsNotOp => Opcode309::IS_OP,
            TokenKind::Less
            | TokenKind::LessEq
            | TokenKind::DblEq
            | TokenKind::NotEq
            | TokenKind::Gre
            | TokenKind::GreEq => Opcode309::COMPARE_OP,
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Closed
            | TokenKind::Open
            | TokenKind::ContainsOp => Opcode309::CALL_FUNCTION, // ERG_BINARY_RANGE,
            _ => {
                CompileError::feature_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    binop.loc(),
                    &binop.inspect().clone(),
                    String::from(binop.content),
                )
                .write_to_stderr();
                Opcode309::NOT_IMPLEMENTED
            }
        };
        let arg = match &binop.kind {
            TokenKind::Less => 0,
            TokenKind::LessEq => 1,
            TokenKind::DblEq => 2,
            TokenKind::NotEq => 3,
            TokenKind::Gre => 4,
            TokenKind::GreEq => 5,
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
        self.stack_dec();
        match &binop.kind {
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Open
            | TokenKind::Closed
            | TokenKind::ContainsOp => {
                self.stack_dec();
            }
            _ => {}
        }
    }

    /// Var args for Python 3.9+ (BUILD_LIST + LIST_EXTEND + LIST_TO_TUPLE).
    pub(crate) fn emit_var_args_311(&mut self, pos_len: usize, var_args: &PosArg) {
        if pos_len > 0 {
            self.write_instr(BUILD_LIST);
            self.write_arg(pos_len);
        }
        self.emit_expr(var_args.expr.clone());
        if pos_len > 0 {
            self.write_instr(self.opcode_set.list_extend());
            self.write_arg(1);
            if self.opcode_set.is_3_12_plus() {
                // 3.12: LIST_TO_TUPLE removed; use CALL_INTRINSIC_1(ListToTuple=6)
                self.write_instr(self.opcode_set.call_intrinsic_1());
                self.write_arg(6);
            } else {
                self.write_instr(self.opcode_set.list_to_tuple());
                self.write_arg(0);
            }
        }
        self.stack_dec();
    }

    /// Keyword var args for Python 3.9+ (BUILD_MAP + DICT_MERGE).
    pub(crate) fn emit_kw_var_args_311(&mut self, pos_len: usize, kw_var: &PosArg) {
        self.write_instr(BUILD_TUPLE);
        self.write_arg(pos_len);
        self.stack_dec_n(pos_len.saturating_sub(1));
        self.write_instr(BUILD_MAP);
        self.write_arg(0);
        self.emit_expr(kw_var.expr.clone());
        self.write_instr(self.opcode_set.dict_merge());
        self.write_arg(1);
    }
}
