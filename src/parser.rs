// =============================================================================
//  parser.rs — Recursive Descent Parser  (v0.2)
// =============================================================================
//
//  v0.2 additions:
//    - Array literals   [1, 2, 3]
//    - Object literals  { key: value, ... }
//    - Postfix chain    expr[i]  |  expr.prop  |  expr.method(args)
//    - Updated assignment: lhs = rhs  where lhs can be Index or Member
//
//  Grammar (updated):
//
//    program    := statement* EOF
//    statement  := fn_decl | var_decl | return_stmt | if_stmt
//                | while_stmt | print_stmt | expr_stmt
//    expr       := assignment
//    assignment := postfix "=" assignment   (converts lhs to Assign/AssignIndex/AssignMember)
//               | logical_or
//    logical_or  := logical_and ("||" logical_and)*
//    logical_and := equality   ("&&" equality)*
//    equality    := comparison (("==" | "!=") comparison)*
//    comparison  := term       (("<" | ">" | "<=" | ">=") term)*
//    term        := factor     (("+" | "-") factor)*
//    factor      := unary      (("*" | "/" | "%") unary)*
//    unary       := ("-" | "!") unary | postfix
//    postfix     := primary ( "[" expr "]"
//                           | "." IDENT "(" args ")"
//                           | "." IDENT
//                           | "(" args ")"   ← only when primary is Identifier
//                           )*
//    primary     := INTEGER | FLOAT | STRING | "true" | "false"
//                 | "[" args "]"          ← array literal
//                 | "{" (IDENT ":" expr ("," IDENT ":" expr)*)? "}"  ← object literal
//                 | IDENT
//                 | "(" expr ")"
// =============================================================================

use crate::ast::{BinOp, Expr, Literal, Program, Stmt, UnOp};
use crate::error::{HanlinError, Result, Span};
use crate::lexer::{Token, TokenKind};

// ---------------------------------------------------------------------------
//  Parser struct
// ---------------------------------------------------------------------------

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, current: 0 }
    }

    pub fn parse(&mut self) -> Result<Program> {
        let mut stmts = Vec::new();
        while !self.is_at_end() {
            stmts.push(self.parse_statement()?);
        }
        Ok(Program::new(stmts))
    }

    // ── Token navigation ───────────────────────────────────────────────────

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn check(&self, kind: &TokenKind) -> bool {
        token_kind_variant_eq(&self.peek().kind, kind)
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, msg: &str) -> Result<Span> {
        if self.check(kind) {
            let span = self.peek().span;
            self.advance();
            Ok(span)
        } else {
            Err(HanlinError::parser(
                Some(self.peek().span),
                format!("{} — got {:?}", msg, self.peek().kind),
            ))
        }
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn current_span(&self) -> Span {
        self.peek().span
    }

    // =========================================================================
    //  Statement parsing
    // =========================================================================

    fn parse_statement(&mut self) -> Result<Stmt> {
        match &self.peek().kind {
            TokenKind::Fn => self.parse_fn_decl(),
            TokenKind::Let => self.parse_var_decl(false),
            TokenKind::Const => self.parse_var_decl(true),
            TokenKind::Return => self.parse_return(),
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Print => self.parse_print(),
            TokenKind::Try => self.parse_try_catch(),
            TokenKind::Break => {
                let span = self.current_span();
                self.advance(); // consume `break`
                self.expect(&TokenKind::Semicolon, "expected ';' after 'break'")?;
                Ok(Stmt::Break { span })
            }
            TokenKind::Continue => {
                let span = self.current_span();
                self.advance(); // consume `continue`
                self.expect(&TokenKind::Semicolon, "expected ';' after 'continue'")?;
                Ok(Stmt::Continue { span })
            }
            _ => self.parse_expr_stmt(),
        }
    }

    fn parse_fn_decl(&mut self) -> Result<Stmt> {
        let span = self.current_span();
        self.advance(); // consume `fn`
        let name = self.expect_identifier("expected function name")?;
        self.expect(&TokenKind::LParen, "expected '(' after function name")?;
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::RParen, "expected ')' after parameters")?;
        let body = self.parse_block()?;
        Ok(Stmt::FnDecl {
            name,
            params,
            body,
            span,
        })
    }

    fn parse_param_list(&mut self) -> Result<Vec<String>> {
        let mut params = Vec::new();
        if self.check(&TokenKind::RParen) {
            return Ok(params);
        }
        params.push(self.expect_identifier("expected parameter name")?);
        while self.match_token(&TokenKind::Comma) {
            params.push(self.expect_identifier("expected parameter name")?);
        }
        Ok(params)
    }

    fn parse_var_decl(&mut self, is_const: bool) -> Result<Stmt> {
        let span = self.current_span();
        self.advance(); // consume `let` / `const`
        let name = self.expect_identifier("expected variable name")?;
        let init = if self.match_token(&TokenKind::Eq) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(
            &TokenKind::Semicolon,
            "expected ';' after variable declaration",
        )?;
        Ok(Stmt::VarDecl {
            name,
            is_const,
            init,
            span,
        })
    }

    fn parse_return(&mut self) -> Result<Stmt> {
        let span = self.current_span();
        self.advance(); // consume `return`
        let value = if !self.check(&TokenKind::Semicolon) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(&TokenKind::Semicolon, "expected ';' after return value")?;
        Ok(Stmt::Return { value, span })
    }

    fn parse_if(&mut self) -> Result<Stmt> {
        let span = self.current_span();
        self.advance(); // consume `if`
        self.expect(&TokenKind::LParen, "expected '(' after 'if'")?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "expected ')' after if condition")?;
        let then_body = self.parse_block()?;

        let else_body = if self.match_token(&TokenKind::Else) {
            if self.check(&TokenKind::If) {
                // Lower `else if` to a nested block containing a single `If` statement.
                let nested_if = self.parse_if()?;
                Some(vec![nested_if])
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };

        Ok(Stmt::If {
            condition,
            then_body,
            else_body,
            span,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt> {
        let span = self.current_span();
        self.advance(); // consume `while`
        self.expect(&TokenKind::LParen, "expected '(' after 'while'")?;
        let condition = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "expected ')' after while condition")?;
        let body = self.parse_block()?;
        Ok(Stmt::While {
            condition,
            body,
            span,
        })
    }

    fn parse_print(&mut self) -> Result<Stmt> {
        let span = self.current_span();
        self.advance(); // consume `print`
        self.expect(&TokenKind::LParen, "expected '(' after 'print'")?;
        let args = self.parse_arg_list()?;
        self.expect(&TokenKind::RParen, "expected ')' after print arguments")?;
        self.expect(&TokenKind::Semicolon, "expected ';' after print statement")?;
        Ok(Stmt::Print { args, span })
    }

    // ── try-catch ─────────────────────────────────────────────────────────────
    //
    //  Syntax:
    //    try { stmt* } catch ( IDENT ) { stmt* }
    //
    //  The catch variable is bound to the error message string in the catch body.

    fn parse_try_catch(&mut self) -> Result<Stmt> {
        let span = self.current_span();
        self.advance(); // consume `try`
        let try_body = self.parse_block()?;

        self.expect(&TokenKind::Catch, "expected 'catch' after try block")?;
        self.expect(&TokenKind::LParen, "expected '(' after 'catch'")?;
        let catch_var = self.expect_identifier("expected error variable name in catch")?;
        self.expect(&TokenKind::RParen, "expected ')' after catch variable")?;
        let catch_body = self.parse_block()?;

        Ok(Stmt::TryCatch {
            try_body,
            catch_var,
            catch_body,
            span,
        })
    }

    // ── for loop ─────────────────────────────────────────────────────────────
    //
    //  Syntax:
    //    for ( init? ; condition? ; update? ) { body }
    //
    //  All three clauses are optional:
    //    for (;;)         — infinite loop (needs break/return to exit)
    //    for (; x < 10;)  — condition-only (like while)
    //    for (let i = 0; i < 10; i = i + 1)  — full C-style
    //
    //  init may be:
    //    - a variable declaration:  let i = 0;    (var_decl, consumes its ';')
    //    - an expression statement: i = 0;        (expr + consumed ';')
    //    - empty:                   ;             (just the separator)
    //
    //  update is an expression (not a statement), no trailing ';' before ')'.

    fn parse_for(&mut self) -> Result<Stmt> {
        let span = self.current_span();
        self.advance(); // consume `for`
        self.expect(&TokenKind::LParen, "expected '(' after 'for'")?;

        // ── init clause ────────────────────────────────────────────────────
        let init: Option<Box<Stmt>> = if self.check(&TokenKind::Semicolon) {
            self.advance(); // consume the empty ';'
            None
        } else if self.check(&TokenKind::Let) || self.check(&TokenKind::Const) {
            // var_decl already consumes its own ';'
            let is_const = self.check(&TokenKind::Const);
            Some(Box::new(self.parse_var_decl(is_const)?))
        } else {
            // expression statement — parse expr, then expect ';'
            let init_span = self.current_span();
            let expr = self.parse_expr()?;
            self.expect(
                &TokenKind::Semicolon,
                "expected ';' after for-loop initializer",
            )?;
            Some(Box::new(Stmt::Expression {
                expr,
                span: init_span,
            }))
        };

        // ── condition clause ───────────────────────────────────────────────
        let condition: Option<Expr> = if self.check(&TokenKind::Semicolon) {
            self.advance(); // consume empty ';'
            None
        } else {
            let cond = self.parse_expr()?;
            self.expect(
                &TokenKind::Semicolon,
                "expected ';' after for-loop condition",
            )?;
            Some(cond)
        };

        // ── update clause ──────────────────────────────────────────────────
        let update: Option<Expr> = if self.check(&TokenKind::RParen) {
            None
        } else {
            Some(self.parse_expr()?)
        };

        self.expect(&TokenKind::RParen, "expected ')' after for-loop clauses")?;

        // ── body ───────────────────────────────────────────────────────────
        let body = self.parse_block()?;

        Ok(Stmt::For {
            init,
            condition,
            update,
            body,
            span,
        })
    }

    fn parse_expr_stmt(&mut self) -> Result<Stmt> {
        let span = self.current_span();
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semicolon, "expected ';' after expression")?;
        Ok(Stmt::Expression { expr, span })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        self.expect(&TokenKind::LBrace, "expected '{'")?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            stmts.push(self.parse_statement()?);
        }
        self.expect(&TokenKind::RBrace, "expected '}'")?;
        Ok(stmts)
    }

    // =========================================================================
    //  Expression parsing
    // =========================================================================

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_assignment()
    }

    // ── Assignment (right-associative, generalised lhs) ─────────────────────
    //
    //  Parse the lhs as a full logical_or expression, then check for `=`.
    //  If we see `=`, validate that lhs is an assignable target:
    //    Identifier(x)        → Assign { name, value }
    //    Index { .. }         → AssignIndex { .. }
    //    Member { .. }        → AssignMember { .. }
    //  Otherwise, return lhs unchanged.

    fn parse_assignment(&mut self) -> Result<Expr> {
        let lhs = self.parse_logical_or()?;

        let assignment_op = if self.match_token(&TokenKind::Eq) {
            Some(None)
        } else if self.match_token(&TokenKind::PlusEq) {
            Some(Some(BinOp::Add))
        } else if self.match_token(&TokenKind::MinusEq) {
            Some(Some(BinOp::Sub))
        } else if self.match_token(&TokenKind::StarEq) {
            Some(Some(BinOp::Mul))
        } else if self.match_token(&TokenKind::SlashEq) {
            Some(Some(BinOp::Div))
        } else {
            None
        };

        if let Some(compound_op) = assignment_op {
            let span = self.previous().span;
            let rhs = self.parse_assignment()?;
            let value = if let Some(op) = compound_op {
                let original = lhs.clone();
                Box::new(Expr::Binary {
                    op,
                    left: Box::new(original),
                    right: Box::new(rhs),
                })
            } else {
                Box::new(rhs)
            };

            return match lhs {
                Expr::Identifier(name) => Ok(Expr::Assign { name, value }),

                Expr::Index {
                    object,
                    index,
                    span: s,
                } => Ok(Expr::AssignIndex {
                    object,
                    index,
                    value,
                    span: s,
                }),

                Expr::Member {
                    object,
                    property,
                    span: s,
                } => Ok(Expr::AssignMember {
                    object,
                    property,
                    value,
                    span: s,
                }),

                _ => Err(HanlinError::parser(
                    Some(span),
                    "invalid assignment target — must be a variable, index, or property",
                )),
            };
        }

        Ok(lhs)
    }

    // ── Logical or / and / equality / comparison / term / factor ────────────
    //  (these are unchanged from v0.1 structurally)

    fn parse_logical_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_logical_and()?;
        while self.match_token(&TokenKind::PipePipe) {
            let right = self.parse_logical_and()?;
            left = Expr::Binary {
                op: BinOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_logical_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_equality()?;
        while self.match_token(&TokenKind::AmpAmp) {
            let right = self.parse_equality()?;
            left = Expr::Binary {
                op: BinOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = if self.match_token(&TokenKind::EqEq) {
                BinOp::EqEq
            } else if self.match_token(&TokenKind::BangEq) {
                BinOp::NotEq
            } else {
                break;
            };
            let right = self.parse_comparison()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let mut left = self.parse_term()?;
        loop {
            let op = if self.match_token(&TokenKind::Lt) {
                BinOp::Lt
            } else if self.match_token(&TokenKind::Gt) {
                BinOp::Gt
            } else if self.match_token(&TokenKind::LtEq) {
                BinOp::LtEq
            } else if self.match_token(&TokenKind::GtEq) {
                BinOp::GtEq
            } else {
                break;
            };
            let right = self.parse_term()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr> {
        let mut left = self.parse_factor()?;
        loop {
            let op = if self.match_token(&TokenKind::Plus) {
                BinOp::Add
            } else if self.match_token(&TokenKind::Minus) {
                BinOp::Sub
            } else {
                break;
            };
            let right = self.parse_factor()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> Result<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = if self.match_token(&TokenKind::Star) {
                BinOp::Mul
            } else if self.match_token(&TokenKind::Slash) {
                BinOp::Div
            } else if self.match_token(&TokenKind::Percent) {
                BinOp::Mod
            } else {
                break;
            };
            let right = self.parse_unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if self.match_token(&TokenKind::Minus) {
            return Ok(Expr::Unary {
                op: UnOp::Neg,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.match_token(&TokenKind::Bang) {
            return Ok(Expr::Unary {
                op: UnOp::Not,
                expr: Box::new(self.parse_unary()?),
            });
        }
        self.parse_postfix()
    }

    // ── Postfix chain (NEW v0.2) ─────────────────────────────────────────────
    //
    //  Handles left-to-right chains of:
    //    [index]         → Expr::Index
    //    .property       → Expr::Member
    //    .method(args)   → Expr::MethodCall
    //    (args)          → Expr::Call  (only when base is Identifier)

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;

        loop {
            let span = self.current_span();

            if self.match_token(&TokenKind::LBracket) {
                // ── Index access: expr[index] ─────────────────────────────
                let index = self.parse_expr()?;
                self.expect(&TokenKind::RBracket, "expected ']' after index")?;
                expr = Expr::Index {
                    object: Box::new(expr),
                    index: Box::new(index),
                    span,
                };
            } else if self.match_token(&TokenKind::Dot) {
                // ── Member / method: expr.name  or  expr.name(args) ──────
                let name = self.expect_identifier("expected property or method name after '.'")?;
                if self.match_token(&TokenKind::LParen) {
                    // Method call
                    let args = self.parse_arg_list()?;
                    self.expect(&TokenKind::RParen, "expected ')' after method arguments")?;
                    expr = Expr::MethodCall {
                        object: Box::new(expr),
                        method: name,
                        args,
                        span,
                    };
                } else {
                    // Property access
                    expr = Expr::Member {
                        object: Box::new(expr),
                        property: name,
                        span,
                    };
                }
            } else if self.check(&TokenKind::LParen) {
                // ── Function call: only valid if base is an identifier ────
                if let Expr::Identifier(callee) = expr {
                    self.advance(); // consume '('
                    let args = self.parse_arg_list()?;
                    self.expect(&TokenKind::RParen, "expected ')' after arguments")?;
                    expr = Expr::Call { callee, args };
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(expr)
    }

    // ── Primary (atoms) ────────────────────────────────────────────────────

    fn parse_primary(&mut self) -> Result<Expr> {
        let token = self.peek().clone();

        match &token.kind {
            TokenKind::Integer(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Literal(Literal::Integer(n)))
            }
            TokenKind::Float(f) => {
                let f = *f;
                self.advance();
                Ok(Expr::Literal(Literal::Float(f)))
            }
            TokenKind::StringLit(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expr::Literal(Literal::Str(s)))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(true)))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::Literal(Literal::Bool(false)))
            }
            TokenKind::Null => {
                self.advance();
                Ok(Expr::Literal(Literal::Null))
            }

            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(Expr::Identifier(name))
            }

            // ── Array literal: [elem, elem, ...] ──────────────────────────
            TokenKind::LBracket => {
                let span = token.span;
                self.advance(); // consume '['
                let mut elements = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    elements.push(self.parse_expr()?);
                    while self.match_token(&TokenKind::Comma) {
                        // Allow trailing comma before ']'
                        if self.check(&TokenKind::RBracket) {
                            break;
                        }
                        elements.push(self.parse_expr()?);
                    }
                }
                self.expect(&TokenKind::RBracket, "expected ']' after array elements")?;
                Ok(Expr::ArrayLiteral { elements, span })
            }

            // ── Object literal: { key: value, key: value } ────────────────
            //    Disambiguated from block statement: only valid as an expression.
            TokenKind::LBrace => {
                let span = token.span;
                self.advance(); // consume '{'
                let mut pairs = Vec::new();

                if !self.check(&TokenKind::RBrace) {
                    // Parse first pair
                    let key = self.expect_identifier("expected property name in object literal")?;
                    self.expect(&TokenKind::Colon, "expected ':' after property name")?;
                    let val = self.parse_expr()?;
                    pairs.push((key, val));

                    // Parse remaining pairs
                    while self.match_token(&TokenKind::Comma) {
                        if self.check(&TokenKind::RBrace) {
                            break;
                        } // trailing comma
                        let key = self.expect_identifier("expected property name")?;
                        self.expect(&TokenKind::Colon, "expected ':' after property name")?;
                        let val = self.parse_expr()?;
                        pairs.push((key, val));
                    }
                }

                self.expect(&TokenKind::RBrace, "expected '}' to close object literal")?;
                Ok(Expr::ObjectLiteral { pairs, span })
            }

            // ── Grouped expression: (expr) ────────────────────────────────
            TokenKind::LParen => {
                self.advance(); // consume '('
                let expr = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "expected ')' after grouped expression")?;
                Ok(expr)
            }

            _ => Err(HanlinError::parser(
                Some(token.span),
                format!("unexpected token {:?} in expression", token.kind),
            )),
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    fn expect_identifier(&mut self, msg: &str) -> Result<String> {
        if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            self.advance();
            Ok(name)
        } else {
            Err(HanlinError::parser(
                Some(self.current_span()),
                format!("{} — got {:?}", msg, self.peek().kind),
            ))
        }
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Expr>> {
        let mut args = Vec::new();
        if self.check(&TokenKind::RParen) {
            return Ok(args);
        }
        args.push(self.parse_expr()?);
        while self.match_token(&TokenKind::Comma) {
            if self.check(&TokenKind::RParen) {
                break;
            }
            args.push(self.parse_expr()?);
        }
        Ok(args)
    }
}

// ---------------------------------------------------------------------------
//  Token kind variant equality (ignores payload)
// ---------------------------------------------------------------------------

fn token_kind_variant_eq(a: &TokenKind, b: &TokenKind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

// =============================================================================
//  Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> Program {
        let tokens = Lexer::new(src).tokenize().expect("lex failed");
        Parser::new(tokens).parse().expect("parse failed")
    }

    #[test]
    fn test_let_declaration() {
        let prog = parse("let x = 42;");
        match &prog.body[0] {
            Stmt::VarDecl {
                name,
                is_const,
                init: Some(Expr::Literal(Literal::Integer(42))),
                ..
            } => {
                assert_eq!(name, "x");
                assert!(!is_const);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_binary_arithmetic() {
        let prog = parse("let result = 3 + 4 * 2;");
        match &prog.body[0] {
            Stmt::VarDecl {
                init:
                    Some(Expr::Binary {
                        op: BinOp::Add,
                        right,
                        ..
                    }),
                ..
            } => {
                assert!(matches!(
                    right.as_ref(),
                    Expr::Binary { op: BinOp::Mul, .. }
                ));
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_function_decl() {
        let prog = parse("fn add(a, b) { return a + b; }");
        match &prog.body[0] {
            Stmt::FnDecl {
                name, params, body, ..
            } => {
                assert_eq!(name, "add");
                assert_eq!(params, &["a", "b"]);
                assert_eq!(body.len(), 1);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_if_else() {
        let prog = parse("if (x > 0) { return x; } else { return 0; }");
        match &prog.body[0] {
            Stmt::If {
                else_body,
                then_body,
                ..
            } => {
                assert!(else_body.is_some());
                assert_eq!(then_body.len(), 1);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_while_loop() {
        let prog = parse("while (i < 10) { i = i + 1; }");
        match &prog.body[0] {
            Stmt::While { body, .. } => assert_eq!(body.len(), 1),
            other => panic!("unexpected: {:?}", other),
        }
    }

    // ── v0.2 new parser tests ───────────────────────────────────────────────

    #[test]
    fn test_array_literal() {
        let prog = parse("let a = [1, 2, 3];");
        match &prog.body[0] {
            Stmt::VarDecl {
                init: Some(Expr::ArrayLiteral { elements, .. }),
                ..
            } => {
                assert_eq!(elements.len(), 3);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_object_literal() {
        let prog = parse(r#"let u = { name: "Han", age: 20 };"#);
        match &prog.body[0] {
            Stmt::VarDecl {
                init: Some(Expr::ObjectLiteral { pairs, .. }),
                ..
            } => {
                assert_eq!(pairs.len(), 2);
                assert_eq!(pairs[0].0, "name");
                assert_eq!(pairs[1].0, "age");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_index_access() {
        let prog = parse("let x = arr[0];");
        match &prog.body[0] {
            Stmt::VarDecl {
                init: Some(Expr::Index { .. }),
                ..
            } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_member_access() {
        let prog = parse("let n = user.name;");
        match &prog.body[0] {
            Stmt::VarDecl {
                init: Some(Expr::Member { property, .. }),
                ..
            } => {
                assert_eq!(property, "name");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_method_call() {
        let prog = parse("arr.push(42);");
        match &prog.body[0] {
            Stmt::Expression {
                expr: Expr::MethodCall { method, args, .. },
                ..
            } => {
                assert_eq!(method, "push");
                assert_eq!(args.len(), 1);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_assign_index() {
        let prog = parse("arr[0] = 99;");
        match &prog.body[0] {
            Stmt::Expression {
                expr: Expr::AssignIndex { .. },
                ..
            } => {}
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_assign_member() {
        let prog = parse("user.age = 21;");
        match &prog.body[0] {
            Stmt::Expression {
                expr: Expr::AssignMember { property, .. },
                ..
            } => {
                assert_eq!(property, "age");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    // ── v0.3 for / break / continue parser tests ─────────────────────────────

    #[test]
    fn test_for_loop_full() {
        // for (let i = 0; i < 10; i = i + 1) { }
        let prog = parse("for (let i = 0; i < 10; i = i + 1) { }");
        match &prog.body[0] {
            Stmt::For {
                init,
                condition,
                update,
                body,
                ..
            } => {
                assert!(init.is_some(), "init should be Some");
                assert!(condition.is_some(), "condition should be Some");
                assert!(update.is_some(), "update should be Some");
                assert_eq!(body.len(), 0, "empty body");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_for_loop_empty_clauses() {
        // for (;;) { }  — all clauses absent
        let prog = parse("for (;;) { }");
        match &prog.body[0] {
            Stmt::For {
                init,
                condition,
                update,
                ..
            } => {
                assert!(init.is_none(), "init should be None");
                assert!(condition.is_none(), "condition should be None");
                assert!(update.is_none(), "update should be None");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_break_stmt() {
        let prog = parse("while (true) { break; }");
        match &prog.body[0] {
            Stmt::While { body, .. } => {
                assert!(matches!(body[0], Stmt::Break { .. }));
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn test_continue_stmt() {
        let prog = parse("while (true) { continue; }");
        match &prog.body[0] {
            Stmt::While { body, .. } => {
                assert!(matches!(body[0], Stmt::Continue { .. }));
            }
            other => panic!("unexpected: {:?}", other),
        }
    }
}
