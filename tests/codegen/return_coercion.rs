//! Purpose:
//! End-to-end coverage for coercion at the function RETURN boundary: values whose static type is
//! `mixed`, a scalar union, `?int` or another scalar representation, returned through a declared
//! scalar type in coercive and `strict_types` mode, plus the inferred return type of undeclared
//! functions and the defaults of untyped / `self::class` parameters.
//!
//! Called from:
//! - The native codegen suite (`tests/codegen/mod.rs`).
//!
//! Key details:
//! - Expected output was produced by PHP 8.5 from the same source.
//! - Every non-coercible case catches the `TypeError` and prints its message, so a silently
//!   converted value changes the output.

use crate::support::*;

/// Verifies a `mixed` value returned through a declared `bool`, `string` or `float` follows PHP's
/// coercive-mode rules: string truthiness for `bool`, numeric strings for `float`, and a
/// catchable `TypeError` for null and arrays instead of a silently converted value.
#[test]
fn test_return_coercion_mixed_to_scalar_weak() {
    let out = compile_and_run(
        r#"<?php
function b(mixed $m): bool { return $m; }
function s(mixed $m): string { return $m; }
function fl(mixed $m): float { return $m; }
function t($f, $v) { try { var_dump($f($v)); } catch (TypeError $e) { echo $e->getMessage(), "\n"; } }
foreach (["x", "", "0", 0, 2, 1.5, true, null, [1]] as $v) { t('b', $v); }
foreach ([7, 1.5, true, false, "s", null] as $v) { t('s', $v); }
foreach ([7, "1.5", " 2", true, "abc", null] as $v) { t('fl', $v); }
"#,
    );
    assert_eq!(
        out,
        "bool(true)\nbool(false)\nbool(false)\nbool(false)\nbool(true)\nbool(true)\nbool(true)\nb(): Return value must be of type bool, null returned\nb(): Return value must be of type bool, array returned\nstring(1) \"7\"\nstring(3) \"1.5\"\nstring(1) \"1\"\nstring(0) \"\"\nstring(1) \"s\"\ns(): Return value must be of type string, null returned\nfloat(7)\nfloat(1.5)\nfloat(2)\nfloat(1)\nfl(): Return value must be of type float, string returned\nfl(): Return value must be of type float, null returned\n"
    );
}

/// Verifies a `mixed` value returned through `int|string`, `int|float`, `string|bool` and
/// `string|false` keeps an exact member, picks PHP's preferred member otherwise, and throws
/// `TypeError` naming the declared union the way PHP spells it.
#[test]
fn test_return_coercion_mixed_to_scalar_union_weak() {
    let out = compile_and_run(
        r#"<?php
function is_(mixed $m): int|string { return $m; }
function if_(mixed $m): int|float { return $m; }
function sb(mixed $m): string|bool { return $m; }
function sf(mixed $m): string|false { return $m; }
function t($f, $v) { try { var_dump($f($v)); } catch (TypeError $e) { echo $e->getMessage(), "\n"; } }
foreach ([3, "3", 2.0, true, null] as $v) { t('is_', $v); }
foreach ([3, "3", "1.5", "1e2", "abc", false] as $v) { t('if_', $v); }
foreach ([3, 1.5, "s", true, false, null] as $v) { t('sb', $v); }
foreach (["s", false, true, 4] as $v) { t('sf', $v); }
"#,
    );
    assert_eq!(
        out,
        "int(3)\nstring(1) \"3\"\nint(2)\nint(1)\nis_(): Return value must be of type string|int, null returned\nint(3)\nint(3)\nfloat(1.5)\nfloat(100)\nif_(): Return value must be of type int|float, string returned\nint(0)\nstring(1) \"3\"\nstring(3) \"1.5\"\nstring(1) \"s\"\nbool(true)\nbool(false)\nsb(): Return value must be of type string|bool, null returned\nstring(1) \"s\"\nbool(false)\nstring(1) \"1\"\nstring(1) \"4\"\n"
    );
}

/// Verifies `?int` (tagged scalar storage), `?float`, `?string` and `?bool` returns from a
/// `mixed` value: null passes, scalars coerce, and non-coercible values throw `TypeError`.
#[test]
fn test_return_coercion_mixed_to_nullable_scalar() {
    let out = compile_and_run(
        r#"<?php
function ni(mixed $m): ?int { return $m; }
function nf(mixed $m): ?float { return $m; }
function ns(mixed $m): ?string { return $m; }
function nb(mixed $m): ?bool { return $m; }
function t($f, $v) { try { var_dump($f($v)); } catch (TypeError $e) { echo $e->getMessage(), "\n"; } }
foreach ([null, 5, "6", 2.0, true, "x", [1]] as $v) { t('ni', $v); }
foreach ([null, 5, "6.5", false] as $v) { t('nf', $v); }
foreach ([null, 5, 2.5, true] as $v) { t('ns', $v); }
foreach ([null, 0, "a", 0.0] as $v) { t('nb', $v); }
$x = ni(7); echo $x + 1, "\n";
"#,
    );
    assert_eq!(
        out,
        "NULL\nint(5)\nint(6)\nint(2)\nint(1)\nni(): Return value must be of type ?int, string returned\nni(): Return value must be of type ?int, array returned\nNULL\nfloat(5)\nfloat(6.5)\nfloat(0)\nNULL\nstring(1) \"5\"\nstring(3) \"2.5\"\nstring(1) \"1\"\nNULL\nbool(false)\nbool(true)\nbool(false)\n8\n"
    );
}

/// Verifies `declare(strict_types=1)` return checks: only the exact type passes (plus the
/// int-to-float widening), and every other runtime value throws PHP's `TypeError`.
#[test]
fn test_return_coercion_strict_types_throws() {
    let out = compile_and_run(
        r#"<?php
declare(strict_types=1);
function i(mixed $m): int { return $m; }
function f(mixed $m): float { return $m; }
function b(mixed $m): bool { return $m; }
function s(mixed $m): string { return $m; }
function u(mixed $m): int|string { return $m; }
function n(mixed $m): ?int { return $m; }
function t($f, $v) { try { var_dump($f($v)); } catch (TypeError $e) { echo $e->getMessage(), "\n"; } }
foreach ([4, 4.0, "4", true] as $v) { t('i', $v); }
foreach ([4, 4.5, "4"] as $v) { t('f', $v); }
foreach ([true, 1, "1"] as $v) { t('b', $v); }
foreach (["s", 1, 1.5] as $v) { t('s', $v); }
foreach ([1, "1", 1.5, false] as $v) { t('u', $v); }
foreach ([null, 1, "1"] as $v) { t('n', $v); }
"#,
    );
    assert_eq!(
        out,
        "int(4)\ni(): Return value must be of type int, float returned\ni(): Return value must be of type int, string returned\ni(): Return value must be of type int, true returned\nfloat(4)\nfloat(4.5)\nf(): Return value must be of type float, string returned\nbool(true)\nb(): Return value must be of type bool, int returned\nb(): Return value must be of type bool, string returned\nstring(1) \"s\"\ns(): Return value must be of type string, int returned\ns(): Return value must be of type string, float returned\nint(1)\nstring(1) \"1\"\nu(): Return value must be of type string|int, float returned\nu(): Return value must be of type string|int, false returned\nNULL\nint(1)\nn(): Return value must be of type ?int, string returned\n"
    );
}

/// Verifies returns whose static type is another scalar representation, a scalar union or a
/// heterogeneous ternary/match (`bool` from an int literal, `string` from
/// `int|float`, `int|string` from `float|string`) coerce like PHP, and that methods throw
/// `TypeError` with the class-qualified message when the value cannot coerce.
#[test]
fn test_return_coercion_static_other_representation() {
    let out = compile_and_run(
        r#"<?php
function b1(): bool { return 3; }
function u3(int|float $v): string { return $v; }
function rb(bool $c): bool { return $c ? 0 : "x"; }
function ru(bool $c): int|string { $m = $c ? 3.0 : "s"; return $m; }
function rm(int $k): int|string { return match ($k) { 1 => 1, default => 3.0 }; }
function ri(bool $c): int { return $c ? "42" : [7]; }
function rf(bool $c): float { return $c ? 3 : "1.5"; }
class C { public function m(mixed $v): int { return $v; } public static function n(mixed $v): bool { return $v; } }
var_dump(b1(), u3(1.5), u3(2), rb(true), rb(false), ru(true), ru(false), rm(2), ri(true), rf(true), rf(false));
try { ri(false); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { (new C)->m("q"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { C::n([]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(C::n("q"));
"#,
    );
    assert_eq!(
        out,
        "bool(true)\nstring(3) \"1.5\"\nstring(1) \"2\"\nbool(false)\nbool(true)\nint(3)\nstring(1) \"s\"\nint(3)\nint(42)\nfloat(3)\nfloat(1.5)\nri(): Return value must be of type int, array returned\nC::m(): Return value must be of type int, string returned\nC::n(): Return value must be of type bool, array returned\nbool(true)\n"
    );
}

/// Verifies an undeclared return mixing int with string, float or array branches infers
/// `mixed` and keeps each value, instead of converting the int to the other branch's type.
#[test]
fn test_inferred_return_keeps_branch_types() {
    let out = compile_and_run(
        r#"<?php
function g(int $k) { if ($k == 0) { return 1; } return "s"; }
function h(int $k) { if ($k == 0) { return 1; } return 1.5; }
function a(int $k) { if ($k == 0) { return [1]; } return "s"; }
class K { public function m(int $k) { if ($k) { return 2; } return "two"; } }
var_dump(g(0), g(1), h(0), h(1), a(0), a(1), (new K)->m(1), (new K)->m(0));
"#,
    );
    assert_eq!(
        out,
        "int(1)\nstring(1) \"s\"\nint(1)\nfloat(1.5)\narray(1) {\n  [0]=>\n  int(1)\n}\nstring(1) \"s\"\nint(2)\nstring(3) \"two\"\n"
    );
}

/// Verifies an untyped parameter with a `null` default keeps reading NULL when another call
/// site specializes it with a float argument, whether the null is explicit or the omitted
/// default, for functions and methods, in either call order.
#[test]
fn test_untyped_null_default_param_with_float_call() {
    let out = compile_and_run(
        r#"<?php
function f($a, $b, $c = null) { var_dump($c); }
function g($s = null) { var_dump($s); }
function h($t = null) { var_dump($t); }
function r($u = null) { var_dump($u); }
class K { public function m($v = null) { var_dump($v); } }
f(1, 'x');
f(1, 'x', null);
f(1, 'x', 2.5);
g(); g("str");
h(); h(true);
r(0.5); r();
(new K)->m(1.5); (new K)->m();
"#,
    );
    assert_eq!(
        out,
        "NULL\nNULL\nfloat(2.5)\nNULL\nstring(3) \"str\"\nNULL\nbool(true)\nfloat(0.5)\nNULL\nfloat(1.5)\nNULL\n"
    );
}

/// Verifies `self::class` in a parameter default names the declaring class, including for
/// calls through a subclass and inside an array default, instead of the literal `self`.
#[test]
fn test_param_default_self_class_resolves_declaring_class() {
    let out = compile_and_run(
        r#"<?php
class K {
    public static function make($x = self::class) { return $x; }
    public function m($y = self::class, $z = [self::class]) { return [$y, $z[0]]; }
}
class L extends K {}
var_dump(K::make(), L::make(), (new K)->m(), (new L)->m());
"#,
    );
    assert_eq!(
        out,
        "string(1) \"K\"\nstring(1) \"K\"\narray(2) {\n  [0]=>\n  string(1) \"K\"\n  [1]=>\n  string(1) \"K\"\n}\narray(2) {\n  [0]=>\n  string(1) \"K\"\n  [1]=>\n  string(1) \"K\"\n}\n"
    );
}

/// Verifies a call whose result is unused still runs the declared return check: the optimizer
/// must not delete `f("z");` as a pure call, nor the `catch (TypeError)` around it.
#[test]
fn test_return_coercion_unused_call_still_throws() {
    let out = compile_and_run(
        r#"<?php
function f(mixed $m): int { return $m; }
function g(mixed $m): bool { return $m; }
try { f("z"); echo "no\n"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { g(null); echo "no\n"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
"#,
    );
    assert_eq!(
        out,
        "f(): Return value must be of type int, string returned\ng(): Return value must be of type bool, null returned\n"
    );
}

/// Verifies the return check in a program that uses `eval`, where every function grows a hidden
/// `func_args` parameter: the helper call must go through the ordinary user-call path.
#[test]
fn test_return_coercion_in_eval_program() {
    let out = compile_and_run(
        r#"<?php
function f(mixed $m): bool { return $m; }
function g(mixed $m): ?int { return $m; }
eval('$z = 1;');
var_dump(f("x"), f(""), g("5"), g(null));
try { f([]); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
"#,
    );
    assert_eq!(
        out,
        "bool(true)\nbool(false)\nint(5)\nNULL\nf(): Return value must be of type bool, array returned\n"
    );
}
