// =============================================================================
//  error.rs — Compiler Error Types & Reporting
// =============================================================================
//
//  Defines a unified `HanlinError` type used across all compiler phases
//  (lexing, parsing, code generation) with source-location information.
// =============================================================================

use std::fmt;

/// Source location (line and column) for error reporting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(line: usize, col: usize) -> Self {
        Span { line, col }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// Which phase of compilation produced this error.
#[derive(Debug, Clone)]
pub enum ErrorKind {
    Lexer,
    Parser,
    CodeGen,
    Runtime,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Lexer => write!(f, "LexError"),
            ErrorKind::Parser => write!(f, "ParseError"),
            ErrorKind::CodeGen => write!(f, "CodeGenError"),
            ErrorKind::Runtime => write!(f, "RuntimeError"),
        }
    }
}

/// A structured compiler error with phase, location, and message.
#[derive(Debug, Clone)]
pub struct HanlinError {
    pub kind: ErrorKind,
    pub span: Option<Span>,
    pub message: String,
}

impl HanlinError {
    pub fn lexer(span: Span, msg: impl Into<String>) -> Self {
        HanlinError {
            kind: ErrorKind::Lexer,
            span: Some(span),
            message: msg.into(),
        }
    }

    pub fn parser(span: Option<Span>, msg: impl Into<String>) -> Self {
        HanlinError {
            kind: ErrorKind::Parser,
            span,
            message: msg.into(),
        }
    }

    pub fn codegen(msg: impl Into<String>) -> Self {
        HanlinError {
            kind: ErrorKind::CodeGen,
            span: None,
            message: msg.into(),
        }
    }

    pub fn runtime(span: Option<Span>, msg: impl Into<String>) -> Self {
        HanlinError {
            kind: ErrorKind::Runtime,
            span,
            message: msg.into(),
        }
    }
}

impl fmt::Display for HanlinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.span {
            Some(span) => write!(f, "[{}] at {} — {}", self.kind, span, self.message),
            None => write!(f, "[{}] — {}", self.kind, self.message),
        }
    }
}

impl std::error::Error for HanlinError {}

/// Convenience alias used throughout the compiler.
pub type Result<T> = std::result::Result<T, HanlinError>;
