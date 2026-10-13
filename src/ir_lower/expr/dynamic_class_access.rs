//! Purpose:
//! Lowers dynamic-receiver class member reads — `$class::NAME` and `$class::$prop` — that the
//! parser desugared into the internal `__elephc_dynamic_class_constant` /
//! `__elephc_dynamic_static_property` primitives.
//!
//! Called from:
//! - `crate::ir_lower::expr::function_calls::lower_function_call()`, before registry lowering.
//!
//! Key details:
//! - The receiver is evaluated exactly once into a synthetic PHP local, then reduced to its
//!   runtime class name: `get_class()` for an object, `ltrim($name, '\\')` for a string, and a
//!   runtime `is_object()` split when the static type is `mixed` or a union.
//! - The name is matched case-insensitively (`strcasecmp`, like PHP class lookup) against every
//!   AOT class-like that resolves the member, own or inherited. Each arm lowers the ordinary
//!   named-receiver node (`C::NAME` / `C::$prop`), so values, enum cases, inheritance and
//!   interface constants come from the same code path as statically named access.
//! - A name that matches no candidate throws a catchable `Error`. PHP distinguishes an unknown
//!   class (`Class "X" not found`) from a missing member; this path reports the member form for
//!   both because a per-site table of every known class would cost code size on the hot path.
//! - Member visibility is not enforced: a private/protected constant or static property of the
//!   matched class is read instead of throwing PHP's visibility `Error`.

use super::*;

/// Which member family a dynamic-receiver read targets.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DynamicMember {
    Constant,
    StaticProperty,
}

/// Expands a direct call to one of the dynamic class-member primitives, or returns `None` when
/// `name` is any other function so the caller continues with ordinary call lowering.
pub(super) fn lower_dynamic_class_member_call(
    ctx: &mut LoweringContext<'_, '_>,
    name: &Name,
    args: &[Expr],
    expr: &Expr,
) -> Option<LoweredValue> {
    let kind = match php_symbol_key(name.as_str().trim_start_matches('\\')).as_str() {
        "__elephc_dynamic_class_constant" => DynamicMember::Constant,
        "__elephc_dynamic_static_property" => DynamicMember::StaticProperty,
        _ => return None,
    };
    let [receiver, member] = args else {
        return None;
    };
    let ExprKind::StringLiteral(member) = &member.kind else {
        return None;
    };
    let span = expr.span;

    // Evaluate the receiver once; later reads go through the synthetic local.
    let receiver_value = lower_expr(ctx, receiver);
    let receiver_type = normalize_value_php_type(ctx.builder.value_php_type(receiver_value.value));
    let receiver_local = ctx.declare_synthetic_php_local(receiver_type.clone());
    store_value_into_temp(ctx, &receiver_local, receiver_type.clone(), receiver_value, span);
    let receiver_var = Expr::new(ExprKind::Variable(receiver_local), span);

    let class_name_expr = runtime_class_name_expr(&receiver_type, receiver_var, span);
    let name_local = ctx.declare_synthetic_php_local(PhpType::Str);
    store_expr_into_temp(ctx, &name_local, PhpType::Str, &class_name_expr, span);

    let candidates = dynamic_member_candidates(ctx, kind, member);
    // The checker types the primitive as its declared `mixed`; the lowered value must agree,
    // or consumers that trust the checked type misread it (see the design notes).
    let result_type = PhpType::Mixed;
    let result_temp = ctx.declare_owned_hidden_temp(result_type.clone());
    let split_initialized = ctx.initialized_slots_snapshot();
    let merge = ctx.builder.create_named_block("dyn_class_member.merge", Vec::new());

    let mut arms = Vec::new();
    for class_name in candidates {
        let arm = ctx.builder.create_named_block("dyn_class_member.arm", Vec::new());
        let skip = ctx.builder.create_named_block("dyn_class_member.skip", Vec::new());
        let matches = lower_expr(ctx, &class_name_matches_expr(&name_local, &class_name, span));
        ctx.builder.terminate(Terminator::CondBr {
            cond: matches.value,
            then_target: arm,
            then_args: Vec::new(),
            else_target: skip,
            else_args: Vec::new(),
        });
        arms.push((arm, class_name));
        ctx.builder.position_at_end(skip);
    }

    // No candidate matched: PHP raises a catchable Error.
    let error = Expr::new(
        ExprKind::NewObject {
            class_name: Name::unqualified("Error"),
            args: vec![missing_member_message_expr(kind, &name_local, member, span)],
        },
        span,
    );
    let error = lower_expr(ctx, &error);
    // Expression-position throw, exactly like `lower_throw_access_error_expr`: the opcode
    // unwinds, and the null store below only keeps the (unreachable) fallthrough well formed.
    ctx.emit_void(
        Op::ThrowException,
        vec![error.value],
        None,
        Op::ThrowException.default_effects(),
        Some(span),
    );
    store_expr_into_temp(ctx, &result_temp, result_type.clone(), &Expr::new(ExprKind::Null, span), span);
    branch_to(ctx, merge);

    for (arm, class_name) in arms {
        ctx.builder.position_at_end(arm);
        ctx.restore_initialized_slots(split_initialized.clone());
        let receiver = StaticReceiver::Named(Name::from(class_name));
        let read = match kind {
            DynamicMember::Constant => Expr::new(
                ExprKind::ScopedConstantAccess { receiver, name: member.clone() },
                span,
            ),
            DynamicMember::StaticProperty => Expr::new(
                ExprKind::StaticPropertyAccess { receiver, property: member.clone() },
                span,
            ),
        };
        store_expr_into_temp(ctx, &result_temp, result_type.clone(), &read, span);
        branch_to(ctx, merge);
    }
    ctx.builder.position_at_end(merge);
    Some(take_owned_temp(ctx, &result_temp, span))
}

/// Builds the expression that yields the receiver's runtime class name as a string.
///
/// PHP strips one leading `\` from a class-name string (`'\Foo'::X` works); `get_class()`
/// never returns one. A `mixed`/union receiver splits on `is_object()` at runtime.
fn runtime_class_name_expr(receiver_type: &PhpType, receiver: Expr, span: Span) -> Expr {
    let call = |name: &str, args: Vec<Expr>| {
        Expr::new(ExprKind::FunctionCall { name: Name::unqualified(name), args }, span)
    };
    let backslash = || Expr::new(ExprKind::StringLiteral("\\".to_string()), span);
    match receiver_type.codegen_repr() {
        PhpType::Object(_) => call("get_class", vec![receiver]),
        PhpType::Str => call("ltrim", vec![receiver, backslash()]),
        _ => Expr::new(
            ExprKind::Ternary {
                condition: Box::new(call("is_object", vec![receiver.clone()])),
                then_expr: Box::new(call("get_class", vec![receiver.clone()])),
                else_expr: Box::new(call(
                    "ltrim",
                    vec![
                        Expr::new(
                            ExprKind::Cast { target: CastType::String, expr: Box::new(receiver) },
                            span,
                        ),
                        backslash(),
                    ],
                )),
            },
            span,
        ),
    }
}

/// Builds `strcasecmp($name, 'Class') === 0`.
fn class_name_matches_expr(name_local: &str, class_name: &str, span: Span) -> Expr {
    let compare = Expr::new(
        ExprKind::FunctionCall {
            name: Name::unqualified("strcasecmp"),
            args: vec![
                Expr::new(ExprKind::Variable(name_local.to_string()), span),
                Expr::new(ExprKind::StringLiteral(class_name.to_string()), span),
            ],
        },
        span,
    );
    Expr::new(
        ExprKind::BinaryOp {
            left: Box::new(compare),
            op: BinOp::StrictEq,
            right: Box::new(Expr::new(ExprKind::IntLiteral(0), span)),
        },
        span,
    )
}

/// Builds PHP's missing-member message with the runtime class name spliced in.
fn missing_member_message_expr(
    kind: DynamicMember,
    name_local: &str,
    member: &str,
    span: Span,
) -> Expr {
    let (prefix, suffix) = match kind {
        DynamicMember::Constant => ("Undefined constant ", format!("::{member}")),
        DynamicMember::StaticProperty => {
            ("Access to undeclared static property ", format!("::${member}"))
        }
    };
    let concat = |left: Expr, right: Expr| {
        Expr::new(
            ExprKind::BinaryOp { left: Box::new(left), op: BinOp::Concat, right: Box::new(right) },
            span,
        )
    };
    concat(
        concat(
            Expr::new(ExprKind::StringLiteral(prefix.to_string()), span),
            Expr::new(ExprKind::Variable(name_local.to_string()), span),
        ),
        Expr::new(ExprKind::StringLiteral(suffix), span),
    )
}

/// Returns, in a deterministic order, every AOT class-like that resolves `member`.
///
/// Constants: classes (own, inherited, or from an implemented interface), interfaces, and
/// enums (cases or constants). Static properties: classes whose static-property table (own
/// or inherited) names `member`.
fn dynamic_member_candidates(
    ctx: &LoweringContext<'_, '_>,
    kind: DynamicMember,
    member: &str,
) -> Vec<String> {
    let mut candidates: Vec<String> = match kind {
        DynamicMember::Constant => ctx
            .classes
            .keys()
            .chain(ctx.interfaces.keys())
            .filter(|class_name| ctx.scoped_constant_value(class_name, member).is_some())
            .chain(ctx.enums.iter().filter_map(|(enum_name, info)| {
                info.cases.iter().any(|case| case.name == member).then_some(enum_name)
            }))
            .cloned()
            .collect(),
        DynamicMember::StaticProperty => ctx
            .classes
            .iter()
            .filter(|(_, info)| info.static_properties.iter().any(|(name, _)| name == member))
            .map(|(class_name, _)| class_name.clone())
            .collect(),
    };
    candidates.sort();
    candidates.dedup();
    candidates
}

