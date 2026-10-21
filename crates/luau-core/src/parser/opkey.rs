//! Exact recovery of the client's multiplicative opcode scramble.
//!
//! A live Roblox client does not ship a permuted opcode *table* — it stores
//! every instruction's opcode byte multiplied by a per-build constant:
//!
//! ```text
//!     stored = canonical * encode_key      (mod 256, encode_key odd)
//! ```
//!
//! Multiplication by an odd number modulo 256 is a bijection, so the canonical
//! byte is recovered exactly:
//!
//! ```text
//!     canonical = stored * inverse(encode_key)   (mod 256)
//! ```
//!
//! That turns an open-ended inference problem into a 128-way search with a
//! decidable winner: for each odd multiplier, walk every proto with the
//! canonical instruction-length table and check the stream lands exactly on the
//! end. A wrong multiplier desynchronises almost immediately (one mis-sized
//! instruction shifts everything after it), so on a real chunk the correct key
//! chains 100% of protos and every rival chains a few percent.
//!
//! Measured on a 1.8 MB live-clientscript chunk (1,610 protos):
//!
//! | mult | protos chaining exactly | bad jumps |
//! |------|------------------------|-----------|
//! | 203  | **1610 / 1610**         | 0         |
//! | 87   | 116 / 1610             | 57        |
//! | 195  | 94 / 1610              | 192       |
//!
//! Confirmations beyond the length chain: every proto must end in a terminator
//! (RETURN / JUMP), jump targets must stay inside the proto, and constant
//! references must index real constants. Those are checked here too, so a key
//! is accepted only when it is structurally beyond doubt — otherwise this
//! module declines and the caller falls back to the existing detectors.

use crate::parser::opcodes::LuauOpcode;
use crate::parser::types::{insn_d, insn_op, Chunk};

/// Outcome of a successful recovery.
pub struct RecoveredKey {
    /// Multiplier that maps a stored byte to its canonical opcode number.
    pub mult: u8,
    /// The client's encode key (`inverse(mult)`), for reporting.
    pub encode_key: u8,
    /// How many protos chained exactly (equals the total on success).
    pub exact_protos: usize,
    pub total_protos: usize,
}

impl RecoveredKey {
    /// Build the byte -> canonical-opcode map this key describes.
    pub fn to_map(&self) -> [u8; 256] {
        let mut map = [255u8; 256];
        let mut b = 0usize;
        while b < 256 {
            let canon = ((b as u16 * self.mult as u16) & 0xFF) as u8;
            // Only bytes that land on a real instruction are mapped; anything
            // else stays unmapped so it surfaces as an unresolved instruction
            // rather than being silently invented.
            if canon <= MAX_CANONICAL_OP {
                map[b] = canon;
            }
            b += 1;
        }
        map
    }
}

/// Highest opcode number in the canonical instruction set this build decodes.
/// 92 == FINCONSTRUCT; the enum parks its legacy placeholders far above this.
const MAX_CANONICAL_OP: u8 = 92;

/// Worst offender count tolerated for a key to be accepted. A correct key
/// scores 0 on a real chunk; the margin exists for exotic-but-legal chunks
/// (e.g. a jump deliberately landing one past the end).
const MAX_OFFENDERS: usize = 0;

struct Score {
    exact_protos: usize,
    bad_jumps: usize,
    bad_consts: usize,
}

fn score(chunk: &Chunk, mult: u8) -> Score {
    let mut exact_protos = 0usize;
    let mut bad_jumps = 0usize;
    let mut bad_consts = 0usize;

    for proto in &chunk.protos {
        let code = &proto.code;
        let nconsts = proto.constants.len();
        let mut pc = 0usize;
        let mut ok = true;
        let mut last_op: Option<LuauOpcode> = None;

        while pc < code.len() {
            let word = code[pc];
            let canon = ((insn_op(word) as u16 * mult as u16) & 0xFF) as u8;
            if canon > MAX_CANONICAL_OP {
                ok = false;
                break;
            }
            let op = LuauOpcode::from_u8(canon);

            // Constant references must index a real constant.
            match op {
                LuauOpcode::LoadK | LuauOpcode::DupTable => {
                    let d = insn_d(word);
                    if d < 0 || d as usize >= nconsts {
                        bad_consts += 1;
                    }
                }
                LuauOpcode::AddK
                | LuauOpcode::SubK
                | LuauOpcode::MulK
                | LuauOpcode::DivK
                | LuauOpcode::ModK
                | LuauOpcode::PowK
                | LuauOpcode::IDivK
                | LuauOpcode::AndK
                | LuauOpcode::OrK
                | LuauOpcode::SubRK
                | LuauOpcode::DivRK
                | LuauOpcode::FastCall2K => {
                    let c = ((word >> 24) & 0xFF) as usize;
                    if c >= nconsts {
                        bad_consts += 1;
                    }
                }
                LuauOpcode::LoadKX => {
                    if let Some(&aux) = code.get(pc + 1) {
                        if (aux as usize) >= nconsts {
                            bad_consts += 1;
                        }
                    }
                }
                LuauOpcode::JumpXEqKN | LuauOpcode::JumpXEqKS => {
                    if let Some(&aux) = code.get(pc + 1) {
                        // AUX packs the constant index in bits 0..30 and the
                        // "compare inverted" flag in bit 31. Masking matters:
                        // an unmasked 0x80000000 reads as a 2-billion index.
                        let kidx = (aux & 0x7FFF_FFFF) as usize;
                        if kidx >= nconsts {
                            bad_consts += 1;
                        }
                    }
                }
                _ => {}
            }

            // Relative jumps must stay inside the code array.
            let is_d_jump = matches!(
                op,
                LuauOpcode::Jump
                    | LuauOpcode::JumpBack
                    | LuauOpcode::JumpIf
                    | LuauOpcode::JumpIfNot
                    | LuauOpcode::JumpIfEq
                    | LuauOpcode::JumpIfLE
                    | LuauOpcode::JumpIfLT
                    | LuauOpcode::JumpIfNotEq
                    | LuauOpcode::JumpIfNotLE
                    | LuauOpcode::JumpIfNotLT
                    | LuauOpcode::JumpXEqKNil
                    | LuauOpcode::JumpXEqKB
                    | LuauOpcode::JumpXEqKN
                    | LuauOpcode::JumpXEqKS
                    | LuauOpcode::ForNPrep
                    | LuauOpcode::ForNLoop
                    | LuauOpcode::ForGLoop
                    | LuauOpcode::ForGPrep
                    | LuauOpcode::ForGPrepINext
                    | LuauOpcode::ForGPrepNext
            );
            if is_d_jump {
                let t = pc as i64 + 1 + insn_d(word) as i64;
                if t < 0 || t > code.len() as i64 {
                    bad_jumps += 1;
                }
            }

            last_op = Some(op);
            pc += if op.has_aux() { 2 } else { 1 };
        }

        if ok && pc == code.len() {
            exact_protos += 1;
        }
        // A function body ends by returning or by jumping out of the way.
        if !matches!(last_op, Some(LuauOpcode::Return) | Some(LuauOpcode::Jump)) {
            bad_jumps += 1;
        }
    }

    Score { exact_protos, bad_jumps, bad_consts }
}

/// Recover the multiplicative opcode key, or `None` when no candidate is
/// structurally beyond doubt (in which case callers keep using the existing
/// detectors).
pub fn recover(chunk: &Chunk) -> Option<RecoveredKey> {
    if chunk.protos.is_empty() {
        return None;
    }

    // Nothing to recover only when identity is *provably* right: every proto
    // chains exactly AND no structural check fires. (`is_canonical_luau` alone
    // is too weak here — a small encode key can map one valid instruction
    // stream onto another that also chains, e.g. key 3 sends LOADK 5 to 15.)
    let identity = score(chunk, 1);
    let identity_perfect = identity.exact_protos == chunk.protos.len()
        && identity.bad_jumps <= MAX_OFFENDERS
        && identity.bad_consts <= MAX_OFFENDERS;
    if identity_perfect {
        return None;
    }

    let total = chunk.protos.len();
    // i64, not usize: an offending candidate scores negative and must sort
    // BELOW a clean one. (With usize the subtraction wrapped and a wholly
    // invalid multiplier outranked the correct one.)
    let mut best: Option<(i64, u8, Score)> = None;

    for mult in (1u16..256).step_by(2) {
        let mult = mult as u8;
        let s = score(chunk, mult);
        // Rank by exact chain first, then by fewest structural offences.
        let rank = s.exact_protos as i64 * 1024
            - s.bad_jumps.min(1000) as i64
            - s.bad_consts.min(1000) as i64;
        let better = match &best {
            None => true,
            Some((br, _, _)) => rank > *br,
        };
        if better {
            best = Some((rank, mult, s));
        }
    }

    let (_, mult, s) = best?;
    if std::env::var("OPKEY_DEBUG").is_ok() {
        eprintln!(
            "opkey: best mult={} exact={}/{} bad_jumps={} bad_consts={}",
            mult, s.exact_protos, total, s.bad_jumps, s.bad_consts
        );
    }

    // Acceptance gate: EVERY proto must chain exactly, with no structural
    // offences. On real chunks the true key scores 0 here and every rival
    // scores in the hundreds, so this is a decision, not a threshold.
    if s.exact_protos != total || s.bad_jumps > MAX_OFFENDERS || s.bad_consts > MAX_OFFENDERS {
        return None;
    }

    let encode_key = mod_inverse_odd(mult)?;
    Some(RecoveredKey {
        mult,
        encode_key,
        exact_protos: s.exact_protos,
        total_protos: total,
    })
}

/// Modular inverse of an odd number modulo 256 (always exists: the odd numbers
/// are exactly the units of Z/256).
fn mod_inverse_odd(a: u8) -> Option<u8> {
    if a % 2 == 0 {
        return None;
    }
    let mut x: u32 = 1;
    for _ in 0..8 {
        x = (x * (2 - (a as u32 * x % 256))) % 256;
    }
    Some(x as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::types::{Constant, Proto};

    fn proto(code: Vec<u32>, consts: usize) -> Proto {
        Proto {
            max_stack_size: 8,
            num_params: 0,
            num_upvalues: 0,
            is_vararg: false,
            flags: 0,
            typeinfo: None,
            code,
            constants: vec![Constant::Number(1.0); consts],
            child_protos: Vec::new(),
            line_defined: 1,
            debug_name: None,
            line_info: None,
            debug_info: None,
        }
    }

    fn chunk(protos: Vec<Proto>) -> Chunk {
        Chunk {
            version: 9,
            types_version: 3,
            strings: Vec::new(),
            protos,
            main_proto: 0,
        }
    }

    /// A chunk shaped like real code: many protos, with instructions whose
    /// operands are range-checked (LOADK into a small constant table,
    /// GETTABLEKS with an AUX constant index, CALL, RETURN). One proto with five
    /// instructions cannot pin a key out of 128 — every candidate chains — so the
    /// fixture mirrors the statistical evidence a real chunk provides.
    fn constraining_chunk(encode_key: u16) -> Chunk {
        let mut protos = Vec::new();
        for p in 0..24u32 {
            let nk = 4 + (p as usize % 5);
            let canonical: Vec<u32> = vec![
                4 | (0 << 8) | (((p % 7 + 1) as u32) << 16), // LOADN R0 k
                5 | (1 << 8) | (((p as u32) % nk as u32) << 16), // LOADK R1 K(p%nk)
                12 | (2 << 8),                    // GETIMPORT R2
                0x0000_0000,                      // AUX (packed import)
                15 | (3 << 8) | (2 << 16),        // GETTABLEKS R3 R2 aux
                (p as u32) % nk as u32,           // AUX const index
                21 | (2 << 8) | (2 << 16),        // CALL R2 1 1
                22 | (0 << 8) | (2 << 16),        // RETURN R0 1
            ];
            // Scramble opcode bytes, leave AUX words untouched.
            let mut stored = canonical.clone();
            let mut pc = 0usize;
            while pc < canonical.len() {
                let w = canonical[pc];
                let op = insn_op(w);
                let opnum = LuauOpcode::from_u8(op);
                stored[pc] = (w & 0xFFFF_FF00)
                    | (((op as u16 * encode_key) & 0xFF) as u32);
                pc += if opnum.has_aux() { 2 } else { 1 };
            }
            protos.push(proto(stored, nk));
        }
        chunk(protos)
    }

    #[test]
    fn multiplicative_key_is_recovered_exactly() {
        for key in [227u16, 3, 5, 197, 101, 255] {
            let c = constraining_chunk(key);

            // The claim is stronger than "recover found something": the true key
            // must be the UNIQUE multiplier that chains every proto with zero
            // structural offences, or the acceptance gate means nothing.
            let mult = mod_inverse_odd(key as u8).unwrap();
            let winners: Vec<u8> = (1u16..256)
                .step_by(2)
                .map(|m| m as u8)
                .filter(|&m| {
                    let s = score(&c, m);
                    s.exact_protos == c.protos.len()
                        && s.bad_jumps == 0
                        && s.bad_consts == 0
                })
                .collect();
            assert_eq!(
                winners,
                vec![mult],
                "encode_key {} must have exactly one structurally valid multiplier",
                key
            );

            let rec = recover(&c).expect("key must be recoverable");
            assert_eq!(rec.mult, mult, "wrong multiplier for key {}", key);
            assert_eq!(rec.encode_key, key as u8, "wrong reported encode key");
            assert_eq!(rec.exact_protos, rec.total_protos);
            assert_eq!(rec.to_map()[0], 0, "opcode 0 stays 0 under any key");
        }
    }

    #[test]
    fn canonical_chunks_are_left_alone() {
        let c = chunk(vec![proto(vec![4 | (1 << 16), 22 | (0 << 8) | (1 << 16)], 2)]);
        assert!(recover(&c).is_none(), "canonical input must be left alone");
    }
}

