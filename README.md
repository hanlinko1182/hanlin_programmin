# hanlin

A dynamically typed, JavaScript-inspired scripting language implemented in Rust.

> **Documentation status:** This README describes the current implementation. When a feature is changed, update this file and `LANGUAGE_SPEC.md` in the same commit.

| Item | Current value |
|---|---|
| Version | `0.2.1` <!-- Keep synchronized with Cargo.toml --> |
| File extension | `.hl` |
| Primary execution | Tree-walking interpreter |
| Secondary execution | Rust AST to C source emitter via `--emit-c` |
| Language style | Dynamically typed, JavaScript-inspired |
| License | MIT |

## Quick Start

### Prerequisites

- Rust stable and Cargo
- GCC is required only when compiling generated C code

### Build

```bash
cargo build --release
```

### Run a Hanlin program

```bash
./target/release/hanlin examples/hello.hl
```

The default mode lexes, parses, and executes the program with the tree-walking interpreter.

### Debug and code-generation modes

```bash
# Print lexer tokens
./target/release/hanlin examples/hello.hl --emit-tokens

# Print the parsed AST
./target/release/hanlin examples/hello.hl --emit-ast

# Print generated C source
./target/release/hanlin examples/hello.hl --emit-c > output.c

# Compile generated C manually
gcc output.c -o output -lm
./output
```

> **Important:** `--emit-c` prints C source to stdout. It does not automatically invoke GCC or create a binary.

## Language Features

| Area | Supported features |
|---|---|
| Values | Integer, Float, String, Bool, Array, Object, Function, Null |
| Variables | `let`, `const`, assignment, lexical environments |
| Operators | Arithmetic, comparison, logical short-circuiting, assignment, compound assignment |
| Control flow | `if`, native `else if`, `while`, C-style `for`, `break`, `continue` |
| Functions | First-class functions, closures, recursion, return values |
| Arrays | Literals, indexing, mutation, `push`, `pop`, `join`, `length` |
| Objects | Literals, dot access, bracket access, property mutation |
| Strings | Indexing, `length`, `split`, `trim`, `toUpperCase`, `toLowerCase` |
| Conversion | `int`, `float`, `str` |
| Built-ins | `print`, `math.abs`, `math.sqrt`, `math.pow`, `fs.readFile`, `fs.writeFile`, `fs.exists` |
| Errors | Lexer, parser, runtime, and code-generation errors; `try-catch` for runtime errors |
| Comments | `//` single-line comments and `/* ... */` multi-line comments |

## Basic Examples

### Variables and arithmetic

```js
let x = 42;
const PI = 3.14159;
let result = (x * 2) + PI;
print("Result:", result);
```

### Conditions and loops

```js
let score = 85;

if (score >= 90) {
    print("A grade");
} else if (score >= 80) {
    print("B grade");
} else {
    print("C grade");
}

for (let i = 0; i < 5; i = i + 1) {
    if (i == 3) { continue; }
    print(i);
}
```

### Functions and closures

```js
fn make_counter() {
    let count = 0;
    fn increment() {
        count = count + 1;
        return count;
    }
    return increment;
}

let counter = make_counter();
print(counter());
print(counter());
```

### Arrays and objects

```js
let numbers = [10, 20, 30];
numbers.push(40);
numbers[1] = 99;
print(numbers.length);

let user = { name: "Han", age: 20 };
user.age = 21;
user["role"] = "developer";
print(user.name, user.age, user.role);
```

### File I/O and error handling

```js
try {
    let content = fs.readFile("data.txt");
    print(content.trim());
} catch (err) {
    print("Read error:", err);
}
```

## Interpreter and C Codegen Compatibility

The interpreter is the primary and most complete execution path. The C emitter is a secondary backend for a restricted subset of the language.

| Feature | Interpreter | C emitter |
|---|---:|---:|
| Arithmetic and comparisons | Yes | Yes |
| Conditions and loops | Yes | Limited to supported subset |
| User functions | Yes | Limited |
| Arrays and array methods | Yes | No |
| Objects and member access | Yes | No |
| Closures and first-class functions | Yes | No |
| `try-catch` | Yes | No |
| Dynamic method calls | Yes | No |
| General string concatenation | Yes | Verify before use; unsupported cases should report `CodeGenError` |

Programs using unsupported C-emitter features should be run in interpreter mode.

## Project Structure

```text
src/
├── main.rs          # CLI entry point and pipeline orchestration
├── lib.rs           # Module exports
├── lexer.rs         # Source text to tokens
├── parser.rs        # Tokens to AST
├── ast.rs           # AST definitions
├── interpreter.rs   # Tree-walking interpreter and built-ins
├── codegen.rs       # AST to C source emitter
└── error.rs         # Error types and source spans

examples/            # Demonstration programs
tests/               # Integration and malformed-input tests
.github/workflows/   # Continuous integration configuration
```

## Testing

Run the standard Rust test suite:

```bash
cargo test
```

Run Clippy:

```bash
cargo clippy -- -D warnings
```

If an integration script is present in the repository, run it with:

```bash
bash scripts/test_integration.sh
```

> Keep the test numbers in this README synchronized with the actual CI output. Do not hard-code a count unless it is verified for the current commit.

Recommended regression cases include modulo by zero, division by zero, deep recursion, integer overflow, empty-array `pop`, nested closures, invalid indexes, filesystem errors, unsupported C codegen features, and malformed input.

## Known Limitations

The following features are not currently part of the language or require a documented policy:

- String interpolation.
- `typeof()`.
- `str.contains()` and `str.replace()`.
- `arr.indexOf()` and `arr.slice()`.
- Module or import support.
- REPL mode.
- Recursion-depth protection and a documented integer-overflow policy.
- Native code generation beyond the current C-emitter subset.

Runtime behavior for division by zero, modulo by zero, integer overflow, deep recursion, and `arr.pop()` on an empty array must be documented and covered by tests.

## Roadmap

### Next release

- [ ] Add runtime checks for division and modulo by zero.
- [ ] Add recursion-depth protection.
- [ ] Decide and document integer-overflow behavior.
- [x] Add compound assignment operators.
- [ ] Add `--help` and `--version` CLI flags.
- [ ] Validate missing values after `-o`.
- [ ] Add regression tests for all runtime edge cases.

### Developer experience

- [ ] Add REPL mode.
- [ ] Add `str.contains()` and `str.replace()`.
- [ ] Add `arr.indexOf()` and `arr.slice()`.
- [ ] Add `typeof()`.
- [ ] Add string interpolation.
- [ ] Split the large interpreter module into focused submodules.

### Long term

- [ ] Add module/import support.
- [ ] Add LSP support.
- [ ] Add a Cranelift or LLVM native backend.

## Continuous Integration and Releases

Every push and pull request targeting `main` runs the GitHub Actions CI workflow. It checks formatting, Clippy warnings, all Rust tests, release compilation, CLI debug modes, and compilation of generated C code. The workflow can also be started manually with `workflow_dispatch`.

Pushing a version tag such as `v0.2.1` starts the release workflow. It builds the optimized Linux binary, creates a compressed archive, generates a SHA-256 checksum file, and publishes both files to a GitHub Release. Release tags should match the version in `Cargo.toml`.

```bash
git tag v0.2.1
git push origin v0.2.1
```

## Contributing

Before opening a pull request:

```bash
cargo fmt -- --check
cargo test
cargo clippy -- -D warnings
```

Update both `README.md` and `LANGUAGE_SPEC.md` when language syntax, runtime behavior, built-ins, CLI behavior, or feature status changes. Add a regression test for every bug fix.

## License

This project is licensed under the MIT License.

## Related Documentation

- [`LANGUAGE_SPEC.md`](LANGUAGE_SPEC.md)
- [`code_review_report.md`](code_review_report.md)
