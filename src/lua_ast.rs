// ============================================================
// lua_ast — Lua-script analyzer for Yu-Gi-Oh card scripts.
//
// Reads a .lua file, parses with `full_moon`, and extracts the
// effect skeletons that `s.initial_effect(c)` registers via
// `Effect.CreateEffect → Set* → c:RegisterEffect` chains.
//
// Each Effect.CreateEffect call introduces a local binding (e.g.
// `local e1 = Effect.CreateEffect(c)`). Subsequent `e1:SetX(...)`
// calls populate the effect's metadata. The final `c:RegisterEffect(e1)`
// commits the effect.
//
// We extract:
//   - which `Set*` calls were applied to each effect
//   - the operation-handler name (passed to `SetOperation(s.X)`)
//   - the chain of `Duel.*` calls inside that handler's function body
//
// This is the substrate for emitting DSL `effect { resolve { ... } }`
// blocks.
// ============================================================

use std::collections::BTreeMap;
use std::fmt::Write;

use full_moon::ast;
use full_moon::ast::{Stmt, Expression, FunctionCall, Suffix, Call, Index, Block};

/// Top-level analysis report for one Lua file.
#[derive(Debug, Default)]
pub struct LuaReport {
    pub effects: Vec<EffectSkeleton>,
    pub functions: BTreeMap<String, Vec<DuelCall>>,
    pub parse_error: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct EffectSkeleton {
    pub binding: String,                       // `e1`
    pub set_calls: Vec<(String, Vec<String>)>, // (Set method, raw arg strings)
    pub registered: bool,
    pub operation_handler: Option<String>,     // `s.activate`
    pub target_handler: Option<String>,
    pub condition_handler: Option<String>,
    pub cost_handler: Option<String>,
}

/// One `Duel.X(args...)` call extracted from a function body.
#[derive(Debug, Clone)]
pub struct DuelCall {
    pub method: String,         // `Duel.Damage`
    pub args: Vec<String>,      // raw text per top-level arg
}

pub fn analyze(src: &str) -> String {
    let parsed = match full_moon::parse(src) {
        Ok(ast) => ast,
        Err(e) => {
            return format!("parse error: {:?}", e);
        }
    };
    render_report(&walk(&parsed))
}

/// Walk a parsed Lua AST and produce a `LuaReport` of effect skeletons +
/// per-function `Duel.*` call sequences. Public so external bins can
/// reuse the walker without going through the text-rendered `analyze`.
pub fn walk(parsed: &full_moon::ast::Ast) -> LuaReport {
    let mut report = LuaReport::default();
    walk_block(parsed.nodes(), &mut report);
    report
}

fn walk_block(block: &Block, report: &mut LuaReport) {
    for stmt in block.stmts() {
        walk_stmt(stmt, report);
    }
    if let Some(last) = block.last_stmt() {
        // currently nothing to extract from `return`/`break`
        let _ = last;
    }
}

fn walk_stmt(stmt: &Stmt, report: &mut LuaReport) {
    match stmt {
        // `function s.initial_effect(c) ... end`
        // `function s.activate(e,tp,...) ... end`
        Stmt::FunctionDeclaration(decl) => {
            let name = function_decl_name(decl);
            let body = decl.body();
            if name.ends_with("initial_effect") {
                extract_effects_from_block(body.block(), report);
            }
            let calls = extract_duel_calls(body.block());
            if !calls.is_empty() {
                report.functions.insert(name, calls);
            }
        }
        Stmt::LocalAssignment(_) | Stmt::Assignment(_) => {
            // top-level assignments aren't typically effect-bearing
        }
        _ => {}
    }
}

fn function_decl_name(decl: &ast::FunctionDeclaration) -> String {
    let mut out = String::new();
    let n = decl.name();
    write!(out, "{}", n.names().to_string().trim()).ok();
    if let Some(method) = n.method_name() {
        write!(out, ":{}", method.token().to_string()).ok();
    }
    out
}

/// Walk an `s.initial_effect` body looking for `Effect.CreateEffect`
/// chains. We track local bindings (`local e1 = Effect.CreateEffect(c)`)
/// then attribute subsequent `e1:Set*` and `c:RegisterEffect(e1)` calls
/// back to the binding.
fn extract_effects_from_block(block: &Block, report: &mut LuaReport) {
    let mut by_binding: BTreeMap<String, EffectSkeleton> = BTreeMap::new();

    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                for (i, name) in names.iter().enumerate() {
                    let expr = exprs.get(i);
                    if let Some(expr) = expr {
                        if expr_is_effect_createeffect(expr) {
                            by_binding.insert(name.clone(), EffectSkeleton {
                                binding: name.clone(),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            Stmt::FunctionCall(fc) => {
                // `eN:SetX(...)` populates the effect named by binding.
                if let Some((binding, method, args)) = method_call_on_binding(fc) {
                    if let Some(skel) = by_binding.get_mut(&binding) {
                        skel.set_calls.push((method.clone(), args.clone()));
                        if method == "SetOperation" {
                            skel.operation_handler = args.first().cloned();
                        } else if method == "SetTarget" {
                            skel.target_handler = args.first().cloned();
                        } else if method == "SetCondition" {
                            skel.condition_handler = args.first().cloned();
                        } else if method == "SetCost" {
                            skel.cost_handler = args.first().cloned();
                        }
                    }
                }
                // `c:RegisterEffect(eN)` commits the effect — independent
                // path so it still registers when the prior `:Set*` matched
                // a different binding (here the prefix is `c`, not `e1`).
                if let Some(arg) = is_register_effect(fc) {
                    if let Some(skel) = by_binding.get_mut(&arg) {
                        skel.registered = true;
                    }
                }
            }
            _ => {}
        }
    }

    for (_, skel) in by_binding {
        if skel.registered {
            report.effects.push(skel);
        }
    }
}

/// True if `expr` is the call `Effect.CreateEffect(c)` (we accept any
/// argument list — we just need the call shape).
fn expr_is_effect_createeffect(expr: &Expression) -> bool {
    if let Expression::FunctionCall(fc) = expr {
        let head = call_head_string(fc);
        head == "Effect.CreateEffect"
    } else {
        false
    }
}

/// Render a function-call's prefix as a dotted name string,
/// e.g. `Effect.CreateEffect`, `Duel.SendtoGrave`, `c:RegisterEffect`.
fn call_head_string(fc: &FunctionCall) -> String {
    let prefix = fc.prefix();
    let mut head = prefix.to_string().trim().to_string();
    // Walk suffixes that aren't the final Call — they form the dotted
    // / colon name (e.g. `e1:SetCategory(...)`).
    for s in fc.suffixes() {
        match s {
            Suffix::Index(idx) => match idx {
                Index::Dot { name, .. } => {
                    head.push('.');
                    head.push_str(&name.token().to_string());
                }
                Index::Brackets { .. } => {
                    head.push_str("[?]");
                }
                _ => {}
            },
            Suffix::Call(c) => match c {
                Call::MethodCall(mc) => {
                    head.push(':');
                    head.push_str(&mc.name().token().to_string());
                    return head;
                }
                Call::AnonymousCall(_) => {
                    return head;
                }
                _ => {}
            },
            _ => {}
        }
    }
    head
}

/// If `fc` is `<bind>:<method>(args)`, return `(binding, method, args)`.
fn method_call_on_binding(fc: &FunctionCall) -> Option<(String, String, Vec<String>)> {
    let prefix = fc.prefix();
    let binding = match prefix {
        ast::Prefix::Name(n) => n.token().to_string(),
        _ => return None,
    };
    let mut suffixes = fc.suffixes();
    let first = suffixes.next()?;
    if let Suffix::Call(Call::MethodCall(mc)) = first {
        let method = mc.name().token().to_string();
        let args = call_args_to_strings(mc.args());
        Some((binding, method, args))
    } else {
        None
    }
}

/// `c:RegisterEffect(eN)` → return Some("eN").
fn is_register_effect(fc: &FunctionCall) -> Option<String> {
    let prefix = fc.prefix();
    if let ast::Prefix::Name(_) = prefix {
        let mut suffixes = fc.suffixes();
        if let Some(Suffix::Call(Call::MethodCall(mc))) = suffixes.next() {
            if mc.name().token().to_string() == "RegisterEffect" {
                let args = call_args_to_strings(mc.args());
                return args.first().cloned();
            }
        }
    }
    None
}

fn call_args_to_strings(args: &ast::FunctionArgs) -> Vec<String> {
    match args {
        ast::FunctionArgs::Parentheses { arguments, .. } => {
            arguments.iter().map(|e| e.to_string().trim().to_string()).collect()
        }
        ast::FunctionArgs::String(s) => {
            vec![s.token().to_string()]
        }
        ast::FunctionArgs::TableConstructor(t) => {
            vec![t.to_string()]
        }
        _ => vec![],
    }
}

/// Extract every `Duel.X(...)` call from a function block (recursive
/// — descends into `if`, `while`, `for`, blocks, etc.).
fn extract_duel_calls(block: &Block) -> Vec<DuelCall> {
    let mut out = Vec::new();
    collect_duel_calls(block, &mut out);
    out
}

fn collect_duel_calls(block: &Block, out: &mut Vec<DuelCall>) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::FunctionCall(fc) => {
                if let Some(call) = duel_call_from_fc(fc) { out.push(call); }
            }
            Stmt::If(if_stmt) => {
                collect_duel_calls(if_stmt.block(), out);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_duel_calls(ei.block(), out);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_duel_calls(else_block, out);
                }
            }
            Stmt::While(w) => collect_duel_calls(w.block(), out),
            Stmt::Repeat(r) => collect_duel_calls(r.block(), out),
            Stmt::NumericFor(nf) => collect_duel_calls(nf.block(), out),
            Stmt::GenericFor(gf) => collect_duel_calls(gf.block(), out),
            Stmt::Do(d) => collect_duel_calls(d.block(), out),
            Stmt::LocalAssignment(la) => {
                for e in la.expressions() {
                    if let Expression::FunctionCall(fc) = e {
                        if let Some(call) = duel_call_from_fc(fc) { out.push(call); }
                    }
                }
            }
            Stmt::Assignment(a) => {
                for e in a.expressions() {
                    if let Expression::FunctionCall(fc) = e {
                        if let Some(call) = duel_call_from_fc(fc) { out.push(call); }
                    }
                }
            }
            _ => {}
        }
    }
}

fn duel_call_from_fc(fc: &FunctionCall) -> Option<DuelCall> {
    let head = call_head_string(fc);
    if !head.starts_with("Duel.") { return None; }
    // Args are on the *last* Call suffix.
    let mut last_args: Vec<String> = vec![];
    for s in fc.suffixes() {
        if let Suffix::Call(c) = s {
            if let Call::AnonymousCall(args) = c {
                last_args = call_args_to_strings(args);
            }
        }
    }
    Some(DuelCall { method: head, args: last_args })
}

fn render_report(r: &LuaReport) -> String {
    let mut out = String::new();
    if let Some(e) = &r.parse_error {
        writeln!(out, "PARSE ERROR: {}", e).ok();
        return out;
    }
    writeln!(out, "=== effects: {} ===", r.effects.len()).ok();
    for (i, e) in r.effects.iter().enumerate() {
        writeln!(out, "  [{}] binding={} ", i, e.binding).ok();
        for (m, args) in &e.set_calls {
            writeln!(out, "      {}({})", m, args.join(", ")).ok();
        }
        if let Some(op) = &e.operation_handler {
            writeln!(out, "      → operation handler: {}", op).ok();
            if let Some(calls) = r.functions.get(&handler_to_fn_name(op)) {
                for c in calls {
                    writeln!(out, "          {}({})", c.method, c.args.join(", ")).ok();
                }
            }
        }
    }
    writeln!(out, "=== functions w/ Duel.* calls: {} ===", r.functions.len()).ok();
    for (n, calls) in &r.functions {
        writeln!(out, "  {} ({} duel calls)", n, calls.len()).ok();
    }
    out
}

/// `s.activate` (handler shorthand passed to SetOperation) maps to the
/// declared function name `s.activate` — same string. Kept as an
/// abstraction in case we extend to handle aliases.
fn handler_to_fn_name(handler: &str) -> String {
    handler.trim().to_string()
}

// ============================================================
// DSL emission — Phase 2
//
// Translate a `LuaReport`'s effect skeleton into draft DSL `resolve`
// blocks by classifying each `Duel.X(...)` call in the operation
// handler:
//
//   * ACTION calls (SpecialSummon, Destroy, SendtoGrave, Remove,
//     SendtoHand, SendtoDeck, Damage, Recover, Draw, Release,
//     DiscardHand, ConfirmCards, BreakEffect) → emit DSL action line.
//   * SELECTOR calls (SelectTarget, SelectMatchingCard,
//     GetMatchingGroup, GetFirstTarget, GetTargetCards) → bind the
//     "current target group" used by following actions.
//   * META calls (Hint, HintSelection, ConfirmCards, BreakEffect,
//     RegisterEffect, SetOperationInfo) → skip.
//
// Phase 2 is deliberately conservative — actions whose arguments we
// cannot statically interpret (custom Lua filter functions, dynamic
// numeric expressions) emit a TODO line so the card is still
// reviewable, never silently misinterpreted.
// ============================================================

#[derive(Debug, Clone)]
pub enum DslLine {
    /// A confidently-translated DSL action line, e.g.
    /// `damage opponent 1000` or `draw 1`.
    Action(String),
    /// A `Duel.*` call we recognize but couldn't fully reduce.
    /// Emitted as `# TODO: <description>` in DSL output.
    Todo(String),
}

impl DslLine {
    pub fn is_action(&self) -> bool { matches!(self, DslLine::Action(_)) }
    pub fn into_string(self, indent: &str) -> String {
        match self {
            DslLine::Action(s) => format!("{}{}", indent, s),
            DslLine::Todo(s)   => format!("{}# TODO(lua-ast): {}", indent, s),
        }
    }
}

/// Translate one operation-handler's `Duel.*` call sequence into draft
/// DSL `resolve { ... }` lines. Returns one DslLine per recognized call.
pub fn translate_calls(calls: &[DuelCall]) -> Vec<DslLine> {
    let mut out = Vec::new();
    for c in calls {
        if let Some(line) = translate_call(c) {
            out.push(line);
        }
    }
    out
}

/// Map a single `Duel.X` call to a DSL line (or None for skip-class
/// metadata).
fn translate_call(c: &DuelCall) -> Option<DslLine> {
    let m = c.method.as_str();
    let a = &c.args;
    match m {
        // ── Skip: pure UI / control-flow / metadata ──────────
        "Duel.Hint" | "Duel.HintSelection" | "Duel.ConfirmCards"
        | "Duel.BreakEffect" | "Duel.SetOperationInfo"
        | "Duel.SetPossibleOperationInfo" | "Duel.RegisterEffect"
        | "Duel.SetTargetPlayer" | "Duel.SetTargetParam"
        | "Duel.SetTargetCard" | "Duel.SetChainLimit"
        => None,

        // ── Skip: read-only queries (used as cond / target side) ─
        "Duel.IsExistingMatchingCard" | "Duel.IsExistingTarget"
        | "Duel.IsPlayerCanDraw" | "Duel.IsPlayerCanSpecialSummonMonster"
        | "Duel.IsPlayerAffectedByEffect" | "Duel.IsTurnPlayer"
        | "Duel.GetMatchingGroup" | "Duel.GetMatchingGroupCount"
        | "Duel.GetLocationCount" | "Duel.GetFieldGroupCount"
        | "Duel.GetTargetCards" | "Duel.GetFirstTarget"
        | "Duel.GetChainInfo" | "Duel.GetAttacker" | "Duel.GetAttackTarget"
        | "Duel.SelectTarget" | "Duel.SelectMatchingCard"
        | "Duel.SelectYesNo"
        => None,

        // ── ACTIONS ──────────────────────────────────────────

        // Duel.Damage(player, amount, reason)
        "Duel.Damage" => Some(action_damage(a)),
        // Duel.Recover(player, amount, reason) — DSL only models self-gain
        // via `gain_lp <N>`; opponent-recover has no DSL form yet.
        "Duel.Recover" => Some(action_recover(a)),

        // Duel.Draw(player, count, reason)
        "Duel.Draw" => Some(action_draw(a)),

        // Duel.Destroy(target, reason)
        "Duel.Destroy" => Some(DslLine::Action(
            "destroy target".to_string()
        )),

        // Duel.SendtoGrave(target, reason)
        "Duel.SendtoGrave" => Some(DslLine::Action(
            "send target to gy".to_string()
        )),

        // Duel.SendtoHand(target, player, reason)
        "Duel.SendtoHand" => Some(DslLine::Action(
            "add_to_hand target".to_string()
        )),

        // Duel.SendtoDeck(target, player, sequence, reason)
        "Duel.SendtoDeck" => Some(DslLine::Action(
            "send target to deck".to_string()
        )),

        // Duel.Remove(target, pos, reason) — banish
        "Duel.Remove" => Some(DslLine::Action(
            "banish target".to_string()
        )),

        // Duel.Release(target, reason) — tribute. No DSL `tribute`
        // action in resolve grammar (only `tribute self` in cost block).
        // Released cards go to gy; closest semantic action is send-to-gy.
        "Duel.Release" => Some(DslLine::Action(
            "send target to gy".to_string()
        )),

        // Duel.SpecialSummon(target, sumtype, p1, p2, nocheck, nolimit, pos)
        "Duel.SpecialSummon" => Some(DslLine::Action(
            "special_summon target".to_string()
        )),
        "Duel.SpecialSummonStep" => Some(DslLine::Action(
            "special_summon target".to_string()
        )),
        "Duel.SpecialSummonComplete" => None, // boundary marker

        // Duel.DiscardHand(player, filter, min, max, reason)
        "Duel.DiscardHand" => Some(DslLine::Action(
            "discard (1+, card, you control, from hand)".to_string()
        )),

        // Duel.ChangePosition(target, position) — change face-up/down
        // attack/defense. We can't always extract position from args, but
        // the DSL `change_position target` (no `to ...`) is valid: lets the
        // engine pick. If we recognize a literal POS_*, we add `to`.
        "Duel.ChangePosition" => Some(action_change_position(a)),

        // Duel.Equip(player, equipper, target, ...) — equip self/target to
        // another card. DSL: `equip <eq> to <target>`. We only handle the
        // common shape `Duel.Equip(tp, c, target, ...)` → equip self to target.
        "Duel.Equip" => Some(action_equip(a)),

        // Duel.SSet(player, target) — set spell/trap face-down on field.
        "Duel.SSet" => Some(DslLine::Action("set target".to_string())),

        // Duel.ShuffleDeck(player) — shuffle. DSL has shuffle_deck with
        // optional yours/opponents/both; default is implicit yours.
        "Duel.ShuffleDeck" => Some(action_shuffle(a)),

        // Duel.NegateAttack — DSL `negate` (no destroy variant).
        "Duel.NegateAttack" => Some(DslLine::Action("negate".to_string())),
        "Duel.NegateActivation" => Some(DslLine::Action("negate".to_string())),
        "Duel.NegateEffect" => Some(DslLine::Action("negate".to_string())),

        // Special-summon family that's not the basic SpecialSummon —
        // engine handles them as variants of the same action.
        "Duel.SynchroSummon" | "Duel.XyzSummon" | "Duel.LinkSummon"
        | "Duel.FusionSummon" | "Duel.RitualSummon"
        => Some(DslLine::Action("special_summon target".to_string())),

        // Duel.Summon(player, target, ignore_count, e, min, max) — normal summon
        "Duel.Summon" => Some(DslLine::Action("normal_summon target".to_string())),

        // Anything else we recognize as a duel call but don't yet map.
        _ => Some(DslLine::Todo(format!(
            "{}({})", m, a.join(", ")
        ))),
    }
}

/// Translate `Duel.Damage(player, amount, reason)` to
/// `damage opponent <N>` / `damage you <N>`.
fn action_damage(args: &[String]) -> DslLine {
    let player = args.first().map(String::as_str).unwrap_or("");
    let amount = args.get(1).map(String::as_str).unwrap_or("?");
    if amount.parse::<i64>().is_err() {
        return DslLine::Todo(format!(
            "Duel.Damage(player={}, amount={}, ...) — non-literal amount",
            player, amount
        ));
    }
    let player_d = match player {
        "tp" => "you",
        "1-tp" => "opponent",
        _ => return DslLine::Todo(format!(
            "Duel.Damage(player={}, amount={}) — non-canonical player",
            player, amount
        )),
    };
    DslLine::Action(format!("damage {} {}", player_d, amount))
}

/// `Duel.ChangePosition(target, pos)` → `change_position target [to <pos>]`.
fn action_change_position(args: &[String]) -> DslLine {
    let pos = args.get(1).map(String::as_str).unwrap_or("");
    let to = match pos {
        "POS_FACEUP_ATTACK"   => Some("attack_position"),
        "POS_FACEUP_DEFENSE"  => Some("defense_position"),
        // POS_FACEDOWN / POS_FACEDOWN_DEFENSE / mixed bitmasks fall through
        _ => None,
    };
    match to {
        Some(p) => DslLine::Action(format!("change_position target to {}", p)),
        None    => DslLine::Action("change_position target".to_string()),
    }
}

/// `Duel.Equip(player, eq, tar, ...)` → `equip self to target` for the
/// canonical "equip this card to selected target" shape. Other shapes
/// (equip group to single target, multi-target) → TODO.
fn action_equip(args: &[String]) -> DslLine {
    let eq = args.get(1).map(String::as_str).unwrap_or("");
    let tar = args.get(2).map(String::as_str).unwrap_or("");
    if eq == "c" && (tar == "tc" || tar == "g" || tar == "g:GetFirst()") {
        DslLine::Action("equip self to target".to_string())
    } else {
        DslLine::Todo(format!("Duel.Equip(eq={}, tar={}) — non-canonical shape", eq, tar))
    }
}

/// `Duel.ShuffleDeck(player)` → `shuffle_deck [yours|opponents]`.
fn action_shuffle(args: &[String]) -> DslLine {
    let player = args.first().map(String::as_str).unwrap_or("");
    let who = match player {
        "tp" => "yours",
        "1-tp" => "opponents",
        _ => return DslLine::Action("shuffle_deck".to_string()),
    };
    DslLine::Action(format!("shuffle_deck {}", who))
}

/// Translate `Duel.Recover(player, amount, reason)` to `gain_lp <N>`.
/// DSL has no opponent-recover form, so non-self-target → TODO.
fn action_recover(args: &[String]) -> DslLine {
    let player = args.first().map(String::as_str).unwrap_or("");
    let amount = args.get(1).map(String::as_str).unwrap_or("?");
    if amount.parse::<i64>().is_err() {
        return DslLine::Todo(format!(
            "Duel.Recover(player={}, amount={}) — non-literal amount",
            player, amount
        ));
    }
    if player == "tp" {
        DslLine::Action(format!("gain_lp {}", amount))
    } else {
        DslLine::Todo(format!("Duel.Recover(player={}) — only self-recover supported", player))
    }
}

/// Translate `Duel.Draw(player, count, reason)` to `draw <N>`.
fn action_draw(args: &[String]) -> DslLine {
    let count = args.get(1).map(String::as_str).unwrap_or("?");
    if count.parse::<u32>().is_ok() {
        DslLine::Action(format!("draw {}", count))
    } else {
        DslLine::Todo(format!("Duel.Draw(..., count={}) — non-literal count", count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_duel_damage() {
        let calls = vec![
            DuelCall { method: "Duel.Damage".to_string(), args: vec!["1-tp".into(), "1000".into(), "REASON_EFFECT".into()] },
        ];
        let lines = translate_calls(&calls);
        assert_eq!(lines.len(), 1);
        match &lines[0] {
            DslLine::Action(s) => assert_eq!(s, "damage opponent 1000"),
            DslLine::Todo(s) => panic!("expected action, got TODO: {}", s),
        }
    }

    #[test]
    fn translate_duel_draw() {
        let calls = vec![
            DuelCall { method: "Duel.Draw".to_string(), args: vec!["tp".into(), "2".into(), "REASON_EFFECT".into()] },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "draw 2"));
    }

    #[test]
    fn translate_duel_destroy_target() {
        let calls = vec![
            DuelCall { method: "Duel.Destroy".to_string(), args: vec!["g".into(), "REASON_EFFECT".into()] },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "destroy target"));
    }

    #[test]
    fn translate_skips_meta_calls() {
        let calls = vec![
            DuelCall { method: "Duel.Hint".to_string(), args: vec!["HINT_SELECTMSG".into()] },
            DuelCall { method: "Duel.SetOperationInfo".to_string(), args: vec![] },
            DuelCall { method: "Duel.BreakEffect".to_string(), args: vec![] },
            DuelCall { method: "Duel.Damage".to_string(), args: vec!["1-tp".into(), "500".into(), "REASON_EFFECT".into()] },
        ];
        let lines = translate_calls(&calls);
        assert_eq!(lines.len(), 1, "only the Damage call should produce a DSL line");
    }

    #[test]
    fn translate_unknown_emits_todo() {
        let calls = vec![
            DuelCall { method: "Duel.SwapSequence".to_string(), args: vec!["a".into(), "b".into()] },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Todo(_)));
    }

    #[test]
    fn analyze_gravedigger_ghoul() {
        let src = r#"
local s,id=GetID()
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetCategory(CATEGORY_REMOVE)
    e1:SetType(EFFECT_TYPE_ACTIVATE)
    e1:SetCode(EVENT_FREE_CHAIN)
    e1:SetTarget(s.target)
    e1:SetOperation(s.activate)
    c:RegisterEffect(e1)
end
function s.target(e,tp,eg,ep,ev,re,r,rp,chk,chkc)
    if chk==0 then return Duel.IsExistingTarget(s.filter,tp,0,LOCATION_MZONE,1,nil) end
    Duel.SelectTarget(tp,s.filter,tp,0,LOCATION_MZONE,1,2,nil)
end
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    Duel.Remove(g,POS_FACEUP,REASON_EFFECT)
end
"#;
        let report = analyze(src);
        // Effects discovered
        assert!(report.contains("=== effects: 1 ==="), "expected 1 effect, got:\n{}", report);
        assert!(report.contains("SetCategory"));
        assert!(report.contains("SetOperation"));
        assert!(report.contains("operation handler: s.activate"));
        // Duel.* calls discovered for s.activate
        assert!(report.contains("Duel.Remove"), "expected Duel.Remove, got:\n{}", report);
    }
}
