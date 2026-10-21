//! Declare captured upvalues that no scope ever declares.
//!
//! ── THE BUG ─────────────────────────────────────────────────────────────
//! Measured on a 1,273-file corpus: **556 files, 2,118 occurrences** — 98.8%
//! of every provably-wrong defect found. One root cause, hit repeatedly.
//!
//! A closure that captures a parent local gets the reference but never the
//! declaration. From `PopperCam.lua`:
//!
//! ```luau
//! local function OnCameraSubjectChanged()
//!     v13 = true                      -- upvalue, never declared anywhere
//!     v8  = CameraSubject.Torso       -- upvalue, never declared anywhere
//! end
//! local function OnWorkspaceChanged2(arg1)
//!     CurrentCamera3 = CurrentCamera2 -- upvalue, never declared anywhere
//!     v7:disconnect()                 -- upvalue, never declared anywhere
//! end
//! ```
//!
//! The original had module-level `local` declarations that these closures
//! captured. Lifting kept the *uses* and lost the *declarations*, so the
//! output assigns to globals instead of the intended locals. It still parses,
//! which is exactly why marker-counting scored these files clean.
//!
//! Note this is a different problem from working out what an upvalue should be
//! *called*. A perfect name is still broken output if nothing declares it.
//! (An unwired `capture_chain.rs` used to attempt the naming half; it was never
//! declared as a module, and wiring it in was measured byte-identical on 400
//! corpus files, so it was deleted rather than finished.)
//!
//! ── THE FIX ─────────────────────────────────────────────────────────────
//! After lifting, walk the chunk and collect every name that is **assigned**
//! but never **bound**. Emit `local <names>` at the top of the chunk, which is
//! where the original module-level locals lived.
//!
//! ── WHY THIS IS SOUND ───────────────────────────────────────────────────
//! The pass only declares a name when ALL of these hold:
//!
//!   1. it is an assignment target somewhere in the chunk — a name that is
//!      only ever *read* is far more likely a global we do not know about,
//!      and declaring it would shadow that global to nil, which is a
//!      regression rather than a fix;
//!   2. no scope binds it — not a local, parameter, loop variable, or
//!      function name anywhere in the chunk;
//!   3. it is not a known Luau or Roblox global.
//!
//! Together these mean: assigned, undeclared, and not a global. In a
//! decompilation that is an upvalue whose declaration was lost. The one
//! remaining possibility is a script deliberately writing a new global, which
//! condition 3's list covers for the realistic cases and which is rare enough
//! in module code to be the right trade.
//!
//! Declaring at chunk top rather than at first use is deliberate: a capture
//! may be written in one closure and read in another that runs earlier, so
//! only the outermost scope is guaranteed to dominate every use.

use crate::ast::{Expr, Stat, TableField};
use std::collections::BTreeSet;

/// Globals that must never be shadowed by a generated declaration.
/// Shadowing one of these to `nil` turns working output into broken output.
const KNOWN_GLOBALS: &[&str] = &[
    // Luau base library
    "assert", "error", "getfenv", "getmetatable", "ipairs", "next", "pairs",
    "pcall", "print", "rawequal", "rawget", "rawlen", "rawset", "require",
    "select", "setfenv", "setmetatable", "tonumber", "tostring", "type",
    "typeof", "unpack", "xpcall", "newproxy", "loadstring", "collectgarbage",
    "coroutine", "debug", "math", "os", "string", "table", "utf8", "bit32",
    "buffer", "task", "vector", "_G", "_VERSION", "shared",
    // Roblox globals and common types
    "game", "workspace", "script", "Enum", "Instance", "Vector2", "Vector3",
    "CFrame", "Color3", "BrickColor", "UDim", "UDim2", "Ray", "Rect",
    "Region3", "TweenInfo", "NumberRange", "NumberSequence", "ColorSequence",
    "PhysicalProperties", "Random", "Faces", "Axes", "DateTime", "Font",
    "OverlapParams", "RaycastParams", "PathWaypoint", "delay", "spawn",
    "wait", "tick", "time", "elapsedTime", "settings", "UserSettings",
    "PluginManager", "DebuggerManager", "stats", "version", "warn",
];

fn is_known_global(name: &str) -> bool {
    KNOWN_GLOBALS.contains(&name)
}

/// Is this a name the decompiler invented, rather than one recovered from
/// debug info or the constant table?
///
/// Matches `v0`, `v30`, `upval_3`, `cap_1`, `arg_2` — shapes no real Luau or
/// Roblox global ever has. That matters: a read-only name of this shape is
/// provably ours, so declaring it cannot shadow anything that exists.
fn is_generated_name(name: &str) -> bool {
    for prefix in ["upval_", "cap_", "arg_", "field_"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
        }
    }
    // `v12` but not `v`, `vec`, `value`
    if let Some(rest) = name.strip_prefix('v') {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    false
}

/// Names bound, names assigned, and names merely read.
#[derive(Default)]
struct Scan {
    bound: BTreeSet<String>,
    assigned: BTreeSet<String>,
    /// Read but never written. Only actionable for generated names — see
    /// [`is_generated_name`].
    read: BTreeSet<String>,
    /// Read at least once OUTSIDE any closure body.
    ///
    /// This is the discriminant that separates a genuine captured upvalue from
    /// a value the lifter dropped. A capture, by definition, is a reference
    /// from inside a nested function to a name in an enclosing scope — so a
    /// real capture is read INSIDE a closure. A name read at chunk top level
    /// that nothing ever assigns is not a capture; it is a hole.
    ///
    /// Declaring a hole is worse than leaving it undeclared, because it makes
    /// the output parse while every use evaluates to nil. See the admission
    /// rules in [`declare_free_vars`].
    read_outside_closure: BTreeSet<String>,
    /// Closure nesting depth during the walk. 0 == chunk top level.
    closure_depth: usize,
    /// When set, `Expr::Function` bodies are not entered. Used to collect the
    /// names bound and assigned by ONE function without its nested closures.
    skip_closures: bool,
}

/// Insert declarations for captured upvalues that nothing declares.
/// Returns the names declared, for diagnostics.
pub fn declare_free_vars(body: &mut Vec<Stat>) -> Vec<String> {
    declare_free_vars_mode(body, false)
}

/// `orphan = true` is used for protos that have **no parent binding at all**
/// (the detached bodies a chunk can report). There is nothing to capture and
/// nothing to shadow: the proto is never called, and every free read in it is a
/// name we invented. Declaring all of them keeps such a body self-consistent
/// instead of leaving `lib`/`script_2`/`upval_N` reads pointing at nil globals.
pub fn declare_free_vars_orphan(body: &mut Vec<Stat>) -> Vec<String> {
    declare_free_vars_mode(body, true)
}

fn declare_free_vars_mode(body: &mut Vec<Stat>, orphan: bool) -> Vec<String> {
    let mut scan = Scan::default();
    scan_stats(body, &mut scan);

    // Two sources, with deliberately different admission rules.
    //
    //   assigned  — any name written but never bound. Writing to an
    //               undeclared name is an upvalue whose declaration was lost.
    //
    //   read-only — ONLY names the decompiler invented (`v30`, `upval_3`).
    //               An upvalue a closure merely reads is still an upvalue,
    //               and this is the majority case: the first version of this
    //               pass skipped all read-only names and left 48 of 53
    //               defects standing. But an unrecognised name that is only
    //               read could be a real global, and declaring that would
    //               shadow it to nil — turning working output into broken
    //               output. Restricting to generated shapes removes that
    //               risk entirely, since nothing real is ever named `v30`.
    let mut free: BTreeSet<String> = scan
        .assigned
        .iter()
        .filter(|n| !scan.bound.contains(*n))
        .filter(|n| !is_known_global(n))
        .filter(|n| !n.is_empty())
        .cloned()
        .collect();

    // Read-only names: declare ONLY if the reads are consistent with a capture.
    //
    // ── WHY THIS GUARD EXISTS ───────────────────────────────────────────
    // The earlier version declared every generated read-only name. It took
    // `undefined_local` from 48 to 0 and that was reported as a fix. It was
    // not: the win conflated two different situations.
    //
    //   * a genuine captured upvalue missing its declaration -> declaring is right
    //   * a value the lifter dropped                          -> declaring is WRONG
    //
    // The second case is worse than the defect it replaces, because it is
    // silent. `undefined_local` cannot see it — the name IS declared — so the
    // output parses while every use evaluates to nil.
    //
    // Measured cost of getting this wrong: on a 628-script corpus a check that
    // free_var_decls could not hide from (`declared_never_assigned`) put the
    // clean rate at 13.7%, not the 94.9% reported while the masking was in
    // place. 532 of 628 files carried silently-nil values.
    //
    // Concretely, `ReplicatedStorage.Badges` inlines a helper at 25 call sites
    // with only its receiver substituted:
    //
    //     if Counter.Count then
    //         Counter.Count = v12      -- declared at chunk top, never assigned
    //     end
    //
    // — so 25 badge counts silently became nil.
    //
    // ── THE DISCRIMINANT ────────────────────────────────────────────────
    // A capture is BY DEFINITION a reference from inside a nested function to a
    // name in an enclosing scope. So a real capture is read INSIDE a closure.
    // A generated name read at chunk top level that nothing ever assigns is not
    // a capture — it is a hole where the lifter dropped a value.
    //
    // Declaring holes hides them. Leaving them undeclared keeps them visible to
    // `undefined_local`, which is the honest outcome: still broken, but broken
    // in a way the tooling reports.
    for n in &scan.read {
        if scan.bound.contains(n) || is_known_global(n) {
            continue;
        }
        if orphan {
            // Detached proto: nothing out here to shadow, nothing to capture.
            free.insert(n.clone());
            continue;
        }
        if !is_generated_name(n) {
            continue;
        }
        // Read only from inside closures -> consistent with a capture.
        if !scan.read_outside_closure.contains(n) {
            free.insert(n.clone());
        }
        // Otherwise: leave undeclared on purpose, so the defect stays visible.
    }

    // Place each admitted name in the innermost function that assigns it —
    // see the `declare_scoped` comment block for why chunk top is wrong both
    // semantically and (at 805 names) syntactically.
    let admitted: BTreeSet<String> = free.clone();
    let mut placed: BTreeSet<String> = BTreeSet::new();
    {
        // `declare_scoped` needs `&mut body`, so the chunk body itself takes
        // part in the walk; names it declares here are the chunk-top names.
        let chunk_scope: BTreeSet<String> = BTreeSet::new();
        declare_scoped(body, &chunk_scope, &admitted, &mut placed);
    }
    // Anything the walker did not reach keeps the old chunk-top placement, so
    // this pass can never leave a previously-declared name undeclared.
    let free: Vec<String> = free.difference(&placed).cloned().collect();
    // Report every name this pass declared, wherever it put it, so the
    // diagnostic log stays truthful about what was fixed.
    let mut reported: Vec<String> = placed.iter().cloned().collect();
    reported.extend(free.iter().cloned());

    // Emitted here rather than at each mint site because this is the only point
    // that knows which branch hoisted the name -- ASSIGNED names are written
    // somewhere in the body and are defensible, READONLY ones evaluate to nil.
    if crate::decompiler::mint_trace::enabled() {
        for n in &free {
            let branch = if scan.assigned.contains(n) { "ASSIGNED" } else { "READONLY" };
            let in_closure = scan.read.contains(n) && !scan.read_outside_closure.contains(n);
            eprintln!(
                "MINT\t{}\t{}\tinclosure={}\tsites={}",
                branch,
                n,
                in_closure,
                crate::decompiler::mint_trace::sites_for(n)
            );
        }
        crate::decompiler::mint_trace::clear();
    }

    if free.is_empty() {
        return reported;
    }

    // Chunk top, but after any leading comment header so the banner stays put.
    let insert_at = body
        .iter()
        .position(|s| !matches!(s, Stat::Comment(_)))
        .unwrap_or(0);

    body.insert(
        insert_at,
        Stat::Local { names: free.clone(), values: Vec::new() },
    );
    reported
}

// ── Scope-local declaration ─────────────────────────────────────────────
//
// `declare_free_vars` used to place EVERY admitted name at chunk top. That is
// the wrong scope, and on a large chunk it is also the wrong SHAPE: 3536
// protos contributed 805 names to a single `local` statement, and Luau allows
// only 255 live registers, so the output could not be compiled at all.
//
// The placement is also what a reader would expect from the source. An
// assignment to a name that the function never declares is, in the original
// program, a `local` at the top of THAT function -- which is exactly where
// the lifter's register tracker believes it put one. Nested functions then
// see it as a genuine upvalue, which is how the bytecode captured it.
//
// Everything below only ever MOVES a declaration from chunk top into a
// function; the admission rules are untouched, because the caller passes the
// chunk-top set as `admitted` and we may only declare names from it. A name
// the walker fails to reach stays at chunk top, so this cannot regress a file
// that used to work.

/// Collect the names bound and assigned by ONE function body, excluding the
/// bodies of any nested closures (those have their own scope).
fn collect_scope(stats: &[Stat]) -> Scan {
    let mut out = Scan { skip_closures: true, ..Scan::default() };
    scan_stats(stats, &mut out);
    out
}

/// Visit every closure body directly nested in `stats`, mutably.
fn visit_closures(stats: &mut [Stat], f: &mut dyn FnMut(&mut Vec<Stat>)) {
    for s in stats.iter_mut() {
        match s {
            Stat::Local { values, .. } => {
                for v in values {
                    visit_closures_expr(v, f);
                }
            }
            Stat::Assign { targets, values } => {
                for t in targets {
                    visit_closures_expr(t, f);
                }
                for v in values {
                    visit_closures_expr(v, f);
                }
            }
            Stat::If { condition, then_body, elseif_clauses, else_body } => {
                visit_closures_expr(condition, f);
                visit_closures(then_body, f);
                for (c, b) in elseif_clauses.iter_mut() {
                    visit_closures_expr(c, f);
                    visit_closures(b, f);
                }
                if let Some(b) = else_body {
                    visit_closures(b, f);
                }
            }
            Stat::While { condition, body } => {
                visit_closures_expr(condition, f);
                visit_closures(body, f);
            }
            Stat::Repeat { body, condition } => {
                visit_closures(body, f);
                visit_closures_expr(condition, f);
            }
            Stat::NumericFor { start, stop, step, body, .. } => {
                visit_closures_expr(start, f);
                visit_closures_expr(stop, f);
                if let Some(st) = step {
                    visit_closures_expr(st, f);
                }
                visit_closures(body, f);
            }
            Stat::GenericFor { iterators, body, .. } => {
                for it in iterators {
                    visit_closures_expr(it, f);
                }
                visit_closures(body, f);
            }
            Stat::Return { values } => {
                for v in values {
                    visit_closures_expr(v, f);
                }
            }
            Stat::DoBlock { body } => visit_closures(body, f),
            Stat::ExprStat(e) => visit_closures_expr(e, f),
            Stat::LocalFunction { func, .. } => visit_closures_expr(func, f),
            Stat::MethodFunction { receiver, func, .. } => {
                visit_closures_expr(receiver, f);
                visit_closures_expr(func, f);
            }
            Stat::Break | Stat::Continue | Stat::Comment(_) => {}
        }
    }
}

fn visit_closures_expr(expr: &mut Expr, f: &mut dyn FnMut(&mut Vec<Stat>)) {
    match expr {
        Expr::Nil
        | Expr::Bool(_)
        | Expr::Number(_)
        | Expr::String(_)
        | Expr::Varargs
        | Expr::Name(_)
        | Expr::Vector(..) => {}
        Expr::Field { object, .. } => visit_closures_expr(object, f),
        Expr::Index { object, key } => {
            visit_closures_expr(object, f);
            visit_closures_expr(key, f);
        }
        Expr::BinOp { left, right, .. } => {
            visit_closures_expr(left, f);
            visit_closures_expr(right, f);
        }
        Expr::UnOp { operand, .. } => visit_closures_expr(operand, f),
        Expr::Call { func, args } => {
            visit_closures_expr(func, f);
            for a in args {
                visit_closures_expr(a, f);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            visit_closures_expr(object, f);
            for a in args {
                visit_closures_expr(a, f);
            }
        }
        Expr::Function { body, .. } => f(body),
        Expr::Table { fields } => {
            for field in fields.iter_mut() {
                match field {
                    TableField::Sequential(e) => visit_closures_expr(e, f),
                    TableField::Named(_, e) => visit_closures_expr(e, f),
                    TableField::Indexed(k, v) => {
                        visit_closures_expr(k, f);
                        visit_closures_expr(v, f);
                    }
                }
            }
        }
        Expr::Ternary { cond, then_expr, else_expr } => {
            visit_closures_expr(cond, f);
            visit_closures_expr(then_expr, f);
            visit_closures_expr(else_expr, f);
        }
    }
}

/// Declare, at the top of each function that assigns them, the names from
/// `admitted` that the function neither binds nor inherits. Recurses into
/// nested closures with the accumulated scope. Returns the names placed.
fn declare_scoped(
    stats: &mut Vec<Stat>,
    inherited: &BTreeSet<String>,
    admitted: &BTreeSet<String>,
    placed: &mut BTreeSet<String>,
) {
    let scope = collect_scope(stats);

    let mut free: Vec<String> = scope
        .assigned
        .iter()
        .filter(|n| admitted.contains(*n))
        .filter(|n| !scope.bound.contains(*n))
        .filter(|n| !inherited.contains(*n))
        .cloned()
        .collect();
    free.sort();
    free.dedup();

    if !free.is_empty() {
        for n in &free {
            placed.insert(n.clone());
        }
        let insert_at = stats
            .iter()
            .position(|s| !matches!(s, Stat::Comment(_)))
            .unwrap_or(0);
        stats.insert(insert_at, Stat::Local { names: free.clone(), values: Vec::new() });
    }

    let mut child_scope: BTreeSet<String> = inherited.union(&scope.bound).cloned().collect();
    child_scope.extend(free);
    visit_closures(stats, &mut |body: &mut Vec<Stat>| {
        declare_scoped(body, &child_scope, admitted, placed);
    });
}

// ── Scanning ────────────────────────────────────────────────────────────

fn scan_stats(stats: &[Stat], out: &mut Scan) {
    for s in stats {
        scan_stat(s, out);
    }
}

fn scan_stat(stat: &Stat, out: &mut Scan) {
    match stat {
        Stat::Local { names, values } => {
            for n in names {
                out.bound.insert(n.clone());
            }
            for v in values {
                scan_expr(v, out);
            }
        }
        Stat::Assign { targets, values } => {
            for t in targets {
                // A bare `Name` target is an assignment to that identifier.
                // `a.b = x` and `a[k] = x` assign to a FIELD of `a`, which
                // still requires `a` to exist — recorded as a read below.
                if let Expr::Name(n) = t {
                    out.assigned.insert(n.clone());
                } else {
                    scan_expr(t, out);
                }
            }
            for v in values {
                scan_expr(v, out);
            }
        }
        Stat::If { condition, then_body, elseif_clauses, else_body } => {
            scan_expr(condition, out);
            scan_stats(then_body, out);
            for (c, b) in elseif_clauses {
                scan_expr(c, out);
                scan_stats(b, out);
            }
            if let Some(b) = else_body {
                scan_stats(b, out);
            }
        }
        Stat::While { condition, body } => {
            scan_expr(condition, out);
            scan_stats(body, out);
        }
        Stat::Repeat { body, condition } => {
            scan_stats(body, out);
            scan_expr(condition, out);
        }
        Stat::NumericFor { var, start, stop, step, body } => {
            out.bound.insert(var.clone());
            scan_expr(start, out);
            scan_expr(stop, out);
            if let Some(s) = step {
                scan_expr(s, out);
            }
            scan_stats(body, out);
        }
        Stat::GenericFor { vars, iterators, body } => {
            for v in vars {
                out.bound.insert(v.clone());
            }
            for it in iterators {
                scan_expr(it, out);
            }
            scan_stats(body, out);
        }
        Stat::Return { values } => {
            for v in values {
                scan_expr(v, out);
            }
        }
        Stat::DoBlock { body } => scan_stats(body, out),
        Stat::ExprStat(e) => scan_expr(e, out),
        Stat::LocalFunction { name, func } => {
            out.bound.insert(name.clone());
            scan_expr(func, out);
        }
        Stat::MethodFunction { receiver, func, .. } => {
            scan_expr(receiver, out);
            scan_expr(func, out);
        }
        Stat::Break | Stat::Continue | Stat::Comment(_) => {}
    }
}

fn scan_expr(expr: &Expr, out: &mut Scan) {
    match expr {
        // Reads are recorded separately from writes. They are only ever
        // actionable for decompiler-generated names — see the admission
        // rules in `declare_free_vars`.
        Expr::Name(n) => {
            out.read.insert(n.clone());
            if out.closure_depth == 0 {
                out.read_outside_closure.insert(n.clone());
            }
        }
        Expr::Field { object, .. } => scan_expr(object, out),
        Expr::Index { object, key } => {
            scan_expr(object, out);
            scan_expr(key, out);
        }
        Expr::BinOp { left, right, .. } => {
            scan_expr(left, out);
            scan_expr(right, out);
        }
        Expr::UnOp { operand, .. } => scan_expr(operand, out),
        Expr::Call { func, args } => {
            scan_expr(func, out);
            for a in args {
                scan_expr(a, out);
            }
        }
        Expr::MethodCall { object, args, .. } => {
            scan_expr(object, out);
            for a in args {
                scan_expr(a, out);
            }
        }
        Expr::Function { params, body, .. } => {
            if out.skip_closures {
                return;
            }
            // Parameters bind INSIDE the closure. Recording them as bound
            // chunk-wide is intentional and conservative: it can only cause
            // us to declare fewer names, never more, so it cannot introduce
            // a shadowing regression.
            for p in params {
                out.bound.insert(p.clone());
            }
            // Track nesting so reads can be attributed to inside-a-closure
            // (a possible capture) or chunk top level (never a capture).
            out.closure_depth += 1;
            scan_stats(body, out);
            out.closure_depth -= 1;
        }
        Expr::Table { fields } => {
            for f in fields {
                match f {
                    TableField::Sequential(v) => scan_expr(v, out),
                    TableField::Named(_, value) => scan_expr(value, out),
                    TableField::Indexed(key, value) => {
                        scan_expr(key, out);
                        scan_expr(value, out);
                    }
                }
            }
        }
        Expr::Ternary { cond, then_expr, else_expr } => {
            scan_expr(cond, out);
            scan_expr(then_expr, out);
            scan_expr(else_expr, out);
        }
        Expr::Nil
        | Expr::Bool(_)
        | Expr::Number(_)
        | Expr::String(_)
        | Expr::Varargs
        | Expr::Vector(..) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(n: &str) -> Expr {
        Expr::Name(n.to_string())
    }

    /// The PopperCam shape: a closure assigns to a captured upvalue that
    /// nothing declares.
    #[test]
    fn declares_assigned_but_unbound() {
        let mut body = vec![Stat::LocalFunction {
            name: "f".into(),
            func: Expr::Function {
                params: vec![],
                is_vararg: false,
                body: vec![Stat::Assign {
                    targets: vec![name("v7")],
                    values: vec![Expr::Bool(true)],
                }],
            },
        }];
        let declared = declare_free_vars(&mut body);
        assert_eq!(declared, vec!["v7".to_string()]);
        // The declaration now goes to the innermost function that assigns the
        // name, not to chunk top: `f` is the only scope that touches `v7`, and
        // a chunk-top local would have made 800-odd of these share the main
        // proto's 200-register budget (see `declare_scoped`).
        let inner = match &body[0] {
            Stat::LocalFunction { func, .. } => match func {
                Expr::Function { body, .. } => body,
                _ => panic!("expected Function"),
            },
            _ => panic!("expected LocalFunction"),
        };
        assert!(
            matches!(&inner[0], Stat::Local { names, .. } if names == &vec!["v7".to_string()]),
            "expected `local v7` inside f, got {:?}",
            inner
        );
    }

    /// Writing to a real global must NOT be shadowed — that would turn
    /// working output into broken output.
    #[test]
    fn never_shadows_known_globals() {
        let mut body = vec![Stat::Assign {
            targets: vec![name("workspace")],
            values: vec![Expr::Nil],
        }];
        let declared = declare_free_vars(&mut body);
        assert!(declared.is_empty(), "must not declare a known global: {:?}", declared);
    }

    /// An UNRECOGNISED name that is only read is left alone — it may be a
    /// global we do not know about, and declaring it would shadow it to nil.
    #[test]
    fn unknown_read_only_name_is_left_alone() {
        let mut body = vec![Stat::ExprStat(Expr::Call {
            func: Box::new(name("SomeUnknownGlobal")),
            args: vec![],
        })];
        let declared = declare_free_vars(&mut body);
        assert!(declared.is_empty(), "read-only name declared: {:?}", declared);
    }

    // ── Read-only generated names: WHERE the read happens decides ─────────
    //
    // These two tests differ in exactly one thing — whether the read sits
    // inside a closure — and they must disagree about the outcome. That
    // difference IS the pass's central judgement, so it is pinned from both
    // sides.
    //
    // An earlier version declared every generated read-only name. It took
    // `undefined_local` from 48 to 0 and was reported as a fix; it was
    // masking. The check that free_var_decls cannot hide from
    // (`declared_never_assigned`) later put the corpus clean rate at 13.7%,
    // not the 94.9% claimed while the masking was in place — 532 of 628 files
    // carried values that were declared and then never assigned.

    /// A generated name read ONLY from inside a closure is a captured upvalue
    /// whose declaration was lost. Declaring it at chunk top is the fix this
    /// pass exists for.
    #[test]
    fn generated_name_read_only_inside_a_closure_is_declared() {
        let mut body = vec![Stat::LocalFunction {
            name: "f".into(),
            func: Expr::Function {
                params: vec![],
                is_vararg: false,
                body: vec![Stat::ExprStat(Expr::Call {
                    func: Box::new(name("print")),
                    args: vec![name("v30")],
                })],
            },
        }];
        let declared = declare_free_vars(&mut body);
        assert_eq!(
            declared,
            vec!["v30".to_string()],
            "a name referenced from inside a nested function, and bound by no \
             scope, is a capture — declaring it is the whole point of the pass"
        );
    }

    /// The same name read at chunk top level is NOT a capture — a capture is by
    /// definition a reference from inside a nested function. It is a hole where
    /// the lifter dropped a value, and it must stay undeclared so
    /// `undefined_local` keeps reporting it.
    ///
    /// Declaring it would be worse than the defect it replaces, because the
    /// output would parse while every use evaluated to nil — silently.
    #[test]
    fn generated_name_read_at_top_level_stays_undeclared() {
        let mut body = vec![Stat::Local {
            names: vec!["x".into()],
            values: vec![Expr::BinOp {
                left: Box::new(name("v30")),
                op: crate::ast::BinOp::Add,
                right: Box::new(name("v29")),
            }],
        }];
        let declared = declare_free_vars(&mut body);
        assert!(
            declared.is_empty(),
            "declared {:?} — a top-level read-only name is a dropped value, not \
             a capture. Declaring it hides the defect instead of fixing it.",
            declared
        );
    }

    /// The generated-name test must be tight: real identifiers that merely
    /// start with `v` are not ours and must never be declared.
    #[test]
    fn generated_name_test_is_tight() {
        assert!(is_generated_name("v0"));
        assert!(is_generated_name("v30"));
        assert!(is_generated_name("upval_3"));
        assert!(is_generated_name("cap_1"));
        assert!(!is_generated_name("v"));
        assert!(!is_generated_name("value"));
        assert!(!is_generated_name("vec3"));
        assert!(!is_generated_name("velocity"));
        assert!(!is_generated_name("workspace"));
    }

    /// Already-declared locals must not be re-declared.
    #[test]
    fn respects_existing_declarations() {
        let mut body = vec![
            Stat::Local { names: vec!["x".into()], values: vec![Expr::Number(1.0)] },
            Stat::Assign { targets: vec![name("x")], values: vec![Expr::Number(2.0)] },
        ];
        let declared = declare_free_vars(&mut body);
        assert!(declared.is_empty(), "re-declared a bound local: {:?}", declared);
    }

    /// Loop variables bind, so assigning to one is not a free variable.
    #[test]
    fn loop_vars_count_as_bound() {
        let mut body = vec![Stat::NumericFor {
            var: "i".into(),
            start: Expr::Number(1.0),
            stop: Expr::Number(10.0),
            step: None,
            body: vec![Stat::Assign {
                targets: vec![name("i")],
                values: vec![Expr::Number(0.0)],
            }],
        }];
        let declared = declare_free_vars(&mut body);
        assert!(declared.is_empty(), "loop var declared: {:?}", declared);
    }

    /// Field assignment (`a.b = x`) requires `a` but does not assign `a`
    /// itself, so it must not create a declaration for `a`.
    #[test]
    fn field_assignment_does_not_declare_object() {
        let mut body = vec![Stat::Assign {
            targets: vec![Expr::Field {
                object: Box::new(name("SomeTable")),
                field: "k".into(),
            }],
            values: vec![Expr::Number(1.0)],
        }];
        let declared = declare_free_vars(&mut body);
        assert!(declared.is_empty(), "declared a field-assign object: {:?}", declared);
    }

    #[test]
    fn declares_multiple_in_one_statement() {
        let mut body = vec![Stat::Assign {
            targets: vec![name("a1"), name("b2")],
            values: vec![Expr::Nil, Expr::Nil],
        }];
        let mut declared = declare_free_vars(&mut body);
        declared.sort();
        assert_eq!(declared, vec!["a1".to_string(), "b2".to_string()]);
    }
}
