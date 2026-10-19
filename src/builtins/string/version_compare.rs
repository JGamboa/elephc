//! Purpose:
//! Home of the PHP `version_compare` builtin: its declaration and semantic metadata.
//!
//! Called from:
//! - Checker, EIR, optimizer, ownership, and callable consumers through
//!   `crate::builtins::registry`.
//!
//! Key details:
//! - The result type depends on the call shape, as in PHP: `int` (-1/0/1) without an operator
//!   or with a `null` one, `bool` with a string operator, and the boxed `int|bool` when the
//!   operator is only known at run time.
//! - An unknown operator raises php-src's catchable `ValueError`, so the runtime function is
//!   `MAY_THROW`.

use crate::builtins::spec::BuiltinCheckCtx;
use crate::errors::CompileError;
use crate::types::PhpType;

builtin! {
    contract: "version_compare",
    check: check,
    semantics: crate::builtins::semantics::runtime_fn_semantics(
        crate::ir::RuntimeFnId::VersionCompare,
    ),
}

/// Returns `int` for the comparison form, `bool` for the operator form, and `int|bool` when
/// the operator may be null at run time.
fn check(cx: &mut BuiltinCheckCtx) -> Result<PhpType, CompileError> {
    let Some(operator) = cx.args.get(2) else {
        return Ok(PhpType::Int);
    };
    match cx.checker.infer_type(operator, cx.env)?.codegen_repr() {
        PhpType::Void | PhpType::Never => Ok(PhpType::Int),
        PhpType::Mixed | PhpType::Union(_) => {
            Ok(cx.checker.normalize_union_type(vec![PhpType::Int, PhpType::Bool]))
        }
        _ => Ok(PhpType::Bool),
    }
}
