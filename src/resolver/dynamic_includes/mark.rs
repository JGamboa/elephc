//! Purpose:
//! Tags every include/require whose path is not literal-shaped with its SITE IDENTITY: the
//! canonical file, the lexical `Class::method` (or function, or `{main}`) and a 1-based ordinal.
//!
//! Called from:
//! - `crate::magic_constants::substitute_file_and_scope_constants`, once per physical file,
//!   right after the scope pass (so `__DIR__` and friends are already plain literals).
//!
//! Key details:
//! - The lexical scope is only known while the file is still its own; the resolver inlines files
//!   into one another, so the identity is captured here and carried on the AST as a marker call
//!   `__elephc_include_site(file, scope, ordinal, line, path)` around the ORIGINAL path.
//! - The ordinal counts, in source order, the non-literal-shaped include paths of one scope. A
//!   path the resolver can still fold (a `const`, a `PHP_VERSION` ternary) is counted too, so the
//!   numbering never depends on resolver state.
//! - The marker never survives resolution: every include path goes through the resolver, which
//!   either folds it (unwrapping the marker) or lowers it through the manifest.

use std::collections::HashMap;
use std::path::Path;

use crate::magic_constants::walker::{walk_program, Pass};
use crate::names::Name;
use crate::parser::ast::{BinOp, Expr, ExprKind, MagicConstant, Program};
use crate::span::Span;

use super::MARKER;

/// Wraps each dynamic include path of `program` in a site marker.
pub(crate) fn mark_include_sites(program: Program, file_path: &Path) -> Program {
    let canonical = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    let mut pass = SiteMarkPass {
        file: canonical.display().to_string(),
        namespace: None,
        namespace_stack: Vec::new(),
        class_stack: Vec::new(),
        function_stack: Vec::new(),
        counters: HashMap::new(),
    };
    walk_program(program, &mut pass)
}

/// Whether a path is built only from literals, concatenation and constant references: the
/// shapes `fold_include_path` folds without any further knowledge.
pub(super) fn is_literal_shaped(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::StringLiteral(_) | ExprKind::ConstRef(_) => true,
        ExprKind::BinaryOp {
            left,
            op: BinOp::Concat,
            right,
        } => is_literal_shaped(left) && is_literal_shaped(right),
        _ => false,
    }
}

/// Lexical scope tracker for the marking walk.
struct SiteMarkPass {
    file: String,
    namespace: Option<String>,
    namespace_stack: Vec<Option<String>>,
    /// Enclosing class-likes (class, interface, trait, enum), innermost last, namespace-qualified.
    class_stack: Vec<String>,
    /// Enclosing functions/methods, innermost last, each paired with the class depth it opened at.
    function_stack: Vec<(String, usize)>,
    counters: HashMap<String, i64>,
}

impl SiteMarkPass {
    fn qualify(&self, name: &str) -> String {
        match self.namespace.as_deref() {
            Some(ns) if !ns.is_empty() => format!("{}\\{}", ns, name),
            _ => name.to_string(),
        }
    }

    /// `Class::method`, `ns\function` or `{main}` for the current position.
    fn scope_id(&self) -> String {
        match self.function_stack.last() {
            Some((name, class_depth)) if *class_depth > 0 && *class_depth == self.class_stack.len() => {
                format!("{}::{}", self.class_stack[class_depth - 1], name)
            }
            Some((name, _)) => self.qualify(name),
            None => "{main}".to_string(),
        }
    }
}

impl Pass for SiteMarkPass {
    fn transform_magic(&self, _span: Span, mc: MagicConstant) -> ExprKind {
        ExprKind::MagicConstant(mc)
    }

    fn transform_include_path(&mut self, path: Expr, span: Span) -> Expr {
        if is_literal_shaped(&path) || super::site_of(&path).is_some() {
            return path;
        }
        let scope = self.scope_id();
        let counter = self.counters.entry(scope.clone()).or_insert(0);
        *counter += 1;
        let ordinal = *counter;
        let path_span = path.span;
        let lit = |value: String| Expr::new(ExprKind::StringLiteral(value), path_span);
        Expr::new(
            ExprKind::FunctionCall {
                name: Name::unqualified(MARKER),
                args: vec![
                    lit(self.file.clone()),
                    lit(scope),
                    Expr::new(ExprKind::IntLiteral(ordinal), path_span),
                    Expr::new(ExprKind::IntLiteral(i64::from(span.line)), path_span),
                    path,
                ],
            },
            path_span,
        )
    }

    fn transform_include_value(&mut self, include: Expr) -> Expr {
        let ExprKind::IncludeValue { path, once, required } = &include.kind else {
            return include;
        };
        match super::site_of(path) {
            Some(site) => super::denied_value_replacement(&site, *once, *required, include.span)
                .unwrap_or(include),
            None => include,
        }
    }

    fn enter_namespace_decl(&mut self, name: &Option<Name>) {
        self.namespace = Some(name.as_ref().map(Name::as_canonical).unwrap_or_default());
    }

    fn enter_namespace_block(&mut self, name: &Option<Name>) {
        self.namespace_stack.push(self.namespace.clone());
        self.namespace = Some(name.as_ref().map(Name::as_canonical).unwrap_or_default());
    }

    fn leave_namespace_block(&mut self) {
        if let Some(previous) = self.namespace_stack.pop() {
            self.namespace = previous;
        }
    }

    fn enter_function(&mut self, name: &str) {
        // A function declared inside a method body is still a global function.
        self.function_stack.push((name.to_string(), 0));
    }

    fn leave_function(&mut self) {
        self.function_stack.pop();
    }

    fn enter_class(&mut self, name: &str) {
        let qualified = self.qualify(name);
        self.class_stack.push(qualified);
    }

    fn leave_class(&mut self) {
        self.class_stack.pop();
    }

    fn enter_trait(&mut self, name: &str) {
        let qualified = self.qualify(name);
        self.class_stack.push(qualified);
    }

    fn leave_trait(&mut self) {
        self.class_stack.pop();
    }

    fn enter_method(&mut self, name: &str, _type_params: &[crate::parser::ast::TypeParam]) {
        self.function_stack.push((name.to_string(), self.class_stack.len()));
    }

    fn leave_method(&mut self) {
        self.function_stack.pop();
    }
}
