//! Purpose:
//! End-to-end coverage for front P49: the checker's fixpoints widen by a lattice JOIN
//! (`crate::types::checker::lattice`), so they converge on Laravel (E6-bis), and untyped
//! parameters join every call-site type, closures included (P47).
//!
//! Called from:
//! - The native codegen suite (`tests/codegen/mod.rs`).
//!
//! Key details:
//! - Each source is a probe of `probes/checker-converge/` in the elephc-laravel workspace;
//!   the expected output was produced by PHP 8.5 from the same source.

use crate::support::*;

/// `__callStatic`'s declared `array $parameters` seen with an int, a float and an int again joins to `array<mixed>` instead of re-adopting `array<int>` (E6-bis, Carbon).
#[test]
fn test_checker_converge_magic_callstatic_int_float_widen() {
    let out = compile_and_run(r#"<?php
// E6-bis repro: __callStatic's declared `array $parameters` seen with int and float.
class CI
{
    public static function __callStatic(string $method, array $parameters)
    {
        return $method . ":" . $parameters[0];
    }
}
function later() { return CI::secs(3); }
echo CI::days(1), "\n";
echo CI::hours(1.5), "\n";
echo CI::mins(2), " ", later(), "\n";
"#);
    assert_eq!(out, "days:1\nhours:1.5\nmins:2 secs:3\n");
}

/// An untyped property filled by constructor calls with a parent and a child class joins to the parent, and scalars of different tags join to mixed (E6-bis, HttpResponseException).
#[test]
fn test_checker_converge_ctor_untyped_property_joins_parent_and_child() {
    let out = compile_and_run(r#"<?php
// E6-bis repro: an untyped property filled from constructor calls with parent and child.
class R { public function name() { return "R"; } }
class IR extends R { public function name() { return "IR"; } }
class HRE
{
    protected $response;
    public function __construct(R $response) { $this->response = $response; }
    public function getResponse() { return $this->response; }
}
function make() { return new HRE(new IR()); }
$a = new HRE(new R());
echo $a->getResponse()->name(), " ", make()->getResponse()->name(), "\n";
class A { public $v; public function __construct($v) { $this->v = $v; } }
class B extends A {}
class C extends A {}
echo gettype((new A(1))->v), " ", gettype((new B("s"))->v), " ", gettype((new C(1.5))->v), "\n";
"#);
    assert_eq!(out, "R IR\ninteger string double\n");
}

/// Laravel's `value()` called with closures and with scalars keeps every value (P47).
#[test]
fn test_untyped_param_value_helper_closure_and_scalars() {
    let out = compile_and_run(r#"<?php
// Laravel Collections/helpers.php, literal.
function value($value, ...$args)
{
    return $value instanceof Closure ? $value(...$args) : $value;
}
echo value(fn () => 5), " ", value(5), " ", value('x'), "\n";
echo value(fn ($a, $b) => $a + $b, 2, 3), " ", value(1.5), " ", value(true) ? "t" : "f", "\n";
$o = new ArrayObject([1, 2]);
echo get_class(value($o)), " ", count(value(fn () => [1, 2, 3])), "\n";
var_dump(value(null));
"#);
    assert_eq!(out, "5 5 x\n5 1.5 t\nArrayObject 3\nNULL\n");
}

/// Laravel's `tap()` called with and without a callback: the untyped `$callback = null` is `callable|null` (P47).
#[test]
fn test_untyped_param_tap_helper_callable_or_null() {
    let out = compile_and_run(r#"<?php
class HigherOrderTapProxy
{
    public $target;
    public function __construct($target) { $this->target = $target; }
}
// Laravel Support/helpers.php, literal.
function tap($value, $callback = null)
{
    if (is_null($callback)) {
        return new HigherOrderTapProxy($value);
    }

    $callback($value);

    return $value;
}
echo tap(5, function ($v) { echo "saw ", $v, "\n"; }), "\n";
echo tap('t', fn ($v) => null), "\n";
echo tap(2.5, fn ($v) => null), "\n";
echo get_class(tap(7)), " ", tap(7)->target, "\n";
"#);
    assert_eq!(out, "saw 5\n5\nt\n2.5\nHigherOrderTapProxy 7\n");
}

/// Laravel's `with()` with and without a callback (P47).
#[test]
fn test_untyped_param_with_helper_nullable_callable() {
    let out = compile_and_run(r#"<?php
// Laravel Support/helpers.php, literal.
function with($value, ?callable $callback = null)
{
    return is_null($callback) ? $value : $callback($value);
}
echo with(5), " ", with('s'), " ", with(3, fn ($v) => $v * 2), " ", with('a', fn ($v) => strtoupper($v)), "\n";
echo with(1.25), " ", count(with([1, 2])), " ", with(2.5, fn ($v) => $v * 2), "\n";
"#);
    assert_eq!(out, "5 s 6 A\n1.25 2 5\n");
}

/// A reduced `data_get()` over arrays, objects and closure defaults (P47).
#[test]
fn test_untyped_param_data_get_helper_mixed_targets() {
    let out = compile_and_run(r#"<?php
function value($value, ...$args) { return $value instanceof Closure ? $value(...$args) : $value; }
// Laravel data_get, reduced to the array/object/default paths (no wildcard, no Collection).
function data_get($target, $key, $default = null)
{
    if (is_null($key)) {
        return $target;
    }
    $key = is_array($key) ? $key : explode('.', $key);
    foreach ($key as $segment) {
        if (is_array($target) && array_key_exists($segment, $target)) {
            $target = $target[$segment];
        } elseif (is_object($target) && isset($target->{$segment})) {
            $target = $target->{$segment};
        } else {
            return value($default);
        }
    }
    return $target;
}
$data = ['a' => ['b' => 5, 'c' => 'str', 'f' => 1.5], 'o' => (object) ['p' => 'prop']];
echo data_get($data, 'a.b'), " ", data_get($data, 'a.c'), " ", data_get($data, 'a.f'), "\n";
echo data_get($data, 'o.p'), " ", data_get($data, 'x.y', 'dflt'), " ", data_get($data, 'x', fn () => 'lazy'), "\n";
echo data_get($data, ['a', 'b']), " ", count(data_get($data, null)), "\n";
"#);
    assert_eq!(out, "5 str 1.5\nprop dflt lazy\n5 2\n");
}

/// Helpers that receive mixed and return it unchanged keep each value's tag (P47).
#[test]
fn test_untyped_param_identity_helpers_keep_tags() {
    let out = compile_and_run(r#"<?php
// Helpers that receive mixed and return the same value.
function id(mixed $v): mixed { return $v; }
function pass($v) { return $v; }
function first_or($v, $d = null) { return $v ?? $d; }
echo id(1), id('a'), id(2.5), " ", pass(3), pass('b'), pass(1.5), "\n";
echo first_or(null, 'd'), first_or('x'), first_or(4), "\n";
var_dump(pass(true), id(null), pass([1])[0]);
"#);
    assert_eq!(out, "1a2.5 3b1.5\ndx4\nbool(true)\nNULL\nint(1)\n");
}

/// `(string)` over a `Stringable` (or user interface) parameter calls the runtime class's `__toString` (P47).
#[test]
fn test_stringable_cast_dispatches_runtime_tostring() {
    let out = compile_and_run(r#"<?php
class N implements Stringable { public function __toString(): string { return "n!"; } }
class M { public function __toString(): string { return "m!"; } }
function s(Stringable $x) { return (string) $x; }
function t(Stringable $x) { return "[" . $x . "]"; }
echo s(new N()), " ", t(new N()), " ", s(new M()), "\n";
interface Named { public function __toString(): string; }
class P implements Named { public function __toString(): string { return "p!"; } }
function u(Named $x): string { return (string) $x; }
echo u(new P()), "\n";
"#);
    assert_eq!(out, "n! [n!] m!\np!\n");
}

/// An untyped method variadic called with a string, an int, a float and a bool keeps each
/// value: the collector element is the lattice join (`mixed`), never `string`.
#[test]
fn test_checker_converge_untyped_method_variadic_mixed_scalars() {
    let out = compile_and_run(r#"<?php
class Cat { public static function cat(...$xs) { $s = ""; foreach ($xs as $x) { $s .= var_export($x, true) . ","; } return $s; } }
echo Cat::cat('x', 1, 2.5, true), "\n";
echo Cat::cat(3), "\n";
"#);
    assert_eq!(out, "'x',1,2.5,true,\n3,\n");
}
