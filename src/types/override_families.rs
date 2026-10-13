//! Purpose:
//! Groups dispatched methods into override families and answers which members share one ABI.
//!
//! Called from:
//! - `Checker::unify_override_family_returns` in `crate::types::checker::override_returns`,
//!   so a caller's inferred type agrees with whatever override the object carries.
//! - `crate::ir_lower::program::virtual_abi`, which gives each family one EIR call ABI.
//!
//! Key details:
//! - A vtable call is typed and compiled against the RECEIVER's declared class, but it runs the
//!   override the object carries. Both halves must reach the same family answer, so the grouping
//!   lives here rather than in either caller.
//! - A family is every class that declares or inherits a non-private, slot-dispatched method from
//!   the same topmost ancestor. Constructors, magic hooks and instantiated generic methods keep
//!   their own contracts and are never grouped.
//! - An abstract declaration without a declared return is a PLACEHOLDER: its recorded return type
//!   is not a contract, and it takes whatever the implementations agree on.

use std::collections::{BTreeMap, HashMap};

use crate::names::php_symbol_key;
use crate::parser::ast::Visibility;
use crate::span::Span;
use crate::types::{ClassInfo, FunctionSig, PhpType};

/// One member of an override family, as seen through its own class.
pub(crate) struct FamilyMember {
    pub class_name: String,
    pub sig: FunctionSig,
    /// Abstract here, with no declared return to stand for.
    pub placeholder: bool,
    /// Declaration span of the body this member dispatches to; `None` for an abstract member.
    pub body_span: Option<Span>,
}

/// Returns every override family with more than one member, keyed by method, in stable order.
///
/// A family reaching a builtin or synthetic body is left out: that body keeps its fixed contract.
pub(crate) fn collect(
    classes: &HashMap<String, ClassInfo>,
    is_static: bool,
) -> Vec<(String, Vec<FamilyMember>)> {
    let mut families: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (class_name, info) in classes {
        for method_key in methods(info, is_static).keys() {
            if !is_dispatched(info, method_key, is_static) {
                continue;
            }
            let mut root = class_name.clone();
            while let Some(parent) = classes.get(&root).and_then(|info| info.parent.clone()) {
                match classes.get(&parent) {
                    Some(parent_info)
                        if methods(parent_info, is_static).contains_key(method_key)
                            && is_dispatched(parent_info, method_key, is_static) =>
                    {
                        root = parent;
                    }
                    _ => break,
                }
            }
            families
                .entry((root, method_key.clone()))
                .or_default()
                .push(class_name.clone());
        }
    }
    let mut result = Vec::new();
    'family: for ((_, method_key), mut names) in families {
        if names.len() < 2 {
            continue;
        }
        names.sort();
        let mut members = Vec::with_capacity(names.len());
        for class_name in names {
            let info = &classes[&class_name];
            let sig = methods(info, is_static)[&method_key].clone();
            let impl_class = if is_static {
                info.static_method_impl_classes.get(&method_key)
            } else {
                info.method_impl_classes.get(&method_key)
            };
            let body_span = match impl_class {
                Some(owner) => match source_decl_span(classes, owner, &method_key, is_static) {
                    Some(span) => Some(span),
                    None => continue 'family,
                },
                None => None,
            };
            members.push(FamilyMember {
                class_name,
                placeholder: impl_class.is_none() && !sig.declared_return,
                sig,
                body_span,
            });
        }
        result.push((method_key, members));
    }
    result
}

/// Returns the return type every caller can share, or `None` when no member has a body yet.
///
/// Implementations that agree on one representation keep their type, anything else is `Mixed`.
pub(crate) fn agreed_return(members: &[FamilyMember]) -> Option<PhpType> {
    let mut concrete = members
        .iter()
        .filter(|member| !member.placeholder)
        .map(|member| &member.sig.return_type);
    let first = concrete.next()?;
    if concrete.all(|ty| same_abi(ty, first)) {
        Some(first.clone())
    } else {
        Some(PhpType::Mixed)
    }
}

/// Returns whether two types travel in the same physical representation.
pub(crate) fn same_abi(left: &PhpType, right: &PhpType) -> bool {
    left.reference_payload_compatible(right) && right.reference_payload_compatible(left)
}

/// Returns whether calls to `method_key` on `info` dispatch through a vtable slot.
fn is_dispatched(info: &ClassInfo, method_key: &str, is_static: bool) -> bool {
    let (visibility, slots) = if is_static {
        (&info.static_method_visibilities, &info.static_vtable_slots)
    } else {
        (&info.method_visibilities, &info.vtable_slots)
    };
    !method_key.starts_with("__")
        && !method_key.contains('<')
        && visibility.get(method_key) != Some(&Visibility::Private)
        && slots.contains_key(method_key)
}

/// Returns the declaration span of a user-source method body, or `None` for synthetic bodies.
fn source_decl_span(
    classes: &HashMap<String, ClassInfo>,
    owner: &str,
    method_key: &str,
    is_static: bool,
) -> Option<Span> {
    classes
        .get(owner)?
        .method_decls
        .iter()
        .find(|method| method.is_static == is_static && php_symbol_key(&method.name) == method_key)
        .filter(|method| method.span.line != 0)
        .map(|method| method.span)
}

/// Selects the instance or static method signature table.
pub(crate) fn methods(info: &ClassInfo, is_static: bool) -> &HashMap<String, FunctionSig> {
    if is_static {
        &info.static_methods
    } else {
        &info.methods
    }
}

/// Mutable counterpart of `methods`.
pub(crate) fn methods_mut(info: &mut ClassInfo, is_static: bool) -> &mut HashMap<String, FunctionSig> {
    if is_static {
        &mut info.static_methods
    } else {
        &mut info.methods
    }
}
