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
    pub functions: BTreeMap<String, FunctionBody>,
    pub parse_error: Option<String>,
}

/// One operation-handler function body: the ordered list of `Duel.*`
/// calls plus any local-variable bindings whose RHS is a Duel.Select*
/// or GetMatching* call (so that downstream actions referencing those
/// bindings can emit a real selector instead of bare `target`).
#[derive(Debug, Default, Clone)]
pub struct FunctionBody {
    pub calls: Vec<DuelCall>,
    pub group_bindings: BTreeMap<String, SelectorSpec>,
    pub register_chains: Vec<RegisterEffectChain>,
}

/// One `Effect.CreateEffect → SetX → <recv>:RegisterEffect(eN)` chain
/// extracted from a function body. Phase 4 uses this to translate
/// continuous-modifier effects (ATK/DEF buffs) created at activation
/// time into DSL `modify_atk` / `modify_def` lines.
#[derive(Debug, Clone, Default)]
pub struct RegisterEffectChain {
    pub code: Option<String>,
    pub value: Option<String>,
    pub reset: Option<String>,
    pub effect_type: Option<String>,
    pub register_target: String,
    pub multi_target: bool,
}

/// A statically-extracted selector intent — built from a single
/// Duel.SelectMatchingCard / GetMatchingGroup / SelectTarget call.
/// Renders to DSL `(qty, kind, controller, zone[, where])`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorSpec {
    pub quantity: String,            // "1", "2", "1+", "all"
    pub kind: String,                // "monster", "card", "spell", ...
    pub controller: Option<String>,  // "you control", "opponent controls", "either controls"
    pub zone: Option<String>,        // "from hand", "from gy", ...
    pub where_clause: Option<String>,
}

impl SelectorSpec {
    pub fn to_dsl(&self) -> String {
        let mut parts: Vec<String> = vec![self.quantity.clone(), self.kind.clone()];
        if let Some(c) = &self.controller { parts.push(c.clone()); }
        if let Some(z) = &self.zone { parts.push(z.clone()); }
        if let Some(w) = &self.where_clause { parts.push(format!("where {}", w)); }
        format!("({})", parts.join(", "))
    }
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
            let body_block = body.block();
            let calls = extract_duel_calls(body_block);
            let group_bindings = extract_group_bindings(body_block);
            let register_chains = extract_register_chains(body_block);
            if !calls.is_empty() || !group_bindings.is_empty() || !register_chains.is_empty() {
                report.functions.insert(name, FunctionBody {
                    calls,
                    group_bindings,
                    register_chains,
                });
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

/// Walk a function body for `local <name> = Duel.<select-call>(...)`
/// and `local <name> = Duel.<select-call>(...)` chains. Each binding
/// captures the SelectorSpec we'd emit if the binding is referenced
/// later as the target of an action (SendtoGrave(<name>), etc.).
fn extract_group_bindings(block: &Block) -> BTreeMap<String, SelectorSpec> {
    let mut out = BTreeMap::new();
    collect_group_bindings(block, &mut out);
    out
}

fn collect_group_bindings(block: &Block, out: &mut BTreeMap<String, SelectorSpec>) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                for (i, name) in names.iter().enumerate() {
                    if let Some(Expression::FunctionCall(fc)) = exprs.get(i) {
                        if let Some(spec) = selector_spec_from_call(fc) {
                            out.insert(name.clone(), spec);
                        }
                    }
                }
            }
            Stmt::If(if_stmt)     => { collect_group_bindings(if_stmt.block(), out); }
            Stmt::While(w)        => { collect_group_bindings(w.block(), out); }
            Stmt::NumericFor(nf)  => { collect_group_bindings(nf.block(), out); }
            Stmt::GenericFor(gf)  => { collect_group_bindings(gf.block(), out); }
            Stmt::Do(d)           => { collect_group_bindings(d.block(), out); }
            _ => {}
        }
    }
}

/// If the call is one of the known selector-producing Duel.* methods,
/// extract a `SelectorSpec` from its arguments. Returns None for
/// unrecognized shapes — caller falls back to the bare `target` placeholder.
fn selector_spec_from_call(fc: &FunctionCall) -> Option<SelectorSpec> {
    let head = call_head_string(fc);
    let args: Vec<String> = match fc.suffixes().last() {
        Some(Suffix::Call(Call::AnonymousCall(a))) => call_args_to_strings(a),
        _ => return None,
    };
    match head.as_str() {
        // Duel.SelectMatchingCard(sel_p, filter, scope_p, my_locs, opp_locs, min, max, exception, ...)
        "Duel.SelectMatchingCard" => spec_from_matching(&args, /*has_opp_locs=*/true, /*has_minmax=*/true),
        // Duel.GetMatchingGroup(filter, scope_p, my_locs, opp_locs, exception, ...)
        // Quantity is "all" since it's the unfiltered group; subsequent
        // action will pick from it.
        "Duel.GetMatchingGroup" => spec_from_get_matching(&args),
        // Duel.SelectTarget(sel_p, filter, scope_p, my_locs, opp_locs, min, max, exception, ...)
        "Duel.SelectTarget" => spec_from_matching(&args, true, true),
        _ => None,
    }
}

fn spec_from_matching(args: &[String], _has_opp_locs: bool, has_minmax: bool) -> Option<SelectorSpec> {
    // args: 0=select_p, 1=filter, 2=scope_p, 3=my_locs, 4=opp_locs, 5=min, 6=max, 7=exception
    if args.len() < 7 { return None; }
    let scope_p = args.get(2)?.as_str();
    let my_locs = args.get(3)?.as_str();
    let opp_locs = args.get(4)?.as_str();
    let (min_s, max_s) = if has_minmax {
        (args.get(5)?.as_str(), args.get(6)?.as_str())
    } else {
        ("1", "1")
    };
    let qty = quantity_from(min_s, max_s)?;
    let controller = controller_from_scope(scope_p, my_locs, opp_locs)?;
    let zone = zone_from_locations(my_locs, opp_locs);
    Some(SelectorSpec {
        quantity: qty,
        kind: "card".to_string(),
        controller: Some(controller),
        zone,
        where_clause: None,
    })
}

fn spec_from_get_matching(args: &[String]) -> Option<SelectorSpec> {
    // args: 0=filter, 1=scope_p, 2=my_locs, 3=opp_locs, 4=exception, ...
    if args.len() < 4 { return None; }
    let scope_p = args.get(1)?.as_str();
    let my_locs = args.get(2)?.as_str();
    let opp_locs = args.get(3)?.as_str();
    let controller = controller_from_scope(scope_p, my_locs, opp_locs)?;
    let zone = zone_from_locations(my_locs, opp_locs);
    Some(SelectorSpec {
        quantity: "all".to_string(),
        kind: "card".to_string(),
        controller: Some(controller),
        zone,
        where_clause: None,
    })
}

fn quantity_from(min_s: &str, max_s: &str) -> Option<String> {
    let mn: u32 = min_s.parse().ok()?;
    let mx: u32 = max_s.parse().ok()?;
    if mn == mx {
        Some(format!("{}", mn))
    } else {
        Some(format!("{}+", mn))
    }
}

fn controller_from_scope(scope_p: &str, my_locs: &str, opp_locs: &str) -> Option<String> {
    // If both my_locs and opp_locs are non-zero → either controls.
    // If only opp_locs is set → opponent controls.
    // Default → you control (relative to scope_p which is typically tp).
    let _ = scope_p;
    let mine_set = my_locs != "0";
    let opp_set = opp_locs != "0";
    Some(match (mine_set, opp_set) {
        (true,  true)  => "either controls".to_string(),
        (false, true)  => "opponent controls".to_string(),
        _              => "you control".to_string(),
    })
}

/// Map LOCATION_* (possibly OR'd) to a DSL `from <zone>` clause. Returns
/// None when locations don't map cleanly (e.g. multi-zone disjunctions
/// the DSL can't express with one `from` token).
fn zone_from_locations(my: &str, opp: &str) -> Option<String> {
    // Pick whichever side is non-zero. If both, prefer my for now —
    // controller field already disambiguates "either".
    let loc = if my != "0" { my } else { opp };
    let zone = match loc {
        "LOCATION_HAND"     => Some("hand"),
        "LOCATION_DECK"     => Some("deck"),
        "LOCATION_GRAVE"    => Some("gy"),
        "LOCATION_REMOVED"  => Some("banished"),
        "LOCATION_MZONE"    => Some("monster_zone"),
        "LOCATION_SZONE"    => Some("spell_trap_zone"),
        "LOCATION_EXTRA"    => Some("extra_deck"),
        "LOCATION_ONFIELD"  => Some("field"),
        _ => None, // OR'd locations or unknown
    }?;
    Some(format!("from {}", zone))
}

/// Walk a function body and extract every `Effect.CreateEffect → Set* →
/// <recv>:RegisterEffect(eN)` chain. Bindings span the whole function
/// scope (Lua does not re-scope `local` per inner block when the same
/// name is reused), so a single `BTreeMap` is threaded through nested
/// blocks. The `in_for_loop` flag flows down through `NumericFor` /
/// `GenericFor` so each emitted chain knows if its `RegisterEffect`
/// fired inside a per-target loop body.
fn extract_register_chains(block: &Block) -> Vec<RegisterEffectChain> {
    let mut chains = Vec::new();
    let mut by_binding: BTreeMap<String, RegisterEffectChain> = BTreeMap::new();
    collect_register_chains(block, false, &mut by_binding, &mut chains);
    chains
}

fn collect_register_chains(
    block: &Block,
    in_for_loop: bool,
    by_binding: &mut BTreeMap<String, RegisterEffectChain>,
    out: &mut Vec<RegisterEffectChain>,
) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                for (i, name) in names.iter().enumerate() {
                    if let Some(expr) = exprs.get(i) {
                        if expr_is_effect_createeffect(expr) {
                            by_binding.insert(name.clone(), RegisterEffectChain::default());
                        } else if let Some(src) = expr_clone_source(expr) {
                            // `local e2 = e1:Clone()` — fork the existing
                            // chain so subsequent overrides apply to e2 only.
                            if let Some(existing) = by_binding.get(&src).cloned() {
                                by_binding.insert(name.clone(), existing);
                            }
                        }
                    }
                }
            }
            Stmt::FunctionCall(fc) => {
                if let Some((bind, method, args)) = method_call_on_binding(fc) {
                    if let Some(chain) = by_binding.get_mut(&bind) {
                        match method.as_str() {
                            "SetCode"  => chain.code = args.first().cloned(),
                            "SetValue" => chain.value = args.first().cloned(),
                            "SetReset" => chain.reset = args.first().cloned(),
                            "SetType"  => chain.effect_type = args.first().cloned(),
                            _ => {}
                        }
                    }
                }
                if let Some((receiver, args)) = try_register_effect_call(fc) {
                    if let Some(eff_name) = args.first() {
                        if let Some(chain) = by_binding.get(eff_name) {
                            // Snapshot the current chain state — keep the
                            // entry in the map so a subsequent `:Clone()`
                            // can still read its values.
                            let mut emitted = chain.clone();
                            emitted.register_target = receiver;
                            emitted.multi_target = in_for_loop;
                            out.push(emitted);
                        }
                    }
                }
            }
            Stmt::If(if_stmt) => {
                collect_register_chains(if_stmt.block(), in_for_loop, by_binding, out);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_register_chains(ei.block(), in_for_loop, by_binding, out);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_register_chains(else_block, in_for_loop, by_binding, out);
                }
            }
            Stmt::While(w)       => collect_register_chains(w.block(), in_for_loop, by_binding, out),
            Stmt::Repeat(r)      => collect_register_chains(r.block(), in_for_loop, by_binding, out),
            Stmt::NumericFor(nf) => collect_register_chains(nf.block(), true, by_binding, out),
            Stmt::GenericFor(gf) => collect_register_chains(gf.block(), true, by_binding, out),
            Stmt::Do(d)          => collect_register_chains(d.block(), in_for_loop, by_binding, out),
            _ => {}
        }
    }
}

/// True if `expr` is `<binding>:Clone()`. Returns `Some(binding)` so the
/// caller can copy the source chain into the new local.
fn expr_clone_source(expr: &Expression) -> Option<String> {
    let fc = match expr {
        Expression::FunctionCall(fc) => fc,
        _ => return None,
    };
    let prefix = match fc.prefix() {
        ast::Prefix::Name(n) => n.token().to_string(),
        _ => return None,
    };
    let mut suffixes = fc.suffixes();
    let first = suffixes.next()?;
    if let Suffix::Call(Call::MethodCall(mc)) = first {
        if mc.name().token().to_string() == "Clone" {
            return Some(prefix);
        }
    }
    None
}

/// Match `<receiver>:RegisterEffect(<arg>)` where `<receiver>` may itself
/// be a chained expression like `e:GetHandler()`. Returns the rendered
/// receiver path and the call arguments so callers can identify which
/// chain was committed and on what.
fn try_register_effect_call(fc: &FunctionCall) -> Option<(String, Vec<String>)> {
    let suffixes: Vec<&Suffix> = fc.suffixes().collect();
    if suffixes.is_empty() { return None; }
    let last = suffixes.last()?;
    let last_args = match last {
        Suffix::Call(Call::MethodCall(mc))
            if mc.name().token().to_string() == "RegisterEffect" =>
        {
            call_args_to_strings(mc.args())
        }
        _ => return None,
    };
    let mut receiver = match fc.prefix() {
        ast::Prefix::Name(n) => n.token().to_string(),
        _ => fc.prefix().to_string().trim().to_string(),
    };
    for s in &suffixes[..suffixes.len() - 1] {
        match s {
            Suffix::Index(Index::Dot { name, .. }) => {
                receiver.push('.');
                receiver.push_str(&name.token().to_string());
            }
            Suffix::Call(Call::MethodCall(mc)) => {
                receiver.push(':');
                receiver.push_str(&mc.name().token().to_string());
                let a = call_args_to_strings(mc.args());
                receiver.push('(');
                receiver.push_str(&a.join(","));
                receiver.push(')');
            }
            Suffix::Call(Call::AnonymousCall(args)) => {
                let a = call_args_to_strings(args);
                receiver.push('(');
                receiver.push_str(&a.join(","));
                receiver.push(')');
            }
            _ => {}
        }
    }
    Some((receiver, last_args))
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
            if let Some(body) = r.functions.get(&handler_to_fn_name(op)) {
                for (b, spec) in &body.group_bindings {
                    writeln!(out, "          [bind] {} := {}", b, spec.to_dsl()).ok();
                }
                for c in &body.calls {
                    writeln!(out, "          {}({})", c.method, c.args.join(", ")).ok();
                }
            }
        }
    }
    writeln!(out, "=== functions w/ Duel.* calls: {} ===", r.functions.len()).ok();
    for (n, body) in &r.functions {
        writeln!(out, "  {} ({} duel calls, {} bindings)",
            n, body.calls.len(), body.group_bindings.len()).ok();
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
///
/// When a call's first argument names a previously-bound selector group
/// (e.g. `Duel.SendtoGrave(g, ...)` after `local g = Duel.SelectMatchingCard(...)`),
/// substitutes the bare `target` placeholder with the real selector spec.
pub fn translate_calls(calls: &[DuelCall]) -> Vec<DslLine> {
    translate_body(&FunctionBody {
        calls: calls.to_vec(),
        group_bindings: BTreeMap::new(),
        register_chains: Vec::new(),
    })
}

/// Selector-aware translator entry point. Emits Duel.* action lines first
/// (Phase 2/3 behavior), then any continuous-modifier `RegisterEffect`
/// chains the body created (Phase 4 — `modify_atk` / `modify_def`).
pub fn translate_body(body: &FunctionBody) -> Vec<DslLine> {
    let mut out = Vec::new();
    for c in &body.calls {
        if let Some(line) = translate_call(c, &body.group_bindings) {
            out.push(line);
        }
    }
    for chain in &body.register_chains {
        if let Some(line) = translate_register_chain(chain) {
            out.push(line);
        }
    }
    out
}

/// Map one `RegisterEffectChain` to a DSL action line. Returns None when
/// the chain isn't one of the shapes Phase 4 covers (non-stat code,
/// non-literal value, multi-target loop, unknown receiver).
fn translate_register_chain(chain: &RegisterEffectChain) -> Option<DslLine> {
    if chain.multi_target { return None; }
    let code = chain.code.as_deref()?;
    let action = match code {
        "EFFECT_UPDATE_ATTACK"  => "modify_atk",
        "EFFECT_UPDATE_DEFENSE" => "modify_def",
        _ => return None,
    };
    let value: i64 = chain.value.as_deref()?.parse().ok()?;
    let selector = match chain.register_target.as_str() {
        "tc" | "tc:GetFirst()" | "g:GetFirst()" => "target",
        "c" | "e:GetHandler()" => "self",
        _ => return None,
    };
    let (op, n) = if value < 0 { ("-", (-value) as u64) } else { ("+", value as u64) };
    let mut line = format!("{} {} {} {}", action, selector, op, n);
    if reset_is_end_of_turn(chain.reset.as_deref()) {
        line.push_str(" until end_of_turn");
    }
    Some(DslLine::Action(line))
}

/// Phase 4 maps any `SetReset` whose argument mentions `PHASE_END` (the
/// edopro idiom for end-of-turn cleanup) to DSL `until end_of_turn`.
/// Other reset shapes (BATTLE_PHASE_END, CHAIN, etc.) are deferred and
/// emit no `until` clause — the engine treats that as until-the-card-leaves
/// which is incorrect for some cases but keeps the corpus parseable.
fn reset_is_end_of_turn(reset: Option<&str>) -> bool {
    match reset {
        Some(s) => s.contains("PHASE_END"),
        None => false,
    }
}

/// Map a single `Duel.X` call to a DSL line (or None for skip-class
/// metadata). `bindings` maps local-variable names to their captured
/// SelectorSpec so referenced groups can become real selectors.
fn translate_call(c: &DuelCall, bindings: &BTreeMap<String, SelectorSpec>) -> Option<DslLine> {
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
            format!("destroy {}", group_arg(a, 0, bindings))
        )),

        // Duel.SendtoGrave(target, reason)
        "Duel.SendtoGrave" => Some(DslLine::Action(
            format!("send {} to gy", group_arg(a, 0, bindings))
        )),

        // Duel.SendtoHand(target, player, reason)
        "Duel.SendtoHand" => Some(DslLine::Action(
            format!("add_to_hand {}", group_arg(a, 0, bindings))
        )),

        // Duel.SendtoDeck(target, player, sequence, reason)
        "Duel.SendtoDeck" => Some(DslLine::Action(
            format!("send {} to deck", group_arg(a, 0, bindings))
        )),

        // Duel.Remove(target, pos, reason) — banish
        "Duel.Remove" => Some(DslLine::Action(
            format!("banish {}", group_arg(a, 0, bindings))
        )),

        // Duel.Release(target, reason) — tribute. No DSL `tribute`
        // action in resolve grammar (only `tribute self` in cost block).
        // Released cards go to gy; closest semantic action is send-to-gy.
        "Duel.Release" => Some(DslLine::Action(
            format!("send {} to gy", group_arg(a, 0, bindings))
        )),

        // Duel.SpecialSummon(target, sumtype, p1, p2, nocheck, nolimit, pos)
        "Duel.SpecialSummon" => Some(DslLine::Action(
            format!("special_summon {}", group_arg(a, 0, bindings))
        )),
        "Duel.SpecialSummonStep" => Some(DslLine::Action(
            format!("special_summon {}", group_arg(a, 0, bindings))
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
        "Duel.ChangePosition" => Some(action_change_position(a, bindings)),

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

/// `Duel.ChangePosition(target, pos)` → `change_position <sel> [to <pos>]`.
fn action_change_position(args: &[String], bindings: &BTreeMap<String, SelectorSpec>) -> DslLine {
    let target = group_arg(args, 0, bindings);
    let pos = args.get(1).map(String::as_str).unwrap_or("");
    let to = match pos {
        "POS_FACEUP_ATTACK"   => Some("attack_position"),
        "POS_FACEUP_DEFENSE"  => Some("defense_position"),
        _ => None,
    };
    match to {
        Some(p) => DslLine::Action(format!("change_position {} to {}", target, p)),
        None    => DslLine::Action(format!("change_position {}", target)),
    }
}

/// Resolve action argument N to a DSL selector expression. If the
/// argument names a known group binding, substitute the captured
/// SelectorSpec; otherwise default to the bare `target` placeholder.
fn group_arg(args: &[String], idx: usize, bindings: &BTreeMap<String, SelectorSpec>) -> String {
    let raw = match args.get(idx) {
        Some(s) => s.trim(),
        None => return "target".to_string(),
    };
    // Strip common ":GetFirst()" or ":Filter(...)" suffix to get base name.
    let base = raw.split(|c| c == ':' || c == '.').next().unwrap_or(raw);
    if let Some(spec) = bindings.get(base) {
        return spec.to_dsl();
    }
    "target".to_string()
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
    fn register_chain_simple_atk_buff_on_target() {
        // Mask of Weakness shape: tc:RegisterEffect with literal value
        // and a PHASE_END reset → modify_atk target -700 until end_of_turn.
        let src = r#"
local s,id=GetID()
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(-700)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        assert_eq!(body.register_chains.len(), 1);
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("modify_atk target - 700 until end_of_turn"));
    }

    #[test]
    fn register_chain_def_buff_on_self_no_reset() {
        // Equip-style passive often has no SetReset and registers on `c`
        // (the equipped card itself) → modify_def self + N (no until).
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_EQUIP)
    e1:SetCode(EFFECT_UPDATE_DEFENSE)
    e1:SetValue(300)
    c:RegisterEffect(e1)
end
"#;
        // Top-level Effect.CreateEffect is matched by the dedicated
        // `extract_effects_from_block` path; we need a non-initial_effect
        // function for the new chain extractor to fire.
        let src2 = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_DEFENSE)
    e1:SetValue(300)
    c:RegisterEffect(e1)
end
"#;
        let _ = src; // silence unused
        let parsed = full_moon::parse(src2).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        assert_eq!(body.register_chains.len(), 1);
        let chain = &body.register_chains[0];
        assert_eq!(chain.code.as_deref(), Some("EFFECT_UPDATE_DEFENSE"));
        assert_eq!(chain.value.as_deref(), Some("300"));
        assert_eq!(chain.register_target, "c");
        assert!(!chain.multi_target);
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("modify_def self + 300"));
    }

    #[test]
    fn register_chain_in_for_loop_marked_multi_target() {
        // Daigusto Falcos: register inside `for tc in aux.Next(g)` →
        // multi_target=true → translator skips emission.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(s.filter,tp,LOCATION_MZONE,LOCATION_MZONE,nil)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(600)
        tc:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        assert_eq!(body.register_chains.len(), 1);
        assert!(body.register_chains[0].multi_target);
        let lines = translate_body(body);
        // No modify_atk emission because multi-target.
        let has_modify = lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("modify_atk")));
        assert!(!has_modify, "multi-target chains should not emit modify_atk");
    }

    #[test]
    fn register_chain_clone_inherits_source_chain() {
        // Laser Cannon Armor: e3 = e2:Clone() then e3:SetCode(...) →
        // e3 inherits e2's value/reset, overrides code.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_SINGLE)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(300)
    c:RegisterEffect(e2)
    local e3=e2:Clone()
    e3:SetCode(EFFECT_UPDATE_DEFENSE)
    c:RegisterEffect(e3)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        assert_eq!(body.register_chains.len(), 2);
        let codes: Vec<_> = body.register_chains.iter()
            .filter_map(|c| c.code.as_deref()).collect();
        assert_eq!(codes, vec!["EFFECT_UPDATE_ATTACK", "EFFECT_UPDATE_DEFENSE"]);
        let values: Vec<_> = body.register_chains.iter()
            .filter_map(|c| c.value.as_deref()).collect();
        assert_eq!(values, vec!["300", "300"]);
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
