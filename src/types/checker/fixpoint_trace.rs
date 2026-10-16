//! Purpose:
//! Opt-in tracing of the checker's signature fixpoints (`ELEPHC_CHECKER_TRACE=1`).
//! Reports, per method pass and per outer round, how long it took and which class or function
//! signatures moved, so a fixpoint that oscillates or crawls is diagnosable on a real program.
//!
//! Called from:
//! - `crate::types::checker::method_pass` and `crate::types::checker::driver`.
//!
//! Key details:
//! - Silent and allocation-free unless the variable is set: every entry point checks
//!   [`enabled`] first, which reads the environment once per process.
//! - `ELEPHC_CHECKER_TRACE_DETAIL=Class::method` (or a function name) additionally prints the
//!   before/after parameter and return types of that one signature whenever it changes.

use std::collections::HashMap;

use crate::types::{ClassInfo, FunctionSig};

/// Returns whether fixpoint tracing was requested for this process.
pub(crate) fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("ELEPHC_CHECKER_TRACE").as_deref() == Ok("1"))
}

/// The one signature whose types are printed on change, if requested.
fn detail_target() -> Option<&'static str> {
    static TARGET: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    TARGET
        .get_or_init(|| std::env::var("ELEPHC_CHECKER_TRACE_DETAIL").ok())
        .as_deref()
}

/// Prints the method and static-method signatures that differ between two class tables.
pub(crate) fn report_class_changes(
    label: &str,
    elapsed: std::time::Duration,
    before: &HashMap<String, ClassInfo>,
    after: &HashMap<String, ClassInfo>,
) {
    let mut changed: Vec<String> = Vec::new();
    for (name, info) in after {
        match before.get(name) {
            Some(old) if old == info => {}
            Some(old) => {
                let mut members: Vec<String> = Vec::new();
                for (key, sig) in &info.methods {
                    let old_sig = old.methods.get(key);
                    if old_sig != Some(sig) {
                        members.push(key.clone());
                        report_detail(&format!("{name}::{key}"), old_sig, sig);
                    }
                }
                for (key, sig) in &info.static_methods {
                    let old_sig = old.static_methods.get(key);
                    if old_sig != Some(sig) {
                        members.push(format!("static {key}"));
                        report_detail(&format!("{name}::{key}"), old_sig, sig);
                    }
                }
                if members.is_empty() {
                    members.push("<non-signature fields>".to_string());
                }
                members.sort();
                changed.push(format!("{name}[{}]", members.join(",")));
            }
            None => changed.push(format!("{name}(new)")),
        }
    }
    changed.sort();
    eprintln!(
        "checker-trace {label}: {:.1}s, {} classes changed {:?}",
        elapsed.as_secs_f64(),
        changed.len(),
        changed.iter().take(12).collect::<Vec<_>>()
    );
}

/// Prints the function signatures that differ between two function tables.
pub(crate) fn report_function_changes(
    label: &str,
    before: &HashMap<String, FunctionSig>,
    after: &HashMap<String, FunctionSig>,
) {
    let mut changed: Vec<&String> = Vec::new();
    for (name, sig) in after {
        let old = before.get(name);
        if old != Some(sig) {
            changed.push(name);
            report_detail(name, old, sig);
        }
    }
    changed.sort();
    eprintln!(
        "checker-trace {label}: {} functions changed {:?}",
        changed.len(),
        changed.iter().take(12).collect::<Vec<_>>()
    );
}

/// Prints one signature's before/after types when it is the requested detail target.
fn report_detail(name: &str, before: Option<&FunctionSig>, after: &FunctionSig) {
    if detail_target().is_none_or(|target| !target.eq_ignore_ascii_case(name)) {
        return;
    }
    let show = |sig: &FunctionSig| format!("params={:?} return={:?}", sig.params, sig.return_type);
    eprintln!(
        "checker-trace   {name}: {} -> {}",
        before.map(show).unwrap_or_else(|| "<none>".to_string()),
        show(after)
    );
}

/// Default number of times one signature may change across a fixpoint before the checker
/// declares the fixpoint divergent (`ELEPHC_CHECKER_OSCILLATION_LIMIT` overrides it).
///
/// Every widening the fixpoints perform is a JOIN (see `crate::types::checker::lattice`), so a
/// signature only climbs, and each of its types climbs a lattice of small finite height: a
/// legitimate signature changes a handful of times, bounded by its parameter count times that
/// height. An entity that keeps changing far beyond that is not converging, it is oscillating
/// — some writer replaces instead of joining — and looping on until the round budget runs out
/// (thousands of rounds on Laravel) only hides it. The detector turns it into a clear internal
/// error that names the entity.
const DEFAULT_OSCILLATION_LIMIT: usize = 48;

/// Counts how often each class member or function signature changes across fixpoint rounds.
pub(crate) struct OscillationDetector {
    label: &'static str,
    limit: usize,
    counts: HashMap<String, usize>,
}

impl OscillationDetector {
    /// Builds a detector for one fixpoint loop, labelled for its diagnostic.
    pub(crate) fn new(label: &'static str) -> Self {
        let limit = std::env::var("ELEPHC_CHECKER_OSCILLATION_LIMIT")
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|limit| *limit > 0)
            .unwrap_or(DEFAULT_OSCILLATION_LIMIT);
        Self { label, limit, counts: HashMap::new() }
    }

    /// Records one round's changes; returns the internal error once an entity exceeds the limit.
    pub(crate) fn record(
        &mut self,
        classes_before: &HashMap<String, ClassInfo>,
        classes_after: &HashMap<String, ClassInfo>,
        functions: Option<(&HashMap<String, FunctionSig>, &HashMap<String, FunctionSig>)>,
    ) -> Option<String> {
        let mut changed = changed_class_entities(classes_before, classes_after);
        if let Some((before, after)) = functions {
            changed.extend(changed_function_entities(before, after));
        }
        let mut worst: Option<(String, usize, String)> = None;
        for (entity, transition) in changed {
            let count = self.counts.entry(entity.clone()).or_insert(0);
            *count += 1;
            if *count > self.limit
                && worst.as_ref().is_none_or(|(name, _, _)| entity < *name)
            {
                worst = Some((entity, *count, transition));
            }
        }
        worst.map(|(entity, count, transition)| {
            format!(
                "internal compiler error: the type checker's {} fixpoint does not converge: \
                 `{}` changed {} times (limit {}; last change {}). A widening on this entity \
                 replaces its previous type instead of joining it; set \
                 ELEPHC_CHECKER_TRACE=1 ELEPHC_CHECKER_TRACE_DETAIL='{}' to see the alternation",
                self.label, entity, count, self.limit, transition, entity
            )
        })
    }
}

/// Lists `(entity, transition)` for every class member whose signature or storage moved.
fn changed_class_entities(
    before: &HashMap<String, ClassInfo>,
    after: &HashMap<String, ClassInfo>,
) -> Vec<(String, String)> {
    let show = |sig: Option<&FunctionSig>| {
        sig.map(|sig| format!("params={:?} return={:?}", sig.params, sig.return_type))
            .unwrap_or_else(|| "<none>".to_string())
    };
    let mut changed = Vec::new();
    for (name, info) in after {
        let Some(old) = before.get(name) else {
            continue;
        };
        if old == info {
            continue;
        }
        let mut any_member = false;
        for (key, sig) in &info.methods {
            let old_sig = old.methods.get(key);
            if old_sig != Some(sig) {
                any_member = true;
                changed.push((
                    format!("{name}::{key}"),
                    format!("{} -> {}", show(old_sig), show(Some(sig))),
                ));
            }
        }
        for (key, sig) in &info.static_methods {
            let old_sig = old.static_methods.get(key);
            if old_sig != Some(sig) {
                any_member = true;
                changed.push((
                    format!("{name}::{key} (static)"),
                    format!("{} -> {}", show(old_sig), show(Some(sig))),
                ));
            }
        }
        for (prop, ty) in &info.properties {
            let old_ty = old.properties.iter().find(|(old_prop, _)| old_prop == prop);
            if old_ty.map(|(_, old_ty)| old_ty) != Some(ty) {
                any_member = true;
                changed.push((
                    format!("{name}::${prop}"),
                    format!("{:?} -> {:?}", old_ty.map(|(_, ty)| ty), ty),
                ));
            }
        }
        if !any_member {
            changed.push((format!("{name} <non-signature fields>"), "<metadata>".to_string()));
        }
    }
    changed
}

/// Lists `(entity, transition)` for every function signature that moved.
fn changed_function_entities(
    before: &HashMap<String, FunctionSig>,
    after: &HashMap<String, FunctionSig>,
) -> Vec<(String, String)> {
    after
        .iter()
        .filter_map(|(name, sig)| {
            let old = before.get(name)?;
            (old != sig).then(|| {
                (
                    name.clone(),
                    format!(
                        "params={:?} return={:?} -> params={:?} return={:?}",
                        old.params, old.return_type, sig.params, sig.return_type
                    ),
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PhpType;

    /// Checks a tiny program and returns its tables, as the fixpoints would see them.
    fn tables() -> (HashMap<String, ClassInfo>, HashMap<String, FunctionSig>) {
        let source = "<?php class Osc { public function m($x) { return $x; } } \
                      function f($x) { return $x; } echo f(1), (new Osc())->m(2);";
        let tokens = crate::lexer::tokenize(source).expect("tokenize");
        let program = crate::parser::parse(&tokens).expect("parse");
        let program = crate::name_resolver::resolve(program).expect("name resolve");
        let checked = crate::types::checker::check_types(
            &program,
            crate::codegen_support::platform::Target::new(
                crate::codegen_support::platform::Platform::MacOS,
                crate::codegen_support::platform::Arch::AArch64,
            ),
        )
        .expect("check");
        (checked.classes, checked.functions)
    }

    /// A signature that alternates between two types is reported by name once it passes the
    /// limit, instead of looping until the round budget runs out.
    #[test]
    fn alternating_signature_is_reported_by_name() {
        let (classes, functions) = tables();
        let mut int_classes = classes.clone();
        let mut float_classes = classes;
        int_classes.get_mut("Osc").unwrap().methods.get_mut("m").unwrap().return_type = PhpType::Int;
        float_classes.get_mut("Osc").unwrap().methods.get_mut("m").unwrap().return_type =
            PhpType::Float;
        let mut detector = OscillationDetector::new("outer");
        let mut reported = None;
        for round in 0..(DEFAULT_OSCILLATION_LIMIT + 2) {
            let (before, after) = if round % 2 == 0 {
                (&int_classes, &float_classes)
            } else {
                (&float_classes, &int_classes)
            };
            if let Some(message) = detector.record(before, after, Some((&functions, &functions))) {
                reported = Some((round, message));
                break;
            }
        }
        let (round, message) = reported.expect("the oscillation must be reported");
        assert_eq!(round, DEFAULT_OSCILLATION_LIMIT, "reported as soon as it passes the limit");
        assert!(message.contains("`Osc::m`"), "names the entity: {message}");
        assert!(message.contains("does not converge"), "{message}");
    }

    /// A function signature that keeps changing is caught the same way.
    #[test]
    fn alternating_function_is_reported_by_name() {
        let (classes, functions) = tables();
        let mut a = functions.clone();
        let mut b = functions;
        a.get_mut("f").unwrap().return_type = PhpType::Str;
        b.get_mut("f").unwrap().return_type = PhpType::Int;
        let mut detector = OscillationDetector::new("outer");
        let mut message = None;
        for round in 0..(DEFAULT_OSCILLATION_LIMIT * 2) {
            let (before, after) = if round % 2 == 0 { (&a, &b) } else { (&b, &a) };
            message = detector.record(&classes, &classes, Some((before, after)));
            if message.is_some() {
                break;
            }
        }
        assert!(message.expect("reported").contains("`f`"));
    }
}
