//! Purpose:
//! Lazy match-expression lowering.
//!
//! Called from:
//! - `crate::ir_lower::expr`.
//!
//! Key details:
//! - Preserves source-order evaluation, EIR typing, effects, and ownership contracts.

use super::*;

/// Lowers a match expression with lazy arm-result evaluation.
pub(super) fn lower_match(
    ctx: &mut LoweringContext<'_, '_>,
    subject: &Expr,
    arms: &[(Vec<Expr>, Expr)],
    default: Option<&Expr>,
    expr: &Expr,
) -> LoweredValue {
    let subject_expr = subject;
    let subject = lower_expr(ctx, subject);
    // A match without `default` throws PHP's catchable `UnhandledMatchError` naming the subject;
    // its message is built from a synthetic local holding the already-evaluated subject.
    let unhandled_subject = default.is_none().then(|| {
        let local_type = normalize_value_php_type(ctx.builder.value_php_type(subject.value));
        let temp = ctx.declare_synthetic_php_local(local_type.clone());
        let stored = crate::ir_lower::ownership::acquire_if_refcounted(ctx, subject, Some(subject_expr.span));
        ctx.store_local(&temp, stored, local_type, Some(subject_expr.span));
        temp
    });
    let merge = ctx.builder.create_named_block("match.merge", Vec::new());
    // Every arm result is parked and the merge temp is typed from the lowered arm values (see
    // `PendingBranchValue`), so an arm the pre-lowering estimate cannot name keeps its value.
    let mut pending = Vec::new();

    for (conditions, result) in arms {
        let result_block = ctx.builder.create_named_block("match.result", Vec::new());
        let mut fallthrough = ctx.builder.insertion_block();
        for condition in conditions {
            let next_test = ctx.builder.create_named_block("match.next", Vec::new());
            let condition = lower_expr(ctx, condition);
            let matched = ctx.emit_value(
                Op::StrictEq,
                vec![subject.value, condition.value],
                None,
                PhpType::Bool,
                Op::StrictEq.default_effects(),
                Some(expr.span),
            );
            ctx.builder.terminate(Terminator::CondBr {
                cond: matched.value,
                then_target: result_block,
                then_args: Vec::new(),
                else_target: next_test,
                else_args: Vec::new(),
            });
            ctx.builder.position_at_end(next_test);
            fallthrough = Some(next_test);
        }
        ctx.builder.position_at_end(result_block);
        pending.extend(lower_pending_branch(ctx, result));
        if let Some(fallthrough) = fallthrough {
            ctx.builder.position_at_end(fallthrough);
        }
    }
    if let Some(default) = default {
        pending.extend(lower_pending_branch(ctx, default));
    } else if let Some(temp) = unhandled_subject.as_deref() {
        if !ctx.builder.insertion_block_is_terminated() {
            let throw = unhandled_match_error_throw(temp, expr.span);
            lower_expr(ctx, &throw);
            if !ctx.builder.insertion_block_is_terminated() {
                ctx.builder.terminate(Terminator::Unreachable);
            }
        }
    }
    let (temp_name, _) = store_pending_branches(ctx, pending, merge, Some(expr), None, expr.span);
    ctx.builder.position_at_end(merge);
    // A matched subject is released here, as PHP releases the temporary, rather than at frame exit.
    if let Some(temp) = unhandled_subject.as_deref() {
        let null = lower_null(ctx, expr);
        ctx.unset_local(temp, null, None);
    }
    take_owned_temp(ctx, &temp_name, expr.span)
}

/// Builds `throw new UnhandledMatchError("Unhandled match case " . <subject>)` over the local
/// `temp`, rendering the subject the way PHP does: an int or float as its string form, a string
/// in single quotes, `true`/`false`, `NULL`, and `of type <class>` / `of type array` otherwise.
fn unhandled_match_error_throw(temp: &str, span: Span) -> Expr {
    let e = |kind: ExprKind| Expr::new(kind, span);
    let var = || e(ExprKind::Variable(temp.to_string()));
    let lit = |text: &str| e(ExprKind::StringLiteral(text.to_string()));
    let call = |function: &str| {
        e(ExprKind::FunctionCall { name: Name::unqualified(function), args: vec![var()] })
    };
    let concat = |left: Expr, right: Expr| {
        e(ExprKind::BinaryOp { left: Box::new(left), op: BinOp::Concat, right: Box::new(right) })
    };
    let ternary = |condition: Expr, then_expr: Expr, else_expr: Expr| {
        e(ExprKind::Ternary {
            condition: Box::new(condition),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
        })
    };
    let numeric = e(ExprKind::BinaryOp {
        left: Box::new(call("is_int")),
        op: BinOp::Or,
        right: Box::new(call("is_float")),
    });
    let rendered = ternary(
        call("is_string"),
        concat(concat(lit("'"), var()), lit("'")),
        ternary(
            numeric,
            e(ExprKind::Cast { target: CastType::String, expr: Box::new(var()) }),
            ternary(
                call("is_bool"),
                ternary(var(), lit("true"), lit("false")),
                ternary(
                    call("is_null"),
                    lit("NULL"),
                    ternary(
                        call("is_object"),
                        concat(lit("of type "), call("get_class")),
                        lit("of type array"),
                    ),
                ),
            ),
        ),
    );
    e(ExprKind::Throw(Box::new(e(ExprKind::NewObject {
        class_name: Name::from("UnhandledMatchError".to_string()),
        args: vec![concat(lit("Unhandled match case "), rendered)],
    }))))
}
