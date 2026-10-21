//! Records, per chunk, every site that minted a decompiler-generated name
//! (`vN`, `upval_N`, `cap_N`), so the largest remaining defect class can be
//! attributed to a call site instead of described as an absence.
//!
//! Gated on `LUAU_MINT_TRACE`; a no-op when unset, and verified
//! behaviour-neutral against the compile gate (621/628), the semantic checks
//! (266/628) and CoreScript ground truth (9/9).
//!
//! See `docs/GENERATED_NAME_ORIGINS.md` for the recipe and the measurement it
//! produced. Kept in the tree rather than reapplied as a patch because the
//! defect it measures is open, and the next attempt on it starts here.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

thread_local! {
    static MINTS: RefCell<BTreeMap<String, BTreeSet<&'static str>>> =
        RefCell::new(BTreeMap::new());
}

pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LUAU_MINT_TRACE").is_ok())
}

/// A name can be minted at several sites within one chunk, so sites accumulate
/// into a set rather than overwriting -- the combination is itself a signal.
pub fn note(site: &'static str, name: &str) {
    if !enabled() {
        return;
    }
    MINTS.with(|m| {
        m.borrow_mut()
            .entry(name.to_string())
            .or_default()
            .insert(site);
    });
}

pub fn sites_for(name: &str) -> String {
    MINTS.with(|m| match m.borrow().get(name) {
        Some(s) => s.iter().cloned().collect::<Vec<_>>().join("+"),
        None => "NONE".to_string(),
    })
}

thread_local! {
    /// Register index -> the site that last set it to `RegVal::Unknown`.
    static UNKNOWN_ORIGIN: RefCell<BTreeMap<usize, &'static str>> =
        RefCell::new(BTreeMap::new());
}

/// Record WHY a register went `Unknown`.
///
/// `note` is keyed by name, so it can say a generated name came from
/// `reg_expr`'s Unknown fallback but not which of the producers emptied that
/// register. That distinction is the whole question: the fallback is one call
/// site with several upstream causes, and some of them are CORRECT -- the
/// registers above a call's result are genuinely cleared and genuinely hold
/// nothing. Suppressing the declaration for those would trade a visible defect
/// for a silent wrong value.
///
/// Keyed by register rather than by name because the name does not exist yet at
/// the moment the register is cleared.
pub fn note_unknown(site: &'static str, reg: usize) {
    if !enabled() {
        return;
    }
    UNKNOWN_ORIGIN.with(|m| {
        m.borrow_mut().insert(reg, site);
    });
}

/// The site that last cleared `reg`, or `UNK_ORIGIN_NONE` if nothing did --
/// which means the register was never written in this proto at all.
pub fn unknown_origin(reg: usize) -> &'static str {
    UNKNOWN_ORIGIN.with(|m| m.borrow().get(&reg).copied().unwrap_or("UNK_ORIGIN_NONE"))
}

/// Register indices are per-proto, so provenance must be dropped when a new
/// proto's register file is created or entries leak across protos.
pub fn clear_unknown_origins() {
    if !enabled() {
        return;
    }
    UNKNOWN_ORIGIN.with(|m| m.borrow_mut().clear());
}

pub fn clear() {
    MINTS.with(|m| m.borrow_mut().clear());
    UNKNOWN_ORIGIN.with(|m| m.borrow_mut().clear());
}

// ── Minted-but-never-declared audit ─────────────────────────────────────
//
// `LUAU_UNDECL_TRACE=1` prints every name this proto MINTED (asked the
// namer for) that no `local` in the same proto binds. Those are precisely the
// identifiers the emitted source would read as globals — the "SSA variant
// without a declaration" defect reported against the client chunk
// (790 names / 3,202 sites). Printing (proto, reg, pc) alongside pins the
// call site instead of the symptom.

thread_local! {
    static MINTED: RefCell<Vec<(usize, u8, usize, String)>> = RefCell::new(Vec::new());
    static DECLARED: RefCell<BTreeSet<String>> = RefCell::new(BTreeSet::new());
}

pub fn undecl_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LUAU_UNDECL_TRACE").is_ok())
}

pub fn record_mint(proto: usize, reg: u8, pc: usize, name: &str) {
    if !undecl_enabled() {
        return;
    }
    MINTED.with(|m| m.borrow_mut().push((proto, reg, pc, name.to_string())));
}

pub fn record_decl(name: &str) {
    if !undecl_enabled() {
        return;
    }
    DECLARED.with(|d| {
        d.borrow_mut().insert(name.to_string());
    });
}

/// Emit the diff for one proto and clear the buffers.
pub fn flush_undecl(proto: usize, label: &str) {
    if !undecl_enabled() {
        return;
    }
    let minted = MINTED.with(|m| m.borrow().clone());
    let declared = DECLARED.with(|d| d.borrow().clone());
    for (p, reg, pc, name) in minted {
        if p == proto && !declared.contains(&name) {
            eprintln!("UNDECL\t{}@{}\treg={}\tpc={}\t{}", label, proto, reg, pc, name);
        }
    }
    MINTED.with(|m| m.borrow_mut().clear());
    DECLARED.with(|d| d.borrow_mut().clear());
}
