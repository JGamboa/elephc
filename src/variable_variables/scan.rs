//! Purpose:
//! Cheap read-only pre-scan deciding whether a body needs the variable-variable rewrite.
//!
//! Called from:
//! - `super::rewrite::Rewriter` before it collects a frame's names.
//!
//! Key details:
//! - The scan descends into nested functions, methods and closures, so a frame is entered
//!   whenever anything inside it needs rewriting; the rewrite then re-checks each nested frame.

use crate::parser::ast::{Expr, ExprKind, Stmt, StmtKind};

use super::rewrite::builtin_call_name;
use super::walk::{walk_expr_children, walk_stmt_children, Visitor};
use super::FRAME_LOCAL;

/// Returns whether `body` holds a variable variable or a `compact()`/`extract()` call.
pub(super) fn needs_rewrite(body: &mut Vec<Stmt>) -> bool {
    let mut scan = Scan { found: false };
    scan.block(body);
    scan.found
}

struct Scan {
    found: bool,
}

impl Visitor for Scan {
    fn stmt(&mut self, stmt: &mut Stmt) {
        if self.found {
            return;
        }
        match &mut stmt.kind {
            StmtKind::FunctionDecl { body, .. } => self.block(body),
            StmtKind::ClassDecl { methods, .. }
            | StmtKind::TraitDecl { methods, .. }
            | StmtKind::InterfaceDecl { methods, .. }
            | StmtKind::EnumDecl { methods, .. } => {
                for method in methods.iter_mut() {
                    self.block(&mut method.body);
                }
            }
            _ => walk_stmt_children(self, stmt),
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        if self.found {
            return;
        }
        match &mut expr.kind {
            ExprKind::Variable(name) if name == FRAME_LOCAL => self.found = true,
            ExprKind::FunctionCall { name, .. }
                if matches!(builtin_call_name(name).as_deref(), Some("compact" | "extract")) =>
            {
                self.found = true
            }
            ExprKind::Closure { body, .. } => self.block(body),
            _ => walk_expr_children(self, expr),
        }
    }
}
