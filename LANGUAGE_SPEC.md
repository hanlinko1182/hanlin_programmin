# Hanlin Language Specification

**Specification version:** `0.2.1`
**Implementation version:** `0.2.1`
**File extension:** `.hl`
**Primary runtime:** Rust tree-walking interpreter
**Secondary backend:** Rust AST to C source emitter
**Status:** Living document

> This specification describes the behavior that the current implementation is intended to provide. If the implementation and this document disagree, add a test, resolve the behavior, and update this file and `README.md` in the same commit.

## Table of Contents

1. [Overview](#1-overview)
2. [Execution Pipeline](#2-execution-pipeline)
3. [Lexical Structure](#3-lexical-structure)
4. [Types and Values](#4-types-and-values)
5. [Literals](#5-literals)
6. [Operators](#6-operators)
7. [Statements](#7-statements)
8. [Expressions](#8-expressions)
9. [Functions and Closures](#9-functions-and-closures)
10. [Arrays](#10-arrays)
11. [Objects](#11-objects)
12. [Strings](#12-strings)
13. [Built-in APIs](#13-built-in-apis)
14. [Exception Handling](#14-exception-handling)
15. [CLI](#15-cli)
16. [C Emitter Compatibility](#16-c-emitter-compatibility)
17. [Errors and Runtime Policies](#17-errors-and-runtime-policies)
18. [Feature Matrix](#18-feature-matrix)
19. [Grammar](#19-grammar)
20. [Roadmap](#20-roadmap)

## 1. Overview

Hanlin is a dynamically typed, JavaScript-inspired scripting language implemented in Rust. A Hanlin source file uses the `.hl` extension. The interpreter is the primary execution path; the C emitter is a restricted secondary backend.

Hanlin does not currently provide static type checking, a module system, a REPL, or complete feature parity between the interpreter and the C emitter.

## 2. Execution Pipeline

### Interpreter mode

```text
source.hl → Lexer → Vec<Token> → Parser → AST → Interpreter → stdout/stderr
```

### C-emitter mode

```text
source.hl → Lexer → Parser → CEmitter → C source on stdout
```

The `--emit-c` command emits C source. It does not automatically invoke GCC or create a native binary.

## 3. Lexical Structure

### 3.1 Whitespace and statements

Spaces, tabs, and newlines separate tokens and are otherwise ignored. Statements are terminated with semicolons unless a future grammar revision explicitly changes this rule.

### 3.2 Comments

```js
// Single-line comment

/* Multi-line
   comment */
```

### 3.3 Identifiers

Identifiers begin with an ASCII letter or underscore and may be followed by ASCII letters, digits, or underscores.

```text
identifier := [a-zA-Z_][a-zA-Z0-9_]*
```

### 3.4 Keywords

| Keyword | Meaning |
|---|---|
| `let` | Mutable variable declaration |
| `const` | Immutable variable declaration |
| `fn` | Function declaration |
| `return` | Return from a function |
| `if` | Conditional branch |
| `else` | Alternate branch; may be followed by `if` |
| `while` | While loop |
| `for` | C-style loop |
| `break` | Exit the nearest loop |
| `continue` | Continue the nearest loop |
| `true` | Boolean literal |
| `false` | Boolean literal |
| `null` | Null literal |
| `print` | Built-in print statement |
| `try` | Exception-handling block |
| `catch` | Exception handler |

## 4. Types and Values

Hanlin is dynamically typed. Values carry their runtime type and may be stored in mixed arrays or object properties.

| Type | Meaning | Example |
|---|---|---|
| `Integer` | Signed 64-bit integer | `42` |
| `Float` | 64-bit floating-point value | `3.14` |
| `String` | UTF-8 text | `"hello"` |
| `Bool` | Boolean value | `true` |
| `Array` | Mutable ordered collection | `[1, 2, 3]` |
| `Object` | Mutable string-keyed map | `{ name: "Han" }` |
| `Function` | User-defined first-class function | `fn add(a,b){...}` |
| `NativeFunction` | Rust function exposed as a built-in | `fs.readFile` |
| `Null` | Absence of a value | `null` |

### Truthiness

| Value | Boolean context |
|---|---|
| `null` | False |
| `false` | False |
| `0` | False |
| `0.0` | False |
| `""` | False |
| `NaN` | False, if produced by a supported operation |
| Arrays, objects, functions | True, including empty values |
| All other values | True |

## 5. Literals

```js
let integer = 42;
let negative = -7;
let decimal = 3.14159;
let text = "Hello, World!";
let yes = true;
let no = false;
let nothing = null;
let values = [1, "two", true];
let user = { name: "Han", age: 20 };
```

Double-quoted strings support the escape sequences implemented by the lexer, including `\n`, `\t`, `\r`, `\"`, and `\\`. Multi-line string literals and string interpolation are not currently part of the language.

## 6. Operators

### 6.1 Supported operators

| Precedence | Operators | Associativity |
|---:|---|---|
| 1 | Unary `-`, `!` | Right |
| 2 | `*`, `/`, `%` | Left |
| 3 | `+`, `-` | Left |
| 4 | `<`, `>`, `<=`, `>=` | Left |
| 5 | `==`, `!=` | Left |
| 6 | `&&` | Left |
| 7 | `\|\|` | Left |
| 8 | `=`, `+=`, `-=`, `*=`, `/=` | Right |

### 6.2 Arithmetic and comparison

Arithmetic supports integer and floating-point values according to the implementation's numeric promotion rules. Division returns a floating-point result when supported by the current runtime. The `+` operator also supports string concatenation when at least one operand is a string.

```js
let sum = 1 + 2;
let quotient = 10 / 4;
let remainder = 7 % 3;
let message = "value: " + 42;
let equal = 3 == 3.0;
```

### 6.3 Logical operators

`&&` and `||` short-circuit. They return the selected operand value rather than necessarily returning a Boolean. `!` always returns a Boolean.

### 6.4 Assignment

```js
x = expression;
arr[index] = expression;
obj.property = expression;
obj["key"] = expression;
```

Compound assignment operators `+=`, `-=`, `*=`, and `/=` are supported for variables, array elements, and object properties.

## 7. Statements

### 7.1 Variable declarations

```js
let name = "Han";
let uninitialized;
const PI = 3.14159;
```

An uninitialized variable receives `null`. Reassigning a `const` variable is a runtime error.

### 7.2 Conditions

```js
if (condition) {
    // then branch
} else if (other_condition) {
    // else-if branch
} else {
    // fallback branch
}
```

The parser may lower `else if` into a nested `if` statement in the AST. This is an implementation detail; the syntax is supported at the language level.

### 7.3 While loops

```js
while (condition) {
    // body
}
```

### 7.4 For loops

```js
for (let i = 0; i < 10; i = i + 1) {
    print(i);
}
```

The initializer, condition, and update clauses may be omitted. An omitted condition behaves as `true`.

### 7.5 Break and continue

```js
for (let i = 0; i < 10; i = i + 1) {
    if (i == 3) { continue; }
    if (i == 8) { break; }
}
```

Using `break` or `continue` outside a loop is a runtime error.

### 7.6 Print

```js
print();
print(value);
print(name, age, active);
```

Arguments are converted to their display representation, separated by spaces, and followed by a newline.

## 8. Expressions

### Function calls

```js
let result = add(3, 4);
```

The number of arguments must match the number of declared parameters unless the implementation explicitly adds variadic functions.

### Index access

```js
arr[0];
text[1];
obj["name"];
```

Indexing is zero-based. Invalid indexes must produce a Hanlin runtime error rather than an unchecked Rust panic.

### Member access and method calls

```js
obj.name;
arr.length;
arr.push(4);
text.trim();
```

## 9. Functions and Closures

```js
fn add(a, b) {
    return a + b;
}

fn make_counter() {
    let count = 0;
    fn increment() {
        count = count + 1;
        return count;
    }
    return increment;
}
```

Functions are first-class values. They may be assigned to variables, passed as arguments, returned from functions, and recursively called. Inner functions capture variables from their enclosing environment.

A function with no explicit return value returns `null`. A bare `return;` also returns `null`.

## 10. Arrays

Arrays are mutable and may contain mixed value types.

```js
let arr = [10, 20, 30];
print(arr[0]);
print(arr.length);
arr[1] = 99;
arr.push(40);
let last = arr.pop();
print(arr.join(", "));
```

| Operation | Syntax | Current behavior |
|---|---|---|
| Create | `[a, b]` | Supported |
| Read | `arr[i]` | Zero-based indexing |
| Write | `arr[i] = value` | Supported |
| Length | `arr.length` | Supported |
| Append | `arr.push(value)` | Supported |
| Remove last | `arr.pop()` | Returns the removed value, or `null` when empty |
| Join | `arr.join(delimiter?)` | Supported; default delimiter is `,` |
| Search/slice | `indexOf`, `slice` | Not currently supported |

## 11. Objects

Objects are mutable maps with string keys. Object literal keys are written as identifiers.

```js
let user = { name: "Han", age: 20 };
print(user.name);
print(user["age"]);
user.age = 21;
user["role"] = "developer";
```

Missing-key behavior must remain consistent between dot access and bracket access and must be covered by tests. Nested objects are supported.

## 12. Strings

Strings are immutable. Methods return new values.

| Property or method | Example | Result |
|---|---|---|
| Length | `text.length` | Integer character count |
| Index | `text[0]` | One-character string |
| Split | `text.split(",")` | Array of strings |
| Trim | `text.trim()` | Trimmed string |
| Uppercase | `text.toUpperCase()` | Uppercase string |
| Lowercase | `text.toLowerCase()` | Lowercase string |
| Contains | `text.contains("x")` | Not currently supported |
| Replace | `text.replace("a", "b")` | Not currently supported |

The implementation should define whether `.length` and indexing operate on Unicode scalar values or bytes. Add tests for non-ASCII text before declaring the behavior stable.

## 13. Built-in APIs

### 13.1 `print`

`print(...)` writes display-formatted values to standard output, separated by spaces, followed by a newline.

### 13.2 `math`

| Function | Example | Purpose |
|---|---|---|
| `math.abs(x)` | `math.abs(-5)` | Absolute value |
| `math.sqrt(x)` | `math.sqrt(16.0)` | Square root |
| `math.pow(x, y)` | `math.pow(2.0, 10.0)` | Power |

### 13.3 `fs`

| Function | Signature | Behavior |
|---|---|---|
| `fs.readFile` | `fs.readFile(path)` | Reads a file as a string or raises a runtime error |
| `fs.writeFile` | `fs.writeFile(path, content)` | Writes content and returns `null` |
| `fs.exists` | `fs.exists(path)` | Returns whether the path exists |

Filesystem access is a security-sensitive capability. Document the expected working directory, path rules, and behavior for permission errors before using Hanlin with untrusted programs.

### 13.4 Conversions

```js
let i = int("42");
let f = float("3.14");
let s = str(100);
```

Conversion failures must produce a catchable runtime error with a source span.

## 14. Exception Handling

```js
try {
    let content = fs.readFile("missing.txt");
    print(content);
} catch (err) {
    print("Error:", err);
}
```

Runtime errors raised in the `try` body are caught and made available to the catch body through the catch variable. Errors raised inside the catch body propagate normally. The exact catch value representation must remain documented and tested.

## 15. CLI

```text
hanlin <source.hl>
hanlin <source.hl> --emit-tokens
hanlin <source.hl> --emit-ast
hanlin <source.hl> --emit-c
hanlin <source.hl> -o <output>
```

| Command | Result |
|---|---|
| `hanlin file.hl` | Interpret and run the source file |
| `--emit-tokens` | Print token stream and exit |
| `--emit-ast` | Parse and print AST and exit |
| `--emit-c` | Print generated C source and exit |
| `-o <file>` | Reserved/output-name behavior must be documented precisely |

Recommended future flags are `--help` and `--version`. Missing values after `-o` must produce a clear command-line error.

## 16. C Emitter Compatibility

The C emitter is not a complete replacement for the interpreter. Unsupported features should return a clear `CodeGenError` and should never silently generate incorrect C code.

| Feature | Interpreter | C emitter |
|---|---:|---:|
| Integer/float arithmetic | Yes | Yes |
| Basic conditions and loops | Yes | Supported subset |
| User functions | Yes | Supported subset |
| Arrays | Yes | No |
| Objects | Yes | No |
| Method calls | Yes | No |
| Closures | Yes | No |
| First-class functions | Yes | No |
| `try-catch` | Yes | No |
| Dynamic filesystem APIs | Yes | No |
| String concatenation | Yes | Verify supported subset; reject unsupported forms |

## 17. Errors and Runtime Policies

Every user-facing error should identify its phase where possible, such as lexer, parser, runtime, or code generation, and include a source location when available.

The implementation must define and test the following policies:

| Situation | Current policy |
|---|---|
| Division by zero | Runtime error; never panic |
| Modulo by zero | Runtime error; never panic |
| Integer overflow | Use checked arithmetic or document wrapping behavior |
| Deep recursion | Add a recursion-depth limit or a safe failure mode |
| Invalid array/string index | Runtime error with index and span |
| Empty-array `pop` | Returns `null` |
| Missing object property | Choose and document `null` or runtime error |
| File permission/path failure | Produce a catchable runtime error |
| Invalid conversion | Produce a catchable runtime error |

## 18. Feature Matrix

| Feature | Status |
|---|---|
| Integer, Float, String, Bool, Null | Implemented |
| Arrays and nested arrays | Implemented |
| Objects and nested objects | Implemented |
| `let` and `const` | Implemented |
| Arithmetic/comparison/logical operators | Implemented |
| `if`, `else`, native `else if` | Implemented |
| `while` and C-style `for` | Implemented |
| `break` and `continue` | Implemented |
| Functions, closures, recursion | Implemented |
| `try-catch` | Implemented |
| `print` | Implemented |
| Array `push`, `pop`, `join`, `length` | Implemented; verify edge-case policy |
| String `split`, `trim`, case conversion | Implemented |
| `int`, `float`, `str` | Implemented |
| `math.abs`, `math.sqrt`, `math.pow` | Implemented |
| `fs.readFile`, `fs.writeFile`, `fs.exists` | Implemented |
| Compound assignment | Implemented |
| String interpolation | Planned |
| `typeof()` | Planned |
| `contains`, `replace`, `indexOf`, `slice` | Planned |
| REPL | Planned |
| Modules/imports | Planned |
| LSP | Long term |
| Cranelift/LLVM backend | Long term |

## 19. Grammar

The following is a high-level grammar. The parser implementation is authoritative for details not represented here.

```text
program       := statement* EOF ;
statement     := fn_decl
               | var_decl
               | return_stmt
               | if_stmt
               | while_stmt
               | for_stmt
               | print_stmt
               | try_stmt
               | break_stmt
               | continue_stmt
               | expr_stmt ;

fn_decl       := "fn" identifier "(" parameters? ")" block ;
var_decl      := ("let" | "const") identifier ("=" expression)? ";" ;
return_stmt   := "return" expression? ";" ;
if_stmt       := "if" "(" expression ")" block ("else" ("if" "(" expression ")" block | block))? ;
while_stmt    := "while" "(" expression ")" block ;
for_stmt      := "for" "(" initializer? ";" condition? ";" update? ")" block ;
try_stmt      := "try" block "catch" "(" identifier ")" block ;
print_stmt    := "print" "(" arguments? ")" ";" ;
break_stmt    := "break" ";" ;
continue_stmt := "continue" ";" ;
expr_stmt     := expression ";" ;
block         := "{" statement* "}" ;
expression    := assignment ;
assignment    := postfix assignment_op assignment | logical_or ;
assignment_op := "=" | "+=" | "-=" | "*=" | "/=" ;
logical_or    := logical_and ("||" logical_and)* ;
logical_and   := equality ("&&" equality)* ;
equality      := comparison (("==" | "!=") comparison)* ;
comparison    := term (("<" | ">" | "<=" | ">=") term)* ;
term          := factor (("+" | "-") factor)* ;
factor        := unary (("*" | "/" | "%") unary)* ;
unary         := ("-" | "!") unary | postfix ;
postfix       := primary ("[" expression "]"
               | "." identifier ("(" arguments? ")")?
               | "(" arguments? ")")* ;
primary       := integer | float | string | "true" | "false" | "null"
               | array | object | identifier | "(" expression ")" ;
```

## 20. Roadmap

### Next release

- [x] Runtime checks for division and modulo by zero.
- [ ] Recursion-depth protection.
- [ ] Explicit integer-overflow policy.
- [x] Compound assignment operators.
- [ ] CLI `--help` and `--version`.
- [ ] Accurate, automated test-count reporting.
- [ ] Regression tests for all runtime edge cases.

### Developer experience

- [ ] REPL mode.
- [ ] Additional string and array methods.
- [ ] `typeof()`.
- [ ] String interpolation.
- [ ] Split the interpreter into focused modules.

### Long term

- [ ] Module/import system.
- [ ] LSP language server.
- [ ] Cranelift or LLVM native backend.

## Document Maintenance

When changing syntax or runtime behavior, update all of the following in one pull request:

1. The lexer/parser/interpreter or code-generation implementation.
2. Relevant unit, integration, and regression tests.
3. `README.md`.
4. This `LANGUAGE_SPEC.md`.
5. The changelog and version metadata when the change is user-visible.

The documentation must not list an implemented feature as planned, and it must not claim C-emitter support for features that are interpreter-only.
