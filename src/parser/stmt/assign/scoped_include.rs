//! Purpose:
//! Lowers `include`/`require` used as the value of a static-property assignment, such as
//! `self::$x = require __DIR__ . '/data.php';` (symfony polyfills, Composer's
//! `InstalledVersions`).
//!
//! Called from:
//! - `crate::parser::stmt::assign::postfix::try_parse_scoped_property_assignment()`.
//!
//! Key details:
//! - The resolver can only expand a value-position include that is the whole right-hand side
//!   of a plain local `Assign` (or a `return`), so the include is bound to a hidden local first
//!   and the static property is assigned from it. Static-property targets without a dimension
//!   have no side-effecting sub-expressions, so the hoist keeps PHP's evaluation order.
//! - `??=` and compound operators are rejected: hoisting would change when the include runs.
//! - Any other target shape fails at compile time with a clear diagnostic.

use crate::errors::CompileError;
use crate::lexer::{SpannedToken, Token};
use crate::parser::ast::{Expr, ExprKind, Stmt, StmtKind};
use crate::span::Span;

use super::super::expect_semicolon;
use super::compound::AssignmentOperator;

/// Returns true when the token at `pos` is `include`, `include_once`, `require` or
/// `require_once`.
pub(super) fn starts_value_include(tokens: &[SpannedToken], pos: usize) -> bool {
    matches!(
        tokens.get(pos).map(|(token, _)| token),
        Some(Token::Include | Token::IncludeOnce | Token::Require | Token::RequireOnce)
    )
}

/// Parses the include at `*pos` (and the statement's `;`) and lowers
/// `<lhs_expr> <op> include …;` for a static-property target.
pub(super) fn lower_scoped_include_assignment(
    tokens: &[SpannedToken],
    pos: &mut usize,
    lhs_expr: Expr,
    op: AssignmentOperator,
    is_append: bool,
    span: Span,
) -> Result<Stmt, CompileError> {
    let include_span = tokens[*pos].1.span;
    let include = super::super::simple::try_parse_value_include(tokens, pos)?
        .expect("caller checked that an include keyword starts the value");
    expect_semicolon(tokens, pos)?;

    let ExprKind::StaticPropertyAccess { receiver, property } = lhs_expr.kind.clone() else {
        return Err(unsupported(include_span));
    };
    let temp = crate::names::generated_local_name(&format!(
        "__elephc_include_value_{}_{}",
        include_span.line, include_span.col
    ));
    let bind = Stmt::new(
        StmtKind::Assign {
            name: temp.clone(),
            value: include,
        },
        span,
    );
    let value = Expr::new(ExprKind::Variable(temp), span);
    let store = |value: Expr| {
        let kind = if is_append {
            StmtKind::StaticPropertyArrayPush {
                receiver: receiver.clone(),
                property: property.clone(),
                value,
            }
        } else {
            StmtKind::StaticPropertyAssign {
                receiver: receiver.clone(),
                property: property.clone(),
                value,
            }
        };
        Stmt::new(kind, span)
    };

    match op {
        AssignmentOperator::Assign => {
            Ok(Stmt::new(StmtKind::Synthetic(vec![bind, store(value)]), span))
        }
        // `self::$x ??= require …` must run the include only while the property is null.
        // That test cannot be lowered safely yet (the checker types an untyped static
        // property from its writes, so a null check on it can be folded away), so it is a
        // compile error rather than an include that runs every time or never.
        _ => Err(unsupported(include_span)),
    }
}

/// The diagnostic for an include value on a target this lowering does not cover.
fn unsupported(span: Span) -> CompileError {
    CompileError::new(
        span,
        "include/require as the value of this assignment is not supported yet; \
         assign it to a local variable first",
    )
}
