//! Register-to-local-name tracking and classification.
//!
//! Extracted from `lifter.rs` as part of Phase B0.52P6 (part 3/3 of the split).
//! Contains the primitive types (`RegVal`, `WriteKind`, `LocalTracker`) and
//! the `is_semantic_local_name` helper used throughout the lifter to decide
//! how to emit register writes.
//!
//! All items are `pub(super)` so the rest of the `lifter` module can use them,
//! but nothing is exposed outside the module tree.

use std::collections::{HashMap, HashSet};

use crate::ast::Expr;

/// Register tracking during decompilation
#[derive(Debug, Clone)]
pub(crate) enum RegVal {
    Unknown,
    Expr(Expr),
    /// A GenericFor loop variable seeded by Phase B0.10.
    ///
    /// Behaves like `Expr(Expr::Name(s))` for read operations inside the loop
    /// body — `reg_expr` returns `Expr::Name(s)`, and `store_complex` detects
    /// self-mutation against the name.
    ///
    /// Critically, the CALL vararg boundary scanner (B=0 path, lines ~2083–2090)
    /// uses `_ => break` for any non-`Expr` variant, so `LoopVar` registers are
    /// NOT picked up as trailing call arguments.  This fixes the regression
    /// introduced by Phase B0.10 where seeding loop variable registers with
    /// `Expr(Name)` caused adjacent vararg CALLs to absorb them as extra args.
    LoopVar(String),
}

/// Phase B0.49 — classifies a register-write emission into one of three kinds.
///
/// * `FirstDecl` — this register has never been written before in this proto;
///   emit `local <name> = <value>`.
/// * `Shadow`    — this register was previously declared with a DIFFERENT
///   semantic name; emit `local <name> = <value>` to shadow the old local.
///   Luau allows shadowing (the new binding masks the old within its scope),
///   so this is semantically correct.  Reads of the register after the shadow
///   resolve to the NEW name because subsequent register-commit sites overwrite
///   `regs[reg]` with `Expr::Name(new_name)`.
/// * `Reassign` — this register was previously declared with the SAME name
///   (or the new name looks generic such as `v\d+`, in which case we prefer
///   to keep using the existing name rather than churn); emit
///   `<existing_name> = <value>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WriteKind {
    FirstDecl,
    Shadow,
    Reassign,
}

/// Tracks which registers have been declared as locals.
///
/// Scoping invariant: Luau `local` is block-scoped but bytecode registers are
/// function-scoped.  When a register is first written inside a loop body we
/// must hoist the declaration before the loop.  Use `snapshot()`/`new_since()`
/// to detect registers first declared in a nested scope, then hoist them.
///
/// Phase B0.49: additionally tracks the CURRENT local name per register so we
/// can emit a shadow-local declaration when a subsequent write gives the
/// register a new semantic name (e.g., two consecutive `NEWCLOSURE` writes to
/// R1 with distinct debug_names — the second used to be emitted as a global
/// assignment because R1 was "already declared").  See the test module
/// `phase_b049_shadow_local_tests` for exhaustive coverage.
// Visible up to `crate::decompiler` because several `pub(super)` lifter helpers
// (ensure_table_reg_declared, store_complex, …) take it in their signatures.
#[derive(Clone)]
pub(in crate::decompiler) struct LocalTracker {
    pub(super) declared: HashSet<usize>,
    param_count: usize,
    /// Phase B0.49 — the name currently bound to each register as a local.
    /// Populated/updated by `classify_write`.  Reads of the register during
    /// lifting resolve via `regs[]` (which mirrors this name), so this map
    /// is only used to decide whether a subsequent write should Shadow or
    /// Reassign.
    current_names: HashMap<usize, String>,
    /// Registers whose `current_names` entry is a read-only ALIAS of an outer
    /// binding (an upvalue read via GETUPVAL), NOT this register's own declared
    /// value. When such a register is later given its own value, `classify_write`
    /// must NOT preserve the alias name: reusing e.g. `v9` (a captured upvalue's
    /// own name) for an unrelated later CALL result collides with the real
    /// upvalue and makes dead-store elimination delete real statements (a corpus case).
    alias_regs: HashSet<usize>,
}

impl LocalTracker {
    pub(super) fn new(param_count: usize) -> Self {
        let mut declared = HashSet::new();
        for i in 0..param_count {
            declared.insert(i);
        }
        Self { declared, param_count, current_names: HashMap::new(), alias_regs: HashSet::new() }
    }

    /// Returns true if this register needs a `local` declaration (first write).
    pub(super) fn needs_local(&mut self, reg: usize) -> bool {
        if reg < self.param_count {
            return false;
        }
        self.declared.insert(reg)
    }

    /// Phase B0.51 — returns true iff `reg` has never been declared as a local
    /// in this proto AND is not a parameter slot.  Used by
    /// `ensure_table_reg_declared` to decide whether an Unknown table register
    /// needs to be materialized as `local vN = {}`.
    pub(super) fn is_undeclared_non_param(&self, reg: usize) -> bool {
        reg >= self.param_count && !self.declared.contains(&reg)
    }

    /// A register that is not one of this proto's parameters.
    pub(super) fn is_non_param(&self, reg: usize) -> bool {
        reg >= self.param_count
    }

    /// Snapshot the current declared set before entering a nested scope.
    pub(super) fn snapshot(&self) -> HashSet<usize> {
        self.declared.clone()
    }

    /// Return registers newly declared since the snapshot.
    pub(super) fn new_since(&self, snap: &HashSet<usize>) -> Vec<usize> {
        let mut v: Vec<usize> = self.declared.difference(snap).copied().collect();
        v.sort_unstable();
        v
    }

    /// Pre-declare a register without emitting a local statement.
    pub(super) fn pre_declare(&mut self, reg: usize) {
        self.declared.insert(reg);
    }

    /// LUAU_PREDECL_TRACE: report a pre-declaration with proto identity.
    ///
    /// `pre_declare` marks a register declared WITHOUT emitting a `local`.
    /// That is only sound when the surrounding code emits the declaration
    /// (a `for` loop -- the loop variable IS declared by the `for` line --
    /// or a multi-assignment `local a, b = f()`). Everywhere else it
    /// manufactures a register that the tracker believes is bound and the
    /// output never binds, so the next write to it emits a bare `X = ...`
    /// and Luau reads that as a GLOBAL write. Those bare writes are what
    /// `free_var_decls` then hoists to chunk top.
    pub(super) fn pre_declare_trace(proto: &crate::parser::types::Proto, reg: usize, pc: usize, why: &str) {
        if std::env::var("LUAU_PREDECL_TRACE").is_ok() {
            eprintln!(
                "PREDECL\t{}@{}\treg={}\tpc={}\t{}",
                proto.debug_name.as_deref().unwrap_or("?"),
                proto.line_defined,
                reg,
                pc,
                why
            );
        }
    }

    /// Force a FRESH shadow identity onto a register, overwriting any stale
    /// declaration and name. Returns the name to use. This is for the branch
    /// seed of a register whose PRIOR lifetime is dead (its live value was
    /// clobbered back to `Unknown`) and which the Luau compiler has REUSED for
    /// a new two-arm-diamond value. `classify_write` would keep the stale
    /// generic name and route the arm writes through `Reassign`, which the
    /// seed gate then rejects, leaving the merge read unbound. Seeding a fresh
    /// name is bit-exact whenever both arms write the register and the merge
    /// reads it before redefining it — the pre-branch (dead) value is provably
    /// never observed — and it never reuses the old name, so the repurposed-slot
    /// hazard (a corpus module) cannot arise.
    pub(super) fn reseed_shadow(&mut self, reg: usize, name: &str) -> String {
        self.declared.insert(reg);
        self.current_names.insert(reg, name.to_string());
        self.alias_regs.remove(&reg);
        name.to_string()
    }

    /// Collapsing variant for writes that cannot carry a function value
    /// (GETGLOBAL / GETIMPORT / GETTABLEKS / NEWTABLE / CALL results).
    ///
    /// Those handlers each minted a fresh local whenever `ctx.reg_name` picked
    /// a different hint for an already-declared register, which is the same
    /// churn `classify_write_ex` collapses -- and the reason the client chunk
    /// reached 722 top-level locals against a 173-register proto. Callers that
    /// CAN store a closure must keep using `classify_write`/`classify_write_ex`,
    /// because collapsing two closure writes onto one name lets dead-store
    /// elimination delete the first body.
    pub(super) fn classify_write_nonfn(&mut self, reg: usize, new_name: &str) -> (WriteKind, String) {
        if std::env::var("LUAU_NO_SHADOW_COLLAPSE").is_ok() {
            return self.classify_write(reg, new_name);
        }
        if reg >= self.param_count && self.declared.contains(&reg) && !self.alias_regs.contains(&reg) {
            if let Some(old) = self.current_names.get(&reg).cloned() {
                if old != "self" {
                    return (WriteKind::Reassign, old);
                }
            }
        }
        self.classify_write(reg, new_name)
    }

    /// Like [`Self::classify_write`], but told what value is being stored.
    ///
    /// A register that is reused for a new value is ONE variable in the source
    /// whose old lifetime has ended -- `local x = a; ...; x = b`. Shadowing it
    /// into a second local (`local x2 = b`) is also valid Luau, but it costs a
    /// register for the rest of the function, and Luau allows only 200. On the
    /// client chunk the main proto declares 173 registers in its bytecode and
    /// was being emitted with 769 top-level locals, because `ctx.reg_name` is
    /// PC-scoped and renames the same register on every pass.
    ///
    /// Function values are the exception and keep the shadow: a closure that
    /// is materialised into a named local must keep its own binding, because
    /// collapsing two closure writes onto one name makes the first a plain
    /// overwrite that dead-store elimination deletes -- together with the
    /// proto body it held (measured: 3,568 -> 200 `function` keywords).
    pub(super) fn classify_write_ex(
        &mut self,
        reg: usize,
        new_name: &str,
        value: &Expr,
    ) -> (WriteKind, String) {
        if std::env::var("LUAU_NO_SHADOW_COLLAPSE").is_ok() {
            return self.classify_write(reg, new_name);
        }
        if matches!(value, Expr::Function { .. }) {
            return self.classify_write(reg, new_name);
        }
        if reg < self.param_count {
            let existing = self.current_names.get(&reg).cloned();
            return (WriteKind::Reassign, existing.unwrap_or_else(|| new_name.to_string()));
        }
        if !self.declared.contains(&reg) {
            self.declared.insert(reg);
            self.current_names.insert(reg, new_name.to_string());
            return (WriteKind::FirstDecl, new_name.to_string());
        }
        if self.alias_regs.contains(&reg) {
            self.alias_regs.remove(&reg);
            self.current_names.insert(reg, new_name.to_string());
            return (WriteKind::FirstDecl, new_name.to_string());
        }
        match self.current_names.get(&reg).cloned() {
            Some(old) if old == new_name => (WriteKind::Reassign, old),
            Some(old) => {
                if old == "self" && new_name != "self" {
                    self.current_names.insert(reg, new_name.to_string());
                    return (WriteKind::Shadow, new_name.to_string());
                }
                self.alias_regs.remove(&reg);
                (WriteKind::Reassign, old)
            }
            None => {
                self.current_names.insert(reg, new_name.to_string());
                (WriteKind::Reassign, new_name.to_string())
            }
        }
    }

    /// The local name currently bound to `reg`, if it holds a live binding.
    pub(super) fn current_name(&self, reg: usize) -> Option<&str> {
        self.current_names.get(&reg).map(|s| s.as_str())
    }

    /// Is `name` the local currently bound to some register in this proto?
    ///
    /// Copy-propagated registers (a plain `MOVE` emits no statement) are not in
    /// `declared`, so a register-keyed check cannot tell that a call argument is
    /// an alias of a real binding. Checking the NAME does.
    pub(super) fn is_bound_name(&self, name: &str) -> bool {
        self.current_names.values().any(|n| n == name)
    }

    /// Record the current local name for a register.  Callers that emit a
    /// `Stat::Local` via the existing `needs_local`/push flow (e.g., multi-
    /// return CALL, GETVARARGS) can call this to keep `current_names` in
    /// sync with the emitted bindings.
    pub(super) fn record_name(&mut self, reg: usize, name: &str) {
        self.current_names.insert(reg, name.to_string());
        self.alias_regs.remove(&reg);
    }

    /// Record that `reg` now holds a read-only ALIAS of an outer binding (an
    /// upvalue read), under the outer binding's own `name`. Behaves like
    /// `record_name` for reads, but marks `reg` so a later write of the
    /// register's OWN value does not silently inherit the alias name and
    /// collide with the real binding (a corpus case). Cleared by the next real write.
    pub(super) fn record_alias_name(&mut self, reg: usize, name: &str) {
        self.current_names.insert(reg, name.to_string());
        self.alias_regs.insert(reg);
    }

    /// Phase B0.49 — decide how to emit a single-register write.
    ///
    /// Returns a `(WriteKind, name_to_use)` pair.  For `FirstDecl` and
    /// `Shadow`, `name_to_use` is the caller's `new_name` (and the tracker
    /// records it as the current binding).  For `Reassign`, `name_to_use`
    /// is the EXISTING declared name — callers MUST use this, not the
    /// freshly-computed `new_name`, to avoid emitting writes to an
    /// undeclared identifier (which Luau parses as a global assignment).
    ///
    /// Narrowness gate: we only shadow when BOTH current and new names are
    /// "semantic" (not `v\d+` fallbacks).  This prevents per-write churn
    /// caused by `ctx.reg_name`'s unique-suffix counter and avoids the
    /// Phase B0.44A regression (which broadly rewrote writes via a
    /// write-tracking map and inflated `v\d+` by +51%).
    pub(super) fn classify_write(&mut self, reg: usize, new_name: &str) -> (WriteKind, String) {
        // Params always reassign — the register IS the parameter, never
        // re-declared.  Mirrors `needs_local`'s behavior.
        //
        // AND THEY REASSIGN UNDER THEIR OWN NAME.
        //
        // This returned `new_name` — the value-shape hint `ctx.reg_name`
        // mints for whatever value happens to be stored — which breaks the
        // contract stated on this function and turns a parameter
        // reassignment into a write to an undeclared identifier:
        //
        //     local function Communicate(arg1, arg2)
        //         if typeof(arg2) ~= "table" then
        //             tbl2 = {}          -- was `arg2 = {}`
        //         end
        //
        // `tbl2` is a NEWTABLE hint, not a binding. Luau reads the emitted
        // line as a GLOBAL write: the parameter keeps its old value and a
        // stray global appears. The value join that should have folded this
        // to `arg2 = cond and {} or arg2` also rejected it, because the arm
        // produced an `Assign` rather than a pure temp — which is how one
        // wrong name cascaded into 858 names hoisted at chunk top.
        //
        // `current_names` is seeded for every parameter by `lift_proto_inner`
        // (it calls `record_name` for each), so the parameter's own name is
        // available here. `new_name` remains the fallback for the impossible
        // case where a parameter register was never recorded.
        if reg < self.param_count {
            let existing = self.current_names.get(&reg).cloned();
            return (WriteKind::Reassign, existing.unwrap_or_else(|| new_name.to_string()));
        }
        if !self.declared.contains(&reg) {
            self.declared.insert(reg);
            self.current_names.insert(reg, new_name.to_string());
            return (WriteKind::FirstDecl, new_name.to_string());
        }
        // B0.131 — an ALIAS register has no local of its own.
        //
        // `GETUPVAL R(a) U(b)` sets `regs[a] = Name(upval)` and records the
        // register as an alias: reads of R(a) are really reads of the upvalue
        // binding, which DOES exist (it is the parent's local or one of this
        // proto's own hoisted upvalues).  So a read needs no declaration, and
        // pre-declaring the register was a cheap way to stop the following
        // SETUPVAL write-back from materialising a shadow.
        //
        // But pre-declaring also tells this function that the register has a
        // BINDING, and it does not: nothing was ever emitted.  When the VM
        // reuses R(a) for an unrelated value, every branch below classifies
        // that write as `Shadow` and emits `newname = <value>` — an assignment
        // to an identifier that was never declared, which Luau reads as a
        // GLOBAL write.  `free_var_decls` then finds those adrift names in
        // `scan.assigned` and hoists them to chunk top: 1,017 of the 1,954
        // bare writes on the client chunk traced back to exactly this site
        // (and, via the hoist, most of the 805 names over the 255 ceiling).
        //
        // A rename here is therefore a genuine FIRST DECLARATION of a local
        // for this register, not a shadow of an existing one.  Emitting
        // `local new = value` keeps the value reachable and leaves
        // `declared`/`current_names` honest, so later writes reassign it.
        // The self-mutation case (`GETUPVAL/ADDK/SETUPVAL` -> `hits = hits+1`)
        // never reaches here: the hint equals the alias name, so `old ==
        // new_name` and the Reassign arm below keeps it writing the upvalue.
        if self.alias_regs.contains(&reg) {
            self.alias_regs.remove(&reg);
            self.current_names.insert(reg, new_name.to_string());
            return (WriteKind::FirstDecl, new_name.to_string());
        }
        // Already declared.  Decide Shadow vs Reassign.
        let old_name = self.current_names.get(&reg).cloned();
        match old_name {
            Some(old) if old == new_name => {
                // Same name → simple reassignment.
                (WriteKind::Reassign, old)
            }
            Some(old) => {
                // B0.130: "self" is a NAMECALL artifact — never preserve
                // it as a reassignment target for non-self values.  Without
                // this, NEWCLOSURE on a register previously holding "self"
                // emits `self = function()...end` (60 remaining instances).
                if old == "self" && new_name != "self" {
                    self.current_names.insert(reg, new_name.to_string());
                    return (WriteKind::Shadow, new_name.to_string());
                }
                // This write gives the register its OWN value, so it is no
                // longer an alias of the outer upvalue.
                let was_alias = self.alias_regs.remove(&reg);
                // Names differ.  Only shadow when BOTH are semantic
                // (non-`v\d+`).  Otherwise reuse the old name to avoid
                // counter-bump churn.
                //
                // Shadowing is NOT optional here.  Reusing one name for every
                // lifetime of a register (`F = f1; F = f2`) leaves the two
                // assignments as plain overwrites, and dead-store elimination
                // then correctly deletes the first -- but the closure it built
                // was reachable through the register earlier in the block, and
                // taking the body with it dropped 3,336 of 3,536 protos
                // (measured: `function` keyword count 3,568 -> 200).  A fresh
                // name per lifetime is what keeps those bodies alive.
                if is_semantic_local_name(&old) && is_semantic_local_name(new_name) {
                    self.current_names.insert(reg, new_name.to_string());
                    (WriteKind::Shadow, new_name.to_string())
                } else if was_alias {
                    // `old` was a read-only alias of a captured upvalue (e.g.
                    // `v9`), not this register's own binding. Reusing it here
                    // would emit `v9 = <call result>` and collide with the real
                    // upvalue `v9`, which dead-store elimination then unpicks by
                    // deleting real writes (a corpus case). Mint the fresh name instead.
                    self.current_names.insert(reg, new_name.to_string());
                    (WriteKind::Shadow, new_name.to_string())
                } else {
                    // (fallthrough) keep the existing binding
                    (WriteKind::Reassign, old)
                }
            }
            None => {
                // Declared without a recorded name (e.g., via pre_declare
                // for CALL multi-return, GETVARARGS, or loop-var seeding).
                // Record the new name and treat as plain reassignment —
                // we can't know whether the pre-declared name matched.
                self.current_names.insert(reg, new_name.to_string());
                (WriteKind::Reassign, new_name.to_string())
            }
        }
    }
}

/// Phase B0.49 helper — is `name` a non-generic, semantic identifier that
/// looks like a hint-derived name (import / global / debug_name / field /
/// call result), as opposed to the `v\d+` fallback that `synthesize_name`
/// produces when no hint applies?
///
/// Rules (conservative — only treats pure `v\d+` as generic):
///   * `name` starts with `v` followed by one or more ASCII digits and nothing
///     else  →  generic  →  returns `false`.
///   * Anything else (including `v1x`, `value`, `verb2`, `arg1`, `self`,
///     `tbl`, `arithmetic`, …)  →  semantic  →  returns `true`.
///
/// We intentionally do NOT blacklist other common fallbacks (e.g., `arg1`,
/// `tbl`, `fn`, `i`, `k`, `v`) because those are still hint-derived and
/// plausibly intentional; shadowing between them is safe and preserves the
/// narrative of the bytecode (e.g., a `fn` closure register getting
/// re-declared as a `tbl` is semantically meaningful).
pub(super) fn is_semantic_local_name(name: &str) -> bool {
    if name.is_empty() { return false; }
    let mut chars = name.chars();
    if chars.next() != Some('v') { return true; }
    // Must be at least one digit after the leading 'v' to qualify as generic
    let mut saw_digit = false;
    for c in chars {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else {
            return true; // Non-digit after 'v' → semantic (e.g., "value")
        }
    }
    // "v" alone with no digits is semantic (loop var fallback); "v12" is generic
    !saw_digit
}
