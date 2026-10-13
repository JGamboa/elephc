//! Purpose:
//! Computes which locals a condition DEFINITELY assigns on its true edge and on its false edge,
//! following PHP's short-circuit evaluation: `$a && ($r = f())` assigns `$r` whenever it is
//! true, `!$x || !($y = g())` assigns `$y` whenever it is false.
//!
//! Called from:
//! - `crate::types::checker::stmt_check::control_flow` (the `if` / `elseif` / loop bodies) and
//!   `crate::types::checker::inference::expr::effects` (ternary arms), which type those locals.
//! - `crate::ir_lower` (`if`, loops, ternary), which marks their slots initialized on that edge.
//!
//! Key details:
//! - Purely syntactic, so the checker and lowering agree on the same set.
//! - A store inside a lazily evaluated operand that this walk does not decompose (a nested
//!   `&&` inside a call argument, a `?:` default, a closure body) is never counted: it may not run.

use std::collections::BTreeSet;

use crate::parser::ast::{BinOp, Expr, ExprKind};

use super::loop_storage::visit_child_expressions;

/// Returns the locals `condition` assigns on every evaluation that yields `edge` (true/false).
pub(crate) fn definitely_assigned_on_edge(condition: &Expr, edge: bool) -> BTreeSet<String> {
    match &condition.kind {
        ExprKind::BinaryOp { left, op: BinOp::And, right } => {
            let left_true = definitely_assigned_on_edge(left, true);
            if edge {
                // Both operands ran and were true.
                left_true.union(&definitely_assigned_on_edge(right, true)).cloned().collect()
            } else {
                // Either the left was false, or it was true and the right was false.
                let left_false = definitely_assigned_on_edge(left, false);
                let through_right: BTreeSet<String> =
                    left_true.union(&definitely_assigned_on_edge(right, false)).cloned().collect();
                left_false.intersection(&through_right).cloned().collect()
            }
        }
        ExprKind::BinaryOp { left, op: BinOp::Or, right } => {
            let left_false = definitely_assigned_on_edge(left, false);
            if edge {
                let left_true = definitely_assigned_on_edge(left, true);
                let through_right: BTreeSet<String> =
                    left_false.union(&definitely_assigned_on_edge(right, true)).cloned().collect();
                left_true.intersection(&through_right).cloned().collect()
            } else {
                left_false.union(&definitely_assigned_on_edge(right, false)).cloned().collect()
            }
        }
        ExprKind::Not(inner) => definitely_assigned_on_edge(inner, !edge),
        _ => {
            let mut out = BTreeSet::new();
            collect_unconditional_assignments(condition, &mut out);
            out
        }
    }
}

/// Collects plain-local assignment targets that run whenever `expr` is evaluated.
fn collect_unconditional_assignments(expr: &Expr, out: &mut BTreeSet<String>) {
    match &expr.kind {
        ExprKind::Assignment { target, value, .. } => {
            collect_unconditional_assignments(value, out);
            if let ExprKind::Variable(name) = &target.kind {
                // `$x ??= v` stores only when `$x` is null, but leaves `$x` defined either way.
                out.insert(name.clone());
            } else {
                collect_unconditional_assignments(target, out);
            }
        }
        // Lazily evaluated operands may not run.
        ExprKind::BinaryOp { left, op: BinOp::And | BinOp::Or, .. } => {
            collect_unconditional_assignments(left, out)
        }
        ExprKind::ShortTernary { value, .. } | ExprKind::NullCoalesce { value, .. } => {
            collect_unconditional_assignments(value, out)
        }
        ExprKind::Ternary { condition, .. } => collect_unconditional_assignments(condition, out),
        ExprKind::Closure { .. } | ExprKind::Match { .. } => {}
        _ => visit_child_expressions(expr, &mut |child| collect_unconditional_assignments(child, out)),
    }
}

impl super::Checker {
    /// Returns the locals `condition` definitely defines on `edge` that `env` does not hold yet,
    /// with their types. The types come from re-inferring the condition on a copy of `env` as if
    /// every lazy operand ran, which is exactly the evaluation that edge implies for those names.
    pub(crate) fn condition_edge_definitions(
        &mut self,
        condition: &Expr,
        env: &crate::types::TypeEnv,
        edge: bool,
    ) -> crate::types::TypeEnv {
        let names = definitely_assigned_on_edge(condition, edge);
        let mut defined = crate::types::TypeEnv::new();
        if names.iter().all(|name| env.contains_key(name)) {
            return defined;
        }
        let all_ran = self.infer_with_every_lazy_operand(condition, env);
        for name in names {
            if env.contains_key(&name) {
                continue;
            }
            if let Some(ty) = all_ran.get(&name) {
                defined.insert(name, ty.clone());
            }
        }
        defined
    }

    /// Infers `expr` on a copy of `env`, applying the assignments of every lazy operand the
    /// edge walk decomposes (`&&`, `||`, `!`) in evaluation order.
    fn infer_with_every_lazy_operand(
        &mut self,
        expr: &Expr,
        env: &crate::types::TypeEnv,
    ) -> crate::types::TypeEnv {
        match &expr.kind {
            ExprKind::BinaryOp { left, op: BinOp::And | BinOp::Or, right } => {
                let after_left = self.infer_with_every_lazy_operand(left, env);
                self.infer_with_every_lazy_operand(right, &after_left)
            }
            ExprKind::Not(inner) => self.infer_with_every_lazy_operand(inner, env),
            _ => {
                let mut copy = env.clone();
                let _ = self.infer_type_with_assignment_effects(expr, &mut copy);
                copy
            }
        }
    }

    /// Inserts edge definitions into `env` for a guarded region, returning what was inserted so
    /// [`Self::remove_edge_definitions`] can take it back out afterwards.
    pub(crate) fn insert_edge_definitions(
        env: &mut crate::types::TypeEnv,
        defined: &crate::types::TypeEnv,
    ) -> Vec<(String, crate::types::PhpType)> {
        let mut inserted = Vec::new();
        for (name, ty) in defined {
            if !env.contains_key(name) {
                env.insert(name.clone(), ty.clone());
                inserted.push((name.clone(), ty.clone()));
            }
        }
        inserted
    }

    /// Removes edge definitions a guarded region did not overwrite: past the region the edge no
    /// longer holds, so the local is only POSSIBLY defined and reading it stays an error.
    pub(crate) fn remove_edge_definitions(
        env: &mut crate::types::TypeEnv,
        inserted: &[(String, crate::types::PhpType)],
    ) {
        for (name, ty) in inserted {
            if env.get(name) == Some(ty) {
                env.remove(name);
            }
        }
    }
}
