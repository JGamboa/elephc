//! Purpose:
//! Diagnostics for the PHP core class-likes elephc declares itself (`UnitEnum`, `BackedEnum`,
//! `Serializable`, `SessionHandlerInterface`, `WeakMap`, `ErrorException`).
//!
//! Called from:
//! - `cargo test --test error_tests native_classes` through Rust's test harness.
//!
//! Key details:
//! - Each rejected shape is one where elephc would otherwise print something PHP does not, or
//!   accept a declaration PHP refuses.

use super::*;

/// A user declaration of a builtin interface name is a redeclaration, as in PHP.
#[test]
fn test_error_redeclaring_unit_enum() {
    expect_error("<?php interface UnitEnum {}", "Cannot redeclare built-in interface: UnitEnum");
    expect_error("<?php class Serializable {}", "Cannot redeclare built-in interface: Serializable");
}

/// A user declaration of a prelude-provided class name is rejected like PHP's fatal error.
#[test]
fn test_error_redeclaring_weakmap_and_error_exception() {
    expect_error(
        "<?php class WeakMap {} $m = new WeakMap();",
        "Cannot declare class WeakMap, because the name is already in use",
    );
    expect_error(
        "<?php class ErrorException extends Exception {}",
        "Cannot declare class ErrorException, because the name is already in use",
    );
}

/// PHP 8.1+ prints a `Deprecated:` notice for a class implementing `Serializable` without
/// `__serialize()`/`__unserialize()`; elephc refuses it instead of silently omitting the notice.
#[test]
fn test_error_legacy_serializable_without_magic_methods() {
    expect_error(
        "<?php class S implements Serializable { public function serialize() { return ''; } public function unserialize(string $data) {} }",
        "S implements the Serializable interface, which is deprecated",
    );
}

/// The modern `__serialize()`/`__unserialize()` pair, inherited or declared, is accepted.
#[test]
fn test_serializable_with_inherited_magic_methods_is_accepted() {
    expect_no_error(
        "<?php class P { public function __serialize(): array { return []; } public function __unserialize(array $d): void {} }
         class S extends P implements Serializable { public function serialize() { return ''; } public function unserialize(string $data) {} }",
    );
}

/// A caller-supplied file or line cannot be honoured by elephc's `getFile()`/`getLine()`
/// intrinsics, so passing one is a compile-time error rather than a silently ignored location.
#[test]
fn test_error_error_exception_with_filename() {
    expect_error(
        "<?php $e = new ErrorException('m', 0, E_WARNING, '/tmp/x.php', 3);",
        "does not support a non-null filename or line in ErrorException",
    );
}

/// `UnitEnum`/`BackedEnum` are reserved for enums, directly or through a derived interface.
#[test]
fn test_error_class_implementing_unit_enum() {
    expect_error(
        "<?php class C implements UnitEnum { public static function cases(): array { return []; } }",
        "Non-enum class C cannot implement interface UnitEnum",
    );
    expect_error(
        "<?php interface M extends BackedEnum {} abstract class C implements M {}",
        "Non-enum class C cannot implement interface UnitEnum",
    );
}

/// Enums may not re-implement their implicit interfaces, and a pure enum cannot reach
/// `BackedEnum`; the messages are PHP's.
#[test]
fn test_error_enum_marker_interface_misuse() {
    expect_error(
        "<?php enum E: int implements UnitEnum { case A = 1; }",
        "Enum E cannot implement previously implemented interface UnitEnum",
    );
    expect_error(
        "<?php enum E: string implements BackedEnum { case A = 'a'; }",
        "Enum E cannot implement previously implemented interface BackedEnum",
    );
    expect_error(
        "<?php enum E implements BackedEnum { case A; }",
        "Non-backed enum E cannot implement interface BackedEnum",
    );
    expect_error(
        "<?php interface M extends BackedEnum {} enum E implements M { case A; }",
        "Non-backed enum E cannot implement interface BackedEnum",
    );
}

/// A class listing `BackedEnum` itself is reported under that name, as PHP does.
#[test]
fn test_error_class_implementing_backed_enum_names_it() {
    expect_error(
        "<?php class C implements BackedEnum {}",
        "Non-enum class C cannot implement interface BackedEnum",
    );
}

/// The unsupported filename/line diagnostic names the limitation, not the internal marker.
#[test]
fn test_error_error_exception_filename_message_is_plain() {
    let message = check_source("<?php $e = new ErrorException('m', 0, E_WARNING, '/tmp/x.php', 3);")
        .expect_err("must fail");
    assert!(message.contains("elephc does not support a non-null filename or line in ErrorException yet"), "{message}");
    assert!(!message.contains("__Elephc"), "{message}");
}

/// A redeclared builtin interface is reported at the user's declaration.
#[test]
fn test_error_redeclared_builtin_interface_has_a_location() {
    let error = check_source_full("<?php\n\ninterface UnitEnum {}").expect_err("must fail");
    let rendered = format!("{error:?}");
    assert!(rendered.contains("Cannot redeclare built-in interface: UnitEnum"), "{rendered}");
    assert!(rendered.contains("line: 3"), "{rendered}");
}
