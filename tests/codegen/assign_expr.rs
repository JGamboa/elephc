//! Purpose:
//! End-to-end coverage for assignments used as the right operand of an operator
//! (`a ?: $o->p = v`, `a || $x[k] = v`, `!$x = f()`, `$a + $b = 3`), call-result dimension
//! writes (`f()[k] = v`), `ArrayAccess` writes used as values (`return $this['env'] = v`) and
//! the lazy-edge join of locals an operand retypes.
//!
//! Called from:
//! - The native codegen suite (`tests/codegen/mod.rs`).
//!
//! Key details:
//! - Expected output was produced by PHP 8.5 from the same source.
//! - Every right operand has an observable effect, so a short-circuit that still evaluated it
//!   (or one that skipped it wrongly) changes the output.

use crate::support::*;

/// Verifies Symfony `UnicodeString`'s `a ?: $o->p = v` statement, the expression form, and that
/// the assignment is skipped when the left side is truthy.
#[test]
fn test_short_ternary_property_assignment_operand() {
    let out = compile_and_run(
        r#"<?php
        class S { public $string = "a"; }
        function chk($v) { echo "chk|"; return $v; }
        function val($v) { echo "val|"; return $v; }
        $s = new S;
        chk(false) ?: $s->string = val("b");
        echo $s->string, "\n";
        chk(true) ?: $s->string = val("c");
        echo $s->string, "\n";
        $r = chk(0) ?: $s->string = val("d");
        echo $r, $s->string, "\n";
        "#,
    );
    assert_eq!(out, "chk|val|b\nchk|b\nchk|val|dd\n");
}

/// Verifies `||`, `&&`, `or`, `and` with property, dimension, append and static-property
/// assignments on the right, in statement and expression position.
#[test]
fn test_logical_operator_assignment_operands_short_circuit() {
    let out = compile_and_run(
        r#"<?php
        class O { public $p = 0; public static $s = 1; }
        function v($x) { echo "v($x)|"; return $x; }
        $o = new O; $a = ['k' => 0]; $list = [1];
        false || $o->p = v(1);
        true || $o->p = v(2);
        true && $a['k'] = v(3);
        false && $a['k'] = v(4);
        true && $list[] = v(5);
        false && $list[] = v(6);
        $r = (false or O::$s = v(7));
        echo "\n", $o->p, $a['k'], implode(",", $list), O::$s, var_export($r, true), "\n";
        $r = true && $list[] = v(8);
        echo count($list), var_export($r, true), "\n";
        "#,
    );
    assert_eq!(out, "v(1)|v(3)|v(5)|v(7)|\n131,57true\nv(8)|3true\n");
}

/// Verifies `??`, `??=` and compound assignments as right operands, and that the right side
/// of `??` stays lazy.
#[test]
fn test_coalesce_and_compound_assignment_operands() {
    let out = compile_and_run(
        r#"<?php
        class O { public $p = null; public $n = 2; public static $s = "a"; }
        function v($x) { echo "v($x)|"; return $x; }
        $o = new O; $null = null; $three = 3;
        $r = $null ?? $o->p = v("x"); echo $r, $o->p, "\n";
        $r = $three ?? $o->p = v("y"); echo $r, $o->p, "\n";
        $r = false ?: $o->p ??= v("z"); echo $r, "\n";
        $r = true && $o->n *= v(3); echo var_export($r, true), $o->n, "\n";
        $r = null ?? O::$s .= v("b"); echo $r, O::$s, "\n";
        "#,
    );
    assert_eq!(out, "v(x)|xx\n3x\nx\nv(3)|true6\nv(b)|abab\n");
}

/// Verifies unary operators and casts applied to an assignment (`!$x = f()`, `-$n = 3`,
/// `(int) $s = "5x"`) and the `while (!$t = f())` idiom.
#[test]
fn test_unary_operator_assignment_operands() {
    let out = compile_and_run(
        r#"<?php
        function f(string $x): string { return $x; }
        function i(int $x): int { return $x; }
        class O { public $p; }
        if (!$x = f("")) { echo "falsy[", $x, "]\n"; }
        $r = -$n = i(3); echo $r, " ", $n, "\n";
        $r = (int) $s = f("5x"); echo $r, " ", $s, "\n";
        $r = ~$b = i(5); echo $r, "\n";
        $o = new O;
        $r = !$o->p = f("1"); echo var_export($r, true), $o->p, "\n";
        while (!$t = i(1)) {}
        echo $t, "\n";
        "#,
    );
    assert_eq!(out, "falsy[]\n-3 3\n5 5x\n-6\nfalse1\n1\n");
}

/// Verifies `$a + $b = 3` and friends bind the assignment to the variable before it, while the
/// usual precedence of `=` is unchanged.
#[test]
fn test_binary_operator_assignment_operand_and_normal_precedence() {
    let out = compile_and_run(
        r#"<?php
        $a = 1; $c = 3;
        $x = $a + $b = 3; echo $x, $b, "\n";
        $x = "p" . $t = "a" . "b"; echo $x, $t, "\n";
        $x = 2 == $e = 2; echo var_export($x, true), "\n";
        $x = $v = 1 xor $w = 1; echo var_export($x, true), $v, $w, "\n";
        $b = 2;
        $y = $b + $c; echo $y, "\n";
        $y = $b ?: $c; echo $y, "\n";
        $y = $z = 3; echo $y, $z, "\n";
        $y = $b and false; echo $y, "\n";
        "#,
    );
    assert_eq!(out, "43\npabab\ntrue\n111\n5\n2\n33\n2\n");
}

/// Verifies `=&` as a right operand binds only when evaluated, and the long ternary runs only
/// the branch it takes.
#[test]
fn test_reference_and_ternary_assignment_operands() {
    let out = compile_and_run(
        r#"<?php
        function v($x) { echo "v($x)|"; return $x; }
        $y = 1; $q = 1; $w = 0;
        $r = false || $z = &$y;
        $z = 5;
        true || $w = &$q;
        $q = 2;
        echo $y, $w, "\n";
        $arr = ['x' => 0, 'y' => 0];
        $c = 0;
        $c ? $arr['x'] = v(1) : $arr['y'] = v(2);
        echo $arr['x'], $arr['y'], "\n";
        "#,
    );
    assert_eq!(out, "50\nv(2)|02\n");
}

/// Verifies `f()[] = v` / `f()[k] = v` write through an `ArrayAccess` result once, are a
/// discarded temporary for an array result, and throw PHP's errors for an object or scalar.
#[test]
fn test_call_result_dimension_writes() {
    let out = compile_and_run(
        r#"<?php
        class Bag implements ArrayAccess {
            public $log = "";
            function offsetExists($k): bool { return false; }
            function offsetGet($k): mixed { echo "get|"; return null; }
            function offsetSet($k, $v): void { $this->log .= var_export($k, true) . "=" . $v . ";"; }
            function offsetUnset($k): void {}
        }
        $bag = new Bag;
        function bag() { global $bag; echo "bag|"; return $bag; }
        function arr() { echo "arr|"; return [1]; }
        function k() { echo "k|"; return "x"; }
        function v($x) { echo "v($x)|"; return $x; }
        bag()[] = v(1);
        bag()[k()] = v(2);
        arr()[k()] = v(3);
        $r = (bag()['y'] = v(4)); echo $r, "\n";
        false && bag()['z'] = v(5);
        echo $bag->log, "\n";
        try { (function () { return new stdClass; })()[0] = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
        try { (function () { return 5; })()[0] = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
        "#,
    );
    assert_eq!(
        out,
        "bag|v(1)|bag|k|v(2)|arr|k|v(3)|bag|v(4)|4\nNULL=1;'x'=2;'y'=4;\nCannot use object of type stdClass as array\nCannot use a scalar value as an array\n"
    );
}

/// Verifies Laravel `Application::detectEnvironment`'s `return $this['env'] = v`: offsetSet runs
/// exactly once and the expression yields the assigned value, also from a closure.
#[test]
fn test_array_access_assignment_used_as_value() {
    let out = compile_and_run(
        r#"<?php
        class App implements ArrayAccess {
            public $log = "";
            function offsetExists($k): bool { return false; }
            function offsetGet($k): mixed { echo "get|"; return "G"; }
            function offsetSet($k, $v): void { $this->log .= "$k=$v;"; }
            function offsetUnset($k): void {}
            function detect() { return $this['env'] = $this->env(); }
            function env() { return "local"; }
        }
        function show($v) { echo "show(", $v, ")|"; return $v; }
        $o = new App;
        echo $o->detect(), "\n";
        $x = $o['a'] = "A"; echo $x, "\n";
        show($o['b'] = "B"); echo "\n";
        $y = $z = $o['c'] = 3; echo $y, $z, "\n";
        $r = (function () use ($o) { return $o['d'] = "D"; })(); echo $r, "\n";
        echo $o->log, "\n";
        "#,
    );
    assert_eq!(out, "local\nA\nshow(B)|\n33\nD\nenv=local;a=A;b=B;c=3;d=D;\n");
}

/// Verifies closures returning a property assignment hand back the assigned value instead of
/// the integer default the syntactic return inference used to answer.
#[test]
fn test_closure_returning_non_local_assignment() {
    let out = compile_and_run(
        r#"<?php
        class P { public $p; }
        $o = new P; $a = [];
        $r = (function () use ($o) { return $o->p = "D"; })(); echo $r, "\n";
        $f = function ($o) { return $o->p = "H"; }; echo $f($o), "\n";
        $g = fn($o) => $o->p = "I"; echo $g($o), "\n";
        "#,
    );
    assert_eq!(out, "D\nH\nI\n");
}

/// Verifies `$mixed[] = v` on an `ArrayAccess` object calls `offsetSet(null, v)` (ledger
/// miscompile: the write was dropped), in statement and expression position.
#[test]
fn test_mixed_append_calls_offset_set() {
    let out = compile_and_run(
        r#"<?php
        class Bag implements ArrayAccess {
            public $n = 0;
            function offsetExists($k): bool { return false; }
            function offsetGet($k): mixed { return null; }
            function offsetSet($k, $v): void { echo "set(", var_export($k, true), ",", $v, ")|"; $this->n++; }
            function offsetUnset($k): void {}
        }
        function make(bool $bag): mixed { return $bag ? new Bag : []; }
        $m = make(true);
        $m[] = 1;
        $r = true && $m[] = 2;
        echo var_export($r, true), $m->n, "\n";
        $a = make(false);
        $a[] = 3;
        true && $a[] = 4;
        echo implode(",", $a), "\n";
        try { $s = make(true) ? new stdClass : null; $mx = $s; $mx[] = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
        "#,
    );
    assert_eq!(out, "set(NULL,1)|set(NULL,2)|true2\n3,4\nCannot use object of type stdClass as array\n");
}

/// Verifies a local an operand retypes is joined across the lazy edges (`$b = null; $c || $b =
/// f()` printed a raw integer on the skipped edge), and that `$c = null; $c || f()` evaluates
/// `f()` (a null-typed condition was branched on as truthy).
#[test]
fn test_conditional_operand_retypes_local_on_one_edge() {
    let out = compile_and_run(
        r#"<?php
        function i(int $x): int { echo "i|"; return $x; }
        function run(bool $c) {
            $b = null; $c || $b = i(2); var_dump($b);
            $n = 0; $c && $n = 2.5; var_dump($n);
            $s = null; $r = $c ? 1 : $s = "x"; var_dump($s);
            $o = null; $c ?: $o = new stdClass; echo gettype($o), "\n";
        }
        run(true);
        run(false);
        $z = null; $z || print("rhs\n"); $z && print("never\n");
        "#,
    );
    assert_eq!(
        out,
        "NULL\nfloat(2.5)\nNULL\nNULL\ni|int(2)\nint(0)\nstring(1) \"x\"\nobject\nrhs\n"
    );
}

/// Verifies a lazy operand of a `while`/`for`/`do` condition that retypes a local keeps the value
/// across iterations (review P2c H1: the short edge re-boxed the entry `null` over the back edge's
/// value) and that a null-typed `!$x` in such a condition is not folded (H2).
#[test]
fn test_loop_condition_lazy_operand_retypes_local() {
    let out = compile_and_run(
        r#"<?php
        function a() { $x = null; $i = 0; while ($i < 3 && ($x = 10)) { $i++; } var_dump($x); echo $x + 1, "\n"; }
        function e() { $x = null; for ($i = 0; $i < 3 && ($x = 10); $i++) {} var_dump($x); }
        function g() { $x = null; $i = 0; do { $i++; } while ($i < 3 && ($x = 10)); var_dump($x); }
        function t() { $x = null; $i = 0; while ($i < 3 ? ($x = 10) : false) { $i++; } var_dump($x); }
        function n() { $x = null; $i = 0; while ($i < 3 && ($x ?? $x = 10)) { $i++; } var_dump($x); }
        function z() { $x = 5; $i = 0; while ($i < 3 && ($x = null) === null) { $i++; } var_dump($x); }
        function c() { $x = null; $i = 0; while ($i++ < 6 && !$x && ($x = $i) < 3) { } var_dump($i, $x); }
        a(); e(); g(); t(); n(); z(); c();
        "#,
    );
    assert_eq!(
        out,
        "int(10)\n11\nint(10)\nint(10)\nint(10)\nint(10)\nNULL\nint(2)\nint(1)\n"
    );
}

/// Verifies `$a = null; $c && $a = [1];` leaves a real PHP null on the skipped edge (review P2c
/// H3: the join adopted the array, so `=== null`, `is_array` and `$a[] = v` misread the null word)
/// and that a function returning such a local keeps the null.
#[test]
fn test_lazy_null_to_array_join_keeps_php_null() {
    let out = compile_and_run(
        r#"<?php
        function t1($c) { $a = null; $c && $a = [1]; var_dump($a === null, is_array($a), gettype($a)); $a[] = 2; var_dump(count($a)); }
        function t2($c) { $a = null; $c || $a = ['k' => 'v']; var_dump(isset($a), gettype($a)); $a['z'] = 1; var_dump(count($a)); }
        function t5($c) { $x = null; $c && $x = 5; return $x; }
        t1(false); t1(true); t2(true); t2(false);
        var_dump(t5(true), t5(false));
        "#,
    );
    assert_eq!(
        out,
        "bool(true)\nbool(false)\nstring(4) \"NULL\"\nint(1)\nbool(false)\nbool(true)\nstring(5) \"array\"\nint(2)\nbool(false)\nstring(4) \"NULL\"\nint(1)\nbool(true)\nstring(5) \"array\"\nint(2)\nint(5)\nNULL\n"
    );
}

/// Verifies a local assigned in a lazy operand of a condition is defined where that edge leads: the `if`/`while`/`for` body on the true edge of `&&`, the code after a diverging `if` on the false edge of `||` (P2c r2).
#[test]
fn test_condition_edge_definitions_reach_the_guarded_region() {
    let out = compile_and_run(
        r#"<?php
        // A local assigned in a lazy operand of a condition is defined on the edge that implies it.
        class O { public $name = "N"; function m() { return $this; } }
        function f($v) { return $v; }
        function a($c) { if ($c && ($x = f(new O))) { return $x->name ?? false; } return "no"; }
        var_dump(a(true), a(false));
        function b($c) { if (!$c || !($y = f(5))) { return "early"; } return $y + 1; }
        var_dump(b(true), b(false));
        function c() { $i = 0; while ($i < 3 && ($r = f($i * 2)) !== null) { echo $r, ","; $i++; } echo "\n"; }
        c();
        function d($c) { if ($c && ($p = f("p")) && ($q = f("q"))) { return $p . $q; } elseif (!$c || ($z = f("z"))) { return "elif"; } return "else"; }
        var_dump(d(true), d(false));
        function e($c) { return ($c && ($t = f(7))) ? $t * 2 : -1; }
        var_dump(e(true), e(false));
        function g($c) { for ($i = 0; $i < 2 && ($v = f($i + 10)); $i++) { echo $v, ","; } echo "\n"; }
        g(1);
        "#,
    );
    assert_eq!(out, "string(1) \"N\"\nstring(2) \"no\"\nint(6)\nstring(5) \"early\"\n0,2,4,\nstring(2) \"pq\"\nstring(4) \"elif\"\nint(14)\nint(-1)\n10,11,\n");
}

/// Verifies Container.php:819 (`is_callable($cb) && ! ($r = new R(...))->isAnonymous()`), nested `&&`/`||`, `!`, elseif false edges and ternary conditions define the assigned local on the right edge (P2c r2).
#[test]
fn test_condition_edge_definitions_nested_and_negated() {
    let out = compile_and_run(
        r#"<?php
        // Container.php:819 shape, nested &&/||, `!`, elseif false edges and diverging `if` (P2c r2).
        class S { public $name = "Scope"; }
        class R { function __construct(public $anon) {} function isAnonymous() { return $this->anon; } function getClosureScopeClass() { return new S; } }
        class C {
            function get($cb) {
                if (is_callable($cb) && ! ($reflector = new R($cb === 'x'))->isAnonymous()) {
                    return $reflector->getClosureScopeClass()->name ?? false;
                }
                return "other";
            }
        }
        $c = new C; var_dump($c->get('strlen'), $c->get('x'), $c->get(5));
        function f($v) { return $v; }
        function n1($a, $b) { if ($a && ($b || ($z = f(3)))) { return isset($z) ? "z" : "noz"; } return "f"; }
        var_dump(n1(true, false), n1(true, true), n1(false, false));
        function n2($a) { if (!(!$a || !($w = f("w")))) { return $w; } return "-"; }
        var_dump(n2(1), n2(0));
        function n3($a) { if (!$a || ($y = f(9)) === null) { return "early"; } return $y * 2; }
        var_dump(n3(1), n3(0));
        function n4($a) { if ($a) { return "a"; } elseif (($q = f("q")) && ($r = f("r"))) { return $q . $r; } else { return "e"; } }
        var_dump(n4(0), n4(1));
        $top = true;
        if (!$top || !($g = f("g"))) { echo "no\n"; } else { echo $g, "\n"; }
        echo ($top && ($h = f(4))) ? $h + 1 : 0, "\n";
        "#,
    );
    assert_eq!(out, "string(5) \"Scope\"\nstring(5) \"other\"\nstring(5) \"other\"\nstring(1) \"z\"\nstring(3) \"noz\"\nstring(1) \"f\"\nstring(1) \"w\"\nstring(1) \"-\"\nint(18)\nstring(5) \"early\"\nstring(2) \"qr\"\nstring(1) \"a\"\ng\n5\n");
}

/// Verifies both arms of `? :` retyping one local are checked from the entry environment and joined (review P2c r1 N1: a spurious "cannot reassign").
#[test]
fn test_ternary_arms_retyping_the_same_local_are_joined() {
    let out = compile_and_run(
        r#"<?php
        // Both arms of `? :` retype the same local: checked in parallel, then joined (P2c r1 N1).
        function g(bool $c) { $y = null; $r = $c ? ($y = "s") : ($y = 1); var_dump($r, $y); }
        g(true); g(false);
        function h(bool $c) { $n = 0; $c ? ($n = 1.5) : ($n = 2); var_dump($n); }
        h(true); h(false);
        "#,
    );
    assert_eq!(out, "string(1) \"s\"\nstring(1) \"s\"\nint(1)\nint(1)\nfloat(1.5)\nint(2)\n");
}
