//! Purpose:
//! End-to-end coverage of PHP-compatible override and interface-implementation signatures
//! (LSP): variadic renames, union covariance,
//! `#[\ReturnTypeWillChange]` and the implicit `__toString(): string`.
//!
//! Called from:
//! - `cargo test` through Rust's test harness.
//!
//! Key details:
//! - Every fixture dispatches through the parent or interface type, so an ABI mismatch between
//!   the parent contract and the override shows up as wrong output instead of passing silently.
//! - Fixtures mirror `probes/lsp/` in the elephc-laravel workspace; expected output is PHP 8.5's.

use super::*;

/// An override may rename the variadic (`...$arrays` as `...$lists`); arguments still collect positionally through the parent.
#[test]
fn test_override_renames_variadic_parameter() {
    let out = compile_and_run(
        r#"<?php
class Base {
    public function merge(array ...$arrays): int { return count($arrays); }
}
class Child extends Base {
    public function merge(array ...$lists): int { return 100 + count($lists); }
}
function viaBase(Base $b): int { return $b->merge([1], [2], [3]); }
echo viaBase(new Base()), "\n";
echo viaBase(new Child()), "\n";
"#,
    );
    assert_eq!(out, "3\n103\n");
}

/// A union return narrowed member by member with the class under construction (`Node|bool` as `Leaf|false`) dispatches through the parent.
#[test]
fn test_override_union_return_with_subclass_member() {
    let out = compile_and_run(
        r#"<?php
class Node {
    public function __construct(public int $v) {}
    public function find(int $x): Node|bool { return $x > 0 ? new Node($x) : false; }
}
class Leaf extends Node {
    public function find(int $x): Leaf|false { return $x > 0 ? new Leaf($x * 10) : false; }
}
function viaBase(Node $n, int $x): string {
    $r = $n->find($x);
    return $r === false ? "false" : get_class($r) . ":" . $r->v;
}
echo viaBase(new Node(1), 2), "\n";
echo viaBase(new Leaf(1), 2), "\n";
echo viaBase(new Leaf(1), 0), "\n";
"#,
    );
    assert_eq!(out, "Node:2\nLeaf:20\nfalse\n");
}

/// A static implementation may narrow an interface `static|false` return to `static`.
#[test]
fn test_interface_static_union_return_narrowed_to_static() {
    let out = compile_and_run(
        r#"<?php
interface Maker { public static function make(string $s): static|false; }
class Thing implements Maker {
    public function __construct(public string $s) {}
    public static function make(string $s): static { return new static($s); }
}
class Other extends Thing {}
$t = Thing::make("hi");
echo get_class($t), ":", $t->s, "\n";
$o = Other::make("yo");
echo get_class($o), ":", $o->s, "\n";
"#,
    );
    assert_eq!(out, "Thing:hi\nOther:yo\n");
}

/// `#[\ReturnTypeWillChange]` lets an untyped method implement a builtin interface method with a tentative return type.
#[test]
fn test_return_type_will_change_untyped_builtin_interface_method() {
    let out = compile_and_run(
        r#"<?php
class Box implements IteratorAggregate, Countable {
    private array $items = [1, 2, 3];
    #[\ReturnTypeWillChange]
    public function count() { return count($this->items); }
    public function getIterator(): Iterator { return new ArrayIterator($this->items); }
}
$b = new Box();
echo count($b), "\n";
echo $b->count(), "\n";
foreach ($b as $i) { echo $i; }
echo "\n";
"#,
    );
    assert_eq!(out, "3\n3\n123\n");
}

/// An untyped `__toString()` gets PHP 8's implicit `: string` under `Stringable` and a builtin parent, including an inheriting subclass that does not override it.
#[test]
fn test_untyped_tostring_gets_implicit_string_return() {
    let out = compile_and_run(
        r#"<?php
class Name implements Stringable {
    public function __construct(private string $n) {}
    public function __toString() { return "name:" . $this->n; }
}
class Nick extends Name {}
class Oops extends Exception {
    public function __toString() { return "oops"; }
}
function show(Stringable $s): string { return $s->__toString(); }
echo new Name("x"), " ", new Nick("y"), " ", new Oops(), "\n";
echo show(new Name("z")), " ", show(new Nick("w")), "\n";
var_dump(new Nick("q") instanceof Stringable);
"#,
    );
    assert_eq!(out, "name:x name:y oops\nname:z name:w\nbool(true)\n");
}

/// Regression guard: exact override signatures (same optional layout, covariant object return) keep compiling and dispatching.
#[test]
fn test_exact_override_signatures_still_dispatch() {
    let out = compile_and_run(
        r#"<?php
abstract class Animal {
    abstract public function speak(string $how = "loud"): string;
    public function intro(): Animal { return $this; }
}
class Dog extends Animal {
    public function speak(string $how = "loud"): string { return "woof " . $how; }
    public function intro(): Dog { return $this; }
}
function talk(Animal $a): string { return $a->speak() . "/" . $a->speak("soft"); }
echo talk(new Dog()), "\n";
echo get_class((new Dog())->intro()), "\n";
"#,
    );
    assert_eq!(out, "woof loud/woof soft\nDog\n");
}

/// `#[\ReturnTypeWillChange]` lets a static override of a builtin class method return `?static` while keeping the boxed return ABI.
#[test]
fn test_return_type_will_change_static_builtin_override() {
    let out = compile_and_run(
        r#"<?php
class MyDate extends DateTime {
    #[\ReturnTypeWillChange]
    public static function createFromFormat(string $format, string $datetime, ?DateTimeZone $timezone = null): ?static
    {
        $d = new MyDate("2020-01-02 03:04:05");
        return $datetime === "" ? null : $d;
    }
}
$d = MyDate::createFromFormat("Y-m-d", "2020-01-02");
echo get_class($d), " ", $d->format("Y-m-d"), "\n";
var_dump(MyDate::createFromFormat("Y-m-d", ""));
"#,
    );
    assert_eq!(out, "MyDate 2020-01-02\nNULL\n");
}

/// Review p10: a renamed variadic with the same element type, called positionally, by name and with spreads.
#[test]
fn test_override_renamed_variadic_with_named_and_spread_args() {
    let out = compile_and_run(
        r#"<?php
class Base { public function m(int $k, array ...$arrays): string { return "B" . $k . ":" . json_encode($arrays); } }
class Child extends Base { public function m(int $k, array ...$lists): string { return "C" . $k . ":" . json_encode($lists); } }
function viaBase(Base $b): string { return $b->m(1, [1], [2]); }
echo viaBase(new Base()), " ", viaBase(new Child()), "\n";
$c = new Child();
echo $c->m(2, lists: [3]), "\n";
echo $c->m(k: 3), "\n";
function viaBaseNamed(Base $b): string { return $b->m(4, arrays: [5]); }
echo viaBaseNamed(new Child()), "\n";
function viaBaseSpread(Base $b): string { return $b->m(5, ...[[6], [7]]); }
echo viaBaseSpread(new Child()), "\n";
"#,
    );
    assert_eq!(out, "B1:[[1],[2]] C1:[[1],[2]]\nC2:{\"lists\":[3]}\nC3:[]\nC4:{\"arrays\":[5]}\nC5:[[6],[7]]\n");
}

/// Review ronda 3 (d4): `= self::X` on parent and override naming the same parent constant is the same default (the `Application::handle(..., $type = self::MAIN_REQUEST)` shape).
#[test]
fn test_override_keeps_inherited_self_constant_default() {
    let out = compile_and_run(
        r#"<?php
abstract class P { const X = 'p'; abstract public function f(string $v = self::X); }
class C extends P { public function f(string $v = self::X) { return "c" . $v; } }
function via(P $b) { echo $b->f(), "\n"; }
via(new C());
echo (new C)->f(), "\n";
"#,
    );
    assert_eq!(out, "cp\ncp\n");
}

/// Review ronda 3 (d7): `= self::MAIN` resolving to the interface constant from both sides.
#[test]
fn test_interface_impl_keeps_interface_self_constant_default() {
    let out = compile_and_run(
        r#"<?php
interface HK { const MAIN = 1; public function handle(int $type = self::MAIN, bool $c = true); }
class App implements HK { public function handle(int $type = self::MAIN, bool $c = true) { return $type; } }
function via(HK $h) { echo $h->handle(), "\n"; }
via(new App()); echo (new App)->handle(), "\n";
"#,
    );
    assert_eq!(out, "1\n1\n");
}

/// Review ronda 3 (c11): union covariance that keeps the boxed representation still dispatches through the parent.
#[test]
fn test_override_union_return_keeping_boxed_representation() {
    let out = compile_and_run(
        r#"<?php
class Node { public $n = "N"; public function find(): Node|string { return "s"; } }
class Twig extends Node { public $n = "T"; public function find(): Twig|string { return $this; } }
class Leaf extends Node { public $n = "L"; public function find(): Leaf|string { return "q"; } }
function via(Node $x) { $r = $x->find(); var_dump(is_object($r) ? $r->n : $r); }
via(new Node()); via(new Twig()); via(new Leaf());
"#,
    );
    assert_eq!(out, "string(1) \"s\"\nstring(1) \"T\"\nstring(1) \"q\"\n");
}

/// Review ronda 4 (x01): parent `self::X` and override `Other::X` resolve to the same value.
#[test]
fn test_override_default_named_constant_matches_parent_self_constant() {
    let out = compile_and_run(
        r#"<?php
class Other { const X = 1; }
class P { const X = 1; public function f(int $v = self::X) { return "p" . $v; } }
class C extends P { public function f(int $v = Other::X) { return "c" . $v; } }
function via(P $p) { echo $p->f(), "\n"; }
via(new P()); via(new C()); echo (new C)->f(), "\n";
"#,
    );
    assert_eq!(out, "p1\nc1\nc1\n");
}

/// Review ronda 4 (x09): parent `parent::X` and override `self::X` both name the grandparent constant.
#[test]
fn test_override_default_parent_constant_from_grandparent() {
    let out = compile_and_run(
        r#"<?php
class G { const X = 'g'; }
class P extends G { public function f(string $v = parent::X) { return "p" . $v; } }
class C extends P { public function f(string $v = self::X) { return "c" . $v; } }
function via(P $p) { echo $p->f(), "\n"; }
via(new P()); via(new C()); echo (new C)->f(), "\n";
"#,
    );
    assert_eq!(out, "pg\ncg\ncg\n");
}

/// Review ronda 4 (x14): `self::X` defined as `self::A * 3` resolves and folds on both sides.
#[test]
fn test_override_default_constant_expression_folds() {
    let out = compile_and_run(
        r#"<?php
class P { const A = 2; const X = self::A * 3; public function f(int $v = self::X) { return "p" . $v; } }
class C extends P { public function f(int $v = self::X) { return "c" . $v; } }
function via(P $p) { echo $p->f(), "\n"; }
via(new P()); via(new C()); echo (new C)->f(), "\n";
"#,
    );
    assert_eq!(out, "p6\nc6\nc6\n");
}

/// Review ronda 4 (x15): `X = self::A` is written in the parent, so a child redefining `A` keeps the same default.
#[test]
fn test_override_default_constant_chain_with_redefined_operand() {
    let out = compile_and_run(
        r#"<?php
class P { const A = 2; const X = self::A; public function f(int $v = self::X) { return "p" . $v; } }
class C extends P { const A = 5; public function f(int $v = self::X) { return "c" . $v; } }
function via(P $p) { echo $p->f(), "\n"; }
via(new P()); via(new C()); echo (new C)->f(), "\n";
"#,
    );
    assert_eq!(out, "p2\nc2\nc2\n");
}

/// Review ronda 4 (x26): the Laravel kernel shape one level down (`MyApp extends App implements HK`, `handle($type = self::MAIN)`).
#[test]
fn test_grandchild_override_default_interface_constant() {
    let out = compile_and_run(
        r#"<?php
interface HK { const MAIN = 1; public function handle(int $type = self::MAIN, bool $c = true); }
class App implements HK { public function handle(int $type = self::MAIN, bool $c = true) { return "a$type"; } }
class MyApp extends App { public function handle(int $type = self::MAIN, bool $c = true) { return "m$type"; } }
function via(HK $h) { echo $h->handle(), "\n"; }
function viaA(App $h) { echo $h->handle(), "\n"; }
via(new App()); via(new MyApp()); viaA(new MyApp()); echo (new MyApp)->handle(), "\n";
"#,
    );
    assert_eq!(out, "a1\nm1\nm1\nm1\n");
}

/// Ronda 5 (P4 family ABI): an untyped override of `int $x` is boxed by the family and dispatches through the parent.
#[test]
fn test_family_override_untyped_param_of_declared_int() {
    let out = compile_and_run(
        r#"<?php
class Base { public function f(int $x): string { return "B$x"; } }
class Child extends Base { public function f($x): string { return "C$x"; } }
function viaBase(Base $b): string { return $b->f(7); }
echo viaBase(new Base()), viaBase(new Child()), "\n";
"#,
    );
    assert_eq!(out, "B7C7\n");
}

/// Ronda 5: `Foo $x` widened to `?Foo $x` is boxed by the family.
#[test]
fn test_family_override_nullable_object_param() {
    let out = compile_and_run(
        r#"<?php
class Foo { public $v = "foo"; }
class Base { public function f(Foo $x): string { return "B" . $x->v; } }
class Child extends Base { public function f(?Foo $x): string { return "C" . ($x === null ? "null" : $x->v); } }
function viaBase(Base $b): string { return $b->f(new Foo()); }
echo viaBase(new Base()), viaBase(new Child()), (new Child())->f(null), "\n";
"#,
    );
    assert_eq!(out, "BfooCfooCnull\n");
}

/// Ronda 5: `int $x` widened to `int|string $x` is boxed and the child sees the int.
#[test]
fn test_family_override_int_param_widened_to_union() {
    let out = compile_and_run(
        r#"<?php
class Base { public function f(int $x): string { return "B$x"; } }
class Child extends Base { public function f(int|string $x): string { return "C" . gettype($x) . $x; } }
function via(Base $b): string { return $b->f(7); }
echo via(new Base()), via(new Child()), (new Child())->f("s"), "\n";
"#,
    );
    assert_eq!(out, "B7Cinteger7Cstrings\n");
}

/// Ronda 5 (c07): `int|null` and `int|Twig` under `Node|int|null` are boxed into the family return.
#[test]
fn test_family_override_union_return_narrowed_to_tagged_scalar() {
    let out = compile_and_run(
        r#"<?php
class Node { public $n = "N"; public function find(): Node|int|null { return 4; } }
class Leaf extends Node { public $n = "L"; public function find(): int|null { return 9; } }
class Twig extends Node { public function find(): int|Twig { return 5; } }
function via(Node $x) { $r = $x->find(); var_dump(is_object($r) ? $r->n : $r); }
via(new Node()); via(new Leaf()); via(new Twig());
"#,
    );
    assert_eq!(out, "int(4)\nint(9)\nint(5)\n");
}

/// Ronda 5 (c10): `false` under `Node|false` keeps `bool(false)` through the family return.
#[test]
fn test_family_override_union_return_narrowed_to_false() {
    let out = compile_and_run(
        r#"<?php
class Node { public $n = "N"; public function find(): Node|false { return false; } }
class Twig extends Node { public $n = "T"; public function find(): Twig|false { return $this; } }
class Leaf extends Node { public $n = "L"; public function find(): false { return false; } }
function via(Node $x) { $r = $x->find(); var_dump(is_object($r) ? $r->n : $r); }
via(new Node()); via(new Twig()); via(new Leaf());
"#,
    );
    assert_eq!(out, "bool(false)\nstring(1) \"T\"\nbool(false)\n");
}

/// Ronda 5 (x20): `?N` narrowed to `N` inside an override family; the object returned through
/// the parent must be boxed by the family return so `$r->n` reads the property.
#[test]
fn test_family_override_object_return_narrowed_from_nullable() {
    let out = compile_and_run(
        r#"<?php
class N { public $n = "N"; }
class P { public function f(): ?N { return null; } }
class C extends P { public function f(): N { return new N(); } }
function via(P $p) { $r = $p->f(); var_dump($r === null ? null : $r->n); }
via(new P()); via(new C());
"#,
    );
    assert_eq!(out, "NULL\nstring(1) \"N\"\n");
}
