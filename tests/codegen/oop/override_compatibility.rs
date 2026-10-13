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
