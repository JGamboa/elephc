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
