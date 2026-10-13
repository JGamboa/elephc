//! Purpose:
//! Keeps the checker's view of a dispatched method's return type consistent across overrides.
//!
//! Called from:
//! - `Checker::type_check_methods_until_stable` after every method-body pass.
//! - `check_types_on_compiler_stack` after the final `propagate_abstract_return_types`.
//!
//! Key details:
//! - A call through `$shape->tag()` is typed from `Shape`'s signature, but it runs whichever
//!   override the object carries. An abstract placeholder used to take the LAST child's return
//!   type, so a caller (or a method returning that call) inferred `int` for a value one sibling
//!   returned as an array, and the lowered code converted it.
//! - Within a family (`crate::types::override_families`): a placeholder takes the type every
//!   implementation agrees on; when they disagree, it and every inferred (undeclared) return take
//!   the family fold (`family_return`): a union naming every member, or `mixed`. A declared
//!   return is the PHP contract and is never touched; EIR lowering boxes it into the family's
//!   shared call ABI instead.
//! - Running inside the fixed-point loop lets a method returning such a call re-infer from it.

use super::Checker;
use crate::types::override_families;
use crate::types::PhpType;

impl Checker {
    /// Applies the family return rule to every instance and static override family.
    pub(super) fn unify_override_family_returns(&mut self) {
        for is_static in [false, true] {
            for (method_key, members) in override_families::collect(&self.classes, is_static) {
                let Some(agreed) = family_return(&members) else {
                    continue;
                };
                for member in &members {
                    let sig = &member.sig;
                    let return_type = if member.placeholder {
                        agreed.clone()
                    } else if !sig.declared_return
                        && !override_families::same_abi(&sig.return_type, &agreed)
                        && !sig.by_ref_return
                        && !sig.is_generator
                    {
                        self.override_family_returns.insert(
                            (member.class_name.clone(), method_key.clone(), is_static),
                            agreed.clone(),
                        );
                        agreed.clone()
                    } else {
                        continue;
                    };
                    if let Some(sig) = self.classes.get_mut(&member.class_name).and_then(|info| {
                        override_families::methods_mut(info, is_static).get_mut(&method_key)
                    }) {
                        sig.return_type = return_type;
                    }
                }
            }
        }
    }
}

/// Folds the implementations' returns the way one body with all of those `return`s would be.
///
/// The fold is kept only when it carries every member unchanged: a union naming each of them
/// (`?A` from `A` and `null`), which the call ABI boxes anyway. A scalar the fold picked that some
/// member does not travel in (`int` and `array` fold to `string`, `int`, `string` and `null` to
/// `?string`) would convert that member's value, so the family return becomes `mixed`.
fn family_return(members: &[override_families::FamilyMember]) -> Option<PhpType> {
    let mut concrete = members
        .iter()
        .filter(|member| !member.placeholder)
        .map(|member| &member.sig.return_type);
    let first = concrete.next()?.clone();
    let mut folded = first;
    let mut all = vec![folded.clone()];
    for ty in concrete {
        folded = Checker::wider_type(&folded, ty);
        all.push(ty.clone());
    }
    if all.iter().all(|ty| covers(&folded, ty)) {
        Some(folded)
    } else {
        Some(PhpType::Mixed)
    }
}

/// Returns whether a value typed `ty` travels unchanged through a result typed `folded`.
pub(super) fn covers(folded: &PhpType, ty: &PhpType) -> bool {
    let member_of = |members: &[PhpType], ty: &PhpType| {
        members.iter().any(|member| override_families::same_abi(member, ty))
    };
    match (folded, ty) {
        (PhpType::Mixed, _) => true,
        (PhpType::Union(members), PhpType::Union(tys)) => {
            tys.iter().all(|ty| member_of(members, ty))
        }
        (PhpType::Union(members), ty) => member_of(members, ty),
        _ => override_families::same_abi(folded, ty),
    }
}
