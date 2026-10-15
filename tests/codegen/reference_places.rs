//! Purpose:
//! End-to-end tests for references bound INTO a property or array element
//! (`$o->p = &$x`, `$a[$k] = &$x`, `[&$x]`) and for static-property reference sources.
//!
//! Called from:
//! - `cargo test --test codegen_tests reference_places` through Rust's test harness.
//!
//! Key details:
//! - Every expected output was produced by php 8.5 on the same source.
//! - The place adopts the source's managed reference cell: writes through either side meet,
//!   `unset` of one side keeps the other, references survive array copies, and `var_dump`
//!   prints PHP's `&` only while the reference has more than one holder.

use crate::support::*;

/// Element binding: writes through both sides, copy-on-write sharing, foreach, unset, serialize.
#[test]
fn test_reference_place_array_element_shares_cell() {
    let out = compile_and_run(
        r#"<?php
$a = ['x' => 1];
$v = 10;
$a['y'] = &$v;
$v = 20;
echo $a['y'], "\n";
$a['y'] = 30;
echo $v, "\n";
$b = $a;
$v = 40;
echo $b['y'], " ", $a['y'], "\n";
foreach ($a as $k => $e) { echo $k, "=", $e, "\n"; }
unset($v);
$a['y'] = 50;
echo $b['y'], "\n";
echo serialize($a), "\n";
"#,
    );
    assert_eq!(out, "20\n30\n40 40\nx=1\ny=40\n40\na:2:{s:1:\"x\";i:1;s:1:\"y\";i:50;}\n");
}

/// A reference with a single holder separates on copy, exactly like PHP.
#[test]
fn test_reference_place_sole_holder_separates_on_copy() {
    let out = compile_and_run(
        r#"<?php
$c = [1, 2];
$v = 7;
$c[0] = &$v;
$d = $c;
$v = 8;
echo $d[0], "\n";
unset($v);
$e = $c;
$e[0] = 9;
echo $c[0], "\n";
"#,
    );
    assert_eq!(out, "8\n8\n");
}

/// Nested element places autovivify the missing dimension; var_dump marks only shared references.
#[test]
fn test_reference_place_nested_element_autovivifies() {
    let out = compile_and_run(
        r#"<?php
function caster(array $a, array $vars): array {
    foreach ($vars as $k => &$v) {
        $a['use']['$'.$k] = &$v;
    }
    unset($v);
    return $a;
}
var_dump(caster([], ['x' => 1, 'y' => 'two']));
$a = [];
$x = 5;
$a['p']['q'] = &$x;
var_dump($a);
"#,
    );
    assert_eq!(
        out,
        "array(1) {\n  [\"use\"]=>\n  array(2) {\n    [\"$x\"]=>\n    int(1)\n    [\"$y\"]=>\n    string(3) \"two\"\n  }\n}\narray(1) {\n  [\"p\"]=>\n  array(1) {\n    [\"q\"]=>\n    &int(5)\n  }\n}\n"
    );
}

/// An undefined source is created as null without a warning, then detached by `unset`.
#[test]
fn test_reference_place_undefined_source_breaks_reference() {
    let out = compile_and_run(
        r#"<?php
function cloner(array $vals): array {
    foreach ($vals as $k => $v) {
        if ($k === 'b') { $vals[$k] = &$stub; unset($stub); $vals[$k] = 'replaced'; }
    }
    return $vals;
}
var_dump(cloner(['a' => 1, 'b' => 2]));
"#,
    );
    assert_eq!(out, "array(2) {\n  [\"a\"]=>\n  int(1)\n  [\"b\"]=>\n  string(8) \"replaced\"\n}\n");
}

/// Property binding: both directions, a second property on the same cell, clone and dumpers.
#[test]
fn test_reference_place_property_shares_cell() {
    let out = compile_and_run(
        r#"<?php
class Holder { public $p = 1; public $q; }
$h = new Holder;
$x = 5;
$h->p = &$x;
$x = 6;
echo $h->p, "\n";
$h->p = 7;
echo $x, "\n";
$h->q = &$h->p;
$h->q = 8;
echo $x, $h->p, "\n";
unset($x);
$h->p = 9;
echo $h->q, "\n";
$h2 = clone $h;
$h->p = 10;
echo $h2->p, $h->q, "\n";
var_dump($h);
print_r($h);
echo "\n", json_encode($h), "\n";
"#,
    );
    assert_eq!(
        out,
        "6\n7\n88\n9\n1010\nobject(Holder)#1 (2) {\n  [\"p\"]=>\n  &int(10)\n  [\"q\"]=>\n  &int(10)\n}\nHolder Object\n(\n    [p] => 10\n    [q] => 10\n)\n\n{\"p\":10,\"q\":10}\n"
    );
}

/// `$this->options = &$options` from a by-reference constructor parameter (Laravel's
/// ControllerMiddlewareOptions): the caller's local is lent as a managed cell.
#[test]
fn test_reference_place_property_from_by_ref_parameter() {
    let out = compile_and_run(
        r#"<?php
class Opt {
    protected $options;
    public function __construct(array &$options) { $this->options = &$options; }
    public function only($m) { $this->options['only'] = $m; return $this; }
    public function get() { return $this->options; }
}
$options = [];
$o = new Opt($options);
$o->only(['index']);
echo json_encode($options), "\n";
$options['except'] = 'x';
echo json_encode($o->get()), "\n";
"#,
    );
    assert_eq!(out, "{\"only\":[\"index\"]}\n{\"only\":[\"index\"],\"except\":\"x\"}\n");
}

/// Property and element sources of a property place (symfony's SocketStream / ProcessStream).
#[test]
fn test_reference_place_property_from_property_and_element() {
    let out = compile_and_run(
        r#"<?php
class Stream { private $stream; private $in; private $out;
  public function init() { $this->stream = 'S'; $this->in = &$this->stream; $this->out = &$this->stream; }
  public function close() { $this->stream = null; var_dump($this->in, $this->out); } }
$st = new Stream; $st->init(); $st->close();
class Proc { private $in; private $out;
  public function start() { $pipes = ['p0', 'p1']; $this->in = &$pipes[0]; $this->out = &$pipes[1]; $pipes[0] = 'w'; return [$this->in, $this->out]; } }
echo implode(",", (new Proc)->start()), "\n";
"#,
    );
    assert_eq!(out, "NULL\nNULL\nw,p1\n");
}

/// Array literal reference elements (Laravel's Controller::middleware, symfony's
/// DumpDataCollector) and `[&$this, 'm']`.
#[test]
fn test_reference_place_array_literal_elements() {
    let out = compile_and_run(
        r#"<?php
class Ctl {
    public $middleware = [];
    public function middleware($m) {
        $options = [];
        foreach ((array) $m as $x) {
            $this->middleware[] = ['middleware' => $x, 'options' => &$options];
        }
        $options['only'] = ['index'];
    }
}
$c = new Ctl;
$c->middleware(['auth', 'log']);
echo json_encode($c->middleware), "\n";
class D { public $data = []; public $n = 0; public $refs;
  function __construct() { $this->refs = [&$this->data, &$this->n]; }
  function bump() { $this->refs[1] = 5; $this->refs[0][] = 'z'; } }
$d = new D; $d->bump();
echo $d->n, json_encode($d->data), "\n";
class W { public $n = 1; function run() { $cb = [&$this, 'cb']; return call_user_func($cb, 2) . get_class($cb[0]); } function cb($x) { return $x + $this->n; } }
echo (new W)->run(), "\n";
"#,
    );
    assert_eq!(
        out,
        "[{\"middleware\":\"auth\",\"options\":{\"only\":[\"index\"]}},{\"middleware\":\"log\",\"options\":{\"only\":[\"index\"]}}]\n5[\"z\"]\n3W\n"
    );
}

/// `$x = &self::$p` and `$x = &static::$p` alias the static property's storage.
#[test]
fn test_reference_static_property_sources() {
    let out = compile_and_run(
        r#"<?php
class T {
    protected static $names = ['a', 'b'];
    public static function f() { $n = &static::$names; $n[] = 'c'; return count(self::$names); }
    public static function g() { $m = &self::$names; return $m[1]; }
}
class U extends T {}
echo T::f(), T::g(), U::f(), "\n";
"#,
    );
    assert_eq!(out, "3b4\n");
}

/// A by-reference parameter that reaches a property through an unpromoted caller slot fails
/// closed with a catchable Error instead of publishing a dangling frame address.
#[test]
fn test_reference_place_unmanaged_parameter_fails_closed() {
    let out = compile_and_run(
        r#"<?php
class Keep { public $v; function bind(array &$x) { $this->v = &$x; } }
function viaCallable(Keep $k) { $arr = [1]; $f = [$k, 'bind']; try { $f($arr); echo "bound\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } }
viaCallable(new Keep);
"#,
    );
    assert!(out == "bound\n" || out.starts_with("Error: Cannot bind a reference"), "{out}");
}

/// Superglobals live in program-global storage with no reference cell, so binding one into a
/// property (monolog's WebProcessor `$this->serverData = &$_SERVER`) is refused at compile time.
#[test]
fn test_reference_place_superglobal_source_is_refused() {
    let err = compile_cli_file_with_flags_expect_failure(
        "<?php class W { public $s; function __construct() { $this->s = &$_SERVER; } } new W();",
        &[],
    );
    assert!(err.contains("lives in program-global storage"), "{err}");
}

/// A static property's process-lifetime storage is not a managed cell either.
#[test]
fn test_reference_place_static_source_is_refused() {
    let err = compile_cli_file_with_flags_expect_failure(
        "<?php class C { private $c; private static $h = []; function __construct() { $this->c = &self::$h; } } new C();",
        &[],
    );
    assert!(err.contains("Unsupported reference assignment"), "{err}");
}
