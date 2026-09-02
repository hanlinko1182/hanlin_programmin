// =============================================================================
//  ast.rs — Abstract Syntax Tree Node Definitions  (v0.2)
// =============================================================================
//
//  v0.2 additions:
//    Expr::ArrayLiteral  — [elem, elem, ...]
//    Expr::ObjectLiteral — { key: expr, key: expr, ... }
//    Expr::Index         — expr[index]
//    Expr::Member        — expr.property
//    Expr::MethodCall    — expr.method(args...)
//    Expr::AssignIndex   — expr[index] = value
//    Expr::AssignMember  — expr.property = value
// =============================================================================

use crate::error::Span;

// ---------------------------------------------------------------------------
//  Literals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Integer(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
}

// ---------------------------------------------------------------------------
//  Operators
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
}

impl BinOp {
    pub fn as_c_op(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::EqEq => "==",
            BinOp::NotEq => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::LtEq => "<=",
            BinOp::GtEq => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg, // -expr
    Not, // !expr
}

impl UnOp {
    pub fn as_c_op(&self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "!",
        }
    }
}

// ---------------------------------------------------------------------------
//  Expressions
// ---------------------------------------------------------------------------

/// An expression — something that produces a value when evaluated.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    // ── Atoms ───────────────────────────────────────────────────────────────
    /// A compile-time constant literal.
    Literal(Literal),

    /// A variable name reference.
    Identifier(String),

    // ── Data structure literals (NEW v0.2) ───────────────────────────────────
    /// Array literal:  `[1, 2, "hello", true]`
    ArrayLiteral {
        elements: Vec<Expr>,
        span: Span,
    },

    /// Object literal: `{ name: "Han", age: 20 }`
    ObjectLiteral {
        /// Ordered list of (key, value) pairs.
        pairs: Vec<(String, Expr)>,
        span: Span,
    },

    // ── Access expressions (NEW v0.2) ────────────────────────────────────────
    /// Index access:  `arr[0]`  or  `map["key"]`
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },

    /// Property access via dot notation: `user.name`
    Member {
        object: Box<Expr>,
        property: String,
        span: Span,
    },

    /// Method call via dot notation: `arr.push(4)`  or  `str.length()`
    MethodCall {
        object: Box<Expr>,
        method: String,
        args: Vec<Expr>,
        span: Span,
    },

    // ── Arithmetic / logical ─────────────────────────────────────────────────
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },

    // ── Assignment ────────────────────────────────────────────────────────────
    /// Simple variable assignment: `x = value`
    Assign {
        name: String,
        value: Box<Expr>,
    },

    /// Index assignment: `arr[0] = value`  (NEW v0.2)
    AssignIndex {
        object: Box<Expr>,
        index: Box<Expr>,
        value: Box<Expr>,
        span: Span,
    },

    /// Member assignment: `user.name = value`  (NEW v0.2)
    AssignMember {
        object: Box<Expr>,
        property: String,
        value: Box<Expr>,
        span: Span,
    },

    // ── Function call ─────────────────────────────────────────────────────────
    /// Regular function call: `add(1, 2)`
    Call {
        callee: String,
        args: Vec<Expr>,
    },
}

// ---------------------------------------------------------------------------
//  Statements
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// Variable declaration: `let x = expr;` or `const PI = 3.14;`
    VarDecl {
        name: String,
        is_const: bool,
        init: Option<Expr>,
        span: Span,
    },

    /// Function declaration: `fn name(params) { body }`
    FnDecl {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
        span: Span,
    },

    /// Return statement: `return expr?;`
    Return { value: Option<Expr>, span: Span },

    /// If / else: `if (cond) { ... } else { ... }`
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
        span: Span,
    },

    /// While loop: `while (cond) { ... }`
    While {
        condition: Expr,
        body: Vec<Stmt>,
        span: Span,
    },

    /// Expression statement: `expr;`
    Expression { expr: Expr, span: Span },

    /// Built-in print: `print(expr, expr, ...);`
    Print { args: Vec<Expr>, span: Span },

    /// TryCatch exception block: `try { try_body } catch (catch_var) { catch_body }`
    TryCatch {
        try_body: Vec<Stmt>,
        catch_var: String,
        catch_body: Vec<Stmt>,
        span: Span,
    },

    /// For loop: `for (init?; condition?; update?) { body }`
    ///
    /// - `init` is either a `VarDecl` or an `Expression` statement (boxed so Stmt
    ///   doesn't become infinitely sized on the stack).
    /// - `condition` is an optional expression evaluated before each iteration;
    ///   when absent the loop runs until a `break` or `return`.
    /// - `update` is an optional expression evaluated after each body execution
    ///   (and also after a `continue`, before re-checking the condition).
    For {
        init: Option<Box<Stmt>>,
        condition: Option<Expr>,
        update: Option<Expr>,
        body: Vec<Stmt>,
        span: Span,
    },

    /// Break statement: exits the innermost `for` or `while` loop.
    ///
    /// Produces a runtime error when used outside any loop.
    Break { span: Span },

    /// Continue statement: skips the remainder of the current loop body,
    /// executes the `for`-loop update expression (if any), then re-checks
    /// the loop condition.
    ///
    /// Produces a runtime error when used outside any loop.
    Continue { span: Span },
}

// ---------------------------------------------------------------------------
//  Program (root node)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub body: Vec<Stmt>,
}

impl Program {
    pub fn new(body: Vec<Stmt>) -> Self {
        Program { body }
    }
}
