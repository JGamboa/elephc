//! Purpose:
//! Gives every override family of a dispatched method one shared EIR call ABI.
//!
//! Called from:
//! - `crate::ir_lower::program::metadata::populate_metadata()`, after per-class normalization.
//!
//! Key details:
//! - A vtable call is compiled against the RECEIVER's declared class signature, but it jumps to
//!   whichever override the object carries. When an override passes or returns a value in another
//!   representation (an untyped `$v` boxed as Mixed against an `int $v` slot, a body returning a
//!   string where the abstract placeholder said int), the caller read a pointer as an integer.
//! - Families come from `crate::types::override_families`, the grouping the checker also uses.
//!   A parameter or return whose representation differs inside a family becomes Mixed in every
//!   member; an abstract placeholder takes the representation all implementations agree on.
//! - Method bodies and their callers both read these signatures, so they stay in agreement.
//! - Widening that would change PHP semantics (a declared `float`, which coerces an int argument
//!   or result), a by-reference return and a generator are refused with a diagnostic instead.

use std::collections::HashMap;

use crate::span::Span;
use crate::types::override_families::{self, FamilyMember};
use crate::types::{ClassInfo, PhpType};

/// Unifies parameter and return ABIs across every instance and static override family.
pub(super) fn unify_override_family_abis(classes: &mut HashMap<String, ClassInfo>) {
    for is_static in [false, true] {
        for (method_key, members) in override_families::collect(classes, is_static) {
            unify_family(classes, &method_key, &members, is_static);
        }
    }
}

/// Rewrites one family so every member shares each parameter and the return representation.
fn unify_family(
    classes: &mut HashMap<String, ClassInfo>,
    method_key: &str,
    members: &[FamilyMember],
    is_static: bool,
) {
    let first = &members[0].sig;
    if members.iter().any(|member| {
        let sig = &member.sig;
        sig.params.len() != first.params.len()
            || sig.ref_params != first.ref_params
            || sig.variadic.is_some()
            || crate::func_args::sig_collects_surplus_args(sig)
            || crate::func_args::sig_has_hidden_argc_param(sig)
    }) {
        return;
    }
    let widened_params: Vec<usize> = (0..first.params.len())
        .filter(|&index| {
            let ty = &first.params[index].1;
            !first.ref_params.get(index).copied().unwrap_or(false)
                && members
                    .iter()
                    .any(|member| !override_families::same_abi(&member.sig.params[index].1, ty))
        })
        .collect();
    let Some(return_type) = override_families::agreed_return(members) else {
        return;
    };
    let return_widened = return_type == PhpType::Mixed;
    if let Some(span) = refused_widening(members, &widened_params, return_widened) {
        crate::ir_lower::diagnostics::refuse(
            span,
            &format!(
                "Overrides of method '{}' pass or return values in incompatible representations \
                 (by-reference, generator or declared float); elephc cannot dispatch them through \
                 one virtual call ABI yet",
                method_key
            ),
        );
        return;
    }
    for member in members {
        let Some(sig) = classes
            .get_mut(&member.class_name)
            .and_then(|info| override_families::methods_mut(info, is_static).get_mut(method_key))
        else {
            continue;
        };
        for &index in &widened_params {
            sig.params[index].1 = PhpType::Mixed;
        }
        sig.return_type = return_type.clone();
    }
}

/// Returns the span of a member whose ABI cannot be widened without changing PHP semantics.
fn refused_widening(
    members: &[FamilyMember],
    widened_params: &[usize],
    return_widened: bool,
) -> Option<Span> {
    members.iter().find_map(|member| {
        let span = member.body_span?;
        let sig = &member.sig;
        let float_param = widened_params.iter().any(|&index| {
            sig.declared_params.get(index).copied().unwrap_or(false)
                && sig.params[index].1.codegen_repr() == PhpType::Float
        });
        let unsafe_return = return_widened
            && sig.return_type.codegen_repr() != PhpType::Mixed
            && (sig.by_ref_return
                || sig.is_generator
                || (sig.declared_return && sig.return_type.codegen_repr() == PhpType::Float));
        (float_param || unsafe_return).then_some(span)
    })
}
