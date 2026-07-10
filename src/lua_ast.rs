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

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::sync::{Mutex, OnceLock};

use full_moon::ast;
use full_moon::ast::{Stmt, Expression, FunctionCall, Suffix, Call, Index, Block, LastStmt};

/// Top-level analysis report for one Lua file.
#[derive(Debug, Default)]
pub struct LuaReport {
    pub effects: Vec<EffectSkeleton>,
    pub functions: BTreeMap<String, FunctionBody>,
    pub parse_error: Option<String>,
    /// Source ordinals of chains that may own a .ds effect block but have
    /// no skeleton in `effects` (Phase 20): clones of summon-helper chains
    /// and clones that never register. The block matcher aligns them as
    /// unknown-signature entities so downstream chains can re-anchor.
    pub phantom_block_ordinals: Vec<usize>,
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
    /// Counter-manipulation method calls (`<recv>:AddCounter(...)` /
    /// `<recv>:RemoveCounter(...)`) found in the body (Phase 13). Empty
    /// when the body had none OR when an op appeared inside an
    /// elseif/else arm — emitting one arm of a runtime either/or would
    /// mis-emit, so the whole body's counter ops are poisoned instead.
    pub counter_ops: Vec<CounterOp>,
    /// Straight-line `local op=Duel.SelectOption(tp, aux.Stringid(id,a), …)`
    /// + `e:SetLabel(op)` pair found at the top level of a TARGET handler
    /// (Phase 16). None when absent, conditional (dynamic option list), or
    /// ambiguous (multiple SelectOption / SetLabel writes).
    pub select_option: Option<SelectOptionSpec>,
    /// Arm bodies of a `local op=e:GetLabel()` + `if op==0 … elseif op==1 …`
    /// label dispatch found in an OPERATION handler (Phase 16). One entry
    /// per arm in option order; empty when the body has no clean dispatch
    /// (untranslatable surrounding statements, non-contiguous labels, …).
    pub choice_arms: Vec<FunctionBody>,
    /// Statement-level calls to script-local helpers (`s.equipop(c,e,tp,tc)`)
    /// found in the body, in source order (Phase 17). Emit time resolves
    /// the head against `LuaReport::functions` to recognise known helper
    /// idioms (select-then-equip).
    pub helper_calls: Vec<(String, Vec<String>)>,
    /// Set when THIS function is a select-then-equip helper: its body
    /// leads with the canonical
    /// `<host>:EquipByEffectAndLimitRegister(e,tp,<target>,…)` statement
    /// (bare, or wrapped in `if not … then return end`). Callers emit
    /// `equip <selection> to self` when the call-site args resolve.
    pub equip_helper: Option<EquipHelperSpec>,
    /// Provenance of derived selection locals (Phase 17): maps a name
    /// bound via `local tc = g:GetFirst()` to its source group binding
    /// `g`. Used to fold alias sets when counting selection consumers.
    pub select_sources: BTreeMap<String, String>,
    /// The (cond, then-expr, else-expr) triple when the body is exactly
    /// the guarded-value idiom (Phase 19): a sole
    /// `if <cond> then return <A> else return <B> end` statement, or
    /// `if <cond> then return <A> end` followed by a bare `return <B>`.
    /// Damage-shaping `SetValue` guards use this shape
    /// (`if (r&REASON_EFFECT)~=0 then return 0 else return val end`).
    /// Multi-statement bodies (local bindings, side effects, elseif
    /// arms) stay None — same conservatism as `return_expr`.
    pub guarded_return: Option<GuardedReturn>,
    /// Declared parameter names, in source order (T38 S1). Named-filter
    /// resolution (`Duel.GetMatchingGroup(s.filter, …)`) requires every
    /// predicate atom in the filter's `return_expr` to be a method call
    /// on the FIRST parameter — an atom touching any other binding (the
    /// effect `e`, an outer local, a second param) is not a per-card
    /// predicate and poisons the mapping.
    pub params: Vec<String>,
}

/// A guarded-value function body (Phase 19): `if cond then return
/// then_expr else return else_expr end` — with the else branch either
/// an explicit `else` block or a trailing bare `return`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedReturn {
    pub cond: String,
    pub then_expr: String,
    pub else_expr: String,
}

/// Parameter mapping for a select-then-equip helper function (Phase 17):
/// which positional call-site args carry the equip host (must resolve to
/// the card itself) and the equip target (must resolve to a one-card
/// selection binding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquipHelperSpec {
    pub host_param: usize,
    pub target_param: usize,
}

/// Phase 16 — the option list of a straight-line `Duel.SelectOption`
/// whose result was stored via `e:SetLabel(op)`. `string_ids` holds the
/// second args of the `aux.Stringid(id, n)` description args, in lua
/// argument order — they index the card's CDB strs (str1 = index 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOptionSpec {
    pub string_ids: Vec<u32>,
}

/// One `<receiver>:AddCounter(...)` / `<receiver>:RemoveCounter(...)`
/// statement extracted from a function body (Phase 13).
///
/// Arg conventions on the lua side:
///   - `AddCounter(countertype, count[, singly])`
///   - `RemoveCounter(player, countertype, count, reason)`
/// Both `countertype` and `count` are kept as raw text; emit time
/// resolves the counter NAME via the strings.conf table and requires a
/// literal count (the DSL grammar slot is `unsigned`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CounterOp {
    /// True for AddCounter (→ `place_counter`), false for RemoveCounter
    /// (→ `remove_counter`).
    pub add: bool,
    /// Receiver text — `c`, `tc`, `e:GetHandler()`, loop vars, …
    pub receiver: String,
    /// Raw countertype arg, possibly a `COUNTER_NEED_ENABLE+COUNTER_X` sum.
    pub counter_arg: String,
    /// Raw count arg. Non-literal counts skip at emit (grammar: unsigned).
    pub count_arg: String,
    /// RemoveCounter's player arg (`tp`, …). Empty for AddCounter.
    pub player_arg: String,
    /// True when the op sat inside any loop (aux.Next / numeric-for /
    /// while / repeat). Only aux.Next-style loops with a mapped source
    /// group are translatable; the rest skip.
    pub multi_target: bool,
    /// Source-group binding when inside `for <var> in aux.Next(g)`.
    pub loop_source_group: Option<String>,
    /// The loop variable when inside an aux.Next-style loop. The emit
    /// path requires `receiver == loop_var` — an op on `c` inside a
    /// group loop targets the card itself, not the group members.
    pub loop_var: Option<String>,
    /// True when the op sat inside any `if`/`while` block. Resolve
    /// emission tolerates this (the IsRelateToEffect gate idiom), cost
    /// extraction does not (a conditional payment is not a fixed cost).
    pub in_branch: bool,
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
    /// Function name passed to `e:SetOperation(...)`. Carries the sub-handler
    /// the chain delegates to when its event code fires (install_watcher
    /// translation path).
    pub operation: Option<String>,
    /// Function name passed to `e:SetCondition(...)`. The install_watcher
    /// path skips any chain that carries one — the watcher grammar has no
    /// condition slot, so emitting would drop the gate (Tyrant Wing e9).
    pub condition: Option<String>,
    /// Raw `SetTargetRange(a, b)` args joined with `,` (Phase 15). For
    /// `EFFECT_TYPE_FIELD` chains this names WHICH cards (LOCATION masks)
    /// or players (1/0 flags) the effect reaches, relative to the
    /// registering player.
    pub target_range: Option<String>,
    /// Raw first arg of `SetTarget(...)` (Phase 15). For FIELD chains this
    /// is the affected-card filter `(e, c) → bool`; resolve-time emit maps
    /// it to a DSL selector kind/where-clause or skips.
    pub set_target: Option<String>,
    /// Raw second arg of `SetReset(reset, count)` when present (Phase 15).
    /// A count above 1 means the effect survives multiple reset events
    /// (e.g. `RESET_PHASE|PHASE_END, 2` = end of NEXT turn) — not a
    /// standard end-of-turn lifetime, so reset-gated emits must skip.
    /// Overwritten (not merged) on every SetReset write: an absent count
    /// arg means the lua default of 1.
    pub reset_count: Option<String>,
    /// True when the chain was committed via `Duel.RegisterEffect(eN, p)`
    /// rather than `<card>:RegisterEffect(eN)` (Phase 15). Field-scope
    /// restriction emit requires this — a card-registered FIELD chain's
    /// lifetime is tied to the card's presence, which the resolve-time
    /// grant form can't express.
    pub duel_registered: bool,
    /// True when the registration site sat inside an arm of an
    /// if/elseif/else that HAS alternative arms (Phase 15) — the
    /// either/or idiom (`SelectEffect` label dispatch, …). Emitting one
    /// arm unconditionally mis-states the card, so choice-arm chains are
    /// skipped (same rationale as the counter-op elseif/else poison;
    /// caught live on c55948544 Judgment of the Pharaoh). Plain
    /// `if`-gated chains keep emitting — the ubiquitous
    /// IsRelateToEffect guard wraps most operation bodies.
    pub in_choice_arm: bool,
    /// True when the same Set* slot was written twice with DIFFERENT args
    /// before registration — the branch-conditional idiom
    /// (`if … then e1:SetValue(a) else e1:SetValue(b) end`). The straight-
    /// line extractor keeps only the last write, so the recorded payload is
    /// one arm of a runtime choice; translating it would mis-emit (Phase 11
    /// guard, found via Spellbook of Wisdom's spells-or-traps immunity).
    pub conflicting_sets: bool,
    /// Slot names whose values were inherited from `eN:Clone()` rather
    /// than written on this binding. The first Set* on a seeded slot is
    /// the linear clone-then-override idiom (`e2=e1:Clone()` +
    /// `e2:SetCode(...)`) — it REPLACES the inherited value and clears
    /// the mark. Only a second differing write after that is a real
    /// branch conflict.
    pub clone_seeded: BTreeSet<&'static str>,
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
    /// False when the lua-side filter predicate (`s.filter`,
    /// `aux.FaceupFilter(…)`, …) had no DSL equivalent and was dropped.
    /// Action selectors tolerate the over-approximation (the engine picks
    /// from a superset), but group-applied modifiers must not — applying a
    /// stat change to the unfiltered group alters cards the lua never
    /// touched, so those paths skip when this is false (Phase 10).
    pub filter_mapped: bool,
    /// True only for `Duel.SelectMatchingCard` — a fresh resolve-time
    /// pick (Phase 17). `Duel.SelectTarget` (declared-target pick) and
    /// `Duel.GetMatchingGroup` (no pick at all) stay false: the chain-
    /// selector path folds ONLY resolve-time selections, preserving the
    /// Phase 13b skip for SelectTarget-provenance receivers.
    pub from_resolve_select: bool,
    /// Raw lua filter arg of the producing `Duel.GetMatchingGroup` call
    /// (T38 S1). When `filter_mapped` is false and this names a script-
    /// local function (`s.filter`), emit time re-resolves it against the
    /// walked function table — the filter body isn't available at walk
    /// time (spec extraction runs per handler, before/independent of the
    /// filter fn's own walk). None on select-provenance specs: the
    /// refinement is scoped to group-applied modifier receivers.
    pub raw_filter: Option<String>,
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
    /// True when this skeleton was created via `Fusion.CreateSummonEff(...)`
    /// rather than the usual `Effect.CreateEffect(c)` + Set* chain. The
    /// fusion helper handles its own UI / operation internally, so the
    /// translator emits a fixed `fusion_summon (1, fusion monster)` line
    /// instead of walking a handler body.
    pub fusion_summon_spec: bool,
    /// True when this skeleton stands in for a `Ritual.AddProcEqual` /
    /// `AddProcGreater` / `CreateProc` / `AddWholeLevelTribute` call. The
    /// ritual helpers attach an activation effect that runs the full
    /// ritual procedure internally; translator emits a fixed
    /// `ritual_summon (1, ritual monster)` line.
    pub ritual_summon_spec: bool,
    /// Parameterized fusion/ritual helper captured from
    /// `eN:SetOperation(Fusion.SummonEffOP(...))` or
    /// `eN:SetOperation(Ritual.Operation(...))` (Phase 12). Params are
    /// raw lua strings; decode happens at emit time so unknown shapes
    /// drop out cleanly instead of mis-emitting.
    pub summon_helper_op: Option<SummonHelperOp>,
    /// True when a chain that owns a .ds effect block WITHOUT consuming
    /// a Pass-A block index precedes this skeleton in `s.initial_effect`
    /// source order: a bare `EFFECT_TYPE_ACTIVATE` chain with no
    /// SetOperation, a clone of a summon-helper chain, or an unregistered
    /// clone (plain registered clones are seeded into real skeletons and
    /// consume indices like any other chain — Phase 14).
    /// Positional block mapping is off-by-N for this effect; only a
    /// signature-forced block assignment (Phase 20 matcher) may fill it.
    pub block_alignment_hazard: bool,
    /// Position of this chain among the effect-bearing chains of
    /// `s.initial_effect`, in source order (Phase 20). The block matcher
    /// aligns entities in this order; note `LuaReport::effects` itself is
    /// in BTreeMap (binding-name) order, which usually coincides.
    pub source_ordinal: usize,
    /// Set* method names whose `set_calls` entries were inherited from
    /// `eN:Clone()` rather than written on this binding. The first Set*
    /// on a seeded method is the clone-then-override idiom — it replaces
    /// ALL inherited entries for that method and clears the mark
    /// (slot-level precedent: [`RegisterEffectChain::clone_seeded`]).
    pub clone_seeded: BTreeSet<String>,
}

/// Which parameterized summon helper produced a [`SummonHelperOp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummonHelperKind {
    /// `Fusion.SummonEffOP(...)` — proc_fusion_spell.lua's operation
    /// factory. Positional params: fusfilter, matfilter, extrafil,
    /// extraop, gc, stage2, exactcount, value, location, chkf,
    /// preselect, nosummoncheck, mincount, maxcount, sumpos.
    FusionSummonEffOp,
    /// `Ritual.Operation(...)` — proc_ritual.lua's operation factory.
    /// Positional params: filter, lvtype, lv, extrafil, extraop,
    /// matfilter, stage2, location, forcedselection, customoperation,
    /// specificmatfilter, requirementfunc, sumpos, self.
    RitualOperation,
}

/// A parameterized summon-helper call found as a `SetOperation` argument.
/// Both helpers are `aux.FunctionWithNamedArgs` wrappers, so card scripts
/// call them with positional args, a named-args table, or the
/// `table.unpack(local_table)` idiom — all three normalize into
/// `(param name, raw lua value)` pairs here, with `nil` values and the
/// no-op `handler` key dropped.
#[derive(Debug, Clone)]
pub struct SummonHelperOp {
    pub kind: SummonHelperKind,
    pub params: Vec<(String, String)>,
    /// Param shape couldn't be decoded safely: the local table failed
    /// the single-assignment / no-mutation taint check, had
    /// expression-keyed or mixed fields, or the ident never resolved.
    /// Emit must skip (skip-not-mis-emit).
    pub unresolved: bool,
}

/// Positional parameter names of `Fusion.SummonEffOP` (from
/// CardScripts/proc_fusion_spell.lua's FunctionWithNamedArgs list, minus
/// the target-only names that the OP factory does not take).
const FUSION_SUMMON_EFF_OP_PARAMS: &[&str] = &[
    "fusfilter", "matfilter", "extrafil", "extraop", "gc", "stage2",
    "exactcount", "value", "location", "chkf", "preselect",
    "nosummoncheck", "mincount", "maxcount", "sumpos",
];

/// Positional parameter names of `Ritual.Operation` (from
/// CardScripts/proc_ritual.lua's FunctionWithNamedArgs list).
const RITUAL_OPERATION_PARAMS: &[&str] = &[
    "filter", "lvtype", "lv", "extrafil", "extraop", "matfilter",
    "stage2", "location", "forcedselection", "customoperation",
    "specificmatfilter", "requirementfunc", "sumpos", "self",
];

/// Positional parameter names of `Fusion.CreateSummonEff` after the
/// leading `handler` arg (from proc_fusion_spell.lua's
/// FunctionWithNamedArgs list). Superset of the OP factory's params:
/// adds the cosmetic `desc` and the target-side `extratg`.
const FUSION_CREATE_SUMMON_EFF_PARAMS: &[&str] = &[
    "fusfilter", "matfilter", "extrafil", "extraop", "gc", "stage2",
    "exactcount", "value", "location", "chkf", "desc", "preselect",
    "nosummoncheck", "extratg", "mincount", "maxcount", "sumpos",
];

/// Positional parameter names of `Ritual.CreateProc` after the leading
/// `handler` arg (from proc_ritual.lua). `lvtype` comes first.
const RITUAL_CREATE_PROC_PARAMS: &[&str] = &[
    "lvtype", "filter", "lv", "desc", "extrafil", "extraop", "matfilter",
    "stage2", "location", "forcedselection", "customoperation",
    "specificmatfilter", "requirementfunc", "sumpos", "extratg", "self",
];

/// Positional parameter names of `Ritual.AddProcGreater` /
/// `Ritual.AddProcEqual` after the leading `handler` arg — same list as
/// CreateProc minus `lvtype` (the level procedure is implied).
const RITUAL_ADD_PROC_LEVEL_PARAMS: &[&str] = &[
    "filter", "lv", "desc", "extrafil", "extraop", "matfilter",
    "stage2", "location", "forcedselection", "customoperation",
    "specificmatfilter", "requirementfunc", "sumpos", "extratg", "self",
];

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

    /// If this skeleton is a stat-modifier passive
    /// (`SetCode(EFFECT_UPDATE_ATTACK|DEFENSE)` + translatable
    /// `SetValue`, no activated-effect handlers), return the spec.
    ///
    /// `SetValue` accepts a literal integer, or (T34) an inline closure
    /// whose body is a single-step overlay/counter or group-count
    /// product — see `passive_value_expr`. Closure bodies that read the
    /// closure's card param only qualify for the `EFFECT_TYPE_SINGLE`
    /// self shape: in EQUIP/FIELD chains that param is each *affected*
    /// card, which DSL `self` would misread. Card-param-free count
    /// bodies (scoped via `e:GetHandlerPlayer()` or a player-symmetric
    /// literal) qualify in every chain shape.
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
        let effect_type = self.first_arg_of("SetType").unwrap_or("").to_string();
        let scope_target = derive_passive_scope_target(
            &effect_type,
            self.args_of("SetTargetRange"),
        )?;
        let raw_value = self.first_arg_of("SetValue")?;
        let self_scoped = scope_target.scope.is_none() && scope_target.target.is_none();
        let (negative, value) = if let Ok(n) = raw_value.parse::<i64>() {
            (n < 0, n.unsigned_abs().to_string())
        } else {
            passive_value_expr(raw_value, self_scoped)?
        };
        Some(PassiveModifierSpec {
            stat: stat.to_string(),
            negative,
            value,
            effect_type,
            scope: scope_target.scope,
            target: scope_target.target,
        })
    }

    /// DSL summon line for skeletons backed by a fusion/ritual helper.
    ///
    /// Both the plain factory forms (`Fusion.CreateSummonEff` /
    /// `Ritual.AddProc*` / `Ritual.CreateProc`) and the parameterized
    /// `SetOperation` helpers (Fusion.SummonEffOP / Ritual.Operation)
    /// decode their params into selector constraints; a call with no
    /// restrictive params emits the bare summon line. Returns None when
    /// params have no DSL equivalent — the resolve stays empty rather
    /// than mis-emitting an over-permissive bare line.
    pub fn summon_helper_line(&self) -> Option<String> {
        // Phase 20: the block-alignment gate lives in
        // `block_match::compute_assignments` — a hazard-flagged helper
        // emits its line only when the matcher forces a block for it.
        let op = self.summon_helper_op.as_ref()?;
        if op.unresolved { return None; }
        match op.kind {
            SummonHelperKind::FusionSummonEffOp => fusion_helper_line(&op.params),
            SummonHelperKind::RitualOperation => ritual_helper_line(&op.params),
        }
    }

    /// True when this skeleton is backed by any fusion/ritual summon
    /// helper. Such chains own a .ds effect block (the activation shell)
    /// even when the emit policy declines a line, so the apply tool's
    /// positional block walks must consume an index for them.
    pub fn is_summon_helper(&self) -> bool {
        self.summon_helper_op.is_some()
            || self.fusion_summon_spec
            || self.ritual_summon_spec
    }
}

/// Emit `fusion_summon ...` from decoded `Fusion.SummonEffOP` params.
///
/// EMIT: `fusfilter` mapping to a where-clause (archetype / race /
/// attribute / level), `matfilter` mapping to a material selector. Any
/// other param (extrafil, extraop, gc, stage2, extratg, location, …) has
/// no DSL equivalent — the whole line skips because dropping the param
/// would change semantics (e.g. gc forces a specific material).
fn fusion_helper_line(params: &[(String, String)]) -> Option<String> {
    let mut where_clause: Option<String> = None;
    let mut using: Option<&'static str> = None;
    for (k, v) in params {
        match k.as_str() {
            "fusfilter" => where_clause = Some(summon_filter_to_where(v)?),
            "matfilter" => using = Some(fusion_matfilter_to_using(v)?),
            _ => return None,
        }
    }
    let sel = match &where_clause {
        Some(w) => format!("(1, fusion monster, where {w})"),
        None => "(1, fusion monster)".to_string(),
    };
    Some(match using {
        Some(u) => format!("fusion_summon {sel} using {u}"),
        None => format!("fusion_summon {sel}"),
    })
}

/// Emit `ritual_summon ...` from decoded `Ritual.Operation` params.
///
/// EMIT: `filter` mapping to a where-clause; `lvtype` of
/// RITPROC_GREATER / RITPROC_EQUAL (both already shipped as the fixed
/// line for Ritual.AddProcGreater / AddProcEqual — the level procedure
/// is the helper's standard behavior, not a DSL-visible constraint).
/// Any other param (lv override, forcedselection, customoperation,
/// requirementfunc, …) skips.
fn ritual_helper_line(params: &[(String, String)]) -> Option<String> {
    let mut where_clause: Option<String> = None;
    for (k, v) in params {
        match k.as_str() {
            "filter" => where_clause = Some(summon_filter_to_where(v)?),
            "lvtype" if v == "RITPROC_GREATER" || v == "RITPROC_EQUAL" => {}
            _ => return None,
        }
    }
    Some(match &where_clause {
        Some(w) => format!("ritual_summon (1, ritual monster, where {w})"),
        None => "ritual_summon (1, ritual monster)".to_string(),
    })
}

/// Map a fusion `matfilter` param to a DSL `using` material selector.
fn fusion_matfilter_to_using(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        // Bare `Fusion.OnFieldMat` fn-ref: restricts the default
        // material pool (hand + field) to the player's field.
        "Fusion.OnFieldMat" => Some("(all, monster, you control)"),
        _ => None,
    }
}

/// Map a summoned-monster filter param (`fusfilter` / ritual `filter`)
/// to a DSL where-clause. Handles the two corpus idioms:
/// `aux.FilterBoolFunction(Card.IsX, ...)` and the single-predicate
/// closure `function(c) return c:IsX(...) end`. Returns None for
/// anything else — caller skips the whole line.
fn summon_filter_to_where(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if let Some(inner) = raw
        .strip_prefix("aux.FilterBoolFunction(")
        .and_then(|r| r.strip_suffix(')'))
    {
        let (method, arg) = inner.split_once(',')?;
        return filter_predicate_to_where(method.trim(), arg.trim());
    }
    closure_filter_to_where(raw)
}

/// Map a `Card.Is*` predicate + raw constant arg to a where-clause.
/// Multi-constant forms (`{SET_A,SET_B}` lists, `RACE_A|RACE_B` masks)
/// expand to `or`-joined atoms — lua treats them as any-of.
fn filter_predicate_to_where(method: &str, arg: &str) -> Option<String> {
    let join = |parts: Vec<String>| {
        if parts.is_empty() { None } else { Some(parts.join(" or ")) }
    };
    match method {
        "Card.IsSetCard" => {
            let consts: Vec<&str> = match arg.strip_prefix('{').and_then(|a| a.strip_suffix('}')) {
                Some(list) => list.split(',').map(str::trim).collect(),
                None => vec![arg],
            };
            let mut parts = Vec::new();
            for c in consts {
                parts.push(format!("archetype == \"{}\"", setcode_const_to_archetype(c)?));
            }
            join(parts)
        }
        "Card.IsRace" => {
            let mut parts = Vec::new();
            for c in arg.split('|').map(str::trim) {
                parts.push(format!("race == {}", race_const_to_dsl(c)?));
            }
            join(parts)
        }
        "Card.IsAttribute" => {
            let mut parts = Vec::new();
            for c in arg.split('|').map(str::trim) {
                parts.push(format!("attribute == {}", attribute_const_to_dsl(c)?));
            }
            join(parts)
        }
        _ => None,
    }
}

/// Map a single-predicate closure filter (`function(c) return
/// c:IsSetCard(SET_X) end` and friends) to a where-clause. Closures
/// with any extra logic (conjunctions, comparisons against other
/// bindings, …) fail the strict shape checks and return None.
fn closure_filter_to_where(raw: &str) -> Option<String> {
    let r = raw.strip_prefix("function")?.trim_start();
    let r = r.strip_prefix('(')?;
    let (param, r) = r.split_once(')')?;
    let param = param.trim();
    if param.is_empty() || !param.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let r = r.trim_start().strip_prefix("return")?;
    let body = r.trim().strip_suffix("end")?.trim();
    let (recv, rest) = body.split_once(':')?;
    if recv.trim() != param { return None; }
    let (method, rest) = rest.split_once('(')?;
    let arg = rest.strip_suffix(')')?.trim();
    match method.trim() {
        "IsSetCard" => filter_predicate_to_where("Card.IsSetCard", arg),
        "IsRace" => filter_predicate_to_where("Card.IsRace", arg),
        "IsAttribute" => filter_predicate_to_where("Card.IsAttribute", arg),
        "IsLevelAbove" => {
            let n: u32 = arg.parse().ok()?;
            Some(format!("level >= {n}"))
        }
        _ => None,
    }
}

/// True when `s` is one balanced `( … )` group covering the whole string.
fn fully_parenthesized(s: &str) -> bool {
    let b = s.as_bytes();
    if b.first() != Some(&b'(') || b.last() != Some(&b')') { return false; }
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 { return i == b.len() - 1; }
            }
            _ => {}
        }
    }
    false
}

/// Map one `Card.Is*` predicate (bare name, raw arg text) to a DSL
/// where-clause fragment (T38 S1). Extends the Phase 12 trio
/// (SetCard/Race/Attribute) with the numeric-bound, exact-stat, and
/// type-bit predicates the stat-chip filter corpus uses. Every unknown
/// name or non-literal arg returns None — poisoning the whole filter
/// (skip-not-mis-emit).
fn card_pred_to_where(name: &str, args: &str) -> Option<String> {
    // Numeric-bound family: IsLevelAbove(n) is level >= n (inclusive on
    // the lua side), IsLevelBelow(n) is level <= n.
    let bound = |field: &str, op: &str| -> Option<String> {
        let n: u32 = args.trim().parse().ok()?;
        Some(format!("{field} {op} {n}"))
    };
    // Exact-value family: lua `IsLevel(a, b, …)` is true for ANY listed
    // value — or-join, one atom per literal.
    let exact = |field: &str| -> Option<String> {
        let mut parts = Vec::new();
        for a in args.split(',') {
            let n: u32 = a.trim().parse().ok()?;
            parts.push(format!("{field} == {n}"));
        }
        if parts.is_empty() { None } else { Some(parts.join(" or ")) }
    };
    match name {
        "IsSetCard"   => filter_predicate_to_where("Card.IsSetCard", args),
        "IsRace"      => filter_predicate_to_where("Card.IsRace", args),
        "IsAttribute" => filter_predicate_to_where("Card.IsAttribute", args),
        "IsFaceup"    if args.is_empty() => Some("is_face_up".to_string()),
        "IsFacedown"  if args.is_empty() => Some("is_face_down".to_string()),
        "IsMonster"   if args.is_empty() => Some("is_monster".to_string()),
        "IsSpell"     if args.is_empty() => Some("is_spell".to_string()),
        "IsTrap"      if args.is_empty() => Some("is_trap".to_string()),
        // Ritual/Fusion/Synchro/Xyz/Link "monster" helpers are the type
        // bit AND the monster supertype — two flat atoms.
        "IsRitualMonster"  if args.is_empty() => Some("is_monster and is_ritual".to_string()),
        "IsFusionMonster"  if args.is_empty() => Some("is_monster and is_fusion".to_string()),
        "IsSynchroMonster" if args.is_empty() => Some("is_monster and is_synchro".to_string()),
        "IsXyzMonster"     if args.is_empty() => Some("is_monster and is_xyz".to_string()),
        "IsLinkMonster"    if args.is_empty() => Some("is_monster and is_link".to_string()),
        "IsLevelAbove"   => bound("level", ">="),
        "IsLevelBelow"   => bound("level", "<="),
        "IsRankAbove"    => bound("rank", ">="),
        "IsRankBelow"    => bound("rank", "<="),
        "IsLinkAbove"    => bound("link", ">="),
        "IsLinkBelow"    => bound("link", "<="),
        "IsAttackAbove"  => bound("atk", ">="),
        "IsAttackBelow"  => bound("atk", "<="),
        "IsDefenseAbove" => bound("def", ">="),
        "IsDefenseBelow" => bound("def", "<="),
        "IsLevel" => exact("level"),
        "IsRank"  => exact("rank"),
        "IsLink"  => exact("link"),
        // Type-bit test: lua `IsType(a|b)` is true when ANY bit matches.
        "IsType" => {
            let mut parts = Vec::new();
            for c in args.split('|') {
                parts.push(type_const_to_pred(c.trim())?.to_string());
            }
            if parts.is_empty() { None } else { Some(parts.join(" or ")) }
        }
        _ => None,
    }
}

/// TYPE_* constant → DSL `is_*` predicate atom (grammar `pred_atom`).
fn type_const_to_pred(c: &str) -> Option<&'static str> {
    Some(match c {
        "TYPE_MONSTER"  => "is_monster",
        "TYPE_SPELL"    => "is_spell",
        "TYPE_TRAP"     => "is_trap",
        "TYPE_NORMAL"   => "is_normal",
        "TYPE_EFFECT"   => "is_effect",
        "TYPE_FUSION"   => "is_fusion",
        "TYPE_RITUAL"   => "is_ritual",
        "TYPE_SYNCHRO"  => "is_synchro",
        "TYPE_XYZ"      => "is_xyz",
        "TYPE_PENDULUM" => "is_pendulum",
        "TYPE_LINK"     => "is_link",
        "TYPE_TUNER"    => "is_tuner",
        "TYPE_FLIP"     => "is_flip",
        "TYPE_TOKEN"    => "is_token",
        _ => return None,
    })
}

/// `Card.Is*`-style predicate with the `Card.` prefix (the
/// `aux.FaceupFilter(Card.IsX, ARG)` composition path).
fn lua_pred_method_to_where(method: &str, arg: &str) -> Option<String> {
    let name = method.strip_prefix("Card.")?;
    card_pred_to_where(name, arg)
}

/// Split a lua boolean expression at its top-level `and`/`or` keywords
/// (paren-depth aware, whitespace-delimited). Returns the operand slices
/// and the connective. MIXED `and`+`or` at one level returns None — the
/// DSL predicate chain is flat and precedence-free, so a faithful
/// rendering needs the lua side's explicit parens (which recurse through
/// the paren branch instead).
fn split_top_level_bool<'a>(expr: &'a str) -> Option<(Vec<&'a str>, &'static str)> {
    let b = expr.as_bytes();
    let mut depth = 0i32;
    let mut cuts: Vec<(usize, usize, &'static str)> = Vec::new(); // (start, end, op)
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth -= 1,
            b' ' if depth == 0 => {
                let rest = &expr[i..];
                if let Some(r) = rest.strip_prefix(" and ") {
                    let _ = r;
                    cuts.push((i, i + 5, "and"));
                    i += 4;
                } else if rest.starts_with(" or ") {
                    cuts.push((i, i + 4, "or"));
                    i += 3;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if cuts.is_empty() { return None; }
    let op = cuts[0].2;
    if cuts.iter().any(|c| c.2 != op) { return None; }
    let mut parts = Vec::new();
    let mut start = 0usize;
    for &(lo, hi, _) in &cuts {
        parts.push(expr[start..lo].trim());
        start = hi;
    }
    parts.push(expr[start..].trim());
    if parts.iter().any(|p| p.is_empty()) { return None; }
    Some((parts, op))
}

/// Map a whole lua filter-predicate expression over `param` to a DSL
/// where-clause (T38 S1). Handles `and`-chains, `or`-chains, redundant
/// parens, `not`, `<param>:Is*(…)` predicate atoms, and
/// `<param>:Get<Stat>() <cmp> <int>` stat compares. Any atom outside
/// that set (a second binding, an outer local, an unknown method, a
/// non-literal operand) poisons the WHOLE expression — group-applied
/// modifiers must match the lua set exactly, never a superset.
///
/// The rendered clause is always a FLAT homogeneous chain: the v2
/// parser's parenthesized-predicate branch is currently broken (a
/// `(A and B)` where-group parses as `not A`; or-groups error), so any
/// mixed `and`/`or` structure that would need grouping returns None
/// (task_bb225d5f tracks the parser fix; those cards are S1b backlog).
fn lua_filter_expr_to_where(expr: &str, param: &str) -> Option<String> {
    let e = expr.trim();
    if e.is_empty() { return None; }
    // Parens only group; strip a full-cover pair and recurse. The
    // flat-chain homogeneity checks below decide expressibility.
    if fully_parenthesized(e) {
        return lua_filter_expr_to_where(&e[1..e.len() - 1], param);
    }
    if let Some((parts, op)) = split_top_level_bool(e) {
        let other = if op == "and" { " or " } else { " and " };
        let mut mapped = Vec::new();
        for p in parts {
            let m = lua_filter_expr_to_where(p, param)?;
            // Same-connective children flatten into the chain; a child
            // carrying the OTHER connective would need parens → None.
            if m.contains(other) { return None; }
            mapped.push(m);
        }
        return Some(mapped.join(&format!(" {} ", op)));
    }
    if let Some(rest) = e.strip_prefix("not ") {
        let m = lua_filter_expr_to_where(rest.trim(), param)?;
        // `not` binds one atom in the DSL; a compound operand would
        // need the broken paren branch → None.
        if m.contains(" and ") || m.contains(" or ") { return None; }
        return Some(format!("not {}", m));
    }
    lua_filter_atom_to_where(e, param)
}

/// One predicate atom: `<param>:IsX(args)` or `<param>:Get<Stat>() <cmp> <int>`.
fn lua_filter_atom_to_where(atom: &str, param: &str) -> Option<String> {
    let a = atom.trim();
    // Stat compare — find a top-level comparison operator. Two-char ops
    // first so `>=` doesn't split at `>`.
    for (lua_op, dsl_op) in
        [("~=", "!="), ("==", "=="), (">=", ">="), ("<=", "<="), (">", ">"), ("<", "<")]
    {
        let Some(pos) = find_top_level(a, lua_op) else { continue };
        let lhs = a[..pos].trim();
        let rhs = a[pos + lua_op.len()..].trim();
        let stat = lhs
            .strip_prefix(param)
            .and_then(|r| r.strip_prefix(':'))
            .and_then(getter_to_stat_field)?;
        let n: i64 = rhs.parse().ok()?;
        if n < 0 { return None; } // grammar expr atoms are unsigned
        return Some(format!("{stat} {dsl_op} {n}"));
    }
    let rest = a.strip_prefix(param)?.strip_prefix(':')?;
    let (name, argrest) = rest.split_once('(')?;
    let args = argrest.strip_suffix(')')?.trim();
    // A chained call (`GetBattledGroup():IsContains(x)`) leaves junk in
    // `args` (unbalanced text) — the per-name arg parses reject it.
    card_pred_to_where(name.trim(), args)
}

/// `Get<Stat>()` method text → DSL stat_field token.
fn getter_to_stat_field(m: &str) -> Option<&'static str> {
    Some(match m.trim() {
        "GetAttack()"      => "atk",
        "GetDefense()"     => "def",
        "GetBaseAttack()"  => "base_atk",
        "GetBaseDefense()" => "base_def",
        "GetLevel()"       => "level",
        "GetRank()"        => "rank",
        "GetLink()"        => "link",
        _ => return None,
    })
}

/// Byte offset of the first top-level (paren-depth-0) occurrence of
/// `needle` in `s`, or None.
fn find_top_level(s: &str, needle: &str) -> Option<usize> {
    let b = s.as_bytes();
    let n = needle.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && b[i..].starts_with(n) {
            return Some(i);
        }
    }
    None
}

/// Resolve a named script-local filter (`s.filter`) against the walked
/// function table (T38 S1). The filter must be a PURE single-return
/// predicate — `return_expr` is only captured for single-statement
/// bodies, which is exactly the purity gate — and every atom must test
/// the filter's first parameter against literals.
fn named_filter_to_where(
    raw: &str,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<String> {
    let raw = raw.trim();
    if !raw.starts_with("s.") || !raw[2..].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let fb = functions.get(raw)?;
    let expr = fb.return_expr.as_deref()?;
    let param = fb.params.first()?;
    if param.is_empty() { return None; }
    lua_filter_expr_to_where(expr, param)
}

/// T38 S1 — emit-time refinement of group bindings whose lua filter was
/// a named script-local function. Walk time can't see the filter fn's
/// body (specs freeze per handler), so the upgrade happens here: an
/// unmapped spec whose `raw_filter` resolves through
/// [`named_filter_to_where`] gains the where-clause and flips
/// `filter_mapped`, letting the Phase 10 group-modifier gate pass with
/// the EXACT member set. Returns None when nothing upgrades (callers
/// keep the borrowed original — no clone cost on the common path).
fn refine_group_bindings(
    body: &FunctionBody,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<FunctionBody> {
    let mut upgrades: Vec<(String, String)> = Vec::new();
    for (name, spec) in &body.group_bindings {
        if spec.filter_mapped { continue; }
        let Some(raw) = spec.raw_filter.as_deref() else { continue };
        let Some(wc) = named_filter_to_where(raw, functions) else { continue };
        upgrades.push((name.clone(), wc));
    }
    if upgrades.is_empty() { return None; }
    let mut out = body.clone();
    for (name, wc) in upgrades {
        if let Some(spec) = out.group_bindings.get_mut(&name) {
            spec.where_clause = Some(wc);
            spec.filter_mapped = true;
        }
    }
    Some(out)
}

/// SET_* archetype constant → DSL archetype string. Curated for the
/// constants that appear as fusion/ritual summon filters in the corpus;
/// names match the TCG archetype strings the compiler's ArchetypeIs
/// predicate (and its card-name substring fallback) expects.
fn setcode_const_to_archetype(c: &str) -> Option<&'static str> {
    Some(match c {
        "SET_AMAZONESS"    => "Amazoness",
        "SET_ANCIENT_GEAR" => "Ancient Gear",
        "SET_ASSAULT_MODE" => "/Assault Mode",
        "SET_AZAMINA"      => "Azamina",
        // T38 S1 — archetypes referenced by stat-chip group filters.
        "SET_CHRONOMALY"   => "Chronomaly",
        "SET_CIPHER"       => "Cipher",
        "SET_DESTINY_HERO" => "Destiny HERO",
        "SET_DOGMATIKA"    => "Dogmatika",
        "SET_FLAMVELL"     => "Flamvell",
        "SET_FUR_HIRE"     => "Fur Hire",
        "SET_GUSTO"        => "Gusto",
        "SET_KARAKURI"     => "Karakuri",
        "SET_LAVAL"        => "Laval",
        "SET_MORGANITE"    => "Morganite",
        "SET_PERFORMAPAL"  => "Performapal",
        "SET_DD"           => "D/D",
        "SET_GEM_KNIGHT"   => "Gem-Knight",
        "SET_DDD"          => "D/D/D",
        "SET_FIENDSMITH"   => "Fiendsmith",
        "SET_FRIGHTFUR"    => "Frightfur",
        "SET_GOLD_PRIDE"   => "Gold Pride",
        "SET_GOUKI"        => "Gouki",
        "SET_GRAVEKEEPERS" => "Gravekeeper's",
        "SET_HEROIC"       => "Heroic",
        "SET_HORUS"        => "Horus",
        "SET_INVOKED"      => "Invoked",
        "SET_LYRILUSC"     => "Lyrilusc",
        "SET_MAGISTUS"     => "Magistus",
        "SET_MEGALITH"     => "Megalith",
        "SET_MEKLORD"      => "Meklord",
        "SET_MELODIOUS"    => "Melodious",
        "SET_METALFOES"    => "Metalfoes",
        "SET_MEMENTO"      => "Memento",
        "SET_NINJA"        => "Ninja",
        "SET_PREDAP"       => "Predap",
        "SET_PUNK"         => "P.U.N.K.",
        "SET_RAIDRAPTOR"   => "Raidraptor",
        "SET_SHADDOLL"     => "Shaddoll",
        "SET_VAYLANTZ"     => "Vaylantz",
        _ => return None,
    })
}

/// RACE_* constant → DSL race name (grammar's `race` rule).
fn race_const_to_dsl(c: &str) -> Option<&'static str> {
    Some(match c {
        "RACE_AQUA"         => "Aqua",
        "RACE_BEAST"        => "Beast",
        "RACE_BEASTWARRIOR" => "Beast-Warrior",
        "RACE_CYBERSE"      => "Cyberse",
        "RACE_DINOSAUR"     => "Dinosaur",
        "RACE_DIVINE"       => "Divine-Beast",
        "RACE_DRAGON"       => "Dragon",
        "RACE_FAIRY"        => "Fairy",
        "RACE_FIEND"        => "Fiend",
        "RACE_FISH"         => "Fish",
        "RACE_ILLUSION"     => "Illusion",
        "RACE_INSECT"       => "Insect",
        "RACE_MACHINE"      => "Machine",
        "RACE_PLANT"        => "Plant",
        "RACE_PSYCHIC"      => "Psychic",
        "RACE_PYRO"         => "Pyro",
        "RACE_REPTILE"      => "Reptile",
        "RACE_ROCK"         => "Rock",
        "RACE_SEASERPENT"   => "Sea Serpent",
        "RACE_SPELLCASTER"  => "Spellcaster",
        "RACE_THUNDER"      => "Thunder",
        "RACE_WARRIOR"      => "Warrior",
        "RACE_WINGEDBEAST"  => "Winged Beast",
        "RACE_WYRM"         => "Wyrm",
        "RACE_ZOMBIE"       => "Zombie",
        _ => return None,
    })
}

/// ATTRIBUTE_* constant → DSL attribute name (grammar's `attribute` rule).
fn attribute_const_to_dsl(c: &str) -> Option<&'static str> {
    Some(match c {
        "ATTRIBUTE_DARK"   => "DARK",
        "ATTRIBUTE_DIVINE" => "DIVINE",
        "ATTRIBUTE_EARTH"  => "EARTH",
        "ATTRIBUTE_FIRE"   => "FIRE",
        "ATTRIBUTE_LIGHT"  => "LIGHT",
        "ATTRIBUTE_WATER"  => "WATER",
        "ATTRIBUTE_WIND"   => "WIND",
        _ => return None,
    })
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

/// Spec for a stat-modifier passive — extracted from an
/// `EffectSkeleton` whose chain is `SetType(EFFECT_TYPE_*) +
/// SetCode(EFFECT_UPDATE_ATTACK|DEFENSE) + SetValue(<int or T34
/// overlay/counter closure>)` with no activated-effect handlers and a
/// `c:RegisterEffect` commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassiveModifierSpec {
    pub stat: String,             // "atk" | "def"
    /// Sign of the modifier line (`-` when true).
    pub negative: bool,
    /// Sign-free DSL value expr: `"500"`, `"self.overlay_count * 700"`,
    /// `"self.counter(\"Spell Counter\") * 300"`, …
    pub value: String,
    pub effect_type: String,      // raw SetType arg
    pub scope: Option<&'static str>,
    pub target: Option<&'static str>,
}

impl PassiveModifierSpec {
    /// The `modifier: …` line this spec renders to (also the apply
    /// tool's duplicate-injection key).
    pub fn modifier_line(&self) -> String {
        let op = if self.negative { '-' } else { '+' };
        format!("modifier: {} {} {}", self.stat, op, self.value)
    }

    /// Render to a DSL `passive "<name>" { … }` block. Emits a `scope:`
    /// or `target:` line when needed so the modifier's reach matches
    /// the underlying Lua chain (e.g. `target: equipped_card` for
    /// `EFFECT_TYPE_EQUIP`).
    pub fn to_dsl_block(&self, name: &str, indent: &str) -> String {
        let mut body = String::new();
        if let Some(scope) = self.scope {
            body.push_str(&format!("{indent}    scope: {scope}\n"));
        }
        if let Some(target) = self.target {
            body.push_str(&format!("{indent}    target: {target}\n"));
        }
        body.push_str(&format!("{indent}    {}\n", self.modifier_line()));
        format!("{indent}passive \"{name}\" {{\n{body}{indent}}}")
    }
}

/// T34 — lower a non-literal `SetValue` arg to a DSL passive value
/// expr. Accepts exactly the inline-closure shape
/// `function(<e>[,<c>]) return <body> end` where `<body>` is a
/// single-step product on either the closure's card param or a
/// group-count global:
///
///   c:GetOverlayCount()              → self.overlay_count
///   c:GetOverlayCount()*N            → self.overlay_count * N
///   N*c:GetOverlayCount()            → self.overlay_count * N
///   c:GetCounter(COUNTER_X | 0xN)*N  → self.counter("<name>") * N
///   Duel.GetMatchingGroupCount(…)*N  → count((all, …)) * N
///   Duel.GetFieldGroupCount(…)*N     → count((all, card, …)) * N
///
/// Counter codes resolve through the Phase 13 embedded map
/// (`counter_arg_to_name`); count calls through
/// `passive_count_call_to_expr` (the passive-aware sibling of the
/// Phase 10 `count_call_to_count_expr`, whose `tp`-only scope gate the
/// handler-less passive closures can never satisfy). Overlay/counter
/// bodies read the card param, so they require `self_scoped` (SINGLE
/// self chains — in EQUIP/FIELD chains the param is each *affected*
/// card, not the passive's own card). Everything else — named function
/// refs, `e:GetHandler()` receivers, multi-step math, non-`*`
/// operators — returns None so the caller skips instead of
/// mis-emitting. A negative factor (`…*-100`) flips the modifier sign.
fn passive_value_expr(raw: &str, self_scoped: bool) -> Option<(bool, String)> {
    let rest = raw.trim().strip_prefix("function")?.trim_start();
    let (params, body) = rest.strip_prefix('(')?.split_once(')')?;
    let mut params = params.split(',').map(str::trim);
    let named = |p: &&str| !p.is_empty() && *p != "_";
    let e_param = params.next().filter(named);
    let card_param = params.next().filter(named);
    let body = body.trim_start().strip_prefix("return")?;
    if !body.starts_with(char::is_whitespace) { return None; }
    let body = body.trim().strip_suffix("end")?.trim_end();
    let (call_txt, factor) = match split_top_level_binop(body) {
        None => (body, 1i64),
        Some((l, '*', r)) => {
            if let Ok(n) = r.parse::<i64>() { (l, n) }
            else if let Ok(n) = l.parse::<i64>() { (r, n) }
            else { return None; }
        }
        Some(_) => return None,
    };
    let stat_ref = if call_txt.starts_with("Duel.GetMatchingGroupCount(")
        || call_txt.starts_with("Duel.GetFieldGroupCount(")
    {
        passive_count_call_to_expr(call_txt, e_param, card_param, self_scoped)?
    } else if self_scoped {
        overlay_counter_call_to_ref(call_txt, card_param?)?
    } else {
        return None;
    };
    let mag = factor.unsigned_abs();
    Some((
        factor < 0,
        if mag == 1 { stat_ref } else { format!("{stat_ref} * {mag}") },
    ))
}

/// Lower a `Duel.GetMatchingGroupCount` / `Duel.GetFieldGroupCount`
/// call inside a passive `SetValue` closure to a DSL `count(…)` expr.
///
/// Passive closures have no `tp` in scope, so unlike the Phase 10
/// resolve-side path the scope player is one of:
///
///   `<e>:GetHandlerPlayer()`  → the passive's controller ("you") —
///                               valid in every chain shape
///   `<c>:GetControler()`      → the affected card's controller; only
///                               `self_scoped` chains guarantee that
///                               card is the passive's own ("you")
///   literal `0`               → absolute player; only translatable
///                               when both location masks are equal, so
///                               the count is player-symmetric
///                               ("either controls")
///
/// The exception arg maps `nil` → none and the passive's own card
/// (`<c>` when self-scoped / `<e>:GetHandler()`) → `except self`; any
/// other exception skips. Filters route through
/// `passive_count_filter`. Skip-not-mis-emit throughout: the count IS
/// the modifier's value, so any unmapped piece drops the whole spec.
fn passive_count_call_to_expr(
    call: &str,
    e_param: Option<&str>,
    card_param: Option<&str>,
    self_scoped: bool,
) -> Option<String> {
    let (is_field, inner) =
        if let Some(rest) = call.strip_prefix("Duel.GetMatchingGroupCount(") {
            (false, rest)
        } else if let Some(rest) = call.strip_prefix("Duel.GetFieldGroupCount(") {
            (true, rest)
        } else {
            return None;
        };
    let inner = inner.strip_suffix(')')?;
    let args = split_top_level_commas(inner)?;
    let (kind, where_clause, scope_p, my, opp, exception) = if is_field {
        // GetFieldGroupCount(player, my_locs, opp_locs) — no filter.
        if args.len() != 3 { return None; }
        ("card".to_string(), None, args[0].as_str(), args[1].as_str(), args[2].as_str(), "nil")
    } else {
        // GetMatchingGroupCount(filter, player, my_locs, opp_locs, exception, filter_args…)
        if args.len() < 5 { return None; }
        let (kind, wc) = passive_count_filter(args[0].as_str(), &args[5..])?;
        (kind, wc, args[1].as_str(), args[2].as_str(), args[3].as_str(), args[4].as_str())
    };
    let scope_ok = e_param.is_some_and(|e| scope_p == format!("{e}:GetHandlerPlayer()"))
        || (self_scoped
            && card_param.is_some_and(|c| scope_p == format!("{c}:GetControler()")))
        || (scope_p == "0" && my == opp && my != "0");
    if !scope_ok { return None; }
    let controller = controller_from_scope(scope_p, my, opp)?;
    let zone = zone_from_locations(my, opp)?;
    let except_self = match exception {
        "nil" => false,
        _ if self_scoped
            && (Some(exception) == card_param
                || e_param.is_some_and(|e| exception == format!("{e}:GetHandler()"))) =>
        {
            true
        }
        _ => return None,
    };
    let mut parts = vec!["all".to_string(), kind, controller, zone];
    if except_self { parts.push("except self".to_string()); }
    if let Some(w) = where_clause { parts.push(format!("where {w}")); }
    Some(format!("count(({}))", parts.join(", ")))
}

/// Map a group-count filter predicate (plus any trailing filter args
/// from the count call) to a DSL `(kind, where-predicate)` pair for
/// the passive count path. Richer than the Phase 10 `map_group_filter`
/// arg-less table: parameterized predicates (`Card.IsType` + one
/// TYPE_*, `Card.IsSetCard`/`IsRace`/`IsAttribute` + one constant) and
/// `aux.FaceupFilter(<inner>[,<args>])` compositions map here.
/// Unmapped predicates — `Card.IsCode` (needs a name lookup),
/// `Card.IsSpellTrap` (the parser has no nested `(a or b)` predicate),
/// custom closures, `s.*` refs — return None.
fn passive_count_filter(
    filter: &str,
    extra: &[String],
) -> Option<(String, Option<String>)> {
    if let Some(inner) = filter
        .strip_prefix("aux.FaceupFilter(")
        .and_then(|r| r.strip_suffix(')'))
    {
        if !extra.is_empty() { return None; }
        let inner_args = split_top_level_commas(inner)?;
        let (kind, wc) = passive_count_filter(inner_args[0].as_str(), &inner_args[1..])?;
        let wc = match wc {
            Some(w) => format!("{w} and is_face_up"),
            None => "is_face_up".to_string(),
        };
        return Some((kind, Some(wc)));
    }
    let kind_only = |k: &str| Some((k.to_string(), None));
    match filter {
        _ if !extra.is_empty() => {
            if extra.len() != 1 { return None; }
            let arg = extra[0].as_str();
            match filter {
                "Card.IsType" => match arg {
                    "TYPE_NORMAL" => kind_only("normal monster"),
                    "TYPE_TUNER"  => kind_only("tuner monster"),
                    _ => None,
                },
                "Card.IsSetCard" | "Card.IsRace" | "Card.IsAttribute" => {
                    Some(("card".to_string(), Some(filter_predicate_to_where(filter, arg)?)))
                }
                _ => None,
            }
        }
        "nil" | "aux.TRUE"     => kind_only("card"),
        "Card.IsMonster"       => kind_only("monster"),
        "Card.IsSpell"         => kind_only("spell"),
        "Card.IsTrap"          => kind_only("trap"),
        "Card.IsRitualMonster" => kind_only("ritual monster"),
        "Card.IsFaceup"   => Some(("card".to_string(), Some("is_face_up".to_string()))),
        "Card.IsFacedown" => Some(("card".to_string(), Some("is_face_down".to_string()))),
        _ => None,
    }
}

/// `<card_param>:GetOverlayCount()` → `self.overlay_count`;
/// `<card_param>:GetCounter(<code>)` → `self.counter("<name>")`.
/// Any other receiver or method returns None.
fn overlay_counter_call_to_ref(call: &str, card_param: &str) -> Option<String> {
    let rest = call.strip_prefix(card_param)?.strip_prefix(':')?;
    if rest == "GetOverlayCount()" {
        return Some("self.overlay_count".to_string());
    }
    let arg = rest.strip_prefix("GetCounter(")?.strip_suffix(')')?;
    let name = counter_arg_to_name(arg)?;
    Some(format!("self.counter(\"{name}\")"))
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
            let (group_bindings, select_sources) = extract_group_bindings(body_block);
            let register_chains = extract_register_chains(body_block);
            let value_bindings = extract_value_bindings(body_block);
            let return_expr = extract_return_expr(body_block);
            let guarded_return = extract_guarded_return(body_block);
            let counter_ops = extract_counter_ops(body_block);
            let (inline_options, choice_arms) = extract_choice_arms(body_block);
            let helper_calls = extract_helper_calls(body_block);
            let equip_helper = extract_equip_helper(body);
            // SetLabel-linked form (target side) and op-side inline form
            // are mutually exclusive in practice — a body matching both
            // would need two SelectOption calls, which both reject.
            let select_option = inline_options
                .or_else(|| extract_select_option(body_block));
            let params: Vec<String> = body
                .parameters()
                .iter()
                .map(|p| p.to_string().trim().to_string())
                .collect();
            if !calls.is_empty() || !group_bindings.is_empty()
                || !register_chains.is_empty() || !value_bindings.is_empty()
                || return_expr.is_some() || !counter_ops.is_empty()
                || select_option.is_some() || !choice_arms.is_empty()
                || !helper_calls.is_empty() || equip_helper.is_some()
                || guarded_return.is_some()
            {
                report.functions.insert(name, FunctionBody {
                    calls,
                    group_bindings,
                    register_chains,
                    value_bindings,
                    return_expr,
                    guarded_return,
                    counter_ops,
                    select_option,
                    choice_arms,
                    helper_calls,
                    equip_helper,
                    select_sources,
                    params,
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
    // Local bindings (`local params = {...}`) visible to summon-helper
    // param decoding, plus the taint set: names that are re-assigned or
    // mutated anywhere in the function fail the single-assignment check
    // (Phase 12, mirroring Phase 10's count-var taint rule).
    let mut local_exprs: BTreeMap<String, &Expression> = BTreeMap::new();
    let mut tainted: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // (effect binding, local var) pairs whose helper params came from a
    // local table — re-checked against the final taint set after the walk.
    let mut helper_var_uses: Vec<(String, String)> = Vec::new();
    // Source-order bookkeeping for the Phase 12 block-alignment guard:
    // each effect-bearing chain gets an ordinal in `s.initial_effect`
    // source order. Plain clone chains (`local e2=e1:Clone()`) seed a
    // real skeleton from the source's state (Phase 14); only clones of
    // summon-helper chains and clones that never register stay on the
    // hazard list.
    let mut source_ord = 0usize;
    let mut ordinals: BTreeMap<String, usize> = BTreeMap::new();
    let mut clone_hazards: Vec<usize> = Vec::new();
    let mut clone_bindings: Vec<String> = Vec::new();

    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                for (i, name) in names.iter().enumerate() {
                    let expr = exprs.get(i);
                    if let Some(expr) = expr {
                        if local_exprs.insert(name.clone(), expr).is_some() {
                            // `local x = ...` twice — second shadows first;
                            // too ambiguous for param decoding.
                            tainted.insert(name.clone());
                        }
                        if expr_is_effect_createeffect(expr) {
                            ordinals.insert(name.clone(), source_ord);
                            source_ord += 1;
                            by_binding.insert(name.clone(), EffectSkeleton {
                                binding: name.clone(),
                                ..Default::default()
                            });
                        } else if let Some(src_skel) = expr_clone_source(expr)
                            .and_then(|src| by_binding.get(&src)).cloned()
                        {
                            if src_skel.is_summon_helper() {
                                // Clone of a fusion/ritual helper chain —
                                // seeding would duplicate the summon line,
                                // so keep the conservative hazard skip.
                                clone_hazards.push(source_ord);
                                source_ord += 1;
                            } else {
                                // `local e2=e1:Clone()` — seed a real
                                // skeleton from the source's state at the
                                // clone point; later Set* calls override
                                // inherited slots (Phase 14).
                                let mut seeded = src_skel;
                                seeded.binding = name.clone();
                                seeded.registered = false;
                                seeded.clone_seeded = seeded.set_calls.iter()
                                    .map(|(m, _)| m.clone()).collect();
                                ordinals.insert(name.clone(), source_ord);
                                source_ord += 1;
                                clone_bindings.push(name.clone());
                                by_binding.insert(name.clone(), seeded);
                            }
                        } else if expr_is_fusion_createsummoneff(expr) {
                            ordinals.insert(name.clone(), source_ord);
                            source_ord += 1;
                            by_binding.insert(name.clone(), EffectSkeleton {
                                binding: name.clone(),
                                fusion_summon_spec: true,
                                summon_helper_op: Some(plain_helper_op_from_expr(
                                    expr,
                                    SummonHelperKind::FusionSummonEffOp,
                                    FUSION_CREATE_SUMMON_EFF_PARAMS,
                                )),
                                ..Default::default()
                            });
                        } else if expr_is_fusion_registersummoneff(expr) {
                            ordinals.insert(name.clone(), source_ord);
                            source_ord += 1;
                            by_binding.insert(name.clone(), EffectSkeleton {
                                binding: name.clone(),
                                fusion_summon_spec: true,
                                summon_helper_op: Some(plain_helper_op_from_expr(
                                    expr,
                                    SummonHelperKind::FusionSummonEffOp,
                                    FUSION_CREATE_SUMMON_EFF_PARAMS,
                                )),
                                // RegisterSummonEff commits the effect
                                // internally — no `c:RegisterEffect(eN)`
                                // follows, so mark registered here.
                                registered: true,
                                ..Default::default()
                            });
                        } else if let Some(op) = ritual_helper_op_from_expr(expr) {
                            ordinals.insert(name.clone(), source_ord);
                            source_ord += 1;
                            by_binding.insert(name.clone(), EffectSkeleton {
                                binding: name.clone(),
                                ritual_summon_spec: true,
                                summon_helper_op: Some(op),
                                // AddProc* register the effect internally
                                // (CreateProc's is committed by the
                                // `c:RegisterEffect(eN)` that follows) —
                                // treat as already committed so the
                                // registered-only filter below keeps it.
                                registered: true,
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            Stmt::Assignment(a) => {
                // Plain assignments taint param tables: `params = ...`
                // re-binds, `params.x = ...` / `params[i] = ...` mutate.
                for var in a.variables() {
                    if let Some(name) = assigned_base_name(var) {
                        tainted.insert(name);
                    }
                }
            }
            Stmt::FunctionCall(fc) => {
                // `table.insert(params, ...)` mutates the param table.
                if call_head_string(fc) == "table.insert" {
                    if let Some(Suffix::Call(Call::AnonymousCall(args))) = fc.suffixes().last() {
                        if let Some(first) = call_args_to_strings(args).first() {
                            if first.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                                tainted.insert(first.clone());
                            }
                        }
                    }
                }
                // `c:RegisterEffect(Fusion.CreateSummonEff(...))` — direct
                // commit without a local binding intermediate. Synthesize
                // an anonymous skeleton so Pass A can fill the resolve.
                if let Some(inner) = register_effect_fusion_inline_expr(fc) {
                    let anon = format!("__fusion_inline_{}", by_binding.len());
                    ordinals.insert(anon.clone(), source_ord);
                    source_ord += 1;
                    by_binding.insert(anon.clone(), EffectSkeleton {
                        binding: anon,
                        fusion_summon_spec: true,
                        summon_helper_op: Some(plain_helper_op_from_expr(
                            inner,
                            SummonHelperKind::FusionSummonEffOp,
                            FUSION_CREATE_SUMMON_EFF_PARAMS,
                        )),
                        registered: true,
                        ..Default::default()
                    });
                }
                // Top-level `Fusion.RegisterSummonEff(c, ...)` — the
                // self-registering variant: forwards its args to
                // CreateSummonEff and registers the effect itself
                // (proc_fusion_spell.lua), so no binding or
                // `c:RegisterEffect(eN)` follows. Same param list and
                // handler-arg skip as CreateSummonEff.
                if call_is_fusion_register_summon_eff(fc) {
                    let anon = format!("__fusion_register_{}", by_binding.len());
                    ordinals.insert(anon.clone(), source_ord);
                    source_ord += 1;
                    by_binding.insert(anon.clone(), EffectSkeleton {
                        binding: anon,
                        fusion_summon_spec: true,
                        summon_helper_op: Some(plain_helper_op_from_call(
                            fc,
                            SummonHelperKind::FusionSummonEffOp,
                            FUSION_CREATE_SUMMON_EFF_PARAMS,
                        )),
                        registered: true,
                        ..Default::default()
                    });
                }
                // Top-level `Ritual.AddProcEqual(...)` / `AddProcGreater(...)`
                // / `AddProcEqualCode(...)` / `AddProcGreaterCode(...)` /
                // `AddWholeLevelTribute(...)` shapes don't bind to a local.
                // The helper registers its own effect, so synthesize an
                // anonymous ritual-spec skeleton.
                if call_is_ritual_proc_helper(fc) {
                    // AddProcEqual/Greater params decode like CreateProc
                    // minus the implied lvtype. The *Code variants filter
                    // by card-code lists and AddWholeLevelTribute changes
                    // the tribute procedure — neither has a DSL
                    // equivalent, so their ops stay unresolved and the
                    // resolve remains an empty stub.
                    let op = match call_head_string(fc).as_str() {
                        "Ritual.AddProcEqual" | "Ritual.AddProcGreater" => {
                            plain_helper_op_from_call(
                                fc,
                                SummonHelperKind::RitualOperation,
                                RITUAL_ADD_PROC_LEVEL_PARAMS,
                            )
                        }
                        _ => SummonHelperOp {
                            kind: SummonHelperKind::RitualOperation,
                            params: Vec::new(),
                            unresolved: true,
                        },
                    };
                    let anon = format!("__ritual_inline_{}", by_binding.len());
                    ordinals.insert(anon.clone(), source_ord);
                    source_ord += 1;
                    by_binding.insert(anon.clone(), EffectSkeleton {
                        binding: anon,
                        ritual_summon_spec: true,
                        summon_helper_op: Some(op),
                        registered: true,
                        ..Default::default()
                    });
                }
                // `eN:SetX(...)` populates the effect named by binding.
                if let Some((binding, method, args)) = method_call_on_binding(fc) {
                    if let Some(skel) = by_binding.get_mut(&binding) {
                        if skel.clone_seeded.remove(&method) {
                            // First write on a clone-inherited slot is the
                            // clone-then-override idiom — drop the
                            // inherited entries so lookups see the
                            // override, not the source's value.
                            skel.set_calls.retain(|(m, _)| *m != method);
                        }
                        skel.set_calls.push((method.clone(), args.clone()));
                        if method == "SetOperation" {
                            skel.operation_handler = args.first().cloned();
                            // Parameterized fusion/ritual helper as the
                            // operation (Phase 12) — decode its params.
                            if let Some(arg_expr) = first_method_arg_expr(fc) {
                                if let Some((op, used_var)) =
                                    summon_helper_from_expr(arg_expr, &local_exprs)
                                {
                                    if let Some(var) = used_var {
                                        helper_var_uses.push((binding.clone(), var));
                                    }
                                    skel.summon_helper_op = Some(op);
                                }
                            }
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

    // Param tables mutated anywhere in the function (even after the
    // SetOperation call) fail the taint check — mark those helper ops
    // unresolved so emit skips them.
    for (binding, var) in helper_var_uses {
        if tainted.contains(&var) {
            if let Some(op) = by_binding
                .get_mut(&binding)
                .and_then(|s| s.summon_helper_op.as_mut())
            {
                op.unresolved = true;
            }
        }
    }

    // Clone skeletons that never saw `<recv>:RegisterEffect(eN)` drop out
    // of walk.effects below, yet may still own a .ds block (e.g. the
    // registration happens behind a shape this walk doesn't track) —
    // keep their ordinals on the hazard list so later skeletons skip.
    for cb in &clone_bindings {
        if by_binding.get(cb).is_some_and(|s| !s.registered) {
            if let Some(ord) = ordinals.get(cb) {
                clone_hazards.push(*ord);
            }
        }
    }

    // Block-alignment guard (Phase 12): bare EFFECT_TYPE_ACTIVATE chains
    // with no SetOperation own a .ds effect block (the activation shell
    // of a spell/trap) yet never consume a Pass-A block index — same for
    // the residual clone hazards collected above (Phase 14 seeds plain
    // registered clones into real skeletons, so they consume indices
    // naturally and no longer poison the mapping; the c99634927 incident
    // — e4's translation landing in the e3 clone's "Effect 3" block —
    // can't recur for seeded clones). ANY skeleton that comes AFTER a
    // hazard chain in source order would fill the wrong block, so it
    // gets flagged and Pass A / A2 / helper emit skip.
    // Phase 20 — clone hazards are exactly the chains that may own a .ds
    // block without a skeleton; expose them so the block matcher can
    // align them as unknown-signature entities.
    report.phantom_block_ordinals = clone_hazards.clone();
    let mut hazards = clone_hazards;
    for skel in by_binding.values() {
        let is_bare_activate = skel.registered
            && skel.operation_handler.is_none()
            && !skel.fusion_summon_spec
            && !skel.ritual_summon_spec
            && skel.first_arg_of("SetType")
                .is_some_and(|t| t.contains("EFFECT_TYPE_ACTIVATE"));
        if is_bare_activate {
            if let Some(ord) = ordinals.get(&skel.binding) {
                hazards.push(*ord);
            }
        }
    }
    if !hazards.is_empty() {
        for (binding, skel) in by_binding.iter_mut() {
            let Some(ord) = ordinals.get(binding) else { continue };
            if hazards.iter().any(|h| h < ord) {
                skel.block_alignment_hazard = true;
            }
        }
    }

    for (binding, mut skel) in by_binding {
        if skel.registered {
            skel.source_ordinal = ordinals.get(&binding).copied().unwrap_or(usize::MAX);
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

/// True if the function call is one of the top-level Ritual.* helpers
/// that register a ritual-summon activation effect: `AddProcEqual`,
/// `AddProcGreater`, `AddProcEqualCode`, `AddProcGreaterCode`, or
/// `AddWholeLevelTribute`. Used to mark anonymous skeletons during walk.
fn call_is_ritual_proc_helper(fc: &FunctionCall) -> bool {
    let head = call_head_string(fc);
    matches!(
        head.as_str(),
        "Ritual.AddProcEqual"
        | "Ritual.AddProcGreater"
        | "Ritual.AddProcEqualCode"
        | "Ritual.AddProcGreaterCode"
        | "Ritual.AddWholeLevelTribute",
    )
}

/// Decode the assigned-form ritual summon helpers: `local
/// e1=Ritual.CreateProc(...)` / `AddProcEqual(...)` /
/// `AddProcGreater(...)`. The *Code variants and AddWholeLevelTribute
/// decode as unresolved ops (card-code filters / tribute-procedure
/// changes have no DSL equivalent), so their skeletons keep block
/// alignment while the resolve stays an empty stub. Mirrors the
/// top-level statement handling in the walker.
fn ritual_helper_op_from_expr(expr: &Expression) -> Option<SummonHelperOp> {
    let Expression::FunctionCall(fc) = expr else { return None };
    match call_head_string(fc).as_str() {
        "Ritual.CreateProc" => Some(plain_helper_op_from_call(
            fc,
            SummonHelperKind::RitualOperation,
            RITUAL_CREATE_PROC_PARAMS,
        )),
        "Ritual.AddProcEqual" | "Ritual.AddProcGreater" => Some(plain_helper_op_from_call(
            fc,
            SummonHelperKind::RitualOperation,
            RITUAL_ADD_PROC_LEVEL_PARAMS,
        )),
        "Ritual.AddProcEqualCode"
        | "Ritual.AddProcGreaterCode"
        | "Ritual.AddWholeLevelTribute" => Some(SummonHelperOp {
            kind: SummonHelperKind::RitualOperation,
            params: Vec::new(),
            unresolved: true,
        }),
        _ => None,
    }
}

/// True if `expr` is the call `Fusion.CreateSummonEff(...)` — the helper
/// that builds a fusion-summon activation effect with its own UI / op
/// pipeline. Translator emits a fixed `fusion_summon (1, fusion monster)`
/// line for skeletons created via this helper.
fn expr_is_fusion_createsummoneff(expr: &Expression) -> bool {
    if let Expression::FunctionCall(fc) = expr {
        let head = call_head_string(fc);
        head == "Fusion.CreateSummonEff"
    } else {
        false
    }
}

/// True if `expr` is `Fusion.RegisterSummonEff(...)` — the binding form
/// of the self-registering fusion helper (`local e1=Fusion.RegisterSummonEff{...}`).
/// It forwards to CreateSummonEff and registers the effect internally.
fn expr_is_fusion_registersummoneff(expr: &Expression) -> bool {
    if let Expression::FunctionCall(fc) = expr {
        call_is_fusion_register_summon_eff(fc)
    } else {
        false
    }
}

/// True if the function call is `Fusion.RegisterSummonEff(...)` — the
/// top-level statement form of the self-registering fusion helper.
fn call_is_fusion_register_summon_eff(fc: &FunctionCall) -> bool {
    call_head_string(fc) == "Fusion.RegisterSummonEff"
}

/// Detect `c:RegisterEffect(Fusion.CreateSummonEff(...))` — the inline
/// commit shape where no local binding holds the effect. Returns the
/// inner `Fusion.CreateSummonEff(...)` expression so the walker can
/// synthesize an anonymous EffectSkeleton with its params decoded.
fn register_effect_fusion_inline_expr(fc: &FunctionCall) -> Option<&Expression> {
    // Detect the outer call: <receiver>:RegisterEffect(<expr>)
    let suffixes: Vec<&Suffix> = fc.suffixes().collect();
    let last = suffixes.last()?;
    let args = match last {
        Suffix::Call(Call::MethodCall(mc))
            if mc.name().token().to_string() == "RegisterEffect" =>
        {
            mc.args()
        }
        _ => return None,
    };
    // Argument list shape: a single FunctionCall whose head is
    // `Fusion.CreateSummonEff`.
    let exprs: Vec<&Expression> = match args {
        full_moon::ast::FunctionArgs::Parentheses { arguments, .. } => arguments.iter().collect(),
        _ => return None,
    };
    let first = exprs.first()?;
    if expr_is_fusion_createsummoneff(first) {
        Some(*first)
    } else {
        None
    }
}

/// Decode the params of a plain summon-helper call expression
/// (`Fusion.CreateSummonEff(c, …)` / `Ritual.CreateProc(c, …)` binding
/// or inline forms). Non-call expressions mark the op unresolved.
fn plain_helper_op_from_expr(
    expr: &Expression,
    kind: SummonHelperKind,
    names: &[&str],
) -> SummonHelperOp {
    match expr {
        Expression::FunctionCall(fc) => plain_helper_op_from_call(fc, kind, names),
        _ => SummonHelperOp { kind, params: Vec::new(), unresolved: true },
    }
}

/// Decode the params of a plain summon-helper call (positional form
/// with a leading `handler` arg, or the named-args table sugar of
/// `aux.FunctionWithNamedArgs`). The `handler` arg and the cosmetic
/// `desc` param are dropped; every other param flows to the emit
/// policy, which rejects anything without a DSL equivalent. Shapes
/// that don't decode mark the op unresolved so the resolve stays an
/// empty stub instead of mis-emitting an over-permissive bare line.
fn plain_helper_op_from_call(
    fc: &FunctionCall,
    kind: SummonHelperKind,
    names: &[&str],
) -> SummonHelperOp {
    let unresolved = SummonHelperOp { kind, params: Vec::new(), unresolved: true };
    let params = match fc.suffixes().last() {
        Some(Suffix::Call(Call::AnonymousCall(ast::FunctionArgs::Parentheses { arguments, .. }))) => {
            // Named-args table passed with explicit parens:
            // `Fusion.CreateSummonEff({handler=c, …})`. The table is the
            // FIRST argument, not the handler card — skipping it as a
            // positional handler would decode zero params and mis-emit
            // an over-permissive bare line.
            if let Some(Expression::TableConstructor(tc)) = arguments.iter().next() {
                if arguments.len() != 1 { return unresolved; }
                match named_table_params(tc) {
                    Some(params) => params,
                    None => return unresolved,
                }
            } else {
                // Positional form — first arg is the handler card, skip it.
                let raws: Vec<String> = arguments
                    .iter()
                    .skip(1)
                    .map(|e| e.to_string().trim().to_string())
                    .collect();
                positional_params(&raws, names)
            }
        }
        // `Fusion.CreateSummonEff{handler=c, …}` named-table sugar —
        // named_table_params drops the handler key itself.
        Some(Suffix::Call(Call::AnonymousCall(ast::FunctionArgs::TableConstructor(tc)))) => {
            match named_table_params(tc) {
                Some(params) => params,
                None => return unresolved,
            }
        }
        _ => return unresolved,
    };
    let params: Vec<(String, String)> =
        params.into_iter().filter(|(k, _)| k != "desc").collect();
    SummonHelperOp { kind, params, unresolved: false }
}

/// Base identifier of an assignment target — `params` for `params = x`,
/// `params.foo = x`, and `params[1] = x`.
fn assigned_base_name(var: &ast::Var) -> Option<String> {
    match var {
        ast::Var::Name(n) => Some(n.token().to_string()),
        ast::Var::Expression(ve) => match ve.prefix() {
            ast::Prefix::Name(n) => Some(n.token().to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// First argument Expression of `<bind>:<method>(...)`.
fn first_method_arg_expr(fc: &FunctionCall) -> Option<&Expression> {
    let mut suffixes = fc.suffixes();
    if let Suffix::Call(Call::MethodCall(mc)) = suffixes.next()? {
        if let ast::FunctionArgs::Parentheses { arguments, .. } = mc.args() {
            return arguments.iter().next();
        }
    }
    None
}

/// Decode a `SetOperation` argument that is a parameterized summon
/// helper call (`Fusion.SummonEffOP(...)` / `Ritual.Operation(...)`).
///
/// Normalizes the three corpus call shapes into named params:
/// - positional args: `Fusion.SummonEffOP(fusfilter, matfilter, ...)`
/// - named-args table: `Ritual.Operation(params)` / inline `{...}` —
///   `aux.FunctionWithNamedArgs` reads named keys only
/// - `Fusion.SummonEffOP(table.unpack(params))` — positional fields of
///   a single-assignment local table
///
/// Returns the op plus the local-table variable it read (if any) so the
/// caller can re-check the taint set after the walk completes.
fn summon_helper_from_expr(
    expr: &Expression,
    local_exprs: &BTreeMap<String, &Expression>,
) -> Option<(SummonHelperOp, Option<String>)> {
    let fc = match expr {
        Expression::FunctionCall(fc) => fc,
        _ => return None,
    };
    let (kind, names): (SummonHelperKind, &[&str]) = match call_head_string(fc).as_str() {
        "Fusion.SummonEffOP" => (SummonHelperKind::FusionSummonEffOp, FUSION_SUMMON_EFF_OP_PARAMS),
        "Ritual.Operation" => (SummonHelperKind::RitualOperation, RITUAL_OPERATION_PARAMS),
        _ => return None,
    };
    let unresolved = |used: Option<String>| Some((
        SummonHelperOp { kind, params: Vec::new(), unresolved: true },
        used,
    ));
    let arg_exprs: Vec<&Expression> = match fc.suffixes().last() {
        Some(Suffix::Call(Call::AnonymousCall(ast::FunctionArgs::Parentheses { arguments, .. }))) => {
            arguments.iter().collect()
        }
        // `Fusion.SummonEffOP{...}` table-call sugar.
        Some(Suffix::Call(Call::AnonymousCall(ast::FunctionArgs::TableConstructor(tc)))) => {
            return match named_table_params(tc) {
                Some(params) => Some((SummonHelperOp { kind, params, unresolved: false }, None)),
                None => unresolved(None),
            };
        }
        _ => return unresolved(None),
    };

    if arg_exprs.len() == 1 {
        let raw = arg_exprs[0].to_string().trim().to_string();
        // `table.unpack(params)` — inline the local table positionally.
        if let Some(var) = raw
            .strip_prefix("table.unpack(")
            .and_then(|r| r.strip_suffix(')'))
            .map(str::trim)
            .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            let var = var.to_string();
            return match local_exprs.get(&var) {
                Some(Expression::TableConstructor(tc)) => {
                    match positional_table_params(tc, names) {
                        Some(params) => Some((
                            SummonHelperOp { kind, params, unresolved: false },
                            Some(var),
                        )),
                        None => unresolved(Some(var)),
                    }
                }
                _ => unresolved(Some(var)),
            };
        }
        // Inline named-args table.
        if let Expression::TableConstructor(tc) = arg_exprs[0] {
            return match named_table_params(tc) {
                Some(params) => Some((SummonHelperOp { kind, params, unresolved: false }, None)),
                None => unresolved(None),
            };
        }
        // Bare identifier — either a named-args table local or a plain
        // value local (used as the first positional param).
        if raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return match local_exprs.get(&raw) {
                Some(Expression::TableConstructor(tc)) => match named_table_params(tc) {
                    Some(params) => Some((
                        SummonHelperOp { kind, params, unresolved: false },
                        Some(raw),
                    )),
                    None => unresolved(Some(raw)),
                },
                Some(other) => {
                    let params = positional_params(&[other.to_string().trim().to_string()], names);
                    Some((SummonHelperOp { kind, params, unresolved: false }, Some(raw)))
                }
                None => unresolved(Some(raw)),
            };
        }
    }

    // General positional form.
    let raws: Vec<String> = arg_exprs.iter().map(|e| e.to_string().trim().to_string()).collect();
    let params = positional_params(&raws, names);
    Some((SummonHelperOp { kind, params, unresolved: false }, None))
}

/// Zip raw positional args with helper param names, dropping `nil`s.
/// Args beyond the helper's arity get an `__overflow` sentinel name so
/// the emit policy (which skips any param it doesn't recognize) rejects
/// the line instead of silently dropping the arg.
fn positional_params(raws: &[String], names: &[&str]) -> Vec<(String, String)> {
    let mut params = Vec::new();
    for (i, raw) in raws.iter().enumerate() {
        if raw == "nil" { continue; }
        match names.get(i) {
            Some(name) => params.push((name.to_string(), raw.clone())),
            // More args than the helper takes — record an unmappable
            // sentinel so the emit policy skips the line.
            None => params.push(("__overflow".to_string(), raw.clone())),
        }
    }
    params
}

/// Decode an all-named-fields table constructor (`{fusfilter=..., ...}`)
/// into params. The no-op `handler` key is dropped (FunctionWithNamedArgs
/// only reads the helper's own names). Positional or expression-keyed
/// fields → None (caller marks unresolved).
fn named_table_params(tc: &ast::TableConstructor) -> Option<Vec<(String, String)>> {
    let mut params = Vec::new();
    for field in tc.fields() {
        match field {
            ast::Field::NameKey { key, value, .. } => {
                let k = key.token().to_string();
                let v = value.to_string().trim().to_string();
                if k == "handler" || v == "nil" { continue; }
                params.push((k, v));
            }
            _ => return None,
        }
    }
    Some(params)
}

/// Decode an all-positional-fields table constructor (the
/// `table.unpack(params)` idiom) into named params. Named or
/// expression-keyed fields → None (caller marks unresolved).
fn positional_table_params(tc: &ast::TableConstructor, names: &[&str]) -> Option<Vec<(String, String)>> {
    let mut raws = Vec::new();
    for field in tc.fields() {
        match field {
            ast::Field::NoKey(expr) => raws.push(expr.to_string().trim().to_string()),
            _ => return None,
        }
    }
    Some(positional_params(&raws, names))
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

/// Recursively walk an `Expression` extracting any `Duel.*` `FunctionCall`
/// sub-nodes. Used to capture calls inside boolean contexts such as
/// `if Duel.Equip(tp,c,tc) then ...` — the Lua side-effects still occur,
/// so the action should appear in the translated resolve body.
fn collect_duel_calls_in_expr(expr: &Expression, out: &mut Vec<DuelCall>) {
    match expr {
        Expression::FunctionCall(fc) => {
            if let Some(call) = duel_call_from_fc(fc) { out.push(call); }
        }
        Expression::BinaryOperator { lhs, rhs, .. } => {
            collect_duel_calls_in_expr(lhs, out);
            collect_duel_calls_in_expr(rhs, out);
        }
        Expression::UnaryOperator { expression, .. } => {
            collect_duel_calls_in_expr(expression, out);
        }
        Expression::Parentheses { expression, .. } => {
            collect_duel_calls_in_expr(expression, out);
        }
        _ => {}
    }
}

fn collect_duel_calls(block: &Block, out: &mut Vec<DuelCall>) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::FunctionCall(fc) => {
                if let Some(call) = duel_call_from_fc(fc) { out.push(call); }
            }
            Stmt::If(if_stmt) => {
                // Walk the if-condition expression for side-effectful Duel.*
                // calls (e.g. `if Duel.Equip(...) then`) — the call happens
                // even when used as a boolean. `translate_call` filters out
                // pure query/UI methods so this stays safe.
                collect_duel_calls_in_expr(if_stmt.condition(), out);
                collect_duel_calls(if_stmt.block(), out);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_duel_calls_in_expr(ei.condition(), out);
                    collect_duel_calls(ei.block(), out);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_duel_calls(else_block, out);
                }
            }
            Stmt::While(w)  => { collect_duel_calls_in_expr(w.condition(), out); collect_duel_calls(w.block(), out); }
            Stmt::Repeat(r) => { collect_duel_calls_in_expr(r.until(), out); collect_duel_calls(r.block(), out); }
            Stmt::NumericFor(nf) => collect_duel_calls(nf.block(), out),
            Stmt::GenericFor(gf) => collect_duel_calls(gf.block(), out),
            Stmt::Do(d) => collect_duel_calls(d.block(), out),
            Stmt::LocalAssignment(la) => {
                for e in la.expressions() {
                    collect_duel_calls_in_expr(e, out);
                }
            }
            Stmt::Assignment(a) => {
                for e in a.expressions() {
                    collect_duel_calls_in_expr(e, out);
                }
            }
            _ => {}
        }
    }
}

/// Extract statement-level script-local helper calls (`s.equipop(c,e,tp,tc)`)
/// from a function block, recursively (Phase 17). Only `s.<name>(...)`
/// heads qualify — Duel./aux./method calls have their own walkers.
fn extract_helper_calls(block: &Block) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    collect_helper_calls(block, &mut out);
    out
}

fn collect_helper_calls(block: &Block, out: &mut Vec<(String, Vec<String>)>) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::FunctionCall(fc) => {
                if let Some(hc) = helper_call_from_fc(fc) { out.push(hc); }
            }
            Stmt::If(if_stmt) => {
                collect_helper_calls(if_stmt.block(), out);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_helper_calls(ei.block(), out);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_helper_calls(else_block, out);
                }
            }
            Stmt::While(w)       => collect_helper_calls(w.block(), out),
            Stmt::NumericFor(nf) => collect_helper_calls(nf.block(), out),
            Stmt::GenericFor(gf) => collect_helper_calls(gf.block(), out),
            Stmt::Do(d)          => collect_helper_calls(d.block(), out),
            _ => {}
        }
    }
}

fn helper_call_from_fc(fc: &FunctionCall) -> Option<(String, Vec<String>)> {
    let head = call_head_string(fc);
    if !head.starts_with("s.") || head.contains(':') { return None; }
    match fc.suffixes().last()? {
        Suffix::Call(Call::AnonymousCall(a)) => Some((head, call_args_to_strings(a))),
        _ => None,
    }
}

/// Recognise a select-then-equip helper function (Phase 17): a declared
/// function whose body LEADS with the canonical
/// `<host>:EquipByEffectAndLimitRegister(e,tp,<target>,…)` statement —
/// either bare or as the `if not <call> then return end` guard — where
/// both `<host>` and `<target>` are parameters. Trailing statements
/// (ATK/DEF rider chains with non-literal values) are tolerated and
/// dropped; the equip itself is the translatable core.
fn extract_equip_helper(body: &ast::FunctionBody) -> Option<EquipHelperSpec> {
    let params: Vec<String> = body.parameters().iter().filter_map(|p| match p {
        ast::Parameter::Name(n) => Some(n.token().to_string()),
        _ => None,
    }).collect();
    let first = body.block().stmts().next()?;
    let fc = match first {
        Stmt::FunctionCall(fc) => fc,
        Stmt::If(if_stmt) => {
            // `if not <call> then return end` — the guard arm must be
            // empty besides the return.
            if if_stmt.block().stmts().next().is_some() { return None; }
            match if_stmt.condition() {
                Expression::UnaryOperator { unop: ast::UnOp::Not(_), expression } => {
                    match expression.as_ref() {
                        Expression::FunctionCall(fc) => fc,
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    let (recv, method, args) = method_call_on_binding(fc)?;
    if method != "EquipByEffectAndLimitRegister" { return None; }
    let host_param = params.iter().position(|p| *p == recv)?;
    let target_arg = args.get(2)?;
    let target_param = params.iter().position(|p| p == target_arg)?;
    Some(EquipHelperSpec { host_param, target_param })
}

/// `Duel.*` calls that only read a selection for UI feedback — they do
/// not act on it, so they don't count as consumers (Phase 17).
const SELECTION_COSMETIC_CALLS: &[&str] = &[
    "Duel.Hint", "Duel.HintSelection", "Duel.ConfirmCards",
    "Duel.SetOperationInfo", "Duel.SetPossibleOperationInfo",
    "Duel.BreakEffect",
];

/// Count how many emit-relevant sites consume a selection binding,
/// across its whole alias set (`g`, `tc = g:GetFirst()`, …). The Phase
/// 17 select-then-do paths require exactly ONE consumer: the DSL folds
/// the selection into the action's selector, so a second consumer would
/// re-select a fresh card instead of reusing the first pick
/// (skip-not-mis-emit).
fn selection_consumers(body: &FunctionBody, name: &str) -> usize {
    let mut aliases: BTreeSet<String> = BTreeSet::new();
    aliases.insert(name.to_string());
    if let Some(src) = body.select_sources.get(name) {
        aliases.insert(src.clone());
    }
    let derived: Vec<String> = body.select_sources.iter()
        .filter(|(_, v)| aliases.contains(*v))
        .map(|(k, _)| k.clone())
        .collect();
    aliases.extend(derived);
    fn base(s: &str) -> &str {
        s.trim().split([':', '.']).next().unwrap_or(s)
    }
    let mut n = 0;
    for c in &body.calls {
        if SELECTION_COSMETIC_CALLS.contains(&c.method.as_str()) { continue; }
        if c.args.iter().any(|a| aliases.contains(base(a))) { n += 1; }
    }
    for ch in &body.register_chains {
        if aliases.contains(base(&ch.register_target)) { n += 1; }
    }
    for op in &body.counter_ops {
        if aliases.contains(base(&op.receiver)) { n += 1; }
    }
    for (_, args) in &body.helper_calls {
        if args.iter().any(|a| aliases.contains(base(a))) { n += 1; }
    }
    n
}

/// Walk a function body for `local <name> = Duel.<select-call>(...)`
/// chains. Each binding captures the SelectorSpec we'd emit if the
/// binding is referenced later as the target of an action
/// (SendtoGrave(<name>), etc.).
///
/// Phase 17 additions:
///   - `local tc = Duel.SelectMatchingCard(...):GetFirst()` — the
///     one-card selection bound directly to a card local. Only
///     `min == max == 1` selects qualify (GetFirst of a wider pick is
///     an arbitrary member, not "the selection").
///   - `local tc = g:GetFirst()` with `g` already bound to a one-card
///     selection — derived alias, recorded in `sources`.
///   - Taint: a name select-bound more than once (shadowing — Dark
///     Magic Expanded's double `local g=`) or re-assigned later has no
///     single statically-known selection; it is dropped along with any
///     alias derived from it (skip-not-mis-emit).
fn extract_group_bindings(block: &Block) -> (BTreeMap<String, SelectorSpec>, BTreeMap<String, String>) {
    let mut out = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut tainted = BTreeSet::new();
    collect_group_bindings(block, &mut out, &mut sources, &mut tainted);
    for name in &tainted {
        out.remove(name);
        sources.remove(name);
    }
    // An alias falls with its source group.
    let stale: Vec<String> = sources.iter()
        .filter(|(_, src)| tainted.contains(*src))
        .map(|(name, _)| name.clone())
        .collect();
    for name in stale {
        out.remove(&name);
        sources.remove(&name);
    }
    (out, sources)
}

fn collect_group_bindings(
    block: &Block,
    out: &mut BTreeMap<String, SelectorSpec>,
    sources: &mut BTreeMap<String, String>,
    tainted: &mut BTreeSet<String>,
) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                for (i, name) in names.iter().enumerate() {
                    let Some(expr) = exprs.get(i) else { continue };
                    if let Expression::FunctionCall(fc) = expr {
                        if let Some(spec) = selector_spec_from_call(fc)
                            .or_else(|| selector_spec_from_getfirst_call(fc))
                        {
                            if out.insert(name.clone(), spec).is_some() {
                                tainted.insert(name.clone());
                            }
                            sources.remove(name);
                            continue;
                        }
                    }
                    // `local tc = g:GetFirst()` — alias of an existing
                    // one-card selection binding.
                    let text = expr.to_string().trim().to_string();
                    if let Some(src) = text.strip_suffix(":GetFirst()") {
                        if let Some(spec) = out.get(src).cloned() {
                            if spec.quantity == "1" {
                                if out.insert(name.clone(), spec).is_some() {
                                    tainted.insert(name.clone());
                                }
                                sources.insert(name.clone(), src.to_string());
                                continue;
                            }
                        }
                    }
                    // Rebinding a tracked selection local to anything
                    // else loses the static selection.
                    if out.contains_key(name) {
                        tainted.insert(name.clone());
                    }
                }
            }
            Stmt::Assignment(a) => {
                // Non-local reassignment of a tracked binding taints it.
                for var in a.variables() {
                    if let ast::Var::Name(n) = var {
                        let name = n.token().to_string();
                        if out.contains_key(&name) {
                            tainted.insert(name);
                        }
                    }
                }
            }
            Stmt::If(if_stmt)     => {
                collect_group_bindings(if_stmt.block(), out, sources, tainted);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_group_bindings(ei.block(), out, sources, tainted);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_group_bindings(else_block, out, sources, tainted);
                }
            }
            Stmt::While(w)        => { collect_group_bindings(w.block(), out, sources, tainted); }
            Stmt::NumericFor(nf)  => { collect_group_bindings(nf.block(), out, sources, tainted); }
            Stmt::GenericFor(gf)  => { collect_group_bindings(gf.block(), out, sources, tainted); }
            Stmt::Do(d)           => { collect_group_bindings(d.block(), out, sources, tainted); }
            _ => {}
        }
    }
}

/// `Duel.SelectMatchingCard(...):GetFirst()` — a one-card selection
/// whose first (only) member is bound directly (Phase 17). The plain
/// extractor rejects this shape because the call's LAST suffix is the
/// `:GetFirst()` method call, not the select's argument list. Only
/// `min == max == 1` qualifies — `GetFirst()` of a wider selection is
/// one arbitrary member, which no DSL selector can name.
fn selector_spec_from_getfirst_call(fc: &FunctionCall) -> Option<SelectorSpec> {
    let head = call_head_string(fc);
    if head != "Duel.SelectMatchingCard" { return None; }
    let suffixes: Vec<&Suffix> = fc.suffixes().collect();
    if suffixes.len() < 2 { return None; }
    let Suffix::Call(Call::MethodCall(mc)) = suffixes[suffixes.len() - 1] else { return None };
    if mc.name().token().to_string() != "GetFirst" { return None; }
    if !call_args_to_strings(mc.args()).is_empty() { return None; }
    let Suffix::Call(Call::AnonymousCall(a)) = suffixes[suffixes.len() - 2] else { return None };
    let args = call_args_to_strings(a);
    let mut spec = spec_from_matching(&args, true, true)?;
    spec.from_resolve_select = true;
    (spec.quantity == "1").then_some(spec)
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
    let mut tainted = Vec::new();
    collect_value_bindings(block, &mut out, &mut tainted);
    // A name written more than once (clamps like `if ct>3 then ct=3 end`,
    // branch-dependent reassignments) has no single statically-known value.
    // Drop it so value resolution skips instead of mis-emitting the first
    // assignment (Phase 10 skip-not-mis-emit).
    for name in tainted {
        out.remove(&name);
    }
    out
}

fn collect_value_bindings(
    block: &Block,
    out: &mut BTreeMap<String, String>,
    tainted: &mut Vec<String>,
) {
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
                            if out.insert(name.clone(), text).is_some() {
                                tainted.push(name.clone());
                            }
                        }
                    }
                }
            }
            Stmt::Assignment(a) => {
                // Non-local reassignment of a tracked binding taints it.
                for var in a.variables() {
                    if let ast::Var::Name(n) = var {
                        tainted.push(n.token().to_string());
                    }
                }
            }
            Stmt::If(if_stmt) => {
                collect_value_bindings(if_stmt.block(), out, tainted);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_value_bindings(ei.block(), out, tainted);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_value_bindings(else_block, out, tainted);
                }
            }
            Stmt::While(w)        => { collect_value_bindings(w.block(), out, tainted); }
            Stmt::NumericFor(nf)  => { collect_value_bindings(nf.block(), out, tainted); }
            Stmt::GenericFor(gf)  => { collect_value_bindings(gf.block(), out, tainted); }
            Stmt::Do(d)           => { collect_value_bindings(d.block(), out, tainted); }
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
        LastStmt::Return(ret) => single_return_value(ret),
        _ => None,
    }
}

/// The single returned expression of a `return <expr>` statement — None
/// for the bare `return` and multi-value returns.
fn single_return_value(ret: &ast::Return) -> Option<String> {
    let mut iter = ret.returns().iter();
    let expr = iter.next()?;
    if iter.next().is_some() { return None; } // multi-value return
    let text = expr.to_string();
    let text = text.trim();
    if text.is_empty() { return None; }
    Some(text.to_string())
}

/// Extract the guarded-value idiom (Phase 19): a body that is exactly
/// `if <cond> then return <A> else return <B> end` (sole statement, no
/// elseif arms, nothing after) or `if <cond> then return <A> end`
/// followed by a bare trailing `return <B>`. Anything else — local
/// bindings, side-effect statements, elseif arms, unreachable trailing
/// code after a closed if/else — returns None.
fn extract_guarded_return(block: &Block) -> Option<GuardedReturn> {
    let mut stmts = block.stmts();
    let Stmt::If(if_stmt) = stmts.next()? else { return None };
    if stmts.next().is_some() { return None; }
    if if_stmt.else_if().is_some_and(|eis| eis.len() > 0) { return None; }
    let then_expr = extract_return_expr(if_stmt.block())?;
    let else_expr = match (if_stmt.else_block(), block.last_stmt()) {
        // `if C then return A else return B end`
        (Some(else_block), None) => extract_return_expr(else_block)?,
        // `if C then return A end return B`
        (None, Some(LastStmt::Return(ret))) => single_return_value(ret)?,
        _ => return None,
    };
    Some(GuardedReturn {
        cond: if_stmt.condition().to_string().trim().to_string(),
        then_expr,
        else_expr,
    })
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
    /// remove_counter "<name>" <n> from <self|selector> (Phase 13)
    RemoveCounter(String, u32, String),
}

impl CostAction {
    fn to_dsl(&self) -> String {
        match self {
            CostAction::PayLp(n)      => format!("pay_lp {}", n),
            CostAction::Discard(sel)  => format!("discard {}", sel),
            CostAction::Tribute(sel)  => format!("tribute {}", sel),
            CostAction::Banish(sel)   => format!("banish {}", sel),
            CostAction::SendToGy(sel) => format!("send {} to gy", sel),
            CostAction::RemoveCounter(name, n, sel) =>
                format!("remove_counter \"{}\" {} from {}", name, n, sel),
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

            // Duel.RemoveCounter as cost — field-wide single-counter
            // removal (Phase 13). Same n == 1 / side-boolean constraints
            // as the resolve path; anything else bails the whole cost.
            "Duel.RemoveCounter" => {
                let (name, controller) = duel_remove_counter_parts(a)?;
                actions.push(CostAction::RemoveCounter(
                    name.to_string(), 1, format!("(1, card, {})", controller),
                ));
            }

            _ if m.starts_with("Duel.") => {
                // Unknown or unhandled Duel action in cost context → skip-not-mis-emit.
                return None;
            }

            _ => {} // non-Duel call (aux.*, etc.) — ignore
        }
    }

    // Counter-removal method calls as cost (Phase 13):
    // `c:RemoveCounter(tp, COUNTER_X, n, REASON_COST)` → remove from self.
    // Constraints are stricter than the resolve path — a cost is a fixed
    // payment, so any counter op we can't express bails the entire block:
    //   - AddCounter in a cost body → bail (not a payment we model);
    //   - branch-nested ops → bail (conditional payment);
    //   - only `self` receivers (c / e:GetHandler()), literal counts,
    //     curated counter names.
    for op in &body.counter_ops {
        if !op.add
            && !op.in_branch
            && !op.multi_target
            && op.player_arg.trim() == "tp"
            && matches!(op.receiver.as_str(), "c" | "e:GetHandler()")
        {
            let name = counter_arg_to_name(&op.counter_arg)?;
            let count: u32 = op.count_arg.trim().parse().ok()?;
            if count == 0 { return None; }
            actions.push(CostAction::RemoveCounter(
                name.to_string(), count, "self".to_string(),
            ));
        } else {
            return None;
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
        "Duel.SelectMatchingCard" => spec_from_matching(&args, /*has_opp_locs=*/true, /*has_minmax=*/true)
            .map(|mut s| { s.from_resolve_select = true; s }),
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
        filter_mapped: map_group_filter(args.get(1).map(String::as_str).unwrap_or("")).is_some(),
        from_resolve_select: false,
        raw_filter: None,
    })
}

/// Map a lua filter predicate to a DSL `(kind, where-predicate)` pair.
/// Returns None for predicates with no DSL equivalent — callers decide
/// whether to over-approximate (action selectors) or skip (group-applied
/// modifiers, count exprs).
///
/// T38 S1 extends the literal table with the `aux.FaceupFilter(Card.IsX,
/// ARG)` composition — face-up ∧ predicate — routed through the same
/// predicate-atom mapper the named-filter path uses. Unknown predicates
/// or arguments poison the whole composition (skip-not-mis-emit).
fn map_group_filter(filter: &str) -> Option<(&'static str, Option<String>)> {
    let filter = filter.trim();
    Some(match filter {
        "nil" | "aux.TRUE" => ("card", None),
        "Card.IsFaceup"    => ("card", Some("is_face_up".to_string())),
        "Card.IsFacedown"  => ("card", Some("is_face_down".to_string())),
        "Card.IsMonster"   => ("monster", None),
        "Card.IsSpell"     => ("spell", None),
        "Card.IsTrap"      => ("trap", None),
        _ => {
            let inner = filter
                .strip_prefix("aux.FaceupFilter(")
                .and_then(|r| r.strip_suffix(')'))?;
            let pred = match inner.split_once(',') {
                Some((method, arg)) => {
                    lua_pred_method_to_where(method.trim(), arg.trim())?
                }
                // Zero-arg form: aux.FaceupFilter(Card.IsMonster) etc.
                None => lua_pred_method_to_where(inner.trim(), "")?,
            };
            // An or-carrying predicate under the face-up conjunction
            // would need a paren group — the v2 parser's nested-
            // predicate branch is broken (see lua_filter_expr_to_where),
            // so skip instead.
            if pred.contains(" or ") { return None; }
            return Some(("card", Some(format!("is_face_up and {}", pred))));
        }
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
    // Filters with a direct DSL equivalent refine the selector (Phase 10);
    // everything else keeps the lenient `card` kind established by earlier
    // phases but is flagged unmapped so group-applied paths can skip.
    let mapped = map_group_filter(args[0].as_str());
    let filter_mapped = mapped.is_some();
    let (kind, where_clause) = mapped.unwrap_or(("card", None));
    Some(SelectorSpec {
        quantity: "all".to_string(),
        kind: kind.to_string(),
        controller: Some(controller),
        zone,
        where_clause,
        filter_mapped,
        from_resolve_select: false,
        raw_filter: Some(args[0].clone()),
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
    collect_register_chains(block, None, false, &mut by_binding, &mut chains);
    chains
}

fn collect_register_chains(
    block: &Block,
    loop_source: Option<&str>,
    choice_arm: bool,
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
                                let mut seeded = existing;
                                seeded.clone_seeded = seeded_slot_names(&seeded);
                                by_binding.insert(name.clone(), seeded);
                            }
                        }
                    }
                }
            }
            Stmt::FunctionCall(fc) => {
                if let Some((bind, method, args)) = method_call_on_binding(fc) {
                    if let Some(chain) = by_binding.get_mut(&bind) {
                        let arg = args.first().cloned();
                        let seeds = &mut chain.clone_seeded;
                        let conflicted = match method.as_str() {
                            "SetCode"      => set_or_conflict(&mut chain.code, arg, seeds, "code"),
                            "SetValue"     => set_or_conflict(&mut chain.value, arg, seeds, "value"),
                            "SetReset"     => {
                                let conflicted = set_or_conflict(&mut chain.reset, arg, seeds, "reset");
                                // Count arg: absent means the lua default of 1 —
                                // overwrite (a clone-override `SetReset(X)` must
                                // not keep the source's stale count).
                                chain.reset_count = args.get(1).cloned();
                                conflicted
                            }
                            "SetTargetRange" => {
                                let joined = (!args.is_empty()).then(|| args.join(","));
                                set_or_conflict(&mut chain.target_range, joined, seeds, "target_range")
                            }
                            "SetTarget"    => set_or_conflict(&mut chain.set_target, arg, seeds, "set_target"),
                            "SetType"      => { chain.effect_type = arg; false }
                            "SetOperation" => set_or_conflict(&mut chain.operation, arg, seeds, "operation"),
                            "SetCondition" => set_or_conflict(&mut chain.condition, arg, seeds, "condition"),
                            _ => false,
                        };
                        if conflicted {
                            chain.conflicting_sets = true;
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
                            emitted.in_choice_arm = choice_arm;
                            out.push(emitted);
                        }
                    }
                } else if let Some(args) = try_duel_register_effect_call(fc) {
                    // `Duel.RegisterEffect(eN, player)` — player-scoped
                    // continuous chain. Use the player text as the chain's
                    // register_target so install_watcher emit still flows;
                    // modifier/grant translators won't fire because their
                    // `resolve_chain_selector` only accepts card sentinels.
                    if let Some(eff_name) = args.first() {
                        if let Some(chain) = by_binding.get(eff_name) {
                            let mut emitted = chain.clone();
                            emitted.register_target = args.get(1).cloned().unwrap_or_else(|| "tp".to_string());
                            emitted.duel_registered = true;
                            emitted.multi_target = loop_source.is_some();
                            emitted.loop_source_group = loop_source.map(str::to_string);
                            emitted.in_choice_arm = choice_arm;
                            out.push(emitted);
                        }
                    }
                }
            }
            Stmt::If(if_stmt) => {
                // An if with elseif/else arms encodes a runtime either/or
                // — chains registered inside ANY arm are one branch of a
                // choice (Phase 15 guard). A plain if is a guard, not a
                // choice.
                let has_arms = if_stmt.else_if().is_some_and(|eis| eis.len() > 0)
                    || if_stmt.else_block().is_some();
                let arm = choice_arm || has_arms;
                collect_register_chains(if_stmt.block(), loop_source, arm, by_binding, out);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_register_chains(ei.block(), loop_source, arm, by_binding, out);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_register_chains(else_block, loop_source, arm, by_binding, out);
                }
            }
            Stmt::While(w)       => collect_register_chains(w.block(), loop_source, choice_arm, by_binding, out),
            Stmt::Repeat(r)      => collect_register_chains(r.block(), loop_source, choice_arm, by_binding, out),
            Stmt::NumericFor(nf) => collect_register_chains(nf.block(), Some(""), choice_arm, by_binding, out),
            Stmt::GenericFor(gf) => {
                let inner = aux_next_source_group(gf)
                    .or_else(|| iter_method_source_group(gf));
                let inner_ref = inner.as_deref().or(Some(""));
                collect_register_chains(gf.block(), inner_ref, choice_arm, by_binding, out);
            }
            Stmt::Do(d)          => collect_register_chains(d.block(), loop_source, choice_arm, by_binding, out),
            _ => {}
        }
    }
}

/// Write `new` into a chain's Set* slot, reporting `true` when the slot
/// already held a DIFFERENT value — the branch-conditional double-set
/// idiom that makes the recorded payload one arm of a runtime choice.
/// Re-setting the same arg is not a conflict (idempotent rewrites).
///
/// A slot named in `seeds` holds a value inherited from `eN:Clone()`;
/// the first write there is the linear clone-then-override idiom — it
/// replaces the inherited value and consumes the seed mark instead of
/// conflicting.
fn set_or_conflict(
    slot: &mut Option<String>,
    new: Option<String>,
    seeds: &mut BTreeSet<&'static str>,
    slot_name: &'static str,
) -> bool {
    let was_seeded = seeds.remove(slot_name);
    let conflicted = !was_seeded
        && matches!((&*slot, &new), (Some(old), Some(n)) if old != n);
    if new.is_some() {
        *slot = new;
    }
    conflicted
}

/// Names of the conflict-tracked slots a chain currently holds values
/// for — these become the clone's seed marks.
fn seeded_slot_names(chain: &RegisterEffectChain) -> BTreeSet<&'static str> {
    let mut names = BTreeSet::new();
    if chain.code.is_some()      { names.insert("code"); }
    if chain.value.is_some()     { names.insert("value"); }
    if chain.reset.is_some()     { names.insert("reset"); }
    if chain.operation.is_some() { names.insert("operation"); }
    if chain.condition.is_some() { names.insert("condition"); }
    if chain.target_range.is_some() { names.insert("target_range"); }
    if chain.set_target.is_some()   { names.insert("set_target"); }
    names
}

// ── Phase 13: counter-op extraction ──────────────────────────────────────

/// Loop context threaded through the counter-op walk: the aux.Next-style
/// source group binding (empty string = untranslatable loop) plus the
/// loop variable name (empty when unknown).
#[derive(Clone, Copy)]
struct CounterLoopCtx<'a> {
    group: &'a str,
    var: &'a str,
}

/// Walk a function body for statement-level `<recv>:AddCounter(...)` /
/// `<recv>:RemoveCounter(...)` calls (Phase 13).
///
/// Returns an EMPTY vec when any counter op sat inside an elseif/else
/// arm: the if/else idiom encodes a runtime either/or, and emitting
/// both arms (or one of them) mis-states the card. Plain `if`-gated ops
/// are kept — the ubiquitous `if c:IsRelateToEffect(e) and c:IsFaceup()`
/// guard wraps virtually every operation body.
///
/// Calls in if-CONDITION position (`if c:AddCounter(...) then`) are not
/// statement-level and are deliberately invisible here — the gated-on-
/// return-value idiom has follow-up actions we can't model.
fn extract_counter_ops(block: &Block) -> Vec<CounterOp> {
    let mut out = Vec::new();
    let mut alt_tainted = false;
    collect_counter_ops(block, None, false, false, &mut out, &mut alt_tainted);
    if alt_tainted { return Vec::new(); }
    out
}

fn collect_counter_ops(
    block: &Block,
    loop_ctx: Option<CounterLoopCtx<'_>>,
    in_branch: bool,
    in_alt_arm: bool,
    out: &mut Vec<CounterOp>,
    alt_tainted: &mut bool,
) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::FunctionCall(fc) => {
                if let Some(op) = counter_op_from_fc(fc, loop_ctx, in_branch) {
                    if in_alt_arm { *alt_tainted = true; }
                    out.push(op);
                }
            }
            Stmt::If(if_stmt) => {
                collect_counter_ops(if_stmt.block(), loop_ctx, true, in_alt_arm, out, alt_tainted);
                for ei in if_stmt.else_if().into_iter().flatten() {
                    collect_counter_ops(ei.block(), loop_ctx, true, true, out, alt_tainted);
                }
                if let Some(else_block) = if_stmt.else_block() {
                    collect_counter_ops(else_block, loop_ctx, true, true, out, alt_tainted);
                }
            }
            // while/repeat/numeric-for loops have no translatable member
            // group — mark ops inside them multi-target with an empty
            // group so the emit path skips them.
            Stmt::While(w) => collect_counter_ops(
                w.block(), Some(CounterLoopCtx { group: "", var: "" }), true, in_alt_arm, out, alt_tainted),
            Stmt::Repeat(r) => collect_counter_ops(
                r.block(), Some(CounterLoopCtx { group: "", var: "" }), in_branch, in_alt_arm, out, alt_tainted),
            Stmt::NumericFor(nf) => collect_counter_ops(
                nf.block(), Some(CounterLoopCtx { group: "", var: "" }), in_branch, in_alt_arm, out, alt_tainted),
            Stmt::GenericFor(gf) => {
                let group = aux_next_source_group(gf)
                    .or_else(|| iter_method_source_group(gf))
                    .unwrap_or_default();
                let var = gf.names().iter().next()
                    .map(|n| n.token().to_string())
                    .unwrap_or_default();
                let ctx = CounterLoopCtx { group: &group, var: &var };
                collect_counter_ops(gf.block(), Some(ctx), in_branch, in_alt_arm, out, alt_tainted);
            }
            Stmt::Do(d) => collect_counter_ops(d.block(), loop_ctx, in_branch, in_alt_arm, out, alt_tainted),
            _ => {}
        }
    }
}

/// Build a `CounterOp` from a statement-level function call whose LAST
/// suffix is a `:AddCounter(...)` / `:RemoveCounter(...)` method call.
/// The receiver is the rendered prefix + intermediate suffixes (same
/// convention as `try_register_effect_call`), e.g. `e:GetHandler()`.
fn counter_op_from_fc(
    fc: &FunctionCall,
    loop_ctx: Option<CounterLoopCtx<'_>>,
    in_branch: bool,
) -> Option<CounterOp> {
    let suffixes: Vec<&Suffix> = fc.suffixes().collect();
    let last = suffixes.last()?;
    let (method, args) = match last {
        Suffix::Call(Call::MethodCall(mc)) => {
            let name = mc.name().token().to_string();
            if name != "AddCounter" && name != "RemoveCounter" { return None; }
            (name, call_args_to_strings(mc.args()))
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
    let add = method == "AddCounter";
    let (player_arg, counter_arg, count_arg) = if add {
        // AddCounter(countertype, count[, singly])
        if args.len() < 2 { return None; }
        (String::new(), args[0].clone(), args[1].clone())
    } else {
        // RemoveCounter(player, countertype, count, reason)
        if args.len() < 4 { return None; }
        (args[0].clone(), args[1].clone(), args[2].clone())
    };
    Some(CounterOp {
        add,
        receiver,
        counter_arg,
        count_arg,
        player_arg,
        multi_target: loop_ctx.is_some(),
        loop_source_group: loop_ctx.map(|c| c.group.to_string()),
        loop_var: loop_ctx.map(|c| c.var.to_string()),
        in_branch,
    })
}

/// Named `COUNTER_*` lua constants → countertype codes. The commonly-
/// used set from CardScripts/card_counter_constants.lua plus the two
/// flag bits from constant.lua. File-local `local COUNTER_X=0x…`
/// definitions are NOT resolved here — those ops skip (backlog).
fn counter_const_code(name: &str) -> Option<u32> {
    Some(match name {
        "COUNTER_A"         => 0x100e,
        "COUNTER_BUSHIDO"   => 0x3,
        "COUNTER_EC"        => 0x217,
        "COUNTER_FEATHER"   => 0x10,
        "COUNTER_FOG"       => 0x1019,
        "COUNTER_KAIJU"     => 0x37,
        "COUNTER_PREDATOR"  => 0x1041,
        "COUNTER_RESONANCE" => 0x211,
        "COUNTER_SIGNAL"    => 0x1148,
        "COUNTER_SPELL"     => 0x1,
        "COUNTER_VENOM"     => 0x1009,
        // Flag bits — appear as addends (`COUNTER_NEED_ENABLE+COUNTER_FOG`).
        "COUNTER_WITHOUT_PERMIT" => 0x1000,
        "COUNTER_NEED_ENABLE"    => 0x2000,
        _ => return None,
    })
}

/// Countertype code → display name, from EDOPro's authoritative
/// `!counter` table (ProjectIgnis/Distribution config/strings.conf,
/// fetched 2026-06-09). Codes keep the 0x1000 placed-without-permit bit
/// — it is part of counter identity there (0x148 Summon Counter vs
/// 0x1148 Signal Counter differ only in that bit). Entries whose name
/// embeds double quotes (`Counter ("B.E.S.")` etc.) are EXCLUDED: the
/// DSL string literal has no escape syntax, so those ops skip.
fn counter_code_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0x1 => "Spell Counter",
        0x3 => "Bushido Counter",
        0x4 => "Psychic Counter",
        0x5 => "Shine Counter",
        0x6 => "Crystal Counter",
        0x8 => "Morph Counter",
        0xa => "Genex Counter",
        0xc => "Thunder Counter",
        0xd => "Greed Counter",
        0xf => "Worm Counter",
        0x10 => "Black Feather Counter",
        0x11 => "Hyper Venom Counter",
        0x12 => "Karakuri Counter",
        0x13 => "Chaos Counter",
        0x16 => "Spellstone Counter",
        0x17 => "Nut Counter",
        0x18 => "Flower Counter",
        0x1a => "Payback Counter",
        0x1b => "Clock Counter",
        0x1c => "D Counter",
        0x1d => "Junk Counter",
        0x1e => "Gate Counter",
        0x20 => "Plant Counter",
        0x22 => "Dragonic Counter",
        0x23 => "Ocean Counter",
        0x25 => "Chronicle Counter",
        0x2b => "Destiny Counter",
        0x2c => "You Got It Boss! Counter",
        0x2e => "Shark Counter",
        0x2f => "Pumpkin Counter",
        0x30 => "Hi-Five the Sky Counter",
        0x31 => "Rising Sun Counter",
        0x32 => "Balloon Counter",
        0x33 => "Yosen Counter",
        0x35 => "Symphonic Counter",
        0x36 => "Performage Counter",
        0x37 => "Kaiju Counter",
        0x43 => "Defect Counter",
        0x4a => "Athlete Counter",
        0x55 => "Hammer Counter",
        0x59 => "Otoshidamashi Counter",
        0x90 => "Maiden Counter",
        0x91 => "Speed Counter",
        0x92 => "Plasma Counter",
        0x93 => "Sacred Beast Counter",
        0x94 => "Earthbound Immortal Counter",
        0x95 => "Crest Counter",
        0x96 => "Battle Buffer Counter",
        0x99 => "Full Moon Counter",
        0xfb => "Trickstar Counter",
        0x103 => "Medal Counter",
        0x107 => "Gearspring Counter",
        0x147 => "Borrel Counter",
        0x148 => "Summon Counter",
        0x201 => "Fire Fist Counter",
        0x202 => "Phantasm Counter",
        0x207 => "Emperor's Key Counter",
        0x20a => "Piece Counter",
        0x20c => "G Golem Counter",
        0x211 => "Resonance Counter",
        0x212 => "Access Counter",
        0x213 => "Schoolwork Counter",
        0x577 => "Hydradrive Counter",
        0x584 => "Counter (Ai Ai Wall)",
        0x1002 => "Wedge Counter",
        0x1009 => "Venom Counter",
        0x100e => "A-Counter",
        0x1015 => "Ice Counter",
        0x1019 => "Fog Counter",
        0x1021 => "Guard Counter",
        0x1024 => "String Counter",
        0x1038 => "Cubic Counter",
        0x1039 => "Zushin Counter",
        0x1041 => "Predator Counter",
        0x1045 => "Scale Counter",
        0x1049 => "Patrol Counter",
        0x1090 => "Maiden Counter",
        0x1096 => "Protection Counter",
        0x1097 => "Des Counter",
        0x1098 => "Chain Counter",
        0x109a => "Scab Counter",
        0x1100 => "Aura Counter",
        0x1101 => "Hallucination Counter",
        0x1102 => "Gear Counter",
        0x1104 => "Thorn Counter",
        0x1105 => "Turn Counter",
        0x1106 => "Shield Counter",
        0x1107 => "Prey Counter",
        0x1108 => "Vaccine Counter",
        0x1109 => "Life Star Counter",
        0x1110 => "Beacon Counter",
        0x1112 => "Disturbance Counter",
        0x1113 => "Charge Counter",
        0x1115 => "G Golem Counter",
        0x1148 => "Signal Counter",
        0x1149 => "Venemy Counter",
        0x1207 => "Burnup Counter",
        0x1208 => "Bunny Ear Counter",
        0x1209 => "Deranged Counter",
        _ => return None,
    })
}

/// Resolve a raw lua countertype argument to its display name.
///
/// Accepts `COUNTER_X`, `0x…` hex literals, and `+`/`|` combinations of
/// those (the `COUNTER_NEED_ENABLE+COUNTER_FOG` idiom). Terms are
/// resolved numerically and OR-ed, then the 0x2000 NEED_ENABLE bit —
/// pure placement-permission metadata — is cleared before the name
/// lookup. Unknown constants and unlisted codes return None (skip,
/// never invent a name).
fn counter_arg_to_name(raw: &str) -> Option<&'static str> {
    let mut code = 0u32;
    for term in raw.split(['+', '|']) {
        let term = term.trim();
        let v = if let Some(c) = counter_const_code(term) {
            c
        } else if let Some(hex) = term.strip_prefix("0x") {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            return None;
        };
        code |= v;
    }
    counter_code_name(code & !0x2000)
}

/// Inspect a `for <names> in <exprs>` loop. If the iterator expression
/// is `aux.Next(<group>)`, return the group binding name. Other shapes
/// (`pairs`, `ipairs`, custom iterators) return None — caller still
/// flags the loop as multi-target, but without a translatable source.
// ── Phase 16: SelectOption label-branch choose extraction ────

/// Parse `aux.Stringid(id, N)` → N. The first arg must be the literal
/// `id` binding from `local s,id=GetID()` — option text resolution at
/// emit time indexes the card's own CDB strings (str1 = index 0).
fn stringid_index(raw: &str) -> Option<u32> {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let inner = s.strip_prefix("aux.Stringid(id,")?.strip_suffix(')')?;
    let n: u32 = inner.parse().ok()?;
    (n < 16).then_some(n)
}

/// Count `Duel.SelectOption` calls anywhere in a block (recursive —
/// reuses the duel-call walker, which descends into branches & loops).
fn count_select_option_calls(block: &Block) -> usize {
    extract_duel_calls(block).iter()
        .filter(|c| c.method == "Duel.SelectOption")
        .count()
}

/// Count `<recv>:SetLabel(...)` statements anywhere in a block.
fn count_set_label_calls(block: &Block) -> usize {
    let mut n = 0usize;
    count_set_label_in(block, &mut n);
    n
}

fn count_set_label_in(block: &Block, n: &mut usize) {
    for stmt in block.stmts() {
        match stmt {
            Stmt::FunctionCall(fc) => {
                if let Some((_, method, _)) = method_call_on_binding(fc) {
                    if method == "SetLabel" { *n += 1; }
                }
            }
            Stmt::If(i) => {
                count_set_label_in(i.block(), n);
                for ei in i.else_if().into_iter().flatten() { count_set_label_in(ei.block(), n); }
                if let Some(b) = i.else_block() { count_set_label_in(b, n); }
            }
            Stmt::While(w)       => count_set_label_in(w.block(), n),
            Stmt::Repeat(r)      => count_set_label_in(r.block(), n),
            Stmt::NumericFor(f)  => count_set_label_in(f.block(), n),
            Stmt::GenericFor(f)  => count_set_label_in(f.block(), n),
            Stmt::Do(d)          => count_set_label_in(d.block(), n),
            _ => {}
        }
    }
}

/// Find the Phase 16 target-side idiom at the TOP level of a handler
/// body: `local <v> = Duel.SelectOption(tp, aux.Stringid(id,a), …)`
/// followed by `e:SetLabel(<v>)`. Conditional SelectOption (dynamic
/// option lists — Junk Changer's level-gated single option), multiple
/// SelectOption calls anywhere in the body, or extra SetLabel writes
/// all return None: the recorded option list would be one runtime
/// shape of many (skip-not-mis-emit).
fn extract_select_option(block: &Block) -> Option<SelectOptionSpec> {
    if count_select_option_calls(block) != 1 { return None; }
    if count_set_label_calls(block) != 1 { return None; }
    let mut bound_var: Option<String> = None;
    let mut spec: Option<SelectOptionSpec> = None;
    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                if names.len() == 1 && exprs.len() == 1 {
                    if let Expression::FunctionCall(fc) = exprs[0] {
                        if let Some(call) = duel_call_from_fc(fc) {
                            if call.method == "Duel.SelectOption" {
                                if call.args.first().map(String::as_str) != Some("tp") {
                                    return None;
                                }
                                let ids: Vec<u32> = call.args[1..].iter()
                                    .map(|a| stringid_index(a))
                                    .collect::<Option<Vec<u32>>>()?;
                                if ids.len() < 2 { return None; }
                                bound_var = Some(names[0].clone());
                                spec = Some(SelectOptionSpec { string_ids: ids });
                            }
                        }
                    }
                }
            }
            Stmt::FunctionCall(fc) => {
                if let Some((bind, method, args)) = method_call_on_binding(fc) {
                    if method == "SetLabel" {
                        // Must store the SelectOption result on `e`.
                        if bind != "e" { return None; }
                        let var = bound_var.as_deref()?;
                        if args.len() != 1 || args[0] != var { return None; }
                        return spec;
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Scan state for [`extract_choice_arms`]. Straight-line group/value
/// bindings declared before the dispatch are collected here so arms can
/// resolve references to them; bindings inside OTHER arms stay
/// invisible (each arm extracts its own block only).
#[derive(Default)]
struct DispatchScan {
    label_var: Option<String>,
    /// Option list when the SelectOption sits in THIS body (the op-side
    /// inline form, no SetLabel hop). None for the SetLabel-linked form
    /// — the option list then comes from the target handler.
    inline_options: Option<SelectOptionSpec>,
    arms: Option<Vec<FunctionBody>>,
    outer_groups: BTreeMap<String, SelectorSpec>,
    outer_values: BTreeMap<String, String>,
    /// Register-effect chains under construction in the straight-line
    /// scope (`local e1=Effect.CreateEffect(…)` + `e1:Set*` writes).
    chains: BTreeMap<String, RegisterEffectChain>,
    /// A chain whose construction forked on the label: one variant per
    /// option. Post-fork `Set*` writes apply to every variant; the
    /// commit (`RegisterEffect`) finalizes the variants into arms.
    fork: Option<ChainFork>,
    failed: bool,
}

struct ChainFork {
    binding: String,
    variants: Vec<RegisterEffectChain>,
}

/// Walk an operation-handler body looking for the Phase 16 label
/// dispatch. Two recognized sources for the label:
///   - `local <w> = e:GetLabel()` (SetLabel-linked; options live in the
///     target handler), or
///   - `local <w> = Duel.SelectOption(tp, aux.Stringid(id,a), …)`
///     inline in this body.
/// Two recognized dispatch shapes on `if <w>==0 … elseif <w>==1 … else`:
///   - statement arms — each arm is a self-contained alternative
///     (Three Strikes Barrier), or
///   - a chain-slot fork — every arm only writes `Set*` slots on ONE
///     chain under construction (Armored Kappa's SetCode fork); the
///     shared prefix/suffix writes apply to every variant.
/// Returns the inline option list (if any) plus one synthesized
/// FunctionBody per arm in option order. Empty arms when no dispatch
/// matches or any statement outside the dispatch could carry an action
/// the emitted choose would silently drop (skip-not-mis-emit).
fn extract_choice_arms(block: &Block) -> (Option<SelectOptionSpec>, Vec<FunctionBody>) {
    let mut st = DispatchScan::default();
    scan_dispatch_block(block, &mut st);
    // An unconsumed fork means the chain never registered on this path.
    if st.failed || st.fork.is_some() { return (None, Vec::new()); }
    match st.arms {
        Some(arms) => (st.inline_options, arms),
        None => (None, Vec::new()),
    }
}

/// Parse `local <v> = Duel.SelectOption(tp, aux.Stringid(id,a), …)`
/// from a single-name local-assignment RHS.
fn inline_select_option(expr: &Expression) -> Option<SelectOptionSpec> {
    let Expression::FunctionCall(fc) = expr else { return None };
    let call = duel_call_from_fc(fc)?;
    if call.method != "Duel.SelectOption" { return None; }
    if call.args.first().map(String::as_str) != Some("tp") { return None; }
    let ids: Vec<u32> = call.args[1..].iter()
        .map(|a| stringid_index(a))
        .collect::<Option<Vec<u32>>>()?;
    (ids.len() >= 2).then_some(SelectOptionSpec { string_ids: ids })
}

fn scan_dispatch_block(block: &Block, st: &mut DispatchScan) {
    for stmt in block.stmts() {
        if st.failed { return; }
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                for (i, name) in names.iter().enumerate() {
                    let Some(expr) = exprs.get(i) else { continue };
                    let text = expr.to_string().trim().to_string();
                    if text == "e:GetLabel()" {
                        if st.label_var.is_some() { st.failed = true; return; }
                        st.label_var = Some(name.clone());
                        continue;
                    }
                    if let Some(spec) = inline_select_option(expr) {
                        // Op-side inline SelectOption — this local IS
                        // the label. A second label source is ambiguous.
                        if st.label_var.is_some() { st.failed = true; return; }
                        st.label_var = Some(name.clone());
                        st.inline_options = Some(spec);
                        continue;
                    }
                    // Any other local whose RHS hides an action call
                    // (incl. a malformed/dynamic SelectOption, which
                    // translates as TODO) would be dropped by the
                    // choose emit — reject the body.
                    if expr_has_action_call(expr) { st.failed = true; return; }
                    if expr_is_effect_createeffect(expr) {
                        st.chains.insert(name.clone(), RegisterEffectChain::default());
                    } else if let Expression::FunctionCall(fc) = *expr {
                        if let Some(spec) = selector_spec_from_call(fc) {
                            st.outer_groups.insert(name.clone(), spec);
                        } else if !text.is_empty() {
                            st.outer_values.insert(name.clone(), text);
                        }
                    } else if !text.is_empty() {
                        st.outer_values.insert(name.clone(), text);
                    }
                }
            }
            Stmt::FunctionCall(fc) => {
                if scan_dispatch_call(fc, st) {
                    if st.failed { return; }
                    continue;
                }
                // Otherwise only cosmetic / query Duel calls may sit
                // outside the arms (Duel.Hint, Duel.SetTargetPlayer, …).
                match duel_call_from_fc(fc) {
                    Some(call) if translate_call(&call, &FunctionBody::default()).is_none() => {}
                    _ => { st.failed = true; return; }
                }
            }
            Stmt::If(if_stmt) => {
                if try_dispatch_if(if_stmt, st) {
                    if st.failed { return; }
                } else if !if_stmt.else_if().is_some_and(|e| e.len() > 0)
                    && if_stmt.else_block().is_none()
                {
                    // Plain-if guard. `if … then return end` early-outs
                    // are fine as-is; otherwise recurse — the dispatch
                    // often sits inside an IsRelateToEffect wrapper.
                    if expr_has_action_call(if_stmt.condition()) { st.failed = true; return; }
                    if !block_is_bare_return(if_stmt.block()) {
                        scan_dispatch_block(if_stmt.block(), st);
                    }
                } else {
                    st.failed = true;
                    return;
                }
            }
            _ => { st.failed = true; return; }
        }
    }
    // A trailing bare `return` is harmless; a value return is not.
    match block.last_stmt() {
        None => {}
        Some(LastStmt::Return(r)) if r.returns().is_empty() => {}
        _ => { st.failed = true; }
    }
}

/// Handle a chain-related statement in the straight-line dispatch
/// scope: `Set*` writes on a tracked (or forked) chain, and the
/// `RegisterEffect` commits that finalize a fork into arms. Returns
/// true when the call was chain-related (possibly setting `st.failed`),
/// false to let the caller try the cosmetic-call path.
fn scan_dispatch_call(fc: &FunctionCall, st: &mut DispatchScan) -> bool {
    if let Some((receiver, args)) = try_register_effect_call(fc) {
        finalize_chain_register(args.first(), receiver, false, st);
        return true;
    }
    if let Some(args) = try_duel_register_effect_call(fc) {
        let player = args.get(1).cloned().unwrap_or_else(|| "tp".to_string());
        finalize_chain_register(args.first(), player, true, st);
        return true;
    }
    if let Some((bind, method, margs)) = method_call_on_binding(fc) {
        if let Some(fork) = st.fork.as_mut() {
            if fork.binding == bind {
                if !method.starts_with("Set") { st.failed = true; return true; }
                for v in &mut fork.variants {
                    apply_chain_write(v, &method, &margs);
                }
                return true;
            }
        }
        if let Some(chain) = st.chains.get_mut(&bind) {
            if !method.starts_with("Set") { st.failed = true; return true; }
            apply_chain_write(chain, &method, &margs);
            return true;
        }
    }
    false
}

/// Commit a chain binding. A forked binding finalizes into one arm per
/// variant; an UNFORKED chain registering in the straight-line scope is
/// an unconditional action outside the arms — reject (the choose emit
/// would drop it).
fn finalize_chain_register(
    binding: Option<&String>,
    register_target: String,
    duel_registered: bool,
    st: &mut DispatchScan,
) {
    let Some(binding) = binding else { st.failed = true; return };
    if let Some(fork) = st.fork.take() {
        if &fork.binding == binding {
            if st.arms.is_some() { st.failed = true; return; }
            let arms = fork.variants.into_iter().map(|mut chain| {
                chain.register_target = register_target.clone();
                chain.duel_registered = duel_registered;
                FunctionBody {
                    group_bindings: st.outer_groups.clone(),
                    value_bindings: st.outer_values.clone(),
                    register_chains: vec![chain],
                    ..FunctionBody::default()
                }
            }).collect();
            st.arms = Some(arms);
            return;
        }
        st.fork = Some(fork);
    }
    st.failed = true;
}

/// Apply one `Set*` write to a chain snapshot. Mirrors the slot list of
/// `collect_register_chains`; methods untracked there (SetDescription,
/// SetProperty, SetHintTiming, …) are cosmetic here too.
fn apply_chain_write(chain: &mut RegisterEffectChain, method: &str, args: &[String]) {
    match method {
        "SetCode"        => chain.code = args.first().cloned(),
        "SetValue"       => chain.value = args.first().cloned(),
        "SetReset"       => {
            chain.reset = args.first().cloned();
            chain.reset_count = args.get(1).cloned();
        }
        "SetTargetRange" => chain.target_range = (!args.is_empty()).then(|| args.join(",")),
        "SetTarget"      => chain.set_target = args.first().cloned(),
        "SetType"        => chain.effect_type = args.first().cloned(),
        "SetOperation"   => chain.operation = args.first().cloned(),
        "SetCondition"   => chain.condition = args.first().cloned(),
        _ => {}
    }
}

/// Try to read `if_stmt` as the label dispatch. Returns true when the
/// head matched `<w>==0` / `e:GetLabel()==0` and the statement was
/// CONSUMED — either into statement arms (`st.arms`), a chain-slot
/// fork (`st.fork`, finalized later at the chain's RegisterEffect
/// commit), or a failure (`st.failed`: bad elseif ladder, second
/// dispatch, arm whitelist miss). Returns false when the head doesn't
/// match — the caller treats the if as a guard.
fn try_dispatch_if(if_stmt: &ast::If, st: &mut DispatchScan) -> bool {
    if !label_cond_matches(if_stmt.condition(), st.label_var.as_deref(), 0) {
        return false;
    }
    let mut arm_blocks: Vec<&Block> = vec![if_stmt.block()];
    let mut next = 1u32;
    for ei in if_stmt.else_if().into_iter().flatten() {
        if !label_cond_matches(ei.condition(), st.label_var.as_deref(), next) {
            st.failed = true;
            return true;
        }
        arm_blocks.push(ei.block());
        next += 1;
    }
    if let Some(else_block) = if_stmt.else_block() {
        arm_blocks.push(else_block);
    }
    if arm_blocks.len() < 2 {
        // `if op==0 then … end` with no alternative — option 2's arm is
        // implicit do-nothing, which the choose grammar can't express.
        st.failed = true;
        return true;
    }
    if st.fork.is_some() || st.arms.is_some() {
        // Second dispatch on the same label — ambiguous.
        st.failed = true;
        return true;
    }
    // Chain-slot fork? Every arm writes Set* slots on the SAME chain
    // currently under construction.
    if let Some((binding, per_arm_writes)) = slot_fork_arms(&arm_blocks, st) {
        let base = st.chains.remove(&binding).expect("fork binding tracked");
        let variants = per_arm_writes.into_iter().map(|writes| {
            let mut v = base.clone();
            for (method, args) in writes {
                apply_chain_write(&mut v, &method, &args);
            }
            v
        }).collect();
        st.fork = Some(ChainFork { binding, variants });
        return true;
    }
    // Statement arms — each a self-contained alternative.
    let mut arms = Vec::new();
    for ab in arm_blocks {
        let mut chain_bindings = BTreeSet::new();
        if !choice_arm_shape_ok(ab, &mut chain_bindings) {
            st.failed = true;
            return true;
        }
        arms.push(build_arm_body(ab, &st.outer_groups, &st.outer_values));
    }
    st.arms = Some(arms);
    true
}

/// Classify dispatch arms as a chain-slot fork: every arm's statements
/// are exclusively `<b>:Set*(…)` writes on one shared binding `b` that
/// is a chain under construction in the enclosing scope. Returns the
/// binding plus each arm's ordered writes.
#[allow(clippy::type_complexity)]
fn slot_fork_arms(
    arm_blocks: &[&Block],
    st: &DispatchScan,
) -> Option<(String, Vec<Vec<(String, Vec<String>)>>)> {
    let mut binding: Option<String> = None;
    let mut per_arm = Vec::new();
    for ab in arm_blocks {
        if ab.last_stmt().is_some() { return None; }
        let mut writes = Vec::new();
        for stmt in ab.stmts() {
            let Stmt::FunctionCall(fc) = stmt else { return None };
            let (bind, method, args) = method_call_on_binding(fc)?;
            if !method.starts_with("Set") || !st.chains.contains_key(&bind) { return None; }
            match &binding {
                Some(b) if *b != bind => return None,
                None => binding = Some(bind.clone()),
                _ => {}
            }
            writes.push((method, args));
        }
        if writes.is_empty() { return None; }
        per_arm.push(writes);
    }
    Some((binding?, per_arm))
}

/// Whitespace-insensitive match of a dispatch condition against
/// `<label_var>==k` or the inline `e:GetLabel()==k` form.
fn label_cond_matches(cond: &Expression, label_var: Option<&str>, k: u32) -> bool {
    let s: String = cond.to_string().chars().filter(|c| !c.is_whitespace()).collect();
    if s == format!("e:GetLabel()=={}", k) { return true; }
    label_var.is_some_and(|w| s == format!("{}=={}", w, k))
}

/// True when the expression contains a Duel.* call that would translate
/// to a DSL line — anything but a known cosmetic / query method.
fn expr_has_action_call(expr: &Expression) -> bool {
    let mut calls = Vec::new();
    collect_duel_calls_in_expr(expr, &mut calls);
    calls.iter().any(|c| translate_call(c, &FunctionBody::default()).is_some())
}

/// True for a block that is exactly one bare `return` (the
/// `if not … then return end` early-out idiom).
fn block_is_bare_return(block: &Block) -> bool {
    block.stmts().next().is_none()
        && matches!(block.last_stmt(), Some(LastStmt::Return(r)) if r.returns().is_empty())
}

/// Structural whitelist for one STATEMENT arm. Every statement must be
/// a shape the per-arm extractors capture — otherwise the emitted
/// option would silently omit lua behavior. Chain Set*/RegisterEffect
/// calls must target bindings CREATED in this arm: a write to an outer
/// chain is the slot-fork idiom, which `slot_fork_arms` classifies
/// before this whitelist runs.
fn choice_arm_shape_ok(block: &Block, chain_bindings: &mut BTreeSet<String>) -> bool {
    for stmt in block.stmts() {
        match stmt {
            Stmt::LocalAssignment(la) => {
                let names: Vec<String> = la.names().iter()
                    .map(|n| n.token().to_string()).collect();
                let exprs: Vec<&Expression> = la.expressions().iter().collect();
                for (i, name) in names.iter().enumerate() {
                    let Some(expr) = exprs.get(i) else { continue };
                    if expr_is_effect_createeffect(expr) {
                        chain_bindings.insert(name.clone());
                    } else if let Some(src) = expr_clone_source(expr) {
                        if !chain_bindings.contains(&src) { return false; }
                        chain_bindings.insert(name.clone());
                    }
                }
            }
            Stmt::FunctionCall(fc) => {
                if let Some((_, args)) = try_register_effect_call(fc) {
                    match args.first() {
                        Some(b) if chain_bindings.contains(b) => {}
                        _ => return false,
                    }
                } else if let Some(args) = try_duel_register_effect_call(fc) {
                    match args.first() {
                        Some(b) if chain_bindings.contains(b) => {}
                        _ => return false,
                    }
                } else if let Some((bind, method, _)) = method_call_on_binding(fc) {
                    let chain_set = chain_bindings.contains(&bind) && method.starts_with("Set");
                    let counter = method == "AddCounter" || method == "RemoveCounter";
                    if !chain_set && !counter { return false; }
                } else if duel_call_from_fc(fc).is_some() {
                    // Captured by the arm's calls extraction; emit-time
                    // coverage rejects untranslatable ones.
                } else {
                    return false;
                }
            }
            Stmt::If(if_stmt) => {
                // Plain guard only — an inner else would hide a second
                // either/or the per-arm extractors can't represent
                // (counter-op poisoning, conflicting Set* writes).
                if if_stmt.else_if().is_some_and(|e| e.len() > 0)
                    || if_stmt.else_block().is_some()
                {
                    return false;
                }
                if !choice_arm_shape_ok(if_stmt.block(), chain_bindings) { return false; }
            }
            _ => return false,
        }
    }
    block.last_stmt().is_none()
}

/// Synthesize a FunctionBody for one dispatch arm. Group/value bindings
/// declared in the outer straight-line scope (before the dispatch) are
/// visible to the arm; arm-local bindings shadow them.
fn build_arm_body(
    block: &Block,
    outer_groups: &BTreeMap<String, SelectorSpec>,
    outer_values: &BTreeMap<String, String>,
) -> FunctionBody {
    let mut group_bindings = outer_groups.clone();
    let (arm_groups, arm_sources) = extract_group_bindings(block);
    group_bindings.extend(arm_groups);
    let mut value_bindings = outer_values.clone();
    value_bindings.extend(extract_value_bindings(block));
    FunctionBody {
        calls: extract_duel_calls(block),
        group_bindings,
        register_chains: extract_register_chains(block),
        value_bindings,
        counter_ops: extract_counter_ops(block),
        select_sources: arm_sources,
        ..FunctionBody::default()
    }
}

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

/// Inspect a `for <names> in <exprs>` loop for the `<group>:Iter()`
/// iterator shape (modern corpus equivalent of `aux.Next(<group>)`).
/// Returns the group binding name on match (Phase 10).
fn iter_method_source_group(gf: &ast::GenericFor) -> Option<String> {
    let expr = gf.expressions().iter().next()?;
    let fc = match expr {
        Expression::FunctionCall(fc) => fc,
        _ => return None,
    };
    let prefix = match fc.prefix() {
        ast::Prefix::Name(n) => n.token().to_string(),
        _ => return None,
    };
    let suffixes: Vec<&Suffix> = fc.suffixes().collect();
    if suffixes.len() != 1 { return None; }
    match suffixes[0] {
        Suffix::Call(Call::MethodCall(mc))
            if mc.name().token().to_string() == "Iter" => Some(prefix),
        _ => None,
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
/// Detect the `Duel.RegisterEffect(eN, player)` global-registration shape.
/// Returns the raw arg strings (first = effect binding, second = player
/// expression) on match, else None. Used as a fallback when the method-
/// call form (`<recv>:RegisterEffect(eN)`) doesn't fit.
fn try_duel_register_effect_call(fc: &FunctionCall) -> Option<Vec<String>> {
    let prefix_name = match fc.prefix() {
        ast::Prefix::Name(n) => n.token().to_string(),
        _ => return None,
    };
    if prefix_name != "Duel" { return None; }
    let suffixes: Vec<&Suffix> = fc.suffixes().collect();
    if suffixes.len() < 2 { return None; }
    // First suffix: `.RegisterEffect`
    match suffixes[0] {
        Suffix::Index(Index::Dot { name, .. })
            if name.token().to_string() == "RegisterEffect" => {}
        _ => return None,
    }
    // Second suffix: the `(eN, player)` argument list.
    match suffixes[1] {
        Suffix::Call(Call::AnonymousCall(args)) => {
            Some(call_args_to_strings(args))
        }
        _ => None,
    }
}

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

/// Passcode → card-name table for `EFFECT_CHANGE_CODE` translation
/// (Phase 11). Populated by the apply binary from BabelCdb before
/// translating; unit tests register fixture names directly. When empty,
/// change-code chains skip instead of emitting an unresolvable id.
static CARD_NAMES: OnceLock<Mutex<BTreeMap<u32, String>>> = OnceLock::new();

/// Register passcode → name pairs for `EFFECT_CHANGE_CODE` lookup.
/// Extends (never replaces) the table so repeated registration — e.g.
/// multiple unit tests in one process — is additive and order-free.
pub fn register_card_names<I: IntoIterator<Item = (u32, String)>>(pairs: I) {
    let table = CARD_NAMES.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut map) = table.lock() {
        map.extend(pairs);
    }
}

/// Resolve a passcode to its registered card name, if any.
fn lookup_card_name(id: u32) -> Option<String> {
    CARD_NAMES.get()?.lock().ok()?.get(&id).cloned()
}

/// Process-wide passcode → CDB strs (str1..str16) table for choose-
/// option label resolution (Phase 16). Mirrors [`CARD_NAMES`].
static CARD_STRINGS: OnceLock<Mutex<BTreeMap<u32, Vec<String>>>> = OnceLock::new();

/// Register passcode → CDB string-list pairs (Phase 16).
pub fn register_card_strings<I: IntoIterator<Item = (u32, Vec<String>)>>(pairs: I) {
    let table = CARD_STRINGS.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut map) = table.lock() {
        map.extend(pairs);
    }
}

/// Resolve `aux.Stringid(<card_id>, <idx>)` to its CDB string. Returns
/// None when missing, empty, or containing a double quote — the DSL
/// string lexer has no escapes, so such labels skip (not mis-emit).
pub fn lookup_card_string(card_id: u32, idx: u32) -> Option<String> {
    let map = CARD_STRINGS.get()?.lock().ok()?;
    let s = map.get(&card_id)?.get(idx as usize)?.trim().to_string();
    (!s.is_empty() && !s.contains('"')).then_some(s)
}

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
        ..FunctionBody::default()
    })
}

/// Selector-aware translator entry point. Emits Duel.* action lines first
/// (Phase 2/3 behavior), then any continuous-modifier `RegisterEffect`
/// chains the body created (Phase 4 — `modify_atk` / `modify_def`).
pub fn translate_body(body: &FunctionBody) -> Vec<DslLine> {
    translate_body_with_functions(body, &BTreeMap::new())
}

/// Variant of [`translate_body`] that has access to the surrounding
/// function-table. Used by translator passes that need to follow
/// `SetOperation(s.<name>)` references into another handler body — for
/// example, the install_watcher path materialises the sub-handler's
/// translated lines as the `check { ... }` body.
pub fn translate_body_with_functions(
    body: &FunctionBody,
    functions: &BTreeMap<String, FunctionBody>,
) -> Vec<DslLine> {
    // T38 S1 — upgrade group bindings whose named script-local filter
    // (`s.filter`) resolves to an exact where-clause. Walk time froze
    // the spec before the filter fn's body was visible; the refinement
    // needs the function table, so it lives here.
    let refined = refine_group_bindings(body, functions);
    let body = refined.as_ref().unwrap_or(body);
    let mut out = Vec::new();
    for c in &body.calls {
        if let Some(line) = translate_call(c, body) {
            out.push(line);
        }
    }
    // Select-then-equip helper calls (Phase 17) — `s.equipop(c,e,tp,tc)`
    // delegating to the canonical EquipByEffectAndLimitRegister wrapper.
    for (name, args) in &body.helper_calls {
        if let Some(line) = translate_equip_helper_call(name, args, body, functions) {
            out.push(line);
        }
    }
    // Counter ops (Phase 13) — emitted after the Duel.* stream. Bodies
    // mixing both are rare and the relative order of a counter placement
    // vs. other actions is not observable in the DSL's resolve model.
    for op in &body.counter_ops {
        if let Some(line) = translate_counter_op(op, body) {
            out.push(line);
        }
    }
    // Stat-write interference guard (Phase 10): lua computes values like
    // `local lv = c:GetLevel()+tc:GetLevel()` ONCE before registering both
    // chains, but the emitted DSL lines evaluate sequentially — a later
    // line whose expr reads a stat an earlier line already wrote would see
    // the post-write value. Drop such lines instead of mis-emitting.
    let mut stat_writes: Vec<(String, String)> = Vec::new();
    for chain in &body.register_chains {
        for line in translate_register_chain(chain, body, functions) {
            if let DslLine::Action(text) = &line {
                if stat_writes.iter().any(|(sel, stat)| {
                    text.contains(&format!("{}.{}", sel, stat))
                }) {
                    continue;
                }
                if let Some(write) = stat_write_of(text) {
                    stat_writes.push(write);
                }
            }
            out.push(line);
        }
    }
    out
}

/// True when translating `body` would DROP a `RegisterEffect` chain —
/// i.e. some chain in the handler produces no DSL line (T38 S1). The
/// apply passes gate resolve fills on this: every chain a handler
/// registers is semantic content, so a fill that emits the translatable
/// sibling while silently omitting another (a delayed EVENT_* trigger, a
/// grant with an unmapped reset, a field-wide registration) under-states
/// the card. Skip-not-mis-emit at the whole-fill granularity.
///
/// Choice-arm chains gate too: they never emit from a plain body walk
/// (only the Phase 16 choose path renders them), so a body carrying them
/// fails here — a plain fill would state one arm's context while
/// dropping the dispatch.
pub fn body_drops_chains(
    body: &FunctionBody,
    functions: &BTreeMap<String, FunctionBody>,
) -> bool {
    let refined = refine_group_bindings(body, functions);
    let body = refined.as_ref().unwrap_or(body);
    body.register_chains.iter().any(|chain| {
        translate_register_chain(chain, body, functions).is_empty()
    })
}

/// Phase 16 — a fully-translated SelectOption choose block: one
/// `(stringid index, action lines)` pair per option, in option order.
#[derive(Debug)]
pub struct ChooseSpec {
    pub options: Vec<(u32, Vec<DslLine>)>,
}

/// Link a [`SelectOptionSpec`] — op-side inline, or SetLabel-linked
/// from the target handler — to the operation handler's label-dispatch
/// arms and translate every arm (Phase 16). Returns None unless the
/// option count matches the arm count AND every arm translates
/// completely — skip-not-mis-emit.
pub fn extract_choose_spec(
    target_body: Option<&FunctionBody>,
    op_body: &FunctionBody,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<ChooseSpec> {
    let sel = op_body.select_option.as_ref()
        .or_else(|| target_body?.select_option.as_ref())?;
    if op_body.choice_arms.is_empty() || op_body.choice_arms.len() != sel.string_ids.len() {
        return None;
    }
    let mut options = Vec::new();
    for (i, arm) in op_body.choice_arms.iter().enumerate() {
        options.push((sel.string_ids[i], translate_choice_arm(arm, functions)?));
    }
    Some(ChooseSpec { options })
}

/// Translate one dispatch arm with FULL coverage: every Duel call must
/// be an action or a known cosmetic/query, every register chain must
/// emit, every counter op must map, and the stat-write interference
/// guard must not need to drop anything. Any gap fails the whole arm —
/// an option that under-states its lua branch would mis-state the card.
fn translate_choice_arm(
    arm: &FunctionBody,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<Vec<DslLine>> {
    let mut out = Vec::new();
    for c in &arm.calls {
        match translate_call(c, arm) {
            Some(DslLine::Action(s)) => out.push(DslLine::Action(s)),
            Some(DslLine::Todo(_)) => return None,
            None => {}
        }
    }
    for op in &arm.counter_ops {
        match translate_counter_op(op, arm) {
            Some(line) if line.is_action() => out.push(line),
            _ => return None,
        }
    }
    let mut stat_writes: Vec<(String, String)> = Vec::new();
    for chain in &arm.register_chains {
        let lines = translate_register_chain(chain, arm, functions);
        if lines.is_empty() { return None; }
        for line in lines {
            let DslLine::Action(text) = &line else { return None };
            if stat_writes.iter().any(|(sel, stat)| {
                text.contains(&format!("{}.{}", sel, stat))
            }) {
                return None;
            }
            if let Some(write) = stat_write_of(text) { stat_writes.push(write); }
            out.push(line);
        }
    }
    if out.is_empty() { return None; }
    Some(out)
}

/// If a DSL action line writes a stat on `self` / `target`, return the
/// (selector, stat) pair — e.g. `set_level self …` → ("self", "level").
/// Group-selector writes return None: they can't be referenced back via
/// a `sel.stat` expr, so they can't interfere.
fn stat_write_of(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("modify_").or_else(|| line.strip_prefix("set_"))?;
    let (stat, rest) = rest.split_once(' ')?;
    let sel = rest.split_whitespace().next()?;
    if sel != "self" && sel != "target" { return None; }
    Some((sel.to_string(), stat.to_string()))
}

/// Map one `RegisterEffectChain` to DSL action lines. Returns an empty
/// vec when the chain isn't one of the shapes the translator covers.
///
/// Families:
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
///   - **Non-stat passives** (Phase 11): EFFECT_CHANGE_ATTRIBUTE /
///     EFFECT_CHANGE_RACE / EFFECT_CHANGE_CODE / EFFECT_IMMUNE_EFFECT.
///     IMMUNE may expand to two grant lines (spell+trap immunity), so
///     this dispatcher returns a Vec.
fn translate_register_chain(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
    functions: &BTreeMap<String, FunctionBody>,
) -> Vec<DslLine> {
    // Branch-conditional Set* writes — the recorded payload is one arm of
    // a runtime choice; any emit would be a mis-emit. Skip.
    if chain.conflicting_sets {
        return Vec::new();
    }
    // EFFECT_TYPE_EQUIP chains register on the equip card but apply to
    // the EQUIPPED monster (Tyrant Wing: +400/400 to the monster, not
    // the trap) — they take the dedicated `equipped_card`-receiver path
    // (Phase 15) instead of the register-target selector lowering below.
    if chain.effect_type.as_deref().is_some_and(|t| t.contains("EFFECT_TYPE_EQUIP")) {
        return translate_equip_chain(chain, body).into_iter().collect();
    }
    let Some(code) = chain.code.as_deref() else { return Vec::new() };
    // EFFECT_TYPE_FIELD chains apply to the cards/players named by
    // SetTargetRange, NOT to the register receiver — receiver-based
    // selector paths (grant/modifier/disable) would mis-aim them (a
    // c-registered field chain restricting opponent monsters must not
    // become `grant self …`). Route them to the Phase 15 field-scope
    // translator; only the install_watcher path (EVENT codes on
    // FIELD+CONTINUOUS watcher chains) may still fall through.
    if chain.effect_type.as_deref().is_some_and(|t| t.contains("EFFECT_TYPE_FIELD")) {
        if let Some(line) = translate_field_scope_chain(code, chain, functions) {
            return vec![line];
        }
        if let Some(line) = translate_player_restrict_chain(code, chain, functions) {
            return vec![line];
        }
        if let Some(line) = translate_damage_rule_chain(code, chain, functions) {
            return vec![line];
        }
        if let Some(trigger) = trigger_for_event_code(code) {
            return translate_install_watcher_chain(trigger, chain, functions)
                .into_iter()
                .collect();
        }
        return Vec::new();
    }
    if code == "EFFECT_IMMUNE_EFFECT" {
        return translate_immune_chain(chain, body, functions);
    }
    let single = if let Some(action) = stat_modifier_action(code) {
        translate_modifier_chain(action, chain, body)
    } else if let Some(action) = set_stat_action(code) {
        translate_set_stat_chain(action, chain, body)
    } else if code == "EFFECT_EXTRA_ATTACK" {
        translate_extra_attack_chain(chain, body)
    } else if code == "EFFECT_DISABLE" {
        translate_disable_chain(chain, body)
    } else if code == "EFFECT_CHANGE_ATTRIBUTE" || code == "EFFECT_CHANGE_RACE" {
        translate_change_property_chain(code, chain, body)
    } else if code == "EFFECT_CHANGE_CODE" {
        translate_change_code_chain(chain, body)
    } else if let Some(ability) = grant_ability_for(code) {
        translate_grant_chain(ability, chain, body)
    } else if let Some(trigger) = trigger_for_event_code(code) {
        translate_install_watcher_chain(trigger, chain, functions)
    } else {
        None
    };
    single.into_iter().collect()
}

/// Map a SET_*_FINAL effect code to the DSL `set_atk` / `set_def` action.
/// `_FINAL` variants override the base value after all modifiers — DSL has
/// no equivalent priority concept yet, so both base and final variants
/// emit the same atom. Returns None for non-set codes.
fn set_stat_action(code: &str) -> Option<&'static str> {
    Some(match code {
        "EFFECT_SET_ATTACK"        => "set_atk",
        "EFFECT_SET_ATTACK_FINAL"  => "set_atk",
        "EFFECT_SET_BASE_ATTACK"   => "set_atk",
        "EFFECT_SET_DEFENSE"       => "set_def",
        "EFFECT_SET_DEFENSE_FINAL" => "set_def",
        "EFFECT_SET_BASE_DEFENSE"  => "set_def",
        // CHANGE_LEVEL sets the level absolutely. Emitted as `set_level`
        // rather than `change_level <sel> to <N>` because change_property
        // grammar has no duration clause — reset-bearing lua chains would
        // silently lose their end-of-turn bound there.
        "EFFECT_CHANGE_LEVEL"      => "set_level",
        _ => return None,
    })
}

/// Set-stat chain → `set_atk <selector> <value>` / `set_def <selector> <value>`.
///
/// Distinct from `translate_modifier_chain`: no `+`/`-` op, no negative
/// magnitudes — the value is set absolutely. Reuses `parse_lua_value` for
/// literal / method-call / local-var resolution.
fn translate_set_stat_chain(
    action: &str,
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    let parsed = parse_lua_value(chain.value.as_deref()?, &body.value_bindings)?;
    if parsed.negative { return None; }
    // Group-applied chains can't carry per-member values: `target.` refs
    // resolve to the selected target, not each loop member (Phase 10 guard).
    if chain.multi_target && parsed.expr.contains("target.") { return None; }
    let selector = resolve_chain_selector(chain, body)?;
    let mut line = format!("{} {} {}", action, selector, parsed.expr);
    match reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref()) {
        Some(dur) => line.push_str(&format!(" until {}", dur)),
        // A reset the mapping can't decode is an unaudited lifetime —
        // emitting without a duration would silently mean "permanently".
        None if chain.reset.is_some() => return None,
        None => {}
    }
    Some(DslLine::Action(line))
}

/// Map an EVENT_* code (from `SetCode`) to the DSL `trigger_expr` form.
/// Returns None for events outside the install_watcher shape this
/// translator currently covers (compound `EVENT_PHASE+PHASE_END` shapes,
/// chain-event family, summon-success variants, etc. are deferred).
fn trigger_for_event_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "EVENT_BATTLE_DESTROYING"   => "destroys_by_battle",
        "EVENT_BATTLE_DESTROYED"    => "destroyed_by_battle",
        "EVENT_DESTROYED"           => "destroyed",
        "EVENT_TO_GRAVE"            => "sent_to gy",
        "EVENT_LEAVE_FIELD"         => "leaves_field",
        "EVENT_BATTLE_DAMAGE"       => "battle_damage",
        "EVENT_ATTACK_ANNOUNCE"     => "attack_declared",
        "EVENT_REMOVE"              => "banished",
        "EVENT_FLIP_SUMMON_SUCCESS" => "flip_summoned",
        "EVENT_SPSUMMON_SUCCESS"   => "special_summoned",
        "EVENT_SUMMON_SUCCESS"     => "normal_summoned",
        "EVENT_BE_MATERIAL"        => "used_as_material",
        // Chain-event: a chain link activates. Without analysing the
        // SetCondition body we don't know who; emit `any_activates` as
        // the broadest valid trigger. The sub-handler usually gates on
        // `Duel.IsChainNegatable` and calls NegateActivation, so the
        // semantic shape matches counter-trap negation.
        "EVENT_CHAINING"           => "any_activates",
        // Compound phase-event shape: EVENT_PHASE + PHASE_<X> in lua.
        "EVENT_PHASE+PHASE_END"     => "end_phase",
        "EVENT_PHASE+PHASE_STANDBY" => "standby_phase",
        "EVENT_PHASE+PHASE_BATTLE"  => "battle_phase",
        _ => return None,
    })
}

/// Install-watcher chain → single-line DSL:
/// `install_watcher "<name>" { event: <trigger> duration: <dur> check { <action> } }`.
///
/// Narrow shape this implementation accepts:
///   - `SetCode` maps to a trigger via `trigger_for_event_code`.
///   - `SetReset` resolves to end-of-turn (the only duration grammar accepts
///     here today; durations beyond this require T-series grammar work).
///   - `SetOperation` names a function in `functions`, and translating that
///     handler body produces at least one DSL action line. The first action
///     line becomes the watcher's `check { … }` body. Subsequent lines are
///     dropped — multi-action checks need richer emit.
fn translate_install_watcher_chain(
    trigger: &str,
    chain: &RegisterEffectChain,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<DslLine> {
    // Watcher duration is hardcoded `end_of_turn`; only accept resets
    // that map to that keyword (don't let damage-step variants slip
    // through with the wrong literal duration).
    if reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref())
        != Some("end_of_turn")
    {
        return None;
    }
    // A SetCondition handler gates when the chain may fire (Tyrant Wing's
    // s.descon: destroy only if the equipped monster attacked twice). The
    // watcher grammar has no condition slot, so emitting would run the
    // check unconditionally on every event. Skip-not-mis-emit.
    if chain.condition.is_some() {
        return None;
    }
    // Choice-arm watcher chains state one arm of a runtime either/or as
    // THE effect — same poison as every other chain family. Latent until
    // Phase 18: caught live on c84117021 (Spell Wall), whose else-arm
    // EVENT_ATTACK_ANNOUNCE watcher became emittable the moment its check
    // body (a restrict line) translated.
    if chain.in_choice_arm {
        return None;
    }
    let op_name = chain.operation.as_deref()?;
    let op_body = functions.get(op_name)?;
    let lines = translate_body_with_functions(op_body, functions);
    // Collect every translated ACTION line from the sub-handler. The DSL
    // `check { action+ }` grammar accepts whitespace-separated action
    // atoms, so we join with a single space rather than emit a multi-line
    // block (which would force the renderer to bake per-line indent into
    // the DslLine::Action string).
    let actions: Vec<String> = lines.into_iter().filter_map(|l| match l {
        DslLine::Action(s) => Some(s),
        _ => None,
    }).collect();
    if actions.is_empty() { return None; }
    // Use the sub-handler name (sans `s.` prefix) as the watcher label so
    // re-applies stay idempotent and the corpus diff stays inspectable.
    let label = op_name.strip_prefix("s.").unwrap_or(op_name);
    Some(DslLine::Action(format!(
        "install_watcher \"{}\" {{ event: {} duration: end_of_turn check {{ {} }} }}",
        label, trigger, actions.join(" "),
    )))
}

/// Map an `EFFECT_UPDATE_*` code to the DSL action verb. Returns None
/// for codes outside the stat-modifier family.
fn stat_modifier_action(code: &str) -> Option<&'static str> {
    Some(match code {
        "EFFECT_UPDATE_ATTACK"  => "modify_atk",
        "EFFECT_UPDATE_DEFENSE" => "modify_def",
        "EFFECT_UPDATE_LEVEL"   => "modify_level",
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
        "EFFECT_CANNOT_SPECIAL_SUMMON"   => "cannot_special_summon",
        "EFFECT_CANNOT_SUMMON"           => "cannot_normal_summon",
        "EFFECT_DIRECT_ATTACK"           => "direct_attack",
        "EFFECT_PIERCE"                  => "piercing",
        "EFFECT_ATTACK_ALL"              => "attack_all_monsters",
        "EFFECT_MUST_ATTACK"             => "must_attack",
        "EFFECT_CANNOT_CHANGE_POSITION"  => "cannot_change_position",
        _ => return None,
    })
}

/// Resolve the DSL selector from the chain's `register_target` /
/// `loop_source_group`. Shared by the stat-modifier and grant paths.
///
/// Single-target (`multi_target=false`):
///   - `tc` single-assigned from `Duel.GetFirstTarget(...)` → `target`
///   - `<g>:GetFirst()` with `g` bound to `Duel.GetTargetCards(...)` → `target`
///   - `c` / `e:GetHandler()` → `self`
///   - any other `tc` provenance (GetFieldCard, SelectMatchingCard,
///     GetAttacker, GetLabelObject, rebinding, …) → None (Phase 13b
///     skip-not-mis-emit gate)
///
/// Multi-target (`multi_target=true`):
///   - `loop_source_group` resolves to a binding in
///     `body.group_bindings` → emit using the spec's DSL form.
///   - `loop_source_group` is a local bound to `Duel.GetTargetCards(…)`
///     → `target` (the loop visits exactly the declared targets).
///   - Otherwise → None.
fn resolve_chain_selector(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<String> {
    resolve_body_selector(
        &chain.register_target,
        chain.multi_target,
        chain.loop_source_group.as_deref(),
        body,
    )
}

/// Shared receiver → DSL selector lowering used by both the
/// RegisterEffect-chain path (above) and the Phase 13 counter-op path.
/// See `resolve_chain_selector`'s doc comment for the case table.
fn resolve_body_selector(
    receiver: &str,
    multi_target: bool,
    loop_source_group: Option<&str>,
    body: &FunctionBody,
) -> Option<String> {
    if multi_target {
        let group = loop_source_group?;
        if let Some(spec) = body.group_bindings.get(group) {
            // A dropped lua filter means the DSL selector matches a SUPERSET
            // of the group the lua iterated — fine when the engine then picks
            // targets, wrong when a modifier applies to every match. Skip
            // (Phase 10 skip-not-mis-emit).
            if !spec.filter_mapped { return None; }
            return Some(spec.to_dsl());
        }
        // Loop over the effect's own chosen targets (Phase 11):
        // `local g=Duel.GetTargetCards(e)` + `for tc in aux.Next(g)` —
        // each member IS a declared target, so the DSL `target` selector
        // covers the whole group.
        let rhs = body.value_bindings.get(group)?;
        if rhs.starts_with("Duel.GetTargetCards(") {
            return Some("target".to_string());
        }
        None
    } else {
        let recv = receiver;
        if recv == "c" || recv == "e:GetHandler()" {
            return Some("self".to_string());
        }
        // Bare `tc` is only the declared target when the body
        // single-assigns it from `Duel.GetFirstTarget(...)` (the taint
        // logic in `extract_value_bindings` drops reassigned names).
        // `tc` bound via Duel.GetFieldCard / Duel.SelectMatchingCard /
        // Duel.GetAttacker / e:GetLabelObject etc. is a different card
        // entirely — emitting `target` mis-aims the line. Same gate as
        // the Phase 13 counter-op path (caught live on c35787450
        // Eternal Dread).
        if recv == "tc"
            && body
                .value_bindings
                .get("tc")
                .is_some_and(|rhs| rhs.starts_with("Duel.GetFirstTarget("))
        {
            return Some("target".to_string());
        }
        // `<g>:GetFirst()` receiver — first member of a group local.
        // The declared-target group (`Duel.GetTargetCards`) lowers to
        // `target` (the loop visits exactly the declared targets).
        if let Some(var) = recv.strip_suffix(":GetFirst()") {
            if body
                .value_bindings
                .get(var)
                .is_some_and(|rhs| rhs.starts_with("Duel.GetTargetCards("))
            {
                return Some("target".to_string());
            }
        }
        // Phase 17 — chain registered on a one-card selection local
        // (`tc` bound from `Duel.SelectMatchingCard(...):GetFirst()` or
        // via `g:GetFirst()` of a select-bound group). The selection
        // folds into the line's selector. Requires exactly one consumer
        // of the selection — a second consumer would re-select instead
        // of reusing the pick.
        let sel_base = recv.strip_suffix(":GetFirst()").unwrap_or(recv);
        if let Some(spec) = body.group_bindings.get(sel_base) {
            if spec.from_resolve_select
                && spec.quantity == "1"
                && selection_consumers(body, sel_base) == 1
            {
                return Some(spec.to_dsl());
            }
        }
        None
    }
}

/// Counter op → `place_counter "<name>" <n> on <sel>` /
/// `remove_counter "<name>" <n> from <sel>` (Phase 13).
///
/// Skip gates (None):
///   - countertype with no curated name (unknown constants, file-local
///     `local COUNTER_X=…` aliases, quoted strings.conf names);
///   - non-literal or zero count — the grammar slot is `unsigned`, and
///     variable counts (`ct`, `e:GetLabel()`, …) have no DSL lowering;
///   - RemoveCounter whose player arg isn't `tp`;
///   - receivers outside the known self/target sentinels;
///   - loop ops whose receiver is not the loop variable, or whose
///     source group doesn't lower (while/repeat/numeric-for, unmapped
///     filters).
fn translate_counter_op(op: &CounterOp, body: &FunctionBody) -> Option<DslLine> {
    let name = counter_arg_to_name(&op.counter_arg)?;
    let count: u32 = op.count_arg.trim().parse().ok()?;
    if count == 0 { return None; }
    if !op.add && op.player_arg.trim() != "tp" { return None; }
    if op.multi_target && op.loop_var.as_deref() != Some(op.receiver.as_str()) {
        // `c:AddCounter(...)` inside a group loop targets the card
        // itself once per member — not a per-member placement we can
        // express. Skip.
        return None;
    }
    let selector = if op.multi_target {
        resolve_body_selector(
            &op.receiver,
            true,
            op.loop_source_group.as_deref(),
            body,
        )?
    } else {
        // Stricter than the chain path's bare-`tc` sentinel: counter
        // receivers named `tc` only map to `target` when the body
        // single-assigns `tc = Duel.GetFirstTarget(...)` (taint logic in
        // `extract_value_bindings` drops reassigned names). Caught live
        // on Eternal Dread (c35787450), where `tc` is the field-zone
        // card via Duel.GetFieldCard — not a declared target.
        match op.receiver.as_str() {
            "c" | "e:GetHandler()" => "self".to_string(),
            "tc" if body
                .value_bindings
                .get("tc")
                .is_some_and(|rhs| rhs.starts_with("Duel.GetFirstTarget(")) =>
            {
                "target".to_string()
            }
            _ => return None,
        }
    };
    let line = if op.add {
        format!("place_counter \"{}\" {} on {}", name, count, selector)
    } else {
        format!("remove_counter \"{}\" {} from {}", name, count, selector)
    };
    Some(DslLine::Action(line))
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
    // Group-applied chains can't carry per-member values: `target.` refs
    // resolve to the selected target, not each loop member (Phase 10 guard).
    if chain.multi_target && parsed.expr.contains("target.") { return None; }
    let selector = resolve_chain_selector(chain, body)?;
    let op = if parsed.negative { '-' } else { '+' };
    let mut line = format!("{} {} {} {}", action, selector, op, parsed.expr);
    match reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref()) {
        Some(dur) => line.push_str(&format!(" until {}", dur)),
        // A reset the mapping can't decode is an unaudited lifetime —
        // emitting without a duration would silently mean "permanently".
        None if chain.reset.is_some() => return None,
        None => {}
    }
    Some(DslLine::Action(line))
}

/// EFFECT_TYPE_EQUIP chain → action line aimed at `equipped_card` (Phase 15).
///
/// Equip-type chains register on the equip card but apply to the monster
/// it is equipped to. The resolve-time `equipped_card` selector reads back
/// the `__equipped__` binding that `Action::Equip` produces, so emission is
/// gated on the same handler performing a translatable `Duel.Equip` (the
/// canonical `equip self to target` shape) — without that producer the
/// selector would resolve to nothing or to a stale card.
///
/// Duration: `RESET_EVENT|RESETS_STANDARD` on an equip chain fires when
/// the equip relation breaks (Tyrant Wing's +400/+400 persists across
/// turns while the trap stays equipped) — `while_equipped`, one notch
/// tighter than the `while_face_up` the shared mapping would give.
/// An explicit PHASE_END reset still means end-of-turn and wins.
///
/// Skip gates (None):
///   - handler has no canonical `Duel.Equip(tp, c, tc)` producer call;
///   - chain registered on something other than the equip card itself;
///   - group-applied chains (`multi_target`);
///   - codes outside the covered set (UPDATE_ATK/DEF, DISABLE,
///     EXTRA_ATTACK[_MONSTER], grant-ability codes).
fn translate_equip_chain(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    if chain.multi_target { return None; }
    if chain.register_target != "c" && chain.register_target != "e:GetHandler()" {
        return None;
    }
    let has_equip_producer = body.calls.iter().any(|c| {
        c.method == "Duel.Equip"
            && matches!(action_equip(&c.args, &body.group_bindings, body), DslLine::Action(_))
    });
    if !has_equip_producer { return None; }
    let dur = if chain.reset.as_deref().is_some_and(|r| r.contains("PHASE_END")) {
        "end_of_turn"
    } else {
        "while_equipped"
    };
    let code = chain.code.as_deref()?;
    match code {
        "EFFECT_UPDATE_ATTACK" | "EFFECT_UPDATE_DEFENSE" => {
            let parsed = parse_lua_value(chain.value.as_deref()?, &body.value_bindings)?;
            if parsed.expr == "0" { return None; }
            let action = if code == "EFFECT_UPDATE_ATTACK" { "modify_atk" } else { "modify_def" };
            let op = if parsed.negative { '-' } else { '+' };
            Some(DslLine::Action(format!(
                "{} equipped_card {} {} until {}",
                action, op, parsed.expr, dur,
            )))
        }
        "EFFECT_DISABLE" => Some(DslLine::Action(format!(
            "negate_effects equipped_card {}",
            dur,
        ))),
        "EFFECT_EXTRA_ATTACK" | "EFFECT_EXTRA_ATTACK_MONSTER" => {
            let value: i64 = chain.value.as_deref()?.trim().parse().ok()?;
            let ability = match value {
                1 => "double_attack",
                2 => "triple_attack",
                _ => return None,
            };
            Some(DslLine::Action(format!(
                "grant equipped_card {} until {}",
                ability, dur,
            )))
        }
        _ => {
            let ability = grant_ability_for(code)?;
            Some(DslLine::Action(format!(
                "grant equipped_card {} until {}",
                ability, dur,
            )))
        }
    }
}

/// Grant chain → `grant <selector> <ability> until <duration>`.
///
/// Phase 4e covers non-stat ability codes that lua expresses as
/// `SetCode(EFFECT_<X>) + SetValue(1) + SetReset(<...>)
/// + <recv>:RegisterEffect(...)`. The reset gate is mandatory: a chain
/// without a recognisable reset would emit a permanent grant from an
/// ambiguous-duration chain, so skip those instead of guessing.
fn translate_grant_chain(
    ability: &str,
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    let dur = reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref())?;
    let selector = resolve_chain_selector(chain, body)?;
    Some(DslLine::Action(format!(
        "grant {} {} until {}",
        selector, ability, dur,
    )))
}

/// EFFECT_DISABLE chain → `negate_effects <selector> <duration>`.
///
/// In the lua corpus EFFECT_DISABLE is the primary negate-effects code;
/// the paired EFFECT_DISABLE_EFFECT chain that usually follows expresses
/// the same intent on already-active effects. We translate only EFFECT_DISABLE
/// here so paired cards emit a single DSL line; EFFECT_DISABLE_EFFECT is
/// intentionally not mapped (would duplicate the action). A mapped reset
/// is mandatory: a chain without a duration would emit a permanent negate.
fn translate_disable_chain(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    let dur = reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref())?;
    let selector = resolve_chain_selector(chain, body)?;
    Some(DslLine::Action(format!(
        "negate_effects {} {}",
        selector, dur,
    )))
}

/// Field-scope restriction chain (Phase 15):
/// `EFFECT_TYPE_FIELD + SetTargetRange(LOCATION…, LOCATION…) +
/// Duel.RegisterEffect(eN, tp)` → `grant (all, …) <ability> until
/// end_of_turn` or `negate_effects (all, …) end_of_turn`.
///
/// Skip gates (None) — every one is a confirmed corpus sub-shape:
///   - codes outside the card-scoped table below: damage-shaping
///     (CHANGE_DAMAGE / AVOID_BATTLE_DAMAGE / REFLECT_* / REVERSE_*)
///     has no DSL form; player-scoped restrictions (CANNOT_ACTIVATE /
///     CANNOT_SPECIAL_SUMMON / CANNOT_BP / SKIP_BP …) fall through to
///     the Phase 18 `restrict` path instead;
///   - registration to any player expr other than the literal `tp`
///     (SetTargetRange sides are relative to the registering player, so
///     `p` / `1-tp` registrations would flip the controller decode);
///   - card-registered FIELD chains (`!duel_registered`) — lifetime is
///     tied to the card's presence on field, inexpressible here;
///   - player-flag or OR'd/unknown LOCATION TargetRange values;
///   - SetTarget filters with no kind/where decode (`e:GetLabel()`
///     except-target idiom, runtime-arg predicates, …);
///   - SetValue other than the literal flag `1` (function values like
///     `aux.indoval` qualify the restriction to one player's effects —
///     emitting the unqualified ability would over-grant);
///   - SetCondition / SetOperation closures we can't classify;
///   - non-standard resets: anything that isn't a plain end-of-turn
///     phase reset with an (implicit) count of 1, including
///     RESET_SELF_TURN / RESET_OPPO_TURN qualifiers.
fn translate_field_scope_chain(
    code: &str,
    chain: &RegisterEffectChain,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<DslLine> {
    if !chain.duel_registered || chain.multi_target || chain.in_choice_arm {
        return None;
    }
    if chain.register_target.trim() != "tp" {
        return None;
    }
    if chain.condition.is_some() || chain.operation.is_some() {
        return None;
    }
    // Standard end-of-turn lifetime only.
    if reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref()) != Some("end_of_turn") {
        return None;
    }
    let reset = chain.reset.as_deref().unwrap_or("");
    if reset.contains("RESET_SELF_TURN") || reset.contains("RESET_OPPO_TURN") {
        return None; // turn-qualified phase reset ≠ end of CURRENT turn
    }
    if chain.reset_count.as_deref().is_some_and(|c| c.trim() != "1") {
        return None; // survives N reset events — not end-of-turn
    }
    let negate = code == "EFFECT_DISABLE";
    let ability = if negate { None } else { Some(field_grant_ability_for(code)?) };
    // Value: restriction grants use SetValue(1) as a flag (or omit it);
    // anything else is a qualifier function — skip. EFFECT_DISABLE takes
    // no value in this shape.
    match chain.value.as_deref().map(str::trim) {
        None => {}
        Some("1") if !negate => {}
        _ => return None,
    }
    let (kind, where_clause) = match chain.set_target.as_deref() {
        None => ("card".to_string(), None),
        Some(filter) => field_filter_to_kind_where(filter, functions)?,
    };
    let selector = field_range_selector(
        chain.target_range.as_deref()?,
        &kind,
        where_clause.as_deref(),
    )?;
    Some(DslLine::Action(match ability {
        Some(a) => format!("grant {} {} until end_of_turn", selector, a),
        None => format!("negate_effects {} end_of_turn", selector),
    }))
}

/// Card-scoped EFFECT codes a field-range restriction can express as a
/// DSL `grant` ability. Deliberately excludes the player-scoped codes
/// that share grant_ability_for's spellings (EFFECT_CANNOT_SPECIAL_SUMMON
/// / EFFECT_CANNOT_SUMMON): on a FIELD chain their TargetRange is player
/// flags and the restriction binds the PLAYER, which the card-selector
/// grant form cannot express — those route to the Phase 18 `restrict`
/// path instead.
fn field_grant_ability_for(code: &str) -> Option<&'static str> {
    Some(match code {
        "EFFECT_INDESTRUCTABLE_BATTLE"   => "cannot_be_destroyed by battle",
        "EFFECT_INDESTRUCTABLE_EFFECT"   => "cannot_be_destroyed by effect",
        "EFFECT_CANNOT_ATTACK"           => "cannot_attack",
        "EFFECT_CANNOT_DIRECT_ATTACK"    => "cannot_attack_directly",
        "EFFECT_CANNOT_BE_EFFECT_TARGET" => "cannot_be_targeted",
        "EFFECT_CANNOT_CHANGE_POSITION"  => "cannot_change_position",
        "EFFECT_DIRECT_ATTACK"           => "direct_attack",
        "EFFECT_PIERCE"                  => "piercing",
        "EFFECT_ATTACK_ALL"              => "attack_all_monsters",
        "EFFECT_MUST_ATTACK"             => "must_attack",
        _ => return None,
    })
}

/// Player-scoped restriction chain (Phase 18, T36):
/// `EFFECT_TYPE_FIELD + SetTargetRange(<player flags>) + SetCode(<player
/// code>) + standard reset + Duel.RegisterEffect(eN, tp)` →
/// `restrict <you|opponent|both_players> <keyword> <duration>`.
///
/// Inverts the T36 `PlayerRestriction` doc table exactly: the six
/// value-less codes map 1:1; `EFFECT_CANNOT_ACTIVATE` keys on the
/// `SetValue` activation filter via `classify_activate_filter`.
///
/// Skip gates (None) — skip-not-mis-emit, all confirmed corpus sub-shapes:
///   - codes outside the 11-keyword T36 vocabulary (CANNOT_DRAW,
///     CANNOT_ATTACK_ANNOUNCE, …);
///   - any `SetTarget` filter — the "cannot special summon, except X"
///     summon-limit idiom qualifies WHICH cards the player is barred
///     from; the unfiltered keyword would over-restrict;
///   - `SetValue` on the value-less codes other than the literal flag
///     `1`, and CANNOT_ACTIVATE filters outside the four T36 shapes
///     (attribute / location / code-specific limits);
///   - SetCondition / SetOperation closures (floodgate-style gating);
///   - registration to any player expr other than the literal `tp`,
///     card-registered chains, group-applied chains, choice arms;
///   - non-standard resets: turn-qualified (RESET_SELF_TURN /
///     RESET_OPPO_TURN), reset counts above 1 (two-turn lifetimes),
///     and missing resets (would guess `permanently`).
fn translate_player_restrict_chain(
    code: &str,
    chain: &RegisterEffectChain,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<DslLine> {
    if !chain.duel_registered || chain.multi_target || chain.in_choice_arm {
        return None;
    }
    if chain.register_target.trim() != "tp" {
        return None;
    }
    if chain.condition.is_some() || chain.operation.is_some() {
        return None;
    }
    if chain.set_target.is_some() {
        return None; // filtered restriction ("…except X" summon limits)
    }
    let dur = reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref())?;
    let reset = chain.reset.as_deref().unwrap_or("");
    if reset.contains("RESET_SELF_TURN") || reset.contains("RESET_OPPO_TURN") {
        return None; // turn-qualified phase reset ≠ end of CURRENT turn
    }
    if chain.reset_count.as_deref().is_some_and(|c| c.trim() != "1") {
        return None; // survives N reset events — two-turn lifetime
    }
    let scope = player_scope_from_range(chain.target_range.as_deref()?)?;
    let value = chain.value.as_deref().map(str::trim);
    let keyword = if code == "EFFECT_CANNOT_ACTIVATE" {
        classify_activate_filter(value?, functions)?
    } else {
        // Value-less codes: lua uses no SetValue (or the literal flag 1);
        // anything else is a qualifier function — skip.
        match value {
            None | Some("1") => {}
            Some(_) => return None,
        }
        match code {
            "EFFECT_CANNOT_SPECIAL_SUMMON" => "cannot_special_summon",
            "EFFECT_CANNOT_SUMMON"         => "cannot_normal_summon",
            "EFFECT_CANNOT_MSET"           => "cannot_set_monsters",
            "EFFECT_CANNOT_SSET"           => "cannot_set_spells_traps",
            "EFFECT_CANNOT_BP"             => "cannot_conduct_battle_phase",
            "EFFECT_SKIP_BP"               => "skip_battle_phase",
            _ => return None,
        }
    };
    Some(DslLine::Action(format!("restrict {} {} {}", scope, keyword, dur)))
}

/// Decode a FIELD chain's player-flag `SetTargetRange` into the DSL
/// player scope. Sides are relative to the registering player (gated to
/// the literal `tp` upstream): (1,0) = "you", (0,1) = "opponent",
/// (1,1) = "both_players". LOCATION masks and anything else return None.
fn player_scope_from_range(range: &str) -> Option<&'static str> {
    let cleaned: String = range.chars().filter(|c| !c.is_whitespace()).collect();
    Some(match cleaned.as_str() {
        "1,0" => "you",
        "0,1" => "opponent",
        "1,1" => "both_players",
        _ => return None,
    })
}

/// Classify an `EFFECT_CANNOT_ACTIVATE` chain's `SetValue` activation
/// filter into the matching T36 keyword. Accepted forms:
///   - the literal flag `1` (everything) → `cannot_activate`;
///   - inline `function(e,re[,tp]) return <expr> end` closures — the
///     activating-effect param is the SECOND one;
///   - named `s.*` filters resolved through the walk's function table
///     whose body is a single `return <expr>` (the `s.aclimit` idiom).
/// Everything else (locals, multi-statement filters) returns None.
fn classify_activate_filter(
    value: &str,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<&'static str> {
    let value = value.trim();
    if value == "1" {
        return Some("cannot_activate");
    }
    if let Some((param, expr)) = activate_filter_closure_body(value) {
        return classify_activate_expr(&expr, &param);
    }
    // Named file-local filter — corpus convention names the activating
    // effect `re` (or `te`); param names aren't recorded, so try both.
    // Conjunct receivers must match uniformly, so a mis-bound first
    // param can't slip through.
    let expr = functions.get(value)?.return_expr.as_deref()?;
    classify_activate_expr(expr, "re").or_else(|| classify_activate_expr(expr, "te"))
}

/// Extract the (effect-param, return-expression) of an inline
/// `function(e,re[,tp]) return <expr> end` activation-filter closure.
/// The activating effect is the SECOND param by lua convention
/// (`(e, re, tp)` / `(_, re)`).
fn activate_filter_closure_body(raw: &str) -> Option<(String, String)> {
    let r = raw.strip_prefix("function")?.trim_start();
    let r = r.strip_prefix('(')?;
    let (params, r) = r.split_once(')')?;
    let params: Vec<&str> = params.split(',').map(str::trim).collect();
    if !(2..=3).contains(&params.len()) {
        return None;
    }
    let r = r.trim_start().strip_prefix("return")?;
    let expr = r.trim().strip_suffix("end")?.trim().to_string();
    Some((params[1].to_string(), expr))
}

/// Classify an activation-filter expression (conjunction of predicates
/// on the activating effect `eff`) into a T36 keyword. The accept set is
/// closed — exactly the four filter shapes the T36 survey blessed; any
/// other conjunct (attribute / location / code-specific limits, owner
/// comparisons, …) fails the whole filter (skip-not-mis-emit).
fn classify_activate_expr(expr: &str, eff: &str) -> Option<&'static str> {
    let normalized = expr.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.contains(" or ") {
        return None;
    }
    // Conjunct atoms over the activating effect.
    let mut card_activation = false; // IsHasType(EFFECT_TYPE_ACTIVATE)
    let mut monster = false;         // IsMonsterEffect()
    let mut spell = false;           // IsSpellEffect() / GetHandler():IsSpell()
    let mut trap = false;            // IsTrapEffect() / GetHandler():IsTrap()
    for conjunct in normalized.split(" and ") {
        let c: String = conjunct.chars().filter(|ch| !ch.is_whitespace()).collect();
        match c.strip_prefix(eff)? {
            ":IsHasType(EFFECT_TYPE_ACTIVATE)" => card_activation = true,
            ":IsMonsterEffect()" => monster = true,
            ":IsSpellEffect()" | ":GetHandler():IsSpell()" => spell = true,
            ":IsTrapEffect()" | ":GetHandler():IsTrap()" => trap = true,
            _ => return None,
        }
    }
    match (card_activation, monster, spell, trap) {
        (true,  false, false, false) => Some("cannot_activate_spells_traps"),
        (false, true,  false, false) => Some("cannot_activate_monster_effects"),
        (true,  false, true,  false) => Some("cannot_activate_spells"),
        (true,  false, false, true)  => Some("cannot_activate_traps"),
        _ => None,
    }
}

/// Player-scoped damage-shaping chain (Phase 19, T37):
/// `EFFECT_TYPE_FIELD + SetTargetRange(<player flags>) + SetCode(<damage
/// code>) + standard reset + Duel.RegisterEffect(eN, tp)` →
/// `damage_rule <you|opponent|both_players> <keyword> <duration>`.
///
/// Inverts the T37 `DamageRule` doc table exactly:
///   - EFFECT_CHANGE_DAMAGE: `SetValue(0)` → `no_damage`; a
///     REASON_EFFECT→0 guard fn → `no_effect_damage`; a REASON_EFFECT→
///     val/2 guard fn → `halve_effect_damage`;
///   - EFFECT_AVOID_BATTLE_DAMAGE: no value or the literal flag `1` →
///     `no_battle_damage`;
///   - EFFECT_CHANGE_BATTLE_DAMAGE: `SetValue(HALF_DAMAGE)` →
///     `halve_battle_damage`, `SetValue(DOUBLE_DAMAGE)` →
///     `double_battle_damage`;
///   - EFFECT_REVERSE_DAMAGE: flag `1`/none → `reverse_damage`, a
///     boolean REASON_EFFECT guard → `reverse_effect_damage`;
///   - EFFECT_REFLECT_DAMAGE: the `(r&REASON_EFFECT)~=0 and rp==1-tp`
///     guard → `reflect_effect_damage`;
///   - EFFECT_REFLECT_BATTLE_DAMAGE: flag `1`/none →
///     `reflect_battle_damage`.
/// The no-value spelling is accepted wherever the doc table says flag
/// `1`: the engine treats an absent value as unconditionally-applies,
/// and the corpus majority shape for AVOID/REFLECT_BATTLE omits it.
///
/// Skip gates (None) — skip-not-mis-emit, all confirmed corpus
/// sub-shapes; the chain-shape gates mirror the Phase 18 restrict path:
///   - guard bodies outside `classify_damage_guard`'s closed set:
///     chain-id "that damage" shapes (Duel.GetCurrentChain /
///     CHAININFO_CHAIN_ID label matching), owner/responsible-player
///     qualifiers (`rp~=e:GetOwnerPlayer()`), flag/counter side
///     effects, unguarded halves (`return math.floor(val/2)` halves
///     ALL damage — no T37 keyword), doubles, race/attribute limits;
///   - arbitrary multipliers and fixed replacement values
///     (`SetValue(400)`, `GetLP()/2`, `aux.ChangeBattleDamage(...)`);
///   - card-scoped LOCATION masks in SetTargetRange (player flags
///     only), registration to any player expr other than the literal
///     `tp`, card-registered chains, group-applied chains, choice arms;
///   - SetCondition / SetOperation closures, SetTarget filters;
///   - non-standard resets: turn-qualified (RESET_SELF_TURN /
///     RESET_OPPO_TURN), reset counts above 1, missing resets.
fn translate_damage_rule_chain(
    code: &str,
    chain: &RegisterEffectChain,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<DslLine> {
    if !chain.duel_registered || chain.multi_target || chain.in_choice_arm {
        return None;
    }
    if chain.register_target.trim() != "tp" {
        return None;
    }
    if chain.condition.is_some() || chain.operation.is_some() {
        return None;
    }
    if chain.set_target.is_some() {
        return None; // affected-card filter — not a plain player rule
    }
    let dur = reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref())?;
    let reset = chain.reset.as_deref().unwrap_or("");
    if reset.contains("RESET_SELF_TURN") || reset.contains("RESET_OPPO_TURN") {
        return None; // turn-qualified phase reset ≠ end of CURRENT turn
    }
    if chain.reset_count.as_deref().is_some_and(|c| c.trim() != "1") {
        return None; // survives N reset events — two-turn lifetime
    }
    let scope = player_scope_from_range(chain.target_range.as_deref()?)?;
    let value = chain.value.as_deref().map(str::trim);
    let keyword = match code {
        "EFFECT_CHANGE_DAMAGE" => match value? {
            "0" => "no_damage",
            v => match classify_damage_guard(v, functions)? {
                DamageGuard::EffectToZero => "no_effect_damage",
                DamageGuard::EffectHalve => "halve_effect_damage",
                _ => return None,
            },
        },
        "EFFECT_AVOID_BATTLE_DAMAGE" => match value {
            None | Some("1") => "no_battle_damage",
            Some(_) => return None,
        },
        "EFFECT_CHANGE_BATTLE_DAMAGE" => match value? {
            "HALF_DAMAGE" => "halve_battle_damage",
            "DOUBLE_DAMAGE" => "double_battle_damage",
            _ => return None,
        },
        "EFFECT_REVERSE_DAMAGE" => match value {
            None | Some("1") => "reverse_damage",
            Some(v) => match classify_damage_guard(v, functions)? {
                DamageGuard::EffectFlag => "reverse_effect_damage",
                _ => return None,
            },
        },
        "EFFECT_REFLECT_DAMAGE" => match classify_damage_guard(value?, functions)? {
            DamageGuard::EffectFromOpponentFlag => "reflect_effect_damage",
            _ => return None,
        },
        "EFFECT_REFLECT_BATTLE_DAMAGE" => match value {
            None | Some("1") => "reflect_battle_damage",
            Some(_) => return None,
        },
        _ => return None,
    };
    Some(DslLine::Action(format!("damage_rule {} {} {}", scope, keyword, dur)))
}

/// The closed set of damage-shaping `SetValue` guard shapes Phase 19
/// classifies. Which shapes are legal for which code is decided by the
/// caller (an EffectFlag on EFFECT_REFLECT_DAMAGE is NOT
/// reflect_effect_damage — the doc-table shape needs the rp conjunct).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DamageGuard {
    /// Value guard: REASON_EFFECT damage → `0`, everything else passes.
    EffectToZero,
    /// Value guard: REASON_EFFECT damage → `val/2` (floored or not),
    /// everything else passes.
    EffectHalve,
    /// Boolean guard: `(r&REASON_EFFECT)~=0` alone.
    EffectFlag,
    /// Boolean guard: `(r&REASON_EFFECT)~=0 and rp==1-tp` — effect
    /// damage the OPPONENT inflicts.
    EffectFromOpponentFlag,
}

/// Classify a damage-shaping `SetValue` argument into a `DamageGuard`.
/// Accepted carriers:
///   - inline `function(<params>) return <expr> end` closures — the
///     value/reason/responsible-player params are positional 3/4/5 by
///     the `(e,re,val,r,rp,rc)` corpus convention;
///   - named `s.*` value fns resolved through the walk's function
///     table, as either a single `return <expr>` body or the Phase 19
///     guarded-return idiom. Param names aren't recorded, so the
///     conventional spellings are tried: reason `r`, value `val` then
///     `dam` (guard and pass-through occurrences must match uniformly,
///     so a mis-bound name can't slip through).
/// Multi-statement bodies (the chain-id CHAININFO shapes, flag/counter
/// side effects) have neither `return_expr` nor `guarded_return` and
/// drop out here.
fn classify_damage_guard(
    value: &str,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<DamageGuard> {
    if let Some((params, expr)) = damage_guard_closure_body(value) {
        let val = params.get(2).map(String::as_str).unwrap_or("");
        let r = params.get(3).map(String::as_str).unwrap_or("");
        let rp = params.get(4).map(String::as_str).unwrap_or("");
        return classify_damage_guard_expr(&expr, val, r, rp);
    }
    let body = functions.get(value)?;
    let classify = |val: &str| {
        if let Some(expr) = body.return_expr.as_deref() {
            return classify_damage_guard_expr(expr, val, "r", "rp");
        }
        let g = body.guarded_return.as_ref()?;
        classify_guard_triple(&g.cond, &g.then_expr, &g.else_expr, val, "r")
    };
    classify("val").or_else(|| classify("dam"))
}

/// Extract the (params, return-expression) of an inline
/// `function(<params>) return <expr> end` damage-value closure.
/// Multi-statement closure bodies return None.
fn damage_guard_closure_body(raw: &str) -> Option<(Vec<String>, String)> {
    let r = raw.strip_prefix("function")?.trim_start();
    let r = r.strip_prefix('(')?;
    let (params, r) = r.split_once(')')?;
    let params: Vec<String> = params.split(',').map(|p| p.trim().to_string()).collect();
    let r = r.trim_start().strip_prefix("return")?;
    let expr = r.trim().strip_suffix("end")?.trim().to_string();
    if expr.is_empty() { return None; }
    Some((params, expr))
}

/// Classify a single-expression damage guard. Two families:
///   - the ternary value guard `<cond> and <A> or <B>` (lua's `0` is
///     truthy, so `r&REASON_EFFECT~=0 and 0 or val` is sound) —
///     delegated to `classify_guard_triple`;
///   - a boolean guard: a conjunction of the effect-reason test and
///     (optionally) the responsible-player-is-opponent test. Any other
///     conjunct (`or`, attack-position checks, IsHasType predicates, …)
///     fails the whole guard (skip-not-mis-emit).
fn classify_damage_guard_expr(expr: &str, val: &str, r: &str, rp: &str) -> Option<DamageGuard> {
    let normalized = expr.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((left, b)) = normalized.rsplit_once(" or ") {
        let (cond, a) = left.split_once(" and ")?;
        return classify_guard_triple(cond, a, b, val, r);
    }
    let mut effect = false;
    let mut from_opponent = false;
    for conjunct in normalized.split(" and ") {
        let c = damage_guard_canon(conjunct);
        if is_effect_reason_test(&c, r) {
            effect = true;
        } else if !rp.is_empty() && (c == format!("{}==1-tp", rp)
            || c == format!("{}==1-e:GetHandlerPlayer", rp))
        {
            from_opponent = true;
        } else {
            return None;
        }
    }
    match (effect, from_opponent) {
        (true, false) => Some(DamageGuard::EffectFlag),
        (true, true) => Some(DamageGuard::EffectFromOpponentFlag),
        _ => None,
    }
}

/// Classify a (cond, then, else) value-guard triple. The guarded branch
/// is the one selected by the effect-reason test (either polarity); the
/// pass-through branch must be exactly the value param. Guarded branch
/// `0` → EffectToZero; a halving of the value param → EffectHalve.
fn classify_guard_triple(
    cond: &str,
    then_expr: &str,
    else_expr: &str,
    val: &str,
    r: &str,
) -> Option<DamageGuard> {
    let cond = damage_guard_canon(cond);
    let (guarded, pass) = if is_effect_reason_test(&cond, r) {
        (then_expr, else_expr)
    } else if cond == format!("{}&REASON_EFFECT==0", r) {
        (else_expr, then_expr)
    } else {
        return None;
    };
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let (guarded, pass) = (strip(guarded), strip(pass));
    if pass != val {
        return None;
    }
    if guarded == "0" {
        return Some(DamageGuard::EffectToZero);
    }
    let halves = [
        format!("{}/2", val),
        format!("{}//2", val),
        format!("math.floor({}/2)", val),
        format!("math.floor({}//2)", val),
    ];
    if halves.contains(&guarded) {
        return Some(DamageGuard::EffectHalve);
    }
    None
}

/// True when a canonicalized conjunct is the effect-reason band test:
/// `r&REASON_EFFECT` compared `~=0` / `>0` / `==REASON_EFFECT`.
fn is_effect_reason_test(canon: &str, r: &str) -> bool {
    canon == format!("{}&REASON_EFFECT~=0", r)
        || canon == format!("{}&REASON_EFFECT>0", r)
        || canon == format!("{}&REASON_EFFECT==REASON_EFFECT", r)
}

/// Canonicalize a guard conjunct: drop whitespace and parentheses.
/// Sound here because every accepted atom is call-free after the drop —
/// `(r&REASON_EFFECT)~=0` and `rp==1-e:GetHandlerPlayer()` both
/// canonicalize to fixed strings, while any expr whose parens carried
/// call arguments can no longer equal an accepted atom.
fn damage_guard_canon(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '(' && *c != ')').collect()
}

/// Decode a FIELD chain's `SetTargetRange` into a full DSL group
/// selector `(all, <kind>, <controller>, <zone>[, where …])`.
///
/// Both sides must be `0` or a single known LOCATION_* token (player
/// flags `1`, OR'd masks, and unknown constants return None); when both
/// sides are set they must name the SAME location so one `from <zone>`
/// clause covers them.
fn field_range_selector(
    range: &str,
    kind: &str,
    where_clause: Option<&str>,
) -> Option<String> {
    let cleaned: String = range.chars().filter(|c| !c.is_whitespace()).collect();
    let (my, opp) = cleaned.split_once(',')?;
    let controller = match (my != "0", opp != "0") {
        (true,  true)  => {
            if my != opp { return None; }
            "either controls"
        }
        (true,  false) => "you control",
        (false, true)  => "opponent controls",
        (false, false) => return None,
    };
    // zone_from_locations rejects player flags / OR'd masks / unknown
    // constants by returning None.
    let zone = zone_from_locations(my, opp)?;
    let mut parts = vec!["all".to_string(), kind.to_string(), controller.to_string(), zone];
    if let Some(w) = where_clause {
        parts.push(format!("where {}", w));
    }
    Some(format!("({})", parts.join(", ")))
}

/// Map a FIELD chain's `SetTarget` filter arg to a selector
/// (kind, where-clause) pair. Accepted forms:
///   - stock names map_group_filter knows (`aux.TRUE`, `Card.IsFaceup`,
///     `Card.IsMonster` / `IsSpell` / `IsTrap`);
///   - `aux.TargetBoolFunction(<Card.Is*>, <const>)` partial application;
///   - inline `(e,c)` closures with a conjunction of mappable predicates;
///   - named `s.*` filters resolved through the walk's function table
///     whose body is a single `return <conjunction>`.
fn field_filter_to_kind_where(
    filter: &str,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<(String, Option<String>)> {
    let filter = filter.trim();
    if let Some((kind, w)) = map_group_filter(filter) {
        return Some((kind.to_string(), w));
    }
    if let Some(inner) = filter
        .strip_prefix("aux.TargetBoolFunction(")
        .and_then(|r| r.strip_suffix(')'))
    {
        let (method, arg) = inner.split_once(',')?;
        let w = filter_predicate_to_where(method.trim(), arg.trim())?;
        return Some(("card".to_string(), Some(w)));
    }
    if let Some(expr) = field_filter_closure_body(filter) {
        return field_filter_expr_to_kind_where(&expr);
    }
    // Named file-local filter — only single-`return` bodies carry a
    // return_expr; multi-statement filters drop out here.
    let body = functions.get(filter)?;
    let expr = body.return_expr.as_deref()?;
    field_filter_expr_to_kind_where(expr)
}

/// Extract the return expression from an inline two-param
/// `function(e,c) return <expr> end` FIELD-target closure. The affected
/// card is the SECOND param and must be literally `c` (the conjunct
/// classifier matches `c:` receivers).
fn field_filter_closure_body(raw: &str) -> Option<String> {
    let r = raw.strip_prefix("function")?.trim_start();
    let r = r.strip_prefix('(')?;
    let (params, r) = r.split_once(')')?;
    let params: Vec<&str> = params.split(',').map(str::trim).collect();
    if params.len() != 2 || params[1] != "c" {
        return None;
    }
    let r = r.trim_start().strip_prefix("return")?;
    Some(r.trim().strip_suffix("end")?.trim().to_string())
}

/// Classify an affected-card filter expression (conjunction of `c:Is*`
/// predicates) into a selector (kind, where-clause). Any conjunct
/// outside the table — runtime args, `or`-groups, except-target
/// `e:GetLabel()` comparisons, position predicates — fails the whole
/// filter (skip-not-mis-emit).
fn field_filter_expr_to_kind_where(expr: &str) -> Option<(String, Option<String>)> {
    let normalized = expr.split_whitespace().collect::<Vec<_>>().join(" ");
    // Top-level disjunctions are not classified — `(A) or (B)` groups
    // would need parenthesised where-clause emit and audit showed every
    // instance also contains unmappable atoms.
    if normalized.contains(" or ") {
        return None;
    }
    let mut kind: Option<&'static str> = None;
    let mut wheres: Vec<String> = Vec::new();
    for conjunct in normalized.split(" and ") {
        let c: String = conjunct.chars().filter(|ch| !ch.is_whitespace()).collect();
        let body = c.strip_prefix("c:")?;
        let (method, rest) = body.split_once('(')?;
        let arg = rest.strip_suffix(')')?;
        let mut set_kind = |k: &'static str| -> Option<()> {
            match kind {
                None => { kind = Some(k); Some(()) }
                Some(existing) if existing == k => Some(()),
                Some(_) => None, // conflicting kinds — unclassifiable
            }
        };
        match (method, arg) {
            ("IsFaceup",   "") => wheres.push("is_face_up".to_string()),
            ("IsFacedown", "") => wheres.push("is_face_down".to_string()),
            ("IsMonster",  "") => set_kind("monster")?,
            ("IsSpell",    "") => set_kind("spell")?,
            ("IsTrap",     "") => set_kind("trap")?,
            ("IsSetCard",   a) => wheres.push(filter_predicate_to_where("Card.IsSetCard", a)?),
            ("IsAttribute", a) => wheres.push(filter_predicate_to_where("Card.IsAttribute", a)?),
            ("IsRace",      a) => wheres.push(filter_predicate_to_where("Card.IsRace", a)?),
            ("IsType",      a) => wheres.push(type_mask_to_where(a)?),
            ("IsLevelBelow", a) => wheres.push(format!("level <= {}", a.parse::<u32>().ok()?)),
            ("IsLevelAbove", a) => wheres.push(format!("level >= {}", a.parse::<u32>().ok()?)),
            ("IsRankBelow",  a) => wheres.push(format!("rank <= {}", a.parse::<u32>().ok()?)),
            ("IsRankAbove",  a) => wheres.push(format!("rank >= {}", a.parse::<u32>().ok()?)),
            _ => return None,
        }
    }
    let where_clause = if wheres.is_empty() { None } else { Some(wheres.join(" and ")) };
    Some((kind.unwrap_or("card").to_string(), where_clause))
}

/// Map a `c:IsType(TYPE_…)` mask to where-clause atoms. `+`/`|`-joined
/// masks are any-of in lua → `or`-joined atoms. Only the frame types
/// with a DSL `is_*` predicate map; spell/trap subtype masks
/// (TYPE_CONTINUOUS / TYPE_FIELD / TYPE_EQUIP …) return None.
fn type_mask_to_where(mask: &str) -> Option<String> {
    let mut parts = Vec::new();
    for t in mask.split(['+', '|']).map(str::trim) {
        parts.push(match t {
            "TYPE_NORMAL"   => "is_normal",
            "TYPE_EFFECT"   => "is_effect",
            "TYPE_FUSION"   => "is_fusion",
            "TYPE_SYNCHRO"  => "is_synchro",
            "TYPE_XYZ"      => "is_xyz",
            "TYPE_LINK"     => "is_link",
            "TYPE_RITUAL"   => "is_ritual",
            "TYPE_PENDULUM" => "is_pendulum",
            "TYPE_TUNER"    => "is_tuner",
            "TYPE_FLIP"     => "is_flip",
            "TYPE_TOKEN"    => "is_token",
            _ => return None,
        }.to_string());
    }
    if parts.is_empty() { return None; }
    Some(parts.join(" or "))
}

/// EFFECT_CHANGE_ATTRIBUTE / EFFECT_CHANGE_RACE chain →
/// `change_attribute <selector> to <ATTR>` / `change_race <selector> to <Race>`.
///
/// Phase 11. Only literal single-constant `SetValue` args translate —
/// `e:GetLabel()` (runtime-chosen), local vars from `Duel.Announce*`,
/// and method-call values skip. The DSL change_attribute / change_race
/// grammar carries no duration clause, so the lua chain's end-of-turn
/// reset cannot be expressed; the property change is emitted without a
/// bound (known lossy — documented in the Phase 11 report). Resets that
/// don't map to a known duration keyword skip entirely so unaudited
/// lifetime shapes never emit.
fn translate_change_property_chain(
    code: &str,
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref())?;
    let raw = chain.value.as_deref()?.trim();
    let (action, token) = match code {
        "EFFECT_CHANGE_ATTRIBUTE" => ("change_attribute", attribute_token(raw)?),
        "EFFECT_CHANGE_RACE"      => ("change_race", race_token(raw)?),
        _ => return None,
    };
    let selector = resolve_chain_selector(chain, body)?;
    Some(DslLine::Action(format!("{} {} to {}", action, selector, token)))
}

/// Map a literal `ATTRIBUTE_*` constant to the DSL attribute token.
/// OR'd multi-attribute values return None (grammar takes one token).
fn attribute_token(value: &str) -> Option<&'static str> {
    Some(match value {
        "ATTRIBUTE_LIGHT"  => "LIGHT",
        "ATTRIBUTE_DARK"   => "DARK",
        "ATTRIBUTE_FIRE"   => "FIRE",
        "ATTRIBUTE_WATER"  => "WATER",
        "ATTRIBUTE_EARTH"  => "EARTH",
        "ATTRIBUTE_WIND"   => "WIND",
        "ATTRIBUTE_DIVINE" => "DIVINE",
        _ => return None,
    })
}

/// Map a literal `RACE_*` constant to the DSL race token (grammar
/// spelling, e.g. RACE_WINGEDBEAST → "Winged Beast").
fn race_token(value: &str) -> Option<&'static str> {
    Some(match value {
        "RACE_WARRIOR"      => "Warrior",
        "RACE_SPELLCASTER"  => "Spellcaster",
        "RACE_FAIRY"        => "Fairy",
        "RACE_FIEND"        => "Fiend",
        "RACE_ZOMBIE"       => "Zombie",
        "RACE_MACHINE"      => "Machine",
        "RACE_AQUA"         => "Aqua",
        "RACE_PYRO"         => "Pyro",
        "RACE_ROCK"         => "Rock",
        "RACE_WINGEDBEAST"  => "Winged Beast",
        "RACE_PLANT"        => "Plant",
        "RACE_INSECT"       => "Insect",
        "RACE_THUNDER"      => "Thunder",
        "RACE_DRAGON"       => "Dragon",
        "RACE_BEAST"        => "Beast",
        "RACE_BEASTWARRIOR" => "Beast-Warrior",
        "RACE_DINOSAUR"     => "Dinosaur",
        "RACE_FISH"         => "Fish",
        "RACE_SEASERPENT"   => "Sea Serpent",
        "RACE_REPTILE"      => "Reptile",
        "RACE_PSYCHIC"      => "Psychic",
        "RACE_DIVINE"       => "Divine-Beast",
        "RACE_WYRM"         => "Wyrm",
        "RACE_CYBERSE"      => "Cyberse",
        "RACE_ILLUSION"     => "Illusion",
        _ => return None,
    })
}

/// EFFECT_CHANGE_CODE chain → `change_name <selector> to "<name>" [until <dur>]`.
///
/// Phase 11. The lua SetValue is a passcode; the DSL atom takes the card
/// NAME, so translation needs the BabelCdb-backed table registered via
/// [`register_card_names`]. Accepts literal integer passcodes plus the
/// audited `CARD_*` named constants. Skips: unresolvable ids (no table /
/// unknown passcode), names containing a double quote (unrepresentable
/// in the DSL string literal), and non-literal values (`e:GetLabel()`,
/// `tc:GetOriginalCode()` locals — DSL has no "name of target" form).
fn translate_change_code_chain(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    let raw = chain.value.as_deref()?.trim();
    let id: u32 = raw.parse().ok().or_else(|| card_constant_id(raw))?;
    let name = lookup_card_name(id)?;
    if name.contains('"') { return None; }
    let selector = resolve_chain_selector(chain, body)?;
    let mut line = format!("change_name {} to \"{}\"", selector, name);
    match reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref()) {
        Some(dur) => line.push_str(&format!(" until {}", dur)),
        // A reset the mapping can't decode is an unaudited lifetime —
        // emitting without a duration would silently mean "permanently".
        None if chain.reset.is_some() => return None,
        None => {}
    }
    Some(DslLine::Action(line))
}

/// Named `CARD_*` passcode constants (CardScripts card_counter_constants.lua)
/// seen in audited EFFECT_CHANGE_CODE chains.
fn card_constant_id(name: &str) -> Option<u32> {
    Some(match name {
        "CARD_CYBER_DRAGON" => 70095154,
        _ => return None,
    })
}

/// EFFECT_IMMUNE_EFFECT chain → `grant <selector> unaffected_by <src> until <dur>`.
///
/// Phase 11. The lua SetValue is a filter predicate `(e, te|re) → bool`
/// deciding which effects the card ignores. Only *stock* single-return
/// filters translate — the function-ref is resolved through the walk's
/// function table and its return expression classified by
/// [`immune_filter_sources`]. Inline closures and multi-statement filter
/// bodies skip (no return_expr). May emit two grant lines for the
/// spell+trap immunity stock filter.
fn translate_immune_chain(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
    functions: &BTreeMap<String, FunctionBody>,
) -> Vec<DslLine> {
    let Some(dur) = reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref()) else { return Vec::new() };
    let Some(selector) = resolve_chain_selector(chain, body) else { return Vec::new() };
    let Some(value) = chain.value.as_deref() else { return Vec::new() };
    let Some(filter_body) = functions.get(value.trim()) else { return Vec::new() };
    let Some(expr) = filter_body.return_expr.as_deref() else { return Vec::new() };
    let Some(sources) = immune_filter_sources(expr) else { return Vec::new() };
    sources
        .into_iter()
        .map(|src| DslLine::Action(format!(
            "grant {} unaffected_by {} until {}",
            selector, src, dur,
        )))
        .collect()
}

/// Classify a stock immune-filter return expression into DSL
/// `unaffected_by` source tokens. Returns None when any conjunct falls
/// outside the recognized trivial forms (skip-not-mis-emit).
///
/// Recognized conjuncts (te/re = the incoming effect, e = the immunity):
///   - other-card: `te:GetOwner()~=e:GetOwner()` and handler/order
///     variants. Drops the "other cards'" qualifier — corpus precedent:
///     the passive path already maps this filter alone to
///     `unaffected_by effects` (the card's own effects on itself are
///     the only loss).
///   - opponent: `GetOwnerPlayer()`-inequality variants → effects owned
///     by the opponent.
///   - effect-kind: `IsMonsterEffect` / `IsSpellEffect` / `IsTrapEffect`
///     / `IsSpellTrapEffect` / `IsActiveType(TYPE_EFFECT)` (normal
///     monsters have no effects, so effect-monster ≡ monster here).
///
/// Combination table:
///   - kind only (± other-card) → that kind's token(s)
///   - opponent (± other-card)  → `opponent_effects`
///   - opponent + kind          → None (grammar can't scope a kind to one
///     player; either over-grant would be a mis-emit)
fn immune_filter_sources(expr: &str) -> Option<Vec<&'static str>> {
    let normalized = expr.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut kinds: Vec<&'static str> = Vec::new();
    let mut opponent = false;
    for conjunct in normalized.split(" and ") {
        let c: String = conjunct.chars().filter(|ch| !ch.is_whitespace()).collect();
        match c.as_str() {
            // other-card qualifier — droppable (see doc comment).
            "te:GetOwner()~=e:GetOwner()" | "re:GetOwner()~=e:GetOwner()"
            | "e:GetOwner()~=te:GetOwner()" | "e:GetOwner()~=re:GetOwner()"
            | "te:GetOwner()~=e:GetHandler()" | "re:GetOwner()~=e:GetHandler()"
            | "e:GetHandler()~=te:GetOwner()" | "e:GetHandler()~=re:GetOwner()" => {}
            // opponent-owned qualifier.
            "te:GetOwnerPlayer()~=e:GetHandlerPlayer()"
            | "re:GetOwnerPlayer()~=e:GetHandlerPlayer()"
            | "te:GetOwnerPlayer()~=e:GetOwnerPlayer()"
            | "re:GetOwnerPlayer()~=e:GetOwnerPlayer()"
            | "e:GetOwnerPlayer()~=te:GetOwnerPlayer()"
            | "e:GetOwnerPlayer()~=re:GetOwnerPlayer()"
            | "e:GetHandlerPlayer()~=te:GetOwnerPlayer()"
            | "e:GetHandlerPlayer()~=re:GetOwnerPlayer()"
            | "e:GetOwnerPlayer()==1-te:GetOwnerPlayer()"
            | "e:GetOwnerPlayer()==1-re:GetOwnerPlayer()"
            | "te:GetOwnerPlayer()==1-e:GetOwnerPlayer()"
            | "re:GetOwnerPlayer()==1-e:GetOwnerPlayer()" => opponent = true,
            // effect-kind qualifiers.
            "te:IsMonsterEffect()" | "re:IsMonsterEffect()"
            | "te:IsActiveType(TYPE_EFFECT)" | "re:IsActiveType(TYPE_EFFECT)" =>
                kinds.push("monsters"),
            "te:IsSpellEffect()" | "re:IsSpellEffect()" => kinds.push("spells"),
            "te:IsTrapEffect()" | "re:IsTrapEffect()" => kinds.push("traps"),
            "te:IsSpellTrapEffect()" | "re:IsSpellTrapEffect()" => {
                kinds.push("spells");
                kinds.push("traps");
            }
            _ => return None,
        }
    }
    kinds.dedup();
    match (kinds.is_empty(), opponent) {
        (true,  true)  => Some(vec!["opponent_effects"]),
        (true,  false) => Some(vec!["effects"]),
        (false, false) => Some(kinds),
        (false, true)  => None, // player-scoped kind — inexpressible
    }
}

/// EFFECT_EXTRA_ATTACK is value-dependent: SetValue(1) → double_attack,
/// SetValue(2) → triple_attack. Other values (variable refs / dynamic
/// expressions) are skipped — the DSL has no `extra_attack <n>` form, so
/// emitting double_attack would mis-translate.
fn translate_extra_attack_chain(
    chain: &RegisterEffectChain,
    body: &FunctionBody,
) -> Option<DslLine> {
    let dur = reset_to_duration_kw(chain.reset.as_deref(), chain.reset_count.as_deref())?;
    let value: i64 = chain.value.as_deref()?.trim().parse().ok()?;
    let ability = match value {
        1 => "double_attack",
        2 => "triple_attack",
        _ => return None,
    };
    let selector = resolve_chain_selector(chain, body)?;
    Some(DslLine::Action(format!(
        "grant {} {} until {}",
        selector, ability, dur,
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

    // Unary minus — recurse and flip sign. Only when the remainder is a
    // single atom: `-a + b` must NOT parse as -(a + b), so anything with
    // a top-level binary operator falls through to the binop splitter.
    if let Some(rest) = arg.strip_prefix('-') {
        let rest = rest.trim();
        if !rest.is_empty() && !rest.starts_with('-')
            && split_top_level_binop(rest).is_none()
        {
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

    // Group-count calls → DSL `count(<selector>)` (Phase 10).
    if let Some(expr) = count_call_to_count_expr(arg) {
        return Some(ParsedValue { expr, negative: false });
    }

    // Single-step binary math: `<lhs> <op> <rhs>` split at the last
    // top-level operator (paren-depth aware — `1-tp` inside a count call's
    // argument list is not a split point). Both operands must recursively
    // resolve to op-free atoms; multi-op expressions are skipped because
    // the DSL's flat expr chain would not preserve Lua's precedence.
    if let Some((lhs, op, rhs)) = split_top_level_binop(arg) {
        let l = parse_lua_value(lhs, bindings)?;
        let r = parse_lua_value(rhs, bindings)?;
        if expr_has_op(&l.expr) || expr_has_op(&r.expr) { return None; }
        return match op {
            // `x * 1` / `1 * x` — common lua sign-flip idiom
            // (`tc:GetDefense()*-1`); elide the redundant factor.
            '*' if r.expr == "1" => Some(ParsedValue {
                expr: l.expr,
                negative: l.negative != r.negative,
            }),
            '*' if l.expr == "1" => Some(ParsedValue {
                expr: r.expr,
                negative: l.negative != r.negative,
            }),
            '*' | '/' => Some(ParsedValue {
                expr: format!("{} {} {}", l.expr, op, r.expr),
                negative: l.negative != r.negative,
            }),
            // Additive ops: a negative operand would need re-bracketing the
            // DSL can't express (`a + -b`), so only plain operands combine.
            '+' | '-' if !l.negative && !r.negative => Some(ParsedValue {
                expr: format!("{} {} {}", l.expr, op, r.expr),
                negative: false,
            }),
            _ => None,
        };
    }

    None
}

/// True when a rendered DSL expr already contains a binary operator —
/// used to reject nested math the flat expr grammar would mis-associate.
fn expr_has_op(expr: &str) -> bool {
    [" + ", " - ", " * ", " / "].iter().any(|op| expr.contains(op))
}

/// Split a Lua expression at its last top-level binary operator, honoring
/// precedence (`+`/`-` before `*`/`/` so the split lands at the loosest
/// binding point). Paren-depth aware; a `-` directly after another
/// operator, an opening paren, or at position 0 is unary, not binary.
fn split_top_level_binop(arg: &str) -> Option<(&str, char, &str)> {
    let bytes = arg.as_bytes();
    let mut depth = 0i32;
    let mut last_addsub: Option<usize> = None;
    let mut last_muldiv: Option<usize> = None;
    let mut prev_meaningful: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'*' | b'/' if depth == 0 => last_muldiv = Some(i),
            b'+' if depth == 0 => last_addsub = Some(i),
            b'-' if depth == 0 => {
                // Unary when at start or right after an operator / `(`.
                let unary = matches!(prev_meaningful, None | Some(b'+') | Some(b'-') | Some(b'*') | Some(b'/') | Some(b'('));
                if !unary { last_addsub = Some(i); }
            }
            _ => {}
        }
        if !b.is_ascii_whitespace() { prev_meaningful = Some(b); }
    }
    let pos = last_addsub.or(last_muldiv)?;
    let lhs = arg[..pos].trim();
    let rhs = arg[pos + 1..].trim();
    if lhs.is_empty() || rhs.is_empty() { return None; }
    Some((lhs, arg.as_bytes()[pos] as char, rhs))
}

/// Lower a `Duel.GetMatchingGroupCount(filter, p, my, opp, exc, …)` or
/// `Duel.GetFieldGroupCount(p, my, opp)` call text to DSL
/// `count((all, <kind>, <controller>, from <zone>[, where …]))`.
///
/// Skip-not-mis-emit gates (Phase 10): the count's numeric value IS the
/// semantics, so unlike the action-selector path the filter must map to
/// a selector the DSL can express — custom `s.filter` predicates, card-
/// code filters, and `aux.FaceupFilter(…)` compositions all return None.
/// The scope player must be `tp` (or `1-tp`, which flips the controller),
/// and the locations must collapse to a single DSL zone.
fn count_call_to_count_expr(arg: &str) -> Option<String> {
    let (is_field, inner) =
        if let Some(rest) = arg.strip_prefix("Duel.GetMatchingGroupCount(") {
            (false, rest)
        } else if let Some(rest) = arg.strip_prefix("Duel.GetFieldGroupCount(") {
            (true, rest)
        } else {
            return None;
        };
    let inner = inner.strip_suffix(')')?;
    let args = split_top_level_commas(inner)?;
    let (kind, where_clause, scope_p, my, opp) = if is_field {
        // GetFieldGroupCount(player, my_locs, opp_locs) — no filter.
        if args.len() != 3 { return None; }
        ("card", None, args[0].as_str(), args[1].as_str(), args[2].as_str())
    } else {
        // GetMatchingGroupCount(filter, player, my_locs, opp_locs, exception, …)
        if args.len() < 5 { return None; }
        let (kind, wc) = map_group_filter(args[0].as_str())?;
        (kind, wc, args[1].as_str(), args[2].as_str(), args[3].as_str())
    };
    // Locations are relative to the scope player; `1-tp` flips ownership.
    let controller = match scope_p {
        "tp"   => controller_from_scope(scope_p, my, opp)?,
        "1-tp" => controller_from_scope(scope_p, opp, my)?,
        _ => return None,
    };
    let zone = zone_from_locations(my, opp)?;
    let spec = SelectorSpec {
        quantity: "all".to_string(),
        kind: kind.to_string(),
        controller: Some(controller),
        zone: Some(zone),
        where_clause,
        filter_mapped: true,
        from_resolve_select: false,
        raw_filter: None,
    };
    Some(format!("count({})", spec.to_dsl()))
}

/// Split a call argument list at top-level commas. Returns None on
/// unbalanced parens (truncated text) so callers skip instead of
/// mis-reading a partial argument.
fn split_top_level_commas(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => {
                depth -= 1;
                if depth < 0 { return None; }
            }
            b',' if depth == 0 => {
                out.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 { return None; }
    out.push(s[start..].trim().to_string());
    Some(out)
}

/// `tc:GetAttack()` / `c:GetLevel()` → `target.atk` / `self.level`.
/// GetBaseAttack/GetBaseDefense are the pre-modification stats — a
/// distinct DSL token (`base_atk`/`base_def`), not an alias for the
/// current value: `set_atk target target.atk * 2` drifts whenever the
/// target's ATK is already modified.
fn method_call_to_stat(arg: &str) -> Option<String> {
    let (recv, method) = match arg {
        s if s.starts_with("tc:") => ("target", &s[3..]),
        s if s.starts_with("c:") => ("self", &s[2..]),
        _ => return None,
    };
    let stat = match method {
        "GetAttack()"      => "atk",
        "GetDefense()"     => "def",
        "GetBaseAttack()"  => "base_atk",
        "GetBaseDefense()" => "base_def",
        "GetLevel()"  => "level",
        "GetRank()"   => "rank",
        // T38 S1 — the grammar's overlay_count_ref accepts ONLY the
        // `self.` receiver (T34 shape); a `target.overlay_count` would
        // fail the corpus check, so `tc:GetOverlayCount()` skips.
        "GetOverlayCount()" if recv == "self" => "overlay_count",
        _ => return None,
    };
    Some(format!("{}.{}", recv, stat))
}

/// Map a `SetReset(reset, count)` pair to a DSL `duration` keyword by
/// exact-token decomposition of the reset expression:
///   - `RESET_PHASE|PHASE_END` (incl. the RESETS_STANDARD[_DISABLE]_PHASE_END
///     macros) → `end_of_turn`
///   - `RESET_PHASE|PHASE_DAMAGE` / `PHASE_DAMAGE_CAL` → `end_of_damage_step`
///   - `RESET_PHASE|PHASE_STANDBY|RESET_SELF_TURN` → `next_standby_phase`
///   - event-only `RESETS_STANDARD` / `RESETS_STANDARD_DISABLE` (no
///     RESET_PHASE component) → `while_face_up`: the effect survives turn
///     ends and dies when the card leaves the field / flips face-down.
///     (`while_face_up` under-resets on RESET_CONTROL / RESET_TURN_SET
///     nuances the DSL can't split, but is the closest keyword; the old
///     substring check mapped these to `end_of_turn`, cutting
///     permanent-while-face-up buffs at the first End Phase.)
///
/// Returns None for everything else (skip-not-mis-emit): turn-qualified
/// PHASE_END resets ("end of your/opponent's turn"), reset counts above 1
/// (`RESET_PHASE|PHASE_END, 2` = end of NEXT turn), bit-arithmetic exprs
/// (`RESETS_STANDARD&~RESET_TOFIELD`), PHASE_BATTLE lifetimes, chain-scoped
/// resets, and any token outside the audited set.
pub fn reset_to_duration_kw(reset: Option<&str>, count: Option<&str>) -> Option<&'static str> {
    let s = reset?;
    // A count above 1 multiplies the phase reset — no DSL keyword for
    // "the Nth end/standby phase". Non-literal counts (`ct`) also skip.
    if count.is_some_and(|c| c.trim() != "1") {
        return None;
    }
    let mut phase_end = false;
    let mut phase_standby = false;
    let mut phase_damage = false;
    let mut self_turn = false;
    let mut oppo_turn = false;
    let mut standard_event = false;
    let mut n_phases = 0usize;
    for tok in s.split(['|', '+']) {
        match tok.trim() {
            // CardScripts convenience macros (constant.lua) fold the
            // event family and the end-phase reset into one token.
            "RESETS_STANDARD_PHASE_END" | "RESETS_STANDARD_DISABLE_PHASE_END" => {
                standard_event = true;
                phase_end = true;
                n_phases += 1;
            }
            "RESETS_STANDARD" | "RESETS_STANDARD_DISABLE" => standard_event = true,
            // Flag markers that scope the other tokens, no lifetime of
            // their own. RESET_DISABLE rides RESETS_STANDARD shapes.
            "RESET_EVENT" | "RESET_PHASE" | "RESET_DISABLE" => {}
            "RESET_SELF_TURN" => self_turn = true,
            "RESET_OPPO_TURN" => oppo_turn = true,
            "PHASE_END" => { phase_end = true; n_phases += 1; }
            "PHASE_STANDBY" => { phase_standby = true; n_phases += 1; }
            "PHASE_DAMAGE" | "PHASE_DAMAGE_CAL" => { phase_damage = true; n_phases += 1; }
            // Narrowed event families (RESETS_STANDARD_EXC_GRAVE, spelled
            // out RESET_TOGRAVE|… lists, RESET_CHAIN, PHASE_BATTLE, …):
            // unaudited lifetimes — skip rather than guess.
            _ => return None,
        }
    }
    if n_phases > 1 {
        return None; // compound phase reset — fires at whichever comes first
    }
    match (phase_end, phase_standby, phase_damage, self_turn, oppo_turn) {
        (true, false, false, false, false) => Some("end_of_turn"),
        (false, true, false, true, false) => Some("next_standby_phase"),
        (false, false, true, false, false) => Some("end_of_damage_step"),
        (false, false, false, false, false) if standard_event => Some("while_face_up"),
        _ => None,
    }
}


/// Map a single `Duel.X` call to a DSL line (or None for skip-class
/// metadata). `bindings` maps local-variable names to their captured
/// SelectorSpec so referenced groups can become real selectors.
/// Phase 17 — translate one `s.<helper>(…)` call through a recognised
/// select-then-equip helper. The lua-side helper runs
/// `Duel.Equip(tp, <target>, <host>)` — the SELECTED card is equipped
/// onto the host — so the DSL form is `equip <selection> to self`.
///
/// Skips (None): unknown helper, host arg not the card itself, target
/// arg without one-card select provenance, or a selection consumed by
/// more than one site.
fn translate_equip_helper_call(
    name: &str,
    args: &[String],
    body: &FunctionBody,
    functions: &BTreeMap<String, FunctionBody>,
) -> Option<DslLine> {
    let spec = functions.get(name)?.equip_helper.as_ref()?;
    let host = args.get(spec.host_param)?.trim();
    if host != "c" && host != "e:GetHandler()" { return None; }
    let target = args.get(spec.target_param)?.trim();
    let sel = body.group_bindings.get(target)?;
    if !sel.from_resolve_select || sel.quantity != "1" { return None; }
    if selection_consumers(body, target) != 1 { return None; }
    Some(DslLine::Action(format!("equip {} to self", sel.to_dsl())))
}

fn translate_call(c: &DuelCall, body: &FunctionBody) -> Option<DslLine> {
    let m = c.method.as_str();
    let a = &c.args;
    let bindings = &body.group_bindings;
    match m {
        // ── Skip: pure UI / control-flow / metadata ──────────
        "Duel.Hint" | "Duel.HintSelection" | "Duel.ConfirmCards"
        | "Duel.BreakEffect" | "Duel.SetOperationInfo"
        | "Duel.SetPossibleOperationInfo" | "Duel.RegisterEffect"
        | "Duel.SetTargetPlayer" | "Duel.SetTargetParam"
        | "Duel.SetTargetCard" | "Duel.SetChainLimit"
        // Flag-effect helpers — bookkeeping only, no semantic DSL action.
        | "Duel.RegisterFlagEffect" | "Duel.HasFlagEffect" | "Duel.GetFlagEffect"
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
        "Duel.Destroy" => Some(gated_action(m, a, body, |t| format!("destroy {}", t))),

        // Duel.SendtoGrave(target, reason)
        "Duel.SendtoGrave" => Some(gated_action(m, a, body, |t| format!("send {} to gy", t))),

        // Duel.SendtoHand(target, player, reason)
        "Duel.SendtoHand" => Some(gated_action(m, a, body, |t| format!("add_to_hand {}", t))),

        // Duel.SendtoDeck(target, player, sequence, reason)
        "Duel.SendtoDeck" => Some(gated_action(m, a, body, |t| format!("send {} to deck", t))),

        // Duel.Remove(target, pos, reason) — banish
        "Duel.Remove" => Some(gated_action(m, a, body, |t| format!("banish {}", t))),

        // Duel.Release(target, reason) — tribute. No DSL `tribute`
        // action in resolve grammar (only `tribute self` in cost block).
        // Released cards go to gy; closest semantic action is send-to-gy.
        "Duel.Release" => Some(gated_action(m, a, body, |t| format!("send {} to gy", t))),

        // Duel.SpecialSummon(target, sumtype, p1, p2, nocheck, nolimit, pos)
        "Duel.SpecialSummon" => Some(gated_action(m, a, body, |t| format!("special_summon {}", t))),
        "Duel.SpecialSummonStep" => Some(gated_action(m, a, body, |t| format!("special_summon {}", t))),
        "Duel.SpecialSummonComplete" => None, // boundary marker

        // Duel.DiscardHand(player, filter, min, max, reason)
        "Duel.DiscardHand" => Some(DslLine::Action(
            "discard (1+, card, you control, from hand)".to_string()
        )),

        // Duel.ChangePosition(target, position) — change face-up/down
        // attack/defense. We can't always extract position from args, but
        // the DSL `change_position target` (no `to ...`) is valid: lets the
        // engine pick. If we recognize a literal POS_*, we add `to`.
        "Duel.ChangePosition" => Some(action_change_position(a, body)),

        // Duel.Equip(player, equipper, target, ...) — equip self/target to
        // another card. DSL: `equip <eq> to <target>`. We only handle the
        // common shape `Duel.Equip(tp, c, target, ...)` → equip self to target.
        "Duel.Equip" => Some(action_equip(a, bindings, body)),

        // Duel.GetControl(targets, player[, reset, count, zone]) — control
        // change (Phase 17). Permanent take-to-you with a resolvable
        // selection → `take_control <selector>`; temporary (reset args)
        // or opponent-directed shapes have no DSL form yet.
        "Duel.GetControl" => Some(action_get_control(a, bindings)),

        // Duel.Overlay(xyz_target, materials, [send_overlay]) — attach
        // materials as Xyz Materials to the target. DSL: `attach <materials>
        // to <target> as_material`. T33 translator extension.
        "Duel.Overlay" => Some(action_overlay(a, bindings, body)),

        // Duel.SSet(player, target) — set spell/trap face-down on field.
        // The card arg is position 1 (position 0 is the player).
        "Duel.SSet" => Some(gated_action_at(m, a, 1, body, |t| format!("set {}", t))),

        // Duel.ShuffleDeck(player) — shuffle. DSL has shuffle_deck with
        // optional yours/opponents/both; default is implicit yours.
        "Duel.ShuffleDeck" => Some(action_shuffle(a)),

        // Duel.ShuffleHand(player) — shuffle hand. DSL `shuffle_hand` with
        // optional yours/opponents owner.
        "Duel.ShuffleHand" => Some(action_shuffle_hand(a)),

        // Duel.DiscardDeck(player, count, reason) — send top N cards of
        // player's deck to gy. DSL `mill N [from opponent_deck]` covers
        // self-mill (default) and opponent-mill. Non-literal N → TODO.
        "Duel.DiscardDeck" => Some(action_discard_deck(a)),

        // Duel.Announce* family — UI prompt for card / attribute / race /
        // level / type. Five DSL atoms in `announce_what`. Number variants
        // (AnnounceNumber/Range/Coin) have no DSL atom → TODO.
        "Duel.AnnounceCard"      => Some(DslLine::Action("announce card".into())),
        "Duel.AnnounceAttribute" => Some(DslLine::Action("announce attribute".into())),
        "Duel.AnnounceRace"      => Some(DslLine::Action("announce race".into())),
        "Duel.AnnounceLevel"     => Some(DslLine::Action("announce level".into())),
        "Duel.AnnounceType"      => Some(DslLine::Action("announce type".into())),

        // Duel.NegateAttack — DSL `negate` (no destroy variant).
        "Duel.NegateAttack" => Some(DslLine::Action("negate".to_string())),
        "Duel.NegateActivation" => Some(DslLine::Action("negate".to_string())),
        "Duel.NegateEffect" => Some(DslLine::Action("negate".to_string())),

        // Special-summon family that's not the basic SpecialSummon —
        // engine handles them as variants of the same action. The summoned
        // card is arg 1 (arg 0 is the player).
        "Duel.SynchroSummon" | "Duel.XyzSummon" | "Duel.LinkSummon"
        | "Duel.FusionSummon" | "Duel.RitualSummon"
        => Some(gated_action_at(m, a, 1, body, |t| format!("special_summon {}", t))),

        // Duel.Summon(player, target, ignore_count, e, min, max) — normal summon
        "Duel.Summon" => Some(gated_action_at(m, a, 1, body, |t| format!("normal_summon {}", t))),

        // Duel.RemoveCounter — field-wide counter removal (Phase 13).
        "Duel.RemoveCounter" => Some(action_duel_remove_counter(a)),

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
fn action_change_position(args: &[String], body: &FunctionBody) -> DslLine {
    let Some(target) = group_arg(args, 0, body) else {
        return DslLine::Todo(format!(
            "Duel.ChangePosition({}) — target provenance unknown",
            args.join(", ")
        ));
    };
    let pos = args.get(1).map(String::as_str).unwrap_or("");
    let to = match pos {
        "POS_FACEUP_ATTACK"     => Some("attack_position"),
        "POS_FACEUP_DEFENSE"    => Some("defense_position"),
        "POS_FACEDOWN_DEFENSE"  => Some("face_down_defense"),
        _ => None,
    };
    match to {
        Some(p) => DslLine::Action(format!("change_position {} to {}", target, p)),
        None    => DslLine::Action(format!("change_position {}", target)),
    }
}

/// Render a consuming action whose first argument is a card/group
/// selector. Unresolvable provenance → TODO line instead of a silently
/// mis-aimed bare `target` (skip-not-mis-emit).
fn gated_action(
    method: &str,
    args: &[String],
    body: &FunctionBody,
    render: impl FnOnce(&str) -> String,
) -> DslLine {
    gated_action_at(method, args, 0, body, render)
}

/// [`gated_action`] for methods whose card/group argument is not in
/// position 0 (SSet / Summon / the extra-deck summon family put the
/// player first).
fn gated_action_at(
    method: &str,
    args: &[String],
    idx: usize,
    body: &FunctionBody,
    render: impl FnOnce(&str) -> String,
) -> DslLine {
    match group_arg(args, idx, body) {
        Some(sel) => DslLine::Action(render(&sel)),
        None => DslLine::Todo(format!(
            "{}({}) — target provenance unknown",
            method,
            args.join(", ")
        )),
    }
}

/// Resolve action argument N to a DSL selector expression. If the
/// argument names a known group binding, substitute the captured
/// SelectorSpec. The bare `target` placeholder is only emitted when the
/// argument demonstrably holds the effect's DECLARED targets —
/// `Duel.GetFirstTarget` / `Duel.GetTargetCards` provenance, inline or
/// via a single-assigned local (same gate as `resolve_body_selector`,
/// Phase 13b). Anything else (event groups, GetFieldCard bindings,
/// GetMatchingGroup locals that lost their spec, helper params, …) is a
/// different card entirely — returns None so callers emit a TODO.
fn group_arg(args: &[String], idx: usize, body: &FunctionBody) -> Option<String> {
    let raw = args.get(idx)?.trim();
    // Strip common ":GetFirst()" or ":Filter(...)" suffix to get base name.
    let base = raw.split(|c| c == ':' || c == '.').next().unwrap_or(raw);
    // Special: `c` and `e:GetHandler()` reliably refer to the host card.
    // Emit `self` so actions like `Duel.SendtoGrave(c, ...)` translate to
    // `send self to gy`.
    if base == "c" || raw == "e:GetHandler()" {
        return Some("self".to_string());
    }
    if let Some(spec) = body.group_bindings.get(base) {
        return Some(spec.to_dsl());
    }
    // Declared-target fetch, inline or via a single-assigned local.
    if is_declared_target(raw, body) {
        return Some("target".to_string());
    }
    None
}

/// True when the argument (or the single-assigned local it names)
/// demonstrably holds the effect's DECLARED targets — an inline
/// `Duel.GetFirstTarget` / `Duel.GetTargetCards` fetch, or a local bound
/// to one (taint logic in `extract_value_bindings` drops reassigned
/// names). Variable NAMES like `tc` carry no provenance: a `tc` fed from
/// `SelectMatchingCard` or `GetFirstCardTarget` is not the declared target.
fn is_declared_target(raw: &str, body: &FunctionBody) -> bool {
    let base = raw.split([':', '.']).next().unwrap_or(raw).trim();
    raw.starts_with("Duel.GetFirstTarget(")
        || raw.starts_with("Duel.GetTargetCards(")
        || body.value_bindings.get(base).is_some_and(|rhs| {
            rhs.starts_with("Duel.GetFirstTarget(") || rhs.starts_with("Duel.GetTargetCards(")
        })
}

/// `Duel.Overlay(xyz_target, materials, [send_overlay])` → `attach <mat> to <xyz> as_material`.
///
/// Argument resolution per side:
///   - literal `c` / `e:GetHandler()` → `self` (the host card)
///   - declared-target provenance (`is_declared_target`) → `target`
///   - any other bare ident → look up in group bindings, use captured spec
///   - else → emit a TODO line (unresolvable selector)
fn action_overlay(
    args: &[String],
    bindings: &BTreeMap<String, SelectorSpec>,
    body: &FunctionBody,
) -> DslLine {
    let target_raw = args.first().map(String::as_str).unwrap_or("");
    let materials_raw = args.get(1).map(String::as_str).unwrap_or("");
    match (
        xyz_arg_to_dsl(target_raw, bindings, body),
        xyz_arg_to_dsl(materials_raw, bindings, body),
    ) {
        (Some(target), Some(materials)) => {
            DslLine::Action(format!("attach {} to {} as_material", materials, target))
        }
        _ => DslLine::Todo(format!(
            "Duel.Overlay(target={}, materials={}) — unresolvable selector",
            target_raw, materials_raw
        )),
    }
}

/// Resolve a single Duel.Overlay argument to a DSL selector expression.
/// Returns None when the argument is neither the host card, a tracked
/// group binding, nor a declared-target fetch — caller emits a TODO.
fn xyz_arg_to_dsl(
    raw: &str,
    bindings: &BTreeMap<String, SelectorSpec>,
    body: &FunctionBody,
) -> Option<String> {
    let raw = raw.trim();
    let base = raw.split(|ch| ch == ':' || ch == '.').next().unwrap_or(raw);
    if base == "c" || raw == "e:GetHandler()" {
        return Some("self".to_string());
    }
    if let Some(spec) = bindings.get(base) {
        return Some(spec.to_dsl());
    }
    if is_declared_target(raw, body) {
        return Some("target".to_string());
    }
    None
}

/// `Duel.Equip(player, eq, tar, ...)` → `equip self to target` for the
/// canonical "equip this card to selected target" shape. Other shapes
/// (equip group to single target, multi-target) → TODO.
///
/// Phase 17: when the recipient names a one-card selection binding
/// (`Duel.SelectMatchingCard` provenance), the selection folds into the
/// recipient selector instead of the bare `target` placeholder — a
/// resolve-time pick is NOT the declared target.
fn action_equip(
    args: &[String],
    bindings: &BTreeMap<String, SelectorSpec>,
    body: &FunctionBody,
) -> DslLine {
    let eq = args.get(1).map(String::as_str).unwrap_or("");
    let tar = args.get(2).map(String::as_str).unwrap_or("");
    // The equip card is "self" only with provenance: the literal host card,
    // or a local bound to `e:GetLabelObject()` (Phase 9 equip-spell
    // self-ref pattern). A variable NAME like `ec` proves nothing — an `ec`
    // fed from `SelectMatchingCard` is a different card entirely.
    let eq_is_self = eq == "c"
        || eq == "e:GetHandler()"
        || body
            .value_bindings
            .get(eq)
            .is_some_and(|rhs| rhs.starts_with("e:GetLabelObject("));
    if eq_is_self {
        let tar_base = tar.split([':', '.']).next().unwrap_or(tar).trim();
        if let Some(spec) = bindings.get(tar_base) {
            if spec.quantity == "1" {
                return DslLine::Action(format!("equip self to {}", spec.to_dsl()));
            }
        } else if is_declared_target(tar, body) {
            return DslLine::Action("equip self to target".to_string());
        }
    }
    DslLine::Todo(format!("Duel.Equip(eq={}, tar={}) — non-canonical shape", eq, tar))
}

/// `Duel.GetControl(targets, player[, reset, count, zone])` →
/// `take_control <selector>` (Phase 17). Emits only the permanent
/// take-to-you shape with a selection-binding target; temporary control
/// (reset args present) and opponent-directed shapes skip as TODO.
fn action_get_control(args: &[String], bindings: &BTreeMap<String, SelectorSpec>) -> DslLine {
    let raw = args.first().map(String::as_str).unwrap_or("");
    let player = args.get(1).map(String::as_str).unwrap_or("");
    if args.len() > 2 || player != "tp" {
        return DslLine::Todo(format!(
            "Duel.GetControl({}) — temporary or non-self control change",
            args.join(", ")
        ));
    }
    let base = raw.split([':', '.']).next().unwrap_or(raw).trim();
    match bindings.get(base) {
        Some(spec) => DslLine::Action(format!("take_control {}", spec.to_dsl())),
        None => DslLine::Todo(format!(
            "Duel.GetControl(targets={}, player=tp) — unresolvable selector", raw
        )),
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

/// `Duel.DiscardDeck(player, count, reason)` → `mill <N> [from opponent_deck]`.
/// Non-literal count → TODO (DSL grammar requires an integer expr).
fn action_discard_deck(args: &[String]) -> DslLine {
    let player = args.first().map(String::as_str).unwrap_or("");
    let count = args.get(1).map(String::as_str).unwrap_or("");
    if count.parse::<u32>().is_err() {
        return DslLine::Todo(format!(
            "Duel.DiscardDeck(player={}, count={}) — non-literal count",
            player, count
        ));
    }
    match player {
        "tp" => DslLine::Action(format!("mill {}", count)),
        "1-tp" => DslLine::Action(format!("mill {} from opponent_deck", count)),
        _ => DslLine::Todo(format!(
            "Duel.DiscardDeck(player={}, count={}) — non-canonical player",
            player, count
        )),
    }
}

/// `Duel.ShuffleHand(player)` → `shuffle_hand [yours|opponents]`.
fn action_shuffle_hand(args: &[String]) -> DslLine {
    let player = args.first().map(String::as_str).unwrap_or("");
    let who = match player {
        "tp" => "yours",
        "1-tp" => "opponents",
        _ => return DslLine::Todo(format!(
            "Duel.ShuffleHand(player={}) — non-canonical player", player
        )),
    };
    DslLine::Action(format!("shuffle_hand {}", who))
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

/// `Duel.RemoveCounter(player, s, o, countertype, count, reason)` →
/// `remove_counter "<name>" 1 from (1, card, <controller>)` (Phase 13).
///
/// `s`/`o` are side booleans: counters may come off the player's own
/// side and/or the opponent's, anywhere on the field. Emitted only for
/// count == 1 — for one counter "anywhere in scope" collapses to one
/// card, whereas the DSL form removes the full count from a SINGLE
/// selected card while the lua allows spreading n > 1 across cards.
/// Everything else → Todo (matches the previous catch-all behavior).
fn action_duel_remove_counter(args: &[String]) -> DslLine {
    match duel_remove_counter_parts(args) {
        Some((name, controller)) => DslLine::Action(format!(
            "remove_counter \"{}\" 1 from (1, card, {})", name, controller
        )),
        None => DslLine::Todo(format!("Duel.RemoveCounter({})", args.join(", "))),
    }
}

/// Decode the translatable `Duel.RemoveCounter` arg shape into
/// (counter name, DSL controller phrase). None when the player isn't
/// `tp`, the count isn't the literal 1, the side booleans aren't a
/// recognized pair, or the countertype has no curated name.
fn duel_remove_counter_parts(args: &[String]) -> Option<(&'static str, &'static str)> {
    let player = args.first().map(String::as_str).unwrap_or("");
    let s = args.get(1).map(String::as_str).unwrap_or("");
    let o = args.get(2).map(String::as_str).unwrap_or("");
    let countertype = args.get(3).map(String::as_str).unwrap_or("");
    let count = args.get(4).map(String::as_str).unwrap_or("");
    if player != "tp" || count != "1" { return None; }
    let controller = match (s, o) {
        ("1", "0") => "you control",
        ("0", "1") => "opponent controls",
        ("1", "1") => "either controls",
        _ => return None,
    };
    let name = counter_arg_to_name(countertype)?;
    Some((name, controller))
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
    fn translate_duel_destroy_unknown_provenance_todo() {
        // `g` with no tracked binding and no declared-target provenance —
        // bare `target` would mis-aim, so the line degrades to a TODO.
        let calls = vec![
            DuelCall { method: "Duel.Destroy".to_string(), args: vec!["g".into(), "REASON_EFFECT".into()] },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Todo(s) if s.contains("target provenance unknown")));
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
    fn translate_duel_overlay_self_target() {
        // Duel.Overlay(c, tc, true) with `tc = Duel.GetFirstTarget()` —
        // attach first target to host as material.
        let body = FunctionBody {
            calls: vec![DuelCall {
                method: "Duel.Overlay".to_string(),
                args: vec!["c".into(), "tc".into(), "true".into()],
            }],
            value_bindings: BTreeMap::from([
                ("tc".to_string(), "Duel.GetFirstTarget()".to_string()),
            ]),
            ..FunctionBody::default()
        };
        let lines = translate_body(&body);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "attach target to self as_material"));
    }

    #[test]
    fn translate_duel_overlay_target_to_target() {
        // Duel.Overlay(tc, c) with declared-target provenance on tc —
        // attach self to the targeted Xyz monster.
        let body = FunctionBody {
            calls: vec![DuelCall {
                method: "Duel.Overlay".to_string(),
                args: vec!["tc".into(), "c".into()],
            }],
            value_bindings: BTreeMap::from([
                ("tc".to_string(), "Duel.GetFirstTarget()".to_string()),
            ]),
            ..FunctionBody::default()
        };
        let lines = translate_body(&body);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "attach self to target as_material"));
    }

    #[test]
    fn translate_duel_overlay_name_alone_carries_no_provenance() {
        // Bare `tc` with no tracked assignment must NOT become `target`:
        // a `tc` fed from `c:GetFirstCardTarget()` (continuous card target)
        // is not the chain's declared target.
        let calls = vec![DuelCall {
            method: "Duel.Overlay".to_string(),
            args: vec!["tc".into(), "c".into()],
        }];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Todo(_)),
            "unproven overlay dest should emit a TODO; got {:?}", lines[0]);
    }

    #[test]
    fn translate_duel_overlay_unbound_emits_todo() {
        // Unknown selectors on both sides → TODO, not silent action.
        let calls = vec![
            DuelCall {
                method: "Duel.Overlay".to_string(),
                args: vec!["sc".into(), "mg".into()],
            },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Todo(_)));
    }

    #[test]
    fn translate_duel_shuffle_hand_yours() {
        let calls = vec![
            DuelCall { method: "Duel.ShuffleHand".to_string(), args: vec!["tp".into()] },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "shuffle_hand yours"));
    }

    #[test]
    fn translate_duel_shuffle_hand_opponents() {
        let calls = vec![
            DuelCall { method: "Duel.ShuffleHand".to_string(), args: vec!["1-tp".into()] },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "shuffle_hand opponents"));
    }

    #[test]
    fn translate_duel_shuffle_hand_non_canonical_player_emits_todo() {
        let calls = vec![
            DuelCall { method: "Duel.ShuffleHand".to_string(), args: vec!["weird".into()] },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Todo(_)));
    }

    #[test]
    fn translate_duel_discard_deck_self_literal() {
        let calls = vec![
            DuelCall {
                method: "Duel.DiscardDeck".to_string(),
                args: vec!["tp".into(), "3".into(), "REASON_EFFECT".into()],
            },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "mill 3"));
    }

    #[test]
    fn translate_duel_discard_deck_opponent_literal() {
        let calls = vec![
            DuelCall {
                method: "Duel.DiscardDeck".to_string(),
                args: vec!["1-tp".into(), "5".into(), "REASON_EFFECT".into()],
            },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "mill 5 from opponent_deck"));
    }

    #[test]
    fn translate_duel_discard_deck_non_literal_count_emits_todo() {
        let calls = vec![
            DuelCall {
                method: "Duel.DiscardDeck".to_string(),
                args: vec!["tp".into(), "ct".into(), "REASON_EFFECT".into()],
            },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Todo(_)));
    }

    #[test]
    fn translate_duel_announce_family() {
        for (method, expected) in [
            ("Duel.AnnounceCard", "announce card"),
            ("Duel.AnnounceAttribute", "announce attribute"),
            ("Duel.AnnounceRace", "announce race"),
            ("Duel.AnnounceLevel", "announce level"),
            ("Duel.AnnounceType", "announce type"),
        ] {
            let calls = vec![
                DuelCall { method: method.to_string(), args: vec!["tp".into()] },
            ];
            let lines = translate_calls(&calls);
            match &lines[0] {
                DslLine::Action(s) => assert_eq!(s, expected, "method={}", method),
                DslLine::Todo(t) => panic!("expected action for {}, got TODO: {}", method, t),
            }
        }
    }

    #[test]
    fn translate_duel_announce_number_still_todo() {
        // No DSL atom for numeric announce → must remain TODO so caller knows.
        let calls = vec![
            DuelCall { method: "Duel.AnnounceNumber".to_string(), args: vec!["tp".into(), "1".into(), "2".into()] },
        ];
        let lines = translate_calls(&calls);
        assert!(matches!(&lines[0], DslLine::Todo(_)));
    }

    #[test]
    fn install_watcher_battle_destroying_damage_shape() {
        // Future Drive shape: an operation handler registers a continuous
        // trigger effect on tc whose own operation (s.damop) deals damage
        // when tc destroys a card by battle.
        let src = r#"
local s,id=GetID()
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_SINGLE+EFFECT_TYPE_CONTINUOUS)
    e3:SetCode(EVENT_BATTLE_DESTROYING)
    e3:SetOperation(s.damop)
    e3:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e3)
end
function s.damop(e,tp,eg,ep,ev,re,r,rp)
    Duel.Damage(1-tp,1000,REASON_EFFECT)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body_with_functions(body, &report.functions);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(
            action,
            Some(r#"install_watcher "damop" { event: destroys_by_battle duration: end_of_turn check { damage opponent 1000 } }"#),
        );
    }

    #[test]
    fn install_watcher_skips_chain_without_end_of_turn_reset() {
        // Same shape minus the SetReset call — should not emit a watcher
        // (no duration guard would let it run forever in DSL).
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local e3=Effect.CreateEffect(c)
    e3:SetCode(EVENT_BATTLE_DESTROYING)
    e3:SetOperation(s.damop)
    tc:RegisterEffect(e3)
end
function s.damop(e,tp,eg,ep,ev,re,r,rp)
    Duel.Damage(1-tp,500,REASON_EFFECT)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body_with_functions(body, &report.functions);
        assert!(
            !lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("install_watcher"))),
            "no SetReset → must not emit install_watcher",
        );
    }

    #[test]
    fn install_watcher_skips_chain_with_set_condition() {
        // Tyrant Wing e9 shape: the trigger chain carries a SetCondition
        // handler (s.descon — flag effect set only when the equipped
        // monster attacked twice). The watcher grammar has no condition
        // slot, so emitting would fire the check unconditionally every
        // end phase. Skip-not-mis-emit.
        let src = r#"
local s,id=GetID()
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e9=Effect.CreateEffect(c)
    e9:SetType(EFFECT_TYPE_FIELD+EFFECT_TYPE_TRIGGER_F)
    e9:SetCode(EVENT_PHASE+PHASE_END)
    e9:SetCondition(s.descon)
    e9:SetOperation(s.desop)
    e9:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e9)
end
function s.descon(e,tp,eg,ep,ev,re,r,rp)
    return e:GetHandler():GetFlagEffect(id)~=0
end
function s.desop(e,tp,eg,ep,ev,re,r,rp)
    Duel.Destroy(e:GetHandler(),REASON_EFFECT)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body_with_functions(body, &report.functions);
        assert!(
            !lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("install_watcher"))),
            "SetCondition chain → must not emit install_watcher, got: {:?}", lines,
        );
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
    fn register_chain_update_level_emits_modify_level() {
        // Mausoleum-style level buff: tc:RegisterEffect with literal value
        // and a PHASE_END reset → modify_level target + 1 until end_of_turn.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_LEVEL)
    e1:SetValue(1)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("modify_level target + 1 until end_of_turn"));
    }

    #[test]
    fn register_chain_update_level_negative_value() {
        // Level reducer: SetValue(-1) → modify_level target - 1 until end_of_turn.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_LEVEL)
    e1:SetValue(-1)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("modify_level target - 1 until end_of_turn"));
    }

    #[test]
    fn register_chain_change_level_emits_set_level() {
        // CHANGE_LEVEL is an absolute set → set_level target 4 until end_of_turn.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CHANGE_LEVEL)
    e1:SetValue(4)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("set_level target 4 until end_of_turn"));
    }

    #[test]
    fn register_chain_change_level_nonliteral_unknown_receiver_skipped() {
        // Unknown register receiver (sc) → no level line; skip-not-mis-emit.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local sc=Duel.GetAttacker()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CHANGE_LEVEL)
    e1:SetValue(4)
    sc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body(body);
        assert!(
            !lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("set_level"))),
            "unknown receiver must not emit set_level",
        );
    }

    #[test]
    fn register_chain_for_loop_uses_group_selector_spec() {
        // Daigusto Falcos shape: `for tc in aux.Next(g)` where g is a known
        // GetMatchingGroup binding → translator emits modify_atk using
        // the captured SelectorSpec. Filter must be mappable (Phase 10) —
        // `nil` keeps the unfiltered `card` kind.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(nil,tp,LOCATION_MZONE,LOCATION_MZONE,nil)
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
    fn p10_count_local_var_times_n() {
        // Phase 10 primary shape: local count var * literal multiplier →
        // `count(<selector>) * N`. Receiver `c` (= e:GetHandler()) → self.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local ct=Duel.GetMatchingGroupCount(nil,tp,LOCATION_GRAVE,0,nil)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(ct*300)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(
            action,
            Some("modify_atk self + count((all, card, you control, from gy)) * 300 until end_of_turn"),
            "got lines: {:?}", lines
        );
    }

    #[test]
    fn p10_inline_field_count_negative_multiplier() {
        // Inline Duel.GetFieldGroupCount(...) * -500: the negative literal
        // flips the modifier sign; the `1-tp`-free scope keeps `you control`.
        // The `-` inside a (hypothetical) arg list must not split — the
        // paren-depth-aware splitter lands on the top-level `*`.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(Duel.GetFieldGroupCount(tp,LOCATION_MZONE,0)*-500)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(
            action,
            Some("modify_atk target - count((all, card, you control, from monster_zone)) * 500 until end_of_turn"),
            "got lines: {:?}", lines
        );
    }

    #[test]
    fn p10_literal_times_count_var() {
        // Commutated form `300*ct` resolves the same as `ct*300`; the
        // `1-tp` scope player flips the controller to `opponent controls`.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local ct=Duel.GetMatchingGroupCount(Card.IsMonster,1-tp,LOCATION_GRAVE,0,nil)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_DEFENSE)
    e1:SetValue(300*ct)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(
            action,
            Some("modify_def target + 300 * count((all, monster, opponent controls, from gy)) until end_of_turn"),
            "got lines: {:?}", lines
        );
    }

    #[test]
    fn p10_g_iter_loop_with_faceup_filter() {
        // Reinforced-Space-style `for tc in g:Iter()` loop: the group
        // binding's Card.IsFaceup filter refines the selector with a
        // `where is_face_up` clause.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(Card.IsFaceup,tp,LOCATION_MZONE,0,nil)
    for tc in g:Iter() do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        e1:SetValue(400)
        tc:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        assert_eq!(body.register_chains.len(), 1);
        assert_eq!(body.register_chains[0].loop_source_group.as_deref(), Some("g"));
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(
            action,
            Some("modify_atk (all, card, you control, from monster_zone, where is_face_up) + 400 until end_of_turn"),
            "got lines: {:?}", lines
        );
    }

    #[test]
    fn p10_reassigned_count_var_skipped() {
        // Clamped count (`if ct>3 then ct=3 end`) — the binding is written
        // twice, so its value is not statically known. Skip, don't mis-emit
        // the unclamped count.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local ct=Duel.GetMatchingGroupCount(nil,tp,LOCATION_GRAVE,0,nil)
    if ct>3 then ct=3 end
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(ct*300)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        let lines = translate_body(body);
        assert!(
            !lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("modify_atk"))),
            "reassigned count var must not emit, got: {:?}", lines
        );
    }

    #[test]
    fn p10_custom_filter_count_skipped() {
        // Custom `s.filter` predicate has no DSL equivalent — the count's
        // numeric value IS the semantics, so skip instead of counting the
        // whole zone.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local ct=Duel.GetMatchingGroupCount(s.filter,tp,LOCATION_GRAVE,0,nil)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(ct*300)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        let lines = translate_body(body);
        assert!(
            !lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("modify_atk"))),
            "custom filter count must not emit, got: {:?}", lines
        );
    }

    #[test]
    fn p10_stat_write_interference_drops_later_line() {
        // Shogi-Lance shape: `local lv=c:GetLevel()+tc:GetLevel()` applied
        // to BOTH cards. Lua computes lv once; sequential DSL lines would
        // make the second read the first's freshly-written self.level, so
        // only the first line survives.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local lv=c:GetLevel()+tc:GetLevel()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CHANGE_LEVEL)
    e1:SetValue(lv)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e1)
    local e2=e1:Clone()
    tc:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("operation body");
        let lines = translate_body(body);
        let actions: Vec<&str> = lines.iter().filter_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        }).collect();
        assert_eq!(
            actions,
            vec!["set_level self self.level + target.level until while_face_up"],
            "second (interfering) set_level must be dropped",
        );
    }

    #[test]
    fn p10_group_with_unmappable_filter_skipped() {
        // Group built with a custom predicate (`aux.FaceupFilter(
        // Card.IsAttributeExcept, …)`): the DSL selector would match a
        // superset of the lua group, and a group-applied modifier would
        // alter cards the lua never touched. Skip the chain.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(aux.FaceupFilter(Card.IsAttributeExcept,ATTRIBUTE_EARTH),tp,LOCATION_MZONE,LOCATION_MZONE,nil)
    for tc in g:Iter() do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        e1:SetValue(-500)
        tc:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body(body);
        assert!(
            !lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("modify_atk"))),
            "unmappable group filter must not emit, got: {:?}", lines
        );
    }

    #[test]
    fn p10_loop_per_member_value_skipped() {
        // Group-applied chain whose value reads each member (`tc:GetAttack()/2`):
        // DSL `target.atk` would resolve to the selected target, not the loop
        // member — skip the whole chain.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(Card.IsFaceup,tp,LOCATION_MZONE,0,nil)
    for tc in g:Iter() do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        e1:SetValue(tc:GetAttack()/2)
        tc:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body(body);
        assert!(
            !lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("modify_atk"))),
            "per-member loop value must not emit, got: {:?}", lines
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
        assert_eq!(spec.value, "300");
        assert!(!spec.negative);
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
        assert_eq!(spec.value, "200");
        assert!(spec.negative);
        let dsl = spec.to_dsl_block("Penalty", "    ");
        assert!(dsl.contains("modifier: def - 200"), "got:\n{}", dsl);
    }

    /// T34 helper: builds a SINGLE self passive chain around `value`.
    fn overlay_chain(value: &str) -> String {
        format!(r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetProperty(EFFECT_FLAG_SINGLE_RANGE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetRange(LOCATION_MZONE)
    e1:SetValue({value})
    c:RegisterEffect(e1)
end
"#)
    }

    fn spec_of(src: &str) -> Option<PassiveModifierSpec> {
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        report.effects[0].passive_modifier_spec()
    }

    #[test]
    fn t34_overlay_closure_emits_overlay_count_expr() {
        // c94503794 shape: "Gains 700 ATK for each material attached".
        let spec = spec_of(&overlay_chain(
            "function(e,c) return c:GetOverlayCount()*700 end")).expect("spec");
        assert_eq!(spec.value, "self.overlay_count * 700");
        assert!(!spec.negative);
        let dsl = spec.to_dsl_block("Material Boost", "    ");
        assert!(dsl.contains("modifier: atk + self.overlay_count * 700"), "got:\n{}", dsl);
    }

    #[test]
    fn t34_overlay_closure_reversed_factor_and_underscore_param() {
        // c44161893 shape: `function(_,c)`; also a leading factor.
        let spec = spec_of(&overlay_chain(
            "function(_,c) return 300*c:GetOverlayCount() end")).expect("spec");
        assert_eq!(spec.value, "self.overlay_count * 300");
    }

    #[test]
    fn t34_counter_closure_emits_counter_expr() {
        // c31924889 shape: "gains 1000 ATK for each Spell Counter on it".
        let spec = spec_of(&overlay_chain(
            "function(e,c) return c:GetCounter(COUNTER_SPELL)*1000 end")).expect("spec");
        assert_eq!(spec.value, "self.counter(\"Spell Counter\") * 1000");
        let dsl = spec.to_dsl_block("Counter Boost", "    ");
        assert!(dsl.contains("modifier: atk + self.counter(\"Spell Counter\") * 1000"),
            "got:\n{}", dsl);
    }

    #[test]
    fn t34_counter_hex_code_resolves_via_phase13_map() {
        let spec = spec_of(&overlay_chain(
            "function(e,c) return c:GetCounter(0x1)*300 end")).expect("spec");
        assert_eq!(spec.value, "self.counter(\"Spell Counter\") * 300");
    }

    #[test]
    fn t34_bare_overlay_call_emits_factorless_ref() {
        let spec = spec_of(&overlay_chain(
            "function(e,c) return c:GetOverlayCount() end")).expect("spec");
        assert_eq!(spec.value, "self.overlay_count");
    }

    #[test]
    fn t34_skip_one_param_closure() {
        // c19369609 shape: no card param — the handler receiver is not
        // the closure's affected-card arg.
        assert!(spec_of(&overlay_chain(
            "function(e) return e:GetHandler():GetOverlayCount()*500 end")).is_none());
    }

    #[test]
    fn t34_skip_handler_receiver() {
        // c9453320 / c60600821 shapes: two-param closure but the count
        // is read off e:GetHandler(), not the card param.
        assert!(spec_of(&overlay_chain(
            "function(e,c) return 300*e:GetHandler():GetOverlayCount() end")).is_none());
        assert!(spec_of(&overlay_chain(
            "function(e,c) return e:GetHandler():GetCounter(COUNTER_SEASON)*400 end")).is_none());
    }

    #[test]
    fn t34_skip_duel_global_counts() {
        // c57448410 / c22011689 shapes: field-wide Duel.* counts are not
        // a self stat.
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetOverlayCount(0,1,1)*100 end")).is_none());
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetCounter(0,1,1,COUNTER_PREDATOR)*200 end")).is_none());
    }

    #[test]
    fn t34_skip_multi_step_math() {
        // c67630394 / c90303227 shapes: composite arithmetic skips.
        assert!(spec_of(&overlay_chain(
            "function(_,_c) return (_c:GetOverlayCount()+_c:GetEquipCount())*300 end")).is_none());
        assert!(spec_of(&overlay_chain(
            "function(e,c) return math.max(0,e:GetHandler():GetOverlayCount()-1) end")).is_none());
    }

    #[test]
    fn t34_skip_named_function_ref() {
        assert!(spec_of(&overlay_chain("s.atkval")).is_none());
    }

    #[test]
    fn t34_skip_unknown_counter_code() {
        assert!(spec_of(&overlay_chain(
            "function(e,c) return c:GetCounter(0x9999)*100 end")).is_none());
    }

    #[test]
    fn t34_skip_closure_value_on_field_chain() {
        // Closure values only qualify for the SINGLE self shape: in a
        // FIELD chain the closure's `c` is each affected card, which
        // DSL `self` would misread.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetRange(LOCATION_MZONE)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(function(e,c) return c:GetOverlayCount()*200 end)
    c:RegisterEffect(e1)
end
"#;
        assert!(spec_of(src).is_none(),
            "closure SetValue on a FIELD chain must skip");
    }

    /// T34 follow-up helper: FIELD passive chain with the given
    /// `SetTargetRange` args around `value`.
    fn field_count_chain(target_range: &str, value: &str) -> String {
        format!(r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetRange(LOCATION_MZONE)
    e1:SetTargetRange({target_range})
    e1:SetValue({value})
    c:RegisterEffect(e1)
end
"#)
    }

    #[test]
    fn t34fu_count_faceup_setcard_self_chain() {
        // c11335209 shape: "gains 1200 ATK for each face-up Horus
        // monster you control"; scope player is c:GetControler().
        let spec = spec_of(&overlay_chain(
            "function(e,c) return Duel.GetMatchingGroupCount(aux.FaceupFilter(Card.IsSetCard,SET_HORUS),c:GetControler(),LOCATION_MZONE,0,nil)*1200 end",
        )).expect("spec");
        assert_eq!(spec.value,
            "count((all, card, you control, from monster_zone, where archetype == \"Horus\" and is_face_up)) * 1200");
        assert!(!spec.negative);
    }

    #[test]
    fn t34fu_count_gfgc_hand_clone_pair() {
        // c98777036 (Tragoedia): ATK chain + Clone'd DEF chain, both
        // counting the controller's hand.
        let src = r#"
function s.initial_effect(c)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_SINGLE)
    e2:SetProperty(EFFECT_FLAG_SINGLE_RANGE)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetRange(LOCATION_MZONE)
    e2:SetValue(function(_,c) return Duel.GetFieldGroupCount(c:GetControler(),LOCATION_HAND,0)*600 end)
    c:RegisterEffect(e2)
    local e3=e2:Clone()
    e3:SetCode(EFFECT_UPDATE_DEFENSE)
    c:RegisterEffect(e3)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let atk = report.effects[0].passive_modifier_spec().expect("atk spec");
        let def = report.effects[1].passive_modifier_spec().expect("def spec");
        assert_eq!(atk.stat, "atk");
        assert_eq!(def.stat, "def");
        assert_eq!(atk.value, "count((all, card, you control, from hand)) * 600");
        assert_eq!(def.value, atk.value);
    }

    #[test]
    fn t34fu_count_field_chain_handler_player() {
        // c63233638 (Megalith Phaleg): FIELD chain, card-param-free
        // closure scoped via e:GetHandlerPlayer(), leading factor.
        let spec = spec_of(&field_count_chain(
            "LOCATION_MZONE,0",
            "function(e,c) return 300*Duel.GetMatchingGroupCount(Card.IsRitualMonster,e:GetHandlerPlayer(),LOCATION_GRAVE,0,nil) end",
        )).expect("spec");
        assert_eq!(spec.value, "count((all, ritual monster, you control, from gy)) * 300");
        assert_eq!(spec.scope, Some("field"));
        assert_eq!(spec.target, Some("(all, monster, you control)"));
    }

    #[test]
    fn t34fu_count_negative_factor_flips_sign() {
        // c90673288 shape: opponent's monsters lose 100 ATK for each
        // spell in the handler's GY.
        let spec = spec_of(&field_count_chain(
            "0,LOCATION_MZONE",
            "function(e) return Duel.GetMatchingGroupCount(Card.IsSpell,e:GetHandlerPlayer(),LOCATION_GRAVE,0,nil)*-100 end",
        )).expect("spec");
        assert!(spec.negative);
        assert_eq!(spec.value, "count((all, spell, you control, from gy)) * 100");
        let dsl = spec.to_dsl_block("Static Drain", "    ");
        assert!(dsl.contains("modifier: atk - count((all, spell, you control, from gy)) * 100"),
            "got:\n{}", dsl);
    }

    #[test]
    fn t34fu_count_symmetric_literal_scope() {
        // c15665977 shape: literal player 0 with equal location masks —
        // player-symmetric, so "either controls" is faithful.
        let spec = spec_of(&overlay_chain(
            "function(e,c) return 300*Duel.GetMatchingGroupCount(Card.IsType,0,LOCATION_GRAVE,LOCATION_GRAVE,nil,TYPE_TUNER) end",
        )).expect("spec");
        assert_eq!(spec.value, "count((all, tuner monster, either controls, from gy)) * 300");
    }

    #[test]
    fn t34fu_count_exception_maps_to_except_self() {
        // c80071619: exception is e:GetHandler() on a self chain.
        let spec = spec_of(&overlay_chain(
            "function(e,c) return 500*Duel.GetMatchingGroupCount(nil,c:GetControler(),LOCATION_MZONE,0,e:GetHandler()) end",
        )).expect("spec");
        assert_eq!(spec.value, "count((all, card, you control, from monster_zone, except self)) * 500");
        // c2137678: exception is the closure's card param; symmetric
        // literal scope + FaceupFilter archetype composition.
        let spec = spec_of(&overlay_chain(
            "function(e,c) return 100*Duel.GetMatchingGroupCount(aux.FaceupFilter(Card.IsSetCard,SET_MEKLORD),0,LOCATION_MZONE,LOCATION_MZONE,c) end",
        )).expect("spec");
        assert_eq!(spec.value,
            "count((all, card, either controls, from monster_zone, except self, where archetype == \"Meklord\" and is_face_up)) * 100");
    }

    #[test]
    fn t34fu_count_equip_chain_card_param_free() {
        // c24839398 shape: EQUIP chain, one-param closure — no card
        // param read, so the non-self scope is safe.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_EQUIP)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(function(e) return Duel.GetMatchingGroupCount(Card.IsTrap,e:GetHandlerPlayer(),LOCATION_GRAVE,0,nil)*400 end)
    c:RegisterEffect(e1)
end
"#;
        let spec = spec_of(src).expect("spec");
        assert_eq!(spec.value, "count((all, trap, you control, from gy)) * 400");
        assert_eq!(spec.target, Some("equipped_card"));
    }

    #[test]
    fn t34fu_count_zero_param_closure() {
        // c2333466 shape: `function()` — fine, the symmetric literal
        // scope needs neither param.
        let spec = spec_of(&overlay_chain(
            "function() return Duel.GetFieldGroupCount(0,LOCATION_REMOVED,LOCATION_REMOVED)*300 end",
        )).expect("spec");
        assert_eq!(spec.value, "count((all, card, either controls, from banished)) * 300");
    }

    #[test]
    fn t34fu_count_istype_extra_arg_kinds() {
        // c13893596 (Exodius): Card.IsType routes its trailing count-call
        // arg into the kind ("normal monster").
        let spec = spec_of(&overlay_chain(
            "function(e,c) return Duel.GetMatchingGroupCount(Card.IsType,c:GetControler(),LOCATION_GRAVE,0,nil,TYPE_NORMAL)*1000 end",
        )).expect("spec");
        assert_eq!(spec.value, "count((all, normal monster, you control, from gy)) * 1000");
    }

    #[test]
    fn t34fu_count_facedown_extra_deck() {
        // c16893370 shape: `function(_,c)` underscore e-param.
        let spec = spec_of(&overlay_chain(
            "function(_,c) return Duel.GetMatchingGroupCount(Card.IsFacedown,c:GetControler(),LOCATION_EXTRA,0,nil)*100 end",
        )).expect("spec");
        assert_eq!(spec.value,
            "count((all, card, you control, from extra_deck, where is_face_down)) * 100");
    }

    #[test]
    fn t34fu_count_asymmetric_opponent_grave() {
        // c45935145 shape: my_locs=0, opp_locs=GRAVE relative to the
        // handler player → opponent's GY.
        let spec = spec_of(&field_count_chain(
            "LOCATION_MZONE,0",
            "function(e) return Duel.GetFieldGroupCount(e:GetHandlerPlayer(),0,LOCATION_GRAVE)*100 end",
        )).expect("spec");
        assert_eq!(spec.value, "count((all, card, opponent controls, from gy)) * 100");
    }

    #[test]
    fn t34fu_skip_card_param_scope_on_non_self_chain() {
        // In FIELD/EQUIP chains the closure's card param is each
        // affected card — its controller is not necessarily "you".
        assert!(spec_of(&field_count_chain(
            "LOCATION_MZONE,0",
            "function(e,c) return Duel.GetMatchingGroupCount(Card.IsSpell,c:GetControler(),LOCATION_GRAVE,0,nil)*100 end",
        )).is_none(), "card-param scope on a FIELD chain must skip");
    }

    #[test]
    fn t34fu_skip_asymmetric_literal_scope() {
        // Literal player with unequal masks is an absolute-player
        // count the relative DSL cannot express.
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetFieldGroupCount(0,LOCATION_GRAVE,0)*100 end",
        )).is_none(), "asymmetric literal-player scope must skip");
    }

    #[test]
    fn t34fu_skip_unmapped_filters() {
        // c32588805: Card.IsSpellTrap would need a nested `(a or b)`
        // predicate the parser does not implement.
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetMatchingGroupCount(aux.FaceupFilter(Card.IsSpellTrap),e:GetHandlerPlayer(),LOCATION_ONFIELD,0,nil)*300 end",
        )).is_none());
        // c43490025: custom closure filter.
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetMatchingGroupCount(function(c) return c:IsSetCard(SET_NUMBER) and c:IsType(TYPE_XYZ) end,0,LOCATION_GRAVE,LOCATION_GRAVE,nil)*500 end",
        )).is_none());
        // Unmapped Card.IsType arg.
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetMatchingGroupCount(Card.IsType,c:GetControler(),LOCATION_GRAVE,0,nil,TYPE_FUSION)*100 end",
        )).is_none());
    }

    #[test]
    fn t34fu_skip_multi_step_count_body() {
        // c89851827 / c51669847 shapes: capped or composite math.
        assert!(spec_of(&overlay_chain(
            "function(e) return math.min(Duel.GetFieldGroupCount(e:GetHandlerPlayer(),LOCATION_REMOVED,0)*100,1000) end",
        )).is_none());
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetFieldGroupCount(0,LOCATION_REMOVED,LOCATION_REMOVED)*100+200 end",
        )).is_none());
    }

    #[test]
    fn t34fu_skip_foreign_exception() {
        // Exception that is neither nil nor the passive's own card.
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetMatchingGroupCount(nil,c:GetControler(),LOCATION_MZONE,0,g)*100 end",
        )).is_none(), "unknown exception binding must skip");
    }

    #[test]
    fn t34fu_skip_or_mask_locations() {
        // Multi-zone OR masks have no single DSL `from` token.
        assert!(spec_of(&overlay_chain(
            "function(e,c) return Duel.GetMatchingGroupCount(Card.IsSpell,c:GetControler(),LOCATION_MZONE|LOCATION_GRAVE,0,nil)*100 end",
        )).is_none());
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
        assert_eq!(action, Some("modify_atk target + target.atk until while_face_up"));
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
        assert_eq!(action, Some("modify_atk target + target.atk / 2 until while_face_up"));
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
        assert_eq!(action, Some("modify_atk target + target.level * 100 until while_face_up"));
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
        // Declared-target battle protection: tc:RegisterEffect with
        // EFFECT_INDESTRUCTABLE_BATTLE + RESET_PHASE|PHASE_DAMAGE reset →
        // grant target cannot_be_destroyed by battle until
        // end_of_damage_step (the reset fires at damage-step end, not
        // at the End Phase).
        // (Originally used the Shield Warrior `e:GetLabelObject()`
        // binding; that provenance is statically unknowable and now
        // skips under the Phase 13b gate — see p13b tests.)
        let src = r#"
function s.atkop(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
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
            Some("grant target cannot_be_destroyed by battle until end_of_damage_step"),
        );
    }

    #[test]
    fn reset_duration_kw_exact_token_mapping() {
        let kw = |r: &str| reset_to_duration_kw(Some(r), None);
        // Phase-end resets — with or without the event family, spelled
        // out or via the CardScripts macros, `|` or `+` joined.
        assert_eq!(kw("RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_END"), Some("end_of_turn"));
        assert_eq!(kw("RESET_EVENT+RESETS_STANDARD+RESET_PHASE+PHASE_END"), Some("end_of_turn"));
        assert_eq!(kw("RESETS_STANDARD_PHASE_END"), Some("end_of_turn"));
        assert_eq!(kw("RESETS_STANDARD_DISABLE_PHASE_END"), Some("end_of_turn"));
        assert_eq!(kw("RESET_PHASE|PHASE_END"), Some("end_of_turn"));
        // Event-only standard resets live while the card stays face-up
        // on the field — NOT until end of turn (c61151074's +1500 buff
        // is permanent while the card sits face-up).
        assert_eq!(kw("RESET_EVENT|RESETS_STANDARD"), Some("while_face_up"));
        assert_eq!(kw("RESET_EVENT|RESETS_STANDARD_DISABLE"), Some("while_face_up"));
        // Standby-phase reset on the owner's turn (c11493868's +800).
        assert_eq!(
            kw("RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_STANDBY|RESET_SELF_TURN"),
            Some("next_standby_phase"),
        );
        assert_eq!(kw("RESET_PHASE|PHASE_STANDBY|RESET_SELF_TURN"), Some("next_standby_phase"));
        // Damage-step resets.
        assert_eq!(
            kw("RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_DAMAGE"),
            Some("end_of_damage_step"),
        );
        assert_eq!(
            kw("RESET_EVENT|RESETS_STANDARD_DISABLE|RESET_PHASE|PHASE_DAMAGE_CAL"),
            Some("end_of_damage_step"),
        );
        // Unmapped lifetimes skip: turn-qualified phase-end ("end of
        // YOUR/opponent's turn"), unqualified standby, battle-phase
        // resets, bit-arithmetic families, bare flags.
        assert_eq!(kw("RESETS_STANDARD_PHASE_END|RESET_OPPO_TURN"), None);
        assert_eq!(kw("RESET_PHASE|PHASE_END|RESET_SELF_TURN"), None);
        assert_eq!(kw("RESET_PHASE|PHASE_STANDBY"), None);
        assert_eq!(kw("RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_BATTLE"), None);
        assert_eq!(kw("RESET_EVENT|RESETS_STANDARD&~RESET_TOFIELD"), None);
        assert_eq!(kw("RESETS_STANDARD_EXC_GRAVE"), None);
        assert_eq!(kw("RESET_EVENT"), None);
        assert_eq!(kw("RESET_CHAIN"), None);
        // Reset counts above 1 shift the lifetime a full turn cycle
        // (`RESET_PHASE|PHASE_END, 2` = end of NEXT turn) — no keyword.
        assert_eq!(reset_to_duration_kw(Some("RESETS_STANDARD_PHASE_END"), Some("2")), None);
        assert_eq!(reset_to_duration_kw(Some("RESETS_STANDARD_PHASE_END"), Some("ct")), None);
        assert_eq!(
            reset_to_duration_kw(Some("RESETS_STANDARD_PHASE_END"), Some("1")),
            Some("end_of_turn"),
        );
        assert_eq!(reset_to_duration_kw(None, None), None);
    }

    #[test]
    fn modifier_chain_unmapped_reset_skips_not_permanent() {
        // A reset the mapping can't decode (battle-phase lifetime) must
        // skip the whole chain — dropping just the `until` clause would
        // turn a battle-phase buff into a permanent one.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(500)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_BATTLE)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        assert!(
            !lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("modify_atk"))),
            "unmapped reset must skip the chain, got: {:?}", lines,
        );
    }

    #[test]
    fn modifier_chain_standby_self_turn_reset() {
        // Fortissimo (c11493868) shape: RESET_PHASE|PHASE_STANDBY|
        // RESET_SELF_TURN → until next_standby_phase, not end_of_turn.
        let src = r#"
function s.atkop(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_STANDBY|RESET_SELF_TURN)
    e1:SetValue(800)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.atkop").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()), _ => None
        });
        assert_eq!(action, Some("modify_atk target + 800 until next_standby_phase"));
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
    fn t10_register_chain_set_attack_target_until_eot() {
        // tc:RegisterEffect with EFFECT_SET_ATTACK_FINAL + literal value +
        // PHASE_END reset → set_atk target <value> until end_of_turn.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_SET_ATTACK_FINAL)
    e1:SetValue(2500)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("set_atk target 2500 until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_set_defense_self_until_eot() {
        // c:RegisterEffect with EFFECT_SET_DEFENSE_FINAL → set_def self.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_SET_DEFENSE_FINAL)
    e1:SetValue(0)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
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
        assert_eq!(action, Some("set_def self 0 until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_pierce_self_grant() {
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_PIERCE)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
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
        assert_eq!(action, Some("grant self piercing until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_attack_all_target_grant() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_ATTACK_ALL)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("grant target attack_all_monsters until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_must_attack_target_grant() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_MUST_ATTACK)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("grant target must_attack until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_cannot_change_position_target_grant() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CANNOT_CHANGE_POSITION)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("grant target cannot_change_position until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_set_base_attack_target_until_eot() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_SET_BASE_ATTACK)
    e1:SetValue(1000)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("set_atk target 1000 until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_set_base_defense_self_until_eot() {
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_SET_BASE_DEFENSE)
    e1:SetValue(0)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
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
        assert_eq!(action, Some("set_def self 0 until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_extra_attack_value_1_double_attack() {
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_EXTRA_ATTACK)
    e1:SetValue(1)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
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
        assert_eq!(action, Some("grant self double_attack until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_extra_attack_value_2_triple_attack() {
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_EXTRA_ATTACK)
    e1:SetValue(2)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
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
        assert_eq!(action, Some("grant target triple_attack until end_of_turn"));
    }

    #[test]
    fn t10_register_chain_extra_attack_variable_value_skipped() {
        // Non-literal value → skip (no DSL form for dynamic extra_attack).
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local ct=2
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_EXTRA_ATTACK)
    e1:SetValue(ct)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.activate").expect("body");
        let lines = translate_body(&body);
        let has_grant = lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.contains("attack")));
        assert!(!has_grant, "variable EXTRA_ATTACK value should not emit");
    }

    #[test]
    fn t10_register_chain_indestructable_battle_phase_damage_end_of_damage_step() {
        // PHASE_DAMAGE reset → grant ... until end_of_damage_step
        // (instead of end_of_turn). Common shape on damage-step-only
        // battle-protection effects.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_DAMAGE)
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
        assert_eq!(action, Some("grant self cannot_be_destroyed by battle until end_of_damage_step"));
    }

    #[test]
    fn t10_register_chain_disable_target_negate_effects() {
        // EFFECT_DISABLE on target with end-of-turn reset → negate_effects.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_DISABLE)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("negate_effects target end_of_turn"));
    }

    #[test]
    fn t10_register_chain_disable_effect_skipped() {
        // EFFECT_DISABLE_EFFECT is the paired companion; we translate only
        // EFFECT_DISABLE to avoid duplicate negate_effects lines on the
        // common DISABLE+DISABLE_EFFECT pair.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_DISABLE_EFFECT)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let has_neg = lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("negate_effects")));
        assert!(!has_neg, "EFFECT_DISABLE_EFFECT alone must not emit a negate_effects line");
    }

    #[test]
    fn t10_register_chain_disable_skips_without_reset() {
        // No SetReset → permanent negate ambiguity → skip.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_DISABLE)
    tc:RegisterEffect(e1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove("s.operation").expect("body");
        let lines = translate_body(&body);
        let has_neg = lines.iter().any(|l| matches!(l, DslLine::Action(s) if s.starts_with("negate_effects")));
        assert!(!has_neg, "EFFECT_DISABLE without reset must not emit");
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
        // Filter must be mappable (Phase 10) — `nil` keeps the `card` kind.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(nil,tp,LOCATION_MZONE,LOCATION_MZONE,nil)
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

    // ── Phase 9 tests ──────────────────────────────────────────────────────

    #[test]
    fn phase9_sset_in_if_condition() {
        // Many Spell/Trap operation bodies use Duel.SSet as a boolean
        // expression inside an if condition:
        //   `if tc:IsRelateToEffect(e) and Duel.SSet(tp,tc)>0 then`
        // The stmt-level walker missed this; the expr walker must find it.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    if tc:IsRelateToEffect(e) and Duel.SSet(tp,tc)>0 then
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_IMMUNE_EFFECT)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("set target"),
            "Duel.SSet in if-condition should produce 'set target'; lines={:?}", lines);
    }

    #[test]
    fn phase9_special_summon_in_if_condition() {
        // Duel.SpecialSummon used as a boolean in an if condition.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    if c:IsRelateToEffect(e) and Duel.SpecialSummon(c,0,tp,tp,false,false,POS_FACEUP)>0 then
        Duel.RegisterFlagEffect(tp,id,RESET_PHASE|PHASE_END,0,1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.activate").expect("activate body");
        let lines = translate_body(body);
        let action = lines.iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(action, Some("special_summon self"),
            "Duel.SpecialSummon(c,...) in if-condition should produce 'special_summon self'; lines={:?}", lines);
    }

    #[test]
    fn phase9_register_flag_effect_skipped() {
        // Duel.RegisterFlagEffect is metadata — should be skipped (None),
        // not produce a TODO that would block the fill.
        let calls = vec![
            DuelCall {
                method: "Duel.RegisterFlagEffect".to_string(),
                args: vec!["tp".into(), "id".into(), "RESET_PHASE|PHASE_END".into(), "0".into(), "1".into()],
            },
            DuelCall {
                method: "Duel.HasFlagEffect".to_string(),
                args: vec!["tp".into(), "id".into()],
            },
            DuelCall {
                method: "Duel.GetFlagEffect".to_string(),
                args: vec!["tp".into(), "id".into()],
            },
        ];
        let lines = translate_calls(&calls);
        assert!(lines.is_empty(),
            "RegisterFlagEffect/HasFlagEffect/GetFlagEffect should all be skipped; got {:?}", lines);
    }

    #[test]
    fn phase9_equip_self_via_label_object() {
        // Equip cards often bind `local eqc = e:GetLabelObject()` and then
        // call `Duel.Equip(tp, eqc, tc, ...)` with `tc = Duel.GetFirstTarget()`.
        // With both provenances tracked this is "equip self to target".
        let body = FunctionBody {
            calls: vec![DuelCall {
                method: "Duel.Equip".to_string(),
                args: vec!["tp".into(), "eqc".into(), "tc".into(), "0".into()],
            }],
            value_bindings: BTreeMap::from([
                ("eqc".to_string(), "e:GetLabelObject()".to_string()),
                ("tc".to_string(), "Duel.GetFirstTarget()".to_string()),
            ]),
            ..FunctionBody::default()
        };
        let lines = translate_body(&body);
        assert_eq!(lines.len(), 1);
        assert!(matches!(&lines[0], DslLine::Action(s) if s == "equip self to target"),
            "eqc self-ref should translate to 'equip self to target'; got {:?}", lines[0]);
    }

    #[test]
    fn phase9_equip_name_alone_carries_no_provenance() {
        // Bare `ec`/`tc` names with no tracked assignment must NOT emit the
        // canonical line: an `ec` fed from SelectMatchingCard and a `tc` fed
        // from GetFirstCardTarget are different cards than self/target.
        let calls = vec![DuelCall {
            method: "Duel.Equip".to_string(),
            args: vec!["tp".into(), "ec".into(), "tc".into(), "0".into()],
        }];
        let lines = translate_calls(&calls);
        assert_eq!(lines.len(), 1);
        assert!(matches!(&lines[0], DslLine::Todo(_)),
            "unproven equip args should emit a TODO; got {:?}", lines[0]);
    }

    // ── Phase 11 tests — non-stat passive codes at resolve time ───────────

    /// Translate the named handler with the full function table in scope
    /// (immune-filter funcrefs resolve through it) and return the action
    /// lines.
    fn p11_actions(src: &str, handler: &str) -> Vec<String> {
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get(handler).expect("handler body");
        translate_body_with_functions(body, &report.functions)
            .into_iter()
            .filter_map(|l| match l {
                DslLine::Action(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn t11_change_attribute_literal_target() {
        // c70369116 shape: literal ATTRIBUTE_* on the resolved target.
        // The DSL change_attribute grammar has no duration clause — the
        // reset gates emission but is not rendered.
        let src = r#"
function s.atop(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    if tc:IsFaceup() and tc:IsRelateToEffect(e) then
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_CHANGE_ATTRIBUTE)
        e1:SetValue(ATTRIBUTE_DARK)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert_eq!(p11_actions(src, "s.atop"), vec!["change_attribute target to DARK"]);
    }

    #[test]
    fn t11_change_attribute_getlabel_skip() {
        // e:GetLabel() values are runtime-chosen (announce effects) — skip.
        let src = r#"
function s.attop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CHANGE_ATTRIBUTE)
    e1:SetValue(e:GetLabel())
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        assert!(p11_actions(src, "s.attop").is_empty(),
            "GetLabel attribute value must skip");
    }

    #[test]
    fn t11_change_race_literal_self() {
        // c9069157 shape: literal RACE_* on the card itself, with the
        // grammar-spelling token (RACE_DRAGON → Dragon).
        let src = r#"
function s.rcop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CHANGE_RACE)
    e1:SetValue(RACE_DRAGON)
    e1:SetReset(RESETS_STANDARD_DISABLE_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p11_actions(src, "s.rcop"), vec!["change_race self to Dragon"]);
    }

    #[test]
    fn t11_change_code_literal_via_name_table() {
        // c16828633 (Genex Spare) shape: literal passcode SetValue →
        // change_name with the CDB-resolved name and the reset duration.
        register_card_names([(68505803u32, "Genex Controller".to_string())]);
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    if not c:IsRelateToEffect(e) or c:IsFacedown() then return end
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CHANGE_CODE)
    e1:SetProperty(EFFECT_FLAG_CANNOT_DISABLE)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(68505803)
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p11_actions(src, "s.operation"),
            vec![r#"change_name self to "Genex Controller" until end_of_turn"#],
        );
    }

    #[test]
    fn t11_change_code_unknown_passcode_skip() {
        // Passcode absent from the name table — skip rather than emit a
        // numeric id the DSL string form can't represent.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CHANGE_CODE)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(999999991)
    c:RegisterEffect(e1)
end
"#;
        assert!(p11_actions(src, "s.operation").is_empty(),
            "unknown passcode must skip");
    }

    #[test]
    fn t11_change_code_method_value_skip() {
        // c2407234 shape: `local code=tc:GetOriginalCode()` — the name is
        // runtime data; DSL change_name needs a literal string. Skip.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local code=tc:GetOriginalCode()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_CHANGE_CODE)
    e1:SetValue(code)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        assert!(p11_actions(src, "s.operation").is_empty(),
            "method-call code value must skip");
    }

    #[test]
    fn t11_immune_spelltrap_two_grants() {
        // c26329679 shape: stock IsSpellTrapEffect filter → two grant
        // lines (the unaffected_by grammar takes one source token).
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_IMMUNE_EFFECT)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(s.efilter)
    tc:RegisterEffect(e1)
end
function s.efilter(e,te)
    return te:IsSpellTrapEffect()
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), vec![
            "grant target unaffected_by spells until end_of_turn",
            "grant target unaffected_by traps until end_of_turn",
        ]);
    }

    #[test]
    fn t11_immune_other_card_effects() {
        // c96434581 shape: owner-inequality stock filter ("other cards'
        // effects") → unaffected_by effects (corpus-precedent mapping).
        let src = r#"
function s.regop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_IMMUNE_EFFECT)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(s.efilter)
    c:RegisterEffect(e1)
end
function s.efilter(e,re)
    return e:GetHandler()~=re:GetOwner()
end
"#;
        assert_eq!(p11_actions(src, "s.regop"),
            vec!["grant self unaffected_by effects until end_of_turn"]);
    }

    #[test]
    fn t11_immune_opponent_effects() {
        // c4059313 shape: owner-player inequality → opponent_effects.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_IMMUNE_EFFECT)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(s.efilter)
    tc:RegisterEffect(e1)
end
function s.efilter(e,re)
    return e:GetOwnerPlayer()==1-re:GetOwnerPlayer()
end
"#;
        assert_eq!(p11_actions(src, "s.activate"),
            vec!["grant target unaffected_by opponent_effects until end_of_turn"]);
    }

    #[test]
    fn t11_immune_monster_plus_other_card() {
        // c52155219 shape: IsMonsterEffect + other-card qualifier →
        // unaffected_by monsters (other-card qualifier drops).
        let src = r#"
function s.immop(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_IMMUNE_EFFECT)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(s.efilter)
    tc:RegisterEffect(e1)
end
function s.efilter(e,te)
    return te:IsMonsterEffect() and te:GetOwner()~=e:GetOwner()
end
"#;
        assert_eq!(p11_actions(src, "s.immop"),
            vec!["grant target unaffected_by monsters until end_of_turn"]);
    }

    #[test]
    fn t11_immune_opponent_scoped_kind_skip() {
        // c79194594 shape: kind + opponent-player conjunct. The grammar
        // can't scope a kind to one player; either over-grant would
        // mis-emit, so skip.
        let src = r#"
function s.immop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_IMMUNE_EFFECT)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(s.imfilter1)
    c:RegisterEffect(e1)
end
function s.imfilter1(e,te)
    return te:IsMonsterEffect() and te:GetOwnerPlayer()~=e:GetHandlerPlayer()
end
"#;
        assert!(p11_actions(src, "s.immop").is_empty(),
            "player-scoped kind immunity must skip");
    }

    #[test]
    fn t11_immune_custom_filter_skip() {
        // c59765225 shape: IsActivated / GetControler conjuncts are not
        // stock forms — skip.
        let src = r#"
function s.atkop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_IMMUNE_EFFECT)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(s.immval)
    c:RegisterEffect(e1)
end
function s.immval(e,te)
    return te:GetOwner()~=e:GetHandler() and te:IsMonsterEffect() and te:IsActivated()
end
"#;
        assert!(p11_actions(src, "s.atkop").is_empty(),
            "non-stock immune filter must skip");
    }

    #[test]
    fn t11_immune_closure_value_skip() {
        // Inline closure SetValue — no function-table entry to resolve,
        // so the chain skips.
        let src = r#"
function s.regop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_IMMUNE_EFFECT)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    e1:SetValue(function(e,re) return e:GetHandler()~=re:GetOwner() end)
    c:RegisterEffect(e1)
end
"#;
        assert!(p11_actions(src, "s.regop").is_empty(),
            "inline-closure immune filter must skip");
    }

    #[test]
    fn t11_get_target_cards_loop_selector() {
        // c29726552 (Kumongous) shape: `local g=Duel.GetTargetCards(e)`
        // + aux.Next loop registering CANNOT_ATTACK / DISABLE per target.
        // The loop selector resolves, but `SetReset(…, 2)` is a TWO-turn
        // lifetime ("until the end of your next turn") — no DSL duration
        // keyword covers it, so every chain skips. (The old substring
        // mapping emitted `until end_of_turn`, a full turn short.)
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local g=Duel.GetTargetCards(e)
    local tc=g:GetFirst()
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(c)
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_CANNOT_ATTACK)
        e1:SetReset(RESETS_STANDARD_PHASE_END,2)
        tc:RegisterEffect(e1)
        local e2=Effect.CreateEffect(c)
        e2:SetType(EFFECT_TYPE_SINGLE)
        e2:SetCode(EFFECT_DISABLE)
        e2:SetReset(RESETS_STANDARD_PHASE_END,2)
        tc:RegisterEffect(e2)
        local e3=Effect.CreateEffect(c)
        e3:SetType(EFFECT_TYPE_SINGLE)
        e3:SetCode(EFFECT_DISABLE_EFFECT)
        e3:SetReset(RESETS_STANDARD_PHASE_END,2)
        tc:RegisterEffect(e3)
    end
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), Vec::<String>::new());
    }

    #[test]
    fn t11_branch_conditional_setvalue_skip() {
        // c88616795 (Spellbook of Wisdom) shape: SetValue differs per
        // if/else arm (player chooses spells-or-traps immunity). The
        // straight-line extractor keeps only the last write — emitting it
        // would hardcode one arm of a runtime choice. Must skip.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local c=e:GetHandler()
    if tc:IsRelateToEffect(e) and tc:IsFaceup() then
        local e1=Effect.CreateEffect(c)
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_IMMUNE_EFFECT)
        if e:GetLabel()==0 then
            e1:SetValue(s.efilter1)
        else
            e1:SetValue(s.efilter2)
        end
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
function s.efilter1(e,te)
    return te:IsSpellEffect() and te:GetOwner()~=e:GetOwner()
end
function s.efilter2(e,te)
    return te:IsTrapEffect()
end
"#;
        assert!(p11_actions(src, "s.activate").is_empty(),
            "branch-conditional SetValue must skip");
    }

    #[test]
    fn t11_event_group_loop_skip() {
        // Loop over `eg` (the raw event group) has no translatable
        // selector — must skip, not guess.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    for tc in aux.Next(eg) do
        local e1=Effect.CreateEffect(c)
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_CANNOT_ATTACK)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert!(p11_actions(src, "s.operation").is_empty(),
            "event-group loop must skip");
    }

    // ── Phase 12: parameterized fusion/ritual helper operations ─────

    /// Walk `src` and return the summon line of the `idx`-th effect.
    fn p12_line(src: &str, idx: usize) -> Option<String> {
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        report.effects.get(idx).expect("effect").summon_helper_line()
    }

    #[test]
    fn p12_unpack_table_archetype_and_onfield_mat() {
        // Fortissimo (c11493868): `local params={...}` +
        // `Fusion.SummonEffOP(table.unpack(params))`.
        let src = r#"
function s.initial_effect(c)
    local params={aux.FilterBoolFunction(Card.IsSetCard,SET_MELODIOUS),Fusion.OnFieldMat}
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_IGNITION)
    e3:SetTarget(Fusion.SummonEffTG(table.unpack(params)))
    e3:SetOperation(Fusion.SummonEffOP(table.unpack(params)))
    c:RegisterEffect(e3)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some(r#"fusion_summon (1, fusion monster, where archetype == "Melodious") using (all, monster, you control)"#),
        );
    }

    #[test]
    fn p12_positional_setcard_filter() {
        // Mementotlan Shleepy (c50042011): direct positional fusfilter.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(aux.FilterBoolFunction(Card.IsSetCard,SET_MEMENTO)))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some(r#"fusion_summon (1, fusion monster, where archetype == "Memento")"#),
        );
    }

    #[test]
    fn p12_inline_named_table_race_mask() {
        // Dracotail Faimena (c1498449): inline named-args table with an
        // OR'd race mask.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP({fusfilter=aux.FilterBoolFunction(Card.IsRace,RACE_DRAGON|RACE_SPELLCASTER)}))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("fusion_summon (1, fusion monster, where race == Dragon or race == Spellcaster)"),
        );
    }

    #[test]
    fn p12_closure_level_filter() {
        // Blazing Cartesia (c95515789): closure fusfilter on level.
        let src = r#"
function s.initial_effect(c)
    local params={function(c) return c:IsLevelAbove(8) end}
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(table.unpack(params)))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("fusion_summon (1, fusion monster, where level >= 8)"),
        );
    }

    #[test]
    fn p12_attribute_filter() {
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(aux.FilterBoolFunction(Card.IsAttribute,ATTRIBUTE_DARK)))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("fusion_summon (1, fusion monster, where attribute == DARK)"),
        );
    }

    #[test]
    fn p12_zero_args_plain_line() {
        // Favorite HERO Flame Wingman (c13243124): no params at all.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP())
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0).as_deref(), Some("fusion_summon (1, fusion monster)"));
    }

    #[test]
    fn p12_ident_resolving_to_plain_filter_local() {
        // Ukiyoe-P.U.N.K. Sharakusai (c13258285): `local fusparam=<filter>`
        // passed bare — a positional fusfilter, not a named-args table.
        let src = r#"
function s.initial_effect(c)
    local fusparam=aux.FilterBoolFunction(Card.IsSetCard,SET_PUNK)
    local e1=Effect.CreateEffect(c)
    e1:SetTarget(Fusion.SummonEffTG(fusparam))
    e1:SetOperation(Fusion.SummonEffOP(fusparam))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some(r#"fusion_summon (1, fusion monster, where archetype == "P.U.N.K.")"#),
        );
    }

    #[test]
    fn p12_ritual_named_table_with_handler_and_lvtype() {
        // Megalith Aratron (c25726386): named-args local table; the
        // `handler` key is a no-op for Ritual.Operation and RITPROC_GREATER
        // is the standard total-level procedure.
        let src = r#"
function s.initial_effect(c)
    local ritual_operation_params={handler=c,lvtype=RITPROC_GREATER,filter=function(ritual_c) return ritual_c:IsSetCard(SET_MEGALITH) end}
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Ritual.Operation(ritual_operation_params))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some(r#"ritual_summon (1, ritual monster, where archetype == "Megalith")"#),
        );
    }

    #[test]
    fn p12_skip_gc_param() {
        // D/D Swirl Slime shape: gc (5th positional) forces a specific
        // material — no DSL equivalent, whole line skips.
        let src = r#"
function s.initial_effect(c)
    local params={aux.FilterBoolFunction(Card.IsSetCard,SET_DDD),nil,nil,nil,Fusion.ForcedHandler}
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(table.unpack(params)))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn p12_skip_extraop_param() {
        // Banish-the-materials variant: extraop changes material
        // disposal — plain emit would be semantically wrong.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(nil,Card.IsAbleToRemove,nil,Fusion.BanishMaterial))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn p12_skip_mutated_param_table() {
        // Param table mutated after assignment — fails the taint check.
        let src = r#"
function s.initial_effect(c)
    local params={aux.FilterBoolFunction(Card.IsSetCard,SET_MELODIOUS)}
    params[2]=Fusion.OnFieldMat
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(table.unpack(params)))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn p12_skip_unknown_set_constant() {
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(aux.FilterBoolFunction(Card.IsSetCard,SET_NOT_IN_MAP)))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn p12_skip_compound_closure() {
        // Closure with extra logic beyond a single predicate.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(function(tc) return tc:IsSetCard(SET_NINJA) and tc:IsLevelAbove(4) end))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn p12_skip_ritual_forcedselection() {
        // Recette de Personnel shape: forcedselection has no DSL form.
        let src = r#"
function s.initial_effect(c)
    local rparams={filter=aux.FilterBoolFunction(Card.IsSetCard,SET_MEGALITH),forcedselection=s.fsel}
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Ritual.Operation(rparams))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn p12_helper_after_bare_activate_chain_is_hazard_gated() {
        // Frightfur Factory (c43698897): e1 is a bare EFFECT_TYPE_ACTIVATE
        // chain (continuous-spell activation shell) — it owns a .ds block
        // but consumes no Pass-A index, so the helper's positional block
        // mapping is off by one. Phase 20 retarget: the helper LINE now
        // decodes (`summon_helper_line` no longer swallows the hazard);
        // whether it may fill a block is decided by
        // `block_match::compute_assignments`, gated on the hazard flag
        // asserted here.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_ACTIVATE)
    e1:SetCode(EVENT_FREE_CHAIN)
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_IGNITION)
    e2:SetOperation(Fusion.SummonEffOP(aux.FilterBoolFunction(Card.IsSetCard,SET_FRIGHTFUR)))
    c:RegisterEffect(e2)
end
"#;
        assert_eq!(
            p12_line(src, 1).as_deref(),
            Some(r#"fusion_summon (1, fusion monster, where archetype == "Frightfur")"#),
        );
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        assert!(report.effects[1].block_alignment_hazard,
            "helper behind a bare-activate shell must stay hazard-flagged");
    }

    #[test]
    fn p12_skip_helper_after_clone_chain() {
        // Fluffal Owl (c65331686) shape: `local e2=e1:Clone()` owns a .ds
        // block but isn't walked — helper after it must skip.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_TRIGGER_O)
    e1:SetOperation(s.thop)
    c:RegisterEffect(e1)
    local e2=e1:Clone()
    e2:SetCode(EVENT_SPSUMMON_SUCCESS)
    c:RegisterEffect(e2)
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_IGNITION)
    e3:SetOperation(Fusion.SummonEffOP(aux.FilterBoolFunction(Card.IsSetCard,SET_FRIGHTFUR)))
    c:RegisterEffect(e3)
end
"#;
        assert_eq!(p12_line(src, 1), None);
    }

    #[test]
    fn p12_emit_helper_before_clone_chain() {
        // Clone chains AFTER the helper don't shift its block index —
        // the emit stands (Megalith Phaleg shape, c63233638).
        let src = r#"
function s.initial_effect(c)
    local params={lvtype=RITPROC_GREATER,filter=function(rc) return rc:IsSetCard(SET_MEGALITH) end}
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_QUICK_O)
    e1:SetOperation(Ritual.Operation(params))
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_SINGLE)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(500)
    c:RegisterEffect(e2)
    local e3=e2:Clone()
    c:RegisterEffect(e3)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some(r#"ritual_summon (1, ritual monster, where archetype == "Megalith")"#),
        );
    }

    #[test]
    fn p12_skip_unresolvable_ident() {
        // Ident with no local assignment in scope (module-level or
        // upvalue) — can't decode, must skip.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetOperation(Fusion.SummonEffOP(fusion_params))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    // ── Phase 13: counter ops ────────────────────────────────────────

    /// Translate the named handler body of a lua snippet and return the
    /// emitted ACTION lines (TODOs dropped, matching apply mode).
    fn p13_actions(src: &str, func: &str) -> Vec<String> {
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let Some(body) = report.functions.get(func) else { return Vec::new() };
        translate_body_with_functions(body, &report.functions)
            .into_iter()
            .filter_map(|l| match l {
                DslLine::Action(s) => Some(s),
                DslLine::Todo(_) => None,
            })
            .collect()
    }

    #[test]
    fn counter_add_on_self_hex_code_if_gated() {
        // Shark Caesar (c14306092) — the canonical relate/faceup gate.
        let src = r#"
function s.ctop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    if c:IsRelateToEffect(e) and c:IsFaceup() then
        c:AddCounter(0x2e,1)
    end
end
"#;
        assert_eq!(
            p13_actions(src, "s.ctop"),
            vec![r#"place_counter "Shark Counter" 1 on self"#],
        );
    }

    #[test]
    fn counter_add_on_target_named_const_multi_count() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    if tc and tc:IsRelateToEffect(e) and tc:IsFaceup() then
        tc:AddCounter(COUNTER_SPELL,2)
    end
end
"#;
        assert_eq!(
            p13_actions(src, "s.operation"),
            vec![r#"place_counter "Spell Counter" 2 on target"#],
        );
    }

    #[test]
    fn counter_add_need_enable_sum_resolves_base_counter() {
        // Cloudian idiom — COUNTER_NEED_ENABLE+COUNTER_FOG (0x2000+0x1019).
        let src = r#"
function s.addc(e,tp,eg,ep,ev,re,r,rp)
    e:GetHandler():AddCounter(COUNTER_NEED_ENABLE+COUNTER_FOG,2)
end
"#;
        assert_eq!(
            p13_actions(src, "s.addc"),
            vec![r#"place_counter "Fog Counter" 2 on self"#],
        );
    }

    #[test]
    fn counter_add_in_get_target_cards_loop_emits_on_target() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetTargetCards(e)
    for tc in aux.Next(g) do
        tc:AddCounter(0x1,1)
    end
end
"#;
        assert_eq!(
            p13_actions(src, "s.operation"),
            vec![r#"place_counter "Spell Counter" 1 on target"#],
        );
    }

    #[test]
    fn counter_remove_on_self_emits() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    if c:IsRelateToEffect(e) then
        c:RemoveCounter(tp,COUNTER_A,1,REASON_EFFECT)
    end
end
"#;
        assert_eq!(
            p13_actions(src, "s.operation"),
            vec![r#"remove_counter "A-Counter" 1 from self"#],
        );
    }

    #[test]
    fn counter_unknown_constant_skips() {
        // File-local constant (`local COUNTER_BES=0x1f`) — not resolved;
        // 0x1f itself names `Counter ("B.E.S.")`, unexpressible anyway.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    e:GetHandler():AddCounter(COUNTER_BES,1)
end
"#;
        assert!(p13_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn counter_quoted_strings_conf_name_skips() {
        // 0x1f → `Counter ("B.E.S.")` — embedded quotes can't live in a
        // DSL string literal.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    e:GetHandler():AddCounter(0x1f,1)
end
"#;
        assert!(p13_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn counter_variable_count_skips() {
        // Grammar slot is `unsigned` — no expr lowering for counts.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local ct=Duel.GetMatchingGroupCount(Card.IsFaceup,tp,LOCATION_MZONE,0,nil)
    e:GetHandler():AddCounter(0x1,ct)
end
"#;
        assert!(p13_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn counter_in_else_arm_poisons_body() {
        // Runtime either/or — emitting both arms (or either) mis-states
        // the card, so ALL counter ops in the body are dropped.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    if Duel.GetTurnPlayer()==tp then
        c:AddCounter(0x1,1)
    else
        c:AddCounter(0x1,2)
    end
end
"#;
        assert!(p13_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn counter_on_self_inside_group_loop_skips() {
        // Receiver is NOT the loop variable — the op hits the card once
        // per member, which the group-selector emit would mis-state.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetTargetCards(e)
    for tc in aux.Next(g) do
        e:GetHandler():AddCounter(0x1,1)
    end
end
"#;
        assert!(p13_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn counter_on_tc_not_bound_to_first_target_skips() {
        // Eternal Dread (c35787450): `tc` is the field-zone card via
        // Duel.GetFieldCard (and reassigned for the opponent's side) —
        // NOT a declared target. The bare-`tc` sentinel must not fire.
        let src = r#"
function s.addc(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFieldCard(tp,LOCATION_FZONE,0)
    if tc and tc:IsFaceup() and tc:IsCode(75041269) then
        tc:AddCounter(0x1b,2)
    end
    tc=Duel.GetFieldCard(1-tp,LOCATION_FZONE,0)
    if tc and tc:IsFaceup() and tc:IsCode(75041269) then
        tc:AddCounter(0x1b,2)
    end
end
"#;
        assert!(p13_actions(src, "s.addc").is_empty());
    }

    #[test]
    fn counter_gated_on_add_return_value_skips() {
        // `if c:AddCounter(...) then` — condition position, not a
        // statement; the gated follow-up isn't modeled, so nothing emits.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    if c:AddCounter(0x1,1) then
        Duel.Draw(tp,1,REASON_EFFECT)
    end
end
"#;
        let lines = p13_actions(src, "s.operation");
        assert!(!lines.iter().any(|l| l.starts_with("place_counter")), "{:?}", lines);
    }

    #[test]
    fn counter_remove_in_while_loop_skips() {
        // while tc do … GetNext idiom — untranslatable member set.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(Card.IsFaceup,tp,LOCATION_MZONE,0,nil)
    local tc=g:GetFirst()
    while tc do
        tc:AddCounter(0x1,1)
        tc=g:GetNext()
    end
end
"#;
        assert!(p13_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn duel_remove_counter_single_from_own_field_emits() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    Duel.RemoveCounter(tp,1,0,COUNTER_SPELL,1,REASON_EFFECT)
end
"#;
        assert_eq!(
            p13_actions(src, "s.operation"),
            vec![r#"remove_counter "Spell Counter" 1 from (1, card, you control)"#],
        );
    }

    #[test]
    fn duel_remove_counter_multi_count_skips() {
        // n > 1 may be spread across cards in lua; the DSL form pulls
        // the whole count from ONE selected card. Skip.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    Duel.RemoveCounter(tp,1,1,COUNTER_RESONANCE,3,REASON_EFFECT)
end
"#;
        assert!(p13_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn cost_counter_remove_from_self_extracts() {
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return e:GetHandler():IsCanRemoveCounter(tp,0x1,2,REASON_COST) end
    e:GetHandler():RemoveCounter(tp,0x1,2,REASON_COST)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let spec = extract_cost_block("s.cost", &report.functions).expect("cost spec");
        assert_eq!(
            spec.actions,
            vec![CostAction::RemoveCounter("Spell Counter".into(), 2, "self".into())],
        );
    }

    #[test]
    fn cost_duel_remove_counter_field_extracts() {
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return Duel.IsCanRemoveCounter(tp,1,0,COUNTER_SPELL,1,REASON_COST) end
    Duel.RemoveCounter(tp,1,0,COUNTER_SPELL,1,REASON_COST)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let spec = extract_cost_block("s.cost", &report.functions).expect("cost spec");
        assert_eq!(
            spec.actions,
            vec![CostAction::RemoveCounter(
                "Spell Counter".into(), 1, "(1, card, you control)".into(),
            )],
        );
    }

    #[test]
    fn cost_branch_nested_counter_remove_bails() {
        // Conditional payment is not a fixed cost — whole block bails.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return true end
    if Duel.SelectYesNo(tp,aux.Stringid(id,0)) then
        e:GetHandler():RemoveCounter(tp,0x1,1,REASON_COST)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        assert!(extract_cost_block("s.cost", &report.functions).is_none());
    }

    #[test]
    fn cost_add_counter_bails() {
        // AddCounter is not a payment shape we model in cost context.
        let src = r#"
function s.cost(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return true end
    e:GetHandler():AddCounter(0x1,1)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        assert!(extract_cost_block("s.cost", &report.functions).is_none());
    }

    #[test]
    fn plain_helper_fusion_no_params_emits_bare_line() {
        let src = r#"
function s.initial_effect(c)
    local e1=Fusion.CreateSummonEff(c)
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("fusion_summon (1, fusion monster)"),
        );
    }

    #[test]
    fn plain_helper_fusion_fusfilter_emits_where_clause() {
        let src = r#"
function s.initial_effect(c)
    local e1=Fusion.CreateSummonEff(c,aux.FilterBoolFunction(Card.IsSetCard,SET_SHADDOLL))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some(r#"fusion_summon (1, fusion monster, where archetype == "Shaddoll")"#),
        );
    }

    #[test]
    fn plain_helper_fusion_named_table_fusfilter() {
        let src = r#"
function s.initial_effect(c)
    local e1=Fusion.CreateSummonEff{handler=c,fusfilter=aux.FilterBoolFunction(Card.IsRace,RACE_FIEND)}
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("fusion_summon (1, fusion monster, where race == Fiend)"),
        );
    }

    #[test]
    fn plain_helper_fusion_undecodable_extrafil_skips() {
        // extrafil widens the material pool — no DSL equivalent, so the
        // old bare-line emit would over-permit. Must skip.
        let src = r#"
function s.initial_effect(c)
    local e1=Fusion.CreateSummonEff(c,aux.FilterBoolFunction(Card.IsRace,RACE_DRAGON),nil,s.fextra)
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn plain_helper_fusion_inline_register_decodes() {
        let src = r#"
function s.initial_effect(c)
    c:RegisterEffect(Fusion.CreateSummonEff(c,aux.FilterBoolFunction(Card.IsSetCard,SET_MELODIOUS)))
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some(r#"fusion_summon (1, fusion monster, where archetype == "Melodious")"#),
        );
    }

    #[test]
    fn plain_helper_ritual_addproc_attribute_filter() {
        let src = r#"
function s.initial_effect(c)
    Ritual.AddProcEqual(c,aux.FilterBoolFunction(Card.IsAttribute,ATTRIBUTE_LIGHT))
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("ritual_summon (1, ritual monster, where attribute == LIGHT)"),
        );
    }

    #[test]
    fn plain_helper_ritual_addproc_explicit_level_skips() {
        // An explicit lv overrides the summoned monster's own level in
        // the tribute check — not expressible, must skip.
        let src = r#"
function s.initial_effect(c)
    Ritual.AddProcGreater(c,aux.FilterBoolFunction(Card.IsAttribute,ATTRIBUTE_LIGHT),8)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn plain_helper_ritual_addproc_code_variant_skips() {
        // *Code variants restrict the ritual target to specific card
        // codes — not expressible as a DSL where-clause yet.
        let src = r#"
function s.initial_effect(c)
    Ritual.AddProcGreaterCode(c,3,nil,99414168)
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn plain_helper_ritual_createproc_desc_only_emits_bare_line() {
        // desc is cosmetic; lvtype GREATER is the helper's standard
        // procedure — the bare line is correct here.
        let src = r#"
function s.initial_effect(c)
    local e1=Ritual.CreateProc(c,RITPROC_GREATER,nil,nil,aux.Stringid(id,1))
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("ritual_summon (1, ritual monster)"),
        );
    }

    #[test]
    fn plain_helper_ritual_addproc_assigned_table_filter_emits() {
        // c14735698 shape (minus extrafil): the assigned brace-table form
        // previously produced NO skeleton, shifting every later block one
        // slot too early in apply.
        let src = r#"
function s.initial_effect(c)
    local e1=Ritual.AddProcEqual{handler=c,filter=aux.FilterBoolFunction(Card.IsSetCard,SET_SHADDOLL)}
    e1:SetCountLimit(1,id)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_IGNITION)
    e2:SetOperation(s.thop)
    c:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        assert_eq!(report.effects.len(), 2);
        assert!(report.effects[0].ritual_summon_spec);
        assert_eq!(report.effects[1].binding, "e2");
        assert_eq!(
            report.effects[0].summon_helper_line().as_deref(),
            Some(r#"ritual_summon (1, ritual monster, where archetype == "Shaddoll")"#),
        );
    }

    #[test]
    fn plain_helper_ritual_addproc_assigned_paren_table_stage2_skips_line() {
        // c12157563 (Forbidden Ritual Art): paren-wrapped named table with
        // stage2/location — no DSL equivalent, so the line skips, but the
        // skeleton must still hold its block slot ahead of e2.
        let src = r#"
function s.initial_effect(c)
    local e1=Ritual.AddProcEqual({handler=c,location=LOCATION_GRAVE,stage2=s.stage2})
    e1:SetCountLimit(1,{id,0})
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_QUICK_O)
    e2:SetCode(EVENT_FREE_CHAIN)
    e2:SetCost(Cost.SelfBanish)
    e2:SetOperation(s.effop)
    c:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        assert_eq!(report.effects.len(), 2);
        assert!(report.effects[0].ritual_summon_spec);
        assert_eq!(report.effects[0].summon_helper_line(), None);
        assert_eq!(report.effects[1].binding, "e2");
    }

    #[test]
    fn plain_helper_ritual_whole_level_tribute_assigned_unresolved() {
        // Assigned AddWholeLevelTribute changes the tribute procedure —
        // unresolved op: skeleton keeps alignment, resolve stays empty.
        let src = r#"
function s.initial_effect(c)
    local e1=Ritual.AddWholeLevelTribute(c,s.tributable)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_IGNITION)
    e2:SetOperation(s.thop)
    c:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        assert_eq!(report.effects.len(), 2);
        assert!(report.effects[0].ritual_summon_spec);
        assert!(report.effects[0].summon_helper_op.as_ref().unwrap().unresolved);
        assert_eq!(report.effects[0].summon_helper_line(), None);
        assert_eq!(report.effects[1].binding, "e2");
    }

    #[test]
    fn plain_helper_fusion_parenthesized_table_stage2_skips() {
        // Dark Fusion (c94820406): named-args table wrapped in parens —
        // the table is arg #1, not the handler. stage2 grants the
        // summoned monster effects, so the line must skip; the old
        // positional decode skipped the table as "handler" and mis-emitted
        // a bare line.
        let src = r#"
function s.initial_effect(c)
    c:RegisterEffect(Fusion.CreateSummonEff({handler=c,fusfilter=aux.FilterBoolFunction(Card.IsRace,RACE_FIEND),stage2=s.stage2}))
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn plain_helper_fusion_parenthesized_table_fusfilter_emits() {
        let src = r#"
function s.initial_effect(c)
    local e1=Fusion.CreateSummonEff({handler=c,fusfilter=aux.FilterBoolFunction(Card.IsRace,RACE_FIEND)})
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("fusion_summon (1, fusion monster, where race == Fiend)"),
        );
    }

    #[test]
    fn plain_helper_fusion_register_no_params_emits_bare_line() {
        // Polymerization (c24094653): top-level statement form, no
        // params beyond the handler.
        let src = r#"
function s.initial_effect(c)
    Fusion.RegisterSummonEff(c)
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some("fusion_summon (1, fusion monster)"),
        );
    }

    #[test]
    fn plain_helper_fusion_register_table_extra_params_skip() {
        // Greater Polymerization (c7614732): mincount changes the
        // material count floor and stage2 grants summoned-monster
        // effects — no DSL equivalent, must skip.
        let src = r#"
function s.initial_effect(c)
    Fusion.RegisterSummonEff{handler=c,mincount=3,stage2=s.stage2}
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn plain_helper_fusion_register_binding_form_fusfilter() {
        // Binding form registers internally — no `c:RegisterEffect(e1)`
        // follows, the skeleton must still count as registered.
        let src = r#"
function s.initial_effect(c)
    local e1=Fusion.RegisterSummonEff{handler=c,fusfilter=aux.FilterBoolFunction(Card.IsSetCard,SET_SHADDOLL)}
end
"#;
        assert_eq!(
            p12_line(src, 0).as_deref(),
            Some(r#"fusion_summon (1, fusion monster, where archetype == "Shaddoll")"#),
        );
    }

    #[test]
    fn plain_helper_fusion_register_binding_extrafil_skips() {
        // Dimension Fusion Destruction (c89190953): extrafil widens the
        // material pool, extraop banishes materials — must skip even
        // though fusfilter alone would decode.
        let src = r#"
function s.initial_effect(c)
    local e1=Fusion.RegisterSummonEff{handler=c,fusfilter=aux.FilterBoolFunction(Card.IsSetCard,SET_PHANTASM),extrafil=s.fextra,stage2=s.stage2}
end
"#;
        assert_eq!(p12_line(src, 0), None);
    }

    #[test]
    fn plain_helper_after_bare_activate_chain_keeps_hazard_flag() {
        // Frightfur Factory shape: a bare EFFECT_TYPE_ACTIVATE chain
        // precedes the helper chain, so positional block mapping is
        // off-by-one. Phase 20 retarget: `summon_helper_line` no longer
        // swallows the hazard itself — the line decodes, and the skip
        // (or a signature-forced rescue) is decided by
        // `block_match::compute_assignments`, which gates every fill on
        // the hazard flag asserted here.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_ACTIVATE)
    e1:SetCode(EVENT_FREE_CHAIN)
    c:RegisterEffect(e1)
    local e2=Fusion.CreateSummonEff(c,aux.FilterBoolFunction(Card.IsSetCard,SET_FRIGHTFUR))
    c:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let helper = report
            .effects
            .iter()
            .find(|e| e.is_summon_helper())
            .expect("helper skeleton");
        assert!(helper.block_alignment_hazard,
            "helper after a bare-activate chain must stay hazard-flagged");
        assert!(helper.summon_helper_line().is_some(),
            "the line itself decodes — the block gate lives in the matcher");
    }

    // ── Phase 13b: chain-path `tc` → `target` gated on GetFirstTarget ──

    /// Minimal single-target chain body parameterized by the lines that
    /// precede the RegisterEffect — used to vary how `tc` is bound.
    fn p13b_src(prelude: &str, receiver: &str) -> String {
        format!(r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    {}
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(500)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    {}:RegisterEffect(e1)
end
"#, prelude, receiver)
    }

    #[test]
    fn p13b_tc_getfirsttarget_emits_target() {
        // Canonical declared-target shape — gate passes.
        let actions = p11_actions(&p13b_src("local tc=Duel.GetFirstTarget()", "tc"), "s.activate");
        assert_eq!(actions, vec!["modify_atk target + 500 until end_of_turn"]);
    }

    #[test]
    fn p13b_tc_fieldcard_binding_skips() {
        // Eternal Dread (c35787450) shape: `tc` is the field-zone card,
        // not a declared target — `target` would mis-aim the line.
        let actions = p11_actions(
            &p13b_src("local tc=Duel.GetFieldCard(tp,LOCATION_FZONE,0)", "tc"),
            "s.activate");
        assert!(actions.is_empty(), "GetFieldCard-bound tc must skip, got {:?}", actions);
    }

    #[test]
    fn p17_tc_selectmatchingcard_binding_folds_selector() {
        // Phase 13b skipped this provenance (a resolve-time selection is
        // not the declared target). Phase 17 folds the one-card selection
        // into the line's selector instead — the selection-implying
        // selector aims the chain at the freshly picked card.
        let actions = p11_actions(
            &p13b_src(
                "local tc=Duel.SelectMatchingCard(tp,s.filter,tp,LOCATION_MZONE,0,1,1,nil):GetFirst()",
                "tc"),
            "s.activate");
        assert_eq!(
            actions,
            vec!["modify_atk (1, card, you control, from monster_zone) + 500 until end_of_turn"],
        );
    }

    #[test]
    fn p13b_tc_unbound_skips() {
        // No `tc` assignment in the body (upvalue / module-level) —
        // provenance unknown, skip.
        let actions = p11_actions(&p13b_src("", "tc"), "s.activate");
        assert!(actions.is_empty(), "unbound tc must skip, got {:?}", actions);
    }

    #[test]
    fn p13b_tc_rebound_skips() {
        // GetFirstTarget then reassigned — taint logic drops the binding,
        // so the gate must skip (single-assignment requirement).
        let actions = p11_actions(
            &p13b_src(
                "local tc=Duel.GetFirstTarget()\n    tc=Duel.GetAttackTarget()",
                "tc"),
            "s.activate");
        assert!(actions.is_empty(), "rebound tc must skip, got {:?}", actions);
    }

    #[test]
    fn p13b_getfirst_receiver_gettargetcards_emits_target() {
        // `g:GetFirst()` where g is the declared-target group — the first
        // declared target, still `target`.
        let actions = p11_actions(
            &p13b_src("local g=Duel.GetTargetCards(e)", "g:GetFirst()"),
            "s.activate");
        assert_eq!(actions, vec!["modify_atk target + 500 until end_of_turn"]);
    }

    #[test]
    fn p13b_getfirst_receiver_selecttarget_skips() {
        // c67901914 shape: group from Duel.SelectTarget — receiver is a
        // fresh selection, not the chain's declared target. Skip.
        let actions = p11_actions(
            &p13b_src("local g=Duel.SelectTarget(tp,s.filter,tp,LOCATION_MZONE,0,1,1,nil)", "g:GetFirst()"),
            "s.activate");
        assert!(actions.is_empty(), "SelectTarget group GetFirst must skip, got {:?}", actions);
    }

    // ── Base-stat tokens: GetBaseAttack/GetBaseDefense ≠ atk/def ──────

    /// Translate `handler` of `src` and return the first Action line.
    fn first_action(src: &str, handler: &str) -> Option<String> {
        let parsed = full_moon::parse(src).expect("parse");
        let body = walk(&parsed).functions.remove(handler).expect("body");
        translate_body(&body).into_iter().find_map(|l| match l {
            DslLine::Action(s) => Some(s),
            _ => None,
        })
    }

    #[test]
    fn base_stat_getbaseattack_emits_base_atk() {
        // c67901914 e1 shape: SET_ATTACK_FINAL valued at
        // tc:GetBaseAttack()*2 — printed/base ATK doubled, NOT the
        // current ATK (`target.atk * 2` drifts once ATK is modified).
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_SET_ATTACK_FINAL)
    e1:SetValue(tc:GetBaseAttack()*2)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        assert_eq!(
            first_action(src, "s.activate").as_deref(),
            Some("set_atk target target.base_atk * 2 until end_of_turn"),
        );
    }

    #[test]
    fn base_stat_getbasedefense_self_emits_base_def() {
        // c39343610 shape: self-targeted SET_DEFENSE_FINAL from
        // c:GetBaseDefense()*2.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_SET_DEFENSE_FINAL)
    e1:SetValue(c:GetBaseDefense()*2)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            first_action(src, "s.operation").as_deref(),
            Some("set_def self self.base_def * 2 until end_of_turn"),
        );
    }

    #[test]
    fn base_stat_getattack_still_emits_atk() {
        // Regression guard: the current-value getter keeps the plain
        // `atk` token (c99634927 atkop3 shape — UPDATE_ATTACK by
        // c:GetAttack() registered on the target).
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(c:GetAttack())
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        assert_eq!(
            first_action(src, "s.operation").as_deref(),
            Some("modify_atk target + self.atk until end_of_turn"),
        );
    }

    #[test]
    fn base_stat_binding_indirection_resolves_base_atk() {
        // c77205367 shape: base ATK captured into a local, then
        // SetValue(<local>) — one level of value_bindings indirection.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local atk=tc:GetBaseAttack()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(atk)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#;
        assert_eq!(
            first_action(src, "s.operation").as_deref(),
            Some("modify_atk self + target.base_atk until end_of_turn"),
        );
    }

    // ── Clone-seeded slot overrides: linear Clone()+Set* is not a conflict ──

    #[test]
    fn clone_override_emits_both_chains() {
        // c67901914 e2 shape: e2=e1:Clone() inherits e1's slots, then
        // SetCode/SetValue override them. Linear and deterministic — the
        // override replaces the inherited value; no branch conflict.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_SET_ATTACK_FINAL)
    e1:SetValue(tc:GetBaseAttack()*2)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
    local e2=e1:Clone()
    e2:SetCode(EFFECT_SET_DEFENSE_FINAL)
    e2:SetValue(tc:GetBaseDefense()*2)
    tc:RegisterEffect(e2)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert_eq!(actions, vec![
            "set_atk target target.base_atk * 2 until end_of_turn",
            "set_def target target.base_def * 2 until end_of_turn",
        ]);
    }

    #[test]
    fn clone_override_then_branch_set_still_conflicts() {
        // The seed mark is consumed by the FIRST override per slot — a
        // later differing write on the same slot is a real branch
        // conflict and the clone's chain must skip.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_SET_ATTACK_FINAL)
    e1:SetValue(tc:GetBaseAttack()*2)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
    local e2=e1:Clone()
    e2:SetCode(EFFECT_SET_DEFENSE_FINAL)
    if tc:IsAttackPos() then
        e2:SetValue(tc:GetBaseDefense()*2)
    else
        e2:SetValue(0)
    end
    tc:RegisterEffect(e2)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert_eq!(
            actions,
            vec!["set_atk target target.base_atk * 2 until end_of_turn"],
            "clone with branch-conditional SetValue must skip, got {:?}", actions,
        );
    }

    #[test]
    fn plain_branch_double_set_still_conflicts() {
        // Regression guard for d5f6f551b's predecessor (d5d637700): a
        // non-clone chain whose slot is written differently in two branch
        // arms is one arm of a runtime choice — must skip.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    if tc:IsAttackPos() then
        e1:SetValue(500)
    else
        e1:SetValue(-500)
    end
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "branch double-set must skip, got {:?}", actions);
    }

    #[test]
    fn clone_in_initial_effect_seeds_real_skeleton() {
        // c99634927 shape: e3=e2:Clone() inherits e2's chain, overrides
        // SetCode, and registers. Phase 14 seeds a real skeleton — e3
        // enters walk.effects with e2's handlers and its own code, the
        // positional block mapping realigns, and e4 carries no hazard.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_IGNITION)
    e1:SetRange(LOCATION_PZONE)
    e1:SetOperation(s.atkop1)
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_SINGLE+EFFECT_TYPE_TRIGGER_O)
    e2:SetCode(EVENT_SUMMON_SUCCESS)
    e2:SetTarget(s.atktg2)
    e2:SetOperation(s.atkop2)
    c:RegisterEffect(e2)
    local e3=e2:Clone()
    e3:SetCode(EVENT_SPSUMMON_SUCCESS)
    c:RegisterEffect(e3)
    local e4=Effect.CreateEffect(c)
    e4:SetType(EFFECT_TYPE_IGNITION)
    e4:SetRange(LOCATION_MZONE)
    e4:SetOperation(s.atkop3)
    c:RegisterEffect(e4)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let summary: Vec<(String, bool)> = report.effects.iter()
            .map(|e| (e.binding.clone(), e.block_alignment_hazard))
            .collect();
        assert_eq!(summary, vec![
            ("e1".to_string(), false),
            ("e2".to_string(), false),
            ("e3".to_string(), false),
            ("e4".to_string(), false),
        ]);
        let e3 = &report.effects[2];
        assert_eq!(e3.operation_handler.as_deref(), Some("s.atkop2"),
            "clone must inherit the source's operation handler");
        assert_eq!(e3.target_handler.as_deref(), Some("s.atktg2"),
            "clone must inherit the source's target handler");
        assert_eq!(e3.first_arg_of("SetCode"), Some("EVENT_SPSUMMON_SUCCESS"),
            "SetCode on the clone must override the inherited entry");
        assert_eq!(
            e3.set_calls.iter().filter(|(m, _)| m == "SetCode").count(), 1,
            "override must replace, not shadow, the inherited SetCode",
        );
    }

    #[test]
    fn unregistered_clone_still_flags_later_skeletons() {
        // A clone that never reaches `c:RegisterEffect(eN)` drops out of
        // walk.effects but may still own a .ds block — later skeletons
        // must keep the conservative hazard skip.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_IGNITION)
    e1:SetOperation(s.op1)
    c:RegisterEffect(e1)
    local e2=e1:Clone()
    e2:SetCode(EVENT_SPSUMMON_SUCCESS)
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_IGNITION)
    e3:SetOperation(s.op3)
    c:RegisterEffect(e3)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let summary: Vec<(String, bool)> = report.effects.iter()
            .map(|e| (e.binding.clone(), e.block_alignment_hazard))
            .collect();
        assert_eq!(summary, vec![
            ("e1".to_string(), false),
            ("e3".to_string(), true),
        ]);
    }

    #[test]
    fn passive_clone_seeds_overridden_stat() {
        // e2=e1:Clone() + SetCode(EFFECT_UPDATE_DEFENSE) on a passive ATK
        // chain: the seeded skeleton must yield a DEF passive with the
        // inherited value, not a duplicate ATK one.
        let src = r#"
function s.initial_effect(c)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(500)
    c:RegisterEffect(e1)
    local e2=e1:Clone()
    e2:SetCode(EFFECT_UPDATE_DEFENSE)
    c:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        assert_eq!(report.effects.len(), 2);
        let spec1 = report.effects[0].passive_modifier_spec().expect("e1 spec");
        let spec2 = report.effects[1].passive_modifier_spec().expect("e2 spec");
        assert_eq!((spec1.stat.as_str(), spec1.value.as_str()), ("atk", "500"));
        assert_eq!((spec2.stat.as_str(), spec2.value.as_str()), ("def", "500"));
    }

    #[test]
    fn p15_equip_chain_without_producer_skips() {
        // EFFECT_TYPE_EQUIP chain but NO Duel.Equip in the handler —
        // nothing produces the `__equipped__` binding the equipped_card
        // selector reads back, so emission would aim at a stale or
        // missing card. Skip.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_EQUIP)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(400)
    e2:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e2)
    local e3=e2:Clone()
    e3:SetCode(EFFECT_UPDATE_DEFENSE)
    c:RegisterEffect(e3)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "no-producer EQUIP chains must skip, got {:?}", actions);
    }

    #[test]
    fn p15_equip_stat_chains_emit_equipped_card() {
        // Tyrant Wing (c57470761) shape: Duel.Equip then EQUIP-type
        // UPDATE_ATTACK / UPDATE_DEFENSE / EXTRA_ATTACK_MONSTER chains
        // registered on the equip card. RESETS_STANDARD without PHASE_END
        // expires with the equip relation → `while_equipped`.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    Duel.Equip(tp,c,tc)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_EQUIP)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(400)
    e2:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e2)
    local e3=e2:Clone()
    e3:SetCode(EFFECT_UPDATE_DEFENSE)
    c:RegisterEffect(e3)
    local e4=Effect.CreateEffect(c)
    e4:SetType(EFFECT_TYPE_EQUIP)
    e4:SetCode(EFFECT_EXTRA_ATTACK_MONSTER)
    e4:SetValue(1)
    e4:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e4)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert_eq!(actions, vec![
            "equip self to target".to_string(),
            "modify_atk equipped_card + 400 until while_equipped".to_string(),
            "modify_def equipped_card + 400 until while_equipped".to_string(),
            "grant equipped_card double_attack until while_equipped".to_string(),
        ]);
    }

    #[test]
    fn p15_equip_disable_and_cannot_attack() {
        // Old Entity Hastorr (c70913714) shape: equip self to the
        // destroyer, negate its effects and forbid attacking — both
        // EQUIP-type chains, lifetime of the equip relation.
        let src = r#"
function s.eqop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    Duel.Equip(tp,c,tc)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_EQUIP)
    e1:SetCode(EFFECT_DISABLE)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_EQUIP)
    e2:SetCode(EFFECT_CANNOT_ATTACK)
    e2:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e2)
end
"#;
        let actions = p11_actions(src, "s.eqop");
        assert_eq!(actions, vec![
            "equip self to target".to_string(),
            "negate_effects equipped_card while_equipped".to_string(),
            "grant equipped_card cannot_attack until while_equipped".to_string(),
        ]);
    }

    #[test]
    fn p15_equip_chain_phase_end_reset_wins() {
        // An explicit PHASE_END reset bounds the boost to the turn even
        // while the equip persists — end_of_turn beats while_equipped.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    Duel.Equip(tp,c,tc)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_EQUIP)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(500)
    e2:SetReset(RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_END)
    c:RegisterEffect(e2)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert_eq!(actions, vec![
            "equip self to target".to_string(),
            "modify_atk equipped_card + 500 until end_of_turn".to_string(),
        ]);
    }

    #[test]
    fn p15_equip_chain_on_target_receiver_skips() {
        // EQUIP-type chain registered on `tc` instead of the equip card —
        // outside the canonical shape, receiver semantics unclear. Skip
        // the chain but keep the equip line itself.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    Duel.Equip(tp,c,tc)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_EQUIP)
    e2:SetCode(EFFECT_UPDATE_ATTACK)
    e2:SetValue(400)
    e2:SetReset(RESET_EVENT|RESETS_STANDARD)
    tc:RegisterEffect(e2)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert_eq!(actions, vec!["equip self to target".to_string()]);
    }

    #[test]
    fn p15_equip_chain_non_literal_value_skips() {
        // Metalmorph's second boost (half the attack target's ATK) is
        // computed at battle time — no literal SetValue, no DSL lowering.
        // The literal +300/+300 pair still emits.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    Duel.Equip(tp,c,tc)
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_EQUIP)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(300)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e1)
    local e2=e1:Clone()
    e2:SetCode(EFFECT_UPDATE_DEFENSE)
    c:RegisterEffect(e2)
    local e3=Effect.CreateEffect(c)
    e3:SetType(EFFECT_TYPE_EQUIP)
    e3:SetCode(EFFECT_UPDATE_ATTACK)
    e3:SetValue(s.atkval)
    e3:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e3)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert_eq!(actions, vec![
            "equip self to target".to_string(),
            "modify_atk equipped_card + 300 until while_equipped".to_string(),
            "modify_def equipped_card + 300 until while_equipped".to_string(),
        ]);
    }

    // ── Phase 15: field-scope restriction chains (Duel.RegisterEffect) ──

    #[test]
    fn p15_field_indestructable_battle_no_filter() {
        // c12607053 shape: all your monsters survive battle this turn.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["grant (all, card, you control, from monster_zone) cannot_be_destroyed by battle until end_of_turn"],
        );
    }

    #[test]
    fn p15_field_named_filter_faceup_archetype() {
        // c19333131 (Ninjitsu Art of Decoy-B) shape: named s.* filter
        // resolved through the function table; conjunction of IsFaceup
        // + IsSetCard maps to a where-clause.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetTarget(s.etarget)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.etarget(e,c)
    return c:IsFaceup() and c:IsSetCard(SET_NINJA)
end
"#;
        assert_eq!(
            p11_actions(src, "s.operation"),
            vec![r#"grant (all, card, you control, from monster_zone, where is_face_up and archetype == "Ninja") cannot_be_destroyed by battle until end_of_turn"#],
        );
    }

    #[test]
    fn p15_field_closure_filter_attribute_race() {
        // c26434972 shape: inline (e,c) closure with attribute + race
        // conjunction.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetTarget(function(e,c) return c:IsAttribute(ATTRIBUTE_LIGHT) and c:IsRace(RACE_FIEND) end)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["grant (all, card, you control, from monster_zone, where attribute == LIGHT and race == Fiend) cannot_be_destroyed by battle until end_of_turn"],
        );
    }

    #[test]
    fn p15_field_targetboolfunction_archetype() {
        // c48608796 (Lyrilusc) shape: aux.TargetBoolFunction partial
        // application as the SetTarget filter.
        let src = r#"
function s.indop(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetTarget(aux.TargetBoolFunction(Card.IsSetCard,SET_LYRILUSC))
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.indop"),
            vec![r#"grant (all, card, you control, from monster_zone, where archetype == "Lyrilusc") cannot_be_destroyed by battle until end_of_turn"#],
        );
    }

    #[test]
    fn p15_field_level_attribute_two_chains() {
        // c7935043 (Umiiruka-style) shape: two chains sharing one filter;
        // level/attribute predicates; both grants emit.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetTarget(s.tg)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=e1:Clone()
    e2:SetCode(EFFECT_INDESTRUCTABLE_EFFECT)
    Duel.RegisterEffect(e2,tp)
end
function s.tg(e,c)
    return c:IsLevelBelow(3) and c:IsAttribute(ATTRIBUTE_WATER)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec![
                "grant (all, card, you control, from monster_zone, where level <= 3 and attribute == WATER) cannot_be_destroyed by battle until end_of_turn",
                "grant (all, card, you control, from monster_zone, where level <= 3 and attribute == WATER) cannot_be_destroyed by effect until end_of_turn",
            ],
        );
    }

    #[test]
    fn p15_field_disable_negate_effects() {
        // c12181376 e-chain shape: EFFECT_DISABLE on the opponent's
        // spell/trap zone with an IsTrap filter → negate_effects.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_DISABLE)
    e1:SetTargetRange(0,LOCATION_SZONE)
    e1:SetTarget(s.distg)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.distg(e,c)
    return c:IsTrap()
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["negate_effects (all, trap, opponent controls, from spell_trap_zone) end_of_turn"],
        );
    }

    #[test]
    fn p15_field_both_sides_xyz_rank() {
        // c10282757 e-chain shape: both monster zones, frame-type +
        // rank predicates.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_CANNOT_ATTACK)
    e2:SetTargetRange(LOCATION_MZONE,LOCATION_MZONE)
    e2:SetTarget(s.filter2)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
function s.filter2(e,c)
    return c:IsType(TYPE_XYZ) and c:IsRankAbove(4)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["grant (all, card, either controls, from monster_zone, where is_xyz and rank >= 4) cannot_attack until end_of_turn"],
        );
    }

    #[test]
    fn p15_skip_player_flag_range() {
        // EFFECT_CANNOT_ACTIVATE with player-flag TargetRange(0,1) and a
        // named `s.aclimit` filter that ISN'T defined in the script —
        // the Phase 18 restrict path can't classify the activation
        // filter, so the chain must still skip.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ACTIVATE)
    e1:SetTargetRange(0,1)
    e1:SetValue(s.aclimit)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "player-scoped field code must skip, got {:?}", actions);
    }

    #[test]
    fn p15_skip_damage_shaping_code() {
        // EFFECT_AVOID_BATTLE_DAMAGE with an aux.ChangeBattleDamage-style
        // helper value — a damage-shaping code the Phase 15 card-scoped
        // path must not touch, and whose exotic value keeps it outside
        // the Phase 19 damage_rule path too.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_AVOID_BATTLE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(aux.ChangeBattleDamage(1,0))
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "damage-shaping field code must skip, got {:?}", actions);
    }

    #[test]
    fn p15_skip_reset_count_two() {
        // c16469012 shape: SetReset(RESET_PHASE|PHASE_END,2) lives until
        // the SECOND end phase — not an end-of-turn lifetime.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ATTACK)
    e1:SetTargetRange(0,LOCATION_MZONE)
    e1:SetReset(RESET_PHASE|PHASE_END,2)
    Duel.RegisterEffect(e1,tp)
end
"#;
        let actions = p11_actions(src, "s.operation");
        assert!(actions.is_empty(), "reset count 2 must skip, got {:?}", actions);
    }

    #[test]
    fn p15_skip_except_target_filter() {
        // c19394153 family: `e:GetLabel()~=c:GetFieldID()` filter is the
        // "every monster EXCEPT the selected one" idiom — no DSL
        // qualifier exists (grammar backlog).
        let src = r#"
function s.atkop(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ATTACK)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetTarget(s.ftarget)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.ftarget(e,c)
    return e:GetLabel()~=c:GetFieldID()
end
"#;
        let actions = p11_actions(src, "s.atkop");
        assert!(actions.is_empty(), "except-target filter must skip, got {:?}", actions);
    }

    #[test]
    fn p15_skip_qualifier_value_indoval() {
        // c72318602 shape: SetValue(aux.indoval) qualifies the protection
        // to the OPPONENT's effects only — emitting the unqualified
        // ability would over-grant.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_INDESTRUCTABLE_EFFECT)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetValue(aux.indoval)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        let actions = p11_actions(src, "s.operation");
        assert!(actions.is_empty(), "qualifier-fn SetValue must skip, got {:?}", actions);
    }

    #[test]
    fn p15_skip_register_player_not_tp() {
        // Hieroglyph-style `Duel.RegisterEffect(e1,p)` — the range decode
        // is relative to an unknown player; controller would flip.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local p=Duel.GetChainInfo(0,CHAININFO_TARGET_PLAYER)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,p)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "non-tp registration must skip, got {:?}", actions);
    }

    #[test]
    fn p15_skip_card_registered_field_chain() {
        // FIELD chain committed via c:RegisterEffect — lifetime tied to
        // the card. Must NOT fall through to the receiver-based grant
        // path (`grant self …` would mis-aim a TargetRange(0,MZONE)
        // restriction on the opponent's monsters).
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ATTACK)
    e1:SetTargetRange(0,LOCATION_MZONE)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD)
    c:RegisterEffect(e1)
end
"#;
        let actions = p11_actions(src, "s.operation");
        assert!(actions.is_empty(), "card-registered FIELD chain must skip, got {:?}", actions);
    }

    #[test]
    fn p15_field_watcher_still_emits_install_watcher() {
        // Regression guard: FIELD+CONTINUOUS watcher chains (EVENT codes
        // via Duel.RegisterEffect) must still flow to install_watcher.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD+EFFECT_TYPE_CONTINUOUS)
    e1:SetCode(EVENT_DESTROYED)
    e1:SetReset(RESET_PHASE|PHASE_END)
    e1:SetOperation(s.deop)
    Duel.RegisterEffect(e1,tp)
end
function s.deop(e,tp,eg,ep,ev,re,r,rp)
    Duel.Draw(tp,1,REASON_EFFECT)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert_eq!(
            actions,
            vec![r#"install_watcher "deop" { event: destroyed duration: end_of_turn check { draw 1 } }"#],
        );
    }

    #[test]
    fn p15_skip_choice_arm_chain() {
        // c55948544 (Judgment of the Pharaoh) shape: the chain sits in
        // one arm of an if/elseif SelectEffect-label dispatch — emitting
        // it unconditionally would state one option as THE effect.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local op=e:GetLabel()
    if op==1 then
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_FIELD)
        e1:SetCode(EFFECT_INDESTRUCTABLE_BATTLE)
        e1:SetTargetRange(LOCATION_MZONE,0)
        e1:SetValue(1)
        e1:SetReset(RESET_PHASE|PHASE_END)
        Duel.RegisterEffect(e1,tp)
    elseif op==2 then
        local e2=Effect.CreateEffect(e:GetHandler())
        e2:SetType(EFFECT_TYPE_FIELD)
        e2:SetCode(EFFECT_DISABLE)
        e2:SetTargetRange(0,LOCATION_SZONE)
        e2:SetReset(RESET_PHASE|PHASE_END)
        Duel.RegisterEffect(e2,tp)
    end
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "choice-arm chains must skip, got {:?}", actions);
    }

    #[test]
    fn p15_plain_if_guard_still_emits() {
        // The ubiquitous plain-if guard (no else) is NOT a choice —
        // field-scope chains inside it keep emitting.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    if Duel.GetTurnPlayer()==tp then
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_FIELD)
        e1:SetCode(EFFECT_PIERCE)
        e1:SetTargetRange(LOCATION_MZONE,0)
        e1:SetReset(RESET_PHASE|PHASE_END)
        Duel.RegisterEffect(e1,tp)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["grant (all, card, you control, from monster_zone) piercing until end_of_turn"],
        );
    }

    #[test]
    fn p15_skip_spell_trap_subtype_mask() {
        // c12253117 shape: IsType(TYPE_FIELD) — spell/trap subtype masks
        // have no is_* predicate; whole filter fails.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_DISABLE)
    e1:SetTargetRange(LOCATION_SZONE,LOCATION_SZONE)
    e1:SetTarget(s.distarget)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.distarget(e,c)
    return c:IsType(TYPE_FIELD)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "subtype mask filter must skip, got {:?}", actions);
    }

    // ── Phase 18: player-scoped restriction chains → restrict ──────

    #[test]
    fn p18_restrict_special_summon_you() {
        // The dominant corpus shape (Cyber-Stein-tax style): the
        // activating player cannot special summon for the turn —
        // CANNOT_SPECIAL_SUMMON, no value, no target filter.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetProperty(EFFECT_FLAG_PLAYER_TARGET)
    e1:SetCode(EFFECT_CANNOT_SPECIAL_SUMMON)
    e1:SetTargetRange(1,0)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["restrict you cannot_special_summon end_of_turn"],
        );
    }

    #[test]
    fn p18_restrict_both_players_and_normal_summon() {
        // (1,1) player flags → both_players; CANNOT_SUMMON →
        // cannot_normal_summon; the literal flag SetValue(1) is accepted.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_SUMMON)
    e1:SetTargetRange(1,1)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.operation"),
            vec!["restrict both_players cannot_normal_summon end_of_turn"],
        );
    }

    #[test]
    fn p18_restrict_activate_spells_traps_named_filter() {
        // The `s.aclimit` idiom: named filter resolved through the
        // function table; IsHasType(EFFECT_TYPE_ACTIVATE) = spell/trap
        // CARD activations.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ACTIVATE)
    e1:SetTargetRange(0,1)
    e1:SetValue(s.aclimit)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.aclimit(e,re,tp)
    return re:IsHasType(EFFECT_TYPE_ACTIVATE)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["restrict opponent cannot_activate_spells_traps end_of_turn"],
        );
    }

    #[test]
    fn p18_restrict_activate_monster_effects_inline_closure() {
        // Inline `(_,re)` closure form of the monster-effect filter.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ACTIVATE)
    e1:SetTargetRange(0,1)
    e1:SetValue(function(_,re) return re:IsMonsterEffect() end)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["restrict opponent cannot_activate_monster_effects end_of_turn"],
        );
    }

    #[test]
    fn p18_restrict_activate_traps_conjunction() {
        // Trap-card activation filter: GetHandler():IsTrap() AND
        // IsHasType(EFFECT_TYPE_ACTIVATE) — both conjuncts required,
        // either order.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ACTIVATE)
    e1:SetTargetRange(1,1)
    e1:SetValue(s.aclimit)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.aclimit(e,re,tp)
    return re:GetHandler():IsTrap() and re:IsHasType(EFFECT_TYPE_ACTIVATE)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["restrict both_players cannot_activate_traps end_of_turn"],
        );
    }

    #[test]
    fn p18_restrict_activate_bare_flag() {
        // SetValue(1) on CANNOT_ACTIVATE = no card may be activated at
        // all → the bare cannot_activate keyword.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ACTIVATE)
    e1:SetTargetRange(0,1)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.operation"),
            vec!["restrict opponent cannot_activate end_of_turn"],
        );
    }

    #[test]
    fn p18_restrict_battle_phase_codes() {
        // CANNOT_BP / SKIP_BP map without a value; two chains, one line
        // each.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_BP)
    e1:SetTargetRange(1,0)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_SKIP_BP)
    e2:SetTargetRange(0,1)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec![
                "restrict you cannot_conduct_battle_phase end_of_turn",
                "restrict opponent skip_battle_phase end_of_turn",
            ],
        );
    }

    #[test]
    fn p18_skip_filtered_special_summon() {
        // The dominant SKIP class: a SetTarget summon-limit filter
        // ("cannot special summon, except Zombie monsters") qualifies
        // WHICH cards — the unfiltered keyword would over-restrict.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_SPECIAL_SUMMON)
    e1:SetTargetRange(1,0)
    e1:SetTarget(s.splimit)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.splimit(e,c)
    return not c:IsRace(RACE_ZOMBIE)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "summon-limit filter must skip, got {:?}", actions);
    }

    #[test]
    fn p18_skip_exotic_activate_filter() {
        // Attribute-qualified activation filter (Mulcharmy-style) — the
        // extra conjunct is outside the four T36 shapes.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ACTIVATE)
    e1:SetTargetRange(0,1)
    e1:SetValue(function(_,re) return re:IsMonsterEffect() and re:GetHandler():IsAttributeExcept(ATTRIBUTE_FIRE) end)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "exotic activation filter must skip, got {:?}", actions);
    }

    #[test]
    fn p18_skip_condition_gated() {
        // SetCondition turns the restriction into a floodgate gated on
        // game state — the unconditional restrict line would mis-state it.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_SKIP_BP)
    e1:SetTargetRange(1,0)
    e1:SetCondition(s.skipcon)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.skipcon(e)
    return Duel.GetTurnPlayer()==e:GetHandlerPlayer()
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "condition-gated chain must skip, got {:?}", actions);
    }

    #[test]
    fn p18_skip_two_turn_and_turn_qualified_resets() {
        // SetReset(...,2) lives until the SECOND end phase, and a
        // RESET_SELF_TURN qualifier is not end-of-CURRENT-turn — both
        // are outside the standard reset class.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_SPECIAL_SUMMON)
    e1:SetTargetRange(1,0)
    e1:SetReset(RESET_PHASE|PHASE_END,2)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_CANNOT_SPECIAL_SUMMON)
    e2:SetTargetRange(1,0)
    e2:SetReset(RESET_PHASE|PHASE_END|RESET_SELF_TURN,2)
    Duel.RegisterEffect(e2,tp)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "non-standard resets must skip, got {:?}", actions);
    }

    #[test]
    fn p18_skip_choice_arm_watcher_with_restrict_check() {
        // c84117021 (Spell Wall) shape: an EVENT watcher chain inside one
        // arm of an if/else whose check body is a now-translatable
        // restrict line. The install_watcher path must honor the
        // choice-arm poison like every other chain family.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local opt=Duel.GetChainInfo(0,CHAININFO_TARGET_PARAM)
    if opt==0 then
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_FIELD)
        e1:SetCode(EFFECT_CANNOT_DISABLE_SUMMON)
        e1:SetReset(RESET_PHASE|PHASE_END)
        Duel.RegisterEffect(e1,tp)
    else
        local tc=Duel.GetFirstTarget()
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE+EFFECT_TYPE_CONTINUOUS)
        e1:SetCode(EVENT_ATTACK_ANNOUNCE)
        e1:SetOperation(s.atkop)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
function s.atkop(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_ACTIVATE)
    e1:SetTargetRange(0,1)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_DAMAGE)
    Duel.RegisterEffect(e1,tp)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "choice-arm watcher must skip, got {:?}", actions);
    }

    #[test]
    fn p18_skip_code_outside_vocabulary() {
        // EFFECT_CANNOT_DRAW is a player-scoped code but NOT in the
        // 11-keyword T36 vocabulary — must skip, not mis-map.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CANNOT_DRAW)
    e1:SetTargetRange(0,1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "out-of-vocabulary code must skip, got {:?}", actions);
    }

    // ── Phase 19: damage-shaping chains → damage_rule ───────

    #[test]
    fn p19_damage_rule_no_damage_you() {
        // The plainest shape: CHANGE_DAMAGE + SetValue(0) on the
        // activating player — "you take no damage this turn".
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetProperty(EFFECT_FLAG_PLAYER_TARGET)
    e1:SetCode(EFFECT_CHANGE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(0)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["damage_rule you no_damage end_of_turn"],
        );
    }

    #[test]
    fn p19_damage_rule_no_effect_damage_named_guard() {
        // The dominant guard-fn shape: named value fn with the if/else
        // REASON_EFFECT→0 body, resolved through guarded_return.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetProperty(EFFECT_FLAG_PLAYER_TARGET)
    e1:SetCode(EFFECT_CHANGE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(s.damval)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.damval(e,re,val,r,rp,rc)
    if (r&REASON_EFFECT)~=0 then return 0 else return val end
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["damage_rule you no_effect_damage end_of_turn"],
        );
    }

    #[test]
    fn p19_damage_rule_no_effect_damage_trailing_return_and_ternary() {
        // Guard-body spelling variants: the `if … then return 0 end
        // return dam` trailing-return form (with the `dam` param name),
        // and the inverted single-expr ternary
        // `(r&REASON_EFFECT)==0 and val or 0`.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CHANGE_DAMAGE)
    e1:SetTargetRange(0,1)
    e1:SetValue(s.damval)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_CHANGE_DAMAGE)
    e2:SetTargetRange(1,0)
    e2:SetValue(s.damval2)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
function s.damval(e,re,dam,r,rp,rc)
    if r&REASON_EFFECT~=0 then return 0 end
    return dam
end
function s.damval2(e,re,val,r,rp,rc)
    return (r&REASON_EFFECT)==0 and val or 0
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec![
                "damage_rule opponent no_effect_damage end_of_turn",
                "damage_rule you no_effect_damage end_of_turn",
            ],
        );
    }

    #[test]
    fn p19_damage_rule_halve_effect_damage_guard() {
        // REASON_EFFECT-guarded halving (floored spelling) →
        // halve_effect_damage.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CHANGE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(s.val)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.val(e,re,dam,r,rp,rc)
    if r&REASON_EFFECT~=0 then return math.floor(dam/2) else return dam end
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["damage_rule you halve_effect_damage end_of_turn"],
        );
    }

    #[test]
    fn p19_damage_rule_no_battle_damage_flag_spellings() {
        // AVOID_BATTLE_DAMAGE with SetValue(1) and with no value are the
        // same flag shape — both emit, each scope.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_AVOID_BATTLE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_AVOID_BATTLE_DAMAGE)
    e2:SetTargetRange(1,1)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec![
                "damage_rule you no_battle_damage end_of_turn",
                "damage_rule both_players no_battle_damage end_of_turn",
            ],
        );
    }

    #[test]
    fn p19_damage_rule_halve_and_double_battle_damage() {
        // CHANGE_BATTLE_DAMAGE keys on the two blessed constants.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CHANGE_BATTLE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(HALF_DAMAGE)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_CHANGE_BATTLE_DAMAGE)
    e2:SetTargetRange(0,1)
    e2:SetValue(DOUBLE_DAMAGE)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec![
                "damage_rule you halve_battle_damage end_of_turn",
                "damage_rule opponent double_battle_damage end_of_turn",
            ],
        );
    }

    #[test]
    fn p19_damage_rule_reverse_variants() {
        // REVERSE_DAMAGE: flag 1 → reverse_damage; a boolean
        // REASON_EFFECT guard (single-return named fn) →
        // reverse_effect_damage.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_REVERSE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_REVERSE_DAMAGE)
    e2:SetTargetRange(1,0)
    e2:SetValue(s.rev)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
function s.rev(e,re,val,r,rp,rc)
    return (r&REASON_EFFECT)~=0
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec![
                "damage_rule you reverse_damage end_of_turn",
                "damage_rule you reverse_effect_damage end_of_turn",
            ],
        );
    }

    #[test]
    fn p19_damage_rule_reflect_effect_damage_rp_guard() {
        // REFLECT_DAMAGE needs BOTH conjuncts of the doc-table guard:
        // effect reason AND rp == opponent of the handler player.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_REFLECT_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(s.refcon)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
function s.refcon(e,re,val,r,rp,rc)
    return (r&REASON_EFFECT)~=0 and rp==1-e:GetHandlerPlayer()
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["damage_rule you reflect_effect_damage end_of_turn"],
        );
    }

    #[test]
    fn p19_damage_rule_reflect_battle_damage_valueless() {
        // REFLECT_BATTLE_DAMAGE with no SetValue — the corpus majority
        // spelling of the flag shape.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_REFLECT_BATTLE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["damage_rule you reflect_battle_damage end_of_turn"],
        );
    }

    #[test]
    fn p19_skip_unguarded_halve_and_bare_reflect_guard() {
        // Two near-miss guards that must skip: an UNGUARDED halve
        // (halves ALL damage — no T37 keyword; Butterspy Protection
        // shape) and a REFLECT guard missing the rp conjunct (reflects
        // even self-inflicted effect damage — not the doc-table shape).
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CHANGE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(function(e,re,val,r,rp,rc) return math.floor(val/2) end)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_REFLECT_DAMAGE)
    e2:SetTargetRange(1,0)
    e2:SetValue(function(e,_,_,r) return (r&REASON_EFFECT)==REASON_EFFECT end)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "near-miss guards must skip, got {:?}", actions);
    }

    #[test]
    fn p19_skip_chain_id_and_side_effect_guards() {
        // The chain-id "that damage" family (GetCurrentChain /
        // CHAININFO label matching) and guards with statement side
        // effects are multi-statement bodies — neither return_expr nor
        // guarded_return captures them, so the chain skips.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CHANGE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(s.damval)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_CHANGE_DAMAGE)
    e2:SetTargetRange(1,0)
    e2:SetValue(s.counterval)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
function s.damval(e,re,val,r,rp,rc)
    local cc=Duel.GetCurrentChain()
    if cc==0 or (r&REASON_EFFECT)==0 then return val end
    local cid=Duel.GetChainInfo(0,CHAININFO_CHAIN_ID)
    if cid~=e:GetLabel() then return val end
    return 0
end
function s.counterval(e,re,val,r,rp,rc)
    if (r&REASON_EFFECT)~=0 then e:GetHandler():AddCounter(COUNTER_FEATHER,1) return 0 end
    return val
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "chain-id / side-effect guards must skip, got {:?}", actions);
    }

    #[test]
    fn p19_skip_owner_qualified_guard_and_fixed_value() {
        // An rp-qualifier OUTSIDE the accepted atoms
        // (`rp~=e:GetOwnerPlayer()` — only opponent-inflicted effect
        // damage becomes 0) and an arbitrary fixed replacement value
        // both fall outside the closed set.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CHANGE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(s.damval)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_CHANGE_BATTLE_DAMAGE)
    e2:SetTargetRange(1,0)
    e2:SetValue(400)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
function s.damval(e,re,val,r,rp,rc)
    if (r&REASON_EFFECT)~=0 and rp~=e:GetOwnerPlayer() then return 0 else return val end
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "qualified guard / fixed value must skip, got {:?}", actions);
    }

    #[test]
    fn p19_skip_card_scoped_range_and_condition_gate() {
        // A LOCATION mask in SetTargetRange is a card-scoped chain (the
        // Phase 15 family), and a SetCondition floodgate mis-states the
        // rule as unconditional — both skip.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_AVOID_BATTLE_DAMAGE)
    e1:SetTargetRange(LOCATION_MZONE,0)
    e1:SetValue(1)
    e1:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e1,tp)
    local e2=Effect.CreateEffect(e:GetHandler())
    e2:SetType(EFFECT_TYPE_FIELD)
    e2:SetCode(EFFECT_CHANGE_DAMAGE)
    e2:SetTargetRange(1,0)
    e2:SetValue(0)
    e2:SetCondition(s.con)
    e2:SetReset(RESET_PHASE|PHASE_END)
    Duel.RegisterEffect(e2,tp)
end
function s.con(e)
    return Duel.GetTurnPlayer()==e:GetHandlerPlayer()
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "card-scoped range / condition gate must skip, got {:?}", actions);
    }

    #[test]
    fn p19_skip_two_turn_reset() {
        // SetReset(...,2) lives until the SECOND end phase — outside
        // the standard reset class.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_FIELD)
    e1:SetCode(EFFECT_CHANGE_DAMAGE)
    e1:SetTargetRange(1,0)
    e1:SetValue(0)
    e1:SetReset(RESET_PHASE|PHASE_END,2)
    Duel.RegisterEffect(e1,tp)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "two-turn reset must skip, got {:?}", actions);
    }

    // ── Phase 16: SelectOption label-branches → choose ──────

    fn p16_spec(src: &str, target_fn: &str, op_fn: &str) -> Option<ChooseSpec> {
        let parsed = full_moon::parse(src).expect("lua parse");
        let report = walk(&parsed);
        let tb = report.functions.get(target_fn);
        let ob = report.functions.get(op_fn).expect("op body");
        extract_choose_spec(tb, ob, &report.functions)
    }

    fn p16_options(spec: &ChooseSpec) -> Vec<(u32, Vec<String>)> {
        spec.options.iter().map(|(idx, lines)| {
            (*idx, lines.iter().map(|l| match l {
                DslLine::Action(s) => s.clone(),
                DslLine::Todo(s) => panic!("unexpected TODO in choose arm: {}", s),
            }).collect())
        }).collect()
    }

    const P16_TARGET_2OPT: &str = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return true end
    local op=Duel.SelectOption(tp,aux.Stringid(id,1),aux.Stringid(id,2))
    e:SetLabel(op)
end
"#;

    #[test]
    fn p16_happy_two_arm_actions() {
        let src = format!("{}{}", P16_TARGET_2OPT, r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local sel=e:GetLabel()
    if sel==0 then
        Duel.Draw(tp,1,REASON_EFFECT)
    else
        Duel.Damage(1-tp,500,REASON_EFFECT)
    end
end
"#);
        let spec = p16_spec(&src, "s.target", "s.activate").expect("choose spec");
        assert_eq!(p16_options(&spec), vec![
            (1, vec!["draw 1".to_string()]),
            (2, vec!["damage opponent 500".to_string()]),
        ]);
    }

    #[test]
    fn p16_three_arm_with_chain_and_getlabel_inline() {
        // Three Strikes Barrier shape: inline `e:GetLabel()==0` head (no
        // local), elseif ladder, register-chain arm. The chain arm reuses
        // the Phase 15 field-scope emit.
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return true end
    local op=Duel.SelectOption(tp,aux.Stringid(id,0),aux.Stringid(id,1),aux.Stringid(id,2))
    e:SetLabel(op)
end
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    if e:GetLabel()==0 then
        Duel.Draw(tp,1,REASON_EFFECT)
    elseif e:GetLabel()==1 then
        Duel.Damage(1-tp,800,REASON_EFFECT)
    else
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_FIELD)
        e1:SetCode(EFFECT_PIERCE)
        e1:SetTargetRange(LOCATION_MZONE,0)
        e1:SetReset(RESET_PHASE|PHASE_END)
        Duel.RegisterEffect(e1,tp)
    end
end
"#;
        let spec = p16_spec(src, "s.target", "s.activate").expect("choose spec");
        assert_eq!(p16_options(&spec), vec![
            (0, vec!["draw 1".to_string()]),
            (1, vec!["damage opponent 800".to_string()]),
            (2, vec!["grant (all, card, you control, from monster_zone) piercing until end_of_turn".to_string()]),
        ]);
    }

    #[test]
    fn p16_counter_arm() {
        let src = format!("{}{}", P16_TARGET_2OPT, r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local sel=e:GetLabel()
    if sel==0 then
        c:AddCounter(COUNTER_SPELL,2)
    else
        Duel.Draw(tp,1,REASON_EFFECT)
    end
end
"#);
        let spec = p16_spec(&src, "s.target", "s.activate").expect("choose spec");
        assert_eq!(p16_options(&spec), vec![
            (1, vec![r#"place_counter "Spell Counter" 2 on self"#.to_string()]),
            (2, vec!["draw 1".to_string()]),
        ]);
    }

    #[test]
    fn p16_skip_select_effect() {
        // SelectEffect label dispatch is NOT a SelectOption — no option
        // descriptions to label a choose with. Phase 15's in_choice_arm
        // guard keeps suppressing the arms on the normal resolve path.
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return true end
    local op=Duel.SelectEffect(tp,aux.Stringid(id,1),aux.Stringid(id,2))
    e:SetLabel(op)
end
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local sel=e:GetLabel()
    if sel==0 then
        Duel.Draw(tp,1,REASON_EFFECT)
    else
        Duel.Damage(1-tp,500,REASON_EFFECT)
    end
end
"#;
        assert!(p16_spec(src, "s.target", "s.activate").is_none());
    }

    #[test]
    fn p16_skip_dynamic_option_list() {
        // Junk Changer (c1006081) shape: the option list depends on the
        // selected monster's level — two SelectOption calls in branches.
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return true end
    local g=Duel.SelectTarget(tp,s.filter,tp,LOCATION_MZONE,LOCATION_MZONE,1,1,nil)
    local op=0
    if g:GetFirst():GetLevel()==1 then
        op=Duel.SelectOption(tp,aux.Stringid(id,1))
    else
        op=Duel.SelectOption(tp,aux.Stringid(id,1),aux.Stringid(id,2))
    end
    e:SetLabel(op)
end
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local sel=e:GetLabel()
    if sel==0 then
        Duel.Draw(tp,1,REASON_EFFECT)
    else
        Duel.Damage(1-tp,500,REASON_EFFECT)
    end
end
"#;
        assert!(p16_spec(src, "s.target", "s.activate").is_none());
    }

    #[test]
    fn p16_skip_untranslatable_arm() {
        // One arm holds a call with no DSL mapping (TODO) — the whole
        // card skips rather than emitting a half-true option.
        let src = format!("{}{}", P16_TARGET_2OPT, r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local sel=e:GetLabel()
    if sel==0 then
        Duel.Draw(tp,1,REASON_EFFECT)
    else
        Duel.MoveSequence(c,1)
    end
end
"#);
        assert!(p16_spec(&src, "s.target", "s.activate").is_none());
    }

    #[test]
    fn p16_skip_label_value_use() {
        // Mermail Abyssmander (c21767650) shape: the label feeds
        // SetValue(opt+1) inside a group loop — no statement-level arms.
        let src = format!("{}{}", P16_TARGET_2OPT, r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local opt=e:GetLabel()
    local g=Duel.GetMatchingGroup(s.filter,tp,LOCATION_MZONE,0,nil)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_LEVEL)
        e1:SetValue(opt+1)
        e1:SetReset(RESET_EVENT|RESETS_STANDARD)
        tc:RegisterEffect(e1)
    end
end
"#);
        assert!(p16_spec(&src, "s.target", "s.activate").is_none());
    }

    #[test]
    fn p16_skip_select_yes_no() {
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return true end
    local op=Duel.SelectYesNo(tp,aux.Stringid(id,0))
    e:SetLabel(op)
end
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local sel=e:GetLabel()
    if sel==0 then
        Duel.Draw(tp,1,REASON_EFFECT)
    else
        Duel.Damage(1-tp,500,REASON_EFFECT)
    end
end
"#;
        assert!(p16_spec(src, "s.target", "s.activate").is_none());
    }

    #[test]
    fn p16_skip_arm_count_mismatch() {
        // Three options picked, but only two dispatch arms — the third
        // option's behavior is unrepresented.
        let src = r#"
function s.target(e,tp,eg,ep,ev,re,r,rp,chk)
    if chk==0 then return true end
    local op=Duel.SelectOption(tp,aux.Stringid(id,0),aux.Stringid(id,1),aux.Stringid(id,2))
    e:SetLabel(op)
end
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local sel=e:GetLabel()
    if sel==0 then
        Duel.Draw(tp,1,REASON_EFFECT)
    else
        Duel.Damage(1-tp,500,REASON_EFFECT)
    end
end
"#;
        assert!(p16_spec(src, "s.target", "s.activate").is_none());
    }

    #[test]
    fn p16_skip_action_outside_dispatch() {
        // An unconditional action before the dispatch would be dropped
        // by the choose emit — the whole body must reject.
        let src = format!("{}{}", P16_TARGET_2OPT, r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    Duel.Draw(tp,1,REASON_EFFECT)
    local sel=e:GetLabel()
    if sel==0 then
        Duel.Recover(tp,500,REASON_EFFECT)
    else
        Duel.Damage(1-tp,500,REASON_EFFECT)
    end
end
"#);
        assert!(p16_spec(&src, "s.target", "s.activate").is_none());
    }

    #[test]
    fn p16_dispatch_inside_relate_guard() {
        // Wind-Up-style wrapper: the dispatch sits inside the ubiquitous
        // IsRelateToEffect plain-if guard — still extracts.
        let src = format!("{}{}", P16_TARGET_2OPT, r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    if c:IsFaceup() and c:IsRelateToEffect(e) then
        local sel=e:GetLabel()
        if sel==0 then
            Duel.Draw(tp,1,REASON_EFFECT)
        else
            Duel.Damage(1-tp,500,REASON_EFFECT)
        end
    end
end
"#);
        let spec = p16_spec(&src, "s.target", "s.activate").expect("choose spec");
        assert_eq!(spec.options.len(), 2);
    }

    #[test]
    fn p16_slot_fork_value() {
        // Wind-Up Shark (c25484449) value-fork shape: the dispatch arms
        // only fork ONE Set* slot of a chain under construction — each
        // option becomes the chain with that arm's writes applied.
        let src = format!("{}{}", P16_TARGET_2OPT, r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_LEVEL)
    if e:GetLabel()==0 then
        e1:SetValue(1)
    else
        e1:SetValue(-1)
    end
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    c:RegisterEffect(e1)
end
"#);
        let spec = p16_spec(&src, "s.target", "s.activate").expect("choose spec");
        assert_eq!(p16_options(&spec), vec![
            (1, vec!["modify_level self + 1 until end_of_turn".to_string()]),
            (2, vec!["modify_level self - 1 until end_of_turn".to_string()]),
        ]);
    }

    #[test]
    fn p16_inline_select_option_code_fork() {
        // Armored Kappa (c50789693) shape: the SelectOption sits in the
        // OPERATION body (no SetLabel hop) and the dispatch forks the
        // chain's SetCode slot. No target handler involved.
        let src = r#"
function s.adop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    if c:IsRelateToEffect(e) and c:IsFaceup() then
        local opt=Duel.SelectOption(tp,aux.Stringid(id,2),aux.Stringid(id,3))
        local e1=Effect.CreateEffect(c)
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetProperty(EFFECT_FLAG_COPY_INHERIT)
        if opt==0 then
            e1:SetCode(EFFECT_UPDATE_ATTACK)
        else
            e1:SetCode(EFFECT_UPDATE_DEFENSE)
        end
        e1:SetValue(1000)
        e1:SetReset(RESET_EVENT|RESETS_STANDARD_DISABLE)
        c:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("lua parse");
        let report = walk(&parsed);
        let ob = report.functions.get("s.adop").expect("op body");
        let spec = extract_choose_spec(None, ob, &report.functions).expect("choose spec");
        assert_eq!(p16_options(&spec), vec![
            (2, vec!["modify_atk self + 1000 until while_face_up".to_string()]),
            (3, vec!["modify_def self + 1000 until while_face_up".to_string()]),
        ]);
    }

    #[test]
    fn p16_skip_fork_with_unresolvable_value() {
        // Cupid Pitch (c21915012) shape: the forked SetValue references
        // a label-object read no static value resolves — both variants
        // must fail to translate, skipping the card.
        let src = r#"
function s.lvop(e,tp,eg,ep,ev,re,r,rp)
    local lv=e:GetLabelObject():GetLabel()
    if lv==0 then return end
    local c=e:GetHandler()
    if c:IsRelateToEffect(e) and c:IsFaceup() then
        local opt=Duel.SelectOption(tp,aux.Stringid(id,2),aux.Stringid(id,3))
        local e1=Effect.CreateEffect(c)
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_LEVEL)
        if opt==0 then
            e1:SetValue(lv)
        else
            e1:SetValue(-lv)
        end
        e1:SetReset(RESET_EVENT|RESETS_STANDARD_DISABLE)
        c:RegisterEffect(e1)
    end
end
"#;
        let parsed = full_moon::parse(src).expect("lua parse");
        let report = walk(&parsed);
        let ob = report.functions.get("s.lvop").expect("op body");
        assert!(extract_choose_spec(None, ob, &report.functions).is_none());
    }

    // ── Phase 17 — select-then-do chains ──────────────────────────────

    #[test]
    fn p17_select_getfirst_action_folds_selector() {
        // `local tc=Duel.SelectMatchingCard(...):GetFirst()` then a direct
        // action on `tc` — the one-card selection folds into the action's
        // selector instead of the bare `target` placeholder.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.SelectMatchingCard(tp,Card.IsFaceup,tp,LOCATION_MZONE,0,1,1,nil):GetFirst()
    if tc then
        Duel.Destroy(tc,REASON_EFFECT)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["destroy (1, card, you control, from monster_zone)"],
        );
    }

    #[test]
    fn p17_select_getfirst_range_minmax_no_binding() {
        // min ≠ max: `GetFirst()` of a wider pick is one arbitrary member
        // — no selector names it, so no binding is created. The pick is
        // NOT the declared target either, so the action degrades to a
        // TODO instead of a mis-aimed bare `target`.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.SelectMatchingCard(tp,Card.IsFaceup,tp,LOCATION_MZONE,0,1,2,nil):GetFirst()
    Duel.Destroy(tc,REASON_EFFECT)
end
"#;
        assert_eq!(p11_actions(src, "s.activate"), Vec::<String>::new());
    }

    // ── group_arg provenance gate ─────────────────────────────────────

    #[test]
    fn group_arg_gate_getfirsttarget_local_is_target() {
        // `local tc=Duel.GetFirstTarget()` — the declared target. Bare
        // `target` is correct.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    if tc:IsRelateToEffect(e) then
        Duel.Destroy(tc,REASON_EFFECT)
    end
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), vec!["destroy target"]);
    }

    #[test]
    fn group_arg_gate_gettargetcards_local_is_target() {
        // `local g=Duel.GetTargetCards(e)` — the declared-target group.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetTargetCards(e)
    Duel.SendtoGrave(g,REASON_EFFECT)
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), vec!["send target to gy"]);
    }

    #[test]
    fn group_arg_gate_inline_getfirsttarget_is_target() {
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    Duel.Remove(Duel.GetFirstTarget(),POS_FACEUP,REASON_EFFECT)
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), vec!["banish target"]);
    }

    #[test]
    fn group_arg_gate_event_group_skips() {
        // `eg` is the EVENT group (e.g. the summoned monsters), not the
        // effect's declared target — bare `target` would mis-aim.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    Duel.Destroy(eg,REASON_EFFECT)
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), Vec::<String>::new());
    }

    #[test]
    fn group_arg_gate_getfieldcard_local_skips() {
        // GetFieldCard binding is a positional fetch, not a declared target.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFieldCard(tp,LOCATION_SZONE,0)
    if tc then
        Duel.Destroy(tc,REASON_EFFECT)
    end
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), Vec::<String>::new());
    }

    #[test]
    fn group_arg_gate_sset_select_pick_skips() {
        // `Duel.SSet(tp, sg)` where sg is an untracked resolve-time pick
        // (SelectUnselectGroup) — the old static `set target` mis-aimed.
        let src = r#"
function s.setop(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(s.setfilter,tp,LOCATION_DECK,0,nil)
    local sg=aux.SelectUnselectGroup(g,e,tp,2,2,s.rescon,1,tp,HINTMSG_SET)
    if #sg==2 then
        Duel.SSet(tp,sg)
    end
end
"#;
        assert_eq!(p11_actions(src, "s.setop"), Vec::<String>::new());
    }

    #[test]
    fn group_arg_gate_summon_getfirsttarget_is_target() {
        // `Duel.Summon(tp, tc, …)` on the declared target.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    Duel.Summon(tp,tc,true,nil)
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), vec!["normal_summon target"]);
    }

    #[test]
    fn group_arg_gate_reassigned_local_skips() {
        // A reassigned local loses single-assignment provenance (taint
        // logic in extract_value_bindings).
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    tc=Duel.GetAttacker()
    Duel.Destroy(tc,REASON_EFFECT)
end
"#;
        assert_eq!(p11_actions(src, "s.operation"), Vec::<String>::new());
    }

    #[test]
    fn p17_alias_getfirst_action_folds_selector() {
        // `local g=Select(...)` + `local tc=g:GetFirst()` — derived alias
        // carries the source selection.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.SelectMatchingCard(tp,Card.IsMonster,tp,LOCATION_GRAVE,0,1,1,nil)
    local tc=g:GetFirst()
    if tc then
        Duel.SpecialSummon(tc,0,tp,tp,false,false,POS_FACEUP)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["special_summon (1, card, you control, from gy)"],
        );
    }

    #[test]
    fn p17_shadowed_group_taints_chain() {
        // Dark Magic Expanded (c111280) hazard: `g` select-bound twice —
        // the alias provenance is ambiguous, so the chain skips.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.SelectMatchingCard(tp,Card.IsFaceup,tp,LOCATION_MZONE,0,1,1,nil)
    local tc=g:GetFirst()
    if tc==nil then
        local g=Duel.SelectMatchingCard(tp,Card.IsFaceup,tp,LOCATION_GRAVE,0,1,1,nil)
        tc=g:GetFirst()
    end
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(500)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let actions = p11_actions(src, "s.activate");
        assert!(actions.is_empty(), "shadowed selection must skip, got {:?}", actions);
    }

    #[test]
    fn p17_chain_selection_consumed_twice_skips() {
        // The selection feeds BOTH an action and a register chain — the
        // emitted selector would re-select a fresh card on the second
        // line, so the chain skips. The action line still emits.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.SelectMatchingCard(tp,Card.IsFaceup,tp,LOCATION_MZONE,0,1,1,nil)
    local tc=g:GetFirst()
    if tc then
        Duel.ChangePosition(tc,POS_FACEUP_DEFENSE)
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(500)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["change_position (1, card, you control, from monster_zone) to defense_position"],
        );
    }

    fn p17_equip_src(helper: &str, call_site: &str) -> String {
        format!(r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    Duel.Hint(HINT_SELECTMSG,tp,HINTMSG_EQUIP)
    local g=Duel.SelectMatchingCard(tp,s.filter,tp,LOCATION_GRAVE,0,1,1,nil)
    local tc=g:GetFirst()
    if tc then
        {}
    end
end
function s.equipop(c,e,tp,tc)
    {}
end
"#, call_site, helper)
    }

    #[test]
    fn p17_equip_helper_emits() {
        // Snatch-steal family: select a card, delegate to the canonical
        // EquipByEffectAndLimitRegister helper → the SELECTED card is
        // equipped onto the host.
        let actions = p11_actions(
            &p17_equip_src(
                "c:EquipByEffectAndLimitRegister(e,tp,tc,id)",
                "s.equipop(c,e,tp,tc)"),
            "s.activate");
        assert_eq!(actions, vec!["equip (1, card, you control, from gy) to self"]);
    }

    #[test]
    fn p17_equip_helper_guard_form_emits() {
        // `if not <equip> then return end` lead — riders after the guard
        // are tolerated (and dropped; they live in the helper's body).
        let actions = p11_actions(
            &p17_equip_src(
                "if not c:EquipByEffectAndLimitRegister(e,tp,tc,nil,true) then return end\n    local atk=tc:GetTextAttack()",
                "s.equipop(c,e,tp,tc)"),
            "s.activate");
        assert_eq!(actions, vec!["equip (1, card, you control, from gy) to self"]);
    }

    #[test]
    fn p17_equip_helper_non_self_host_skips() {
        // Host arg is the selected card, not the card itself — exotic
        // direction, skip.
        let actions = p11_actions(
            &p17_equip_src(
                "c:EquipByEffectAndLimitRegister(e,tp,tc,id)",
                "s.equipop(tc,e,tp,tc)"),
            "s.activate");
        assert!(actions.is_empty(), "non-self host must skip, got {:?}", actions);
    }

    #[test]
    fn p17_take_control_emits() {
        // c43932352 shape: permanent take-to-you on a one-card selection.
        let src = r#"
function s.ctrlop(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.SelectMatchingCard(tp,Card.IsAbleToChangeControler,tp,0,LOCATION_MZONE,1,1,nil)
    if #g>0 then
        Duel.HintSelection(g)
        Duel.GetControl(g,tp)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.ctrlop"),
            vec!["take_control (1, card, opponent controls, from monster_zone)"],
        );
    }

    #[test]
    fn p17_take_control_temporary_skips() {
        // Reset args present — temporary control has no DSL form.
        let src = r#"
function s.ctrlop(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.SelectMatchingCard(tp,nil,tp,0,LOCATION_MZONE,1,1,nil)
    Duel.GetControl(g,tp,RESET_PHASE|PHASE_END,1)
end
"#;
        let actions = p11_actions(src, "s.ctrlop");
        assert!(actions.is_empty(), "temporary control must skip, got {:?}", actions);
    }

    #[test]
    fn p17_equip_action_select_recipient_folds_selector() {
        // Direct `Duel.Equip(tp, c, tc)` where the recipient is a fresh
        // one-card selection — fold the selection, don't claim `target`.
        let src = r#"
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.SelectMatchingCard(tp,Card.IsFaceup,tp,LOCATION_MZONE,0,1,1,nil):GetFirst()
    if tc then
        Duel.Equip(tp,c,tc)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["equip self to (1, card, you control, from monster_zone)"],
        );
    }

    // ── T38 S1 — passive stat chips ─────────────────────────────

    #[test]
    fn s1_faceup_filter_attribute_group_loop() {
        // c13314457 Deepsea Macrotrema shape: aux.FaceupFilter composition
        // refines the group selector, group-applied modifier emits with
        // the exact where-clause.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local g=Duel.GetMatchingGroup(aux.FaceupFilter(Card.IsAttribute,ATTRIBUTE_WATER),tp,LOCATION_MZONE,0,nil)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(c)
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(500)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.operation"),
            vec!["modify_atk (all, card, you control, from monster_zone, where is_face_up and attribute == WATER) + 500 until end_of_turn"],
        );
    }

    #[test]
    fn s1_named_filter_archetype_conjunction() {
        // c51554871 shape: named `s.filter` resolved through the walked
        // function table — IsFaceup ∧ IsSetCard conjunction, event-only
        // reset lowers to while_face_up.
        let src = r#"
function s.filter(c)
    return c:IsFaceup() and c:IsSetCard(SET_LAVAL)
end
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(s.filter,tp,LOCATION_MZONE,0,nil)
    local c=e:GetHandler()
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(c)
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(400)
        e1:SetReset(RESET_EVENT|RESETS_STANDARD)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.operation"),
            vec![r#"modify_atk (all, card, you control, from monster_zone, where is_face_up and archetype == "Laval") + 400 until while_face_up"#],
        );
    }

    #[test]
    fn s1_named_filter_stat_compare_set_final_zero() {
        // c67113830 shape: stat-compare atom (`GetAttack()>0`) in the
        // named filter; SET_ATTACK_FINAL with value 0 emits (the `+ 0`
        // no-op guard is a modify_-path rule, set semantics keep 0).
        let src = r#"
function s.filter(c)
    return c:IsFaceup() and c:GetAttack()>0
end
function s.activate(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(s.filter,tp,0,LOCATION_MZONE,nil)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_SET_ATTACK_FINAL)
        e1:SetValue(0)
        e1:SetReset(RESET_EVENT|RESETS_STANDARD)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert_eq!(
            p11_actions(src, "s.activate"),
            vec!["set_atk (all, card, opponent controls, from monster_zone, where is_face_up and atk > 0) 0 until while_face_up"],
        );
    }

    #[test]
    fn s1_facedown_count_value_clone_pair() {
        // c57296396 shape: value is a local count of face-down banished
        // cards times a negative factor; DEF twin via Clone(). Both lines
        // must carry the count expr and the end-of-turn duration.
        let src = r#"
function s.atkdefop(e,tp,eg,ep,ev,re,r,rp)
    local tc=Duel.GetFirstTarget()
    local ct=Duel.GetMatchingGroupCount(Card.IsFacedown,tp,LOCATION_REMOVED,0,nil)
    local e1=Effect.CreateEffect(e:GetHandler())
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(ct*-100)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
    local e2=e1:Clone()
    e2:SetCode(EFFECT_UPDATE_DEFENSE)
    tc:RegisterEffect(e2)
end
"#;
        assert_eq!(
            p11_actions(src, "s.atkdefop"),
            vec![
                "modify_atk target - count((all, card, you control, from banished, where is_face_down)) * 100 until end_of_turn",
                "modify_def target - count((all, card, you control, from banished, where is_face_down)) * 100 until end_of_turn",
            ],
        );
    }

    #[test]
    fn s1_overlay_count_value_self_only() {
        // c8491961 shape: value reads the HANDLER's overlay count —
        // `self.overlay_count` (grammar allows only the self receiver).
        let src = r#"
function s.atkop(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local oc=c:GetOverlayCount()
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(300*oc)
    e1:SetReset(RESET_EVENT|RESETS_STANDARD)
    tc:RegisterEffect(e1)
end
"#;
        assert_eq!(
            p11_actions(src, "s.atkop"),
            vec!["modify_atk target + 300 * self.overlay_count until while_face_up"],
        );
        // TARGET-side overlay count has no grammar form — must skip.
        let src2 = src.replace("c:GetOverlayCount()", "tc:GetOverlayCount()");
        assert!(p11_actions(&src2, "s.atkop").is_empty());
    }

    #[test]
    fn s1_or_group_under_conjunction_skips() {
        // c9999961 shape: `RACE_BEAST|RACE_WINGEDBEAST` expands to an
        // or-join under the face-up conjunction — needs a parenthesized
        // predicate the v2 parser can't parse yet (task_bb225d5f), so the
        // whole filter must poison.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(aux.FaceupFilter(Card.IsRace,RACE_BEAST|RACE_WINGEDBEAST),tp,LOCATION_MZONE,0,nil)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(200)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert!(p11_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn s1_named_filter_unmapped_atom_poisons() {
        // An atom with no DSL equivalent (IsAbleToGrave) must poison the
        // WHOLE filter — a partial where-clause would buff a superset.
        let src = r#"
function s.filter(c)
    return c:IsFaceup() and c:IsAbleToGrave()
end
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(s.filter,tp,LOCATION_MZONE,0,nil)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(300)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert!(p11_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn s1_named_filter_foreign_binding_poisons() {
        // Atoms must test the filter's own parameter — an atom on the
        // effect binding (`e`) or an outer local is not a per-card
        // predicate.
        let src = r#"
function s.filter(c,e)
    return c:IsFaceup() and not c:IsImmuneToEffect(e)
end
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local g=Duel.GetMatchingGroup(s.filter,tp,LOCATION_MZONE,0,nil,e)
    for tc in aux.Next(g) do
        local e1=Effect.CreateEffect(e:GetHandler())
        e1:SetType(EFFECT_TYPE_SINGLE)
        e1:SetCode(EFFECT_UPDATE_ATTACK)
        e1:SetValue(300)
        e1:SetReset(RESETS_STANDARD_PHASE_END)
        tc:RegisterEffect(e1)
    end
end
"#;
        assert!(p11_actions(src, "s.operation").is_empty());
    }

    #[test]
    fn s1_body_drops_chains_gate() {
        // c6022371 shape: the handler registers a translatable stat chip
        // AND a chain outside translator coverage (EFFECT_CANNOT_TRIGGER).
        // Filling only the chip would under-state the card — the apply
        // passes consult body_drops_chains and skip the whole fill.
        let src = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(500)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
    local e2=Effect.CreateEffect(c)
    e2:SetType(EFFECT_TYPE_SINGLE)
    e2:SetCode(EFFECT_CANNOT_TRIGGER)
    e2:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e2)
end
"#;
        let parsed = full_moon::parse(src).expect("parse");
        let report = walk(&parsed);
        let body = report.functions.get("s.operation").expect("body");
        assert!(body_drops_chains(body, &report.functions));

        // Clean sibling-free version must pass the gate.
        let src2 = r#"
function s.operation(e,tp,eg,ep,ev,re,r,rp)
    local c=e:GetHandler()
    local tc=Duel.GetFirstTarget()
    local e1=Effect.CreateEffect(c)
    e1:SetType(EFFECT_TYPE_SINGLE)
    e1:SetCode(EFFECT_UPDATE_ATTACK)
    e1:SetValue(500)
    e1:SetReset(RESETS_STANDARD_PHASE_END)
    tc:RegisterEffect(e1)
end
"#;
        let parsed2 = full_moon::parse(src2).expect("parse");
        let report2 = walk(&parsed2);
        let body2 = report2.functions.get("s.operation").expect("body");
        assert!(!body_drops_chains(body2, &report2.functions));
    }

    #[test]
    fn s1_filter_expr_mapper_shapes() {
        // Direct-mapper coverage for atoms the corpus shapes exercise.
        let w = |e: &str| lua_filter_expr_to_where(e, "c");
        assert_eq!(
            w("c:IsFaceup() and c:IsLevelAbove(8) and c:IsRace(RACE_DRAGON)").as_deref(),
            Some("is_face_up and level >= 8 and race == Dragon"),
        );
        assert_eq!(
            w("c:IsFaceup() and c:IsRitualMonster()").as_deref(),
            Some("is_face_up and is_monster and is_ritual"),
        );
        assert_eq!(
            w("c:IsFaceup() and c:GetLevel()==4").as_deref(),
            Some("is_face_up and level == 4"),
        );
        // Redundant full-cover parens strip cleanly.
        assert_eq!(w("(c:IsFaceup())").as_deref(), Some("is_face_up"));
        // Flat or-chain is expressible…
        assert_eq!(
            w("c:IsLevel(3) or c:IsRank(3)").as_deref(),
            Some("level == 3 or rank == 3"),
        );
        // …but mixed and/or (even lua-parenthesized) is not, until the
        // parser's nested-predicate branch is fixed.
        assert_eq!(w("c:IsFaceup() and (c:IsLevel(3) or c:IsRank(3))"), None);
        // `not` binds one atom; not-of-compound would need parens.
        assert_eq!(w("not c:IsLevel(4)").as_deref(), Some("not level == 4"));
        assert_eq!(w("not (c:IsFaceup() and c:IsLevel(4))"), None);
        // Multi-value IsLevel is an or-join — fine alone, poison in a chain.
        assert_eq!(w("c:IsLevel(3,4)").as_deref(), Some("level == 3 or level == 4"));
        assert_eq!(w("c:IsFaceup() and c:IsLevel(3,4)"), None);
        // Non-literal compare operand → None.
        assert_eq!(w("c:GetAttack()~=atk"), None);
    }
}
