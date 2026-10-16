//! Purpose:
//! End-to-end coverage for static-member syntax and semantics found on the Laravel frontier
//! (front P42a): increments on static elements, runtime-named static properties
//! (`self::${$p}`), `??=` with `require` on static places, magic constants in class and trait
//! constants, and three static-property miscompiles (owner of a returned static property,
//! null-defaulted array slots compared with null, string keys on untyped static arrays).
//!
//! Called from:
//! - The native codegen suite (`tests/codegen/mod.rs`).
//!
//! Key details:
//! - Each source mirrors a probe in `probes/static-member/` of the elephc-laravel workspace;
//!   expected output was produced by PHP 8.5 from the same source.

use crate::support::*;

/// Verifies `++self::$rand[$i];` (symfony Ulid), `--`, postfix forms and nested `.=`/`??=` on static array elements.
#[test]
fn test_prefix_postfix_incdec_on_static_property_dimension() {
    let out = compile_and_run(r#"<?php
class U {
    public static array $rand = [5, 9, 0];
    public static int $n = 0;
    public static array $m = ['a' => ['b' => 'x']];
    public static function bump(): void {
        for ($i = 2; $i >= 0; --$i) {
            ++self::$rand[$i];
        }
        --self::$rand[0];
        self::$rand[1]++;
        static::$rand[2]--;
        echo ++self::$rand[0], " ", self::$rand[1]--, " ", --static::$rand[2], "\n";
        if (1 !== ++self::$n) { echo "no\n"; }
        echo self::$n++, " ", self::$n, "\n";
        U::$m['a']['b'] .= 'y';
        self::$m['a']['c'] ??= 'z';
        self::$m['a']['b'] .= self::$m['a']['c'];
        echo json_encode(self::$rand), json_encode(self::$m), "\n";
    }
}
U::bump();
"#);
    assert_eq!(out, r#"6 11 -1
1 2
[6,10,-1]{"a":{"b":"xyz","c":"z"}}
"#);
}

/// Verifies `static::$x->p++` and `++self::$x->a[0]` on an object held by a static property.
#[test]
fn test_incdec_on_static_property_member() {
    let out = compile_and_run(r#"<?php
class Box { public int $p = 1; public array $a = [1]; }
class S {
    public static Box $x;
    public static function run(): void {
        static::$x = new Box();
        static::$x->p++;
        ++self::$x->p;
        echo static::$x->p--, " ", --self::$x->p, "\n";
        self::$x->a[0]++;
        echo self::$x->a[0], "\n";
    }
}
S::run();
"#);
    assert_eq!(out, r#"3 1
2
"#);
}

/// Verifies PHP evaluation order of `self::$a[f()] = g()`, compound writes and increments on static elements.
#[test]
fn test_static_dimension_write_evaluation_order() {
    let out = compile_and_run(r#"<?php
function f($v) { echo "f($v) "; return $v; }
class O {
    public static array $a = [0, 0];
    public static int $i = 0;
    public static function run(): void {
        self::$a[f(0)] = f(7);
        echo "\n";
        self::$a[f(1)] += f(5);
        echo "\n";
        echo ++self::$a[f(0)], "\n";
        echo self::$a[f(1)]++, "\n";
        self::$a[self::$i++] = self::$i;
        echo json_encode(self::$a), " ", self::$i, "\n";
    }
}
O::run();
"#);
    assert_eq!(out, r#"f(0) f(7) 
f(1) f(5) 
f(0) 8
f(1) 5
[1,6] 1
"#);
}

/// Verifies `__CLASS__` / `self::class` / `__NAMESPACE__` inside class-constant arrays (symfony ReflectionCaster) used to panic.
#[test]
fn test_magic_constants_in_class_constant_values() {
    let out = compile_and_run(r#"<?php
namespace N;
class R {
    public const UNSET = ['Closure' => __CLASS__ . '::unset'];
    public const NAMES = [__CLASS__, self::class, __NAMESPACE__];
    public const NESTED = ['a' => ['b' => __CLASS__ . '::x']];
    public function m(): array { return [self::UNSET, self::NAMES, static::NESTED]; }
}
interface I { const K = [__CLASS__ => 1]; }
class C implements I {}
echo json_encode((new R())->m()), "\n";
echo json_encode(R::UNSET), json_encode(C::K), "\n";
"#);
    assert_eq!(out, r#"[{"Closure":"N\\R::unset"},["N\\R","N\\R","N"],{"a":{"b":"N\\R::x"}}]
{"Closure":"N\\R::unset"}{"N\\I":1}
"#);
}

/// Verifies `__CLASS__` in a trait constant names the using class; `__FUNCTION__`/`__METHOD__` are empty in a constant.
#[test]
fn test_magic_constants_in_trait_constant_values_rebind_to_using_class() {
    let out = compile_and_run(r#"<?php
namespace N;
trait T {
    const X = [__CLASS__, __TRAIT__, __NAMESPACE__];
    public function m() { return self::X; }
}
class C { use T; }
class D { use T; }
echo json_encode(C::X), json_encode((new D())->m()), "\n";
class F { const DIR_OK = __DIR__ === '' ? 'empty' : 'dir'; const FN = [__FUNCTION__, __METHOD__]; }
echo F::DIR_OK, json_encode(F::FN), "\n";
"#);
    assert_eq!(out, r#"["N\\C","N\\T","N"]["N\\D","N\\T","N"]
dir["",""]
"#);
}

/// Verifies `self::${$p}` / `static::${$p . "Methods"}` (symfony DebugClassLoader): writes, isset, `??=`, `++`, `=&`, `??`, `empty`.
#[test]
fn test_dynamic_static_property_name_reads_writes_isset_coalesce_ref() {
    let out = compile_and_run(r#"<?php
// self::${$p} / static::${...} (symfony DebugClassLoader): lectura, escritura, isset, ??=, ++ y referencia.
class D {
    private static array $final = [];
    private static array $finalMethods = [];
    private static int $caseCheck = 0;
    public static function run(string $p): void {
        self::${$p}['A'] = 1;
        static::${$p . 'Methods'}['A']['m'] = 2;
        echo isset(self::${$p}['A']) ? "set" : "unset", " ", isset(self::${$p}['B']) ? "set" : "unset", "\n";
        self::${$p}['B'] ??= 3;
        self::${$p}['B'] ??= 4;
        self::${$p}['A'] = self::${$p}['A'] ? self::${$p}['B'] + self::${$p}['A'] : 0;
        echo json_encode(self::$final), json_encode(self::$finalMethods), "\n";
        $r = &self::${$p};
        $r['C'] = 5;
        ++self::${$p}['C'];
        self::${'case' . 'Check'} = 7;
        echo count(self::$final), " ", self::$final['C'], " ", self::${'caseCheck'}, " ", self::$$p['A'], "\n";
        echo self::${$p}['Z'] ?? 'dflt', " ", empty(self::${$p}['Z']) ? 'empty' : 'full', "\n";
    }
}
D::run('final');
"#);
    assert_eq!(out, r#"set unset
{"A":4,"B":3}{"A":{"m":2}}
3 6 7 4
dflt empty
"#);
}

/// Verifies An unknown name throws PHP's `Access to undeclared static property`; `isset` answers false.
#[test]
fn test_dynamic_static_property_name_undeclared_throws_error() {
    let out = compile_and_run(r#"<?php
class D {
    public static $a = 1;
    public static function run(string $p): void {
        echo self::${'a'}, "\n";
        try {
            echo self::${$p};
        } catch (\Error $e) {
            echo get_class($e), ": ", $e->getMessage(), "\n";
        }
        try {
            self::${$p} = 2;
        } catch (\Error $e) {
            echo get_class($e), ": ", $e->getMessage(), "\n";
        }
        echo isset(self::${$p}) ? "yes" : "no", "\n";
    }
}
D::run('nope');
"#);
    assert_eq!(out, r#"1
Error: Access to undeclared static property D::$nope
Error: Access to undeclared static property D::$nope
no
"#);
}

/// Verifies Inherited, trait-imported and ancestor-private static properties through `static::${$p}`.
#[test]
fn test_dynamic_static_property_name_inherited_trait_and_private() {
    let out = compile_and_run(r#"<?php
trait Holds { protected static array $fromTrait = ['t' => 1]; }
class Base { protected static array $shared = []; private static array $hidden = []; public static $pub = 'b'; }
class Child extends Base {
    use Holds;
    private static array $own = [];
    public static function put(string $p, string $k, $v): void { static::${$p}[$k] = $v; }
    public static function get(string $p) { return self::${$p}; }
    public static function probe(string $p): string {
        try { return json_encode(self::${$p}); } catch (\Error $e) { return $e->getMessage(); }
    }
}
Child::put('own', 'a', 1);
Child::put('shared', 'b', 2);
Child::put('fromTrait', 'c', 3);
echo json_encode(Child::get('own')), json_encode(Child::get('shared')), json_encode(Child::get('fromTrait')), Child::get('pub'), "\n";
echo Child::probe('hidden'), "\n";
echo Child::probe('nope'), "\n";
"#);
    assert_eq!(out, r#"{"a":1}{"b":2}{"t":1,"c":3}b
Cannot access private property Child::$hidden
Access to undeclared static property Child::$nope
"#);
}

/// Verifies `return self::$p;` with a discarded result used to free the property payload.
#[test]
fn test_return_of_static_property_keeps_its_owner() {
    let out = compile_and_run(r#"<?php
// `return self::$p;` con el resultado descartado no debe liberar el valor de la propiedad.
class L {
    public static ?array $n = ['a'];
    public static array $t = ['b'];
    public static $u = ['c'];
    public static ?string $s = 'str';
    public static function n() { return self::$n; }
    public static function t(): array { return self::$t; }
    public static function u() { return static::$u; }
    public static function s() { return self::$s; }
}
for ($i = 0; $i < 3; $i++) { L::n(); L::t(); L::u(); L::s(); $x = L::n(); unset($x); }
echo json_encode(L::$n), json_encode(L::$t), json_encode(L::$u), L::$s, "\n";
"#);
    assert_eq!(out, r#"["a"]["b"]["c"]str
"#);
}

/// Verifies An untyped null-defaulted property later assigned an array answers `=== null`, `isset` and `??` like PHP.
#[test]
fn test_null_defaulted_property_holding_array_compares_with_null() {
    let out = compile_and_run(r#"<?php
// Prop sin tipo con default null que luego guarda un array: === null, isset, is_null y ??.
class L {
    public static $n = null;
    public $p = null;
    public static function names() {
        if (self::$n === null) { self::$n = ['a', 'b']; }
        return self::$n;
    }
    public function inst() {
        $before = [$this->p === null, isset($this->p), $this->p !== null];
        $this->p = [1];
        return [$before, $this->p === null, isset($this->p)];
    }
}
var_dump(L::$n === null, isset(L::$n), is_null(L::$n), L::$n ?? 'dflt');
echo json_encode(L::names()), json_encode(L::names()), "\n";
var_dump(L::$n === null, isset(L::$n), L::$n !== null);
echo json_encode((new L())->inst()), "\n";
"#);
    assert_eq!(out, r#"bool(true)
bool(false)
bool(true)
string(4) "dflt"
["a","b"]["a","b"]
bool(false)
bool(true)
bool(true)
[[true,false,false],false,true]
"#);
}

/// Verifies `self::$cache[$k] = v` with a string key on an untyped `[]` static stored `[1]`.
#[test]
fn test_untyped_static_array_with_string_keys() {
    let out = compile_and_run(r#"<?php
// Array estático sin tipo escrito con clave string (Router/PhpFileLoader self::$cache[$path]).
class L {
    public static $c = [];
    public static $d = [];
    public static $e = [];
    public static $u = ['x' => 0];
    public static function run(string $k) {
        self::$c[$k] = 1;
        self::$d[$k] ??= strlen($k);
        self::$e['lit'] ??= 3;
        self::$u[$k] = 1;
        return self::$d[$k];
    }
}
echo L::run('a'), L::run('bb'), L::run('a'), "\n";
echo json_encode(L::$c), json_encode(L::$d), json_encode(L::$e), json_encode(L::$u), "\n";
"#);
    assert_eq!(out, r#"121
{"a":1,"bb":1}{"a":1,"bb":2}{"lit":3}{"x":0,"a":1,"bb":1}
"#);
}

/// Verifies `static::$names ??= require F;` and `return self::$cache[$k] ??= require F;`
/// (Carbon Language, symfony Router / PhpFileLoader) include the file only while the place is unset.
#[test]
fn test_static_property_null_coalesce_assign_with_require() {
    let out = compile_and_run_files(
        &[
            ("main.php", r#"<?php
// ??= con require sobre propiedades estáticas sin tipo (Carbon Language, symfony Router/PhpFileLoader).
class L {
    protected static $names;
    private static $cache = [];
    public static function names() {
        static::$names ??= require __DIR__ . '/list.php';
        return static::$names;
    }
    public static function keys() { return array_keys(self::$cache); }
    public static function cached(string $k) {
        return self::$cache[$k] ??= require __DIR__ . '/list.php';
    }
}
echo json_encode(L::names()), " ", json_encode(L::names()), "\n";
echo L::cached('k')['es'], " ", L::cached('k')['en'], " ", L::cached('j')['en'], "\n";
echo json_encode(L::keys()), "\n";
"#),
            ("list.php", r#"<?php
echo "loaded ";
return ['en' => 'English', 'es' => 'Spanish'];
"#),
        ],
        "main.php",
    );
    assert_eq!(out, r#"loaded {"en":"English","es":"Spanish"} {"en":"English","es":"Spanish"}
loaded Spanish English loaded English
["k","j"]
"#);
}
