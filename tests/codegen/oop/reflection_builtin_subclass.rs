//! Purpose:
//! Regressions for reflecting user subclasses of PHP's builtin Reflection classes and for
//! compiler-generated members that PHP's Reflection API never lists.
//!
//! Called from:
//! - `tests/codegen/oop.rs` in the codegen integration test suite.
//!
//! Key details:
//! - `new ReflectionClass(<name>)` materializes its whole metadata graph eagerly, and every
//!   ReflectionMethod embeds a declaring-class ReflectionClass. Listing the private `__*`
//!   backing slots of the builtin Reflection classes in each of those nested reflectors made
//!   `new ReflectionClass('Mine')` over `class Mine extends ReflectionClass {}` exhaust the
//!   default 8 MiB heap.
//! - Expected outputs were measured on PHP 8.5.

use crate::support::compile_and_run;

/// Minimal reproducer: reflecting a user subclass of ReflectionClass must fit the default heap.
#[test]
fn reflecting_a_reflection_class_subclass_fits_the_default_heap() {
    let out = compile_and_run(
        r#"<?php
class Mine extends ReflectionClass {}
echo (new ReflectionClass('Mine'))->getName(), "\n";
"#,
    );
    assert_eq!(out, "Mine\n");
}

/// The private `__*` storage slots elephc declares on builtin Reflection classes are not PHP
/// properties, and the `name` property's generated get accessor is not a PHP method.
#[test]
fn reflection_class_backing_slots_and_name_accessor_are_not_reflected() {
    let out = compile_and_run(
        r#"<?php
class Mine extends ReflectionClass {}
$mine = new ReflectionClass('Mine');
$declaring = $mine->getMethod('getName')->getDeclaringClass();
echo $declaring->getName(), "\n";
var_dump($declaring->getDefaultProperties());
var_dump($declaring->hasProperty('__name'));
var_dump($mine->hasMethod('__propget_name'));
"#,
    );
    assert_eq!(
        out,
        "ReflectionClass\narray(0) {\n}\nbool(false)\nbool(false)\n"
    );
}

/// A user property hook compiles to `__propget_*` / `__propset_*` methods that PHP's
/// `getMethods()` and `hasMethod()` never report.
#[test]
fn property_hook_accessors_are_not_reflected_as_methods() {
    let out = compile_and_run(
        r#"<?php
class Hooked {
    public string $label { get => "x"; set { } }
    public function run(): void {}
}
$names = [];
foreach ((new ReflectionClass('Hooked'))->getMethods() as $method) {
    $names[] = $method->getName();
}
echo implode(",", $names), "\n";
var_dump((new ReflectionClass('Hooked'))->hasMethod('__propget_label'));
var_dump((new ReflectionClass('Hooked'))->hasMethod('__propset_label'));
"#,
    );
    assert_eq!(out, "run\nbool(false)\nbool(false)\n");
}
