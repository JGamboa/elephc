//! Purpose:
//! End-to-end coverage for conditional declarations found on the Laravel frontier (front P45):
//! the `if (! function_exists('f')) { function f() {} }` polyfill idiom of every Laravel
//! `helpers.php`, `use` imports inside `yield`, and the `UnitEnum`/`BackedEnum` interfaces.
//!
//! Called from:
//! - The native codegen suite (`tests/codegen/mod.rs`).
//!
//! Key details:
//! - Each source mirrors a probe in `probes/cond-decl/` of the elephc-laravel workspace;
//!   expected output was produced by PHP 8.5 from the same source.

use crate::support::*;

/// Verifies a guarded user function is declared, and that `function_exists` follows program order.
#[test]
fn test_function_exists_guard_declares_user_function() {
    let out = compile_and_run(r#"<?php
var_dump(function_exists('g_late'));
if (!function_exists('g_late')) { function g_late() { return 'late'; } }
var_dump(function_exists('g_late'));
echo g_late(), "\n";
function probe() { return function_exists('g_late') ? 'y' : 'n'; }
echo probe(), "\n";
"#);
    assert_eq!(out, "bool(false)\nbool(true)\nlate\ny\n");
}

/// Verifies an existing builtin or unconditional user function keeps the branch from declaring.
#[test]
fn test_function_exists_guard_skips_existing_functions() {
    let out = compile_and_run(r#"<?php
if (!function_exists('strlen')) { echo "no\n"; }
function plain() { return 'plain'; }
if (!function_exists('plain')) { function plain2() { return 'x'; } }
var_dump(function_exists('plain2'));
echo plain(), "\n";
if (function_exists('nowhere')) { echo "no\n"; } else { echo "absent\n"; }
"#);
    assert_eq!(out, "bool(false)\nplain\nabsent\n");
}

/// Verifies the leading backslash, case-insensitive names and an invalid `\\` needle.
#[test]
fn test_function_exists_guard_name_normalization() {
    let out = compile_and_run(r#"<?php
if (!function_exists('\Mixed_Case')) { function mixed_case() { return 'mc'; } }
echo MIXED_CASE(), "\n";
var_dump(function_exists('\\\\mixed_case'), function_exists('\MIXED_case'));
"#);
    assert_eq!(out, "mc\nbool(false)\nbool(true)\n");
}

/// Verifies Laravel's helpers.php pattern across includes: first included declaration wins,
/// calls from earlier-declared functions resolve, and a namespaced guard name works.
#[test]
fn test_function_exists_guard_across_included_helpers() {
    let out = compile_and_run_files(
        &[
            (
                "main.php",
                r#"<?php
require __DIR__ . '/support/helpers.php';
require __DIR__ . '/foundation/helpers.php';
require __DIR__ . '/support/functions.php';
enum Suit: string { case Hearts = 'H'; }
enum Pure { case One; }
function use_helpers() { return tapx('a') . only_b(); }
echo use_helpers(), "\n";
echo Illuminate\Support\enum_valuex(Suit::Hearts), Illuminate\Support\enum_valuex(Pure::One), "\n";
var_dump(function_exists('\illuminate\support\ENUM_VALUEX'));
"#,
            ),
            (
                "support/helpers.php",
                "<?php\nif (! function_exists('tapx')) {\n    /** doc */\n    function tapx($v) { return $v; }\n}\n",
            ),
            (
                "foundation/helpers.php",
                "<?php\nif (! function_exists('tapx')) { function tapx($v) { return 'B'; } }\nif (! function_exists('only_b')) { function only_b() { return 'b'; } }\n",
            ),
            (
                "support/functions.php",
                r#"<?php
namespace Illuminate\Support;
use BackedEnum;
use UnitEnum;
if (! function_exists('Illuminate\Support\enum_valuex')) {
    function enum_valuex($value) {
        return match (true) {
            $value instanceof BackedEnum => $value->value,
            $value instanceof UnitEnum => $value->name,
            default => $value,
        };
    }
}
"#,
            ),
        ],
        "main.php",
    );
    assert_eq!(out, "ab\nHOne\nbool(true)\n");
}

/// Verifies a guard whose declaration is not a clean top-level polyfill stays a clear error.
#[test]
fn test_function_exists_guard_inside_loop_is_not_hoisted() {
    let err = compile_cli_file_with_flags_expect_failure(
        "<?php\nforeach ([1] as $_) { if (!function_exists('lp')) { function lp() { return 1; } } }\necho lp();\n",
        &[],
    );
    assert!(err.contains("Undefined function: lp"), "{err}");
}
