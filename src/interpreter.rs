// =============================================================================
//  interpreter.rs — Tree-Walking Interpreter for hanlin
// =============================================================================

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::ast::{BinOp, Expr, Literal, Program, Stmt, UnOp};
use crate::error::{HanlinError, Result, Span};

// ---------------------------------------------------------------------------
//  Value definition
// ---------------------------------------------------------------------------

/// A native Rust function callable from hanlin code.
/// The closure receives evaluated argument values and returns a Value or runtime error.
pub type NativeFnPtr = fn(Vec<Value>, Span) -> Result<Value>;

#[derive(Clone)]
pub enum Value {
    Integer(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Array(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<HashMap<String, Value>>>),
    Fn {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
        env: Env,
    },
    /// A built-in Rust function exposed to hanlin code (e.g. fs.readFile).
    NativeFn {
        name: String,
        func: NativeFnPtr,
    },
    Null,
}

impl Value {
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Integer(i) => *i != 0,
            Value::Float(f) => *f != 0.0 && !f.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::Array(_) => true,
            Value::Object(_) => true,
            Value::Fn { .. } => true,
            Value::NativeFn { .. } => true,
        }
    }

    pub fn to_string_display(&self) -> String {
        match self {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Str(s) => s.clone(),
            Value::Array(arr) => {
                let elements = arr.borrow();
                let parts: Vec<String> = elements.iter().map(|v| v.to_string_repr()).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Object(obj) => {
                let map = obj.borrow();
                let mut parts: Vec<String> = map
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_string_repr()))
                    .collect();
                parts.sort();
                format!("{{ {} }}", parts.join(", "))
            }
            Value::Fn { name, .. } => format!("[Function: {}]", name),
            Value::NativeFn { name, .. } => format!("[NativeFunction: {}]", name),
        }
    }

    pub fn to_string_repr(&self) -> String {
        match self {
            Value::Str(s) => format!("\"{}\"", s),
            _ => self.to_string_display(),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_repr())
    }
}

// ---------------------------------------------------------------------------
//  Environment scope management
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Env(Rc<RefCell<EnvInner>>);

#[derive(Debug)]
pub struct EnvInner {
    parent: Option<Env>,
    bindings: HashMap<String, (Value, bool)>, // (value, is_const)
}

#[allow(clippy::new_without_default)]
impl Env {
    pub fn new() -> Self {
        Env(Rc::new(RefCell::new(EnvInner {
            parent: None,
            bindings: HashMap::new(),
        })))
    }

    pub fn new_child(parent: &Env) -> Self {
        Env(Rc::new(RefCell::new(EnvInner {
            parent: Some(parent.clone()),
            bindings: HashMap::new(),
        })))
    }

    pub fn define(&self, name: String, value: Value, is_const: bool) {
        self.0.borrow_mut().bindings.insert(name, (value, is_const));
    }

    pub fn assign(&self, name: &str, value: Value, span: Span) -> Result<()> {
        let mut inner = self.0.borrow_mut();
        if let Some((existing_val, is_const)) = inner.bindings.get_mut(name) {
            if *is_const {
                return Err(HanlinError::runtime(
                    Some(span),
                    format!("Assignment to constant variable '{}'", name),
                ));
            }
            *existing_val = value;
            return Ok(());
        }
        if let Some(ref parent) = inner.parent {
            parent.assign(name, value, span)
        } else {
            Err(HanlinError::runtime(
                Some(span),
                format!("Undefined variable '{}' in assignment", name),
            ))
        }
    }

    pub fn get(&self, name: &str, span: Span) -> Result<Value> {
        let inner = self.0.borrow();
        if let Some((value, _)) = inner.bindings.get(name) {
            return Ok(value.clone());
        }
        if let Some(ref parent) = inner.parent {
            parent.get(name, span)
        } else {
            Err(HanlinError::runtime(
                Some(span),
                format!("Undefined variable '{}'", name),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
//  Interpreter implementation
// ---------------------------------------------------------------------------

pub enum Signal {
    None,
    Return(Value),
    /// Produced by a `break;` statement — consumed by the nearest enclosing loop.
    Break,
    /// Produced by a `continue;` statement — consumed by the nearest enclosing loop.
    Continue,
}

pub struct Interpreter {
    env: Env,
    last_span: Span,
    /// Tracks the nesting depth of active loops (while / for).
    /// `break` and `continue` are only legal when `loop_depth > 0`.
    loop_depth: usize,
}

impl Interpreter {
    pub fn new(env: Env) -> Self {
        Interpreter {
            env,
            last_span: Span::new(1, 1),
            loop_depth: 0,
        }
    }

    pub fn interpret(&mut self, program: &Program) -> Result<()> {
        for stmt in &program.body {
            if let Signal::Return(_) = self.eval_stmt(stmt)? {
                // Top-level returns are ignored or can exit, typical of JS scripts.
                break;
            }
        }
        Ok(())
    }

    // ── Statement Evaluation ───────────────────────────────────────────────

    pub fn eval_stmt(&mut self, stmt: &Stmt) -> Result<Signal> {
        match stmt {
            Stmt::VarDecl {
                name,
                is_const,
                init,
                span,
            } => {
                self.last_span = *span;
                let val = match init {
                    Some(expr) => self.eval_expr(expr)?,
                    None => Value::Null,
                };
                self.env.define(name.clone(), val, *is_const);
            }
            Stmt::FnDecl {
                name,
                params,
                body,
                span,
            } => {
                self.last_span = *span;
                // Capture the current environment to support closures
                let func = Value::Fn {
                    name: name.clone(),
                    params: params.clone(),
                    body: body.clone(),
                    env: self.env.clone(),
                };
                self.env.define(name.clone(), func, false);
            }
            Stmt::Return { value, span } => {
                self.last_span = *span;
                let val = match value {
                    Some(expr) => self.eval_expr(expr)?,
                    None => Value::Null,
                };
                return Ok(Signal::Return(val));
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                span,
            } => {
                self.last_span = *span;
                let cond_val = self.eval_expr(condition)?;
                if cond_val.is_truthy() {
                    let sig = self.eval_statements(then_body)?;
                    if matches!(sig, Signal::Return(_) | Signal::Break | Signal::Continue) {
                        return Ok(sig);
                    }
                } else if let Some(ref else_stmts) = else_body {
                    let sig = self.eval_statements(else_stmts)?;
                    if matches!(sig, Signal::Return(_) | Signal::Break | Signal::Continue) {
                        return Ok(sig);
                    }
                }
            }
            Stmt::While {
                condition,
                body,
                span,
            } => {
                self.last_span = *span;
                self.loop_depth += 1;
                'while_loop: loop {
                    if !self.eval_expr(condition)?.is_truthy() {
                        break 'while_loop;
                    }
                    match self.eval_statements(body)? {
                        Signal::Return(v) => {
                            self.loop_depth -= 1;
                            return Ok(Signal::Return(v));
                        }
                        Signal::Break => break 'while_loop,
                        Signal::Continue => continue 'while_loop,
                        Signal::None => {}
                    }
                }
                self.loop_depth -= 1;
            }
            Stmt::Print { args, span } => {
                self.last_span = *span;
                let mut results = Vec::new();
                for arg in args {
                    results.push(self.eval_expr(arg)?.to_string_display());
                }
                println!("{}", results.join(" "));
            }
            Stmt::Expression { expr, span } => {
                self.last_span = *span;
                self.eval_expr(expr)?;
            }

            // ── try-catch ─────────────────────────────────────────────────────
            //
            // Evaluate try_body inside a fresh child scope.  If any statement
            // in it returns an Err(HanlinError), catch it, bind the error
            // message string to `catch_var` in a new child scope, then run
            // catch_body. Unlike real propagation, errors inside catch_body
            // do propagate normally (i.e. they are NOT caught again).
            Stmt::TryCatch {
                try_body,
                catch_var,
                catch_body,
                span,
            } => {
                self.last_span = *span;

                // Run the try block; collect result without propagating error
                let try_result = self.eval_try_body(try_body);

                match try_result {
                    Ok(Signal::Return(v)) => return Ok(Signal::Return(v)),
                    Ok(Signal::None) => { /* try succeeded, nothing to catch */ }
                    // Break/Continue cannot escape a try block without a loop
                    // (loop_depth guards them). Treat them as non-errors here.
                    Ok(Signal::Break | Signal::Continue) => {}
                    Err(err) => {
                        // Bind the error message to catch_var in a child scope
                        let previous_env = self.env.clone();
                        self.env = Env::new_child(&previous_env);
                        self.env
                            .define(catch_var.clone(), Value::Str(err.message.clone()), false);

                        for stmt in catch_body {
                            let sig = self.eval_stmt(stmt);
                            if sig.is_err() || matches!(sig, Ok(Signal::Return(_))) {
                                self.env = previous_env;
                                return sig;
                            }
                        }

                        self.env = previous_env;
                    }
                }
            }

            // ── for loop ──────────────────────────────────────────────────────────────
            //
            //  Execution order (C-standard):
            //    1. init  (once, before any iteration)
            //    2. condition  (before each iteration; absent ⇒ treated as true)
            //    3. body
            //    4. update  (after body AND after 'continue', before next condition)
            //    5. repeat from 2
            Stmt::For {
                init,
                condition,
                update,
                body,
                span,
            } => {
                self.last_span = *span;

                // The init clause may declare a variable that should be scoped
                // to the for loop (not visible outside it).
                let prev_env = self.env.clone();
                self.env = Env::new_child(&prev_env);

                // Execute init once
                if let Some(init_stmt) = init {
                    self.eval_stmt(init_stmt)?;
                }

                self.loop_depth += 1;
                let loop_result = 'for_loop: loop {
                    // Evaluate condition; absent ⇒ always true
                    if let Some(cond_expr) = condition {
                        if !self.eval_expr(cond_expr)?.is_truthy() {
                            break 'for_loop Signal::None;
                        }
                    }

                    // Execute body
                    match self.eval_statements(body)? {
                        Signal::Return(v) => break 'for_loop Signal::Return(v),
                        Signal::Break => break 'for_loop Signal::None,
                        Signal::Continue => {
                            // Run update before going back to condition check
                            if let Some(upd) = update {
                                self.eval_expr(upd)?;
                            }
                            continue 'for_loop;
                        }
                        Signal::None => {}
                    }

                    // Execute update at end of normal body execution
                    if let Some(upd) = update {
                        self.eval_expr(upd)?;
                    }
                };
                self.loop_depth -= 1;

                // Restore environment (for-init variables go out of scope)
                self.env = prev_env;

                // Propagate Return upward; Break is consumed here
                if let Signal::Return(v) = loop_result {
                    return Ok(Signal::Return(v));
                }
            }

            // ── break ─────────────────────────────────────────────────────────────────
            Stmt::Break { span } => {
                self.last_span = *span;
                if self.loop_depth == 0 {
                    return Err(HanlinError::runtime(
                        Some(*span),
                        "'break' used outside of a loop",
                    ));
                }
                return Ok(Signal::Break);
            }

            // ── continue ─────────────────────────────────────────────────────────────
            Stmt::Continue { span } => {
                self.last_span = *span;
                if self.loop_depth == 0 {
                    return Err(HanlinError::runtime(
                        Some(*span),
                        "'continue' used outside of a loop",
                    ));
                }
                return Ok(Signal::Continue);
            }
        }
        Ok(Signal::None)
    }

    /// Run a list of statements as a try-block — returns the first Err
    /// or the Signal from the block without panicking the interpreter.
    fn eval_try_body(&mut self, stmts: &[Stmt]) -> Result<Signal> {
        let previous_env = self.env.clone();
        self.env = Env::new_child(&previous_env);

        let mut result = Ok(Signal::None);
        for stmt in stmts {
            let sig = self.eval_stmt(stmt);
            match sig {
                Err(e) => {
                    result = Err(e);
                    break;
                }
                Ok(Signal::Return(v)) => {
                    result = Ok(Signal::Return(v));
                    break;
                }
                Ok(Signal::Break) => {
                    result = Ok(Signal::Break);
                    break;
                }
                Ok(Signal::Continue) => {
                    result = Ok(Signal::Continue);
                    break;
                }
                Ok(Signal::None) => {}
            }
        }

        self.env = previous_env;
        result
    }

    pub fn eval_statements(&mut self, stmts: &[Stmt]) -> Result<Signal> {
        // Blocks have their own lexical scope
        let previous_env = self.env.clone();
        self.env = Env::new_child(&previous_env);

        for stmt in stmts {
            let sig = self.eval_stmt(stmt);
            // Propagate errors, Return, Break, and Continue upward
            if sig.is_err()
                || matches!(
                    sig,
                    Ok(Signal::Return(_) | Signal::Break | Signal::Continue)
                )
            {
                self.env = previous_env; // restore env before returning
                return sig;
            }
        }

        self.env = previous_env;
        Ok(Signal::None)
    }

    // ── Expression Evaluation ──────────────────────────────────────────────

    pub fn eval_expr(&mut self, expr: &Expr) -> Result<Value> {
        match expr {
            Expr::Literal(lit) => match lit {
                Literal::Integer(i) => Ok(Value::Integer(*i)),
                Literal::Float(f) => Ok(Value::Float(*f)),
                Literal::Str(s) => Ok(Value::Str(s.clone())),
                Literal::Bool(b) => Ok(Value::Bool(*b)),
                Literal::Null => Ok(Value::Null),
            },

            Expr::Identifier(name) => self.env.get(name, self.last_span),

            Expr::ArrayLiteral { elements, span } => {
                self.last_span = *span;
                let mut evaluated = Vec::new();
                for elem in elements {
                    evaluated.push(self.eval_expr(elem)?);
                }
                Ok(Value::Array(Rc::new(RefCell::new(evaluated))))
            }

            Expr::ObjectLiteral { pairs, span } => {
                self.last_span = *span;
                let mut map = HashMap::new();
                for (key, val_expr) in pairs {
                    map.insert(key.clone(), self.eval_expr(val_expr)?);
                }
                Ok(Value::Object(Rc::new(RefCell::new(map))))
            }

            Expr::Index {
                object,
                index,
                span,
            } => {
                self.last_span = *span;
                let obj_val = self.eval_expr(object)?;
                let idx_val = self.eval_expr(index)?;

                match &obj_val {
                    Value::Array(arr) => {
                        let idx = self.to_integer_index(&idx_val, *span)?;
                        let elements = arr.borrow();
                        if idx < 0 || idx >= elements.len() as i64 {
                            return Err(HanlinError::runtime(
                                Some(*span),
                                format!(
                                    "Index {} out of bounds for array of length {}",
                                    idx,
                                    elements.len()
                                ),
                            ));
                        }
                        Ok(elements[idx as usize].clone())
                    }
                    Value::Object(obj) => {
                        let key = match idx_val {
                            Value::Str(s) => s,
                            other => {
                                return Err(HanlinError::runtime(
                                    Some(*span),
                                    format!(
                                        "Object property bracket index must be a string, got {:?}",
                                        other
                                    ),
                                ))
                            }
                        };
                        let map = obj.borrow();
                        Ok(map.get(&key).cloned().unwrap_or(Value::Null))
                    }
                    Value::Str(s) => {
                        let idx = self.to_integer_index(&idx_val, *span)?;
                        let chars: Vec<char> = s.chars().collect();
                        if idx < 0 || idx >= chars.len() as i64 {
                            return Err(HanlinError::runtime(
                                Some(*span),
                                format!(
                                    "Index {} out of bounds for string of length {}",
                                    idx,
                                    chars.len()
                                ),
                            ));
                        }
                        Ok(Value::Str(chars[idx as usize].to_string()))
                    }
                    other => Err(HanlinError::runtime(
                        Some(*span),
                        format!("Cannot read property/index of {:?}", other),
                    )),
                }
            }

            Expr::Member {
                object,
                property,
                span,
            } => {
                self.last_span = *span;
                let obj_val = self.eval_expr(object)?;

                if property == "length" {
                    return match &obj_val {
                        Value::Array(arr) => Ok(Value::Integer(arr.borrow().len() as i64)),
                        Value::Str(s) => Ok(Value::Integer(s.chars().count() as i64)),
                        other => Err(HanlinError::runtime(
                            Some(*span),
                            format!("Property '.length' is not supported on {:?}", other),
                        )),
                    };
                }

                match &obj_val {
                    Value::Object(obj) => {
                        let map = obj.borrow();
                        Ok(map.get(property).cloned().unwrap_or(Value::Null))
                    }
                    other => Err(HanlinError::runtime(
                        Some(*span),
                        format!("Cannot read property '{}' of {:?}", property, other),
                    )),
                }
            }

            Expr::MethodCall {
                object,
                method,
                args,
                span,
            } => {
                self.last_span = *span;
                let obj_val = self.eval_expr(object)?;

                let mut evaluated_args = Vec::new();
                for arg in args {
                    evaluated_args.push(self.eval_expr(arg)?);
                }
                dispatch_method(&obj_val, method, evaluated_args, *span)
            }

            Expr::Binary { op, left, right } => {
                let left_val = self.eval_expr(left)?;
                // Handle short-circuiting logical operations
                if *op == BinOp::And {
                    return if left_val.is_truthy() {
                        self.eval_expr(right)
                    } else {
                        Ok(left_val)
                    };
                }
                if *op == BinOp::Or {
                    return if left_val.is_truthy() {
                        Ok(left_val)
                    } else {
                        self.eval_expr(right)
                    };
                }

                let right_val = self.eval_expr(right)?;
                self.eval_binary_op(*op, left_val, right_val)
            }

            Expr::Unary { op, expr } => {
                let val = self.eval_expr(expr)?;
                match op {
                    UnOp::Neg => match val {
                        Value::Integer(i) => Ok(Value::Integer(-i)),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        other => Err(HanlinError::runtime(
                            Some(self.last_span),
                            format!("Unary minus operator is not supported on {:?}", other),
                        )),
                    },
                    UnOp::Not => Ok(Value::Bool(!val.is_truthy())),
                }
            }

            Expr::Assign { name, value } => {
                let val = self.eval_expr(value)?;
                self.env.assign(name, val.clone(), self.last_span)?;
                Ok(val)
            }

            Expr::AssignIndex {
                object,
                index,
                value,
                span,
            } => {
                self.last_span = *span;
                let obj_val = self.eval_expr(object)?;
                let idx_val = self.eval_expr(index)?;
                let val = self.eval_expr(value)?;

                match &obj_val {
                    Value::Array(arr) => {
                        let idx = self.to_integer_index(&idx_val, *span)?;
                        let mut elements = arr.borrow_mut();
                        if idx < 0 || idx >= elements.len() as i64 {
                            return Err(HanlinError::runtime(
                                Some(*span),
                                format!(
                                    "Index {} out of bounds for assignment to array of length {}",
                                    idx,
                                    elements.len()
                                ),
                            ));
                        }
                        elements[idx as usize] = val.clone();
                        Ok(val)
                    }
                    Value::Object(obj) => {
                        let key = match idx_val {
                            Value::Str(s) => s,
                            other => {
                                return Err(HanlinError::runtime(
                                    Some(*span),
                                    format!(
                                        "Object property bracket index must be a string, got {:?}",
                                        other
                                    ),
                                ))
                            }
                        };
                        obj.borrow_mut().insert(key, val.clone());
                        Ok(val)
                    }
                    other => Err(HanlinError::runtime(
                        Some(*span),
                        format!("Cannot assign to property/index of {:?}", other),
                    )),
                }
            }

            Expr::AssignMember {
                object,
                property,
                value,
                span,
            } => {
                self.last_span = *span;
                let obj_val = self.eval_expr(object)?;
                let val = self.eval_expr(value)?;

                match &obj_val {
                    Value::Object(obj) => {
                        obj.borrow_mut().insert(property.clone(), val.clone());
                        Ok(val)
                    }
                    other => Err(HanlinError::runtime(
                        Some(*span),
                        format!("Cannot assign to property '{}' of {:?}", property, other),
                    )),
                }
            }

            Expr::Call { callee, args } => {
                let func_val = self.env.get(callee, self.last_span)?;
                match func_val {
                    Value::Fn {
                        params,
                        body,
                        env: decl_env,
                        ..
                    } => {
                        if params.len() != args.len() {
                            return Err(HanlinError::runtime(
                                Some(self.last_span),
                                format!(
                                    "Function '{}' expects {} arguments, but got {}",
                                    callee,
                                    params.len(),
                                    args.len()
                                ),
                            ));
                        }
                        let mut evaluated_args = Vec::new();
                        for arg in args {
                            evaluated_args.push(self.eval_expr(arg)?);
                        }

                        // Create lexical environment child of declaration env
                        let child_env = Env::new_child(&decl_env);
                        for (param, val) in params.iter().zip(evaluated_args) {
                            child_env.define(param.clone(), val, false);
                        }

                        let mut sub_interpreter = Interpreter::new(child_env);
                        sub_interpreter.last_span = self.last_span;
                        match sub_interpreter.eval_statements(&body)? {
                            Signal::Return(val) => Ok(val),
                            // Break/Continue cannot escape a function body:
                            // the sub_interpreter's loop_depth = 0, so any
                            // naked break/continue already raised a RuntimeError.
                            // Reaching here means the body ended normally.
                            Signal::Break | Signal::Continue => Ok(Value::Null),
                            Signal::None => Ok(Value::Null),
                        }
                    }
                    // Dispatch a top-level NativeFn (e.g. a bare native function in scope)
                    Value::NativeFn { func, .. } => {
                        let mut evaluated_args = Vec::new();
                        for arg in args {
                            evaluated_args.push(self.eval_expr(arg)?);
                        }
                        func(evaluated_args, self.last_span)
                    }
                    other => Err(HanlinError::runtime(
                        Some(self.last_span),
                        format!("Identifier '{}' is not callable (got {:?})", callee, other),
                    )),
                }
            }
        }
    }

    // ── Operation Helpers ──────────────────────────────────────────────────

    fn to_integer_index(&self, val: &Value, span: Span) -> Result<i64> {
        match val {
            Value::Integer(i) => Ok(*i),
            Value::Float(f) => {
                if f.fract() == 0.0 {
                    Ok(*f as i64)
                } else {
                    Err(HanlinError::runtime(
                        Some(span),
                        format!("Array/string index must be an integer, got float {:?}", f),
                    ))
                }
            }
            other => Err(HanlinError::runtime(
                Some(span),
                format!("Array/string index must be an integer, got {:?}", other),
            )),
        }
    }

    fn eval_binary_op(&self, op: BinOp, left: Value, right: Value) -> Result<Value> {
        let span = self.last_span;
        match op {
            BinOp::Add => {
                // String concatenation
                if let (Value::Str(l), r) = (&left, &right) {
                    return Ok(Value::Str(format!("{}{}", l, r.to_string_display())));
                }
                if let (l, Value::Str(r)) = (&left, &right) {
                    return Ok(Value::Str(format!("{}{}", l.to_string_display(), r)));
                }

                match (left, right) {
                    (Value::Integer(l), Value::Integer(r)) => Ok(Value::Integer(l + r)),
                    (Value::Integer(l), Value::Float(r)) => Ok(Value::Float(l as f64 + r)),
                    (Value::Float(l), Value::Integer(r)) => Ok(Value::Float(l + r as f64)),
                    (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l + r)),
                    _ => Err(HanlinError::runtime(Some(span), "Invalid operands for '+'")),
                }
            }
            BinOp::Sub => match (left, right) {
                (Value::Integer(l), Value::Integer(r)) => Ok(Value::Integer(l - r)),
                (Value::Integer(l), Value::Float(r)) => Ok(Value::Float(l as f64 - r)),
                (Value::Float(l), Value::Integer(r)) => Ok(Value::Float(l - r as f64)),
                (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l - r)),
                _ => Err(HanlinError::runtime(Some(span), "Invalid operands for '-'")),
            },
            BinOp::Mul => match (left, right) {
                (Value::Integer(l), Value::Integer(r)) => Ok(Value::Integer(l * r)),
                (Value::Integer(l), Value::Float(r)) => Ok(Value::Float(l as f64 * r)),
                (Value::Float(l), Value::Integer(r)) => Ok(Value::Float(l * r as f64)),
                (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l * r)),
                _ => Err(HanlinError::runtime(Some(span), "Invalid operands for '*'")),
            },
            BinOp::Div => {
                let l_f = match left {
                    Value::Integer(i) => i as f64,
                    Value::Float(f) => f,
                    _ => {
                        return Err(HanlinError::runtime(
                            Some(span),
                            "Invalid left operand for '/'",
                        ))
                    }
                };
                let r_f = match right {
                    Value::Integer(i) => i as f64,
                    Value::Float(f) => f,
                    _ => {
                        return Err(HanlinError::runtime(
                            Some(span),
                            "Invalid right operand for '/'",
                        ))
                    }
                };
                if r_f == 0.0 {
                    return Err(HanlinError::runtime(Some(span), "Division by zero"));
                }
                Ok(Value::Float(l_f / r_f))
            }
            BinOp::Mod => match (left, right) {
                (Value::Integer(l), Value::Integer(r)) => {
                    if r == 0 {
                        return Err(HanlinError::runtime(Some(span), "Modulo by zero"));
                    }
                    Ok(Value::Integer(l % r))
                }
                (Value::Integer(l), Value::Float(r)) => {
                    if r == 0.0 {
                        return Err(HanlinError::runtime(Some(span), "Modulo by zero"));
                    }
                    Ok(Value::Float(l as f64 % r))
                }
                (Value::Float(l), Value::Integer(r)) => {
                    if r == 0 {
                        return Err(HanlinError::runtime(Some(span), "Modulo by zero"));
                    }
                    Ok(Value::Float(l % r as f64))
                }
                (Value::Float(l), Value::Float(r)) => {
                    if r == 0.0 {
                        return Err(HanlinError::runtime(Some(span), "Modulo by zero"));
                    }
                    Ok(Value::Float(l % r))
                }
                _ => Err(HanlinError::runtime(Some(span), "Invalid operands for '%'")),
            },
            BinOp::EqEq => Ok(Value::Bool(self.values_equal(&left, &right))),
            BinOp::NotEq => Ok(Value::Bool(!self.values_equal(&left, &right))),
            BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                self.compare_values(op, left, right)
            }
            _ => unreachable!(),
        }
    }

    fn values_equal(&self, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(l), Value::Bool(r)) => l == r,
            (Value::Integer(l), Value::Integer(r)) => l == r,
            (Value::Float(l), Value::Float(r)) => l == r,
            (Value::Integer(l), Value::Float(r)) => *l as f64 == *r,
            (Value::Float(l), Value::Integer(r)) => *l == *r as f64,
            (Value::Str(l), Value::Str(r)) => l == r,
            (Value::Array(l), Value::Array(r)) => Rc::ptr_eq(l, r),
            (Value::Object(l), Value::Object(r)) => Rc::ptr_eq(l, r),
            _ => false,
        }
    }

    fn compare_values(&self, op: BinOp, left: Value, right: Value) -> Result<Value> {
        let span = self.last_span;
        match (left, right) {
            (Value::Integer(l), Value::Integer(r)) => {
                let res = match op {
                    BinOp::Lt => l < r,
                    BinOp::Gt => l > r,
                    BinOp::LtEq => l <= r,
                    BinOp::GtEq => l >= r,
                    _ => unreachable!(),
                };
                Ok(Value::Bool(res))
            }
            (Value::Float(l), Value::Float(r)) => {
                let res = match op {
                    BinOp::Lt => l < r,
                    BinOp::Gt => l > r,
                    BinOp::LtEq => l <= r,
                    BinOp::GtEq => l >= r,
                    _ => unreachable!(),
                };
                Ok(Value::Bool(res))
            }
            (Value::Integer(l), Value::Float(r)) => {
                let l_f = l as f64;
                let res = match op {
                    BinOp::Lt => l_f < r,
                    BinOp::Gt => l_f > r,
                    BinOp::LtEq => l_f <= r,
                    BinOp::GtEq => l_f >= r,
                    _ => unreachable!(),
                };
                Ok(Value::Bool(res))
            }
            (Value::Float(l), Value::Integer(r)) => {
                let r_f = r as f64;
                let res = match op {
                    BinOp::Lt => l < r_f,
                    BinOp::Gt => l > r_f,
                    BinOp::LtEq => l <= r_f,
                    BinOp::GtEq => l >= r_f,
                    _ => unreachable!(),
                };
                Ok(Value::Bool(res))
            }
            (Value::Str(l), Value::Str(r)) => {
                let res = match op {
                    BinOp::Lt => l < r,
                    BinOp::Gt => l > r,
                    BinOp::LtEq => l <= r,
                    BinOp::GtEq => l >= r,
                    _ => unreachable!(),
                };
                Ok(Value::Bool(res))
            }
            _ => Err(HanlinError::runtime(
                Some(span),
                "Operands for comparison must be numbers or strings",
            )),
        }
    }
}

// =============================================================================
//  Centralized Method Dispatch
// =============================================================================

fn dispatch_method(obj: &Value, method: &str, args_val: Vec<Value>, span: Span) -> Result<Value> {
    match obj {
        Value::Str(s) => match method {
            "length" => {
                if !args_val.is_empty() {
                    return Err(HanlinError::runtime(
                        Some(span),
                        "Method '.length()' expects 0 arguments",
                    ));
                }
                Ok(Value::Integer(s.chars().count() as i64))
            }
            "split" => {
                if args_val.len() != 1 {
                    return Err(HanlinError::runtime(
                        Some(span),
                        format!(
                            "Method '.split()' expects exactly 1 argument, got {}",
                            args_val.len()
                        ),
                    ));
                }
                match &args_val[0] {
                    Value::Str(delim) => {
                        let parts = s
                            .split(delim.as_str())
                            .map(|p| Value::Str(p.to_string()))
                            .collect();
                        Ok(Value::Array(Rc::new(RefCell::new(parts))))
                    }
                    other => Err(HanlinError::runtime(
                        Some(span),
                        format!("'.split()' delimiter must be a string, got {:?}", other),
                    )),
                }
            }
            "trim" => {
                if !args_val.is_empty() {
                    return Err(HanlinError::runtime(
                        Some(span),
                        format!(
                            "Method '.trim()' expects 0 arguments, got {}",
                            args_val.len()
                        ),
                    ));
                }
                Ok(Value::Str(s.trim().to_string()))
            }
            "toUpperCase" => {
                if !args_val.is_empty() {
                    return Err(HanlinError::runtime(
                        Some(span),
                        format!(
                            "Method '.toUpperCase()' expects 0 arguments, got {}",
                            args_val.len()
                        ),
                    ));
                }
                Ok(Value::Str(s.to_uppercase()))
            }
            "toLowerCase" => {
                if !args_val.is_empty() {
                    return Err(HanlinError::runtime(
                        Some(span),
                        format!(
                            "Method '.toLowerCase()' expects 0 arguments, got {}",
                            args_val.len()
                        ),
                    ));
                }
                Ok(Value::Str(s.to_lowercase()))
            }
            _ => Err(HanlinError::runtime(
                Some(span),
                format!("Method '{}()' is not supported on String", method),
            )),
        },
        Value::Array(arr) => match method {
            "length" => {
                if !args_val.is_empty() {
                    return Err(HanlinError::runtime(
                        Some(span),
                        "Method '.length()' expects 0 arguments",
                    ));
                }
                Ok(Value::Integer(arr.borrow().len() as i64))
            }
            "push" => {
                if args_val.len() != 1 {
                    return Err(HanlinError::runtime(
                        Some(span),
                        format!(
                            "Method '.push()' expects exactly 1 argument, got {}",
                            args_val.len()
                        ),
                    ));
                }
                arr.borrow_mut().push(args_val[0].clone());
                Ok(Value::Null)
            }
            "pop" => {
                if !args_val.is_empty() {
                    return Err(HanlinError::runtime(
                        Some(span),
                        format!(
                            "Method '.pop()' expects 0 arguments, got {}",
                            args_val.len()
                        ),
                    ));
                }
                Ok(arr.borrow_mut().pop().unwrap_or(Value::Null))
            }
            "join" => {
                if args_val.len() > 1 {
                    return Err(HanlinError::runtime(
                        Some(span),
                        format!(
                            "Method '.join()' expects 0 or 1 argument, got {}",
                            args_val.len()
                        ),
                    ));
                }
                let delim = if args_val.len() == 1 {
                    match &args_val[0] {
                        Value::Str(d) => d.clone(),
                        other => {
                            return Err(HanlinError::runtime(
                                Some(span),
                                format!("join() delimiter must be a string, got {:?}", other),
                            ))
                        }
                    }
                } else {
                    ",".to_string()
                };
                let mut out = Vec::new();
                for item in arr.borrow().iter() {
                    out.push(item.to_string_display());
                }
                Ok(Value::Str(out.join(&delim)))
            }
            _ => Err(HanlinError::runtime(
                Some(span),
                format!("Method '{}()' is not supported on Array", method),
            )),
        },
        Value::Object(obj_rc) => {
            // Check native methods (e.g., fs.readFile, math.abs)
            let native = obj_rc.borrow().get(method).cloned();
            if let Some(Value::NativeFn { func, .. }) = native {
                return func(args_val, span);
            }
            Err(HanlinError::runtime(
                Some(span),
                format!("Method '{}()' is undefined", method),
            ))
        }
        other => Err(HanlinError::runtime(
            Some(span),
            format!("Method '{}()' is not supported on {:?}", method, other),
        )),
    }
}

// ---------------------------------------------------------------------------
//  Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn run_code(src: &str) -> std::result::Result<Value, HanlinError> {
        let tokens = Lexer::new(src).tokenize()?;
        let program = Parser::new(tokens).parse()?;
        let env = Env::new();
        let mut interpreter = Interpreter::new(env.clone());
        interpreter.interpret(&program)?;
        if let Ok(val) = env.get("x", Span::new(1, 1)) {
            Ok(val)
        } else {
            Ok(Value::Null)
        }
    }

    #[test]
    fn test_interpreter_basics() {
        let res = run_code("let x = 10 + 20;").unwrap();
        assert!(matches!(res, Value::Integer(30)));
    }

    #[test]
    fn test_interpreter_arrays() {
        let res = run_code("let arr = [10, 20, 30]; let x = arr[1];").unwrap();
        assert!(matches!(res, Value::Integer(20)));
    }

    #[test]
    fn test_interpreter_array_push_length() {
        let res = run_code("let arr = [10, 20]; arr.push(30); let x = arr.length;").unwrap();
        assert!(matches!(res, Value::Integer(3)));
    }

    #[test]
    fn test_interpreter_objects() {
        let res = run_code("let u = { name: \"Han\", age: 20 }; let x = u.name;").unwrap();
        if let Value::Str(s) = res {
            assert_eq!(s, "Han");
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_interpreter_object_bracket_access() {
        let res = run_code("let u = { name: \"Han\", age: 20 }; let x = u[\"age\"];").unwrap();
        assert!(matches!(res, Value::Integer(20)));
    }

    #[test]
    fn test_interpreter_object_assignment() {
        let res =
            run_code("let u = { name: \"Han\", age: 20 }; u.age = 21; let x = u.age;").unwrap();
        assert!(matches!(res, Value::Integer(21)));
    }

    #[test]
    fn test_interpreter_closure() {
        let res = run_code(
            "
            fn make_counter() {
                let count = 0;
                fn incr() {
                    count = count + 1;
                    return count;
                }
                return incr;
            }
            let c = make_counter();
            c();
            let x = c();
        ",
        )
        .unwrap();
        assert!(matches!(res, Value::Integer(2)));
    }

    #[test]
    fn test_interpreter_runtime_error_undefined() {
        let res = run_code("let x = y;");
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("Undefined variable 'y'"));
    }

    #[test]
    fn test_interpreter_runtime_error_const_assignment() {
        let res = run_code("const PI = 3.14; PI = 3.15;");
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("Assignment to constant variable"));
    }

    // ── v0.2.1 tests ──────────────────────────────────────────────────────────

    #[test]
    fn test_try_catch_no_error() {
        // try block succeeds → catch body should NOT run
        let res = run_code("let x = 0; try { x = 42; } catch (e) { x = 99; }").unwrap();
        assert!(matches!(res, Value::Integer(42)));
    }

    #[test]
    fn test_try_catch_catches_runtime_error() {
        // Accessing undefined var triggers RuntimeError → catch binds message
        let res = run_code(
            r#"let x = "none"; try { let bad = undefinedVar + 1; } catch (err) { x = err; }"#,
        )
        .unwrap();
        if let Value::Str(msg) = res {
            assert!(
                msg.contains("Undefined variable"),
                "unexpected msg: {}",
                msg
            );
        } else {
            panic!("expected string error message, got {:?}", res);
        }
    }

    #[test]
    fn test_try_catch_const_error() {
        // const reassignment inside try → caught
        let res = run_code(r#"let x = "ok"; try { const N = 1; N = 2; } catch (err) { x = err; }"#)
            .unwrap();
        if let Value::Str(msg) = res {
            assert!(msg.contains("constant"), "unexpected msg: {}", msg);
        } else {
            panic!("expected caught error string");
        }
    }

    // run_code_with_builtins: same as run_code but registers fs + stdlib
    fn run_code_builtins(src: &str) -> std::result::Result<Value, HanlinError> {
        let tokens = Lexer::new(src).tokenize()?;
        let program = Parser::new(tokens).parse()?;
        let env = Env::new();
        register_builtins(&env);
        let mut interpreter = Interpreter::new(env.clone());
        interpreter.interpret(&program)?;
        if let Ok(val) = env.get("x", Span::new(1, 1)) {
            Ok(val)
        } else {
            Ok(Value::Null)
        }
    }

    #[test]
    fn test_fs_read_missing_file_caught() {
        // Reading a non-existent file inside try → catch receives the OS error
        let res = run_code_builtins(
            r#"let x = "caught"; try { let c = fs.readFile("/no/such/file.txt"); x = c; } catch (err) { x = "err:" + err; }"#
        ).unwrap();
        if let Value::Str(msg) = res {
            assert!(
                msg.starts_with("err:"),
                "expected caught prefix, got: {}",
                msg
            );
        } else {
            panic!("expected string result, got {:?}", res);
        }
    }

    // ── v0.3 for / break / continue interpreter tests ─────────────────────────

    #[test]
    fn test_for_basic_sum() {
        // Sum 0..4 via for loop → x = 10
        let res = run_code("let x = 0; for (let i = 0; i < 5; i = i + 1) { x = x + i; }").unwrap();
        assert!(
            matches!(res, Value::Integer(10)),
            "expected 10, got {:?}",
            res
        );
    }

    #[test]
    fn test_for_zero_iterations() {
        // Condition false from the start → x stays 99
        let res = run_code("let x = 99; for (let i = 0; i < 0; i = i + 1) { x = 0; }").unwrap();
        assert!(
            matches!(res, Value::Integer(99)),
            "expected 99 (zero iterations)"
        );
    }

    #[test]
    fn test_for_expression_init() {
        // Init as expression (pre-declared variable), not var-decl
        let res = run_code("let i = 0; let x = 0; for (i = 1; i <= 3; i = i + 1) { x = x + i; }")
            .unwrap();
        assert!(
            matches!(res, Value::Integer(6)),
            "expected 1+2+3=6, got {:?}",
            res
        );
    }

    #[test]
    fn test_for_variable_scope() {
        // Variable declared in for init must not be visible outside
        let src = "for (let inner = 0; inner < 3; inner = inner + 1) { } let x = inner;";
        let res = run_code(src);
        assert!(
            res.is_err(),
            "expected error: 'inner' should not be visible after for"
        );
        assert!(res.unwrap_err().to_string().contains("Undefined variable"));
    }

    #[test]
    fn test_while_break() {
        // Break exits immediately — x should be 3
        let res =
            run_code("let x = 0; while (true) { x = x + 1; if (x == 3) { break; } }").unwrap();
        assert!(matches!(res, Value::Integer(3)), "expected 3 after break");
    }

    #[test]
    fn test_for_break() {
        // Break inside for — accumulate until i==3, then stop
        let res = run_code(
            "let x = 0; for (let i = 0; i < 10; i = i + 1) { if (i == 3) { break; } x = x + 1; }",
        )
        .unwrap();
        // Iterations with body executed: i=0,1,2 (break before x+=1 when i==3)
        assert!(
            matches!(res, Value::Integer(3)),
            "expected 3, got {:?}",
            res
        );
    }

    #[test]
    fn test_for_continue() {
        // Continue skips odd numbers — sum of even numbers 0,2,4 = 6
        let res = run_code(
            r#"
            let x = 0;
            for (let i = 0; i < 6; i = i + 1) {
                if (i % 2 != 0) { continue; }
                x = x + i;
            }
        "#,
        )
        .unwrap();
        assert!(
            matches!(res, Value::Integer(6)),
            "expected 0+2+4=6, got {:?}",
            res
        );
    }

    #[test]
    fn test_nested_loops_break_inner_only() {
        // Outer loop runs 3 times; inner breaks immediately each time → x = 3
        let res = run_code(
            r#"
            let x = 0;
            let outer = 0;
            while (outer < 3) {
                let inner = 0;
                while (inner < 100) {
                    break;
                }
                x = x + 1;
                outer = outer + 1;
            }
        "#,
        )
        .unwrap();
        assert!(
            matches!(res, Value::Integer(3)),
            "expected 3 outer iterations, got {:?}",
            res
        );
    }

    #[test]
    fn test_break_outside_loop_is_error() {
        let res = run_code("break;");
        assert!(
            res.is_err(),
            "expected runtime error for break outside loop"
        );
        assert!(res.unwrap_err().to_string().contains("outside of a loop"));
    }

    #[test]
    fn test_continue_outside_loop_is_error() {
        let res = run_code("continue;");
        assert!(
            res.is_err(),
            "expected runtime error for continue outside loop"
        );
        assert!(res.unwrap_err().to_string().contains("outside of a loop"));
    }

    #[test]
    fn test_return_inside_for_exits_function() {
        // return inside a for loop should exit the function, not just the loop
        let res = run_code(
            r#"
            fn find_first_even(n) {
                for (let i = 0; i < n; i = i + 1) {
                    if (i % 2 == 0 && i > 0) { return i; }
                }
                return -1;
            }
            let x = find_first_even(10);
        "#,
        )
        .unwrap();
        assert!(
            matches!(res, Value::Integer(2)),
            "expected first even > 0 is 2, got {:?}",
            res
        );
    }

    #[test]
    fn test_for_infinite_with_break() {
        // for (;;) must terminate via break; x should reach 5
        let res = run_code(
            r#"
            let x = 0;
            for (;;) {
                x = x + 1;
                if (x == 5) { break; }
            }
        "#,
        )
        .unwrap();
        assert!(
            matches!(res, Value::Integer(5)),
            "expected 5, got {:?}",
            res
        );
    }

    #[test]
    fn test_null_equality() {
        let res1 = run_code("let a = null; let x = a == null;").unwrap();
        assert!(
            matches!(res1, Value::Bool(true)),
            "expected true, got {:?}",
            res1
        );

        let res2 = run_code("let a = null; let x = a == false;").unwrap();
        assert!(
            matches!(res2, Value::Bool(false)),
            "expected false, got {:?}",
            res2
        );
    }

    #[test]
    fn test_null_property_access_error() {
        let err = run_code("let a = null; let x = a.foo;").unwrap_err();
        assert!(err.message.contains("Cannot read property 'foo' of null"));
    }

    #[test]
    fn test_null_index_access_error() {
        let err = run_code("let a = null; let x = a[0];").unwrap_err();
        assert!(err.message.contains("Cannot read property/index of null"));
    }

    #[test]
    fn test_invalid_index() {
        let err = run_code("let a = [1, 2]; let x = a[5];").unwrap_err();
        assert!(err.message.contains("Index 5 out of bounds"));
    }

    #[test]
    fn test_wrong_operand_type() {
        let err = run_code("let a = 5 * \"hello\";").unwrap_err();
        assert!(err.message.contains("Invalid operands for '*'"));
    }

    #[test]
    fn test_else_if_parsing_and_eval() {
        let src = r#"
            let val = 10;
            let x = 0;
            if (val == 5) {
                x = 1;
            } else if (val == 10) {
                x = 2;
            } else {
                x = 3;
            }
        "#;
        let res = run_code(src).unwrap();
        assert!(
            matches!(res, Value::Integer(2)),
            "expected 2, got {:?}",
            res
        );
    }
    #[test]
    fn test_fs_write_and_exists() {
        let src = r#"
            let path = "/tmp/hanlin_test_file.txt";
            fs.writeFile(path, "hello hanlin");
            let x = fs.exists(path);
        "#;
        let res = run_code_builtins(src).unwrap();
        assert!(matches!(res, Value::Bool(true)));
        let _ = std::fs::remove_file("/tmp/hanlin_test_file.txt");
    }

    #[test]
    fn test_math_functions() {
        let src = r#"
            let a = math.abs(-5);
            let b = math.sqrt(16);
            let c = math.pow(2, 3);
            let x = a + b + c; // 5 + 4 + 8 = 17
        "#;
        let res = run_code_builtins(src).unwrap();
        match res {
            Value::Integer(17) | Value::Float(_) => (),
            _ => panic!("expected 17, got {:?}", res),
        }
    }

    #[test]
    fn test_global_conversions() {
        let src = r#"
            let a = int("10");
            let b = float(3);
            let c = str(10.5);
            let x = a == 10 && b == 3.0 && c == "10.5";
        "#;
        let res = run_code_builtins(src).unwrap();
        assert!(matches!(res, Value::Bool(true)));
    }

    #[test]
    fn test_string_methods() {
        let src = r#"
            let s = "Hello World";
            let a = s.toUpperCase();
            let b = s.toLowerCase();
            let c = s.length();
            let x = a == "HELLO WORLD" && b == "hello world" && c == 11;
        "#;
        let res = run_code_builtins(src).unwrap();
        assert!(matches!(res, Value::Bool(true)));
    }

    #[test]
    fn test_array_methods() {
        let src = r#"
            let arr = [1, 2, 3];
            arr.push(4);
            let last = arr.pop();
            let len = arr.length();
            let joined = arr.join("-");
            let x = last == 4 && len == 3 && joined == "1-2-3";
        "#;
        let res = run_code_builtins(src).unwrap();
        assert!(matches!(res, Value::Bool(true)));
    }

    #[test]
    fn test_string_split() {
        let src = r#"
            let s = "a,b,c";
            let parts = s.split(",");
            let x = parts.length() == 3 && parts[0] == "a" && parts[1] == "b" && parts[2] == "c";
        "#;
        let res = run_code_builtins(src).unwrap();
        assert!(matches!(res, Value::Bool(true)));
    }

    #[test]
    fn test_string_trim() {
        let src = r#"
            let s = "  hello  ";
            let trimmed = s.trim();
            let x = trimmed == "hello";
        "#;
        let res = run_code_builtins(src).unwrap();
        assert!(matches!(res, Value::Bool(true)));
    }

    #[test]
    fn test_compound_assignments() {
        let src = r#"
            let x = 10;
            x += 5;
            x -= 3;
            x *= 2;
            x /= 4;
        "#;
        let res = run_code(src).unwrap();
        match res {
            Value::Integer(6) => {}
            Value::Float(v) if (v - 6.0).abs() < f64::EPSILON => {}
            other => panic!("expected 6, got {:?}", other),
        }
    }

    #[test]
    fn test_compound_assignment_array_and_object() {
        let src = r#"
            let arr = [1];
            arr[0] += 4;
            let obj = { count: 2 };
            obj.count *= 3;
            let x = arr[0] == 5 && obj.count == 6;
        "#;
        let res = run_code(src).unwrap();
        assert!(matches!(res, Value::Bool(true)));
    }

    #[test]
    fn test_modulo_by_zero_is_runtime_error() {
        let err = run_code("let x = 10 % 0;").unwrap_err();
        assert!(err.message.contains("Modulo by zero"));
    }
}

/// Register all built-in namespaces and functions in the given environment.
///
/// This must be called once on the global environment before interpretation.
pub fn register_builtins(env: &Env) {
    // ── fs namespace ─────────────────────────────────────────────────────────

    fn fs_read_file(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 1 {
            return Err(HanlinError::runtime(
                Some(span),
                format!(
                    "fs.readFile() expects 1 argument (filename), got {}",
                    args.len()
                ),
            ));
        }
        let path = match &args[0] {
            Value::Str(s) => s.clone(),
            other => {
                return Err(HanlinError::runtime(
                    Some(span),
                    format!("fs.readFile() argument must be a string, got {:?}", other),
                ))
            }
        };
        std::fs::read_to_string(&path).map(Value::Str).map_err(|e| {
            HanlinError::runtime(
                Some(span),
                format!("fs.readFile(\"{}\") failed: {}", path, e),
            )
        })
    }

    fn fs_write_file(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 2 {
            return Err(HanlinError::runtime(
                Some(span),
                format!("fs.writeFile() expects 2 arguments, got {}", args.len()),
            ));
        }
        let path = match &args[0] {
            Value::Str(s) => s.clone(),
            other => {
                return Err(HanlinError::runtime(
                    Some(span),
                    format!("fs.writeFile() path must be a string, got {:?}", other),
                ))
            }
        };
        let content = match &args[1] {
            Value::Str(s) => s.clone(),
            other => other.to_string_display(),
        };
        std::fs::write(&path, content)
            .map(|_| Value::Null)
            .map_err(|e| {
                HanlinError::runtime(
                    Some(span),
                    format!("fs.writeFile(\"{}\") failed: {}", path, e),
                )
            })
    }

    fn fs_exists(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 1 {
            return Err(HanlinError::runtime(
                Some(span),
                format!("fs.exists() expects 1 argument, got {}", args.len()),
            ));
        }
        let path = match &args[0] {
            Value::Str(s) => s.clone(),
            other => {
                return Err(HanlinError::runtime(
                    Some(span),
                    format!("fs.exists() argument must be a string, got {:?}", other),
                ))
            }
        };
        Ok(Value::Bool(std::path::Path::new(&path).exists()))
    }

    let fs_map: HashMap<String, Value> = {
        let mut m = HashMap::new();
        m.insert(
            "readFile".to_string(),
            Value::NativeFn {
                name: "readFile".to_string(),
                func: fs_read_file,
            },
        );
        m.insert(
            "writeFile".to_string(),
            Value::NativeFn {
                name: "writeFile".to_string(),
                func: fs_write_file,
            },
        );
        m.insert(
            "exists".to_string(),
            Value::NativeFn {
                name: "exists".to_string(),
                func: fs_exists,
            },
        );
        m
    };
    env.define(
        "fs".to_string(),
        Value::Object(Rc::new(RefCell::new(fs_map))),
        false,
    );

    // ── math namespace ───────────────────────────────────────────────────────

    fn math_abs(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 1 {
            return Err(HanlinError::runtime(
                Some(span),
                format!("math.abs() expects 1 argument, got {}", args.len()),
            ));
        }
        match &args[0] {
            Value::Integer(i) => Ok(Value::Integer(i.abs())),
            Value::Float(f) => Ok(Value::Float(f.abs())),
            other => Err(HanlinError::runtime(
                Some(span),
                format!("math.abs() expects a number, got {:?}", other),
            )),
        }
    }

    fn math_sqrt(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 1 {
            return Err(HanlinError::runtime(
                Some(span),
                format!("math.sqrt() expects 1 argument, got {}", args.len()),
            ));
        }
        let val = match &args[0] {
            Value::Integer(i) => *i as f64,
            Value::Float(f) => *f,
            other => {
                return Err(HanlinError::runtime(
                    Some(span),
                    format!("math.sqrt() expects a number, got {:?}", other),
                ))
            }
        };
        Ok(Value::Float(val.sqrt()))
    }

    fn math_pow(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 2 {
            return Err(HanlinError::runtime(
                Some(span),
                format!("math.pow() expects 2 arguments, got {}", args.len()),
            ));
        }
        let base = match &args[0] {
            Value::Integer(i) => *i as f64,
            Value::Float(f) => *f,
            other => {
                return Err(HanlinError::runtime(
                    Some(span),
                    format!("math.pow() base must be a number, got {:?}", other),
                ))
            }
        };
        let exp = match &args[1] {
            Value::Integer(i) => *i as f64,
            Value::Float(f) => *f,
            other => {
                return Err(HanlinError::runtime(
                    Some(span),
                    format!("math.pow() exponent must be a number, got {:?}", other),
                ))
            }
        };
        Ok(Value::Float(base.powf(exp)))
    }

    let math_map: HashMap<String, Value> = {
        let mut m = HashMap::new();
        m.insert(
            "abs".to_string(),
            Value::NativeFn {
                name: "abs".to_string(),
                func: math_abs,
            },
        );
        m.insert(
            "sqrt".to_string(),
            Value::NativeFn {
                name: "sqrt".to_string(),
                func: math_sqrt,
            },
        );
        m.insert(
            "pow".to_string(),
            Value::NativeFn {
                name: "pow".to_string(),
                func: math_pow,
            },
        );
        m
    };
    env.define(
        "math".to_string(),
        Value::Object(Rc::new(RefCell::new(math_map))),
        false,
    );

    // ── global functions ─────────────────────────────────────────────────────

    fn global_int(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 1 {
            return Err(HanlinError::runtime(
                Some(span),
                format!("int() expects 1 argument, got {}", args.len()),
            ));
        }
        match &args[0] {
            Value::Integer(i) => Ok(Value::Integer(*i)),
            Value::Float(f) => Ok(Value::Integer(*f as i64)),
            Value::Str(s) => s.parse::<i64>().map(Value::Integer).map_err(|_| {
                HanlinError::runtime(Some(span), format!("Cannot parse '{}' to int", s))
            }),
            other => Err(HanlinError::runtime(
                Some(span),
                format!("int() cannot convert {:?}", other),
            )),
        }
    }

    fn global_float(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 1 {
            return Err(HanlinError::runtime(
                Some(span),
                format!("float() expects 1 argument, got {}", args.len()),
            ));
        }
        match &args[0] {
            Value::Integer(i) => Ok(Value::Float(*i as f64)),
            Value::Float(f) => Ok(Value::Float(*f)),
            Value::Str(s) => s.parse::<f64>().map(Value::Float).map_err(|_| {
                HanlinError::runtime(Some(span), format!("Cannot parse '{}' to float", s))
            }),
            other => Err(HanlinError::runtime(
                Some(span),
                format!("float() cannot convert {:?}", other),
            )),
        }
    }

    fn global_str(args: Vec<Value>, span: Span) -> Result<Value> {
        if args.len() != 1 {
            return Err(HanlinError::runtime(
                Some(span),
                format!("str() expects 1 argument, got {}", args.len()),
            ));
        }
        Ok(Value::Str(args[0].to_string_display()))
    }

    env.define(
        "int".to_string(),
        Value::NativeFn {
            name: "int".to_string(),
            func: global_int,
        },
        false,
    );
    env.define(
        "float".to_string(),
        Value::NativeFn {
            name: "float".to_string(),
            func: global_float,
        },
        false,
    );
    env.define(
        "str".to_string(),
        Value::NativeFn {
            name: "str".to_string(),
            func: global_str,
        },
        false,
    );
}
