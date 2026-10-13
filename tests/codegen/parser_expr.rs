//! Purpose:
//! End-to-end coverage for expression-level parser and lowering fixes found on the Laravel
//! frontier: subscripts over `mixed` call results, `$obj::staticMethod()`, and the PHP 8.x
//! expression forms listed in the lookahead inventory.
//!
//! Called from:
//! - The native codegen suite (`tests/codegen/mod.rs`).
//!
//! Key details:
//! - Expected output was produced by PHP 8.5 from the same source.
//! - Receivers come from functions returning `mixed`, so no pass can narrow them statically.

use crate::support::*;

/// Verifies indexing a `mixed` value reads string offsets, array elements and `ArrayAccess`
/// offsets like PHP, whether the receiver is a call result or a local (monolog's
/// `$client::VERSION[0]` shape used to print an empty string).
#[test]
fn test_mixed_receiver_subscript_reads_string_array_and_array_access() {
    let out = compile_and_run(
        r#"<?php
        class Box implements ArrayAccess {
            public function offsetExists(mixed $o): bool { return true; }
            public function offsetGet(mixed $o): mixed { return "box[$o]"; }
            public function offsetSet(mixed $o, mixed $v): void {}
            public function offsetUnset(mixed $o): void {}
        }
        function s(): mixed { return '8.1'; }
        function a(): mixed { return ['x' => 1, 2 => 'two', 0 => 'zero']; }
        function o(): mixed { return new Box(); }
        function pick(int $k): mixed { $all = ['abc', [5, 6], new Box()]; return $all[$k]; }
        echo s()[0], '|', s()[2], '|', a()['x'], '|', a()[2], '|', a()[0], '|', o()['q'], "\n";
        echo pick(0)[1], '|', pick(1)[1], '|', pick(2)[7], '|', strlen(s()[1]), "\n";
        $m = s(); echo $m[0], $m[-1], "\n";
        "#,
    );
    assert_eq!(out, "8|1|1|two|zero|box[q]\nb|6|box[7]|1\n81\n");
}

/// Verifies `??` over a `mixed` string offset stays silent and falls back on a miss, and that a
/// `string|array` union receiver also reads string offsets.
#[test]
fn test_mixed_receiver_subscript_silent_and_union_string() {
    let out = compile_and_run(
        r#"<?php
        function s(): mixed { return 'ab'; }
        function u(bool $str): string|array { return $str ? 'xy' : ['p', 'q']; }
        echo s()[1] ?? 'd', '|', s()[5] ?? 'd', '|', u(true)[1], '|', u(false)[1], "\n";
        "#,
    );
    assert_eq!(out, "b|d|y|q\n");
}

/// Verifies the string-offset arm over a `mixed` receiver frees what it allocates: owned call
/// results, locals, `??` misses and boxed keys, read in a loop under heap debug.
#[test]
fn test_mixed_receiver_subscript_string_arm_is_leak_free() {
    let output = compile_and_run_with_heap_debug(
        r#"<?php
        function v(int $i): mixed { return $i % 2 ? 'v' . $i . '.0' : ['k' . $i, 'w']; }
        function idx(mixed $k): mixed { return $k; }
        $out = '';
        for ($i = 0; $i < 4; $i++) {
            $m = v($i);
            $out .= v($i)[1] . $m[0] . (v($i)[9] ?? '-') . v($i)[idx(1)] . ',';
        }
        echo $out;
        "#,
    );
    assert!(output.success, "stdout={:?}\nstderr={}", output.stdout, output.stderr);
    assert_eq!(output.stdout, "wk0-w,1v-1,wk2-w,3v-3,");
    assert!(output.stderr.contains("HEAP DEBUG: leak summary: clean"), "{}", output.stderr);
}

/// Verifies an out-of-range offset on a `mixed` string still warns like PHP's string read.
#[test]
fn test_mixed_receiver_subscript_string_miss_warns() {
    let output = compile_and_run_capture(
        r#"<?php
        function v(): mixed { return 'v1'; }
        echo v()[7], "|end";
        "#,
    );
    assert_eq!(output.stdout, "|end");
    assert!(output.stderr.contains("Uninitialized string offset 7"), "{}", output.stderr);
}

/// Verifies `$receiver::method()` dispatches on the receiver's runtime class: statics resolve
/// with late static binding, `$this::inst()` runs on `$this`, an instance method from outside
/// throws PHP's "cannot be called statically" Error, and an unknown method is a catchable Error.
#[test]
fn test_dynamic_receiver_static_method_call() {
    let out = compile_and_run(
        r#"<?php
        class A {
            public static function hi($x) { return "A:$x"; }
            public static function who() { return static::class; }
            public static function log($m): void { echo "log:$m\n"; }
            public function inst() { return 'inst:' . static::class; }
            public function viaThis() { return $this::inst() . '/' . $this::who(); }
        }
        class B extends A { public static function hi($x) { return "B:$x"; } }
        function m(mixed $r) { return $r::hi('m') . '/' . $r::who(); }
        $a = new A; $b = new B; $c = 'b';
        echo $a::hi(1), '|', $b::hi(2), '|', $c::hi(3), '|', $b::who(), "\n";
        echo m($b), '|', m('A'), '|', $b->viaThis(), "\n";
        var_dump($b::log('x'));
        try { $a::inst(); } catch (\Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
        try { $a::nope(); } catch (\Error $e) { echo $e->getMessage(), "\n"; }
        "#,
    );
    assert_eq!(
        out,
        "A:1|B:2|B:3|B\nB:m/B|A:m/A|inst:B/B\nlog:x\nNULL\n\
         Error: Non-static method A::inst() cannot be called statically\n\
         Call to undefined method A::nope()\n"
    );
}

/// Verifies the variable-method-name form (`$cls::$m()`) keeps its existing callable path.
#[test]
fn test_dynamic_receiver_variable_method_name_still_works() {
    let out = compile_and_run(
        r#"<?php
        class A { public static function hi() { return 'hi'; } }
        $c = 'A'; $m = 'hi';
        echo $c::$m(), "\n";
        "#,
    );
    assert_eq!(out, "hi\n");
}

/// Verifies a by-reference parameter reached through `$receiver::method()` is refused at compile
/// time instead of silently passing a copy.
#[test]
fn test_dynamic_receiver_static_call_by_ref_is_refused() {
    let error = compile_cli_file_with_flags_expect_failure(
        r#"<?php
        class A { public static function bump(&$n) { $n++; } }
        $b = new A; $n = 1;
        $b::bump($n); echo $n;
        "#,
        &[],
    );
    assert!(error.contains("takes a by-reference parameter"), "{error}");
}

/// Verifies `--strict-php` accepts `$obj::method()` although it desugars to an internal call.
#[test]
fn test_dynamic_receiver_static_call_is_accepted_by_strict_php() {
    let out = compile_cli_file_and_run_with_flags(
        r#"<?php
        class Shell { public static function v() { return 'v1'; } }
        function v($shell) { return $shell::v(); }
        echo v(new Shell()), v('Shell');
        "#,
        &["--strict-php"],
    );
    assert_eq!(out, "v1v1");
}

/// Verifies `call()[] = $v` in expression position (Laravel's `defer()` arrow fn): an
/// `ArrayAccess` result gets `offsetSet(null, $v)`, an array result is a discarded temporary, a
/// string throws PHP's Error, and the expression yields `$v`.
#[test]
fn test_append_to_call_result_in_expression() {
    let out = compile_and_run(
        r#"<?php
        class Coll implements ArrayAccess {
            public static array $log = [];
            public function offsetExists(mixed $o): bool { return true; }
            public function offsetGet(mixed $o): mixed { return 1; }
            public function offsetSet(mixed $o, mixed $v): void { self::$log[] = var_export($o, true) . '=' . $v; }
            public function offsetUnset(mixed $o): void {}
        }
        function app(): mixed { return new Coll; }
        function typed(): Coll { return new Coll; }
        function arr(): array { return [1]; }
        function txt(): mixed { return 'abc'; }
        $f = fn($d) => app()[] = $d;
        echo $f('a'), $f('b'), (fn() => typed()[] = 'c')(), '|', implode(',', Coll::$log), "\n";
        echo (fn() => arr()[] = 9)(), "\n";
        try { (fn() => txt()[] = 'z')(); } catch (\Error $e) { echo $e->getMessage(), "\n"; }
        "#,
    );
    assert_eq!(out, "abc|NULL=a,NULL=b,NULL=c\n9\n[] operator not supported for strings\n");
}

/// Verifies PHP's `new` class-name references: `new static::$prop(...)` (late static binding),
/// `new self::$prop`, `new C::$prop`, `new $obj->prop(...)` and `new $obj->map['k'](...)`.
#[test]
fn test_new_with_dynamic_class_name_reference() {
    let out = compile_and_run(
        r#"<?php
        class Q { public function __construct(public string $v = "q") {} }
        class R extends Q {}
        class M2 {
            public static $builder = Q::class;
            public static function make($x) { return new static::$builder($x); }
            public static function bare() { return new self::$builder; }
        }
        class M3 extends M2 { public static $builder = R::class; }
        class P { public $cls = R::class; public $map = ['q' => Q::class]; }
        $p = new P;
        echo get_class(M2::make('a')), M2::make('a')->v, '|', get_class(M3::make('b')), '|', M2::bare()->v, "\n";
        echo get_class(new $p->cls('c')), '|', (new $p->map['q']('d'))->v, '|', get_class(new M2::$builder), "\n";
        $names = ['Q']; echo get_class(new $p->cls), (new $names[0])->v, "\n";
        "#,
    );
    assert_eq!(out, "Qa|R|q\nR|d|Q\nRq\n");
}

/// Verifies `new Foo` / `new $class(...)` keep compiling as before.
#[test]
fn test_new_plain_forms_still_work() {
    let out = compile_and_run(
        r#"<?php
        class Q { public function __construct(public string $v = "q") {} }
        $c = 'Q';
        echo (new Q)->v, (new $c('x'))->v, new Q('y')->v, "\n";
        "#,
    );
    assert_eq!(out, "qxy\n");
}
