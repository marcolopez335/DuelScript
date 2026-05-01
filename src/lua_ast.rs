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
use full_moon::ast::{Stmt, Expression, FunctionCall, Suffix, Call, Index, Block, LastStmt};

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
    /// Local-variable assignments keyed by name, value = RHS text.
    /// Phase 4c uses this to resolve `SetValue(atk)` shapes where `atk`
    /// is `local atk = c:GetAttack()` defined earlier in the handler.
    pub value_bindings: BTreeMap<String, String>,
    /// The return expression text, when the function body is a single
    /// `return <expr>` with no preceding statements. Phase 6 uses this to
    /// extract DSL `condition: <expr>` from `s.condition` handler bodies.
    pub return_expr: Option<String>,
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
    /// When the chain's `RegisterEffect` fired inside a `for tc in
    /// aux.Next(g) do … end` loop, this carries the name of the source
    /// group binding (`g`). The translator looks it up in
    /// `FunctionBody::group_bindings` to render a real selector instead
    /// of the bare `target` placeholder.
    pub loop_source_group: Option<String>,
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

impl EffectSkeleton {
    /// First arg passed to `<binding>:<method>(...)`, or None if not called.
    fn first_arg_of(&self, method: &str) -> Option<&str> {
        self.set_calls.iter().find(|(m, _)| m == method)
            .and_then(|(_, args)| args.first().map(String::as_str))
    }

    /// All args passed to `<binding>:<method>(...)`, or None if not called.
    fn args_of(&self, method: &str) -> Option<&[String]> {
        self.set_calls.iter().find(|(m, _)| m == method)
            .map(|(_, args)| args.as_slice())
    }

    /// True when this skeleton has none of the activated-effect handlers
    /// (`SetOperation` / `SetTarget` / `SetCondition` / `SetCost`). Such
    /// chains are typically continuous modifiers registered on the card
    /// itself in `s.initial_effect` and map to DSL `passive` blocks.
    fn is_purely_passive(&self) -> bool {
        self.operation_handler.is_none()
            && self.target_handler.is_none()
            && self.condition_handler.is_none()
            && self.cost_handler.is_none()
    }

    /// If this skeleton is a literal stat-modifier passive
    /// (`SetCode(EFFECT_UPDATE_ATTACK|DEFENSE)` + literal `SetValue`,
    /// no activated-effect handlers), return the spec.
    ///
    /// Returns None for `EFFECT_TYPE_FIELD` chains whose `SetTargetRange`
    /// is missing — without that arg we cannot know whether the modifier
    /// applies to your monsters, opponent's, or both, and emitting a
    /// default `scope: self` would misrepresent the semantics.
    pub fn passive_modifier_spec(&self) -> Option<PassiveModifierSpec> {
        if !self.registered { return None; }
        if !self.is_purely_passive() { return None; }
        let code = self.first_arg_of("SetCode")?;
        let stat = match code {
            "EFFECT_UPDATE_ATTACK"  => "atk",
            "EFFECT_UPDATE_DEFENSE" => "def",
            _ => return None,
        };
        let value: i64 = self.first_arg_of("SetValue")?.parse().ok()?;
        let effect_type = self.first_arg_of("SetType").unwrap_or("").to_string();
        let scope_target = derive_passive_scope_target(
            &effect_type,
            self.args_of("SetTargetRange"),
        )?;
        Some(PassiveModifierSpec {
            stat: stat.to_string(),
            value,
            effect_type,
            scope: scope_target.scope,
            target: scope_target.target,
        })
    }
}

/// Derived scope/target pair for a passive's emit text. `None` means we
/// don't have enough information to render it correctly — the spec is
/// then dropped rather than mis-emitted.
struct PassiveScopeTarget {
    scope: Option<&'static str>,         // "self" | "field" | None (omit)
    target: Option<&'static str>,        // selector text or None (omit)
}

fn derive_passive_scope_target(
    effect_type: &str,
    target_range: Option<&[String]>,
) -> Option<PassiveScopeTarget> {
    // EFFECT_TYPE_EQUIP: modifier applies to the monster the spell is
    // equipped to. DSL → `target: equipped_card`.
    if effect_type.contains("EFFECT_TYPE_EQUIP") {
        return Some(PassiveScopeTarget {
            scope: None,
            target: Some("equipped_card"),
        });
    }
    // EFFECT_TYPE_FIELD: continuous field-wide modifier. SetTargetRange
    // determines whose monsters are affected. We map three common shapes
    // and drop the rest (multi-zone OR-masks, custom locations, …).
    if effect_type.contains("EFFECT_TYPE_FIELD") {
        let args = target_range?;
        if args.len() < 2 { return None; }
        let my = args[0].trim();
        let opp = args[1].trim();
        return Some(PassiveScopeTarget {
            scope: Some("field"),
            target: Some(match (my, opp) {
                ("LOCATION_MZONE", "0")              => "(all, monster, you control)",
                ("0", "LOCATION_MZONE")              => "(all, monster, opponent controls)",
                ("LOCATION_MZONE", "LOCATION_MZONE") => "(all, monster, either controls)",
                _ => return None,
            }),
        });
    }
    // EFFECT_TYPE_SINGLE: modifier applies to the card itself. DSL
    // default (no scope, no target) means self — leave both None.
    if effect_type.contains("EFFECT_TYPE_SINGLE") {
        return Some(PassiveScopeTarget { scope: None, target: None });
    }
    None
}

/// Spec for a literal stat-modifier passive — extracted from an
/// `EffectSkeleton` whose chain is `SetType(EFFECT_TYPE_*) +
/// SetCode(EFFECT_UPDATE_ATTACK|DEFENSE) + SetValue(<int>)` with no
/// activated-effect handlers and a `c:RegisterEffect` commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassiveModifierSpec {
    pub stat: String,             // "atk" | "def"
    pub value: i64,               // signed delta
    pub effect_type: String,      // raw SetType arg
    pub scope: Option<&'static str>,
    pub target: Option<&'static str>,
}

impl PassiveModifierSpec {
    /// Render to a DSL `passive "<name>" { … }` block. Emits a `scope:`
    /// or `target:` line when needed so the modifier's reach matches
    /// the underlying Lua chain (e.g. `target: equipped_card` for
    /// `EFFECT_TYPE_EQUIP`).
    pub fn to_dsl_block(&self, name: &str, indent: &str) -> String {
        let op = if self.value < 0 { '-' } else { '+' };
        let n = self.value.unsigned_abs();
        let mut body = String::new();
        if let Some(scope) = self.scope {
            body.push_str(&format!("{indent}    scope: {scope}\n"));
        }
        if let Some(target) = self.target {
            body.push_str(&format!("{indent}    target: {target}\n"));
        }
        body.push_str(&format!("{indent}    modifier: {} {} {}\n", self.stat, op, n));
        format!("{indent}passive \"{name}\" {{\n{body}{indent}}}")
    }
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
            let value_bindings = extract_value_bindings(body_block);
            let return_expr = extract_return_expr(body_block);
            if !calls.is_empty() || !group_bindings.is_empty()
                || !register_chains.is_empty() || !value_bindings.is_empty()
                || return_expr.is_some()
            {
                report.functions.insert(name, FunctionBody {
                    calls,
                    group_bindings,
                    register_chains,
                    value_bindings,
                    return_expr,
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

/// Walk a function body and collect every `local <name> = <RHS>` whose
/// RHS is *not* an Effect.CreateEffect or a Duel.Select* / GetMatching*
/// call (those have their own walkers). Used by Phase 4c to resolve
/// `SetValue(<name>)` shapes — the modifier value's source is whatever
/// expression was assigned to that local.
///
/// RHS is captured as raw text. The Phase 4c `parse_lua_value` helper
/// then attempts a recursive translation into DSL `expr` syntax.
fn extract_value_bindings(block: &Block) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    collect_value_bindings(block, &mut out);
    out
}

fn collect_value_bindings(block: &Block, out: &mut BTreeMap<String, String>) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                for (i, name) in names.iter().enumerate() {
                    if let Some(expr) = exprs.get(i) {
                        // Skip the special-cased shapes — already tracked.
                        if expr_is_effect_createeffect(expr) { continue; }
                        if let Expression::FunctionCall(fc) = expr {
                            if selector_spec_from_call(fc).is_some() { continue; }
                        }
                        let text = expr.to_string().trim().to_string();
                        if !text.is_empty() {
                            out.insert(name.clone(), text);
                        }
                    }
                }
            }
            Stmt::If(if_stmt)     => { collect_value_bindings(if_stmt.block(), out); }
            Stmt::While(w)        => { collect_value_bindings(w.block(), out); }
            Stmt::NumericFor(nf)  => { collect_value_bindings(nf.block(), out); }
            Stmt::GenericFor(gf)  => { collect_value_bindings(gf.block(), out); }
            Stmt::Do(d)           => { collect_value_bindings(d.block(), out); }
            _ => {}
        }
    }
}

/// Extract the return expression text from a block whose last statement is
/// `return <expr>`. Only succeeds when the block has NO preceding statements
/// (pure-predicate functions), ensuring we don't misread the expression for
/// multi-statement bodies where local aliases would need substitution.
fn extract_return_expr(block: &Block) -> Option<String> {
    // Reject multi-statement bodies — local bindings before `return` would
    // require alias substitution we don't do here.
    if block.stmts().next().is_some() { return None; }
    match block.last_stmt()? {
        LastStmt::Return(ret) => {
            let mut iter = ret.returns().iter();
            let expr = iter.next()?;
            if iter.next().is_some() { return None; } // multi-value return
            let text = expr.to_string();
            let text = text.trim();
            if text.is_empty() { return None; }
            Some(text.to_string())
        }
        _ => None,
    }
}

// ── Phase 7: cost block extraction ───────────────────────────────────────

/// A single cost action atom extracted from a `s.cost` handler body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CostAction {
    PayLp(String),      // pay_lp <N>
    Discard(String),    // discard <selector>
    Tribute(String),    // tribute <selector>
    Banish(String),     // banish <selector>
    SendToGy(String),   // send <selector> to gy
}

impl CostAction {
    fn to_dsl(&self) -> String {
        match self {
            CostAction::PayLp(n)      => format!("pay_lp {}", n),
            CostAction::Discard(sel)  => format!("discard {}", sel),
            CostAction::Tribute(sel)  => format!("tribute {}", sel),
            CostAction::Banish(sel)   => format!("banish {}", sel),
            CostAction::SendToGy(sel) => format!("send {} to gy", sel),
        }
    }
}

/// A `cost { … }` block built from one or more cost actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostBlockSpec {
    pub actions: Vec<CostAction>,
}

impl CostBlockSpec {
    /// Render to a `cost { … }` block string.
    ///
    /// The opening `cost {` has NO leading whitespace — the caller supplies the
    /// indent from the surrounding text (e.g. the 8 spaces already present
    /// before `resolve {` in the .ds file). `indent` controls the closing `}`
    /// and action lines (= `indent` + 4 spaces).
    pub fn to_dsl_block(&self, indent: &str) -> String {
        let mut out = "cost {\n".to_string();
        for action in &self.actions {
            out.push_str(&format!("{}    {}\n", indent, action.to_dsl()));
        }
        out.push_str(&format!("{}}}", indent));
        out
    }
}

/// Known meta / check-only Duel calls that appear in cost function bodies
/// but are not cost-payment actions. These are silently skipped so they
/// don't trigger the "unknown action → bail" guard.
fn is_cost_skip_call(method: &str) -> bool {
    matches!(method,
        "Duel.SetOperationInfo" | "Duel.SetPossibleOperationInfo"
        | "Duel.Hint" | "Duel.HintSelection" | "Duel.BreakEffect"
        | "Duel.SetTargetPlayer" | "Duel.SetTargetParam" | "Duel.SetChainLimit"
        | "Duel.CheckLPCost" | "Duel.CheckReleaseGroupCost"
        | "Duel.IsPlayerCanDraw"
    )
}

/// Generic discard filters that map to an unspecified-card selector.
/// Custom filters (s.cfilter etc.) require a DSL `where` clause we cannot
/// derive statically, so they cause a bail-out.
fn is_generic_discard_filter(filter: &str) -> bool {
    matches!(filter, "nil" | "Card.IsDiscardable" | "Card.IsAbleToGraveAsCost")
}

/// Translate a `s.cost` / `Cost.PayLP(N)` handler into a `CostBlockSpec`.
///
/// `cost_handler` is the raw arg stored in `EffectSkeleton::cost_handler`
/// — either a function name like `"s.cost"` or a built-in like
/// `"Cost.PayLP(1000)"`.
///
/// Returns `None` when:
/// - The handler is a `Cost.PayLP(…)` with a non-literal amount.
/// - The function body contains Duel calls we can't map to cost actions
///   (skip-not-mis-emit).
/// - The function body yields no recognizable cost actions.
pub fn extract_cost_block(
    cost_handler: &str,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<CostBlockSpec> {
    let handler = cost_handler.trim();

    // Inline Cost.PayLP(N) built-in — no function body.
    if let Some(rest) = handler.strip_prefix("Cost.PayLP(") {
        if let Some(n_str) = rest.strip_suffix(')') {
            let n_str = n_str.trim();
            if !n_str.is_empty() && n_str.chars().all(|c| c.is_ascii_digit()) {
                return Some(CostBlockSpec {
                    actions: vec![CostAction::PayLp(n_str.to_string())],
                });
            }
        }
        return None;
    }

    // Named function — look up extracted body.
    let body = functions.get(handler)?;
    extract_cost_from_body(body)
}

/// Determine the DSL "self" argument from a Duel call's first arg string.
/// Both `c` and `e:GetHandler()` refer to the effect's owning card.
fn is_self_arg(arg: &str) -> bool {
    let arg = arg.trim();
    arg == "c" || arg == "e:GetHandler()"
}

fn extract_cost_from_body(body: &FunctionBody) -> Option<CostBlockSpec> {
    let mut actions: Vec<CostAction> = Vec::new();

    for call in &body.calls {
        let m = call.method.as_str();
        let a = &call.args;

        if is_cost_skip_call(m) { continue; }

        match m {
            "Duel.PayLPCost" => {
                let player = a.first().map(String::as_str).unwrap_or("");
                let amount = a.get(1).map(String::as_str).unwrap_or("").trim();
                if player != "tp" { return None; }
                if amount.is_empty() || !amount.chars().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                actions.push(CostAction::PayLp(amount.to_string()));
            }

            "Duel.DiscardHand" => {
                let player = a.first().map(String::as_str).unwrap_or("");
                let filter = a.get(1).map(String::as_str).unwrap_or("").trim();
                let min_s  = a.get(2).map(String::as_str).unwrap_or("1").trim();
                let max_s  = a.get(3).map(String::as_str).unwrap_or("1").trim();
                if player != "tp" { return None; }
                if !is_generic_discard_filter(filter) { return None; }
                let qty = if min_s == max_s {
                    let n: u32 = min_s.parse().ok()?;
                    n.to_string()
                } else {
                    let mn: u32 = min_s.parse().ok()?;
                    format!("{}+", mn)
                };
                actions.push(CostAction::Discard(
                    format!("({}, card, you control, from hand)", qty)
                ));
            }

            "Duel.Release" => {
                let card_arg = a.first().map(String::as_str).unwrap_or("");
                if is_self_arg(card_arg) {
                    actions.push(CostAction::Tribute("self".to_string()));
                } else {
                    return None;
                }
            }

            "Duel.Remove" => {
                let card_arg = a.first().map(String::as_str).unwrap_or("");
                if is_self_arg(card_arg) {
                    actions.push(CostAction::Banish("self".to_string()));
                } else {
                    return None;
                }
            }

            "Duel.SendtoGrave" => {
                let card_arg = a.first().map(String::as_str).unwrap_or("");
                if is_self_arg(card_arg) {
                    actions.push(CostAction::SendToGy("self".to_string()));
                } else {
                    return None;
                }
            }

            _ if m.starts_with("Duel.") => {
                // Unknown or unhandled Duel action in cost context → skip-not-mis-emit.
                return None;
            }

            _ => {} // non-Duel call (aux.*, etc.) — ignore
        }
    }

    if actions.is_empty() { return None; }
    Some(CostBlockSpec { actions })
}

// ── Phase 8: target declaration extraction ───────────────────────────────

/// Translate a `s.target` handler body into a `SelectorSpec` for the
/// effect-level `target <selector>` declaration.
///
/// Returns `Some(SelectorSpec)` when:
/// - The handler body contains a `Duel.SelectTarget` call.
/// - The filter arg (index 1) is `nil` or `aux.TRUE` — generic selector,
///   no custom predicate to mis-emit.
/// - Both min and max quantity args are integer literals (fixed quantity).
///
/// Returns `None` when the filter is a custom function reference or the
/// quantity is non-literal — skip-not-mis-emit per Phase 8 spec.
pub fn extract_target_decl(
    target_handler: &str,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<SelectorSpec> {
    let handler = target_handler.trim();
    let body = functions.get(handler)?;

    // Find the first Duel.SelectTarget call in the handler body.
    let call = body.calls.iter().find(|c| c.method == "Duel.SelectTarget")?;
    let args = &call.args;
    // args: 0=select_p, 1=filter, 2=scope_p, 3=my_locs, 4=opp_locs, 5=min, 6=max, [7=exception]
    if args.len() < 7 { return None; }

    // Filter must be nil or aux.TRUE — custom predicates are deferred.
    let filter = args[1].trim();
    if filter != "nil" && filter != "aux.TRUE" { return None; }

    // Reuse the existing resolve-context extractor (same arg layout).
    spec_from_matching(args, true, true)
}

// ── Phase 6: condition expression extraction ─────────────────────────────

/// Translate a `s.condition` handler body into a DSL `condition: <expr>`
/// string. Returns `None` when the body is complex (multi-line) or uses
/// a predicate shape that has no grammar atom — skip-not-mis-emit.
///
/// Supported atoms (backed by the `condition_atom` grammar rule):
/// - `phase == <phase>`      from `Duel.IsBattlePhase()` / `Duel.IsPhase(PHASE_*)`
/// - `in_gy`                 from `e:GetHandler():IsLocation(LOCATION_GRAVE)`
/// - `on_field`              from `e:GetHandler():IsLocation(LOCATION_MZONE/ONFIELD)`
/// - `in_hand`               from `e:GetHandler():IsLocation(LOCATION_HAND)`
/// - `in_banished`           from `e:GetHandler():IsLocation(LOCATION_REMOVED)`
/// - `previous_location == <zone>` from `GetPreviousLocation()` / `IsPreviousLocation`
/// - `reason == <filter>`    from `IsReason(REASON_*)` or `r==REASON_*`
/// - `reason includes <filter>` from `(r&REASON_*)~=0`
/// - `lp <op> N`             from `Duel.GetLP(tp) <op> N`
/// - `opponent_lp <op> N`    from `Duel.GetLP(1-tp) <op> N`
///
/// Compound conditions (`A and B`, `A or B`) are supported when each atom
/// translates — mirrors `condition_expr = condition_atom ~ (conjunction ~ condition_atom)*`.
pub fn extract_condition_expr(body: &FunctionBody) -> Option<String> {
    let ret = body.return_expr.as_deref()?;
    let ret = ret.trim();
    // Try single atom first.
    if let Some(dsl) = cond_atom(ret) { return Some(dsl); }
    // Try compound: split on " and " then " or ".
    cond_compound(ret, " and ", " and ")
        .or_else(|| cond_compound(ret, " or ", " or "))
}

/// Attempt to translate the compound expression `lhs <conj_lua> rhs` (split
/// on `conj_lua`) into DSL atoms joined by `conj_dsl`. Returns None if any
/// part fails to translate.
fn cond_compound(ret: &str, conj_lua: &str, conj_dsl: &str) -> Option<String> {
    let parts: Vec<&str> = ret.splitn(usize::MAX, conj_lua).collect();
    if parts.len() < 2 { return None; }
    let translated: Vec<String> = parts.iter()
        .map(|p| cond_atom(p.trim()))
        .collect::<Option<Vec<_>>>()?;
    Some(translated.join(conj_dsl))
}

/// Map a single Lua return-expression to a DSL `condition_atom`. Returns None
/// for any shape that lacks a grammar atom — the caller will skip the card.
fn cond_atom(expr: &str) -> Option<String> {
    let expr = expr.trim();

    // "not <atom>"
    if let Some(inner) = expr.strip_prefix("not ") {
        return cond_atom(inner.trim()).map(|a| format!("not {a}"));
    }

    // Phase predicates
    if expr == "Duel.IsBattlePhase()" {
        return Some("phase == battle".to_string());
    }
    if let Some(rest) = expr.strip_prefix("Duel.IsPhase(") {
        if let Some(phase) = rest.strip_suffix(')') {
            if let Some(dsl) = phase_const_to_dsl(phase) {
                return Some(format!("phase == {dsl}"));
            }
        }
    }

    // LP comparisons: Duel.GetLP(tp) <op> N
    if let Some(rest) = expr.strip_prefix("Duel.GetLP(tp)") {
        if let Some(dsl) = lp_cmp_to_dsl(rest.trim(), "lp") {
            return Some(dsl);
        }
    }
    if let Some(rest) = expr.strip_prefix("Duel.GetLP(1-tp)") {
        if let Some(dsl) = lp_cmp_to_dsl(rest.trim(), "opponent_lp") {
            return Some(dsl);
        }
    }

    // Self-location: e:GetHandler():IsLocation(LOCATION_*)
    if let Some(rest) = expr.strip_prefix("e:GetHandler():IsLocation(") {
        if let Some(loc) = rest.strip_suffix(')') {
            if let Some(dsl) = self_loc_to_dsl(loc) {
                return Some(dsl.to_string());
            }
        }
    }

    // Previous location — two API variants
    if let Some(rest) = expr.strip_prefix("e:GetHandler():GetPreviousLocation()==") {
        if let Some(dsl) = zone_const_to_dsl(rest) {
            return Some(format!("previous_location == {dsl}"));
        }
    }
    if let Some(rest) = expr.strip_prefix("e:GetHandler():IsPreviousLocation(") {
        if let Some(loc) = rest.strip_suffix(')') {
            if let Some(dsl) = zone_const_to_dsl(loc) {
                return Some(format!("previous_location == {dsl}"));
            }
        }
    }

    // Reason — via IsReason method
    if let Some(rest) = expr.strip_prefix("e:GetHandler():IsReason(") {
        if let Some(reason) = rest.strip_suffix(')') {
            if let Some(dsl) = reason_const_to_dsl(reason) {
                return Some(format!("reason == {dsl}"));
            }
        }
    }
    // Reason — via r==REASON_* (exact equality)
    if let Some(rest) = expr.strip_prefix("r==") {
        if let Some(dsl) = reason_const_to_dsl(rest) {
            return Some(format!("reason == {dsl}"));
        }
    }
    // Reason — via (r&REASON_*)~=0 (bit-flag membership)
    if let (Some(inner), true) = (
        expr.strip_prefix('(').and_then(|s| s.strip_suffix(")~=0")),
        expr.ends_with(")~=0"),
    ) {
        if let Some(rest) = inner.strip_prefix("r&") {
            if let Some(dsl) = reason_const_to_dsl(rest) {
                return Some(format!("reason includes {dsl}"));
            }
        }
    }

    None
}

fn phase_const_to_dsl(c: &str) -> Option<&'static str> {
    Some(match c {
        "PHASE_DRAW"       => "draw",
        "PHASE_STANDBY"    => "standby",
        "PHASE_MAIN1"      => "main1",
        "PHASE_BATTLE"     => "battle",
        "PHASE_MAIN2"      => "main2",
        "PHASE_END"        => "end",
        "PHASE_DAMAGE"     => "damage",
        "PHASE_DAMAGE_CAL" => "damage_calculation",
        _ => return None,
    })
}

fn self_loc_to_dsl(loc: &str) -> Option<&'static str> {
    Some(match loc {
        "LOCATION_GRAVE"   => "in_gy",
        "LOCATION_MZONE"   => "on_field",
        "LOCATION_ONFIELD" => "on_field",
        "LOCATION_HAND"    => "in_hand",
        "LOCATION_REMOVED" => "in_banished",
        _ => return None,
    })
}

fn zone_const_to_dsl(loc: &str) -> Option<&'static str> {
    Some(match loc {
        "LOCATION_GRAVE"   => "gy",
        "LOCATION_MZONE"   => "field",
        "LOCATION_ONFIELD" => "field",
        "LOCATION_HAND"    => "hand",
        "LOCATION_REMOVED" => "banished",
        "LOCATION_DECK"    => "deck",
        "LOCATION_EXTRA"   => "extra_deck",
        "LOCATION_SZONE"   => "spell_trap_zone",
        _ => return None,
    })
}

fn reason_const_to_dsl(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "REASON_EFFECT"   => "effect",
        "REASON_BATTLE"   => "battle",
        "REASON_COST"     => "cost",
        "REASON_MATERIAL" => "material",
        "REASON_RELEASE"  => "release",
        "REASON_RULE"     => "rule",
        "REASON_DISCARD"  => "discard",
        "REASON_RETURN"   => "return",
        "REASON_SUMMON"   => "summon",
        "REASON_DESTROY"  => "destroy",
        _ => return None,
    })
}

/// Translate a LP-comparison suffix like `<=3000`, `>= 100` into DSL form.
fn lp_cmp_to_dsl(rest: &str, prefix: &str) -> Option<String> {
    let (op, num) = parse_cmp_suffix(rest)?;
    // Validate it's a valid integer
    num.parse::<u64>().ok()?;
    Some(format!("{prefix} {op} {num}"))
}

/// Split a `<op><num>` string (e.g. `<=3000`, `>= 100`) into (op, num).
fn parse_cmp_suffix(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();
    for op in [">=", "<=", "==", "!=", ">", "<"] {
        if let Some(rest) = s.strip_prefix(op) {
            let num = rest.trim();
            if !num.is_empty() { return Some((op, num)); }
        }
    }
    None
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
        "LOCATION_PZONE"    => Some("pendulum_zone"),
        "LOCATION_MMZONE"   => Some("extra_monster_zone"),
        _ => None, // OR'd locations or unknown
    }?;
    Some(format!("from {}", zone))
}

/// Walk a function body and extract every `Effect.CreateEffect → Set* →
/// <recv>:RegisterEffect(eN)` chain. Bindings span the whole function
/// scope (Lua does not re-scope `local` per inner block when the same
/// name is reused), so a single `BTreeMap` is threaded through nested
/// blocks. The `loop_source` parameter carries the source-group binding
/// name when we're inside a `for tc in aux.Next(g)` loop — emitted
/// chains then carry it as `loop_source_group` so the translator can
/// look up the captured `SelectorSpec`.
fn extract_register_chains(block: &Block) -> Vec<RegisterEffectChain> {
    let mut chains = Vec::new();
    let mut by_binding: BTreeMap<String, RegisterEffectChain> = BTreeMap::new();
    collect_register_chains(block, None, &mut by_binding, &mut chains);
    chains
}

fn collect_register_chains(
    block: &Block,
    loop_source: Option<&str>,
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
                            let mut emitted = chain.clone();
                            emitted.register_target = receiver;
                            emitted.multi_target = loop_source.is_some();
                            emitted.loop_source_group = loop_source.map(str::to_string);
                            out.push(emitted);
                        }
                    }
                }
            }
            Stmt::If(if_stmt) => {
                collect_register_chains(if_stmt.block(), loop_source, by_binding, out);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_register_chains(ei.block(), loop_source, by_binding, out);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_register_chains(else_block, loop_source, by_binding, out);
                }
            }
            Stmt::While(w)       => collect_register_chains(w.block(), loop_source, by_binding, out),
            Stmt::Repeat(r)      => collect_register_chains(r.block(), loop_source, by_binding, out),
            Stmt::NumericFor(nf) => collect_register_chains(nf.block(), Some(""), by_binding, out),
            Stmt::GenericFor(gf) => {
                let inner = aux_next_source_group(gf).map(|s| s.to_string());
                let inner_ref = inner.as_deref().or(Some(""));
                collect_register_chains(gf.block(), inner_ref, by_binding, out);
            }
            Stmt::Do(d)          => collect_register_chains(d.block(), loop_source, by_binding, out),
            _ => {}
        }
    }
}

/// Inspect a `for <names> in <exprs>` loop. If the iterator expression
/// is `aux.Next(<group>)`, return the group binding name. Other shapes
/// (`pairs`, `ipairs`, custom iterators) return None — caller still
/// flags the loop as multi-target, but without a translatable source.
fn aux_next_source_group(gf: &ast::GenericFor) -> Option<String> {
    let expr = gf.expressions().iter().next()?;
    let fc = match expr {
        Expression::FunctionCall(fc) => fc,
        _ => return None,
    };
    // Prefix must be `aux`
    let prefix = match fc.prefix() {
        ast::Prefix::Name(n) => n.token().to_string(),
        _ => return None,
    };
    if prefix != "aux" { return None; }
    // First suffix Index::Dot(Next), second suffix Call(group_name)
    let suffixes: Vec<&Suffix> = fc.suffixes().collect();
    if suffixes.len() < 2 { return None; }
    let is_next = matches!(suffixes[0],
        Suffix::Index(Index::Dot { name, .. })
            if name.token().to_string() == "Next"
    );
    if !is_next { return None; }
    let args = match suffixes[1] {
        Suffix::Call(Call::AnonymousCall(a)) => call_args_to_strings(a),
        _ => return None,
    };
    args.first().cloned()
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
        value_bindings: BTreeMap::new(),
        return_expr: None,
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
        if let Some(line) = translate_register_chain(chain, body) {
            out.push(line);
        }
    }
    out
}

/// Map one `RegisterEffectChain` to a DSL action line. Returns None when
/// the chain isn't one of the shapes the translator covers.
///
/// Two families:
///   - **Stat modifiers** (Phase 4 / 4b / 4c): EFFECT_UPDATE_ATTACK /
///     EFFECT_UPDATE_DEFENSE → `modify_atk` / `modify_def` with the
///     parsed `SetValue` as the magnitude.
///   - **Grants** (Phase 4e): non-stat ability codes
///     (EFFECT_INDESTRUCTABLE_BATTLE / EFFECT_INDESTRUCTABLE_EFFECT /
///     EFFECT_CANNOT_ATTACK / EFFECT_CANNOT_BE_EFFECT_TARGET) → DSL
///     `grant <selector> <ability> until end_of_turn`. Grants ignore
///     `chain.value` (the lua side uses `SetValue(1)` as a flag) and
///     require an end-of-turn reset to avoid emitting a permanent
///     grant from an ambiguous-duration chain.
fn translate_register_chain(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    let code = chain.code.as_deref()?;
    if let Some(action) = stat_modifier_action(code) {
        return translate_modifier_chain(action, chain, body);
    }
    if let Some(ability) = grant_ability_for(code) {
        return translate_grant_chain(ability, chain, body);
    }
    None
}

/// Map an `EFFECT_UPDATE_*` code to the DSL action verb. Returns None
/// for codes outside the stat-modifier family.
fn stat_modifier_action(code: &str) -> Option<&'static str> {
    Some(match code {
        "EFFECT_UPDATE_ATTACK"  => "modify_atk",
        "EFFECT_UPDATE_DEFENSE" => "modify_def",
        _ => return None,
    })
}

/// Map a non-stat EFFECT code to the DSL `grant` ability phrase.
/// Returns None for codes Phase 4e does not cover.
fn grant_ability_for(code: &str) -> Option<&'static str> {
    Some(match code {
        "EFFECT_INDESTRUCTABLE_BATTLE"   => "cannot_be_destroyed by battle",
        "EFFECT_INDESTRUCTABLE_EFFECT"   => "cannot_be_destroyed by effect",
        "EFFECT_CANNOT_ATTACK"           => "cannot_attack",
        "EFFECT_CANNOT_BE_EFFECT_TARGET" => "cannot_be_targeted",
        _ => return None,
    })
}

/// Resolve the DSL selector from the chain's `register_target` /
/// `loop_source_group`. Shared by the stat-modifier and grant paths.
///
/// Single-target (`multi_target=false`):
///   - `tc` / `tc:GetFirst()` / `g:GetFirst()` → `target`
///   - `c` / `e:GetHandler()` → `self`
///
/// Multi-target (`multi_target=true`):
///   - `loop_source_group` resolves to a binding in
///     `body.group_bindings` → emit using the spec's DSL form.
///   - Otherwise → None.
fn resolve_chain_selector(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<String> {
    if chain.multi_target {
        let group = chain.loop_source_group.as_deref()?;
        let spec = body.group_bindings.get(group)?;
        Some(spec.to_dsl())
    } else {
        match chain.register_target.as_str() {
            "tc" | "tc:GetFirst()" | "g:GetFirst()" => Some("target".to_string()),
            "c" | "e:GetHandler()" => Some("self".to_string()),
            _ => None,
        }
    }
}

/// Stat-modifier chain → `modify_atk` / `modify_def` line.
///
/// Value resolution (Phase 4c):
///   - literal int → `+ N` / `- N`
///   - method call `tc:GetAttack()` etc. → `+ target.atk`
///   - method call `c:GetLevel()` etc.   → `+ self.level`
///   - local-var ref → recurse on the binding's RHS
///   - unary minus → flip sign
///   - method-call * literal / method-call / literal → DSL math expr
fn translate_modifier_chain(
    action: &str,
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    let parsed = parse_lua_value(chain.value.as_deref()?, &body.value_bindings)?;
    // No-op modifier — local-var resolution often lands on `local atk=0`
    // initialisers; the real value is reassigned later in branches we
    // don't track. Skip rather than emit a useless `+ 0` line.
    if parsed.expr == "0" { return None; }
    let selector = resolve_chain_selector(chain, body)?;
    let op = if parsed.negative { '-' } else { '+' };
    let mut line = format!("{} {} {} {}", action, selector, op, parsed.expr);
    if reset_is_end_of_turn(chain.reset.as_deref()) {
        line.push_str(" until end_of_turn");
    }
    Some(DslLine::Action(line))
}

/// Grant chain → `grant <selector> <ability> until end_of_turn`.
///
/// Phase 4e covers non-stat ability codes that lua expresses as
/// `SetCode(EFFECT_<X>) + SetValue(1) + SetReset(<end-of-turn>)
/// + <recv>:RegisterEffect(...)`. The reset gate is mandatory: a chain
/// without `RESETS_STANDARD` / `PHASE_END` would emit a permanent grant
/// from an ambiguous-duration chain, so skip those instead of guessing.
fn translate_grant_chain(
    ability: &str,
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    if !reset_is_end_of_turn(chain.reset.as_deref()) { return None; }
    let selector = resolve_chain_selector(chain, body)?;
    Some(DslLine::Action(format!(
        "grant {} {} until end_of_turn",
        selector, ability
    )))
}

/// Parsed Lua expression as it maps to DSL `expr` syntax. `expr` is
/// always non-negative; the directional sign is carried in `negative`
/// so the caller can emit `+ <expr>` or `- <expr>` for `modify_*`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedValue {
    expr: String,
    negative: bool,
}

/// Recursively translate a Lua expression text into DSL `expr` form.
/// Handles literal ints, method calls on `tc` / `c` (mapping to
/// `target.<stat>` / `self.<stat>`), single-step math (`<m> * N`,
/// `<m> / N`), unary minus, and local-variable substitution via
/// `value_bindings`.
fn parse_lua_value(arg: &str, bindings: &BTreeMap<String, String>) -> Option<ParsedValue> {
    let arg = arg.trim();

    // Unary minus — recurse and flip sign.
    if let Some(rest) = arg.strip_prefix('-') {
        let rest = rest.trim();
        if !rest.is_empty() && !rest.starts_with('-') {
            let inner = parse_lua_value(rest, bindings)?;
            return Some(ParsedValue { expr: inner.expr, negative: !inner.negative });
        }
    }

    // Literal integer.
    if let Ok(n) = arg.parse::<i64>() {
        return Some(ParsedValue {
            expr: n.unsigned_abs().to_string(),
            negative: n < 0,
        });
    }

    // Identifier — resolve via local-var bindings (one level of indirection).
    if arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !arg.is_empty() {
        if let Some(rhs) = bindings.get(arg) {
            return parse_lua_value(rhs, bindings);
        }
    }

    // Direct method calls on `tc` / `c` → DSL `target.<stat>` / `self.<stat>`.
    if let Some(stat) = method_call_to_stat(arg) {
        return Some(ParsedValue { expr: stat, negative: false });
    }

    // Single-step math: `<lhs> <op> <rhs>` where rhs is a literal int.
    // Skip if there's nesting we can't statically split.
    for op in ['*', '/'] {
        if let Some((lhs, rhs)) = arg.rsplit_once(op) {
            let lhs = lhs.trim();
            let rhs = rhs.trim();
            if let (Some(l), Ok(r)) = (method_call_to_stat(lhs), rhs.parse::<u64>()) {
                if r > 0 {
                    return Some(ParsedValue {
                        expr: format!("{} {} {}", l, op, r),
                        negative: false,
                    });
                }
            }
        }
    }

    None
}

/// `tc:GetAttack()` / `c:GetLevel()` → `target.atk` / `self.level`.
fn method_call_to_stat(arg: &str) -> Option<String> {
    let (recv, method) = match arg {
        s if s.starts_with("tc:") => ("target", &s[3..]),
        s if s.starts_with("c:") => ("self", &s[2..]),
        _ => return None,
    };
    let stat = match method {
        "GetAttack()" | "GetBaseAttack()"   => "atk",
        "GetDefense()" | "GetBaseDefense()" => "def",
        "GetLevel()"  => "level",
        "GetRank()"   => "rank",
        _ => return None,
    };
    Some(format!("{}.{}", recv, stat))
}

/// Phase 4 maps a `SetReset` argument to DSL `until end_of_turn` when it
/// uses any of the standard end-of-turn idioms:
///   - explicit `PHASE_END` token
///   - `RESETS_STANDARD` bundle (expands to RESET_PHASE+PHASE_END+...)
///
/// Other reset shapes (battle-step only, chain-only, etc.) are deferred —
/// no `until` clause emitted, leaving the engine's default behavior.
fn reset_is_end_of_turn(reset: Option<&str>) -> bool {
    match reset {
        Some(s) => s.contains("PHASE_END") || s.contains("RESETS_STANDARD"),
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
    fn register_chain_for_loop_uses_group_selector_spec() {
        // Daigusto Falcos: `for tc in aux.Next(g)` where g is a known
        // GetMatchingGroup binding → translator emits modify_atk using
        // the captured SelectorSpec.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(s.filter,tp,LOCATION_MZONE,LOCATION_MZONE,nil)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(600)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        assert_eq!(body.register_chains.len(), 1);
        let chain = &body.register_chains[0];
        assert!(chain.multi_target);
        assert_eq!(chain.loop_source_group.as_deref(), Some("g"));
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) if s.starts_with("modify_atk") => Some(s.as_str()),
            _ => None,
        });
        // Quantity is "all" since GetMatchingGroup is the unfiltered group.
        // Both my_locs and opp_locs are LOCATION_MZONE (non-zero) → either controls.
        assert_eq!(
            action,
            Some("modify_atk (all, card, either controls, from monster_zone) + 600 until end_of_turn"),
            "got lines: {:?}", lines
        );
    }

    #[test]
    fn register_chain_for_loop_without_known_group_skipped() {
        // for-loop iterating an unknown variable (no GetMatchingGroup binding) →
        // multi_target=true but no source spec → translator skips.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    for tc in aux.Next(opaque) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(600)
        tc:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        let lines = translate_body(body);
        let has_modify = lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("modify_atk")));
        assert!(!has_modify, "loop without known source should not emit modify_atk");
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
    fn passive_modifier_spec_extracted_from_equip_chain() {
        // Laser Cannon Armor: e2 is EFFECT_TYPE_EQUIP + UPDATE_ATTACK +
        // SetValue(300) registered on c with no SetOperation/Target.
        let src = r#"
function s.initial_effect(c)
    aux.AddEquipProcedure(c,nil)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_EQUIP)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(300)
    c:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let skel = report.effects.iter().find(|s| s.binding == "e2").expect("e2 skel");
        let spec = skel.passive_modifier_spec().expect("passive spec");
        assert_eq!(spec.stat, "atk");
        assert_eq!(spec.value, 300);
        assert_eq!(spec.effect_type, "EFFECT_TYPE_EQUIP");
        assert_eq!(spec.target, Some("equipped_card"));
        let dsl = spec.to_dsl_block("Equip ATK", "    ");
        assert_eq!(
            dsl,
            "    passive \"Equip ATK\" {\n\
             \x20       target: equipped_card\n\
             \x20       modifier: atk + 300\n\
             \x20   }"
        );
    }

    #[test]
    fn passive_modifier_spec_field_with_target_range() {
        // Threshold Borg: EFFECT_TYPE_FIELD with TargetRange(0, MZONE)
        // → opponent monsters get -500 ATK.
        let src = r#"
function s.initial_effect(c)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetRange(LOCATION_MZONE)
    e2:SetTargetRange(0,LOCATION_MZONE)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(-500)
    c:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let skel = &report.effects[0];
        let spec = skel.passive_modifier_spec().expect("passive spec");
        assert_eq!(spec.scope, Some("field"));
        assert_eq!(spec.target, Some("(all, monster, opponent controls)"));
        let dsl = spec.to_dsl_block("Penalty", "    ");
        assert!(dsl.contains("scope: field"), "got:\n{}", dsl);
        assert!(dsl.contains("target: (all, monster, opponent controls)"));
        assert!(dsl.contains("modifier: atk - 500"));
    }

    #[test]
    fn passive_modifier_spec_field_without_target_range_skipped() {
        let src = r#"
function s.initial_effect(c)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetRange(LOCATION_MZONE)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(100)
    c:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let skel = &report.effects[0];
        assert!(skel.passive_modifier_spec().is_none(),
            "FIELD chain without SetTargetRange should be skipped");
    }

    #[test]
    fn passive_modifier_spec_skips_activated_effects() {
        // Chain with SetOperation is an activated effect — not passive.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_ACTIVATE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(500)
    e1:SetOperation(s.activate)
    c:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let skel = &report.effects[0];
        assert!(skel.passive_modifier_spec().is_none(),
            "skeletons with SetOperation should not be passive candidates");
    }

    #[test]
    fn passive_modifier_spec_skips_non_literal_value() {
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_EQUIP)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(s.atkval)
    c:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let skel = &report.effects[0];
        assert!(skel.passive_modifier_spec().is_none(),
            "non-literal SetValue should not be passive candidate");
    }

    #[test]
    fn passive_modifier_spec_negative_value() {
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_EQUIP)
    e1:SetCode(EFFECT_UPDATE_DEFENSE)
    e1:SetValue(-200)
    c:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let skel = &report.effects[0];
        let spec = skel.passive_modifier_spec().expect("passive spec");
        assert_eq!(spec.stat, "def");
        assert_eq!(spec.value, -200);
        let dsl = spec.to_dsl_block("Penalty", "    ");
        assert!(dsl.contains("modifier: def - 200"), "got:\n{}", dsl);
    }

    #[test]
    fn phase4c_value_method_call_target_atk() {
        // SetValue(tc:GetAttack()) → modify_atk target + target.atk.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(tc:GetAttack())
    e1:SetReset(RESET_EVENT|RESETS_STANDARD)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()), _ => None
        });
        assert_eq!(action, Some("modify_atk target + target.atk until end_of_turn"));
    }

    #[test]
    fn phase4c_value_method_div_literal() {
        // SetValue(tc:GetAttack()/2) → modify_atk target + target.atk / 2.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(tc:GetAttack()/2)
    e1:SetReset(RESETS_STANDARD)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()), _ => None
        });
        assert_eq!(action, Some("modify_atk target + target.atk / 2 until end_of_turn"));
    }

    #[test]
    fn phase4c_value_local_var_resolved() {
        // local atk = c:GetLevel() * 100; SetValue(atk) → resolves through binding.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local atk=tc:GetLevel()*100
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(atk)
    e1:SetReset(RESETS_STANDARD)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        assert!(body.value_bindings.contains_key("atk"),
            "atk should be in value_bindings; got {:?}", body.value_bindings);
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()), _ => None
        });
        assert_eq!(action, Some("modify_atk target + target.level * 100 until end_of_turn"));
    }

    #[test]
    fn phase4c_value_negation() {
        // SetValue(-atk) where atk = c:GetAttack() → modify_atk target - target.atk.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local atk=tc:GetAttack()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(-atk)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()), _ => None
        });
        assert_eq!(action, Some("modify_atk target - target.atk"));
    }

    #[test]
    fn phase4c_value_unknown_skipped() {
        // SetValue(s.atkval) — function ref. Phase 4c does not yet
        // walk function bodies. Should skip emission, not panic.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(s.atkval)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        let has_modify = lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("modify_atk")));
        assert!(!has_modify, "function-ref SetValue should not emit modify_atk");
    }

    #[test]
    fn t10_register_chain_indestructable_battle_target_grant() {
        // Shield Warrior shape: tc:RegisterEffect with
        // EFFECT_INDESTRUCTABLE_BATTLE + RESETS_STANDARD reset →
        // grant target cannot_be_destroyed by battle until end_of_turn.
        let src = r#"
function s.atkop(e,tp,eg,ep,ev,re,r,rp)
    local tc=e:GetLabelObject()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetValue(1)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_DAMAGE)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.atkop").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(
            action,
            Some("grant target cannot_be_destroyed by battle until end_of_turn"),
        );
    }

    #[test]
    fn t10_register_chain_cannot_attack_self_grant() {
        // c:RegisterEffect with EFFECT_CANNOT_ATTACK + RESETS_STANDARD →
        // grant self cannot_attack until end_of_turn.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CANNOT_ATTACK)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("grant self cannot_attack until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_cannot_be_targeted_grant() {
        // tc:RegisterEffect with EFFECT_CANNOT_BE_EFFECT_TARGET +
        // RESETS_STANDARD → grant target cannot_be_targeted until end_of_turn.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CANNOT_BE_EFFECT_TARGET)
    e1:SetValue(1)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(
            action,
            Some("grant target cannot_be_targeted until end_of_turn"),
        );
    }

    #[test]
    fn t10_register_chain_grant_skips_without_reset() {
        // Same code, no SetReset → permanent grant ambiguity → skip.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetValue(1)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        let has_grant = lines
            .iter()
            .any(|l| matches!(l, DslLine::Action(s) if s.starts_with("grant ")));
        assert!(!has_grant, "grant chain without reset should not emit");
    }

    #[test]
    fn t10_register_chain_grant_multi_target() {
        // for tc in aux.Next(g) loop with EFFECT_CANNOT_ATTACK +
        // RESETS_STANDARD → grant <group selector> cannot_attack until end_of_turn.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(s.filter,tp,LOCATION_MZONE,LOCATION_MZONE,nil)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_CANNOT_ATTACK)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) if s.starts_with("grant ") => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(
            action,
            Some("grant (all, card, either controls, from monster_zone) cannot_attack until end_of_turn"),
            "got lines: {:?}", lines,
        );
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

    // ── Phase 6 — condition extraction tests ──────────────────────────────

    fn cond_expr_from_lua(src: &str, handler: &str) -> Option<String> {
        let parsed = full_moon::parse(src).expect("lua parse");
        let mut report = walk(&parsed);
        let body = report.functions.remove(handler)?;
        extract_condition_expr(&body)
    }

    #[test]
    fn phase6_in_gy_and_reason_battle() {
        // Most common shape (75 cards): self is in GY and was sent by battle.
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return e:GetHandler():IsLocation(LOCATION_GRAVE) and e:GetHandler():IsReason(REASON_BATTLE)
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("in_gy and reason == battle".to_string()),
        );
    }

    #[test]
    fn phase6_phase_main1() {
        // Phase predicate (19 cards).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return Duel.IsPhase(PHASE_MAIN1)
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("phase == main1".to_string()),
        );
    }

    #[test]
    fn phase6_previous_location_field() {
        // Previous-location predicate via GetPreviousLocation (17 cards).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return e:GetHandler():GetPreviousLocation()==LOCATION_ONFIELD
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("previous_location == field".to_string()),
        );
    }

    #[test]
    fn phase6_previous_location_gy_is_method() {
        // Previous-location via IsPreviousLocation (alternate API, 10 cards).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return e:GetHandler():IsPreviousLocation(LOCATION_GRAVE)
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("previous_location == gy".to_string()),
        );
    }

    #[test]
    fn phase6_in_gy() {
        // Self-location single atom (8 cards).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return e:GetHandler():IsLocation(LOCATION_GRAVE)
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("in_gy".to_string()),
        );
    }

    #[test]
    fn phase6_phase_battle() {
        // IsBattlePhase shorthand (5 cards).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return Duel.IsBattlePhase()
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("phase == battle".to_string()),
        );
    }

    #[test]
    fn phase6_lp_compare() {
        // LP comparison (3 cards).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return Duel.GetLP(tp)<=3000
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("lp <= 3000".to_string()),
        );
    }

    #[test]
    fn phase6_opponent_lp_compare() {
        // Opponent LP comparison (3 cards).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return Duel.GetLP(1-tp)>=4000
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("opponent_lp >= 4000".to_string()),
        );
    }

    #[test]
    fn phase6_reason_destroy_compound() {
        // Compound: previous_location == field AND reason == destroy (3 cards).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return e:GetHandler():IsPreviousLocation(LOCATION_ONFIELD) and e:GetHandler():IsReason(REASON_DESTROY)
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("previous_location == field and reason == destroy".to_string()),
        );
    }

    #[test]
    fn phase6_not_reason_battle() {
        // Negated atom: not reason == battle (1 card).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return not e:GetHandler():IsReason(REASON_BATTLE) and e:GetHandler():IsPreviousLocation(LOCATION_ONFIELD)
end
"#;
        assert_eq!(
            cond_expr_from_lua(src, "s.condition"),
            Some("not reason == battle and previous_location == field".to_string()),
        );
    }

    #[test]
    fn phase6_multi_line_body_skipped() {
        // Multi-line body should NOT be extracted (complex logic).
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    return c:IsLocation(LOCATION_GRAVE)
end
"#;
        // Local binding before return → body.return_expr is None → no extraction.
        assert_eq!(cond_expr_from_lua(src, "s.condition"), None);
    }

    #[test]
    fn phase6_untranslatable_shape_skipped() {
        // IsTurnPlayer has no grammar atom → should return None.
        let src = r#"
function s.condition(e,tp,eg,ep,ev,re,r,rp)
    return Duel.IsTurnPlayer(tp)
end
"#;
        assert_eq!(cond_expr_from_lua(src, "s.condition"), None);
    }

    // ── Phase 7 — cost block extraction tests ────────────────────────────

    fn cost_block_from_lua(src: &str, handler: &str) -> Option<CostBlockSpec> {
        let parsed = full_moon::parse(src).expect("lua parse");
        let report = walk(&parsed);
        extract_cost_block(handler, &report.functions)
    }

    #[test]
    fn phase7_discard_one_card() {
        // Most common shape: discard 1 card from hand with generic filter.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return Duel.IsDiscardable(e:GetHandler()) end
    Duel.DiscardHand(tp,Card.IsDiscardable,1,1,REASON_COST|REASON_DISCARD)
end
"#;
        let spec = cost_block_from_lua(src, "s.cost").expect("cost block");
        assert_eq!(spec.actions.len(), 1);
        assert_eq!(spec.actions[0], CostAction::Discard("(1, card, you control, from hand)".to_string()));
        // to_dsl_block has no leading indent on the opening line (caller supplies it).
        assert_eq!(
            spec.to_dsl_block("        "),
            "cost {\n            discard (1, card, you control, from hand)\n        }"
        );
    }

    #[test]
    fn phase7_discard_two_cards() {
        // Exact quantity 2.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return Duel.IsDiscardable(e:GetHandler()) end
    Duel.DiscardHand(tp,Card.IsDiscardable,2,2,REASON_COST|REASON_DISCARD)
end
"#;
        let spec = cost_block_from_lua(src, "s.cost").expect("cost block");
        assert_eq!(spec.actions[0], CostAction::Discard("(2, card, you control, from hand)".to_string()));
    }

    #[test]
    fn phase7_discard_custom_filter_skipped() {
        // Custom filter (s.cfilter) → cannot derive generic selector → skip.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return Duel.IsDiscardable(e:GetHandler()) end
    Duel.DiscardHand(tp,s.cfilter,1,1,REASON_COST|REASON_DISCARD)
end
"#;
        assert_eq!(cost_block_from_lua(src, "s.cost"), None);
    }

    #[test]
    fn phase7_pay_lp_literal() {
        // Literal LP cost via Duel.PayLPCost.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return Duel.CheckLPCost(tp,1000) end
    Duel.PayLPCost(tp,1000)
end
"#;
        let spec = cost_block_from_lua(src, "s.cost").expect("cost block");
        assert_eq!(spec.actions[0], CostAction::PayLp("1000".to_string()));
    }

    #[test]
    fn phase7_pay_lp_computed_skipped() {
        // Non-literal LP amount (variable) → bail.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    local lp=Duel.GetLP(tp)-100
    if chk==0 then return Duel.CheckLPCost(tp,lp) end
    Duel.PayLPCost(tp,lp)
end
"#;
        assert_eq!(cost_block_from_lua(src, "s.cost"), None);
    }

    #[test]
    fn phase7_cost_pay_lp_inline() {
        // Cost.PayLP(N) inline — stored directly in cost_handler, no body.
        let parsed = full_moon::parse("function s.initial_effect(c) end").expect("parse");
        let report = walk(&parsed);
        let spec = extract_cost_block("Cost.PayLP(500)", &report.functions).expect("spec");
        assert_eq!(spec.actions[0], CostAction::PayLp("500".to_string()));
    }

    #[test]
    fn phase7_tribute_self() {
        // Release self as cost → tribute self.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return e:GetHandler():IsRelateToEffect(e) end
    Duel.Release(c,REASON_COST)
end
"#;
        let spec = cost_block_from_lua(src, "s.cost").expect("cost block");
        assert_eq!(spec.actions[0], CostAction::Tribute("self".to_string()));
    }

    #[test]
    fn phase7_banish_self() {
        // Remove self face-up as cost → banish self.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return e:GetHandler():IsRelateToEffect(e) end
    Duel.Remove(c,POS_FACEUP,REASON_COST)
end
"#;
        let spec = cost_block_from_lua(src, "s.cost").expect("cost block");
        assert_eq!(spec.actions[0], CostAction::Banish("self".to_string()));
    }

    #[test]
    fn phase7_send_self_to_gy() {
        // SendtoGrave self as cost → send self to gy.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return e:GetHandler():IsRelateToEffect(e) end
    Duel.SendtoGrave(c,REASON_COST)
end
"#;
        let spec = cost_block_from_lua(src, "s.cost").expect("cost block");
        assert_eq!(spec.actions[0], CostAction::SendToGy("self".to_string()));
    }

    #[test]
    fn phase7_banish_self_via_get_handler() {
        // e:GetHandler() is treated as self equivalent.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return e:GetHandler():IsRelateToEffect(e) end
    Duel.Remove(e:GetHandler(),POS_FACEUP,REASON_COST)
end
"#;
        let spec = cost_block_from_lua(src, "s.cost").expect("cost block");
        assert_eq!(spec.actions[0], CostAction::Banish("self".to_string()));
    }

    #[test]
    fn phase7_unknown_duel_call_bails() {
        // Unknown Duel call as a top-level statement → None (skip-not-mis-emit).
        // Duel.SelectMatchingCard is not a recognized cost action.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return false end
    Duel.SelectMatchingCard(tp,s.filter,tp,LOCATION_HAND,0,1,1,nil)
    Duel.DiscardHand(tp,Card.IsDiscardable,1,1,REASON_COST)
end
"#;
        assert_eq!(cost_block_from_lua(src, "s.cost"), None);
    }

    #[test]
    fn phase7_meta_calls_ignored() {
        // SetOperationInfo is a meta call → ignored, cost block still emitted.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return Duel.CheckLPCost(tp,500) end
    Duel.PayLPCost(tp,500)
    Duel.SetOperationInfo(0,CATEGORY_COSTLP,nil,0,tp,500)
end
"#;
        let spec = cost_block_from_lua(src, "s.cost").expect("cost block");
        assert_eq!(spec.actions.len(), 1);
        assert_eq!(spec.actions[0], CostAction::PayLp("500".to_string()));
    }

    // ── Phase 8 — target declaration extraction tests ────────────────────

    fn target_decl_from_lua(src: &str, handler: &str) -> Option<SelectorSpec> {
        let parsed = full_moon::parse(src).expect("lua parse");
        let report = walk(&parsed);
        extract_target_decl(handler, &report.functions)
    }

    #[test]
    fn phase8_nil_filter_either_field() {
        // Most common shape: nil filter, both sides LOCATION_ONFIELD.
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk,chkc)
    if chkc then return chkc:IsOnField() end
    if chk==0 then return Duel.IsExistingTarget(nil,tp,LOCATION_ONFIELD,LOCATION_ONFIELD,1,nil) end
    Duel.SelectTarget(tp,nil,tp,LOCATION_ONFIELD,LOCATION_ONFIELD,1,1,nil)
end
"#;
        let spec = target_decl_from_lua(src, "s.target").expect("spec");
        assert_eq!(spec.quantity, "1");
        assert_eq!(spec.controller, Some("either controls".to_string()));
        assert_eq!(spec.zone, Some("from field".to_string()));
        assert_eq!(spec.to_dsl(), "(1, card, either controls, from field)");
    }

    #[test]
    fn phase8_nil_filter_opponent_mzone() {
        // Opponent monster zone only — typical "destroy opponent monster" shape.
        let src = r#"
function s.destg(e,tp,eg,ep,ev,re,r,rp,chk,chkc)
    if chkc then return chkc:IsLocation(LOCATION_MZONE) and chkc:IsControler(1-tp) end
    if chk==0 then return Duel.IsExistingTarget(nil,tp,0,LOCATION_MZONE,1,nil) end
    Duel.SelectTarget(tp,nil,tp,0,LOCATION_MZONE,1,1,nil)
end
"#;
        let spec = target_decl_from_lua(src, "s.destg").expect("spec");
        assert_eq!(spec.quantity, "1");
        assert_eq!(spec.controller, Some("opponent controls".to_string()));
        assert_eq!(spec.zone, Some("from monster_zone".to_string()));
        assert_eq!(spec.to_dsl(), "(1, card, opponent controls, from monster_zone)");
    }

    #[test]
    fn phase8_aux_true_filter_opponent_field() {
        // aux.TRUE is semantically equivalent to nil — also translatable.
        let src = r#"
function s.destg(e,tp,eg,ep,ev,re,r,rp,chk,chkc)
    if chkc then return chkc:IsOnField() and chkc:IsControler(1-tp) end
    if chk==0 then return Duel.IsExistingTarget(aux.TRUE,tp,0,LOCATION_ONFIELD,1,nil) end
    Duel.SelectTarget(tp,aux.TRUE,tp,0,LOCATION_ONFIELD,1,1,nil)
end
"#;
        let spec = target_decl_from_lua(src, "s.destg").expect("spec");
        assert_eq!(spec.to_dsl(), "(1, card, opponent controls, from field)");
    }

    #[test]
    fn phase8_you_control_mzone() {
        // Your own monster zone.
        let src = r#"
function s.negtg(e,tp,eg,ep,ev,re,r,rp,chk,chkc)
    if chkc then return chkc:IsLocation(LOCATION_MZONE) and chkc:IsControler(tp) end
    if chk==0 then return Duel.IsExistingTarget(nil,tp,LOCATION_MZONE,0,1,nil) end
    Duel.SelectTarget(tp,nil,tp,LOCATION_MZONE,0,1,1,nil)
end
"#;
        let spec = target_decl_from_lua(src, "s.negtg").expect("spec");
        assert_eq!(spec.to_dsl(), "(1, card, you control, from monster_zone)");
    }

    #[test]
    fn phase8_custom_filter_skipped() {
        // Named filter function → skip to avoid mis-emit.
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk,chkc)
    if chkc then return chkc:IsLocation(LOCATION_MZONE) and s.filter(chkc) end
    if chk==0 then return Duel.IsExistingTarget(s.filter,tp,LOCATION_MZONE,LOCATION_MZONE,1,nil) end
    Duel.SelectTarget(tp,s.filter,tp,LOCATION_MZONE,LOCATION_MZONE,1,1,nil)
end
"#;
        assert_eq!(target_decl_from_lua(src, "s.target"), None);
    }

    #[test]
    fn phase8_variable_quantity_skipped() {
        // Non-literal max (variable ct) → skip.
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk,chkc)
    local ct=3
    if chk==0 then return Duel.IsExistingTarget(nil,tp,LOCATION_ONFIELD,LOCATION_ONFIELD,1,nil) end
    Duel.SelectTarget(tp,nil,tp,LOCATION_ONFIELD,LOCATION_ONFIELD,1,ct,nil)
end
"#;
        assert_eq!(target_decl_from_lua(src, "s.target"), None);
    }

    #[test]
    fn phase8_two_targets_fixed_qty() {
        // Fixed quantity 2.
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk,chkc)
    if chkc then return chkc:IsOnField() end
    if chk==0 then return Duel.IsExistingTarget(nil,tp,LOCATION_ONFIELD,LOCATION_ONFIELD,2,nil) end
    Duel.SelectTarget(tp,nil,tp,LOCATION_ONFIELD,LOCATION_ONFIELD,2,2,nil)
end
"#;
        let spec = target_decl_from_lua(src, "s.target").expect("spec");
        assert_eq!(spec.quantity, "2");
        assert_eq!(spec.to_dsl(), "(2, card, either controls, from field)");
    }
}
