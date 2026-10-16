//! Purpose:
//! Compiles include/require sites whose path is only known at runtime, by closing them over the
//! application's `elephc.includes.toml` manifest; and reports every such site at once.
//!
//! Called from:
//! - `crate::magic_constants::substitute_file_and_scope_constants` (site marking, per file).
//! - `crate::resolver::engine` (lowering) and `crate::resolver::discovery::includes` (declarations).
//! - `crate::cli` / `crate::pipeline` for `--dynamic-includes=report` and `--include-manifest`.
//!
//! Key details:
//! - A site is identified as `<file relative to the app root>@<Class::method>#<ordinal>`.
//! - A site with `candidates` is lowered to: evaluate the path once, `realpath` it at runtime,
//!   compare it with the compile-time `canonicalize()` of every candidate, and run the matching
//!   candidate as an ordinary STATIC include of that path. Return values, `_once` labels, the
//!   caller's scope and declaration discovery therefore behave exactly as for a static include.
//! - A missing file keeps PHP's semantics (`include`: two warnings and `false`; `require`: an
//!   `Error`); an existing file outside the candidate set throws `\Error`; a `deny` site throws
//!   `\Error` with the manifest's reason. A site with no entry stays a compile error, unless
//!   `--dynamic-includes=report` asked for the whole inventory.

mod manifest;
pub(crate) mod mark;
pub(in crate::resolver) mod version_fold;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::errors::CompileError;
use crate::names::{Name, NameKind};
use crate::parser::ast::{BinOp, Expr, ExprKind, Stmt, StmtKind};
use crate::span::Span;

use super::include_path::fold_include_path;
use super::state::ResolveState;

use manifest::{Manifest, SiteAction};
pub(crate) use manifest::MANIFEST_FILE;

/// Name of the marker call that carries a site's identity around its original path.
pub(crate) const MARKER: &str = "__elephc_include_site";

/// `include_path` reported by the missing-file diagnostics (php-cli's compiled default here).
const INCLUDE_PATH: &str = ".:";

static REPORT_MODE: AtomicBool = AtomicBool::new(false);
static MANIFEST_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);
static APP_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);
static REPORT: Mutex<BTreeMap<String, ReportEntry>> = Mutex::new(BTreeMap::new());
static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);
static SKIPPED_FILES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// In report mode, the autoloader skips a class file that fails to compile instead of stopping,
/// so the inventory still covers the rest of the program; the failure is listed in the report.
pub fn record_skipped_file(file: &Path, error: &CompileError) {
    SKIPPED_FILES.lock().unwrap().push(format!(
        "{}: {}",
        error.file.clone().unwrap_or_else(|| file.display().to_string()),
        error.message.lines().next().unwrap_or_default()
    ));
}

/// Turns on `--dynamic-includes=report`: every non-foldable site is recorded instead of failing.
pub fn set_report_mode(enabled: bool) {
    REPORT_MODE.store(enabled, Ordering::Relaxed);
}

/// Whether `--dynamic-includes=report` is active.
pub fn report_mode() -> bool {
    REPORT_MODE.load(Ordering::Relaxed)
}

/// `--include-manifest <path>`: use this manifest for every site.
pub fn set_manifest_override(path: Option<PathBuf>) {
    *MANIFEST_OVERRIDE.lock().unwrap() = path;
}

/// The entry file's directory, used to name sites when no manifest exists yet.
pub fn set_app_root(root: &Path) {
    let root = if root.as_os_str().is_empty() { Path::new(".") } else { root };
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    *APP_ROOT.lock().unwrap() = Some(canonical);
}

/// One line of the `--dynamic-includes=report` inventory.
#[derive(Clone)]
struct ReportEntry {
    file: PathBuf,
    line: i64,
    md5: String,
    status: String,
    expression: String,
}

/// Renders the inventory collected so far (empty string when nothing was recorded).
pub fn render_report() -> String {
    let report = REPORT.lock().unwrap();
    let mut out = String::new();
    out.push_str(&format!(
        "dynamic include sites: {} (non-foldable include/require paths)\n",
        report.len()
    ));
    for (index, (id, entry)) in report.iter().enumerate() {
        out.push_str(&format!(
            "{:>3}. {}\n     at {}:{}\n     expr: {}\n     md5: {}\n     status: {}\n",
            index + 1,
            id,
            entry.file.display(),
            entry.line,
            entry.expression,
            entry.md5,
            entry.status
        ));
    }
    let skipped = SKIPPED_FILES.lock().unwrap();
    if !skipped.is_empty() {
        out.push_str(&format!(
            "\n# {} autoloaded file(s) failed to compile and were skipped (their sites, if any, are missing above):\n",
            skipped.len()
        ));
        for line in skipped.iter() {
            out.push_str(&format!("#   {}\n", line));
        }
    }
    let unlisted: Vec<_> = report
        .iter()
        .filter(|(_, entry)| entry.status.starts_with("unlisted"))
        .collect();
    if !unlisted.is_empty() {
        out.push_str(&format!(
            "\n# {} site(s) without a manifest entry; stub for {}:\n",
            unlisted.len(),
            MANIFEST_FILE
        ));
        for (id, entry) in unlisted {
            out.push_str(&format!(
                "\n[[site]]\nid = '{}'\nmd5 = '{}'\n# expr: {}\ndeny = \"TODO: review\"\n",
                id, entry.md5, entry.expression
            ));
        }
    }
    out
}

/// The identity a marker carries.
pub(in crate::resolver) struct Site<'a> {
    file: &'a str,
    scope: &'a str,
    ordinal: i64,
    line: i64,
    path: &'a Expr,
}

/// Decodes a site marker, if `expr` is one.
pub(in crate::resolver) fn site_of(expr: &Expr) -> Option<Site<'_>> {
    let ExprKind::FunctionCall { name, args } = &expr.kind else {
        return None;
    };
    if name.parts.len() != 1 || name.parts[0] != MARKER || args.len() != 5 {
        return None;
    }
    let (ExprKind::StringLiteral(file), ExprKind::StringLiteral(scope), ExprKind::IntLiteral(ordinal), ExprKind::IntLiteral(line)) =
        (&args[0].kind, &args[1].kind, &args[2].kind, &args[3].kind)
    else {
        return None;
    };
    Some(Site {
        file,
        scope,
        ordinal: *ordinal,
        line: *line,
        path: &args[4],
    })
}

/// The original path under a site marker (or `expr` itself).
pub(crate) fn unwrap_site(expr: &Expr) -> &Expr {
    site_of(expr).map_or(expr, |site| site.path)
}

/// Where the value of a lowered include goes.
pub(in crate::resolver) enum SiteCapture {
    /// Statement position: the value is discarded.
    Stmt,
    /// `$name = include ...;`
    Assign(String),
    /// `return include ...;`
    Return,
}

/// A site resolved against its manifest.
struct ResolvedSite {
    id: String,
    action: Option<SiteAction>,
    root: Option<PathBuf>,
    manifest: Option<Manifest>,
}

/// Finds the manifest entry of a site, checking its md5.
fn resolve_site(site: &Site<'_>, span: Span) -> Result<(ResolvedSite, String), CompileError> {
    let file = PathBuf::from(site.file);
    let error = |message: String| CompileError::new(span, &message).with_file(site.file.to_string());
    let override_path = MANIFEST_OVERRIDE.lock().unwrap().clone();
    let manifest = match manifest::locate(&file, override_path.as_deref()) {
        Some(path) => Some(Manifest::load(&path).map_err(&error)?),
        None => None,
    };
    let root = manifest
        .as_ref()
        .map(|m| m.root.clone())
        .or_else(|| APP_ROOT.lock().unwrap().clone());
    let relative = root
        .as_ref()
        .and_then(|root| file.strip_prefix(root).ok())
        .map(|rel| {
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|| site.file.to_string());
    let id = format!("{}@{}#{}", relative, site.scope, site.ordinal);
    let md5 = manifest::file_md5(&file).map_err(&error)?;
    let entry = manifest.as_ref().and_then(|m| m.sites.get(&id).cloned());
    let mut status = match &entry {
        None => match &manifest {
            Some(m) => format!("unlisted (no entry in {})", m.path.display()),
            None => "unlisted (no elephc.includes.toml found)".to_string(),
        },
        Some(entry) => match &entry.action {
            SiteAction::Candidates(globs) => format!("candidates {:?}", globs),
            SiteAction::Deny(reason) => format!("deny: {}", reason),
        },
    };
    let mut action = entry.as_ref().map(|e| e.action.clone());
    if let Some(entry) = &entry {
        if entry.md5 != md5 {
            let message = format!(
                "dynamic include site `{}`: '{}' has md5 {} but {} pins {}; the file changed \
                 since this site was reviewed. Re-run `elephc --dynamic-includes=report`, re-check \
                 the site and update its `md5`",
                id,
                site.file,
                md5,
                manifest.as_ref().map(|m| m.path.display().to_string()).unwrap_or_default(),
                entry.md5
            );
            if !report_mode() {
                return Err(error(message));
            }
            status = format!("STALE md5 (manifest pins {})", entry.md5);
            action = None;
        }
    }
    Ok((
        ResolvedSite {
            id,
            action,
            root,
            manifest,
        },
        format!("{}\u{0}{}", md5, status),
    ))
}

/// Records a site in the report inventory.
fn record(site: &Site<'_>, id: &str, md5_and_status: &str) {
    let (md5, status) = md5_and_status.split_once('\u{0}').unwrap_or((md5_and_status, ""));
    let expression = source_snippet(Path::new(site.file), site.line);
    REPORT.lock().unwrap().insert(
        id.to_string(),
        ReportEntry {
            file: PathBuf::from(site.file),
            line: site.line,
            md5: md5.to_string(),
            status: status.to_string(),
            expression,
        },
    );
}

/// The include expression's source text, from its line to the first `;` (whitespace collapsed).
fn source_snippet(file: &Path, line: i64) -> String {
    let Ok(text) = std::fs::read_to_string(file) else {
        return String::new();
    };
    let start = usize::try_from(line.max(1) - 1).unwrap_or(0);
    let mut snippet = String::new();
    for source_line in text.lines().skip(start).take(8) {
        snippet.push_str(source_line.trim());
        snippet.push(' ');
        if source_line.contains(';') {
            break;
        }
    }
    let collapsed = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = collapsed.find(';').map_or(collapsed.as_str(), |end| &collapsed[..=end]);
    cut.chars().take(200).collect()
}

/// Lowers a dynamic include site through its manifest.
///
/// Returns `Ok(None)` when `path` is not a marked site or still folds to a static path (the
/// caller then takes the static route, which also produces today's diagnostic for an unmarked
/// dynamic path). For a site with `candidates`, the returned statements contain only STATIC
/// includes, so the caller resolves them with the ordinary machinery.
pub(in crate::resolver) fn lower_dynamic_include(
    span: Span,
    path: &Expr,
    once: bool,
    required: bool,
    capture: SiteCapture,
    state: &ResolveState,
) -> Result<Option<Vec<Stmt>>, CompileError> {
    let Some(site) = site_of(path) else {
        return Ok(None);
    };
    let fold_error = match fold_include_path(path, state) {
        Ok(_) => return Ok(None),
        Err(message) => message,
    };
    let (resolved, md5_and_status) = resolve_site(&site, span)?;
    if report_mode() {
        record(&site, &resolved.id, &md5_and_status);
    }
    let keyword = include_keyword(once, required);
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path_var = format!("__elephc_dyn_{n}");
    let real_var = format!("{path_var}_rp");
    let var = |name: &str| Expr::new(ExprKind::Variable(name.to_string()), span);
    let lit = |value: &str| Expr::new(ExprKind::StringLiteral(value.to_string()), span);
    let mut out = vec![assign(&path_var, site.path.clone(), span)];

    let globs = match resolved.action {
        None if !report_mode() => {
            let where_ = match &resolved.manifest {
                Some(m) => format!("has no entry in {}", m.path.display()),
                None => format!("has no {} (searched upward from its file)", MANIFEST_FILE),
            };
            return Err(CompileError::new(
                span,
                &format!(
                    "{}\n  dynamic include site `{}` {}; declare it with `candidates = [...]` \
                     or `deny = \"reason\"` (list every site with `elephc --dynamic-includes=report`)",
                    fold_error, resolved.id, where_
                ),
            )
            .with_file(site.file.to_string()));
        }
        None => {
            out.push(throw_error(lit(&unlisted_message(keyword, &resolved.id)), span));
            return Ok(Some(out));
        }
        Some(SiteAction::Deny(reason)) => {
            out.push(throw_error(lit(&deny_message(keyword, &resolved.id, &reason)), span));
            return Ok(Some(out));
        }
        Some(SiteAction::Candidates(globs)) => globs,
    };

    let manifest = resolved.manifest.as_ref().expect("a manifest entry implies a manifest");
    let mut candidates: Vec<PathBuf> = Vec::new();
    for glob in &globs {
        let matched = manifest
            .expand_glob(glob)
            .map_err(|m| CompileError::new(span, &m).with_file(site.file.to_string()))?;
        if matched.is_empty() {
            return Err(CompileError::new(
                span,
                &format!(
                    "dynamic include site `{}`: candidate '{}' matches no file under {}",
                    resolved.id,
                    glob,
                    resolved.root.as_ref().map(|r| r.display().to_string()).unwrap_or_default()
                ),
            )
            .with_file(site.file.to_string()));
        }
        candidates.extend(matched);
    }
    candidates.sort();
    candidates.dedup();

    let realpath = |arg: Expr| {
        Expr::new(
            ExprKind::FunctionCall {
                name: Name::from_parts(NameKind::FullyQualified, vec!["realpath".to_string()]),
                args: vec![arg],
            },
            span,
        )
    };
    let is_false = |name: &str| strict_eq(var(name), Expr::new(ExprKind::BoolLiteral(false), span), span);
    out.push(assign(&real_var, realpath(var(&path_var)), span));
    // A relative path PHP did not find through `include_path` ('.', the cwd) is looked up next to
    // the including script.
    let site_dir = Path::new(site.file)
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    out.push(Stmt::new(
        StmtKind::If {
            condition: is_false(&real_var),
            then_body: vec![assign(
                &real_var,
                realpath(concat(vec![lit(&format!("{}/", site_dir)), var(&path_var)])),
                span,
            )],
            elseif_clauses: Vec::new(),
            else_body: None,
        },
        span,
    ));

    let mut clauses: Vec<(Expr, Vec<Stmt>)> = candidates
        .iter()
        .map(|candidate| {
            let candidate = candidate.display().to_string();
            (
                strict_eq(var(&real_var), lit(&candidate), span),
                candidate_body(&candidate, once, required, &capture, span),
            )
        })
        .collect();
    clauses.push((is_false(&real_var), missing_body(&path_var, keyword, required, &capture, span)));
    let else_body = vec![throw_error(
        concat(vec![
            lit(&format!("elephc: dynamic {} of '", keyword)),
            var(&real_var),
            lit(&format!(
                "' at {} is not one of the {} candidate(s) listed in {}",
                resolved.id,
                candidates.len(),
                MANIFEST_FILE
            )),
        ]),
        span,
    )];
    let (condition, then_body) = clauses.remove(0);
    out.push(Stmt::new(
        StmtKind::If {
            condition,
            then_body,
            elseif_clauses: clauses,
            else_body: Some(else_body),
        },
        span,
    ));
    Ok(Some(out))
}

/// Runtime message of a site the manifest denies.
fn deny_message(keyword: &str, id: &str, reason: &str) -> String {
    format!("elephc: dynamic {} at {} is denied by {}: {}", keyword, id, MANIFEST_FILE, reason)
}

/// Runtime message of a site compiled without a valid entry under `--dynamic-includes=report`.
fn unlisted_message(keyword: &str, id: &str) -> String {
    format!(
        "elephc: dynamic {} at {} has no valid entry in {} (compiled with --dynamic-includes=report)",
        keyword, id, MANIFEST_FILE
    )
}

/// For an EXPRESSION-position include whose site the manifest denies (or that has no valid
/// entry under `--dynamic-includes=report`), the `throw` expression that replaces it in place.
///
/// Done while marking, before the resolver's value-include hoisting, because a denied site has
/// nothing to inline: hoisting it would only hit the hoister's restrictions (a by-reference
/// argument before the include, a `static` target) for code that can never run. The path is
/// not evaluated. Anything this cannot decide here (no manifest, unreadable manifest, stale
/// md5 outside report mode, a path that still folds) is left to `lower_dynamic_include`, which
/// reports it.
pub(super) fn denied_value_replacement(site: &Site<'_>, once: bool, required: bool, span: Span) -> Option<Expr> {
    if fold_include_path(site.path, &ResolveState::default()).is_ok() {
        return None;
    }
    let (resolved, md5_and_status) = resolve_site(site, span).ok()?;
    let keyword = include_keyword(once, required);
    let message = match resolved.action {
        Some(SiteAction::Deny(reason)) => deny_message(keyword, &resolved.id, &reason),
        None if report_mode() => unlisted_message(keyword, &resolved.id),
        _ => return None,
    };
    if report_mode() {
        record(site, &resolved.id, &md5_and_status);
    }
    Some(Expr::new(
        ExprKind::Throw(Box::new(Expr::new(
            ExprKind::NewObject {
                class_name: Name::from_parts(NameKind::FullyQualified, vec!["Error".to_string()]),
                args: vec![Expr::new(ExprKind::StringLiteral(message), span)],
            },
            span,
        ))),
        span,
    ))
}

/// The PHP keyword spelling of an include form.
fn include_keyword(once: bool, required: bool) -> &'static str {
    match (required, once) {
        (false, false) => "include",
        (false, true) => "include_once",
        (true, false) => "require",
        (true, true) => "require_once",
    }
}

/// The static include of one candidate, delivering its value like the original site.
fn candidate_body(candidate: &str, once: bool, required: bool, capture: &SiteCapture, span: Span) -> Vec<Stmt> {
    let path = Expr::new(ExprKind::StringLiteral(candidate.to_string()), span);
    let value = || {
        Expr::new(
            ExprKind::IncludeValue {
                path: Box::new(path.clone()),
                once,
                required,
            },
            span,
        )
    };
    let stmt = match capture {
        SiteCapture::Stmt => StmtKind::Include {
            path: path.clone(),
            once,
            required,
        },
        SiteCapture::Assign(name) => StmtKind::Assign {
            name: name.clone(),
            value: value(),
        },
        SiteCapture::Return => StmtKind::Return(Some(value())),
    };
    vec![Stmt::new(stmt, span)]
}

/// PHP's behaviour for a path that names no file.
fn missing_body(path_var: &str, keyword: &str, required: bool, capture: &SiteCapture, span: Span) -> Vec<Stmt> {
    let var = || Expr::new(ExprKind::Variable(path_var.to_string()), span);
    let lit = |value: &str| Expr::new(ExprKind::StringLiteral(value.to_string()), span);
    let warning = |message: Expr| {
        Stmt::new(
            StmtKind::ExprStmt(Expr::new(
                ExprKind::FunctionCall {
                    name: Name::from_parts(NameKind::FullyQualified, vec!["trigger_error".to_string()]),
                    args: vec![message, Expr::new(ExprKind::ConstRef(Name::from_parts(NameKind::FullyQualified, vec!["E_USER_WARNING".to_string()])), span)],
                },
                span,
            )),
            span,
        )
    };
    let mut body = vec![warning(concat(vec![
        lit(&format!("{}(", keyword)),
        var(),
        lit("): Failed to open stream: No such file or directory"),
    ]))];
    if required {
        body.push(throw_error(
            concat(vec![
                lit("Failed opening required '"),
                var(),
                lit(&format!("' (include_path='{}')", INCLUDE_PATH)),
            ]),
            span,
        ));
        return body;
    }
    body.push(warning(concat(vec![
        lit(&format!("{}(): Failed opening '", keyword)),
        var(),
        lit(&format!("' for inclusion (include_path='{}')", INCLUDE_PATH)),
    ])));
    let falsy = Expr::new(ExprKind::BoolLiteral(false), span);
    match capture {
        SiteCapture::Stmt => {}
        SiteCapture::Assign(name) => body.push(assign(name, falsy, span)),
        SiteCapture::Return => body.push(Stmt::new(StmtKind::Return(Some(falsy)), span)),
    }
    body
}

fn assign(name: &str, value: Expr, span: Span) -> Stmt {
    Stmt::new(
        StmtKind::Assign {
            name: name.to_string(),
            value,
        },
        span,
    )
}

fn strict_eq(left: Expr, right: Expr, span: Span) -> Expr {
    Expr::new(
        ExprKind::BinaryOp {
            left: Box::new(left),
            op: BinOp::StrictEq,
            right: Box::new(right),
        },
        span,
    )
}

/// Left-associated concatenation of `parts`.
fn concat(mut parts: Vec<Expr>) -> Expr {
    let first = parts.remove(0);
    parts.into_iter().fold(first, |left, right| {
        let span = left.span;
        Expr::new(
            ExprKind::BinaryOp {
                left: Box::new(left),
                op: BinOp::Concat,
                right: Box::new(right),
            },
            span,
        )
    })
}

/// `throw new \Error(<message>);`
fn throw_error(message: Expr, span: Span) -> Stmt {
    Stmt::new(
        StmtKind::Throw(Expr::new(
            ExprKind::NewObject {
                class_name: Name::from_parts(NameKind::FullyQualified, vec!["Error".to_string()]),
                args: vec![message],
            },
            span,
        )),
        span,
    )
}
