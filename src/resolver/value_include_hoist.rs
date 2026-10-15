//! Purpose:
//! Lowers `include`/`require` used as an expression anywhere in a statement — an argument
//! (`array_replace_recursive(require __DIR__.'/en.php', [...])`, Carbon's lang files), an
//! operand (`10 + require F`), an array element, a static-property value, a `??=` value or a
//! ternary branch — into the one shape the resolver can expand: `$tmp = include F;` followed by
//! the statement reading `$tmp`.
//!
//! Called from:
//! - `crate::resolver::engine::resolve_stmts()`, before generic expression resolution.
//!
//! Key details:
//! - The included file runs in the CALLER's scope, so it is expanded as statements, not as an
//!   expression. Hoisting it in front of the statement is only correct when nothing the
//!   statement evaluates before the include has an observable effect. The walk visits the
//!   statement's own expressions in PHP evaluation order and refuses an include that follows a
//!   call, an assignment or any other effectful operand: that is a compile error, never a
//!   reordering. Variable reads are not effects here: php-src fetches a CV operand when the
//!   operator runs, after the include, which is also what the hoisted form does.
//! - An include in a conditionally evaluated operand (a ternary branch, the right side of `??`,
//!   `?:`, `&&`, `||`, so also `$x ??= require F`) hoists the whole conditional into an `if`
//!   that runs the include only when PHP would, with the operator left in place over one
//!   temporary per operand (see `Hoister::hoist_conditional`). A temporary assigned in one
//!   branch only is typed `mixed` by the checker, so a typed `return` of such a value can be
//!   refused at compile time; it is never silently wrong.
//! - An include in a `match` arm, a nullsafe call's arguments, an assignment expression or
//!   another node this walk does not model is a compile error.
//! - Closure and arrow-function bodies are not entered: their includes are deferred, and the
//!   closure body is resolved as its own statement list.
//! - Anything left over is refused by the safety net in `crate::resolver::exprs::resolve_expr`.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::errors::CompileError;
use crate::parser::ast::{ArrayEntry, BinOp, Expr, ExprKind, Stmt, StmtKind};
use crate::span::Span;

use super::contains::expr_has_includes;

/// Process-global counter producing unique hidden temporaries for hoisted includes.
static HOIST_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Rewrites the expression-position includes in `stmt`'s own expressions, unless the include
/// is already the whole value of a plain assignment or a `return` (those are expanded directly
/// by `engine::try_expand_value_include`). Returns `None` when there is nothing to rewrite.
///
/// On success it returns the hoisted `$tmp = include F;` statements, to run first, and the
/// rewritten statement, which reads the temporaries and keeps any conditionally evaluated
/// include as a statement expression.
pub(super) fn hoist_value_includes(
    stmt: &Stmt,
) -> Result<Option<(Vec<Stmt>, Stmt)>, CompileError> {
    if !stmt_head_has_includes(stmt) {
        return Ok(None);
    }
    let mut rewritten = stmt.clone();
    let mut hoister = Hoister::default();
    match &mut rewritten.kind {
        StmtKind::ExprStmt(value)
        | StmtKind::Echo(value)
        | StmtKind::Throw(value)
        | StmtKind::Return(Some(value))
        | StmtKind::Assign { value, .. }
        | StmtKind::TypedAssign { value, .. }
        | StmtKind::ArrayPush { value, .. }
        | StmtKind::StaticPropertyAssign { value, .. }
        | StmtKind::StaticPropertyArrayPush { value, .. }
        | StmtKind::ListUnpack { value, .. } => hoister.visit(value)?,
        StmtKind::ArrayAssign { index, value, .. }
        | StmtKind::StaticPropertyArrayAssign { index, value, .. } => {
            hoister.visit(index)?;
            hoister.visit(value)?;
        }
        StmtKind::NestedArrayAssign { target, value } => {
            hoister.visit(target)?;
            hoister.visit(value)?;
        }
        StmtKind::PropertyAssign { object, value, .. }
        | StmtKind::PropertyArrayPush { object, value, .. } => {
            hoister.visit(object)?;
            hoister.visit(value)?;
        }
        StmtKind::PropertyArrayAssign { object, index, value, .. } => {
            hoister.visit(object)?;
            hoister.visit(index)?;
            hoister.visit(value)?;
        }
        // Only the expression evaluated once, before any body, can be hoisted. An `elseif`
        // condition, a `case` value or a loop condition runs conditionally or repeatedly and is
        // left to the safety net.
        StmtKind::If { condition, .. } => hoister.visit(condition)?,
        StmtKind::Switch { subject, .. } => hoister.visit(subject)?,
        StmtKind::Foreach { array, .. } => hoister.visit(array)?,
        _ => return Ok(None),
    }
    if hoister.hoisted.is_empty() {
        return Ok(None);
    }
    Ok(Some((hoister.hoisted, rewritten)))
}

/// Whether the statement's own (non-body) expressions hold an include. A cheap pre-check so
/// statements without one are not cloned.
fn stmt_head_has_includes(stmt: &Stmt) -> bool {
    match &stmt.kind {
        // The direct forms are expanded by `engine::try_expand_value_include`.
        StmtKind::Assign { value, .. } | StmtKind::Return(Some(value))
            if matches!(value.kind, ExprKind::IncludeValue { .. }) =>
        {
            false
        }
        StmtKind::ExprStmt(value)
        | StmtKind::Echo(value)
        | StmtKind::Throw(value)
        | StmtKind::Return(Some(value))
        | StmtKind::Assign { value, .. }
        | StmtKind::TypedAssign { value, .. }
        | StmtKind::ArrayPush { value, .. }
        | StmtKind::StaticPropertyAssign { value, .. }
        | StmtKind::StaticPropertyArrayPush { value, .. }
        | StmtKind::ListUnpack { value, .. }
        | StmtKind::If { condition: value, .. }
        | StmtKind::Switch { subject: value, .. }
        | StmtKind::Foreach { array: value, .. } => expr_has_includes(value),
        StmtKind::ArrayAssign { index, value, .. }
        | StmtKind::StaticPropertyArrayAssign { index, value, .. } => {
            expr_has_includes(index) || expr_has_includes(value)
        }
        StmtKind::NestedArrayAssign { target, value } => {
            expr_has_includes(target) || expr_has_includes(value)
        }
        StmtKind::PropertyAssign { object, value, .. }
        | StmtKind::PropertyArrayPush { object, value, .. } => {
            expr_has_includes(object) || expr_has_includes(value)
        }
        StmtKind::PropertyArrayAssign { object, index, value, .. } => {
            expr_has_includes(object) || expr_has_includes(index) || expr_has_includes(value)
        }
        _ => false,
    }
}

/// Collects the hoisted statements while walking one statement in evaluation order.
#[derive(Default)]
struct Hoister {
    /// Statements to run before the rewritten statement, in order.
    hoisted: Vec<Stmt>,
    /// Whether an operand with an observable effect has been evaluated (and left in place)
    /// before the current point. An include after it cannot be hoisted without reordering.
    effect_seen: bool,
}

impl Hoister {
    /// Visits `expr` in PHP evaluation order, replacing each hoistable include (or conditional
    /// holding one) by a hidden temporary.
    fn visit(&mut self, expr: &mut Expr) -> Result<(), CompileError> {
        let span = expr.span;
        match &mut expr.kind {
            ExprKind::IncludeValue { .. } => {
                if self.effect_seen {
                    return Err(after_effect_error(span));
                }
                let temp = fresh_temp();
                let include =
                    std::mem::replace(expr, Expr::new(ExprKind::Variable(temp.clone()), span));
                self.hoisted.push(assign(&temp, include, span));
                Ok(())
            }

            // Leaves that read without an effect of their own.
            ExprKind::StringLiteral(_)
            | ExprKind::IntLiteral(_)
            | ExprKind::FloatLiteral(_)
            | ExprKind::BoolLiteral(_)
            | ExprKind::Null
            | ExprKind::Variable(_)
            | ExprKind::ConstRef(_)
            | ExprKind::MagicConstant(_)
            | ExprKind::This
            | ExprKind::ClassConstant { .. }
            | ExprKind::ScopedConstantAccess { .. }
            | ExprKind::StaticPropertyAccess { .. } => Ok(()),
            // Creating a closure runs none of its body; an include inside it is deferred and
            // is resolved with the closure body.
            ExprKind::Closure { .. } => Ok(()),

            // Containers: their elements are evaluated in order and building them has no effect.
            ExprKind::ArrayLiteral(items) => {
                for item in items {
                    self.visit(item)?;
                }
                Ok(())
            }
            ExprKind::ArrayLiteralAssoc(entries) => {
                for (key, value) in entries {
                    self.visit(key)?;
                    self.visit(value)?;
                }
                Ok(())
            }
            ExprKind::ArrayLiteralMixed(entries) => {
                for entry in entries {
                    if matches!(entry, ArrayEntry::Spread(_)) {
                        // Spreading a Traversable runs its iterator.
                        for value in entry.exprs_mut() {
                            self.visit(value)?;
                        }
                        self.effect_seen = true;
                    } else {
                        for value in entry.exprs_mut() {
                            self.visit(value)?;
                        }
                    }
                }
                Ok(())
            }
            ExprKind::NamedArg { value, .. } => self.visit(value),
            // `@include F`: the include is hoisted out of the `@`, as the statement form
            // `@include F;` already drops the operator.
            ExprKind::ErrorSuppress(inner) => self.visit(inner),

            // Conditionally evaluated operands. An include in the lazy operand moves into an
            // `if` with the rest of the conditional (see `hoist_conditional`); otherwise the
            // node is an ordinary effectful operation.
            ExprKind::BinaryOp {
                left,
                op: BinOp::And | BinOp::Or,
                right,
            } => {
                if expr_has_includes(right) {
                    return self.hoist_conditional(expr);
                }
                self.visit(left)?;
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                if expr_has_includes(then_expr) || expr_has_includes(else_expr) {
                    return self.hoist_conditional(expr);
                }
                self.visit(condition)?;
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::NullCoalesce { value, default }
            | ExprKind::ShortTernary { value, default } => {
                if expr_has_includes(default) {
                    return self.hoist_conditional(expr);
                }
                self.visit(value)?;
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::Match { subject, arms, default } => {
                let arms_have_include = arms.iter().any(|(patterns, value)| {
                    patterns.iter().any(expr_has_includes) || expr_has_includes(value)
                }) || default.as_deref().is_some_and(expr_has_includes);
                if arms_have_include {
                    return Err(CompileError::new(
                        span,
                        "include/require in a `match` arm is not supported yet; \
                         use an `if` and assign it to a local variable first",
                    ));
                }
                self.visit(subject)?;
                self.effect_seen = true;
                Ok(())
            }

            // Operators and accesses: operands first, then the operation itself, which can
            // warn, throw or run user code (`__get`, `__toString`, `offsetGet`).
            ExprKind::BinaryOp { left, right, .. } => {
                self.visit(left)?;
                self.visit(right)?;
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::Negate(inner)
            | ExprKind::Not(inner)
            | ExprKind::BitNot(inner)
            | ExprKind::Print(inner)
            | ExprKind::Clone(inner)
            | ExprKind::Throw(inner)
            | ExprKind::Cast { expr: inner, .. }
            | ExprKind::InstanceOf { value: inner, .. }
            | ExprKind::PropertyAccess { object: inner, .. }
            | ExprKind::ObjectClassName { object: inner } => {
                self.visit(inner)?;
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::ArrayAccess { array, index } => {
                self.visit(array)?;
                self.visit(index)?;
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::Pipe { value, callable } => {
                self.visit(value)?;
                self.visit(callable)?;
                self.effect_seen = true;
                Ok(())
            }

            // Calls and constructions: the callee or receiver, then the arguments, then the call.
            ExprKind::FunctionCall { args, .. }
            | ExprKind::ClosureCall { args, .. }
            | ExprKind::StaticMethodCall { args, .. }
            | ExprKind::NewObject { args, .. }
            | ExprKind::NewGeneric { args, .. }
            | ExprKind::NewScopedObject { args, .. } => {
                for arg in args {
                    self.visit(arg)?;
                }
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::MethodCall { object, args, .. } => {
                self.visit(object)?;
                for arg in args {
                    self.visit(arg)?;
                }
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::ExprCall { callee, args } => {
                self.visit(callee)?;
                for arg in args {
                    self.visit(arg)?;
                }
                self.effect_seen = true;
                Ok(())
            }
            ExprKind::NewDynamic { name_expr, args } => {
                self.visit(name_expr)?;
                for arg in args {
                    self.visit(arg)?;
                }
                self.effect_seen = true;
                Ok(())
            }

            // Everything else (assignment expressions with their preludes, nullsafe chains whose
            // arguments run conditionally, `yield`, increments, FFI nodes...) is not modelled.
            _ => {
                if expr_has_includes(expr) {
                    return Err(CompileError::new(
                        span,
                        "include/require is not supported inside this expression yet; \
                         assign it to a local variable first",
                    ));
                }
                self.effect_seen = true;
                Ok(())
            }
        }
    }

    /// Hoists a conditional whose lazily evaluated operand holds an include into an `if` that
    /// runs that operand only when PHP would, and leaves the operator in place over
    /// temporaries:
    ///
    /// - `c ? a : b` → `$c = c; if ($c) { $a = a; } else { $b = b; }`, then `$c ? $a : $b`
    ///   (a branch without an include stays inline and is still evaluated lazily);
    /// - `v ?? d` → `$v = v ?? null; if ($v === null) { $d = d; }`, then `$v ?? $d`;
    /// - `v ?: d` → `$v = v; if (!$v) { $d = d; }`, then `$v ?: $d`;
    /// - `l && r` / `l || r` → `$l = l; if ($l) { $r = r; }` (`if (!$l)` for `||`), then
    ///   `$l && $r` / `$l || $r`.
    ///
    /// Every operand gets its OWN temporary and the operator stays: elephc types a local from
    /// its assignments, so one temporary shared by operands of different types would not
    /// type-check, while the operator unifies them as the original did. The first operand is
    /// evaluated once, and the whole conditional is hoisted, so nothing effectful may precede it.
    fn hoist_conditional(&mut self, expr: &mut Expr) -> Result<(), CompileError> {
        let span = expr.span;
        if self.effect_seen {
            return Err(after_effect_error(span));
        }
        let placeholder = Expr::new(ExprKind::Null, span);
        let original = std::mem::replace(expr, placeholder).kind;
        let null = || Expr::new(ExprKind::Null, span);
        match original {
            ExprKind::Ternary {
                mut condition,
                then_expr,
                else_expr,
            } => {
                // Includes in the condition itself are hoisted first, in order.
                self.visit(&mut condition)?;
                let cond_temp = fresh_temp();
                self.hoisted.push(assign(&cond_temp, *condition, span));
                let (then_body, then_expr) = operand_into_temp(*then_expr, span);
                let (else_body, else_expr) = operand_into_temp(*else_expr, span);
                self.push_if(variable(&cond_temp, span), then_body, else_body, span);
                // A branch left inline still runs at the ternary's place.
                self.effect_seen = !is_inert(&then_expr) || !is_inert(&else_expr);
                *expr = Expr::new(
                    ExprKind::Ternary {
                        condition: Box::new(variable(&cond_temp, span)),
                        then_expr: Box::new(then_expr),
                        else_expr: Box::new(else_expr),
                    },
                    span,
                );
            }
            ExprKind::NullCoalesce { mut value, default } => {
                self.visit(&mut value)?;
                let value_temp = fresh_temp();
                let probe = ExprKind::NullCoalesce { value, default: Box::new(null()) };
                self.hoisted.push(assign(&value_temp, Expr::new(probe, span), span));
                let is_null = ExprKind::BinaryOp {
                    left: Box::new(variable(&value_temp, span)),
                    op: BinOp::StrictEq,
                    right: Box::new(null()),
                };
                let (body, default) = operand_into_temp(*default, span);
                self.push_if(Expr::new(is_null, span), body, Vec::new(), span);
                self.effect_seen = false;
                *expr = Expr::new(
                    ExprKind::NullCoalesce {
                        value: Box::new(variable(&value_temp, span)),
                        default: Box::new(default),
                    },
                    span,
                );
            }
            ExprKind::ShortTernary { mut value, default } => {
                self.visit(&mut value)?;
                let value_temp = fresh_temp();
                self.hoisted.push(assign(&value_temp, *value, span));
                let falsy = ExprKind::Not(Box::new(variable(&value_temp, span)));
                let (body, default) = operand_into_temp(*default, span);
                self.push_if(Expr::new(falsy, span), body, Vec::new(), span);
                self.effect_seen = false;
                *expr = Expr::new(
                    ExprKind::ShortTernary {
                        value: Box::new(variable(&value_temp, span)),
                        default: Box::new(default),
                    },
                    span,
                );
            }
            ExprKind::BinaryOp { mut left, op, right } => {
                self.visit(&mut left)?;
                let left_temp = fresh_temp();
                self.hoisted.push(assign(&left_temp, *left, span));
                let runs_right = if op == BinOp::And {
                    variable(&left_temp, span)
                } else {
                    Expr::new(ExprKind::Not(Box::new(variable(&left_temp, span))), span)
                };
                let (body, right) = operand_into_temp(*right, span);
                self.push_if(runs_right, body, Vec::new(), span);
                self.effect_seen = false;
                *expr = Expr::new(
                    ExprKind::BinaryOp {
                        left: Box::new(variable(&left_temp, span)),
                        op,
                        right: Box::new(right),
                    },
                    span,
                );
            }
            _ => unreachable!("hoist_conditional is only called on a conditional operator"),
        }
        Ok(())
    }

    /// Pushes `if (condition) { then_body } [else { else_body }]` onto the hoisted statements.
    fn push_if(&mut self, condition: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt>, span: Span) {
        self.hoisted.push(Stmt::new(
            StmtKind::If {
                condition,
                then_body,
                elseif_clauses: Vec::new(),
                else_body: (!else_body.is_empty()).then_some(else_body),
            },
            span,
        ));
    }
}

/// Moves an operand holding an include into `[$t = operand;]`, returning that body and the
/// `$t` read that replaces it; an operand without one stays as it is.
fn operand_into_temp(operand: Expr, span: Span) -> (Vec<Stmt>, Expr) {
    if !expr_has_includes(&operand) {
        return (Vec::new(), operand);
    }
    let temp = fresh_temp();
    (vec![assign(&temp, operand, span)], variable(&temp, span))
}

/// Whether evaluating `expr` can have no observable effect (a literal, variable or constant).
fn is_inert(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::StringLiteral(_)
            | ExprKind::IntLiteral(_)
            | ExprKind::FloatLiteral(_)
            | ExprKind::BoolLiteral(_)
            | ExprKind::Null
            | ExprKind::Variable(_)
            | ExprKind::ConstRef(_)
            | ExprKind::MagicConstant(_)
            | ExprKind::ClassConstant { .. }
    )
}

/// `$name`
fn variable(name: &str, span: Span) -> Expr {
    Expr::new(ExprKind::Variable(name.to_string()), span)
}

/// A fresh compiler-generated local for one hoisted value.
fn fresh_temp() -> String {
    crate::names::generated_local_name(&format!(
        "__elephc_include_value_{}",
        HOIST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

/// `$temp = value;`
fn assign(temp: &str, value: Expr, span: Span) -> Stmt {
    Stmt::new(
        StmtKind::Assign {
            name: temp.to_string(),
            value,
        },
        span,
    )
}

/// The diagnostic for an include that follows an effectful operand of the same statement.
fn after_effect_error(span: Span) -> CompileError {
    CompileError::new(
        span,
        "include/require after a call, assignment or other effectful operand of the same \
         statement is not supported yet: hoisting it would change PHP's evaluation order; \
         assign it to a local variable first",
    )
}
