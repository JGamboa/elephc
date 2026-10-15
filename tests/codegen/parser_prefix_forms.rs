//! Purpose:
//! End-to-end coverage for prefix and member-name forms found by the P21 parse oracle on the
//! Laravel frontier: unary `+`, reserved words as method/constant names after a dynamic `::` or
//! `->`, and `new (expr)` / `new static::$map[...]` class-name references.
//!
//! Called from:
//! - The native codegen suite (`tests/codegen/mod.rs`).
//!
//! Key details:
//! - Expected output was produced by PHP 8.5 from the same source (`probes/require-expr`).

use crate::support::*;

/// Verifies unary `+` lowers like php-src's `expr * 1`: numeric strings and bools convert,
/// precedence matches unary minus, and arrays or non-numeric strings throw the same TypeError.
#[test]
fn test_unary_plus_semantics() {
    let out = compile_and_run(
        r#"<?php
// Unario + (php-parser ConstExprEvaluator.php:120).
function ev($v) { return $v; }
function plus($v) { return +ev($v); }
var_dump(plus(5), plus(-2.5), plus("12"), plus("1.5e1"), plus(true), plus(null));
$n = 3;
var_dump(+$n, -+$n, +-$n, 2 * +$n, +$n ** 2);
var_dump(random_int(+$n, +$n));
try {
    plus([1]);
} catch (TypeError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
try {
    plus("abc");
} catch (TypeError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
const P = +4;
var_dump(P);
"#,
    );
    assert_eq!(
        out,
        r#"int(5)
float(-2.5)
int(12)
float(15)
int(1)
int(0)
int(3)
int(-3)
int(-3)
int(6)
int(9)
int(3)
TypeError: Unsupported operand types: array * int
TypeError: Unsupported operand types: string * int
int(4)
"#
    );
}

/// Verifies reserved words are method and constant names after a dynamic `::` receiver
/// (`$factory::new()`, `static::$factory::new()`, `$cls::match()`) and after `->`.
#[test]
fn test_keyword_member() {
    let out = compile_and_run(
        r#"<?php
// Nombres de método/constante reservados tras :: y -> (Factory.php:1030).
class Factory
{
    const DEFAULT = 'def';
    const LIST = 'lst';
    public static function new(): string { return 'new!'; }
    public static function match(string $s): bool { return $s === 'x'; }
    public function list(): string { return 'list!'; }
    public function print(): string { return 'print!'; }
    public static function make(): string { $f = static::class; return $f::new(); }
}
class Holder
{
    public static $factory = Factory::class;
    public static function build(): string { return static::$factory::new(); }
}
$factory = Factory::class;
echo $factory::new(), ' ', Holder::build(), ' ', Factory::make(), "\n";
var_dump($factory::match('x'), $factory::match('y'));
$o = new Factory();
echo $o->list(), ' ', $o->print(), ' ', $o::new(), "\n";
echo $factory::DEFAULT, ' ', $o::LIST, ' ', Factory::DEFAULT, "\n";
"#,
    );
    assert_eq!(
        out,
        r#"new! new! new!
bool(true)
bool(false)
list! print! new!
def lst def
"#
    );
}

/// Verifies `new (expr)` and `new static::$classes[static::class]($m)` build the class the
/// expression names at runtime, and an unknown name throws `Error`.
#[test]
fn test_new_expr() {
    let out = compile_and_run(
        r#"<?php
// new (expr) y new static::$x[...] (BelongsToMany.php:408, HasCollection.php:30).
class Pivot { public function __construct(public string $tag = 'p') {} }
class Other extends Pivot {}
class Collection { public function __construct(public array $items = []) {} }
class Model
{
    protected static array $classes = [];
    public function getPivotClass(): string { return Pivot::class; }
    public function pivot(): Pivot { return (new ($this->getPivotClass())('x')); }
    public static function boot(): void { static::$classes[static::class] = Collection::class; }
    public function collect(array $m): Collection { return new static::$classes[static::class]($m); }
}
$m = new Model();
echo get_class($m->pivot()), ' ', $m->pivot()->tag, ' ', (new ($m->getPivotClass()))->tag, "\n";
Model::boot();
echo count($m->collect([1, 2, 3])->items), "\n";
$c = true;
echo get_class(new ($c ? Other::class : Pivot::class)('z')), "\n";
echo (new ('Oth' . 'er'))->tag, "\n";
try {
    new ('No' . 'Such');
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
"#,
    );
    assert_eq!(
        out,
        r#"Pivot x p
3
Other
p
Error: Class "NoSuch" not found
"#
    );
}
