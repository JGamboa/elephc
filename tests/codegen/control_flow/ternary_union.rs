//! Purpose:
//! Regression tests for value-producing branch merges (`?:` ternary, short `?:`, `??`, `match`)
//! whose branches have DIFFERENT representations: a branch the pre-lowering type estimate could
//! not name (a property read through a `mixed` or `T|false` receiver, a constant, a static
//! property, a negated float, ...) used to be coerced into the other branch's slot type.
//!
//! Called from:
//! - `cargo test` through Rust's test harness.
//!
//! Key details:
//! - Each fixture prints what PHP 8.5 prints; the homogeneous fixtures at the end pin the
//!   pre-existing behaviour of int/int, string/string and object/object merges.

use super::*;

/// Shared declarations: a `mixed` receiver and an `R|false` receiver over the same class.
const PRELUDE: &str = r#"<?php
class R { public $n = "str"; public $i = 5; public static $s = "st"; const CI = 7; }
function gr(): mixed { return new R(); }
function uf(int $k): R|false { return $k > 9 ? false : new R(); }
$t = count($argv) < 9; $f = !$t; $rm = gr(); $ru = uf(1);
"#;

fn run(body: &str) -> String {
    compile_and_run(&format!("{PRELUDE}{body}"))
}

/// `$c ? null : $r->n` over a `mixed` `$r` printed the string payload's address as an int.
#[test]
fn test_ternary_null_or_property_of_mixed_receiver() {
    let out = run(r#"var_dump($f ? null : $rm->n); var_dump($t ? null : $rm->n);"#);
    assert_eq!(out, "string(3) \"str\"\nNULL\n");
}

/// `$r === false ? false : $r->n` over `M|false` printed `int(0)` for a string property.
#[test]
fn test_ternary_false_guard_or_property_of_union_receiver() {
    let out = compile_and_run(
        r#"<?php
class M { public string $n = "M"; public function dup(): M|false { return $this; } }
class S extends M { public function dup(): M|false { return false; } }
function go(M $m) { $r = $m->dup(); var_dump($r === false ? false : $r->n); }
go(new M()); go(new S());
"#,
    );
    assert_eq!(out, "string(1) \"M\"\nbool(false)\n");
}

/// A `static|false` method narrowed by `=== false` (the LSP-family shape).
#[test]
fn test_ternary_static_false_receiver_property() {
    let out = compile_and_run(
        r#"<?php
class M { public $n = "M"; public function dup(): static|false { return false; } }
final class A extends M { public function dup(): static { return $this; } }
function go(M $m) { $r = $m->dup(); var_dump($r === false ? false : $r->n); }
go(new M()); go(new A());
"#,
    );
    assert_eq!(out, "bool(false)\nstring(1) \"M\"\n");
}

/// Two property reads with different runtime types through one `mixed` receiver.
#[test]
fn test_ternary_two_properties_of_mixed_receiver() {
    let out = run(r#"var_dump($t ? $rm->n : $rm->i, $f ? $rm->n : $rm->i);"#);
    assert_eq!(out, "string(3) \"str\"\nint(5)\n");
}

/// Short ternary: the default arm is a property of a `T|false` receiver.
#[test]
fn test_short_ternary_property_of_union_receiver() {
    let out = run(r#"var_dump(0 ?: $ru->n, $rm->i ?: true, $ru->n ?: null);"#);
    assert_eq!(out, "string(3) \"str\"\nint(5)\nstring(3) \"str\"\n");
}

/// Null coalesce: a non-null left operand keeps its own type next to a boxed default.
#[test]
fn test_null_coalesce_mixed_property_default() {
    let out = run(r#"var_dump(true ?? $rm->n, $rm->n ?? 1, $rm?->n ?? null);"#);
    assert_eq!(out, "bool(true)\nstring(3) \"str\"\nstring(3) \"str\"\n");
}

/// `match` arms share the merge: a mixed-receiver property next to an `int` and a `false` arm.
#[test]
fn test_match_arms_mixed_property_int_false() {
    let out = run(
        r#"foreach ([0, 1, 2] as $k) {
    var_dump(match ($k) { 0 => 2, 1 => false, default => $rm->n });
}"#,
    );
    assert_eq!(out, "int(2)\nbool(false)\nstring(3) \"str\"\n");
}

/// `int` and `bool` share a register but not a PHP type; neither may relabel the other.
#[test]
fn test_ternary_int_bool_branches_keep_their_types() {
    let out = run(
        r#"var_dump($t ? 42 : false, $f ? 42 : false, $t ? true : 42, $f ? true : 42);
var_dump(match ($t) { true => 1, default => false });"#,
    );
    assert_eq!(out, "int(42)\nbool(false)\nbool(true)\nint(42)\nint(1)\n");
}

/// Constants, class constants, static properties, negation and `!` beside null/int/false.
#[test]
fn test_ternary_unestimated_expression_kinds() {
    let out = compile_and_run(
        r#"<?php
const CS = "cs";
class K { const CI = 7; public static $s = "st"; }
$t = count($argv) < 9; $fl = 2.5;
var_dump($t ? CS : 42, $t ? K::CI : "s", $t ? K::$s : null, $t ? -$fl : 1, $t ? !$t : 3);
"#,
    );
    assert_eq!(
        out,
        "string(2) \"cs\"\nint(7)\nstring(2) \"st\"\nfloat(-2.5)\nbool(false)\n"
    );
}

/// The merged value in every value context: assignment, return, argument, echo, `===`, and
/// string interpolation.
#[test]
fn test_ternary_union_value_contexts() {
    let out = run(
        r#"function fr($t, $rm) { return $t ? $rm->n : null; }
function fa($x) { var_dump($x); }
$v = $f ? false : $rm->n;
var_dump($v, fr($t, $rm));
fa($t ? $ru->n : 1);
echo $t ? $rm->n : 0, "\n";
var_dump(($t ? $rm->n : null) === "str");
$w = $t ? $ru->n : false;
echo "[$w]\n";
"#,
    );
    assert_eq!(
        out,
        "string(3) \"str\"\nstring(3) \"str\"\nstring(3) \"str\"\nstr\nbool(true)\n[str]\n"
    );
}

/// Merges used as array-literal elements (indexed and associative) keep each arm's value,
/// including a nullable merge, which an array slot holds boxed.
#[test]
fn test_ternary_union_array_literal_elements() {
    let out = run(
        r#"$x = [$t ? $rm->n : null, $t ? 7 : null, $f ?: null, $rm->i ?? "d"];
$y = ["k" => match ($t) { true => $ru->n, default => 0 }, "j" => $f ? 1 : null];
echo json_encode($x), json_encode($y), "\n";
"#,
    );
    assert_eq!(out, "[\"str\",7,null,5]{\"k\":\"str\",\"j\":null}\n");
}

/// Pre-existing behaviour: homogeneous int/int, string/string and object/object merges.
#[test]
fn test_ternary_homogeneous_branches_unchanged() {
    let out = compile_and_run(
        r#"<?php
class P { public function __construct(public int $v) {} }
function pick(bool $c, int $a, int $b): int { return $c ? $a : $b; }
function name(bool $c, string $a, string $b): string { return $c ? $a : $b; }
function obj(bool $c, P $a, P $b): P { return $c ? $a : $b; }
function sum(int $n): int { $s = 0; for ($i = 0; $i < $n; $i++) { $s += $i % 2 ? $i : -$i; } return $s; }
$t = count($argv) < 9;
var_dump(pick($t, 1, 2), pick(!$t, 1, 2), name($t, "a", "b"), obj(!$t, new P(1), new P(2))->v);
var_dump(match ($t) { true => "x", default => "y" }, ($t ? 3 : 0) ?: 9, sum(10));
"#,
    );
    assert_eq!(
        out,
        "int(1)\nint(2)\nstring(1) \"a\"\nint(2)\nstring(1) \"x\"\nint(3)\nint(5)\n"
    );
}
