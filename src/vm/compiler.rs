//! AST-to-bytecode compiler for Hanlin's stack VM.
//!
//! The compiler supports primitive expressions, globals, control flow,
//! short-circuit logical expressions, and named functions with frame-local
//! parameters and variables. Supported expression nodes do not carry source
//! spans in the current AST, so their containing statement's span is applied
//! to every instruction they emit. An empty program uses [`EMPTY_PROGRAM_SPAN`]
//! for its synthetic `Null` and `Return` instructions.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::ast::{BinOp, Expr, Literal, Program, Stmt, UnOp};
use crate::error::Span;

use super::{Arity, Chunk, ChunkError, Function, JumpOffset, LocalSlot, OpCode, Value};

const EMPTY_PROGRAM_SPAN: Span = Span { line: 1, col: 1 };

/// A structured failure produced while lowering Hanlin AST into bytecode.
#[derive(Clone, Debug, PartialEq)]
pub enum CompileError {
    UnsupportedStatement {
        kind: &'static str,
        span: Span,
    },
    UnsupportedExpression {
        kind: &'static str,
        span: Span,
    },
    ConstantPool {
        error: ChunkError,
        span: Span,
    },
    BreakOutsideLoop {
        span: Span,
    },
    ContinueOutsideLoop {
        span: Span,
    },
    ReturnOutsideFunction {
        span: Span,
    },
    NestedFunctionUnsupported {
        name: String,
        span: Span,
    },
    BlockLocalUnsupported {
        name: String,
        span: Span,
    },
    TooManyParameters {
        function: String,
        count: usize,
        maximum: usize,
        span: Span,
    },
    TooManyArguments {
        count: usize,
        maximum: usize,
        span: Span,
    },
    TooManyLocals {
        function: String,
        count: usize,
        maximum: usize,
        span: Span,
    },
    JumpTooLarge {
        distance: usize,
        span: Span,
    },
    InvalidJumpPatch {
        instruction_offset: usize,
        target: usize,
        span: Span,
    },
    ChunkPatch {
        error: ChunkError,
        span: Span,
    },
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
            Self::BreakOutsideLoop { span } => write!(f, "break outside a loop at {span}"),
            Self::ContinueOutsideLoop { span } => {
                write!(f, "continue outside a loop at {span}")
            }
            Self::ReturnOutsideFunction { span } => {
                write!(f, "return outside a function at {span}")
            }
            Self::NestedFunctionUnsupported { name, span } => write!(
                f,
                "nested function declaration '{name}' is unsupported without closures at {span}"
            ),
            Self::BlockLocalUnsupported { name, span } => write!(
                f,
                "block-local declaration '{name}' is unsupported in VM functions at {span}"
            ),
            Self::TooManyParameters {
                function,
                count,
                maximum,
                span,
            } => write!(
                f,
                "function '{function}' has {count} parameters, exceeding the maximum {maximum} at {span}"
            ),
            Self::TooManyArguments {
                count,
                maximum,
                span,
            } => write!(
                f,
                "call has {count} arguments, exceeding the maximum {maximum} at {span}"
            ),
            Self::TooManyLocals {
                function,
                count,
                maximum,
                span,
            } => write!(
                f,
                "function '{function}' needs {count} local slots, exceeding the maximum {maximum} at {span}"
            ),
            Self::JumpTooLarge { distance, span } => write!(
                f,
                "jump distance {distance} exceeds the bytecode operand capacity at {span}"
            ),
            Self::InvalidJumpPatch {
                instruction_offset,
                target,
                span,
            } => write!(
                f,
                "cannot patch jump at instruction {instruction_offset} to target {target} at {span}"
            ),
            Self::ChunkPatch { error, span } => {
                write!(f, "failed to patch bytecode at {span}: {error}")
            }
        }
    }
}

impl std::error::Error for CompileError {}

#[derive(Clone, Copy, Debug)]
struct PendingJump {
    instruction_offset: usize,
    span: Span,
}

#[derive(Debug)]
struct LoopContext {
    loop_start: usize,
    continue_target: usize,
    break_jumps: Vec<PendingJump>,
}

#[derive(Debug)]
struct FunctionContext {
    name: String,
    locals: HashMap<String, LocalSlot>,
    next_slot: usize,
    block_depth: usize,
}

/// Compiles the supported subset of Hanlin's existing AST into a [`Chunk`].
#[derive(Debug, Default)]
pub struct Compiler {
    loop_contexts: Vec<LoopContext>,
    function_context: Option<FunctionContext>,
}

impl Compiler {
    pub const fn new() -> Self {
        Self {
            loop_contexts: Vec::new(),
            function_context: None,
        }
    }

    /// Compiles a top-level program and always emits explicit termination.
    pub fn compile(&mut self, program: &Program) -> Result<Chunk, CompileError> {
        self.loop_contexts.clear();
        self.function_context = None;
        let result = self.compile_program(program);
        self.loop_contexts.clear();
        self.function_context = None;
        result
    }

    fn compile_program(&mut self, program: &Program) -> Result<Chunk, CompileError> {
        let mut chunk = Chunk::new();
        let mut termination_span = EMPTY_PROGRAM_SPAN;

        for statement in &program.body {
            termination_span = statement_span(statement);
            self.compile_statement(&mut chunk, statement)?;
        }

        chunk.write_instruction(OpCode::Null, termination_span);
        chunk.write_instruction(OpCode::Return, termination_span);
        Ok(chunk)
    }

    fn compile_statement(
        &mut self,
        chunk: &mut Chunk,
        statement: &Stmt,
    ) -> Result<(), CompileError> {
        match statement {
            Stmt::VarDecl {
                name, init, span, ..
            } => self.compile_variable_declaration(chunk, name, init.as_ref(), *span),
            Stmt::Expression { expr, span } => {
                self.compile_expression(chunk, expr, *span)?;
                chunk.write_instruction(OpCode::Pop, *span);
                Ok(())
            }
            Stmt::FnDecl {
                name,
                params,
                body,
                span,
            } => self.compile_function_declaration(chunk, name, params, body, *span),
            Stmt::Return { value, span } => self.compile_return(chunk, value.as_ref(), *span),
            Stmt::If {
                condition,
                then_body,
                else_body,
                span,
            } => self.compile_if(chunk, condition, then_body, else_body.as_deref(), *span),
            Stmt::While {
                condition,
                body,
                span,
            } => self.compile_while(chunk, condition, body, *span),
            Stmt::For { span, .. } => Err(Self::unsupported_statement("for statement", *span)),
            Stmt::Break { span } => self.compile_break(chunk, *span),
            Stmt::Continue { span } => self.compile_continue(chunk, *span),
            Stmt::TryCatch { span, .. } => {
                Err(Self::unsupported_statement("try/catch statement", *span))
            }
            Stmt::Print { span, .. } => Err(Self::unsupported_statement("print statement", *span)),
        }
    }

    fn compile_variable_declaration(
        &mut self,
        chunk: &mut Chunk,
        name: &str,
        initializer: Option<&Expr>,
        span: Span,
    ) -> Result<(), CompileError> {
        if self
            .function_context
            .as_ref()
            .is_some_and(|context| context.block_depth > 0)
        {
            return Err(CompileError::BlockLocalUnsupported {
                name: name.to_owned(),
                span,
            });
        }

        if let Some(initializer) = initializer {
            self.compile_expression(chunk, initializer, span)?;
        } else {
            chunk.write_instruction(OpCode::Null, span);
        }

        if let Some(context) = self.function_context.as_mut() {
            let slot = LocalSlot::try_from(context.next_slot).map_err(|_| {
                CompileError::TooManyLocals {
                    function: context.name.clone(),
                    count: context.next_slot + 1,
                    maximum: LocalSlot::MAX.as_usize() + 1,
                    span,
                }
            })?;
            context.next_slot += 1;
            context.locals.insert(name.to_owned(), slot);
            return Ok(());
        }

        let name = Self::add_constant(chunk, Value::String(name.to_owned()), span)?;
        chunk.write_instruction(OpCode::DefineGlobal(name), span);
        Ok(())
    }

    fn compile_function_declaration(
        &mut self,
        chunk: &mut Chunk,
        name: &str,
        params: &[String],
        body: &[Stmt],
        span: Span,
    ) -> Result<(), CompileError> {
        if self.function_context.is_some() {
            return Err(CompileError::NestedFunctionUnsupported {
                name: name.to_owned(),
                span,
            });
        }

        let arity = Arity::try_from(params.len()).map_err(|_| CompileError::TooManyParameters {
            function: name.to_owned(),
            count: params.len(),
            maximum: Arity::MAX.as_usize(),
            span,
        })?;
        let mut locals = HashMap::new();
        for (index, parameter) in params.iter().enumerate() {
            let slot =
                LocalSlot::try_from(index).expect("u8 arity always fits in a u16 local slot");
            locals.insert(parameter.clone(), slot);
        }

        let saved_loops = std::mem::take(&mut self.loop_contexts);
        self.function_context = Some(FunctionContext {
            name: name.to_owned(),
            locals,
            next_slot: params.len(),
            block_depth: 0,
        });

        let mut function_chunk = Chunk::new();
        let result = (|| {
            let mut termination_span = span;
            for statement in body {
                termination_span = statement_span(statement);
                self.compile_statement(&mut function_chunk, statement)?;
            }
            function_chunk.write_instruction(OpCode::Null, termination_span);
            function_chunk.write_instruction(OpCode::Return, termination_span);
            Ok(())
        })();

        self.function_context = None;
        self.loop_contexts = saved_loops;
        result?;

        let function = Value::Function(Rc::new(Function::new(name, arity, function_chunk)));
        let function = Self::add_constant(chunk, function, span)?;
        chunk.write_instruction(OpCode::Constant(function), span);
        let global_name = Self::add_constant(chunk, Value::String(name.to_owned()), span)?;
        chunk.write_instruction(OpCode::DefineGlobal(global_name), span);
        Ok(())
    }

    fn compile_return(
        &mut self,
        chunk: &mut Chunk,
        value: Option<&Expr>,
        span: Span,
    ) -> Result<(), CompileError> {
        if self.function_context.is_none() {
            return Err(CompileError::ReturnOutsideFunction { span });
        }
        if let Some(value) = value {
            self.compile_expression(chunk, value, span)?;
        } else {
            chunk.write_instruction(OpCode::Null, span);
        }
        chunk.write_instruction(OpCode::Return, span);
        Ok(())
    }

    fn compile_if(
        &mut self,
        chunk: &mut Chunk,
        condition: &Expr,
        then_body: &[Stmt],
        else_body: Option<&[Stmt]>,
        span: Span,
    ) -> Result<(), CompileError> {
        self.compile_expression(chunk, condition, span)?;
        let false_jump = Self::emit_jump(chunk, true, span);
        chunk.write_instruction(OpCode::Pop, span);

        self.compile_nested_statements(chunk, then_body)?;
        let end_jump = Self::emit_jump(chunk, false, span);

        Self::patch_jump(chunk, false_jump, chunk.instructions().len(), span)?;
        chunk.write_instruction(OpCode::Pop, span);
        if let Some(else_body) = else_body {
            self.compile_nested_statements(chunk, else_body)?;
        }

        Self::patch_jump(chunk, end_jump, chunk.instructions().len(), span)
    }

    fn compile_while(
        &mut self,
        chunk: &mut Chunk,
        condition: &Expr,
        body: &[Stmt],
        span: Span,
    ) -> Result<(), CompileError> {
        let loop_start = chunk.instructions().len();
        self.compile_expression(chunk, condition, span)?;
        let exit_jump = Self::emit_jump(chunk, true, span);
        chunk.write_instruction(OpCode::Pop, span);

        self.loop_contexts.push(LoopContext {
            loop_start,
            continue_target: loop_start,
            break_jumps: Vec::new(),
        });
        self.compile_nested_statements(chunk, body)?;
        Self::emit_loop(chunk, loop_start, span)?;

        Self::patch_jump(chunk, exit_jump, chunk.instructions().len(), span)?;
        chunk.write_instruction(OpCode::Pop, span);
        let break_target = chunk.instructions().len();
        let loop_context = self
            .loop_contexts
            .pop()
            .ok_or(CompileError::InvalidJumpPatch {
                instruction_offset: loop_start,
                target: break_target,
                span,
            })?;
        debug_assert_eq!(loop_context.loop_start, loop_start);
        for pending in loop_context.break_jumps {
            Self::patch_jump(
                chunk,
                pending.instruction_offset,
                break_target,
                pending.span,
            )?;
        }
        Ok(())
    }

    fn compile_break(&mut self, chunk: &mut Chunk, span: Span) -> Result<(), CompileError> {
        let loop_context = self
            .loop_contexts
            .last_mut()
            .ok_or(CompileError::BreakOutsideLoop { span })?;
        let instruction_offset = Self::emit_jump(chunk, false, span);
        loop_context.break_jumps.push(PendingJump {
            instruction_offset,
            span,
        });
        Ok(())
    }

    fn compile_continue(&mut self, chunk: &mut Chunk, span: Span) -> Result<(), CompileError> {
        let continue_target = self
            .loop_contexts
            .last()
            .ok_or(CompileError::ContinueOutsideLoop { span })?
            .continue_target;
        Self::emit_loop(chunk, continue_target, span)
    }

    fn compile_statements(
        &mut self,
        chunk: &mut Chunk,
        statements: &[Stmt],
    ) -> Result<(), CompileError> {
        for statement in statements {
            self.compile_statement(chunk, statement)?;
        }
        Ok(())
    }

    fn compile_nested_statements(
        &mut self,
        chunk: &mut Chunk,
        statements: &[Stmt],
    ) -> Result<(), CompileError> {
        if let Some(context) = self.function_context.as_mut() {
            context.block_depth += 1;
        }
        let result = self.compile_statements(chunk, statements);
        if let Some(context) = self.function_context.as_mut() {
            context.block_depth -= 1;
        }
        result
    }

    fn emit_jump(chunk: &mut Chunk, conditional: bool, span: Span) -> usize {
        let offset = JumpOffset::new(0);
        let opcode = if conditional {
            OpCode::JumpIfFalse(offset)
        } else {
            OpCode::Jump(offset)
        };
        chunk.write_instruction(opcode, span)
    }

    fn patch_jump(
        chunk: &mut Chunk,
        instruction_offset: usize,
        target: usize,
        span: Span,
    ) -> Result<(), CompileError> {
        let post_fetch =
            instruction_offset
                .checked_add(1)
                .ok_or(CompileError::InvalidJumpPatch {
                    instruction_offset,
                    target,
                    span,
                })?;
        let distance = target
            .checked_sub(post_fetch)
            .ok_or(CompileError::InvalidJumpPatch {
                instruction_offset,
                target,
                span,
            })?;
        let offset = JumpOffset::try_from(distance)
            .map_err(|_| CompileError::JumpTooLarge { distance, span })?;
        chunk
            .patch_jump(instruction_offset, offset)
            .map_err(|error| CompileError::ChunkPatch { error, span })
    }

    fn emit_loop(chunk: &mut Chunk, target: usize, span: Span) -> Result<(), CompileError> {
        let instruction_offset = chunk.instructions().len();
        let post_fetch =
            instruction_offset
                .checked_add(1)
                .ok_or(CompileError::InvalidJumpPatch {
                    instruction_offset,
                    target,
                    span,
                })?;
        let distance = post_fetch
            .checked_sub(target)
            .ok_or(CompileError::InvalidJumpPatch {
                instruction_offset,
                target,
                span,
            })?;
        let offset = JumpOffset::try_from(distance)
            .map_err(|_| CompileError::JumpTooLarge { distance, span })?;
        chunk.write_instruction(OpCode::Loop(offset), span);
        Ok(())
    }

    fn compile_expression(
        &mut self,
        chunk: &mut Chunk,
        expression: &Expr,
        fallback_span: Span,
    ) -> Result<(), CompileError> {
        match expression {
            Expr::Literal(literal) => Self::compile_literal(chunk, literal, fallback_span),
            Expr::Identifier(name) => {
                if let Some(slot) = self.resolve_local(name) {
                    chunk.write_instruction(OpCode::GetLocal(slot), fallback_span);
                } else {
                    let name =
                        Self::add_constant(chunk, Value::String(name.clone()), fallback_span)?;
                    chunk.write_instruction(OpCode::GetGlobal(name), fallback_span);
                }
                Ok(())
            }
            Expr::Unary { op, expr } => {
                self.compile_expression(chunk, expr, fallback_span)?;
                let opcode = match op {
                    UnOp::Neg => OpCode::Negate,
                    UnOp::Not => OpCode::Not,
                };
                chunk.write_instruction(opcode, fallback_span);
                Ok(())
            }
            Expr::Binary { op, left, right } => {
                match op {
                    BinOp::And => {
                        return self.compile_logical_and(chunk, left, right, fallback_span);
                    }
                    BinOp::Or => {
                        return self.compile_logical_or(chunk, left, right, fallback_span);
                    }
                    _ => {}
                }
                let opcode = Self::binary_opcode(*op);
                self.compile_expression(chunk, left, fallback_span)?;
                self.compile_expression(chunk, right, fallback_span)?;
                chunk.write_instruction(opcode, fallback_span);
                Ok(())
            }
            Expr::Assign { name, value } => {
                self.compile_expression(chunk, value, fallback_span)?;
                if let Some(slot) = self.resolve_local(name) {
                    chunk.write_instruction(OpCode::SetLocal(slot), fallback_span);
                } else {
                    let name =
                        Self::add_constant(chunk, Value::String(name.clone()), fallback_span)?;
                    chunk.write_instruction(OpCode::SetGlobal(name), fallback_span);
                }
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
            Expr::Call { callee, args } => {
                let arity =
                    Arity::try_from(args.len()).map_err(|_| CompileError::TooManyArguments {
                        count: args.len(),
                        maximum: Arity::MAX.as_usize(),
                        span: fallback_span,
                    })?;
                let callee =
                    Self::add_constant(chunk, Value::String(callee.clone()), fallback_span)?;
                chunk.write_instruction(OpCode::GetGlobal(callee), fallback_span);
                for argument in args {
                    self.compile_expression(chunk, argument, fallback_span)?;
                }
                chunk.write_instruction(OpCode::Call(arity), fallback_span);
                Ok(())
            }
        }
    }

    fn compile_logical_and(
        &mut self,
        chunk: &mut Chunk,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<(), CompileError> {
        self.compile_expression(chunk, left, span)?;
        let end_jump = Self::emit_jump(chunk, true, span);
        chunk.write_instruction(OpCode::Pop, span);
        self.compile_expression(chunk, right, span)?;
        Self::patch_jump(chunk, end_jump, chunk.instructions().len(), span)
    }

    fn compile_logical_or(
        &mut self,
        chunk: &mut Chunk,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Result<(), CompileError> {
        self.compile_expression(chunk, left, span)?;
        let right_jump = Self::emit_jump(chunk, true, span);
        let end_jump = Self::emit_jump(chunk, false, span);

        Self::patch_jump(chunk, right_jump, chunk.instructions().len(), span)?;
        chunk.write_instruction(OpCode::Pop, span);
        self.compile_expression(chunk, right, span)?;
        Self::patch_jump(chunk, end_jump, chunk.instructions().len(), span)
    }

    fn resolve_local(&self, name: &str) -> Option<LocalSlot> {
        self.function_context
            .as_ref()
            .and_then(|context| context.locals.get(name).copied())
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

    fn binary_opcode(operator: BinOp) -> OpCode {
        match operator {
            BinOp::Add => OpCode::Add,
            BinOp::Sub => OpCode::Subtract,
            BinOp::Mul => OpCode::Multiply,
            BinOp::Div => OpCode::Divide,
            BinOp::Mod => OpCode::Modulo,
            BinOp::EqEq => OpCode::Equal,
            BinOp::NotEq => OpCode::NotEqual,
            BinOp::Lt => OpCode::Less,
            BinOp::LtEq => OpCode::LessEqual,
            BinOp::Gt => OpCode::Greater,
            BinOp::GtEq => OpCode::GreaterEqual,
            BinOp::And | BinOp::Or => {
                unreachable!("logical operators are lowered with short-circuit control flow")
            }
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
    use crate::vm::{
        disassemble_chunk, Arity, Chunk, Function, JumpOffset, LocalSlot, OpCode, Value, Vm,
        VmError, MAX_CALL_FRAMES,
    };

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

    fn run_source_result(source: &str) -> Result<Value, VmError> {
        let chunk = compile_source(source).unwrap();
        Vm::new().run(&chunk)
    }

    fn compiled_function(source: &str, name: &str) -> std::rc::Rc<Function> {
        compile_source(source)
            .unwrap()
            .constants()
            .iter()
            .find_map(|value| match value {
                Value::Function(function) if function.name() == name => {
                    Some(std::rc::Rc::clone(function))
                }
                _ => None,
            })
            .unwrap()
    }

    fn run_source_and_get(source: &str, name: &str) -> Value {
        let chunk = compile_source(source).unwrap();
        let mut vm = Vm::new();
        assert_eq!(vm.run(&chunk), Ok(Value::Null));

        read_global(&mut vm, name)
    }

    fn read_global(vm: &mut Vm, name: &str) -> Value {
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
    fn if_true_executes_body() {
        assert_eq!(
            run_source_and_get("let x = 0; if (true) { x = 1; }", "x"),
            Value::Int(1)
        );
    }

    #[test]
    fn if_false_skips_body() {
        assert_eq!(
            run_source_and_get("let x = 0; if (false) { x = 1; }", "x"),
            Value::Int(0)
        );
    }

    #[test]
    fn if_else_executes_true_branch() {
        assert_eq!(
            run_source_and_get("let x = 0; if (true) { x = 1; } else { x = 2; }", "x",),
            Value::Int(1)
        );
    }

    #[test]
    fn if_else_executes_false_branch() {
        assert_eq!(
            run_source_and_get("let x = 0; if (false) { x = 1; } else { x = 2; }", "x",),
            Value::Int(2)
        );
    }

    #[test]
    fn compiles_nested_if() {
        assert_eq!(
            run_source_and_get("let x = 0; if (true) { if (true) { x = 3; } }", "x",),
            Value::Int(3)
        );
    }

    #[test]
    fn compiles_nested_if_else() {
        let source = concat!(
            "let x = 0; ",
            "if (true) { ",
            "  if (false) { x = 1; } else { x = 2; } ",
            "} else { x = 3; }"
        );

        assert_eq!(run_source_and_get(source, "x"), Value::Int(2));
    }

    #[test]
    fn if_condition_is_cleaned_on_both_paths() {
        for condition in ["true", "false"] {
            let source =
                format!("let x = 0; if ({condition}) {{ 1 + 2; }} else {{ 3 + 4; }} x = x + 1;");

            assert_eq!(run_source_and_get(&source, "x"), Value::Int(1));
        }
    }

    #[test]
    fn while_executes_repeatedly() {
        assert_eq!(
            run_source_and_get("let x = 0; while (x < 3) { x = x + 1; }", "x",),
            Value::Int(3)
        );
    }

    #[test]
    fn while_false_executes_zero_times() {
        assert_eq!(
            run_source_and_get("let x = 0; while (false) { x = 1; }", "x"),
            Value::Int(0)
        );
    }

    #[test]
    fn while_updates_global_state() {
        let source = concat!(
            "let x = 0; let sum = 0; ",
            "while (x < 4) { sum = sum + x; x = x + 1; }"
        );

        assert_eq!(run_source_and_get(source, "sum"), Value::Int(6));
    }

    #[test]
    fn while_exits_at_false_condition() {
        let source = "let x = 0; while (x < 2) { x = x + 1; } let done = x == 2;";

        assert_eq!(run_source_and_get(source, "done"), Value::Bool(true));
    }

    #[test]
    fn while_condition_is_cleaned_after_exit() {
        let source = "let x = 0; while (x < 2) { x = x + 1; } x = x + 40;";

        assert_eq!(run_source_and_get(source, "x"), Value::Int(42));
    }

    #[test]
    fn break_exits_while() {
        let source = "let x = 0; while (true) { x = x + 1; break; }";

        assert_eq!(run_source_and_get(source, "x"), Value::Int(1));
    }

    #[test]
    fn break_inside_conditional_exits_while() {
        let source = concat!(
            "let x = 0; ",
            "while (x < 5) { x = x + 1; if (x == 2) { break; } }"
        );

        assert_eq!(run_source_and_get(source, "x"), Value::Int(2));
    }

    #[test]
    fn break_targets_nearest_nested_loop() {
        let source = concat!(
            "let outer = 0; let inner = 0; let hits = 0; ",
            "while (outer < 2) { ",
            "  outer = outer + 1; inner = 0; ",
            "  while (inner < 3) { inner = inner + 1; break; } ",
            "  hits = hits + 1; ",
            "}"
        );

        assert_eq!(run_source_and_get(source, "outer"), Value::Int(2));
        assert_eq!(run_source_and_get(source, "hits"), Value::Int(2));
    }

    #[test]
    fn break_outside_loop_is_compile_error() {
        assert_eq!(
            compile_source("break;"),
            Err(CompileError::BreakOutsideLoop {
                span: Span::new(1, 1),
            })
        );
    }

    #[test]
    fn continue_restarts_while_condition() {
        let source = concat!(
            "let x = 0; let skipped = 0; ",
            "while (x < 3) { x = x + 1; continue; skipped = skipped + 1; }"
        );

        assert_eq!(run_source_and_get(source, "x"), Value::Int(3));
        assert_eq!(run_source_and_get(source, "skipped"), Value::Int(0));
    }

    #[test]
    fn continue_inside_conditional_restarts_while() {
        let source = concat!(
            "let x = 0; let hits = 0; ",
            "while (x < 3) { ",
            "  x = x + 1; if (x < 3) { continue; } hits = hits + 1; ",
            "}"
        );

        assert_eq!(run_source_and_get(source, "hits"), Value::Int(1));
    }

    #[test]
    fn continue_targets_nearest_nested_loop() {
        let source = concat!(
            "let outer = 0; let inner = 0; let hits = 0; ",
            "while (outer < 2) { ",
            "  outer = outer + 1; inner = 0; ",
            "  while (inner < 2) { inner = inner + 1; continue; } ",
            "  hits = hits + 1; ",
            "}"
        );

        assert_eq!(run_source_and_get(source, "outer"), Value::Int(2));
        assert_eq!(run_source_and_get(source, "hits"), Value::Int(2));
    }

    #[test]
    fn continue_outside_loop_is_compile_error() {
        assert_eq!(
            compile_source("continue;"),
            Err(CompileError::ContinueOutsideLoop {
                span: Span::new(1, 1),
            })
        );
    }

    #[test]
    fn compiler_patches_forward_branch_targets() {
        let chunk = compile_source("if (true) { 1; }").unwrap();
        let output = disassemble_chunk(&chunk, "if").unwrap();

        assert!(output.contains("JUMP_IF_FALSE    4 -> 6"));
        assert!(output.contains("JUMP             1 -> 7"));
    }

    #[test]
    fn multiple_break_sites_patch_to_same_loop_exit() {
        let source = concat!(
            "let x = 0;\n",
            "while (x < 5) {\n",
            "  if (x == 1) {\n",
            "    break;\n",
            "  }\n",
            "  if (x == 2) {\n",
            "    break;\n",
            "  }\n",
            "  x = x + 1;\n",
            "}"
        );
        let chunk = compile_source(source).unwrap();
        let break_targets: Vec<_> = chunk
            .instructions()
            .iter()
            .enumerate()
            .filter_map(|(instruction_offset, instruction)| {
                if !matches!(instruction.span().line, 4 | 7) {
                    return None;
                }
                match instruction.opcode() {
                    OpCode::Jump(offset) => Some(instruction_offset + 1 + offset.as_usize()),
                    _ => None,
                }
            })
            .collect();

        assert_eq!(break_targets.len(), 2);
        assert_eq!(break_targets[0], break_targets[1]);
        assert_eq!(run_source_and_get(source, "x"), Value::Int(1));
    }

    #[test]
    fn nested_branch_patching_does_not_interfere() {
        let source = concat!(
            "let result = 0; ",
            "if (true) { ",
            "  if (false) { result = 1; } else { result = 2; } ",
            "} else { result = 3; }"
        );

        let chunk = compile_source(source).unwrap();
        assert!(disassemble_chunk(&chunk, "nested").is_ok());
        assert_eq!(run_source_and_get(source, "result"), Value::Int(2));
    }

    #[test]
    fn oversized_jump_distance_is_compile_error() {
        let span = Span::new(9, 4);
        let mut chunk = Chunk::new();
        let instruction_offset = Compiler::emit_jump(&mut chunk, false, span);
        let distance = usize::from(u16::MAX) + 1;
        let target = instruction_offset + 1 + distance;

        assert_eq!(
            Compiler::patch_jump(&mut chunk, instruction_offset, target, span),
            Err(CompileError::JumpTooLarge { distance, span })
        );
        assert_eq!(
            chunk.instruction(instruction_offset).unwrap().opcode(),
            OpCode::Jump(JumpOffset::new(0))
        );
    }

    #[test]
    fn vm_stack_behavior_remains_balanced_after_if() {
        let source = concat!(
            "let x = 0; ",
            "if (false) { 1 + 2; } else { 3 + 4; } ",
            "x = x + 1;"
        );

        assert_eq!(run_source_and_get(source, "x"), Value::Int(1));
        assert_eq!(run_source(source), Value::Null);
    }

    #[test]
    fn vm_stack_behavior_remains_balanced_after_while() {
        let source = "let x = 0; while (x < 3) { x = x + 1; } 40 + 2;";

        assert_eq!(run_source_and_get(source, "x"), Value::Int(3));
        assert_eq!(run_source(source), Value::Null);
    }

    #[test]
    fn vm_is_reusable_after_compiled_control_flow() {
        let first = compile_source("let x = 0; while (x < 2) { x = x + 1; }").unwrap();
        let second = compile_source("let y = 0; if (true) { y = 7; }").unwrap();
        let mut vm = Vm::new();

        assert_eq!(vm.run(&first), Ok(Value::Null));
        assert_eq!(vm.run(&second), Ok(Value::Null));
        assert_eq!(read_global(&mut vm, "x"), Value::Int(2));
        assert_eq!(read_global(&mut vm, "y"), Value::Int(7));
    }

    #[test]
    fn control_flow_instructions_preserve_statement_spans() {
        let source = concat!(
            "if (true) { 1; }\n",
            "while (false) {\n",
            "  break;\n",
            "  continue;\n",
            "}"
        );
        let chunk = compile_source(source).unwrap();

        assert!(chunk.instructions().iter().any(|instruction| {
            matches!(instruction.opcode(), OpCode::JumpIfFalse(_))
                && instruction.span() == Span::new(1, 1)
        }));
        assert!(chunk.instructions().iter().any(|instruction| {
            matches!(
                instruction.opcode(),
                OpCode::JumpIfFalse(_) | OpCode::Loop(_)
            ) && instruction.span() == Span::new(2, 1)
        }));
        assert!(chunk.instructions().iter().any(|instruction| {
            matches!(instruction.opcode(), OpCode::Jump(_)) && instruction.span() == Span::new(3, 3)
        }));
        assert!(chunk.instructions().iter().any(|instruction| {
            matches!(instruction.opcode(), OpCode::Loop(_)) && instruction.span() == Span::new(4, 3)
        }));
    }

    #[test]
    fn compiles_true_and_true() {
        assert_eq!(
            run_source_and_get("let result = true && true;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_true_and_false() {
        assert_eq!(
            run_source_and_get("let result = true && false;", "result"),
            Value::Bool(false)
        );
    }

    #[test]
    fn compiles_false_and_true() {
        assert_eq!(
            run_source_and_get("let result = false && true;", "result"),
            Value::Bool(false)
        );
    }

    #[test]
    fn compiles_false_and_false() {
        assert_eq!(
            run_source_and_get("let result = false && false;", "result"),
            Value::Bool(false)
        );
    }

    #[test]
    fn compiles_true_or_true() {
        assert_eq!(
            run_source_and_get("let result = true || true;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_true_or_false() {
        assert_eq!(
            run_source_and_get("let result = true || false;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_false_or_true() {
        assert_eq!(
            run_source_and_get("let result = false || true;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_false_or_false() {
        assert_eq!(
            run_source_and_get("let result = false || false;", "result"),
            Value::Bool(false)
        );
    }

    #[test]
    fn logical_and_preserves_zero_left_operand() {
        assert_eq!(
            run_source_and_get("let result = 0 && 42;", "result"),
            Value::Int(0)
        );
    }

    #[test]
    fn logical_and_returns_right_operand_for_truthy_nonzero_left() {
        assert_eq!(
            run_source_and_get("let result = 2 && 42;", "result"),
            Value::Int(42)
        );
    }

    #[test]
    fn logical_or_returns_fallback_for_empty_string() {
        assert_eq!(
            run_source_and_get("let result = \"\" || \"fallback\";", "result"),
            Value::String("fallback".to_owned())
        );
    }

    #[test]
    fn logical_or_preserves_non_empty_string_left_operand() {
        assert_eq!(
            run_source_and_get("let result = \"left\" || \"fallback\";", "result"),
            Value::String("left".to_owned())
        );
    }

    #[test]
    fn false_and_skips_assignment_rhs() {
        assert_eq!(
            run_source_and_get("let x = 0; false && (x = 1);", "x"),
            Value::Int(0)
        );
    }

    #[test]
    fn true_or_skips_assignment_rhs() {
        assert_eq!(
            run_source_and_get("let x = 0; true || (x = 1);", "x"),
            Value::Int(0)
        );
    }

    #[test]
    fn true_and_evaluates_assignment_rhs() {
        assert_eq!(
            run_source_and_get("let x = 0; true && (x = 1);", "x"),
            Value::Int(1)
        );
    }

    #[test]
    fn false_or_evaluates_assignment_rhs() {
        assert_eq!(
            run_source_and_get("let x = 0; false || (x = 1);", "x"),
            Value::Int(1)
        );
    }

    #[test]
    fn compiles_chained_logical_and() {
        assert_eq!(
            run_source_and_get("let result = true && true && false;", "result"),
            Value::Bool(false)
        );
    }

    #[test]
    fn compiles_chained_logical_or() {
        assert_eq!(
            run_source_and_get("let result = false || true || false;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn compiles_mixed_logical_operators() {
        assert_eq!(
            run_source_and_get("let result = false || \"yes\" && 7;", "result"),
            Value::Int(7)
        );
    }

    #[test]
    fn compiles_parenthesized_logical_expression() {
        assert_eq!(
            run_source_and_get("let result = (true && false) || \"done\";", "result"),
            Value::String("done".to_owned())
        );
    }

    #[test]
    fn logical_compilation_preserves_parser_precedence() {
        assert_eq!(
            run_source_and_get("let result = true || false && false;", "result"),
            Value::Bool(true)
        );
    }

    #[test]
    fn logical_expression_statement_remains_stack_balanced() {
        let source = "let x = 0; false || 5; x = x + 1;";

        assert_eq!(run_source_and_get(source, "x"), Value::Int(1));
        assert_eq!(run_source(source), Value::Null);
    }

    #[test]
    fn logical_expression_works_as_if_condition() {
        assert_eq!(
            run_source_and_get("let x = 0; if (0 || \"go\") { x = 1; }", "x"),
            Value::Int(1)
        );
    }

    #[test]
    fn logical_expression_works_as_while_condition() {
        let source = "let x = 0; while (x < 3 && true) { x = x + 1; }";

        assert_eq!(run_source_and_get(source, "x"), Value::Int(3));
    }

    #[test]
    fn vm_is_reusable_after_compiled_logical_expressions() {
        let first = compile_source("let x = false || 1;").unwrap();
        let second = compile_source("let y = true && 2;").unwrap();
        let mut vm = Vm::new();

        assert_eq!(vm.run(&first), Ok(Value::Null));
        assert_eq!(vm.run(&second), Ok(Value::Null));
        assert_eq!(read_global(&mut vm, "x"), Value::Int(1));
        assert_eq!(read_global(&mut vm, "y"), Value::Int(2));
    }

    #[test]
    fn logical_and_disassembly_has_patched_valid_target() {
        let chunk = compile_source("let result = false && true;").unwrap();
        let output = disassemble_chunk(&chunk, "and").unwrap();

        assert!(output.contains("JUMP_IF_FALSE    2 -> 4"));
    }

    #[test]
    fn logical_or_disassembly_has_patched_valid_targets() {
        let chunk = compile_source("let result = true || false;").unwrap();
        let output = disassemble_chunk(&chunk, "or").unwrap();

        assert!(output.contains("JUMP_IF_FALSE    1 -> 3"));
        assert!(output.contains("JUMP             2 -> 5"));
    }

    #[test]
    fn logical_expression_disassembly_is_deterministic() {
        let chunk = compile_source("let result = false || true && false;").unwrap();

        assert_eq!(
            disassemble_chunk(&chunk, "logical").unwrap(),
            disassemble_chunk(&chunk, "logical").unwrap()
        );
    }

    #[test]
    fn vm_matches_interpreter_for_logical_and() {
        let source = "let result = 0 && 42;";

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }

    #[test]
    fn vm_matches_interpreter_for_logical_or() {
        let source = "let result = \"\" || \"fallback\";";

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }

    #[test]
    fn vm_matches_interpreter_for_mixed_logical_expression() {
        let source = "let result = false || \"yes\" && 7;";

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }

    #[test]
    fn vm_matches_interpreter_for_short_circuit_side_effect() {
        let source = "let x = 0; false && (x = 1); let result = x;";

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }

    #[test]
    fn function_declaration_defines_global_callable() {
        let chunk = compile_source("fn answer() { return 42; }").unwrap();
        let mut vm = Vm::new();

        assert_eq!(vm.run(&chunk), Ok(Value::Null));
        assert_eq!(read_global(&mut vm, "answer").to_string(), "<fn answer>");
    }

    #[test]
    fn calls_zero_argument_function() {
        assert_eq!(
            run_source_and_get(
                "fn answer() { return 42; } let result = answer();",
                "result"
            ),
            Value::Int(42)
        );
    }

    #[test]
    fn calls_one_argument_function() {
        assert_eq!(
            run_source_and_get(
                "fn identity(value) { return value; } let result = identity(7);",
                "result"
            ),
            Value::Int(7)
        );
    }

    #[test]
    fn calls_function_with_multiple_arguments() {
        assert_eq!(
            run_source_and_get(
                "fn add(a, b, c) { return a + b + c; } let result = add(10, 20, 12);",
                "result"
            ),
            Value::Int(42)
        );
    }

    #[test]
    fn function_returns_float() {
        assert_eq!(
            run_source_and_get("fn value() { return 2.5; } let result = value();", "result"),
            Value::Float(2.5)
        );
    }

    #[test]
    fn function_returns_string() {
        assert_eq!(
            run_source_and_get(
                "fn value() { return \"hanlin\"; } let result = value();",
                "result"
            ),
            Value::String("hanlin".to_owned())
        );
    }

    #[test]
    fn function_implicit_return_is_null() {
        assert_eq!(
            run_source_and_get("fn noop() { let x = 1; } let result = noop();", "result"),
            Value::Null
        );
    }

    #[test]
    fn function_bare_return_is_null() {
        assert_eq!(
            run_source_and_get("fn noop() { return; } let result = noop();", "result"),
            Value::Null
        );
    }

    #[test]
    fn parameter_compiles_to_local_slot_zero() {
        let function = compiled_function("fn identity(value) { return value; }", "identity");

        assert_eq!(
            function.chunk().instruction(0).unwrap().opcode(),
            OpCode::GetLocal(LocalSlot::new(0))
        );
    }

    #[test]
    fn multiple_parameters_use_distinct_local_slots() {
        let function = compiled_function("fn subtract(a, b) { return a - b; }", "subtract");
        let opcodes: Vec<_> = function
            .chunk()
            .instructions()
            .iter()
            .map(|instruction| instruction.opcode())
            .collect();

        assert_eq!(opcodes[0], OpCode::GetLocal(LocalSlot::new(0)));
        assert_eq!(opcodes[1], OpCode::GetLocal(LocalSlot::new(1)));
        assert_eq!(
            run_source_and_get(
                "fn subtract(a, b) { return a - b; } let result = subtract(9, 4);",
                "result"
            ),
            Value::Int(5)
        );
    }

    #[test]
    fn function_local_declaration_uses_stack_slot() {
        assert_eq!(
            run_source_and_get(
                "fn add_one(x) { let y = x + 1; return y; } let result = add_one(41);",
                "result"
            ),
            Value::Int(42)
        );
    }

    #[test]
    fn function_local_assignment_uses_set_local() {
        let source = "fn bump(x) { let y = x; y = y + 1; return y; } let result = bump(9);";
        let function = compiled_function(source, "bump");

        assert!(function
            .chunk()
            .instructions()
            .iter()
            .any(|instruction| { instruction.opcode() == OpCode::SetLocal(LocalSlot::new(1)) }));
        assert_eq!(run_source_and_get(source, "result"), Value::Int(10));
    }

    #[test]
    fn function_parameter_shadows_global() {
        let source = "let x = 100; fn identity(x) { return x; } let result = identity(7);";

        assert_eq!(run_source_and_get(source, "result"), Value::Int(7));
    }

    #[test]
    fn function_identifier_falls_back_to_global() {
        let source = "let base = 40; fn add_base(x) { return base + x; } let result = add_base(2);";

        assert_eq!(run_source_and_get(source, "result"), Value::Int(42));
    }

    #[test]
    fn too_few_arguments_is_runtime_error() {
        let error = run_source_result("fn add(a, b) { return a + b; } add(1);").unwrap_err();

        assert!(matches!(
            error,
            VmError::ArityMismatch {
                function,
                expected,
                got,
                ..
            } if function == "add" && expected == Arity::new(2) && got == Arity::new(1)
        ));
    }

    #[test]
    fn too_many_arguments_is_runtime_error() {
        let error = run_source_result("fn add(a, b) { return a + b; } add(1, 2, 3);").unwrap_err();

        assert!(matches!(
            error,
            VmError::ArityMismatch {
                function,
                expected,
                got,
                ..
            } if function == "add" && expected == Arity::new(2) && got == Arity::new(3)
        ));
    }

    #[test]
    fn calling_non_function_is_runtime_error() {
        let error = run_source_result("let value = 10; value();").unwrap_err();

        assert!(matches!(
            error,
            VmError::NotCallable {
                value: Value::Int(10),
                ..
            }
        ));
    }

    #[test]
    fn call_stack_is_cleaned_after_each_return() {
        let source = concat!(
            "fn add_one(x) { let y = x + 1; return y; } ",
            "let result = add_one(20) + add_one(20);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(42));
    }

    #[test]
    fn vm_is_reusable_after_call_error() {
        let bad = compile_source("let value = 10; value();").unwrap();
        let good = compile_source("fn answer() { return 42; } let result = answer();").unwrap();
        let mut vm = Vm::new();

        assert!(matches!(vm.run(&bad), Err(VmError::NotCallable { .. })));
        assert_eq!(vm.run(&good), Ok(Value::Null));
        assert_eq!(read_global(&mut vm, "result"), Value::Int(42));
    }

    #[test]
    fn function_can_call_another_function() {
        let source = concat!(
            "fn double(x) { return x * 2; } ",
            "fn add_two(x) { return double(x) + 2; } ",
            "let result = add_two(20);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(42));
    }

    #[test]
    fn supports_multiple_sequential_calls() {
        let source = concat!(
            "fn next(x) { return x + 1; } ",
            "let first = next(1); let second = next(first); let result = next(second);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(4));
    }

    #[test]
    fn call_result_works_inside_arithmetic_expression() {
        let source = "fn value() { return 6; } let result = value() * 7;";

        assert_eq!(run_source_and_get(source, "result"), Value::Int(42));
    }

    #[test]
    fn nested_calls_preserve_return_values() {
        let source = concat!(
            "fn add(a, b) { return a + b; } ",
            "fn double(x) { return x * 2; } ",
            "let result = double(add(10, 11));"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(42));
    }

    #[test]
    fn recursive_factorial_uses_call_frames() {
        let source = concat!(
            "fn factorial(n) { ",
            "  if (n <= 1) { return 1; } ",
            "  return n * factorial(n - 1); ",
            "} ",
            "let result = factorial(5);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(120));
    }

    #[test]
    fn recursive_countdown_returns_at_base_case() {
        let source = concat!(
            "fn countdown(n) { ",
            "  if (n <= 0) { return n; } ",
            "  return countdown(n - 1); ",
            "} ",
            "let result = countdown(20);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(0));
    }

    #[test]
    fn recursive_frames_grow_and_unwind_without_stale_values() {
        let source = concat!(
            "fn sum_to(n) { ",
            "  if (n <= 0) { return 0; } ",
            "  return n + sum_to(n - 1); ",
            "} ",
            "let first = sum_to(10); let result = sum_to(3) + first;"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(61));
    }

    #[test]
    fn recursive_call_stack_limit_is_structured_error() {
        let error = run_source_result("fn recurse() { return recurse(); } recurse();").unwrap_err();

        assert!(matches!(
            error,
            VmError::CallStackOverflow { limit, .. } if limit == MAX_CALL_FRAMES
        ));
    }

    #[test]
    fn function_supports_if_else() {
        let source = concat!(
            "fn sign(x) { if (x < 0) { return -1; } else { return 1; } } ",
            "let result = sign(-2);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(-1));
    }

    #[test]
    fn function_supports_while_loop() {
        let source = concat!(
            "fn sum(n) { let total = 0; while (n > 0) { ",
            "  total = total + n; n = n - 1; ",
            "} return total; } let result = sum(6);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(21));
    }

    #[test]
    fn return_inside_conditional_unwinds_function() {
        let source = concat!(
            "fn choose(flag) { if (flag) { return 42; } return 0; } ",
            "let result = choose(true);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(42));
    }

    #[test]
    fn return_inside_loop_unwinds_function() {
        let source = concat!(
            "fn first(n) { while (n > 0) { return n; } return 0; } ",
            "let result = first(42);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(42));
    }

    #[test]
    fn nested_function_declaration_is_compile_error() {
        assert_eq!(
            compile_source("fn outer() { fn inner() { return 1; } return 2; }"),
            Err(CompileError::NestedFunctionUnsupported {
                name: "inner".to_owned(),
                span: Span::new(1, 14),
            })
        );
    }

    #[test]
    fn block_local_declaration_in_function_is_compile_error() {
        assert!(matches!(
            compile_source("fn invalid() { if (true) { let value = 1; } }"),
            Err(CompileError::BlockLocalUnsupported { name, .. }) if name == "value"
        ));
    }

    #[test]
    fn excessive_parameter_count_is_compile_error() {
        let parameters = (0..=u8::MAX)
            .map(|index| format!("p{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("fn oversized({parameters}) {{ return null; }}");

        assert!(matches!(
            compile_source(&source),
            Err(CompileError::TooManyParameters {
                count: 256,
                maximum: 255,
                ..
            })
        ));
    }

    #[test]
    fn excessive_call_argument_count_is_compile_error() {
        let arguments = std::iter::repeat_n("0", usize::from(u8::MAX) + 1)
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("oversized({arguments});");

        assert!(matches!(
            compile_source(&source),
            Err(CompileError::TooManyArguments {
                count: 256,
                maximum: 255,
                ..
            })
        ));
    }

    #[test]
    fn call_arguments_evaluate_left_to_right() {
        let source = concat!(
            "let x = 0; fn pair(a, b) { return a * 10 + b; } ",
            "let result = pair(x = 1, x = 2);"
        );

        assert_eq!(run_source_and_get(source, "result"), Value::Int(12));
    }

    #[test]
    fn vm_matches_interpreter_for_simple_function_call() {
        let source = "fn add(a, b) { return a + b; } let result = add(20, 22);";

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }

    #[test]
    fn vm_matches_interpreter_for_function_with_locals() {
        let source = "fn add_one(x) { let y = x + 1; return y; } let result = add_one(41);";

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }

    #[test]
    fn vm_matches_interpreter_for_recursion() {
        let source = concat!(
            "fn factorial(n) { if (n <= 1) { return 1; } ",
            "return n * factorial(n - 1); } let result = factorial(5);"
        );

        assert_eq!(
            run_source_and_get(source, "result").to_string(),
            interpreter_global(source, "result")
        );
    }

    #[test]
    fn rejects_source_return_statement_with_span() {
        assert_eq!(
            compile_source("return 1;"),
            Err(CompileError::ReturnOutsideFunction {
                span: Span::new(1, 1),
            })
        );
    }

    #[test]
    fn rejects_for_statement() {
        assert_unsupported_statement("for (;;) { break; }", "for statement");
    }

    #[test]
    fn compiles_function_declaration() {
        assert!(compile_source("fn answer() { return 42; }")
            .unwrap()
            .constants()
            .iter()
            .any(|value| matches!(value, Value::Function(_))));
    }

    #[test]
    fn compiles_function_call() {
        assert_eq!(
            run_source_and_get(
                "fn answer() { return 42; } let result = answer();",
                "result"
            ),
            Value::Int(42)
        );
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
    fn rejects_object_literal() {
        assert_unsupported_expression("let result = { value: 1 };", "object literal");
    }

    #[test]
    fn rejects_member_access() {
        assert_unsupported_expression("let result = value.member;", "member access");
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

    #[test]
    fn vm_matches_interpreter_for_if_global() {
        let source = "let x = 0; if (true) { x = 10; }";

        assert_eq!(
            run_source_and_get(source, "x").to_string(),
            interpreter_global(source, "x")
        );
    }

    #[test]
    fn vm_matches_interpreter_for_while_global() {
        let source = "let x = 0; while (x < 3) { x = x + 1; }";

        assert_eq!(
            run_source_and_get(source, "x").to_string(),
            interpreter_global(source, "x")
        );
    }
}
