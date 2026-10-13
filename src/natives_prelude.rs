//! Purpose:
//! Declares PHP core classes that elephc implements as ordinary elephc-PHP: `WeakMap` and
//! `ErrorException`. Each is injected only when the program can reach it.
//!
//! Called from:
//! - `crate::pipeline::compile()` and the codegen/error test harnesses via `inject_if_used`,
//!   AFTER `autoload::run` (next to `object_cast_prelude`), so a reference that lives only in an
//!   autoloaded class file is detected.
//!
//! Key details:
//! - The declarations are PARSED from the PHP form below (`parser::parse_internal`, cached like
//!   the PDO prelude's extern block) and name-resolved on the way in, because injection runs past
//!   the pipeline's own name-resolution pass. They therefore enter the checker and lowering as
//!   exactly the nodes the parser produces for that PHP.
//! - `WeakMap` HOLDS ITS KEYS STRONGLY. elephc has no weak references, so an entry is not dropped
//!   when its key object becomes unreachable elsewhere: `count()`, iteration and the key's
//!   `__destruct` timing differ from PHP for a program that DEPENDS on collection (drops every
//!   other reference to a key and then observes the map). Every program that does not rely on
//!   collection behaves as in PHP: `offsetGet/offsetSet/offsetExists/offsetUnset`, `count()`,
//!   insertion-ordered iteration with the key objects as keys, the `TypeError` for a non-object
//!   key, the `Error` for a missing key and for `$map[] = ...`. Keys are identified by
//!   `spl_object_id`, which cannot be reused while the map keeps the object alive.
//! - `ErrorException` supports `severity` / `getSeverity()`. Its `$filename` and `$line`
//!   parameters accept only `null`: elephc's `getFile()` / `getLine()` are compiler intrinsics
//!   that read a per-module path and the `new` line, so honouring a caller-supplied file or line
//!   needs a backend change. The parameters are typed with an internal marker interface no value
//!   can satisfy, which turns a non-null argument into a compile-time type error naming
//!   `__ElephcUnsupportedErrorExceptionFileOrLine` instead of a silently wrong `getLine()`.
//! - A user declaration of either global class name is rejected, as PHP rejects it.

use std::sync::OnceLock;

use crate::errors::CompileError;
use crate::names::php_symbol_key;
use crate::parser::ast::{Program, Stmt, StmtKind};

/// The PHP form of the `WeakMap` declaration.
const WEAKMAP_SRC: &str = r##"<?php
final class WeakMap implements ArrayAccess, Countable, IteratorAggregate
{
    private array $__elephc_keys = [];
    private array $__elephc_values = [];

    private static function __elephc_id(mixed $object): int
    {
        if (!is_object($object)) {
            throw new TypeError("WeakMap key must be an object");
        }
        $__elephc_ids = array_map(fn (object $__elephc_o): int => spl_object_id($__elephc_o), [$object]);
        return $__elephc_ids[0];
    }

    public function offsetGet(mixed $object): mixed
    {
        $__elephc_id = self::__elephc_id($object);
        if (!array_key_exists($__elephc_id, $this->__elephc_keys)) {
            throw new Error("Object " . get_class($object) . "#" . $__elephc_id . " not contained in WeakMap");
        }
        return $this->__elephc_values[$__elephc_id];
    }

    public function offsetSet(mixed $object, mixed $value): void
    {
        if ($object === null) {
            throw new Error("Cannot append to WeakMap");
        }
        $__elephc_id = self::__elephc_id($object);
        $this->__elephc_keys[$__elephc_id] = $object;
        $this->__elephc_values[$__elephc_id] = $value;
    }

    public function offsetExists(mixed $object): bool
    {
        $__elephc_id = self::__elephc_id($object);
        return isset($this->__elephc_values[$__elephc_id]);
    }

    public function offsetUnset(mixed $object): void
    {
        $__elephc_id = self::__elephc_id($object);
        $__elephc_keys = $this->__elephc_keys;
        $__elephc_values = $this->__elephc_values;
        unset($__elephc_keys[$__elephc_id]);
        unset($__elephc_values[$__elephc_id]);
        $this->__elephc_keys = $__elephc_keys;
        $this->__elephc_values = $__elephc_values;
    }

    public function count(): int
    {
        return count($this->__elephc_keys);
    }

    public function getIterator(): Iterator
    {
        foreach ($this->__elephc_keys as $__elephc_id => $__elephc_key) {
            yield $__elephc_key => $this->__elephc_values[$__elephc_id];
        }
    }
}
"##;

/// The PHP form of the `ErrorException` declaration and its file/line marker.
const ERROR_EXCEPTION_SRC: &str = r##"<?php
interface __ElephcUnsupportedErrorExceptionFileOrLine
{
}

class ErrorException extends Exception
{
    // `1` is E_ERROR: a constant-expression default on a Throwable subclass property is not
    // lowerable by the throwable `new` path today.
    protected int $severity = 1;

    public function __construct(
        string $message = "",
        int $code = 0,
        int $severity = E_ERROR,
        ?__ElephcUnsupportedErrorExceptionFileOrLine $filename = null,
        ?__ElephcUnsupportedErrorExceptionFileOrLine $line = null,
        ?Throwable $previous = null
    ) {
        parent::__construct($message, $code, $previous);
        $this->severity = $severity;
        $_elephc_unused_filename = $filename;
        $_elephc_unused_line = $line;
    }

    final public function getSeverity(): int
    {
        return $this->severity;
    }
}
"##;

/// One injectable class: its PHP name, its PHP form and its parsed-template cache.
struct NativeClass {
    name: &'static str,
    source: &'static str,
    parsed: &'static OnceLock<Program>,
}

static WEAKMAP_PARSED: OnceLock<Program> = OnceLock::new();
static ERROR_EXCEPTION_PARSED: OnceLock<Program> = OnceLock::new();

/// Every class this prelude can inject, in injection order.
const NATIVE_CLASSES: &[NativeClass] = &[
    NativeClass { name: "WeakMap", source: WEAKMAP_SRC, parsed: &WEAKMAP_PARSED },
    NativeClass {
        name: "ErrorException",
        source: ERROR_EXCEPTION_SRC,
        parsed: &ERROR_EXCEPTION_PARSED,
    },
];

/// Returns an independent copy of one class's parsed declarations.
///
/// Later passes mutate injected ASTs, so the cache keeps an immutable template and clones it.
fn parsed(class: &NativeClass) -> Program {
    class
        .parsed
        .get_or_init(|| {
            let tokens = crate::lexer::tokenize(class.source)
                .unwrap_or_else(|e| panic!("{} prelude must tokenize: {e:?}", class.name));
            crate::parser::parse_internal(&tokens)
                .unwrap_or_else(|e| panic!("{} prelude must parse: {e:?}", class.name))
        })
        .clone()
}

/// Returns the names of the classes this prelude would inject into `program`.
///
/// A class is wanted when the program names it in a class position or a string literal
/// (`class_exists('WeakMap')`), or when it enumerates or constructs classes dynamically
/// (`get_declared_classes()`, `new $c`), the same over-approximation the SPL gate uses.
pub fn wanted_classes(program: &[Stmt]) -> Vec<&'static str> {
    let usage = crate::prelude_prune::usage::collect(program);
    let dynamic = usage.introspects || usage.constructs_dynamic_class;
    NATIVE_CLASSES
        .iter()
        .filter(|class| {
            let key = php_symbol_key(class.name);
            dynamic || usage.classes.contains(&key) || usage.literals.contains(&key)
        })
        .map(|class| class.name)
        .collect()
}

/// Returns the span of a user declaration of a global class-like named `name`, if any.
fn user_declaration(program: &[Stmt], name: &str) -> Option<crate::span::Span> {
    let key = php_symbol_key(name);
    program.iter().find_map(|stmt| match &stmt.kind {
        StmtKind::ClassDecl { name, .. }
        | StmtKind::InterfaceDecl { name, .. }
        | StmtKind::TraitDecl { name, .. }
        | StmtKind::EnumDecl { name, .. }
            if php_symbol_key(name.trim_start_matches('\\')) == key =>
        {
            Some(stmt.span)
        }
        StmtKind::NamespaceBlock { body, .. }
        | StmtKind::IncludeOnceGuard { body, .. }
        | StmtKind::Synthetic(body) => user_declaration(body, name),
        _ => None,
    })
}

/// Prepends the wanted native classes, or returns the program unchanged when none is reachable.
///
/// Runs under `crate::compiler_stack::with_compiler_stack`, like the other usage-scanning
/// preludes, so an embedder gets the same stack budget as the CLI driver.
pub fn inject_if_used(
    program: Program,
    inventory: &mut crate::optimize::reachability::PreludeInventory,
) -> Result<Program, CompileError> {
    crate::compiler_stack::with_compiler_stack(|| inject_on_compiler_stack(program, inventory))
}

/// The body of [`inject_if_used`], run on the stack `with_compiler_stack` sized.
fn inject_on_compiler_stack(
    program: Program,
    inventory: &mut crate::optimize::reachability::PreludeInventory,
) -> Result<Program, CompileError> {
    for class in NATIVE_CLASSES {
        if let Some(span) = user_declaration(&program, class.name) {
            return Err(CompileError::new(
                span,
                &format!(
                    "Cannot declare class {}, because the name is already in use",
                    class.name
                ),
            ));
        }
    }
    let wanted = wanted_classes(&program);
    if wanted.is_empty() {
        return Ok(program);
    }
    let mut declarations = Vec::new();
    for class in NATIVE_CLASSES.iter().filter(|class| wanted.contains(&class.name)) {
        declarations.extend(parsed(class));
    }
    let mut combined = crate::name_resolver::resolve(declarations)?;
    inventory.record_program("natives", &combined);
    combined.extend(program);
    Ok(combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a fixture the way the pipeline does, so the detector sees a real AST.
    fn program(source: &str) -> Program {
        let tokens = crate::lexer::tokenize(source).expect("tokenize");
        crate::parser::parse(&tokens).expect("parse")
    }

    /// Both PHP forms parse; a broken one would only surface at a user's first reference.
    #[test]
    fn every_native_class_parses() {
        for class in NATIVE_CLASSES {
            assert!(!parsed(class).is_empty(), "{}", class.name);
        }
    }

    /// The prelude is pay-for-use: an unrelated program is returned untouched.
    #[test]
    fn unrelated_programs_are_not_injected() {
        let mut inventory = crate::optimize::reachability::PreludeInventory::new();
        let source = program("<?php echo strlen('x');");
        let injected = inject_if_used(source.clone(), &mut inventory).expect("inject");
        assert_eq!(injected.len(), source.len());
    }

    /// Each class is injected only when named.
    #[test]
    fn classes_are_wanted_by_name() {
        assert_eq!(wanted_classes(&program("<?php $m = new \\WeakMap();")), vec!["WeakMap"]);
        assert_eq!(
            wanted_classes(&program("<?php function f(ErrorException $e) {}")),
            vec!["ErrorException"]
        );
    }

    /// A user's own global `WeakMap` is a redeclaration, as in PHP.
    #[test]
    fn user_redeclaration_is_rejected() {
        let mut inventory = crate::optimize::reachability::PreludeInventory::new();
        let error = inject_if_used(program("<?php class WeakMap {}"), &mut inventory)
            .expect_err("redeclaration must fail");
        assert!(error.message.contains("Cannot declare class WeakMap"));
    }
}
