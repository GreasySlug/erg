//! Version-specific code generation for Python 3.7–3.8.
//!
//! This module contains instruction sequences that differ structurally from
//! later Python versions.

use erg_common::error::ErrorDisplay;
use erg_common::opcode::CommonOpcode::*;
use erg_common::opcode308::Opcode308;
use erg_common::opcode309::Opcode309;
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
    /// WITH implementation for Python 3.8 (uses BEGIN_FINALLY + WITH_CLEANUP_START/FINISH).
    pub(crate) fn emit_with_instr_308(&mut self, mut args: Args) {
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
        self.write_instr(Opcode309::SETUP_WITH);
        self.write_arg(0);
        // push __exit__, __enter__() to the stack
        // self.stack_inc_n(2);
        let lambda_line = lambda.body.last().unwrap().ln_begin().unwrap_or(0);
        self.emit_with_block(lambda.body, params);
        let stash = Identifier::private_with_line(self.fresh_gen.fresh_varname(), lambda_line);
        self.emit_store_instr(stash.clone(), AccessKind::Name);
        self.write_instr(POP_BLOCK);
        self.write_arg(0);
        self.write_instr(Opcode308::BEGIN_FINALLY);
        self.write_arg(0);
        self.write_instr(Opcode308::WITH_CLEANUP_START);
        self.write_arg(0);
        self.edit_code(idx_setup_with + 1, (self.lasti() - idx_setup_with - 2) / 2);
        self.write_instr(Opcode308::WITH_CLEANUP_FINISH);
        self.write_arg(0);
        self.write_instr(Opcode308::END_FINALLY);
        self.write_arg(0);
        self.emit_load_name_instr(stash);
    }

    /// WITH implementation for Python 3.7 (uses WITH_CLEANUP_START/FINISH without BEGIN_FINALLY).
    pub(crate) fn emit_with_instr_307(&mut self, mut args: Args) {
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
        self.write_instr(Opcode309::SETUP_WITH);
        self.write_arg(0);
        // push __exit__, __enter__() to the stack
        // self.stack_inc_n(2);
        let lambda_line = lambda.body.last().unwrap().ln_begin().unwrap_or(0);
        self.emit_with_block(lambda.body, params);
        let stash = Identifier::private_with_line(self.fresh_gen.fresh_varname(), lambda_line);
        self.emit_store_instr(stash.clone(), AccessKind::Name);
        self.write_instr(POP_BLOCK);
        self.write_arg(0);
        self.emit_load_const(ValueObj::None);
        self.stack_dec();
        self.write_instr(Opcode308::WITH_CLEANUP_START);
        self.write_arg(0);
        self.edit_code(idx_setup_with + 1, (self.lasti() - idx_setup_with - 2) / 2);
        self.write_instr(Opcode308::WITH_CLEANUP_FINISH);
        self.write_arg(0);
        self.write_instr(Opcode308::END_FINALLY);
        self.write_arg(0);
        self.emit_load_name_instr(stash);
    }

    /// Binary operation for Python 3.7–3.8 (individual BINARY_* opcodes, IS via COMPARE_OP).
    pub(crate) fn emit_binop_instr_307(&mut self, binop: Token, type_pair: TypePair) {
        let instr = match &binop.kind {
            TokenKind::Plus => Opcode308::BINARY_ADD,
            TokenKind::Minus => Opcode308::BINARY_SUBTRACT,
            TokenKind::Star => Opcode308::BINARY_MULTIPLY,
            TokenKind::Slash => Opcode308::BINARY_TRUE_DIVIDE,
            TokenKind::FloorDiv => Opcode308::BINARY_FLOOR_DIVIDE,
            TokenKind::Pow => Opcode308::BINARY_POWER,
            TokenKind::Mod => Opcode308::BINARY_MODULO,
            TokenKind::AndOp | TokenKind::BitAnd => Opcode308::BINARY_AND,
            TokenKind::OrOp | TokenKind::BitOr => Opcode308::BINARY_OR,
            TokenKind::BitXor => Opcode308::BINARY_XOR,
            TokenKind::Less
            | TokenKind::LessEq
            | TokenKind::DblEq
            | TokenKind::NotEq
            | TokenKind::Gre
            | TokenKind::GreEq
            | TokenKind::InOp
            | TokenKind::NotInOp
            | TokenKind::IsOp
            | TokenKind::IsNotOp => Opcode308::COMPARE_OP,
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Closed
            | TokenKind::Open
            | TokenKind::ContainsOp => Opcode308::CALL_FUNCTION, // ERG_BINARY_RANGE,
            _ => {
                CompileError::feature_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    binop.loc(),
                    &binop.inspect().clone(),
                    String::from(binop.content),
                )
                .write_to_stderr();
                Opcode308::NOT_IMPLEMENTED
            }
        };
        let arg = match &binop.kind {
            TokenKind::Less => 0,
            TokenKind::LessEq => 1,
            TokenKind::DblEq => 2,
            TokenKind::NotEq => 3,
            TokenKind::Gre => 4,
            TokenKind::GreEq => 5,
            TokenKind::InOp => 6,
            TokenKind::NotInOp => 7,
            TokenKind::IsOp => 8,
            TokenKind::IsNotOp => 9,
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

    /// Var args for Python 3.8 and below (BUILD_TUPLE + BUILD_TUPLE_UNPACK_WITH_CALL).
    pub(crate) fn emit_var_args_308(&mut self, pos_len: usize, var_args: &PosArg) {
        if pos_len > 0 {
            self.write_instr(BUILD_TUPLE);
            self.write_arg(pos_len);
        }
        self.emit_expr(var_args.expr.clone());
        if pos_len > 0 {
            self.write_instr(self.opcode_set.build_tuple_unpack_with_call());
            self.write_arg(2);
        }
        self.stack_dec();
    }

    /// Keyword var args for Python 3.8 and below.
    pub(crate) fn emit_kw_var_args_308(&mut self, pos_len: usize, kw_var: &PosArg) {
        self.write_instr(BUILD_TUPLE);
        self.write_arg(pos_len);
        self.emit_expr(kw_var.expr.clone());
        self.stack_dec_n(pos_len.saturating_sub(1));
    }
}
