// =============================================================================
//  lexer.rs — Lexer / Tokenizer for the hanlin language  (v0.2)
// =============================================================================
//
//  v0.2 additions:
//    - `Colon`  token  ':'  for object literals  { key: value }
//
//  Supported tokens:
//    Keywords  : let, const, fn, if, else, return, while, for, true, false, print
//    Literals  : integers (42), floats (3.14), strings ("hello")
//    Operators : + - * / % = == != < > <= >= && || !
//    Delimiters: ( ) { } [ ] ; , . :
//    Special   : identifiers, EOF
// =============================================================================

use crate::error::{HanlinError, Result, Span};

// ---------------------------------------------------------------------------
//  Token Definition
// ---------------------------------------------------------------------------

/// Every distinct syntactic unit of the hanlin language.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ── Literals ──────────────────────────────────────────────────────────
    Integer(i64),
    Float(f64),
    StringLit(String),
    True,
    False,
    Null,

    // ── Identifiers ───────────────────────────────────────────────────────
    Identifier(String),

    // ── Keywords ──────────────────────────────────────────────────────────
    Let,
    Const,
    Fn,
    If,
    Else,
    Return,
    While,
    For,
    Print,
    Try,
    Catch,
    Break,
    Continue,

    // ── Arithmetic operators ───────────────────────────────────────────────
    Plus,    // +
    PlusEq,  // +=
    Minus,   // -
    MinusEq, // -=
    Star,    // *
    StarEq,  // *=
    Slash,   // /
    SlashEq, // /=
    Percent, // %

    // ── Comparison / logical operators ────────────────────────────────────
    Eq,       // =
    EqEq,     // ==
    BangEq,   // !=
    Lt,       // <
    Gt,       // >
    LtEq,     // <=
    GtEq,     // >=
    AmpAmp,   // &&
    PipePipe, // ||
    Bang,     // !

    // ── Delimiters ────────────────────────────────────────────────────────
    LParen,    // (
    RParen,    // )
    LBrace,    // {
    RBrace,    // }
    LBracket,  // [
    RBracket,  // ]
    Semicolon, // ;
    Comma,     // ,
    Dot,       // .
    Colon,     // :   ← NEW v0.2: object literal separator { key: value }

    // ── End of file ───────────────────────────────────────────────────────
    Eof,
}

/// A token with its kind and source location (line + col, 1-indexed).
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    fn new(kind: TokenKind, line: usize, col: usize) -> Self {
        Token {
            kind,
            span: Span::new(line, col),
        }
    }
}

// ---------------------------------------------------------------------------
//  Lexer
// ---------------------------------------------------------------------------

/// Reads a raw source string character-by-character and produces Vec<Token>.
pub struct Lexer {
    /// Source code as a vector of characters for easy O(1) indexing.
    chars: Vec<char>,
    /// Current read position (byte index into `chars`).
    pos: usize,
    /// Current source line (1-indexed), updated on every '\n'.
    line: usize,
    /// Current source column (1-indexed), reset to 1 on each new line.
    col: usize,
}

impl Lexer {
    /// Create a new Lexer from a source string.
    pub fn new(source: &str) -> Self {
        Lexer {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    /// Tokenize the entire input and return a `Vec<Token>`.
    /// The last token is always `TokenKind::Eof`.
    pub fn tokenize(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            if self.is_at_end() {
                tokens.push(Token::new(TokenKind::Eof, self.line, self.col));
                break;
            }
            tokens.push(self.next_token()?);
        }
        Ok(tokens)
    }

    // ── Internal helpers ───────────────────────────────────────────────────

    fn peek(&self) -> char {
        if self.is_at_end() {
            '\0'
        } else {
            self.chars[self.pos]
        }
    }

    fn peek_next(&self) -> char {
        if self.pos + 1 >= self.chars.len() {
            '\0'
        } else {
            self.chars[self.pos + 1]
        }
    }

    fn advance(&mut self) -> char {
        let ch = self.chars[self.pos];
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        ch
    }

    fn consume_if(&mut self, expected: char) -> bool {
        if !self.is_at_end() && self.peek() == expected {
            self.advance();
            true
        } else {
            false
        }
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    // ── Whitespace & comment skipping ──────────────────────────────────────

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while !self.is_at_end() && self.peek().is_ascii_whitespace() {
                self.advance();
            }
            if !self.is_at_end() && self.peek() == '/' && self.peek_next() == '/' {
                while !self.is_at_end() && self.peek() != '\n' {
                    self.advance();
                }
                continue;
            }
            if !self.is_at_end() && self.peek() == '/' && self.peek_next() == '*' {
                self.advance();
                self.advance(); // consume '/*'
                while !self.is_at_end() {
                    if self.peek() == '*' && self.peek_next() == '/' {
                        self.advance();
                        self.advance();
                        break;
                    }
                    self.advance();
                }
                continue;
            }
            break;
        }
    }

    // ── Token dispatch ─────────────────────────────────────────────────────

    fn next_token(&mut self) -> Result<Token> {
        let (start_line, start_col) = (self.line, self.col);
        let ch = self.advance();

        let kind = match ch {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            ':' => TokenKind::Colon, // ← NEW v0.2
            '+' => {
                if self.consume_if('=') {
                    TokenKind::PlusEq
                } else {
                    TokenKind::Plus
                }
            }
            '-' => {
                if self.consume_if('=') {
                    TokenKind::MinusEq
                } else {
                    TokenKind::Minus
                }
            }
            '*' => {
                if self.consume_if('=') {
                    TokenKind::StarEq
                } else {
                    TokenKind::Star
                }
            }
            '/' => {
                if self.consume_if('=') {
                    TokenKind::SlashEq
                } else {
                    TokenKind::Slash
                }
            }
            '%' => TokenKind::Percent,

            '=' => {
                if self.consume_if('=') {
                    TokenKind::EqEq
                } else {
                    TokenKind::Eq
                }
            }
            '!' => {
                if self.consume_if('=') {
                    TokenKind::BangEq
                } else {
                    TokenKind::Bang
                }
            }
            '<' => {
                if self.consume_if('=') {
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.consume_if('=') {
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            '&' => {
                if self.consume_if('&') {
                    TokenKind::AmpAmp
                } else {
                    return Err(HanlinError::lexer(
                        Span::new(start_line, start_col),
                        "expected '&&', single '&' is not supported",
                    ));
                }
            }
            '|' => {
                if self.consume_if('|') {
                    TokenKind::PipePipe
                } else {
                    return Err(HanlinError::lexer(
                        Span::new(start_line, start_col),
                        "expected '||', single '|' is not supported",
                    ));
                }
            }

            '"' => self.lex_string(start_line, start_col)?,
            c if c.is_ascii_digit() => self.lex_number(c, start_line, start_col)?,
            c if c.is_alphabetic() || c == '_' => self.lex_identifier(c),

            other => {
                return Err(HanlinError::lexer(
                    Span::new(start_line, start_col),
                    format!("unexpected character '{}'", other),
                ));
            }
        };

        Ok(Token::new(kind, start_line, start_col))
    }

    // ── Lexer sub-routines ─────────────────────────────────────────────────

    /// Lex a double-quoted string literal (opening `"` already consumed).
    fn lex_string(&mut self, line: usize, col: usize) -> Result<TokenKind> {
        let mut s = String::new();
        loop {
            if self.is_at_end() {
                return Err(HanlinError::lexer(
                    Span::new(line, col),
                    "unterminated string literal",
                ));
            }
            match self.advance() {
                '"' => break,
                '\\' => match self.advance() {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    '"' => s.push('"'),
                    '\\' => s.push('\\'),
                    c => s.push(c),
                },
                c => s.push(c),
            }
        }
        Ok(TokenKind::StringLit(s))
    }

    /// Lex an integer or float literal (first digit `first` already consumed).
    fn lex_number(&mut self, first: char, line: usize, col: usize) -> Result<TokenKind> {
        let mut s = String::from(first);
        while !self.is_at_end() && self.peek().is_ascii_digit() {
            s.push(self.advance());
        }
        if !self.is_at_end() && self.peek() == '.' && self.peek_next().is_ascii_digit() {
            s.push(self.advance()); // '.'
            while !self.is_at_end() && self.peek().is_ascii_digit() {
                s.push(self.advance());
            }
            let v: f64 = s.parse().map_err(|_| {
                HanlinError::lexer(Span::new(line, col), format!("invalid float '{}'", s))
            })?;
            return Ok(TokenKind::Float(v));
        }
        let v: i64 = s.parse().map_err(|_| {
            HanlinError::lexer(Span::new(line, col), format!("invalid integer '{}'", s))
        })?;
        Ok(TokenKind::Integer(v))
    }

    /// Lex an identifier or keyword (first char `first` already consumed).
    fn lex_identifier(&mut self, first: char) -> TokenKind {
        let mut ident = String::from(first);
        while !self.is_at_end() && (self.peek().is_alphanumeric() || self.peek() == '_') {
            ident.push(self.advance());
        }
        match ident.as_str() {
            "let" => TokenKind::Let,
            "const" => TokenKind::Const,
            "fn" => TokenKind::Fn,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "return" => TokenKind::Return,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "null" => TokenKind::Null,
            "print" => TokenKind::Print,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            _ => TokenKind::Identifier(ident),
        }
    }
}

// =============================================================================
//  Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(src: &str) -> Vec<TokenKind> {
        Lexer::new(src)
            .tokenize()
            .expect("lex failed")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn test_simple_tokens() {
        assert_eq!(
            tokenize("+ - * /"),
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_keywords() {
        assert_eq!(
            tokenize("let const fn if else return while try catch break continue"),
            vec![
                TokenKind::Let,
                TokenKind::Const,
                TokenKind::Fn,
                TokenKind::If,
                TokenKind::Else,
                TokenKind::Return,
                TokenKind::While,
                TokenKind::Try,
                TokenKind::Catch,
                TokenKind::Break,
                TokenKind::Continue,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_literals() {
        assert_eq!(
            tokenize(r#"42 3.14 "hello""#),
            vec![
                TokenKind::Integer(42),
                TokenKind::Float(3.14),
                TokenKind::StringLit("hello".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_two_char_operators() {
        assert_eq!(
            tokenize("== != <= >= && ||"),
            vec![
                TokenKind::EqEq,
                TokenKind::BangEq,
                TokenKind::LtEq,
                TokenKind::GtEq,
                TokenKind::AmpAmp,
                TokenKind::PipePipe,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_skips_line_comments() {
        assert_eq!(
            tokenize("let x // comment\n= 5;"),
            vec![
                TokenKind::Let,
                TokenKind::Identifier("x".into()),
                TokenKind::Eq,
                TokenKind::Integer(5),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_identifier() {
        assert_eq!(
            tokenize("myVar _private camelCase"),
            vec![
                TokenKind::Identifier("myVar".into()),
                TokenKind::Identifier("_private".into()),
                TokenKind::Identifier("camelCase".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_full_let_statement() {
        assert_eq!(
            tokenize("let x = 42;"),
            vec![
                TokenKind::Let,
                TokenKind::Identifier("x".into()),
                TokenKind::Eq,
                TokenKind::Integer(42),
                TokenKind::Semicolon,
                TokenKind::Eof,
            ]
        );
    }

    // ── v0.2 new tests ─────────────────────────────────────────────────────

    #[test]
    fn test_colon_token() {
        assert_eq!(
            tokenize("{ name: \"Han\" }"),
            vec![
                TokenKind::LBrace,
                TokenKind::Identifier("name".into()),
                TokenKind::Colon,
                TokenKind::StringLit("Han".into()),
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_array_tokens() {
        assert_eq!(
            tokenize("[1, 2, 3]"),
            vec![
                TokenKind::LBracket,
                TokenKind::Integer(1),
                TokenKind::Comma,
                TokenKind::Integer(2),
                TokenKind::Comma,
                TokenKind::Integer(3),
                TokenKind::RBracket,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn test_compound_assignment_tokens() {
        assert_eq!(
            tokenize("+= -= *= /="),
            vec![
                TokenKind::PlusEq,
                TokenKind::MinusEq,
                TokenKind::StarEq,
                TokenKind::SlashEq,
                TokenKind::Eof,
            ]
        );
    }
}
