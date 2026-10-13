//! Purpose:
//! Diagnostics for override and interface-implementation signatures (LSP) that PHP also
//! rejects, plus the PHP-valid shapes elephc refuses instead of miscompiling.
//!
//! Called from:
//! - `cargo test --test error_tests` through Rust's test harness.
//!
//! Key details:
//! - Complements `tests/codegen/oop/override_compatibility.rs`, which covers the accepted shapes.

use super::*;

/// PHP accepts it, but fixed-arity dispatch cannot supply the added default through the parent: refused, not miscompiled.
#[test]
fn test_error_override_adds_optional_parameter() {
    expect_error(
        r#"<?php class Base { public function greet($n) { return $n; } } class Child extends Base { public function greet($n, $s = "!") { return $n . $s; } }"#,
        r#"Cannot add optional parameters when overriding method: Child::greet"#,
    );
}

/// Same refusal against a builtin interface (`json_encode` calls `jsonSerialize()` with no argument).
#[test]
fn test_error_json_serialize_adds_optional_parameter() {
    expect_error(
        r#"<?php class Bag implements JsonSerializable { public function jsonSerialize($depth = 1): mixed { return 1; } }"#,
        r#"Cannot add optional parameters when implementing interface method: Bag::jsonserialize"#,
    );
}

/// PHP rejects making an optional parameter required.
#[test]
fn test_error_override_makes_optional_parameter_required() {
    expect_error(
        r#"<?php class Base { public function f($a = 1) { return $a; } } class Child extends Base { public function f($a) { return $a; } }"#,
        r#"Cannot change optional parameter layout when overriding method: Child::f"#,
    );
}

/// PHP rejects dropping a parameter.
#[test]
fn test_error_override_removes_required_parameter() {
    expect_error(
        r#"<?php class P { public function f($a, $b) { return 1; } } class C extends P { public function f($a) { return 1; } }"#,
        r#"Cannot change parameter count when overriding method: C::f"#,
    );
}

/// PHP rejects a wider (contravariant) return.
#[test]
fn test_error_override_contravariant_return() {
    expect_error(
        r#"<?php class Node {} class Leaf extends Node {} class P { public function f(): Leaf { return new Leaf(); } } class C extends P { public function f(): Node { return new Node(); } }"#,
        r#"Cannot override method C::f with incompatible return type Node (parent returns Leaf)"#,
    );
}

/// Union covariance is member by member: `bool` does not narrow `false`.
#[test]
fn test_error_override_union_return_widens_member() {
    expect_error(
        r#"<?php class Node {} class Leaf extends Node {} class P { public function f(): Node|false { return false; } } class C extends P { public function f(): Leaf|bool { return true; } }"#,
        r#"Cannot override method C::f with incompatible return type Leaf|bool (parent returns Node|false)"#,
    );
}

/// `#[\ReturnTypeWillChange]` only relaxes internal parents, as in PHP.
#[test]
fn test_error_return_type_will_change_on_source_parent() {
    expect_error(
        r#"<?php class P { public function f(): int { return 1; } } class C extends P { #[\ReturnTypeWillChange] public function f(): string { return "x"; } }"#,
        r#"Cannot override method C::f with incompatible return type string (parent returns int)"#,
    );
}

/// A relaxed return that changes the parent's physical return representation is refused.
#[test]
fn test_error_return_type_will_change_changes_class_return_abi() {
    expect_error(
        r#"<?php class X extends ArrayIterator { #[\ReturnTypeWillChange] public function count(): string { return "n"; } }"#,
        r#"Cannot override method X::count with #[\ReturnTypeWillChange] return type string: elephc cannot dispatch it through the parent's native return type int"#,
    );
}

/// Interface dispatch only boxes into a `mixed` contract; a scalar contract cannot absorb a string.
#[test]
fn test_error_return_type_will_change_changes_interface_return_abi() {
    expect_error(
        r#"<?php class X implements Countable { #[\ReturnTypeWillChange] public function count(): string { return "n"; } }"#,
        r#"Cannot implement interface method X::count with #[\ReturnTypeWillChange] return type string: elephc cannot dispatch it through the interface's native return type int"#,
    );
}

/// Renaming a variadic is allowed; removing it is not.
#[test]
fn test_error_override_drops_variadic() {
    expect_error(
        r#"<?php class P { public function f(...$a) { return 1; } } class C extends P { public function f($a) { return 1; } }"#,
        r#"Cannot change variadic parameter shape when overriding method: C::f"#,
    );
}

/// PHP rejects `static|false` against `?static`.
#[test]
fn test_error_interface_static_nullable_static_vs_false() {
    expect_error(
        r#"<?php interface Maker { public static function make(string $s): ?static; } class Thing implements Maker { public static function make(string $s): static|false { return false; } }"#,
        r#"Cannot implement interface static method Thing::make with incompatible return type"#,
    );
}
