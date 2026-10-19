//! Purpose:
//! Injects PHP's `SessionHandlerInterface` so user session handlers type-check in every
//! compilation mode, not only under `--web`.
//!
//! Called from:
//! - `crate::types::checker::driver` during builtin type/schema initialization.
//!
//! Key details:
//! - The `--web` prelude (`web_prelude::build`) also declares `SessionHandlerInterface`. That
//!   internal declaration wins and this injection skips the name; a declaration in the user's
//!   own source is a redeclaration error, as PHP reports it.

use std::collections::HashMap;

use crate::errors::CompileError;
use crate::names::php_symbol_key;
use crate::parser::ast::{ClassMethod, Program, StmtKind, TypeExpr, Visibility};
use crate::source::SourceMode;
use crate::types::traits::FlattenedClass;

use super::builtin_types::InterfaceDeclInfo;

const SESSION_HANDLER_INTERFACE: &str = "SessionHandlerInterface";

/// Registers `SessionHandlerInterface` unless an internal prelude already declared it.
///
/// Rejects user redeclarations (PHP case-insensitive) with the same message as the other
/// builtin interfaces.
pub(crate) fn inject_builtin_session_interfaces(
    program: &Program,
    interface_map: &mut HashMap<String, InterfaceDeclInfo>,
    class_map: &mut HashMap<String, FlattenedClass>,
) -> Result<(), CompileError> {
    let key = php_symbol_key(SESSION_HANDLER_INTERFACE);
    let collision = interface_map
        .iter()
        .filter(|(name, _)| php_symbol_key(name) == key)
        .map(|(_, info)| info.span)
        .chain(
            class_map
                .iter()
                .filter(|(name, _)| php_symbol_key(name) == key)
                .map(|(_, class)| class.span),
        )
        .next();
    if let Some(span) = collision {
        if declared_internally(program, &key) {
            return Ok(());
        }
        return Err(CompileError::new(
            span,
            &format!("Cannot redeclare built-in interface: {}", SESSION_HANDLER_INTERFACE),
        ));
    }

    let str_or_false = || TypeExpr::Union(vec![TypeExpr::Str, TypeExpr::False]);
    let int_or_false = || TypeExpr::Union(vec![TypeExpr::Int, TypeExpr::False]);
    interface_map.insert(
        SESSION_HANDLER_INTERFACE.to_string(),
        InterfaceDeclInfo {
            name: SESSION_HANDLER_INTERFACE.to_string(),
            extends: Vec::new(),
            properties: Vec::new(),
            methods: vec![
                abstract_method(
                    "open",
                    vec![("path", TypeExpr::Str), ("name", TypeExpr::Str)],
                    TypeExpr::Bool,
                ),
                abstract_method("close", Vec::new(), TypeExpr::Bool),
                abstract_method("read", vec![("id", TypeExpr::Str)], str_or_false()),
                abstract_method(
                    "write",
                    vec![("id", TypeExpr::Str), ("data", TypeExpr::Str)],
                    TypeExpr::Bool,
                ),
                abstract_method("destroy", vec![("id", TypeExpr::Str)], TypeExpr::Bool),
                abstract_method("gc", vec![("max_lifetime", TypeExpr::Int)], int_or_false()),
            ],
            span: crate::span::Span::dummy(),
            constants: Vec::new(),
        },
    );
    Ok(())
}

/// Returns whether a top-level declaration named `key` came from an internal prelude.
fn declared_internally(program: &Program, key: &str) -> bool {
    program.iter().any(|stmt| {
        stmt.source_mode == SourceMode::Internal
            && match &stmt.kind {
                StmtKind::InterfaceDecl { name, .. } | StmtKind::ClassDecl { name, .. } => {
                    php_symbol_key(name) == key
                }
                _ => false,
            }
    })
}

/// Builds a public abstract interface method with typed parameters and return type.
fn abstract_method(name: &str, params: Vec<(&str, TypeExpr)>, return_type: TypeExpr) -> ClassMethod {
    ClassMethod {
        type_params: Vec::new(),
        name: name.to_string(),
        visibility: Visibility::Public,
        is_static: false,
        is_abstract: true,
        is_final: false,
        has_body: false,
        params: params
            .into_iter()
            .map(|(param, ty)| (param.to_string(), Some(ty), None, false))
            .collect(),
        param_attributes: Vec::new(),
        variadic: None,
        variadic_by_ref: false,
        variadic_type: None,
        return_type: Some(return_type),
        by_ref_return: false,
        body: Vec::new(),
        span: crate::span::Span::dummy(),
        attributes: Vec::new(),
    }
}
