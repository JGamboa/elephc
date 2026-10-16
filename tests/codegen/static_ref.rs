//! Purpose:
//! End-to-end coverage for front P52: references between places of static properties
//! (symfony DebugClassLoader `self::$darwinCache[$dir] = &self::$darwinCache[$k]`), `unset` of
//! static-property elements, and untyped `static` locals that hold arrays, objects or mixed
//! values (symfony LocaleFallbackProvider `static $p; $p ??= require F;`).
//!
//! Called from:
//! - The native codegen suite (`tests/codegen/mod.rs`).
//!
//! Key details:
//! - Each source mirrors a probe in `probes/static-ref/` of the elephc-laravel workspace;
//!   expected output was produced by PHP 8.5 from the same source.
//! - The initializer of a `static` local runs once: several tests count its side effects.

use crate::support::*;

/// Verifies the DebugClassLoader shape: an element of a static property bound by reference to
/// another element of the same property, inside `if`/`while`, then written on one side, read on
/// the other, probed with `isset` and removed with `unset`.
#[test]
fn test_static_property_element_reference_darwin_cache() {
    let out = compile_and_run(r#"<?php
class D {
    private static array $c = [];
    public static function f(string $dir, string $k): void {
        if (!isset(self::$c[$k])) {
            while (!isset(self::$c[$k])) {
                self::$c[$k] = [$dir, []];
                self::$c[$dir] = &self::$c[$k];
            }
        }
    }
    public static function dump(): void {
        self::$c['/A'][1][] = 'x';
        echo json_encode(self::$c['/a']), "\n";
        var_dump(isset(self::$c['/A']));
        unset(self::$c['/A']);
        var_dump(isset(self::$c['/A']), self::$c['/a'][1]);
    }
}
D::f('/A', '/a'); D::dump();
"#);
    assert_eq!(out, r#"["\/A",["x"]]
bool(true)
bool(false)
array(1) {
  [0]=>
  string(1) "x"
}
"#);
}

/// Verifies `self::$a[$x] = &$local`, `$local = &self::$a[$x]` and `static::$a[..][..] = &...`
/// on an untyped static property, with writes on either side of each alias.
#[test]
fn test_static_property_element_reference_place_kinds() {
    let out = compile_and_run(r#"<?php
class P {
    public static $a = [];
    public static function run(): void {
        $v = 1;
        self::$a['x'] = &$v;
        $v = 2;
        echo self::$a['x'], "\n";
        self::$a['x'] = 3;
        echo $v, "\n";
        $r = &self::$a['y'];
        $r = 'ry';
        echo self::$a['y'], "\n";
        static::$a['m']['n'] = &self::$a['y'];
        static::$a['m']['n'] = 'mn';
        echo $r, " ", self::$a['y'], "\n";
        unset($r);
        echo json_encode(self::$a), "\n";
    }
}
P::run();
"#);
    assert_eq!(out, "2\n3\nry\nmn mn\n{\"x\":3,\"y\":\"mn\",\"m\":{\"n\":\"mn\"}}\n");
}

/// Verifies element references into an untyped static property whose default is a non-empty
/// array: the slot is widened to the boxed PHP array representation (it crashed before).
#[test]
fn test_reference_into_untyped_static_property_with_array_default() {
    let out = compile_and_run(r#"<?php
class D {
    public static $u = ["a" => 1, "b" => ["x" => 1]];
    static function f() {
        $r = &self::$u["b"]["x"]; $r = 5;
        $q = &self::$u["a"]; $q = 6;
        self::$u["c"] = &self::$u["a"]; self::$u["c"] = 7;
        echo json_encode(self::$u), "\n";
    }
}
D::f();
"#);
    assert_eq!(out, "{\"a\":7,\"b\":{\"x\":5},\"c\":7}\n");
}

/// Verifies a reference to a static-property element bound inside a branch stays visible after it.
#[test]
fn test_conditional_reference_to_static_property_element() {
    let out = compile_and_run(r#"<?php
class D {
    public static $a = ["k" => 1];
    static function f($c) {
        $r = 0;
        if ($c) { $r = &self::$a["k"]; }
        $r = 5;
        echo json_encode(self::$a), "\n";
        if ($c) { self::$a["x"] = &$r; }
        $r = 6;
        echo json_encode(self::$a), "\n";
    }
}
D::f(true); D::f(false);
"#);
    assert_eq!(out, "{\"k\":5}\n{\"k\":6,\"x\":6}\n{\"k\":6,\"x\":6}\n{\"k\":6,\"x\":6}\n");
}

/// Verifies `unset(C::$p[$k])` and `unset(C::$p[$a][$b])` on typed, untyped and packed static
/// arrays, leaving an earlier by-value copy untouched.
#[test]
fn test_unset_static_property_elements() {
    let out = compile_and_run(r#"<?php
class D {
    public static array $c = ['a' => 1, 'b' => 2];
    public static $u = ['a' => 1, 'b' => ['x' => 1, 'y' => 2]];
    public static array $l = [1, 2, 3];
    static function f() {
        $copy = self::$c;
        unset(self::$c['a']);
        unset(static::$u['b']['x']);
        unset(self::$l[1]);
        echo json_encode(self::$c), json_encode(self::$u), json_encode(self::$l), json_encode($copy), "\n";
    }
}
D::f(); D::f();
var_dump(isset(D::$u['b']['y']), isset(D::$u['b']['x']));
"#);
    assert_eq!(out, r#"{"b":2}{"a":1,"b":{"y":2}}{"0":1,"2":3}{"a":1,"b":2}
{"b":2}{"a":1,"b":{"y":2}}{"0":1,"2":3}{"b":2}
bool(true)
bool(false)
"#);
}

/// Verifies an untyped `static` local with no initializer persists an associative array and
/// runs its `??=` value once (P42a gap D), plus a string-keyed cache over `static $c = []`.
#[test]
fn test_untyped_static_local_assoc_array_initializer_runs_once() {
    let out = compile_and_run(r#"<?php
function make() { echo "make "; return ['a' => 1]; }
function f() { static $p; $p ??= make(); $p['n'] = ($p['n'] ?? 0) + 1; return $p; }
echo json_encode(f()), json_encode(f()), json_encode(f()), "\n";
function c($k) { static $c = []; $c[$k] = ($c[$k] ?? 0) + 1; return json_encode($c); }
echo c("a"), c("b"), c("a"), "\n";
function q($k) { static $c = []; return $c[$k] ??= strtoupper($k); }
echo q("a"), q("b"), q("a"), "\n";
"#);
    assert_eq!(out, "make {\"a\":1,\"n\":1}{\"a\":1,\"n\":2}{\"a\":1,\"n\":3}\n{\"a\":1}{\"a\":1,\"b\":1}{\"a\":2,\"b\":1}\nABA\n");
}

/// Verifies `static $x = null; if ($x === null) { ... }` initializes once, with arrays, objects,
/// floats and strings, and that a mixed static keeps whatever the previous call stored.
#[test]
fn test_null_defaulted_static_local_initializes_once() {
    let out = compile_and_run(r#"<?php
function h() {
    static $x = null;
    if ($x === null) { echo "init "; $x = ['es_CL' => 'es_419', 'es_419' => 'es']; }
    return $x;
}
echo json_encode(h()), count(h()), "\n";
function o() { static $o; $o ??= new ArrayObject([]); $o['c'] = ($o['c'] ?? 0) + 1; return $o['c']; }
echo o(), o(), o(), "\n";
function m($v) { static $m = null; $prev = $m; $m = $v; return json_encode($prev); }
echo m(1), m('s'), m([1 => 'x']), m(null), "\n";
function s() { static $s = null; $s = ($s ?? "") . "x"; return $s; }
echo s(), s(), s(), "\n";
function d() { static $p = null; if (is_null($p)) { $p = 1.5; } else { $p *= 2; } return $p; }
echo d(), " ", d(), " ", d(), "\n";
"#);
    assert_eq!(out, "init {\"es_CL\":\"es_419\",\"es_419\":\"es\"}2\n123\nnull1\"s\"{\"1\":\"x\"}\nxxxxxx\n1.5 3 6\n");
}

/// Verifies the LocaleFallbackProvider shape: `static $p; $p ??= require F;` includes the file
/// once and keeps the associative array for later calls.
#[test]
fn test_static_local_coalesce_require_includes_once() {
    let out = compile_and_run_files(
        &[
            (
                "main.php",
                r#"<?php
function parents() {
    static $p;
    $p ??= require __DIR__ . '/parents.php';
    return $p;
}
echo json_encode(parents()), "\n";
echo parents()['es_CL'], count(parents()), "\n";
"#,
            ),
            (
                "parents.php",
                r#"<?php
echo "loaded\n";
return ['es_CL' => 'es_419', 'es_419' => 'es'];
"#,
            ),
        ],
        "main.php",
    );
    assert_eq!(out, "loaded\n{\"es_CL\":\"es_419\",\"es_419\":\"es\"}\nes_4192\n");
}

/// Verifies a method's static locals are shared by the class and a subclass that inherits the
/// method, including `parent::` from an override (PHP 8.1+ semantics).
#[test]
fn test_method_static_local_shared_with_inheriting_subclass() {
    let out = compile_and_run(r#"<?php
class A {
    public function c() { static $n = []; $n[static::class] = ($n[static::class] ?? 0) + 1; return json_encode($n); }
    public static function s() { static $x; $x ??= ['k' => static::class]; return $x['k']; }
}
class B extends A {}
class C extends A { public function c() { return 'C' . parent::c(); } }
echo (new A)->c(), (new B)->c(), (new A)->c(), "\n";
echo (new C)->c(), "\n";
echo A::s(), B::s(), "\n";
"#);
    assert_eq!(out, "{\"A\":1}{\"A\":1,\"B\":1}{\"A\":2,\"B\":1}\nC{\"A\":2,\"B\":1,\"C\":1}\nAA\n");
}

/// Verifies element references into a null-defaulted static local (nette ObjectHelpers
/// `$props = &$cache[$class]`) and array builtins over a boxed static array.
#[test]
fn test_element_reference_into_static_local_cache() {
    let out = compile_and_run(r#"<?php
function f($c) { static $cache; $props = &$cache[$c]; if ($props === null) { echo "calc "; $props = strlen($c); } return $props; }
echo f("ab"), f("ab"), f("abc"), "\n";
function g($x) { static $c = []; $c = array_merge($c, [$x]); return implode(",", $c) . count($c) . (in_array(1, $c) ? "y" : "n"); }
echo g(1), g(2), "\n";
"#);
    assert_eq!(out, "calc 22calc 3\n11y1,22y\n");
}

/// Verifies boxed static locals and static-property element references release what they own.
#[test]
fn test_static_ref_and_static_local_heap_debug_clean() {
    let out = compile_and_run_with_heap_debug(r#"<?php
class D {
    private static array $c = [];
    public static $u = ['a' => ['x' => 'y'], 'b' => 'w'];
    public static function f(string $dir, string $k): void {
        self::$c[$k] = [$dir, []];
        self::$c[$dir] = &self::$c[$k];
        self::$c[$dir][1][] = str_repeat('z', 3);
        $r = &self::$u['a']['x'];
        $r = str_repeat('q', 2);
        unset($r);
        unset(self::$u['b']);
    }
}
function make() { return ['a' => str_repeat('m', 4)]; }
function f($k) { static $p; $p ??= make(); static $c = []; $c[$k] = ($c[$k] ?? '') . $k; static $s = null; $s = ($s ?? '') . $k; return count($p) + count($c) + strlen($s); }
D::f('/A', '/a'); D::f('/B', '/b');
echo f('a'), f('b'), f('a'), "\n";
"#);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(out.stdout, "356\n");
    assert!(
        out.stderr.contains("HEAP DEBUG: leak summary: clean"),
        "static-ref leaked: {}",
        out.stderr
    );
}

/// Verifies the review P42a static-local array probes `s1`/`s3`: a keyed write accumulates across
/// calls for string and int keys, and `count` sees every key stored by earlier calls.
#[test]
fn test_static_local_keyed_array_accumulates_review_s1_s3() {
    let out = compile_and_run(r#"<?php
function loc($k) { static $c = []; $c[$k] = ($c[$k] ?? 0) + 1; return $c; }
loc('a'); loc(1); echo json_encode(loc('a')), "\n";
function cnt($k) { static $c = []; $c[$k] = 1; return count($c); }
echo cnt('a'), cnt('b'), cnt('a'), "\n";
function loci($k) { static $c = []; $c[$k] = ($c[$k] ?? 0) + 1; return $c[$k]; }
echo loci(1), loci(1), loci(2), "\n";
function locs($k) { static $c = []; $c[$k] = ($c[$k] ?? 0) + 1; return $c[$k]; }
echo locs('a'), locs('a'), locs('b'), "\n";
"#);
    assert_eq!(out, "{\"a\":2,\"1\":1}\n122\n121\n121\n");
}

/// Verifies the review P42a probe `s2`: an int key written into a static local initialized with a
/// string-keyed array keeps both keys (it corrupted the hash to `{"\u0000":0}`).
#[test]
fn test_static_local_string_keyed_initializer_takes_int_key_review_s2() {
    let out = compile_and_run(r#"<?php
function loc2($k) { static $u = ['x' => 0]; $u[$k] = 1; return $u; }
echo json_encode(loc2('y')), json_encode(loc2(3)), "\n";
echo "after\n";
"#);
    assert_eq!(out, "{\"x\":0,\"y\":1}{\"x\":0,\"y\":1,\"3\":1}\nafter\n");
}
