// =============================================================================
//  main.rs — hanlin Compiler CLI Entry Point
// =============================================================================
//
//  Usage:
//    hanlin <source.hl>              Compile and run
//    hanlin <source.hl> --emit-c    Compile to C and print the C source
//    hanlin <source.hl> --emit-ast  Parse and print the AST (debug)
//    hanlin <source.hl> --emit-tokens  Lex and print the token stream
//
//  Full pipeline:
//    1. Read the .hl source file
//    2. Lex → Vec<Token>
//    3. Parse → AST (Program)
//    4. CodeGen → C source string
//    5. Write C to a temp file
//    6. Invoke `gcc` to compile to a native binary
//    7. Execute the binary
// =============================================================================

use hanlin::codegen;
use hanlin::interpreter;
use hanlin::lexer;
use hanlin::parser;

use std::process;

use codegen::CEmitter;
use lexer::Lexer;
use parser::Parser;

// ---------------------------------------------------------------------------
//  CLI flags
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Flags {
    emit_tokens: bool,
    emit_ast: bool,
    emit_c: bool,
    run: bool, // default: compile + run
    output: Option<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("hanlin compiler v0.2.1");
        eprintln!("Usage: hanlin <source.hl> [options]");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --emit-tokens   Print the token stream and exit");
        eprintln!("  --emit-ast      Print the AST and exit");
        eprintln!("  --emit-c        Print generated C source and exit");
        eprintln!("  -o <file>       Write binary to <file> (default: same name as source)");
        process::exit(1);
    }

    let source_path = &args[1];

    // Parse flags
    let mut flags = Flags::default();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--emit-tokens" => flags.emit_tokens = true,
            "--emit-ast" => flags.emit_ast = true,
            "--emit-c" => flags.emit_c = true,
            "-o" => {
                i += 1;
                flags.output = args.get(i).cloned();
            }
            other => {
                eprintln!("hanlin: unknown flag '{}'", other);
                process::exit(1);
            }
        }
        i += 1;
    }

    // If no special emit flag is set, compile + run by default
    flags.run = !flags.emit_tokens && !flags.emit_ast && !flags.emit_c;

    // ── Step 1: Read source file ───────────────────────────────────────────
    let source = match std::fs::read_to_string(source_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hanlin: could not read '{}': {}", source_path, e);
            process::exit(1);
        }
    };

    // ── Step 2: Lex ────────────────────────────────────────────────────────
    let tokens = match Lexer::new(&source).tokenize() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hanlin: {}", e);
            process::exit(1);
        }
    };

    if flags.emit_tokens {
        println!("=== Token Stream ===");
        for tok in &tokens {
            println!("  {:?}", tok);
        }
        return;
    }

    // ── Step 3: Parse ──────────────────────────────────────────────────────
    let program = match Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("hanlin: {}", e);
            process::exit(1);
        }
    };

    if flags.emit_ast {
        println!("=== Abstract Syntax Tree ===");
        println!("{:#?}", program);
        return;
    }

    if flags.emit_c {
        // ── Step 4: Code generation ────────────────────────────────────────────
        let c_source = match CEmitter::new().emit_program(&program) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("hanlin: {}", e);
                process::exit(1);
            }
        };
        println!("{}", c_source);
        return;
    }

    // ── Step 5: Execute with Tree-Walking Interpreter ──────────────────────
    if flags.run {
        let global_env = interpreter::Env::new();
        // Register built-in namespaces (fs, etc.) before execution
        interpreter::register_builtins(&global_env);
        let mut interp = interpreter::Interpreter::new(global_env);
        if let Err(e) = interp.interpret(&program) {
            eprintln!("hanlin: {}", e);
            process::exit(1);
        }
    }
}
