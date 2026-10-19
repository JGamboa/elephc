//! Purpose:
//! Home of the PHP `random_bytes` builtin: its declaration and semantic metadata.
//!
//! Called from:
//! - Checker, EIR, optimizer, ownership, and callable consumers through
//!   `crate::builtins::registry`.
//!
//! Key details:
//! - The bytes come from the operating system's CSPRNG (`arc4random_buf` on macOS,
//!   `getrandom` on Linux); there is no weak fallback.
//! - A `$length` below 1 raises php-src's catchable `ValueError`.

builtin! {
    contract: "random_bytes",
    semantics: crate::builtins::semantics::runtime_fn_semantics(
        crate::ir::RuntimeFnId::RandomBytes,
    ),
}
