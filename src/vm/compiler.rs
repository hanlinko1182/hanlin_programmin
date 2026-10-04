//! Initial AST-to-bytecode compiler for Hanlin's stack VM.
//!
//! VM-09 deliberately compiles only primitive expressions and top-level
//! globals. Supported expression nodes do not carry source spans in the
//! current AST, so their containing statement's span is applied to every
//! instruction they emit. An empty program uses [`EMPTY_PROGRAM_SPAN`] for
//! its synthetic `Null` and `Return` instructions.

use std::fmt;

use crate::ast::{BinOp, Expr, Literal, Program, Stmt, UnOp};
use crate::error::Span;

use super::{Chunk, ChunkError, OpCode, Value};

const EMPTY_PROGRAM_SPAN: Span = Span { line: 1, col: 1 };

/// A structured failure produced while lowering Hanlin AST into bytecode.
#[derive(Clone, Debug, PartialEq)]
pub enum CompileError {
    UnsupportedStatement { kind: &'static str, span: Span },
    UnsupportedExpression { kind: &'static str, span: Span },
    ConstantPool { error: ChunkError, span: Span },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedStatement { kind, span } => {
                write!(f, "unsupported {kind} at {span}")
            }
            Self::UnsupportedExpression { kind, span } => {
                write!(f, "unsupported {kind} expression at {span}")
            }
            Self::ConstantPool { error, span } => {
                write!(f, "constant-pool error at {span}: {error}")
            }
        }
    }
}

impl std::error::Error for CompileError {}

/// Compiles the VM-09 subset of Hanlin's existing AST into a [`Chunk`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Compiler;

impl Compiler {
    pub const fn new() -> Self {
        Self
    }

    /// Compiles a top-level program and always emits explicit termination.
    pub fn compile(&mut self, program: &Program) -> Result<Chunk, CompileError> {
        let mut chunk = Chunk::new();
        let mut termination_span = EMPTY_PROGRAM_SPAN;

        for statement in &program.body {
            termination_span = statement_span(statement);
            Self::compile_statement(&mut chunk, statement)?;
        }

        chunk.write_instruction(OpCode::Null, termination_span);
        chunk.write_instruction(OpCode::Return, termination_span);
        Ok(chunk)
    }

    fn compile_statement(chunk: &mut Chunk, statement: &Stmt) -> Result<(), CompileError> {
        match statement {
            Stmt::VarDecl {
                name, init, span, ..
            } => {
                if let Some(initializer) = init {
                    Self::compile_expression(chunk, initializer, *span)?;
                } else {
                    chunk.write_instruction(OpCode::Null, *span);
                }
                let name = Self::add_constant(chunk, Value::String(name.clone()), *span)?;
                chunk.write_instruction(OpCode::DefineGlobal(name), *span);
                Ok(())
            }
            Stmt::Expression { expr, span } => {
                Self::compile_expression(chunk, expr, *span)?;
                chunk.write_instruction(OpCode::Pop, *span);
                Ok(())
            }
            Stmt::FnDecl { span, .. } => {
                Err(Self::unsupported_statement("function declaration", *span))
            }
            Stmt::Return { span, .. } => {
                Err(Self::unsupported_statement("return statement", *span))
            }
            Stmt::If { span, .. } => Err(Self::unsupported_statement("if statement", *span)),
            Stmt::While { span, .. } => Err(Self::unsupported_statement("while statement", *span)),
            Stmt::For { span, .. } => Err(Self::unsupported_statement("for statement", *span)),
            Stmt::Break { span } => Err(Self::unsupported_statement("break statement", *span)),
            Stmt::Continue { span } => {
                Err(Self::unsupported_statement("continue statement", *span))
            }
            Stmt::TryCatch { span, .. } => {
                Err(Self::unsupported_statement("try/catch statement", *span))
            }
            Stmt::Print { span, .. } => Err(Self::unsupported_statement("print statement", *span)),
        }
    }

    fn compile_expression(
        chunk: &mut Chunk,
        expression: &Expr,
        fallback_span: Span,
    ) -> Result<(), CompileError> {
        match expression {
            Expr::Literal(literal) => Self::compile_literal(chunk, literal, fallback_span),
            Expr::Identifier(name) => {
                let name = Self::add_constant(chunk, Value::String(name.clone()), fallback_span)?;
                chunk.write_instruction(OpCode::GetGlobal(name), fallback_span);
                Ok(())
            }
            Expr::Unary { op, expr } => {
                Self::compile_expression(chunk, expr, fallback_span)?;
                let opcode = match op {
                    UnOp::Neg => OpCode::Negate,
                    UnOp::Not => OpCode::Not,
                };
                chunk.write_instruction(opcode, fallback_span);
                Ok(())
            }
            Expr::Binary { op, left, right } => {
                let opcode = Self::binary_opcode(*op, fallback_span)?;
                Self::compile_expression(chunk, left, fallback_span)?;
                Self::compile_expression(chunk, right, fallback_span)?;
                chunk.write_instruction(opcode, fallback_span);
                Ok(())
            }
            Expr::Assign { name, value } => {
                Self::compile_expression(chunk, value, fallback_span)?;
                let name = Self::add_constant(chunk, Value::String(name.clone()), fallback_span)?;
                chunk.write_instruction(OpCode::SetGlobal(name), fallback_span);
                Ok(())
            }
            Expr::ArrayLiteral { span, .. } => {
                Err(Self::unsupported_expression("array literal", *span))
            }
            Expr::ObjectLiteral { span, .. } => {
                Err(Self::unsupported_expression("object literal", *span))
            }
            Expr::Index { span, .. } => Err(Self::unsupported_expression("index access", *span)),
            Expr::Member { span, .. } => Err(Self::unsupported_expression("member access", *span)),
            Expr::MethodCall { span, .. } => {
                Err(Self::unsupported_expression("method call", *span))
            }
            Expr::AssignIndex { span, .. } => {
                Err(Self::unsupported_expression("index assignment", *span))
            }
            Expr::AssignMember { span, .. } => {
                Err(Self::unsupported_expression("member assignment", *span))
            }
            Expr::Call { .. } => Err(Self::unsupported_expression("function call", fallback_span)),
        }
    }

    fn compile_literal(
        chunk: &mut Chunk,
        literal: &Literal,
        span: Span,
    ) -> Result<(), CompileError> {
        let opcode = match literal {
            Literal::Integer(value) => {
                OpCode::Constant(Self::add_constant(chunk, Value::Int(*value), span)?)
            }
            Literal::Float(value) => {
                OpCode::Constant(Self::add_constant(chunk, Value::Float(*value), span)?)
            }
            Literal::Str(value) => OpCode::Constant(Self::add_constant(
                chunk,
                Value::String(value.clone()),
                span,
            )?),
            Literal::Bool(true) => OpCode::True,
            Literal::Bool(false) => OpCode::False,
            Literal::Null => OpCode::Null,
        };
        chunk.write_instruction(opcode, span);
        Ok(())
    }

    fn binary_opcode(operator: BinOp, span: Span) -> Result<OpCode, CompileError> {
        match operator {
            BinOp::Add => Ok(OpCode::Add),
            BinOp::Sub => Ok(OpCode::Subtract),
            BinOp::Mul => Ok(OpCode::Multiply),
            BinOp::Div => Ok(OpCode::Divide),
            BinOp::Mod => Ok(OpCode::Modulo),
            BinOp::EqEq => Ok(OpCode::Equal),
            BinOp::NotEq => Ok(OpCode::NotEqual),
            BinOp::Lt => Ok(OpCode::Less),
            BinOp::LtEq => Ok(OpCode::LessEqual),
            BinOp::Gt => Ok(OpCode::Greater),
            BinOp::GtEq => Ok(OpCode::GreaterEqual),
            BinOp::And => Err(Self::unsupported_expression("logical And", span)),
            BinOp::Or => Err(Self::unsupported_expression("logical Or", span)),
        }
    }

    fn add_constant(
        chunk: &mut Chunk,
        value: Value,
        span: Span,
    ) -> Result<super::ConstantIndex, CompileError> {
        chunk
            .add_constant(value)
            .map_err(|error| CompileError::ConstantPool { error, span })
    }

    const fn unsupported_statement(kind: &'static str, span: Span) -> CompileError {
        CompileError::UnsupportedStatement { kind, span }
    }

    const fn unsupported_expression(kind: &'static str, span: Span) -> CompileError {
        CompileError::UnsupportedExpression { kind, span }
    }
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::VarDecl { span, .. }
        | Stmt::FnDecl { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::Expression { span, .. }
        | Stmt::Print { span, .. }
        | Stmt::TryCatch { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span } => *span,
    }
}

#[cfg(test)]
mod tests {
    use super::{CompileError, Compiler};
    use crate::error::Span;
    use crate::interpreter::{Env, Interpreter};
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::vm::{disassemble_chunk, Chunk, OpCode, Value, Vm};

    fn parse_source(source: &str) -> crate::ast::Program {
        let tokens = Lexer::new(source).tokenize().unwrap();
        Parser::new(tokens).parse().unwrap()
    }

    fn compile_source(source: &str) -> Result<Chunk, CompileError> {
        Compiler::new().compile(&parse_source(source))
    }

    fn run_source(source: &str) -> Value {
        let chunk = compile_source(source).unwrap();
        Vm::new().run(&chunk).unwrap()
    }

    fn run_source_and_get(source: &str, name: &str) -> Value {
        let chunk = compile_source(source).unwrap();
        let mut vm = Vm::new();
        assert_eq!(vm.run(&chunk), Ok(Value::Null));

        let mut read = Chunk::new();
        let name = read.add_constant(Value::String(name.to_owned())).unwrap();
        read.write_instruction(OpCode::GetGlobal(name), Span::new(1, 1));
        read.write_instruction(OpCode::Return, Span::new(1, 1));
        vm.run(&read).unwrap()
    }

    fn interpreter_global(source: &str, name: &str) -> String {
        let program = parse_source(source);
        let environment = Env::new();
        Interpreter::new(environment.clone())
            .interpret(&program)
            .unwrap();
        environment
            .get(name, Span::new(1, 1))
            .unwrap()
            .to_string_display()
    }

    fn assert_unsupported_statement(source: &str, kind: &'static str) {
        assert!(matches!(
            compile_source(source),
            Err(CompileError::UnsupportedStatement {
                kind: actual,
                ..
            }) if actual == kind
        ));
    }

    fn assert_unsupported_expression(source: &str, kind: &'static str) {
        assert!(matches!(
            compile_source(source),
            Err(CompileError::UnsupportedExpression {
                kind: actual,
                ..
            }) if actual == kind
        ));
    }

    #[test]
    fn compiles_integer_literal_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 42;", "result"),
            Value::Int(42)
        );
    }

    #[test]
    fn compiles_float_literal_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 3.5;", "result"),
            Value::Float(3.5)
        );
    }

    #[test]
    fn compiles_string_literal_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = \"hanlin\";", "result"),
            Value::String("hanlin".to_owned())
        );
    }

    #[test]
    fn compiles_true_literal_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = true;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_false_literal_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = false;", "result"),
            Value::Bool(false)
        );
    }

    #[test]
    fn compiles_null_literal_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = null;", "result"),
            Value::Null
        );
    }

    #[test]
    fn compiles_integer_negation_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = -10;", "result"),
            Value::Int(-10)
        );
    }

    #[test]
    fn compiles_float_negation_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = -2.5;", "result"),
            Value::Float(-2.5)
        );
    }

    #[test]
    fn compiles_logical_not_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = !false;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_addition_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 10 + 20;", "result"),
            Value::Int(30)
        );
    }

    #[test]
    fn compiles_subtraction_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 10 - 3;", "result"),
            Value::Int(7)
        );
    }

    #[test]
    fn compiles_multiplication_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 4 * 5;", "result"),
            Value::Int(20)
        );
    }

    #[test]
    fn compiles_division_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 5 / 2;", "result"),
            Value::Float(2.5)
        );
    }

    #[test]
    fn compiles_modulo_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 10 % 3;", "result"),
            Value::Int(1)
        );
    }

    #[test]
    fn parser_precedence_is_preserved_without_constant_folding() {
        let chunk = compile_source("let result = 10 + 20 * 2;").unwrap();
        let opcodes: Vec<_> = chunk
            .instructions()
            .iter()
            .map(|instruction| instruction.opcode())
            .collect();

        assert!(matches!(opcodes[0], OpCode::Constant(_)));
        assert!(matches!(opcodes[1], OpCode::Constant(_)));
        assert!(matches!(opcodes[2], OpCode::Constant(_)));
        assert_eq!(opcodes[3], OpCode::Multiply);
        assert_eq!(opcodes[4], OpCode::Add);
        assert_eq!(
            run_source_and_get("let result = 10 + 20 * 2;", "result"),
            Value::Int(50)
        );
    }

    #[test]
    fn compiles_equality_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 2 == 2;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_inequality_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 2 != 3;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_less_than_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 2 < 3;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_less_equal_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 3 <= 3;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_greater_than_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 4 > 3;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_greater_equal_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 4 >= 4;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_mixed_numeric_comparison_end_to_end() {
        assert_eq!(
            run_source_and_get("let result = 2 < 2.5;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_global_declaration() {
        assert_eq!(
            run_source_and_get("let answer = 42;", "answer"),
            Value::Int(42)
        );
    }

    #[test]
    fn compiles_global_read() {
        assert_eq!(
            run_source_and_get("let x = 10; let result = x;", "result"),
            Value::Int(10)
        );
    }

    #[test]
    fn compiles_global_assignment() {
        assert_eq!(
            run_source_and_get("let x = 10; x = 99;", "x"),
            Value::Int(99)
        );
    }

    #[test]
    fn compiles_multiple_globals() {
        assert_eq!(
            run_source_and_get("let x = 10; let y = 20; let result = x + y;", "result"),
            Value::Int(30)
        );
    }

    #[test]
    fn declaration_without_initializer_uses_null() {
        assert_eq!(run_source_and_get("let result;", "result"), Value::Null);
    }

    #[test]
    fn expression_statement_discards_its_result() {
        let chunk = compile_source("1 + 2;").unwrap();
        let opcodes: Vec<_> = chunk
            .instructions()
            .iter()
            .map(|instruction| instruction.opcode())
            .collect();

        assert!(matches!(opcodes[0], OpCode::Constant(_)));
        assert!(matches!(opcodes[1], OpCode::Constant(_)));
        assert_eq!(opcodes[2], OpCode::Add);
        assert_eq!(opcodes[3], OpCode::Pop);
        assert_eq!(run_source("1 + 2;"), Value::Null);
    }

    #[test]
    fn compiled_program_terminates_with_null_and_return() {
        for source in ["", "let x = 1;"] {
            let chunk = compile_source(source).unwrap();
            let instructions = chunk.instructions();

            assert_eq!(instructions[instructions.len() - 2].opcode(), OpCode::Null);
            assert_eq!(
                instructions[instructions.len() - 1].opcode(),
                OpCode::Return
            );
        }
    }

    #[test]
    fn vm_executes_compiled_program() {
        assert_eq!(
            run_source_and_get("let x = 6; x = x * 7;", "x"),
            Value::Int(42)
        );
    }

    #[test]
    fn rejects_if_statement_with_span() {
        assert_eq!(
            compile_source("if (true) { let x = 1; }"),
            Err(CompileError::UnsupportedStatement {
                kind: "if statement",
                span: Span::new(1, 1),
            })
        );
    }

    #[test]
    fn rejects_while_statement() {
        assert_unsupported_statement("while (true) { break; }", "while statement");
    }

    #[test]
    fn rejects_function_declaration() {
        assert_unsupported_statement("fn answer() { return 42; }", "function declaration");
    }

    #[test]
    fn rejects_function_call() {
        assert_unsupported_expression("answer();", "function call");
    }

    #[test]
    fn rejects_array_literal_with_its_precise_span() {
        assert_eq!(
            compile_source("let values = [1, 2];"),
            Err(CompileError::UnsupportedExpression {
                kind: "array literal",
                span: Span::new(1, 14),
            })
        );
    }

    #[test]
    fn rejects_logical_and() {
        assert_unsupported_expression("let result = true && false;", "logical And");
    }

    #[test]
    fn rejects_logical_or() {
        assert_unsupported_expression("let result = true || false;", "logical Or");
    }

    #[test]
    fn compiled_chunk_is_compatible_with_disassembler() {
        let chunk = compile_source("let x = 10;").unwrap();

        assert_eq!(
            disassemble_chunk(&chunk, "compiled").unwrap(),
            concat!(
                "== compiled ==\n\n",
                "0000  1:1    CONSTANT       0    10\n",
                "0001  1:1    DEFINE_GLOBAL  1    \"x\"\n",
                "0002  1:1    NULL\n",
                "0003  1:1    RETURN\n"
            )
        );
    }

    #[test]
    fn expression_instructions_use_containing_statement_span() {
        let chunk = compile_source("\n  let result = 1 + 2;").unwrap();

        assert!(chunk
            .instructions()
            .iter()
            .all(|instruction| instruction.span() == Span::new(2, 3)));
    }

    #[test]
    fn bool_and_null_literals_do_not_enter_constant_pool() {
        let chunk = compile_source("let a = true; let b = false; let c = null;").unwrap();

        assert_eq!(chunk.constants().len(), 3);
        assert!(chunk
            .constants()
            .iter()
            .all(|value| matches!(value, Value::String(_))));
    }

    #[test]
    fn vm_matches_interpreter_for_arithmetic_global() {
        let source = "let result = 10 + 20 * 2;";

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }

    #[test]
    fn vm_matches_interpreter_for_global_assignment() {
        let source = "let x = 10; x = x + 5; let result = x >= 15;";

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }
}
