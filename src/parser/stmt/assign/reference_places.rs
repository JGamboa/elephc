//! Purpose:
//! Parses `<place> = &<source>;` for places other than a plain local: an object property
//! (`$this->options = &$options`) or an array element at any depth (`$a[$k]['x'] = &$v`).
//!
//! Called from:
//! - `crate::parser::stmt::assign::postfix::try_parse_postfix_assignment()`.
//!
//! Key details:
//! - The result is always a `StmtKind::RefAssignPlace` whose source is a PLAIN VARIABLE. Any
//!   other reference source (`&$pipes[0]`, `&$this->stream`, `&self::$cache`, `&f()`) is first
//!   bound to a hidden local with the ordinary `$tmp = &<source>` statement, so the checker and
//!   lowering only ever see one source shape and reuse every source rule `RefAssign` has.
//! - PHP evaluates the place's sub-expressions BEFORE the source (`zend_compile_assign_ref`
//!   delay-compiles the target). When the source is hoisted, every index and receiver of the
//!   place that is not already a literal or a plain variable is evaluated into a hidden local
//!   first, so the hoist cannot reorder side effects.
//! - The hidden locals are unset at the end, which only drops their own link: the place keeps
//!   the reference set alive.

use crate::errors::CompileError;
use crate::lexer::{SpannedToken, Token};
use crate::parser::ast::{Expr, ExprKind, Stmt, StmtKind};
use crate::parser::expr::parse_expr;
use crate::span::Span;

use super::super::expect_semicolon;

/// Message for a place this lowering cannot bind by reference.
const UNSUPPORTED_PLACE: &str = "Reference assignment target must be a variable, an object property \
     (`$o->p = &$x`) or an array element (`$a[$k] = &$x`)";

/// Parses the `& <source> ;` tail of `<place> = &<source>;`; `*pos` is on the `&`.
pub(super) fn parse_place_ref_assign(
    tokens: &[SpannedToken],
    pos: &mut usize,
    target: Expr,
    span: Span,
) -> Result<Stmt, CompileError> {
    debug_assert!(matches!(tokens.get(*pos), Some((Token::Ampersand, _))));
    *pos += 1;
    let source = parse_expr(tokens, pos)?;
    crate::parser::expr::reject_dynamic_member_write(&source, span)?;
    crate::parser::expr::reject_nullsafe_write(&source, span)?;
    if !super::compound::is_valid_reference_source(&source.kind) {
        return Err(CompileError::new(
            span,
            "Reference assignment source must be a variable, array/property element, or a by-reference call",
        ));
    }
    expect_semicolon(tokens, pos)?;
    lower_place_ref_assign(target, source, span)
}

/// Builds the statement(s) for `<target> = &<source>` once both sides are parsed.
pub(crate) fn lower_place_ref_assign(
    target: Expr,
    source: Expr,
    span: Span,
) -> Result<Stmt, CompileError> {
    if !is_reference_place(&target) {
        return Err(CompileError::new(span, UNSUPPORTED_PLACE));
    }
    if matches!(source.kind, ExprKind::Variable(_)) {
        return Ok(Stmt::new(StmtKind::RefAssignPlace { target, source }, span));
    }
    let mut prelude = Vec::new();
    let mut temps = Vec::new();
    let target = stabilize_place(target, span, &mut prelude, &mut temps);
    let cell = crate::names::generated_local_name(&format!(
        "__elephc_ref_place_{}_{}",
        span.line, span.col
    ));
    prelude.push(Stmt::new(
        StmtKind::RefAssign {
            target: cell.clone(),
            source,
        },
        span,
    ));
    prelude.push(Stmt::new(
        StmtKind::RefAssignPlace {
            target,
            source: Expr::new(ExprKind::Variable(cell.clone()), span),
        },
        span,
    ));
    temps.push(cell);
    let args = temps
        .into_iter()
        .map(|name| Expr::new(ExprKind::Variable(name), span))
        .collect();
    prelude.push(Stmt::new(
        StmtKind::ExprStmt(Expr::new(
            ExprKind::FunctionCall {
                name: "unset".into(),
                args,
            },
            span,
        )),
        span,
    ));
    Ok(Stmt::new(StmtKind::Synthetic(prelude), span))
}

/// Returns whether `expr` is a place `RefAssignPlace` can bind: a property of any receiver, or
/// an element whose container chain bottoms out in a variable, `$this`, or a property.
pub(crate) fn is_reference_place(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::PropertyAccess { .. } => true,
        ExprKind::ArrayAccess { array, .. } => is_reference_container(array),
        _ => false,
    }
}

/// Returns whether `expr` can hold an element reference place.
fn is_reference_container(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Variable(_) | ExprKind::PropertyAccess { .. } => true,
        ExprKind::ArrayAccess { array, .. } => is_reference_container(array),
        _ => false,
    }
}

/// Evaluates every index and receiver of `place` that could observe or cause a side effect into
/// a hidden local, root first, so the later source binding cannot run before them.
fn stabilize_place(place: Expr, span: Span, prelude: &mut Vec<Stmt>, temps: &mut Vec<String>) -> Expr {
    let place_span = place.span;
    match place.kind {
        ExprKind::ArrayAccess { array, index } => {
            let array = stabilize_place(*array, span, prelude, temps);
            let index = stabilize_operand(*index, span, prelude, temps);
            Expr::new(
                ExprKind::ArrayAccess {
                    array: Box::new(array),
                    index: Box::new(index),
                },
                place_span,
            )
        }
        ExprKind::PropertyAccess { object, property } => {
            let object = match object.kind {
                ExprKind::Variable(_) | ExprKind::This => *object,
                ExprKind::PropertyAccess { .. } | ExprKind::ArrayAccess { .. } => {
                    stabilize_place(*object, span, prelude, temps)
                }
                _ => stabilize_operand(*object, span, prelude, temps),
            };
            Expr::new(
                ExprKind::PropertyAccess {
                    object: Box::new(object),
                    property,
                },
                place_span,
            )
        }
        kind => Expr::new(kind, place_span),
    }
}

/// Moves one operand into a hidden local unless it is already free of side effects.
fn stabilize_operand(operand: Expr, span: Span, prelude: &mut Vec<Stmt>, temps: &mut Vec<String>) -> Expr {
    if matches!(
        operand.kind,
        ExprKind::IntLiteral(_)
            | ExprKind::StringLiteral(_)
            | ExprKind::BoolLiteral(_)
            | ExprKind::Null
            | ExprKind::Variable(_)
            | ExprKind::This
    ) {
        return operand;
    }
    let name = crate::names::generated_local_name(&format!(
        "__elephc_ref_place_key_{}_{}_{}",
        span.line,
        span.col,
        temps.len()
    ));
    let operand_span = operand.span;
    prelude.push(Stmt::new(
        StmtKind::Assign {
            name: name.clone(),
            value: operand,
        },
        span,
    ));
    temps.push(name.clone());
    Expr::new(ExprKind::Variable(name), operand_span)
}
