//! Purpose:
//! Eval registry entry and implementation for `random_bytes`.
//!
//! Called from:
//! - `crate::interpreter::builtins::hooks`.
//!
//! Key details:
//! - The bytes are read from the operating system's CSPRNG (`/dev/urandom`), the same
//!   source class the compiled helper uses; there is no pseudo-random fallback.
//! - A `$length` below 1 raises php-src's catchable `ValueError`; an unreadable generator
//!   throws an `Exception` with php-src's message (`Random\RandomException` is not declared).

use std::io::Read;

use super::super::super::*;

eval_builtin! {
    contract: "random_bytes",
    area: Math,
    direct: RandomBytes,
    values: RandomBytes,
}

/// php-src's `ValueError` wording for a `$length` below 1.
const RANDOM_BYTES_LENGTH_MESSAGE: &str =
    "random_bytes(): Argument #1 ($length) must be greater than 0";

/// php-src's message when the operating system cannot provide randomness.
const RANDOM_BYTES_FAILURE_MESSAGE: &str =
    "Failed to retrieve randomness from the operating system";

/// Evaluates PHP `random_bytes()` over one eval expression.
pub(in crate::interpreter) fn eval_builtin_random_bytes(
    args: &[EvalExpr],
    context: &mut ElephcEvalContext,
    scope: &mut ElephcEvalScope,
    values: &mut impl RuntimeValueOps,
) -> Result<RuntimeCellHandle, EvalStatus> {
    let [length] = args else {
        return Err(EvalStatus::RuntimeFatal);
    };
    let length = eval_expr(length, context, scope, values)?;
    eval_random_bytes_values_result(&[length], context, values)
}

/// Dispatches by-value `random_bytes()` calls after argument binding.
pub(in crate::interpreter) fn eval_random_bytes_values_result(
    evaluated_args: &[RuntimeCellHandle],
    context: &mut ElephcEvalContext,
    values: &mut impl RuntimeValueOps,
) -> Result<RuntimeCellHandle, EvalStatus> {
    let [length] = evaluated_args else {
        return Err(EvalStatus::RuntimeFatal);
    };
    let length = eval_int_value(*length, values)?;
    if length < 1 {
        return eval_random_bytes_throw("ValueError", RANDOM_BYTES_LENGTH_MESSAGE, context, values);
    }
    let mut bytes = vec![0u8; usize::try_from(length).map_err(|_| EvalStatus::RuntimeFatal)?];
    let filled = std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes));
    if filled.is_err() {
        return eval_random_bytes_throw("Exception", RANDOM_BYTES_FAILURE_MESSAGE, context, values);
    }
    values.string_bytes_value(&bytes)
}

/// Raises one catchable throwable with a php-src message.
fn eval_random_bytes_throw<T>(
    class: &str,
    message: &str,
    context: &mut ElephcEvalContext,
    values: &mut impl RuntimeValueOps,
) -> Result<T, EvalStatus> {
    let exception = values.new_object(class)?;
    let message = values.string(message)?;
    let code = values.int(0)?;
    values.construct_object(exception, vec![message, code])?;
    context.set_pending_throw(exception);
    Err(EvalStatus::UncaughtThrowable)
}
