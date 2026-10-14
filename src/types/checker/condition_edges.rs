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

/// Maybe-undefined locals (review P2c r2, R2-2).
///
/// A local a join point may leave undefined — assigned in one `if` arm, on one lazy edge, or only
/// inside a loop body — is tracked in `Checker::maybe_undefined_locals` while the body is walked.
/// A read of such a local is recorded in `maybe_undefined_result` and typed `mixed`, and EIR
/// lowering answers it from the local's hidden "defined" flag: PHP null (after an `Undefined
/// variable` warning for a plain read) when the local was never assigned on the path taken.
/// Before this, the read loaded whatever the slot held: a previous call's value for `isset($r)`,
/// a folded null for `$r ?? $d` even when the edge had assigned it.
impl super::Checker {
    /// Whether `name` can be tracked: a frame local this body owns and writes only by name.
    pub(crate) fn local_is_flaggable(&self, name: &str) -> bool {
        !self.body_contains_eval
            && name != "this"
            && !name.starts_with('\u{1}')
            && !self.active_ref_params.contains(name)
            && !self.ref_aliased_locals.contains(name)
            && !self.by_ref_argument_locals.contains(name)
            && !self.active_globals.contains(name)
            && !self.static_local_names.contains(name)
            && !self.name_is_seeded_program_storage(name)
            && !self.top_level_binding_is_program_global(name)
    }

    /// Marks `name` maybe-undefined from this point of the walk.
    pub(crate) fn mark_maybe_undefined(&mut self, name: &str) {
        if self.local_is_flaggable(name) {
            self.maybe_undefined_locals.insert(name.to_string());
        }
    }

    /// Records a read of a maybe-undefined local; returns whether the read is flagged.
    pub(crate) fn record_maybe_undefined_read(&mut self, name: &str, span: crate::span::Span) -> bool {
        if !self.maybe_undefined_locals.contains(name) || !self.local_is_flaggable(name) {
            return false;
        }
        let warns = self.null_probe_depth == 0;
        self.maybe_undefined_result
            .reads
            .insert((span, name.to_string()), warns);
        self.maybe_undefined_result
            .by_scope
            .entry(self.current_loop_storage_scope.clone())
            .or_default()
            .insert(name.to_string());
        true
    }

    /// Joins a construct whose body may not run (a loop) or whose parts the walk cannot split
    /// into arms (`switch`, `try`): a mark the body cleared does not hold after it, and a local
    /// the body introduced, outside `definite`, may be undefined after it.
    pub(crate) fn join_maybe_undefined_after_optional_region(
        &mut self,
        marks_before: &std::collections::HashSet<String>,
        keys_before: &std::collections::HashSet<String>,
        definite: &BTreeSet<String>,
        mark_new: bool,
        env: &crate::types::TypeEnv,
    ) {
        for name in marks_before {
            self.maybe_undefined_locals.insert(name.clone());
        }
        if mark_new {
            let new = env
                .keys()
                .filter(|name| !keys_before.contains(*name) && !definite.contains(*name))
                .cloned()
                .collect::<Vec<_>>();
            for name in new {
                self.mark_maybe_undefined(&name);
            }
        }
    }
}

/// Assignments a statement runs before its body for sure (`for` init, the unconditional part of
/// a loop condition), which stay defined after the loop.
pub(crate) fn loop_head_definite_assignments(stmt: &crate::parser::ast::Stmt) -> BTreeSet<String> {
    use crate::parser::ast::StmtKind;
    let mut out = BTreeSet::new();
    match &stmt.kind {
        StmtKind::While { condition, .. } => collect_unconditional_assignments(condition, &mut out),
        StmtKind::For { init, condition, .. } => {
            if let Some(init) = init {
                if let StmtKind::Assign { name, .. } = &init.kind {
                    out.insert(name.clone());
                } else if let StmtKind::ExprStmt(expr) = &init.kind {
                    collect_unconditional_assignments(expr, &mut out);
                } else if let StmtKind::Synthetic(stmts) = &init.kind {
                    for stmt in stmts {
                        if let StmtKind::Assign { name, .. } = &stmt.kind {
                            out.insert(name.clone());
                        }
                    }
                }
            }
            if let Some(condition) = condition {
                collect_unconditional_assignments(condition, &mut out);
            }
        }
        _ => {}
    }
    out
}
