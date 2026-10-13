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
        r#"Cannot override method X::count: #[\ReturnTypeWillChange] return type string cannot be dispatched through the parent's native return type int"#,
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

/// Review p40: `Foo` -> `?Foo = null` would read the parent's object word as a boxed value.
#[test]
fn test_error_override_widens_object_param_to_nullable_default() {
    expect_error(
        r#"<?php class Foo { public $v = "foo"; } class Base { public function f(Foo $x): string { return "B"; } } class Child extends Base { public function f(?Foo $x = null): string { return "C"; } }"#,
        r#"Cannot widen parameter representation when overriding method: Child::f parameter $x (the parent passes Foo, the override expects Foo|null"#,
    );
}

/// Review p42: int -> int|string crashed (exit 139).
#[test]
fn test_error_override_widens_int_param_to_union_default() {
    expect_error(
        r#"<?php class Base { public function f(int $x): string { return "B$x"; } } class Child extends Base { public function f(int|string $x = 5): string { return "C$x"; } }"#,
        r#"Cannot widen parameter representation when overriding method: Child::f parameter $x (the parent passes int, the override expects int|string"#,
    );
}

/// Review p43: an untyped override parameter is typed later from call sites, so it cannot replace a declared one.
#[test]
fn test_error_override_drops_declared_param_type() {
    expect_error(
        r#"<?php class Base { public function f(int $x, array $o): string { return "B"; } } class Child extends Base { public function f($x = 1, $o = []): string { return "C"; } }"#,
        r#"Cannot widen parameter representation when overriding method: Child::f parameter $x (the parent passes int, the override drops the declared type"#,
    );
}

/// Review p41 (pre-existing miscompile, printed `Cnull`): the representation rule covers every parameter, not only relaxed ones.
#[test]
fn test_error_override_widens_param_without_default() {
    expect_error(
        r#"<?php class Foo {} class Base { public function f(Foo $x): string { return "B"; } } class Child extends Base { public function f(?Foo $x): string { return "C"; } }"#,
        r#"Cannot widen parameter representation when overriding method: Child::f parameter $x"#,
    );
}

/// Review p24: an untyped RTWC override of a `mixed` builtin crashed (SIGSEGV).
#[test]
fn test_error_return_type_will_change_untyped_over_mixed_builtin() {
    expect_error(
        r#"<?php class X extends ArrayIterator { #[\ReturnTypeWillChange] public function current() { return "cur"; } }"#,
        r#"Cannot override method X::current: an untyped #[\ReturnTypeWillChange] override cannot be dispatched through the parent's native return type mixed"#,
    );
}

/// RTWC always requires the parent's return representation, even when the types are compatible.
#[test]
fn test_error_return_type_will_change_typed_over_mixed_builtin() {
    expect_error(
        r#"<?php class X extends ArrayIterator { #[\ReturnTypeWillChange] public function current(): string { return "cur"; } }"#,
        r#"Cannot override method X::current: #[\ReturnTypeWillChange] return type string cannot be dispatched through the parent's native return type mixed"#,
    );
}

/// Review p04 (pre-existing miscompile): calls through the parent passed the parent's default.
#[test]
fn test_error_override_changes_inherited_default() {
    expect_error(
        r#"<?php class A { public function f(int $n, string $u = "a"): string { return "A$n$u"; } } class B extends A { public function f(int $n, string $u = "b"): string { return "B$n$u"; } }"#,
        r#"Cannot change the default value of parameter $u when overriding method: B::f (not yet supported until P10"#,
    );
}

/// Same rule for interface contracts: the interface's default is filled at the call site.
#[test]
fn test_error_interface_impl_changes_inherited_default() {
    expect_error(
        r#"<?php interface I { public function f(int $n = 1): string; } class C implements I { public function f(int $n = 2): string { return "C$n"; } }"#,
        r#"Cannot change the default value of parameter $n when implementing interface method: C::f"#,
    );
}

/// Review n7c: a renamed variadic still has to keep the parent's element type (crashed with exit 139).
#[test]
fn test_error_override_variadic_renamed_with_incompatible_type() {
    expect_error(
        r#"<?php class P { public function f(int ...$a) { return count($a); } } class C extends P { public function f(string ...$b) { return 1; } }"#,
        r#"Cannot widen parameter representation when overriding method: C::f parameter $b"#,
    );
}

/// Variadic parameters are contravariant: a narrower element type is rejected, as in PHP.
#[test]
fn test_error_override_variadic_narrows_object_type() {
    expect_error(
        r#"<?php class Foo {} class Bar extends Foo {} class P { public function f(Foo ...$a) { return 1; } } class C extends P { public function f(Bar ...$b) { return 1; } }"#,
        r#"Cannot narrow variadic parameter $b when overriding method: C::f"#,
    );
}

/// Review p33: string conversion calls `__toString()` statically, so the implicit `: string` is not applied when a subclass overrides it.
#[test]
fn test_error_untyped_tostring_overridden_by_subclass_keeps_contract_error() {
    expect_error(
        r#"<?php class N implements Stringable { public function __toString() { return "n"; } } class M extends N { public function __toString() { return "m"; } }"#,
        r#"Cannot implement interface method N::__tostring without declaring a compatible return type"#,
    );
}
