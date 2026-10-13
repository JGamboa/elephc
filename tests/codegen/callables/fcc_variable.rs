//! Purpose:
//! End-to-end coverage for first-class callable syntax on a variable (`$callback(...)`),
//! `Closure::fromCallable()` over runtime values, and `ReflectionFunction` over the resulting
//! Closures (`isAnonymous()`, `isClosure()`, `getClosureScopeClass()`, `->name`).
//!
//! Called from:
//! - `cargo test --test codegen_tests fcc_variable`.
//!
//! Key details:
//! - The first tests mirror the external `probes/fcc/` family and assert PHP 8.5's output.
//! - Reflection queries that have no runtime answer for a Closure reflected at run time must
//!   throw `Error`, never return the empty compile-time defaults.

use crate::support::*;

/// Mirrors `probes/fcc/var_function_names.php`.
#[test]
fn test_fcc_variable_probe_var_function_names() {
    let out = compile_and_run(
        r#"<?php
// $var(...) over callable strings: user function, builtin, namespaced, 'Class::method'.
namespace App;

function twice(int $x): int { return $x * 2; }
class Math { public static function inc(int $x): int { return $x + 1; } }

function convert(mixed $name): \Closure { return $name(...); }

$cases = ['App\twice', 'strrev', 'App\Math::inc', 'APP\TWICE'];
foreach ($cases as $name) {
    $fn = convert($name);
    var_dump($fn($name === 'strrev' ? 'abc' : 20));
}
$local = 'App\twice';
$g = $local(...);
echo $g(7), "\n";
"#,
    );
    assert_eq!(out, "int(40)\nstring(3) \"cba\"\nint(21)\nint(40)\n14\n");
}

/// Mirrors `probes/fcc/var_callable_arrays.php`.
#[test]
fn test_fcc_variable_probe_var_callable_arrays() {
    let out = compile_and_run(
        r#"<?php
// $var(...) over callable arrays: bound instance method and static method.
class Greeter {
    public function __construct(private string $who) {}
    public function hello(string $p): string { return "$p {$this->who}"; }
    public static function shout(string $s): string { return strtoupper($s); }
}

function convert(mixed $cb) { return $cb(...); }

$bound = convert([new Greeter('Ada'), 'hello']);
echo $bound('hi'), "\n";
$static = convert(['Greeter', 'shout']);
echo $static('quiet'), "\n";
$cb = [new Greeter('Bob'), 'hello'];
$again = $cb(...);
echo $again('hey'), "\n";
"#,
    );
    assert_eq!(out, "hi Ada\nQUIET\nhey Bob\n");
}

/// Mirrors `probes/fcc/var_invokable_and_closure.php`.
#[test]
fn test_fcc_variable_probe_var_invokable_and_closure() {
    let out = compile_and_run(
        r#"<?php
// $var(...) over an invokable object and over an existing Closure.
class Doubler { public function __invoke(int $x): int { return $x * 2; } }

function convert(mixed $cb) { return $cb(...); }

$fromObject = convert(new Doubler());
echo $fromObject(21), "\n";
$closure = fn (int $x): int => $x + 100;
$same = convert($closure);
echo $same(1), "\n";
$direct = $closure(...);
echo $direct(2), "\n";
var_dump($same instanceof Closure);
"#,
    );
    assert_eq!(out, "42\n101\n102\nbool(true)\n");
}

/// Mirrors `probes/fcc/var_not_callable.php`.
#[test]
fn test_fcc_variable_probe_var_not_callable() {
    let out = compile_and_run(
        r#"<?php
// $var(...) over values that name nothing callable throws Error (not TypeError).
class Plain { public function m() {} }

function convert(mixed $cb) { return $cb(...); }

$values = [42, 'no_such_function', [new Plain(), 'missing'], 'Plain::missing', new Plain(), null];
foreach ($values as $value) {
    try {
        convert($value);
        echo "converted\n";
    } catch (TypeError $e) {
        echo "TypeError\n";
    } catch (Error $e) {
        echo get_class($e), "\n";
    }
}
echo "done\n";
"#,
    );
    assert_eq!(out, "Error\nError\nError\nError\nError\nError\ndone\n");
}

/// Mirrors `probes/fcc/from_callable_runtime.php`.
#[test]
fn test_fcc_variable_probe_from_callable_runtime() {
    let out = compile_and_run(
        r#"<?php
// Closure::fromCallable() over a runtime value; non-callables throw TypeError.
class Box { public function get(int $x): int { return $x * 3; } }
function plus(int $x): int { return $x + 5; }

function make(mixed $cb) { return Closure::fromCallable($cb); }

echo make('plus')(1), "\n";
echo make([new Box(), 'get'])(2), "\n";
echo make(fn ($x) => "c$x")(3), "\n";
foreach ([7, 'nope', [new Box(), 'zz']] as $bad) {
    try {
        make($bad);
    } catch (Throwable $e) {
        echo get_class($e), "\n";
    }
}
"#,
    );
    assert_eq!(out, "6\n6\nc3\nTypeError\nTypeError\nTypeError\n");
}

/// Mirrors `probes/fcc/reflection_is_anonymous.php`.
#[test]
fn test_fcc_variable_probe_reflection_is_anonymous() {
    let out = compile_and_run(
        r#"<?php
// ReflectionFunction::isAnonymous()/isClosure() over Closures built at run time.
function named() {}
class K { public function m() {} public static function s() {} public function __invoke() {} }

function anon(mixed $cb): bool { return (new ReflectionFunction($cb(...)))->isAnonymous(); }

var_dump(anon('named'), anon('strlen'), anon('K::s'), anon([new K(), 'm']), anon(new K()));
var_dump(anon(function () {}), anon(fn () => 1));
$byName = new ReflectionFunction('named');
var_dump($byName->isAnonymous(), $byName->isClosure(), $byName->getClosureScopeClass());
var_dump((new ReflectionFunction(named(...)))->isClosure());
echo $byName->name, "\n";
"#,
    );
    assert_eq!(out, "bool(false)\nbool(false)\nbool(false)\nbool(false)\nbool(false)\nbool(true)\nbool(true)\nbool(false)\nbool(false)\nNULL\nbool(true)\nnamed\n");
}

/// Mirrors `probes/fcc/reflection_scope_class.php`.
#[test]
fn test_fcc_variable_probe_reflection_scope_class() {
    let out = compile_and_run(
        r#"<?php
// ReflectionFunction::getClosureScopeClass() reports the declaring class (or null).
namespace App;
use ReflectionFunction;

function helper() {}
class Base { public function inherited() {} public static function make() {} }
final class Child extends Base {
    public function own() { return function () {}; }
    public function __invoke() {}
}

function scope(mixed $cb): ?string { return (new ReflectionFunction($cb(...)))->getClosureScopeClass()?->name; }

var_dump(scope('App\helper'), scope('strlen'), scope('App\Base::make'));
var_dump(scope([new Child(), 'inherited']), scope([new Child(), 'own']), scope(new Child()));
var_dump(scope((new Child())->own()), scope(function () {}));
$scope = (new ReflectionFunction(scope(...)))->getClosureScopeClass();
var_dump($scope);
$name = 'App\Child::make';
$cls = (new ReflectionFunction($name(...)))->getClosureScopeClass();
var_dump($cls->getName());
"#,
    );
    assert_eq!(out, "NULL\nNULL\nstring(8) \"App\\Base\"\nstring(8) \"App\\Base\"\nstring(9) \"App\\Child\"\nstring(9) \"App\\Child\"\nstring(9) \"App\\Child\"\nNULL\nNULL\nstring(8) \"App\\Base\"\n");
}

/// Mirrors `probes/fcc/container_class_for_callable.php`.
#[test]
fn test_fcc_variable_probe_container_class_for_callable() {
    let out = compile_and_run(
        r#"<?php
// Illuminate\Container\Container::getClassForCallable() pattern over every callable shape.
namespace App;
use ReflectionFunction;

class Service { public function handle() {} public static function build() {} }
function helper() {}

class Container {
    protected function getClassForCallable($callback)
    {
        if (is_callable($callback)) {
            $reflector = new ReflectionFunction($callback(...));
            if (! $reflector->isAnonymous()) {
                return $reflector->getClosureScopeClass()->name ?? false;
            }
        }
        return false;
    }
    public function probe($cb) { return $this->getClassForCallable($cb); }
}

$c = new Container();
foreach (['App\helper', 'App\Service::build', [new Service(), 'handle'], fn () => 1, 'missing', 5] as $cb) {
    var_dump($c->probe($cb));
}
"#,
    );
    assert_eq!(out, "bool(false)\nstring(11) \"App\\Service\"\nstring(11) \"App\\Service\"\nbool(false)\nbool(false)\nbool(false)\n");
}

/// Reflection queries with no runtime answer throw instead of reporting empty defaults.
#[test]
fn test_fcc_variable_reflection_unsupported_queries_throw() {
    let out = compile_and_run(
        r#"<?php
class P { public function pm() {} }
function f(int $a) {}
$name = 'f';
$r = new ReflectionFunction($name(...));
foreach (['getName', 'getParameters', 'getNumberOfParameters'] as $i => $label) {
    try {
        if ($i === 0) { $r->getName(); }
        if ($i === 1) { $r->getParameters(); }
        if ($i === 2) { $r->getNumberOfParameters(); }
        echo "$label answered\n";
    } catch (Error $e) {
        echo get_class($e), " ", $label, "\n";
    }
}
$pair = [new P(), 'pm'];
$scope = (new ReflectionFunction($pair(...)))->getClosureScopeClass();
echo $scope->name, " ", $scope->getName(), "\n";
try { $scope->getMethods(); echo "answered\n"; } catch (Error $e) { echo get_class($e), "\n"; }
"#,
    );
    assert_eq!(
        out,
        "Error getName\nError getParameters\nError getNumberOfParameters\nP P\nError\n"
    );
}

/// A late-bound `static::m(...)` Closure has no static scope, so reflection refuses to guess.
#[test]
fn test_fcc_variable_reflection_late_static_scope_throws() {
    let out = compile_and_run(
        r#"<?php
class A {
    public static function m() {}
    public static function grab() { return static::m(...); }
}
class B extends A { public static function m() {} }
$r = new ReflectionFunction(B::grab());
var_dump($r->isAnonymous());
try { $r->getClosureScopeClass(); echo "answered\n"; } catch (Error $e) { echo get_class($e), "\n"; }
"#,
    );
    assert_eq!(out, "bool(false)\nError\n");
}

/// A get-hooked property read on a null receiver yields null, so `?? $default` answers.
#[test]
fn test_hooked_property_on_null_receiver_coalesces_to_default() {
    let out = compile_and_run(
        r#"<?php
class H { private string $n = "x"; public string $name { get => $this->n; } }
function pick(bool $b): ?H { return $b ? new H() : null; }
var_dump(pick(true)->name ?? false);
var_dump(pick(false)->name ?? false);
$h = pick(false);
var_dump($h?->name);
"#,
    );
    assert_eq!(out, "string(1) \"x\"\nbool(false)\nNULL\n");
}

/// The public `name` property mirrors `getName()` on statically reflected owners.
#[test]
fn test_reflection_name_property_on_static_owners() {
    let out = compile_and_run(
        r#"<?php
class Shape { public int $side = 1; public function area() {} }
function draw() {}
echo (new ReflectionClass('Shape'))->name, "\n";
echo (new ReflectionObject(new Shape()))->name, "\n";
echo (new ReflectionFunction('draw'))->name, "\n";
echo (new ReflectionMethod('Shape', 'area'))->name, "\n";
echo (new ReflectionProperty('Shape', 'side'))->name, "\n";
"#,
    );
    assert_eq!(out, "Shape\nShape\ndraw\narea\nside\n");
}
