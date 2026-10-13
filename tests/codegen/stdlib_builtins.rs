//! Purpose:
//! End-to-end tests for the PHP-written stdlib prelude (`src/stdlib_prelude/*.php`): `preg_quote`,
//! `strspn`, `strcspn`, `strrchr`, `strip_tags`, `substr_compare`, `get_debug_type`,
//! `http_build_query`, `version_compare`, `random_bytes`, `pack` and `unpack`.
//!
//! Called from:
//! - `cargo test --test codegen_tests stdlib_builtins` through Rust's test harness.
//!
//! Key details:
//! - Each fixture is a probe from `probes/builtins` and its expected output was generated with
//!   `php` 8.5.10, so these assert byte-for-byte PHP parity.
//! - The last tests pin that code which never touches the prelude is unchanged and that a user
//!   declaration of the same name wins over the injected one.

use crate::support::*;

/// Parity with PHP 8.5.10 for the `guarded_polyfill` probe.
#[test]
fn test_stdlib_guarded_polyfill_matches_php() {
    let source = r####"<?php
if (!function_exists('get_debug_type')) {
    function get_debug_type($v) { return "polyfill"; }
}
if (!function_exists('random_bytes')) {
    function random_bytes($n) { return "polyfill"; }
}
echo get_debug_type(1), "|", strlen(random_bytes(3)), "\n";
"####;
    let expected = r####"int|3
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `http_build_query` probe.
#[test]
fn test_stdlib_http_build_query_matches_php() {
    let source = r####"<?php
echo http_build_query(['a' => 1, 'b' => 'x y', 'c' => [1, 2], 'd' => ['k' => 'v', 'n' => [true, false]]]), "\n";
echo http_build_query([1, 2, 'x' => null, 'y' => 'é&=']), "\n";
echo http_build_query([1, 2], 'p_'), "\n";
echo http_build_query(['a' => 'b c', 'd' => 'e'], '', ';'), "\n";
echo http_build_query(['a' => 'b c~'], '', '&', PHP_QUERY_RFC3986), "\n";
echo http_build_query(['a b' => 'c', 'f' => 1.5, 'g' => 0.1, 'h' => 2.0]), "\n";
$o = new stdClass; $o->x = 1; $o->y = [3];
echo http_build_query($o), "|", http_build_query([]), "|\n";
"####;
    let expected = r####"a=1&b=x+y&c%5B0%5D=1&c%5B1%5D=2&d%5Bk%5D=v&d%5Bn%5D%5B0%5D=1&d%5Bn%5D%5B1%5D=0
0=1&1=2&y=%C3%A9%26%3D
p_0=1&p_1=2
a=b+c;d=e
a=b%20c~
a+b=c&f=1.5&g=0.1&h=2
x=1&y%5B0%5D=3||
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `namespaced_callable` probe.
#[test]
fn test_stdlib_namespaced_callable_matches_php() {
    let source = r####"<?php
namespace App\Util;
echo \get_debug_type(1), "|", get_debug_type("x"), "\n";
$f = strspn(...);
echo $f("aab", "a"), "\n";
echo implode(",", array_map('strrev', ['ab'])), "|", call_user_func('version_compare', '1.2', '1.10'), "\n";
var_dump(function_exists('pack'), function_exists('unpack'));
echo \strrchr("a.b.c", "."), PHP_EOL;
echo bin2hex(\pack("N", 258)), PHP_EOL;
"####;
    let expected = r####"int|string
2
ba|-1
bool(true)
bool(true)
.c
00000102
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `pack_laravel_usage` probe.
#[test]
fn test_stdlib_pack_laravel_usage_matches_php() {
    let source = r####"<?php
echo unpack('N', "\x00\x00\x00\x2a")[1], "\n";
$hex = (array) unpack('H*hex', "\xde\xad\xbe\xef");
echo $hex['hex'], "\n";
echo bin2hex(pack('H32', md5('abc'))), "\n";
$words = [1, 2, 258];
echo bin2hex(pack('V*', ...$words)), "\n";
echo bin2hex(pack('n*', ...array_reverse([1, 2, 3]))), "\n";
$u = unpack('n*', "\x00\x01\x00\x02");
echo implode(",", array_values($u)), "\n";
echo implode(",", array_values(unpack('C*', "AB"))), "\n";
$r = unpack('N2', "\x00\x00\x00\x01\x00\x00\x00\x02");
echo $r[1] + $r[2], "\n";
echo bin2hex(pack('E', 1.0)) === "3ff0000000000000" ? "ok" : "bad", "\n";
echo unpack('J', pack('E', 1.0))[1], "\n";
echo bin2hex(pack('vC2v4C12@32', 1, 2, 3, 4, 5, 6, 7, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)), "\n";
"####;
    let expected = r####"42
deadbeef
900150983cd24fb0d6963f7d28e17f72
010000000200000002010000
000300020001
1,2
65,66
3
ok
4607182418800017408
0100020304000500060007000102030405060708090a0b0c0000000000000000
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `pack_unpack` probe.
#[test]
fn test_stdlib_pack_unpack_matches_php() {
    let source = r####"<?php
echo bin2hex(pack("nvc*", 0x1234, 0x5678, 65, 66)), "\n";
echo bin2hex(pack("NVJPq", 1, 2, 3, 4, -5)), "\n";
echo bin2hex(pack("A5a5Z5", "ab", "cd", "efghij")), "\n";
echo bin2hex(pack("H*h*", "1f2e3", "1f2e3")), "\n";
echo bin2hex(pack("vC2v4C12@32", 1, 2, 3, 4, 5, 6, 7, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)), "\n";
echo bin2hex(pack("EeGgdf", 1.5, -2.25, 0.1, 3.0, 1e100, 0.1)), "\n";
echo bin2hex(pack("s2x2X1", -2, 300)), "\n";
print_r(unpack("N", "\x00\x00\x01\x00"));
print_r(unpack("nbig/vlittle/Cbyte", "\x01\x02\x01\x02\xff"));
print_r(unpack("C*", "abc"));
print_r(unpack("n2x/a3s", "\x00\x01\x00\x02abc"));
print_r(unpack("H*hex", "\x1f\xe2"));
print_r(unpack("c2chars/a*rest", "\xff\x7fzz"));
print_r(unpack("Jv", pack("J", PHP_INT_MAX)));
print_r(unpack("qv/Qw", pack("qQ", -7, 7)));
print_r(unpack("Ef/Gg/dd", pack("EGd", 1.5, -0.5, 3.14159)));
print_r(unpack("a2x/A3y/Z*z", "hi  ab \0qqq"));
print_r(unpack("N", "xxxx\x00\x00\x00\x07", 4));
"####;
    let expected = r####"123478564142
000000010200000000000000000000030400000000000000fbffffffffffffff
616220202063640000006566676800
1f2e30f1e203
0100020304000500060007000102030405060708090a0b0c0000000000000000
3ff800000000000000000000000002c03dcccccd000040407dc39425ad49b254cdcccc3d
feff2c0100
Array
(
    [1] => 256
)
Array
(
    [big] => 258
    [little] => 513
    [byte] => 255
)
Array
(
    [1] => 97
    [2] => 98
    [3] => 99
)
Array
(
    [x1] => 1
    [x2] => 2
    [s] => abc
)
Array
(
    [hex] => 1fe2
)
Array
(
    [chars1] => -1
    [chars2] => 127
    [rest] => zz
)
Array
(
    [v] => 9223372036854775807
)
Array
(
    [v] => -7
    [w] => 7
)
Array
(
    [f] => 1.5
    [g] => -0.5
    [d] => 3.14159
)
Array
(
    [x] => hi
    [y] =>   a
    [z] => b 
)
Array
(
    [1] => 7
)
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `preg_quote_debugtype` probe.
#[test]
fn test_stdlib_preg_quote_debugtype_matches_php() {
    let source = r####"<?php
echo preg_quote("Hello.World?[1+1=2]"), "\n";
echo preg_quote("a/b#c", "/"), "\n";
echo preg_quote("x\\y^$|(){}<>!:-"), "\n";
echo bin2hex(preg_quote("a\0b")), "\n";
echo preg_quote(""), "|\n";
echo get_debug_type(null), get_debug_type(true), get_debug_type(1), get_debug_type(1.5), "\n";
echo get_debug_type("s"), get_debug_type([1]), get_debug_type(new stdClass), "\n";
echo get_debug_type(function () {}), "\n";
echo get_debug_type(new class {}), "\n";
"####;
    let expected = r####"Hello\.World\?\[1\+1\=2\]
a\/b\#c
x\\y\^\$\|\(\)\{\}\<\>\!\:\-
615c30303062
|
nullboolintfloat
stringarraystdClass
Closure
class@anonymous
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `random_bytes` probe.
#[test]
fn test_stdlib_random_bytes_matches_php() {
    let source = r####"<?php
$a = random_bytes(16); $b = random_bytes(16);
var_dump(strlen($a), strlen(random_bytes(1)), $a === $b, strlen(bin2hex($a)));
try { random_bytes(0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$seen = [];
for ($i = 0; $i < 4000; $i++) { $seen[ord(random_bytes(1))] = true; }
var_dump(count($seen) > 200);
"####;
    let expected = r####"int(16)
int(1)
bool(false)
int(32)
random_bytes(): Argument #1 ($length) must be greater than 0
bool(true)
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `strip_tags` probe.
#[test]
fn test_stdlib_strip_tags_matches_php() {
    let source = r####"<?php
echo strip_tags("<p>Hello <b>World</b></p>"), "\n";
echo strip_tags("<p>Hello <b>World</b></p>", "<b>"), "\n";
echo strip_tags("<p>Hello <b>World</b><br/></p>", ["p", "br"]), "\n";
echo strip_tags("a < b and c > d"), "\n";
echo strip_tags("1<2"), "\n";
echo strip_tags("x<!-- comment -->y<?php echo 1; ?>z"), "\n";
echo strip_tags('<a href="x>y">link</a> text'), "\n";
echo strip_tags("<script>alert(1)</script>ok"), "\n";
echo strip_tags("unterminated <b"), "|\n";
echo strip_tags("<P>Upper</P>", "<p>"), "\n";
echo strip_tags("<<b>>x"), "\n";
"####;
    let expected = r####"Hello World
Hello <b>World</b>
<p>Hello World<br/></p>
a < b and c > d
1
xyz
link text
alert(1)ok
unterminated |
<P>Upper</P>
x
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `strip_tags_corpus` probe.
#[test]
fn test_stdlib_strip_tags_corpus_matches_php() {
    let source = r####"<?php
$cases = ["<b>bold</b> text", "a<br>b<br/>c<br />d", "<a href='x'>l</a>", "if (a<b && c>d) {}", "<!DOCTYPE html><html>x</html>", "<p class=\"a>b\">q</p>", "text <", "<>empty", "< b>sp</ b>", "<b\n>nl</b>", "x<?= 1 ?>y", "<style>p{}</style>z", "5 > 3 and 2 < 4", "<img src=x onerror=alert(1)>t", "a<!-- c -->b<!-- d"];
foreach ($cases as $c) {
    echo json_encode(strip_tags($c)), json_encode(strip_tags($c, "<b><p>")), json_encode(strip_tags($c, ["br", "a"])), "\n";
}
"####;
    let expected = r####""bold text""<b>bold<\/b> text""bold text"
"abcd""abcd""a<br>b<br\/>c<br \/>d"
"l""l""<a href='x'>l<\/a>"
"if (ad) {}""if (a<b && c>d) {}""if (ad) {}"
"x""x""x"
"q""<p class=\"ab\">q<\/p>""q"
"text ""text ""text "
"empty""empty""empty"
"< b>sp""< b>sp""< b>sp"
"nl""<b\n>nl<\/b>""nl"
"xy""xy""xy"
"p{}z""p{}z""p{}z"
"5 > 3 and 2 < 4""5 > 3 and 2 < 4""5 > 3 and 2 < 4"
"t""t""t"
"ab""ab""ab"
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `strrchr_substr_compare` probe.
#[test]
fn test_stdlib_strrchr_substr_compare_matches_php() {
    let source = r####"<?php
var_dump(strrchr("/usr/local/bin", "/"));
var_dump(strrchr("abc", "z"));
var_dump(strrchr("abcabc", "bx"));
var_dump(strrchr("héllo", "l"));
var_dump(substr_compare("Hello", "ello", 1));
var_dump(substr_compare("Hello", "ELLO", 1, 4, true));
var_dump(substr_compare("abcde", "cd", -3, 2));
var_dump(substr_compare("abcde", "bc", 1, 3));
var_dump(substr_compare("abcde", "BC", 1, 2));
var_dump(substr_compare("abcde", "zz", 1, 0));
try { substr_compare("abc", "a", 9); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { substr_compare("abc", "a", 0, -1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
"####;
    let expected = r####"string(4) "/bin"
bool(false)
string(2) "bc"
string(2) "lo"
int(0)
int(0)
int(0)
int(1)
int(32)
int(0)
substr_compare(): Argument #3 ($offset) must be contained in argument #1 ($haystack)
substr_compare(): Argument #4 ($length) must be greater than or equal to 0
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `strspn_strcspn` probe.
#[test]
fn test_stdlib_strspn_strcspn_matches_php() {
    let source = r####"<?php
var_dump(strspn("42 is the answer", "1234567890"));
var_dump(strspn("foo", "o", 1, 2));
var_dump(strspn("foo", "o", -2));
var_dump(strspn("foo", "o", 1, -1));
var_dump(strspn("abc", "abc", 5));
var_dump(strspn("", "abc"));
var_dump(strcspn("abcd", "cd"));
var_dump(strcspn("abcd", "cd", -3));
var_dump(strcspn("hello", "z"));
var_dump(strcspn("hello", "l", 0, 2));
var_dump(strcspn("abc", "x", 1, -5));
var_dump(strcspn("abc", "", 0));
"####;
    let expected = r####"int(2)
int(2)
int(2)
int(1)
int(0)
int(0)
int(2)
int(1)
int(5)
int(2)
int(0)
int(3)
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Parity with PHP 8.5.10 for the `version_compare` probe.
#[test]
fn test_stdlib_version_compare_matches_php() {
    let source = r####"<?php
$vs = ["1.0", "1.0.0", "1.0.1", "1.0rc1", "1.0RC2", "1.0-dev", "1.0alpha", "1.0b2", "1.0pl1", "1.0.0-beta.2", "8.5.10", "8.10.0", "", "1", "1.0.", "5.2", "5.2.0-dev", "v1.2"];
$out = [];
foreach ($vs as $a) { foreach ($vs as $b) { $out[] = version_compare($a, $b); } }
echo md5(implode(",", $out)), "\n";
echo implode(",", array_slice($out, 0, 40)), "\n";
var_dump(version_compare("8.5.10", "8.2", ">="));
var_dump(version_compare("1.0", "1.1", "lt"));
var_dump(version_compare("1.0", "1.0.0", "eq"));
var_dump(version_compare("1.0", "1.0", "<>"));
try { version_compare("1", "2", "bad"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
"####;
    let expected = r####"b63798b31b148f4482a273514b8e38db
0,-1,-1,1,1,1,1,1,-1,-1,-1,-1,1,1,0,-1,-1,1,1,0,-1,1,1,1,1,1,-1,1,-1,-1,1,1,1,-1,-1,1,1,1,0,1
bool(true)
bool(true)
bool(false)
bool(false)
version_compare(): Argument #3 ($operator) must be a valid comparison operator
"####;
    assert_eq!(compile_and_run(source), expected);
}

/// Code that references none of the prelude functions compiles and runs exactly as before.
#[test]
fn test_stdlib_prelude_is_pay_for_use_and_leaves_existing_builtins_alone() {
    let out = compile_and_run(
        r#"<?php
echo strpos("hello", "l"), substr("hello", 1, 3), strrev("abc"), str_pad("7", 3, "0", STR_PAD_LEFT), "\n";
echo implode(",", array_map(fn($x) => $x * 2, [1, 2, 3])), "|", ucfirst("xy"), "|", rawurlencode("a b"), "\n";
var_dump(function_exists('strlen'), function_exists('pack'));
"#,
    );
    assert_eq!(out, "2ellcba007\n2,4,6|Xy|a%20b\nbool(true)\nbool(true)\n");
}

/// A user-declared function with the same name wins; the prelude copy is not injected.
#[test]
fn test_stdlib_prelude_user_declaration_wins() {
    let out = compile_and_run(
        r#"<?php
function strrchr($h, $n) { return "mine"; }
echo strrchr("a/b", "/"), "\n";
"#,
    );
    assert_eq!(out, "mine\n");
}
