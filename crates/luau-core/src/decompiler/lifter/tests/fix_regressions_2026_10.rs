//! Regression locks for the 2026-10 correctness fixes.
//!
//! Each test pins a defect that produced source which *parsed and looked
//! right* while computing a different program. They are bytecode-level
//! (except the pure-AST ones) so they exercise the same path a real chunk
//! takes: parser -> structurer -> lifter -> post-passes -> emitter.
//!
//! 1. `parallel_store_swap_is_one_statement`
//!       `t[i], t[j] = t[j], t[i]` used to lift to two sequential stores,
//!       turning a swap into two overlapping copies ({20,20} instead of
//!       {20,10}).
//!
//! 2. `modulo_constant_fold_is_floored`
//!       Rust's `%` is a truncated remainder; Luau's is floored. Folding
//!       `-7 % 3` gave -1 where the program computes 2.
//!
//! 3. `recursive_local_function_is_never_inlined`
//!       A self-recursive `local function` inlined into its call site loses
//!       its binding, so the recursive call becomes a nil global.

use crate::ast::{BinOp, Expr, Stat};
use crate::parser::opcodes::LuauOpcode;
use crate::decompiler::{decompile_proto, DecompileContext};
use crate::parser::types::{Chunk, Proto};

// Canonical (non-shuffled) Luau opcode bytes.
const OP_LOADN: u8 = LuauOpcode::LoadN as u8;
const OP_GETTABLEN: u8 = LuauOpcode::GetTableN as u8;
const OP_SETTABLEN: u8 = LuauOpcode::SetTableN as u8;
const OP_NEWTABLE: u8 = LuauOpcode::NewTable as u8;
const OP_SETLIST: u8 = LuauOpcode::SetList as u8;
const OP_RETURN: u8 = LuauOpcode::Return as u8;

fn insn_ad(op: u8, a: u8, d: i16) -> u32 {
    let du = d as u16 as u32;
    (op as u32) | ((a as u32) << 8) | (du << 16)
}

fn insn_abc(op: u8, a: u8, b: u8, c: u8) -> u32 {
    (op as u32) | ((a as u32) << 8) | ((b as u32) << 16) | ((c as u32) << 24)
}

fn make_chunk(proto: Proto) -> Chunk {
    Chunk {
        version: 6,
        types_version: 0,
        strings: vec!["table".to_string()],
        protos: vec![proto],
        main_proto: 0,
    }
}

fn make_proto(code: Vec<u32>, num_params: u8, max_stack: u8, name: &str) -> Proto {
    Proto {
        max_stack_size: max_stack,
        num_params,
        num_upvalues: 0,
        is_vararg: false,
        flags: 0,
        typeinfo: None,
        code,
        constants: Vec::new(),
        child_protos: Vec::new(),
        line_defined: 1,
        debug_name: Some(name.to_string()),
        line_info: None,
        debug_info: None,
    }
}

/// `local t = {10, 20, 30}; t[1], t[2] = t[2], t[1]; return t`
///
/// The compiler reads both slots into temporaries *before* storing either,
/// which is the only way a swap can work:
///
///     NEWTABLE  R0
///     SETLIST   R0 R1 3
///     GETTABLEN R1 R0 1     ; R1 = t[2]   (C is 0-based)
///     GETTABLEN R2 R0 0     ; R2 = t[1]
///     SETTABLEN R1 R0 0     ; t[1] = R1
///     SETTABLEN R2 R0 1     ; t[2] = R2
///
/// Lifting the stores one at a time yields
///     t[1] = t[2]; t[2] = t[1]
/// which writes {20, 20}.
fn build_swap_chunk() -> Chunk {
    let code = vec![
        insn_abc(OP_NEWTABLE, 0, 0, 0),
        0, // NEWTABLE AUX: hash size hint
        insn_ad(OP_LOADN, 1, 10),
        insn_ad(OP_LOADN, 2, 20),
        insn_ad(OP_LOADN, 3, 30),
        insn_abc(OP_SETLIST, 0, 1, 3),
        1, // SETLIST AUX: table index offset
        insn_abc(OP_GETTABLEN, 1, 0, 1), // t[2]
        insn_abc(OP_GETTABLEN, 2, 0, 0), // t[1]
        insn_abc(OP_SETTABLEN, 1, 0, 0), // t[1] = R1
        insn_abc(OP_SETTABLEN, 2, 0, 1), // t[2] = R2
        insn_abc(OP_RETURN, 0, 1, 0),
    ];
    make_chunk(make_proto(code, 0, 5, "swap_parallel"))
}

#[test]
fn parallel_store_swap_is_one_statement() {
    let chunk = build_swap_chunk();
    let mut ctx = DecompileContext::new(&chunk);
    let source = decompile_proto(&mut ctx, &chunk.protos[0], 0, 0);

    // The swap must survive as ONE simultaneous assignment. Two separate
    // statements are a different program even though both parse.
    let has_multi_target = source
        .lines()
        .any(|l| l.contains(',') && l.contains('=') && l.matches('[').count() >= 2);
    assert!(
        has_multi_target,
        "parallel store must lift to a single multi-target assignment, got:\n{}",
        source
    );

    // And the RHS must read the two slots, not the same one twice.
    assert!(
        !source.contains("[1] = tbl[1]") && !source.contains("[1] = v1[1]"),
        "swap collapsed into a self-assignment:\n{}",
        source
    );
}

/// `-7 % 3` is 2 in Luau (floored) and -1 in Rust's `%` (truncated).
#[test]
fn modulo_constant_fold_is_floored() {
    let folded = super::super::simplify_expr(&Expr::BinOp {
        left: Box::new(Expr::Number(-7.0)),
        op: BinOp::Mod,
        right: Box::new(Expr::Number(3.0)),
    });
    match folded {
        Expr::Number(n) => assert!(
            (n - 2.0).abs() < 1e-12,
            "floored modulo: -7 % 3 must be 2, got {}",
            n
        ),
        other => panic!("expected a folded Number, got {:?}", other),
    }

    // Positive operands are unchanged.
    let folded = super::super::simplify_expr(&Expr::BinOp {
        left: Box::new(Expr::Number(7.0)),
        op: BinOp::Mod,
        right: Box::new(Expr::Number(3.0)),
    });
    assert!(matches!(folded, Expr::Number(n) if (n - 1.0).abs() < 1e-12));
}

/// A self-recursive function must keep its binding: the body needs the name.
#[test]
fn recursive_local_function_is_never_inlined() {
    let mut stmts = vec![
        Stat::Local {
            names: vec!["fn1".to_string()],
            values: vec![Expr::Function {
                params: vec!["n".to_string()],
                is_vararg: false,
                body: vec![
                    Stat::If {
                        condition: Expr::BinOp {
                            left: Box::new(Expr::Name("n".to_string())),
                            op: BinOp::LE,
                            right: Box::new(Expr::Number(1.0)),
                        },
                        then_body: vec![Stat::Return { values: vec![Expr::Number(1.0)] }],
                        elseif_clauses: vec![],
                        else_body: None,
                    },
                    Stat::Return {
                        values: vec![Expr::BinOp {
                            left: Box::new(Expr::Name("n".to_string())),
                            op: BinOp::Mul,
                            right: Box::new(Expr::Call {
                                func: Box::new(Expr::Name("fn1".to_string())),
                                args: vec![Expr::BinOp {
                                    left: Box::new(Expr::Name("n".to_string())),
                                    op: BinOp::Sub,
                                    right: Box::new(Expr::Number(1.0)),
                                }],
                            }),
                        }],
                    },
                ],
            }],
        },
        Stat::ExprStat(Expr::Call {
            func: Box::new(Expr::Name("print".to_string())),
            args: vec![Expr::Call {
                func: Box::new(Expr::Name("fn1".to_string())),
                args: vec![Expr::Number(5.0)],
            }],
        }),
    ];

    super::super::post_passes::inline_single_use_temps(&mut stmts);

    assert!(
        matches!(&stmts[0], Stat::Local { names, .. } if names == &vec!["fn1".to_string()]),
        "self-recursive function was inlined, deleting the binding it calls:\n{:#?}",
        stmts
    );
}
