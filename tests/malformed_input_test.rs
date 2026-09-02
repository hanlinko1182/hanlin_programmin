use hanlin::lexer::Lexer;
use hanlin::parser::Parser;

#[test]
fn test_lexer_fuzz_resilience() {
    // A string with weird characters, unclosed strings, etc.
    let bad_input = "let x = @#$%; \"unclosed string \n let y = 2;";

    // Lexer should either parse it or return a HanlinError, but it MUST NOT panic.
    let mut lexer = Lexer::new(bad_input);
    let _ = lexer.tokenize();
}

#[test]
fn test_parser_fuzz_resilience() {
    let inputs = vec![
        "let x =",
        "if () { }",
        "print(;",
        "1 + * 2",
        "let x = [1, 2, ;",
        "for (let i = 0 i < 10; i = i + 1) {}",
    ];

    for input in inputs {
        let mut lexer = Lexer::new(input);
        if let Ok(tokens) = lexer.tokenize() {
            let mut parser = Parser::new(tokens);
            // Parser should return an error, but MUST NOT panic
            let _ = parser.parse();
        }
    }
}
