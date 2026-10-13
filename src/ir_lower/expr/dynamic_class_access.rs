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
        "__elephc_dynamic_static_call" => return lower_dynamic_static_call(ctx, args, expr),
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


/// Expands `__elephc_dynamic_static_call($recv, 'm', ...$args)` — PHP's `$recv::m(...)`.
///
/// Like the member reads, the receiver is evaluated once and reduced to its runtime class name,
/// then matched against every AOT class that declares or inherits `m`. Each arm lowers the
/// ordinary call, after the class is resolved, as PHP orders it (the arguments are evaluated
/// inside the matched arm only):
/// - a static `m` becomes `C::m(...)`, so late static binding sees `C`;
/// - an instance `m` is PHP's "call statically": it runs on `$this` when the current method has
///   one that is an instance of `C` (exact for a `$this` receiver; for another receiver only when
///   no subclass of `C` overrides `m`, otherwise a catchable "not supported" `Error`), and
///   throws PHP's `Non-static method C::m() cannot be called statically` `Error` otherwise.
/// An unmatched name throws `Call to undefined method X::m()`.
fn lower_dynamic_static_call(
    ctx: &mut LoweringContext<'_, '_>,
    args: &[Expr],
    expr: &Expr,
) -> Option<LoweredValue> {
    let [receiver, method, call_args @ ..] = args else {
        return None;
    };
    let ExprKind::StringLiteral(method) = &method.kind else {
        return None;
    };
    let span = expr.span;
    let receiver_is_this = matches!(receiver.kind, ExprKind::This);
    let receiver_value = lower_expr(ctx, receiver);
    let receiver_type = normalize_value_php_type(ctx.builder.value_php_type(receiver_value.value));
    let receiver_local = ctx.declare_synthetic_php_local(receiver_type.clone());
    store_value_into_temp(ctx, &receiver_local, receiver_type.clone(), receiver_value, span);
    let receiver_var = Expr::new(ExprKind::Variable(receiver_local), span);
    let class_name_expr = runtime_class_name_expr(&receiver_type, receiver_var, span);
    let name_local = ctx.declare_synthetic_php_local(PhpType::Str);
    store_expr_into_temp(ctx, &name_local, PhpType::Str, &class_name_expr, span);

    let method_key = php_symbol_key(method);
    let mut by_ref_candidate = None;
    let mut candidates: Vec<(String, bool)> = ctx
        .classes
        .iter()
        .filter_map(|(class_name, info)| {
            let (sig, is_static) = match info.static_methods.get(&method_key) {
                Some(sig) => (sig, true),
                None => (info.methods.get(&method_key)?, false),
            };
            if sig.ref_params.iter().any(|by_ref| *by_ref) {
                by_ref_candidate.get_or_insert_with(|| class_name.clone());
            }
            Some((class_name.clone(), is_static))
        })
        .collect();
    candidates.sort();
    if let Some(class_name) = by_ref_candidate {
        // The checker only sees the by-value primitive, so a by-reference argument would be
        // passed as a copy: refuse instead of silently dropping the write-back.
        crate::ir_lower::diagnostics::refuse(
            span,
            &format!(
                "Calling `{class_name}::{method}()` through a dynamic class (`$class::{method}()`) is not supported because it takes a by-reference parameter; name the class"
            ),
        );
    }
    let has_this = ctx.has_local_slot("this");

    let result_temp = ctx.declare_owned_hidden_temp(PhpType::Mixed);
    let split_initialized = ctx.initialized_slots_snapshot();
    let merge = ctx.builder.create_named_block("dyn_static_call.merge", Vec::new());
    let mut arms = Vec::new();
    for (class_name, is_static) in candidates {
        let arm = ctx.builder.create_named_block("dyn_static_call.arm", Vec::new());
        let skip = ctx.builder.create_named_block("dyn_static_call.skip", Vec::new());
        let matches = lower_expr(ctx, &class_name_matches_expr(&name_local, &class_name, span));
        conditional_branch(ctx, matches.value, arm, skip);
        arms.push((arm, class_name, is_static));
        ctx.builder.position_at_end(skip);
    }
    let undefined = string_concat_expr(
        vec![
            literal("Call to undefined method ", span),
            Expr::new(ExprKind::Variable(name_local.clone()), span),
            literal(&format!("::{method}()"), span),
        ],
        span,
    );
    throw_error_and_merge(ctx, undefined, &result_temp, merge, span);

    for (arm, class_name, is_static) in arms {
        ctx.builder.position_at_end(arm);
        ctx.restore_initialized_slots(split_initialized.clone());
        if is_static {
            let call = Expr::new(
                ExprKind::StaticMethodCall {
                    receiver: StaticReceiver::Named(Name::from(class_name)),
                    method: method.clone(),
                    args: call_args.to_vec(),
                },
                span,
            );
            store_expr_into_temp(ctx, &result_temp, PhpType::Mixed, &call, span);
            branch_to(ctx, merge);
            continue;
        }
        let non_static = literal(
            &format!("Non-static method {class_name}::{method}() cannot be called statically"),
            span,
        );
        if !has_this {
            throw_error_and_merge(ctx, non_static, &result_temp, merge, span);
            continue;
        }
        // `$this instanceof C`: PHP runs `C::m()` on `$this`.
        let this_is_c = Expr::new(
            ExprKind::InstanceOf {
                value: Box::new(Expr::new(ExprKind::This, span)),
                target: InstanceOfTarget::Name(Name::from(class_name.clone())),
            },
            span,
        );
        let on_this = ctx.builder.create_named_block("dyn_static_call.this", Vec::new());
        let not_this = ctx.builder.create_named_block("dyn_static_call.no_this", Vec::new());
        let this_is_c = lower_expr(ctx, &this_is_c);
        conditional_branch(ctx, this_is_c.value, on_this, not_this);
        ctx.builder.position_at_end(not_this);
        throw_error_and_merge(ctx, non_static, &result_temp, merge, span);
        ctx.builder.position_at_end(on_this);
        if receiver_is_this || !subclass_overrides_method(ctx, &class_name, &method_key) {
            let call = Expr::new(
                ExprKind::MethodCall {
                    object: Box::new(Expr::new(ExprKind::This, span)),
                    method: method.clone(),
                    args: call_args.to_vec(),
                },
                span,
            );
            store_expr_into_temp(ctx, &result_temp, PhpType::Mixed, &call, span);
            branch_to(ctx, merge);
        } else {
            let unsupported = literal(
                &format!(
                    "elephc: calling overridden instance method {class_name}::{method}() through `$object::{method}()` is not supported"
                ),
                span,
            );
            throw_error_and_merge(ctx, unsupported, &result_temp, merge, span);
        }
    }
    ctx.builder.position_at_end(merge);
    Some(take_owned_temp(ctx, &result_temp, span))
}

/// Returns whether any subclass of `class_name` declares its own `method_key`.
fn subclass_overrides_method(ctx: &LoweringContext<'_, '_>, class_name: &str, method_key: &str) -> bool {
    let declaring = ctx
        .classes
        .get(class_name)
        .and_then(|info| info.method_declaring_classes.get(method_key));
    ctx.classes.iter().any(|(other, info)| {
        other != class_name
            && super::property_access::class_extends_class(ctx, other, class_name)
            && info.method_declaring_classes.get(method_key) != declaring
    })
}

/// Builds a string literal expression.
fn literal(text: &str, span: Span) -> Expr {
    Expr::new(ExprKind::StringLiteral(text.to_string()), span)
}

/// Folds `parts` into one left-associated `.` concatenation.
fn string_concat_expr(parts: Vec<Expr>, span: Span) -> Expr {
    parts
        .into_iter()
        .reduce(|left, right| {
            Expr::new(
                ExprKind::BinaryOp { left: Box::new(left), op: BinOp::Concat, right: Box::new(right) },
                span,
            )
        })
        .expect("at least one part")
}

/// Terminates the current block with a `CondBr` on `cond`.
fn conditional_branch(ctx: &mut LoweringContext<'_, '_>, cond: ValueId, then_target: BlockId, else_target: BlockId) {
    ctx.builder.terminate(Terminator::CondBr {
        cond,
        then_target,
        then_args: Vec::new(),
        else_target,
        else_args: Vec::new(),
    });
}

/// Throws `new Error($message)` and keeps the unreachable fallthrough well formed.
fn throw_error_and_merge(
    ctx: &mut LoweringContext<'_, '_>,
    message: Expr,
    result_temp: &str,
    merge: BlockId,
    span: Span,
) {
    let error = Expr::new(
        ExprKind::NewObject { class_name: Name::unqualified("Error"), args: vec![message] },
        span,
    );
    let error = lower_expr(ctx, &error);
    ctx.emit_void(
        Op::ThrowException,
        vec![error.value],
        None,
        Op::ThrowException.default_effects(),
        Some(span),
    );
    store_expr_into_temp(ctx, result_temp, PhpType::Mixed, &Expr::new(ExprKind::Null, span), span);
    branch_to(ctx, merge);
}
