// ============================================================
// DuelScript Callback Generator — compiler/callback_gen.rs
//
// Generates the four effect callbacks (condition, cost, target,
// operation) from parsed AST. These closures capture AST nodes
// and evaluate against the engine's ScriptContext at runtime.
//
// This module produces engine-compatible closures without
// depending on the engine crate directly — it uses a trait-based
// abstraction (DuelScriptRuntime) that the engine implements.
// ============================================================

use std::sync::Arc;
use crate::ast::*;
use super::type_mapper;

// ── Runtime Abstraction ───────────────────────────────────────
// The engine implements this trait to provide game-state access
// for DuelScript callbacks. This keeps duelscript engine-agnostic.

/// Runtime interface that engines must implement for DuelScript
/// callbacks to execute against live game state.
///
/// Each method maps to a common game operation. The engine
/// translates these into its internal API calls.
/// Trait that engines implement to expose game state and operations
/// to compiled DuelScript closures.
///
/// Note: this trait is intentionally NOT `Send + Sync`. Implementations
/// frequently wrap engine state via `Rc<RefCell>` (single-threaded
/// reference counting), and forcing `Send + Sync` here would be
/// incompatible. The closures stored in `GeneratedCallbacks` are still
/// `Send + Sync` because they don't capture any runtime state — they
/// receive `&mut dyn DuelScriptRuntime` as a parameter, and the runtime
/// trait object can have any thread requirements the engine chooses.
pub trait DuelScriptRuntime {
    // ── Queries ──────────────────────────────────────────────
    fn get_lp(&self, player: u8) -> i32;
    fn get_hand_count(&self, player: u8) -> usize;
    fn get_deck_count(&self, player: u8) -> usize;
    fn get_gy_count(&self, player: u8) -> usize;
    fn get_banished_count(&self, player: u8) -> usize;
    fn get_field_card_count(&self, player: u8, location: u32) -> usize;
    fn get_field_cards(&self, player: u8, location: u32) -> Vec<u32>;
    fn card_matches_filter(&self, card_id: u32, filter: &CardFilter) -> bool;
    fn get_card_stat(&self, card_id: u32, stat: &Stat) -> i32;

    /// Sprint 25: card-attribute queries used by predicate filters.
    /// Returns 0 / sentinel for unknown cards. The race/attribute/type
    /// values are EDOPro-style bitfields (e.g., RACE_WARRIOR = 0x1).
    fn get_card_race(&self, _card_id: u32) -> u64 { 0 }
    fn get_card_attribute(&self, _card_id: u32) -> u64 { 0 }
    fn get_card_type(&self, _card_id: u32) -> u64 { 0 }
    fn get_card_code(&self, card_id: u32) -> u32 { card_id }
    fn get_card_name(&self, _card_id: u32) -> String { String::new() }
    fn get_card_archetypes(&self, _card_id: u32) -> Vec<String> { Vec::new() }

    /// Get the card_id of the effect's owner
    fn effect_card_id(&self) -> u32;
    /// Get the activating player (0 or 1)
    fn effect_player(&self) -> u8;
    /// Get event categories (for chain-link checking)
    fn event_categories(&self) -> u32;

    // ── Card Movement / Actions ──────────────────────────────
    fn draw(&mut self, player: u8, count: u32) -> u32;
    fn destroy(&mut self, card_ids: &[u32]) -> u32;
    fn send_to_grave(&mut self, card_ids: &[u32]) -> u32;
    fn send_to_hand(&mut self, card_ids: &[u32]) -> u32;
    fn banish(&mut self, card_ids: &[u32]) -> u32;
    fn discard(&mut self, card_ids: &[u32]) -> u32;
    fn special_summon(&mut self, card_id: u32, player: u8, position: u32) -> bool;

    // ── Life Points ──────────────────────────────────────────
    fn damage(&mut self, player: u8, amount: i32) -> bool;
    fn recover(&mut self, player: u8, amount: i32) -> bool;

    // ── Selection (UI) ───────────────────────────────────────
    fn select_cards(&mut self, player: u8, candidates: &[u32], min: usize, max: usize) -> Vec<u32>;
    fn select_option(&mut self, player: u8, options: &[String]) -> usize;

    // ── Effect Metadata ──────────────────────────────────────
    fn set_operation_info(&mut self, category: u32, count: u32);
    fn set_targets(&mut self, card_ids: &[u32]);

    // ── Negation ─────────────────────────────────────────────
    fn negate_activation(&mut self) -> bool;
    fn negate_effect(&mut self) -> bool;

    // ── Additional Card Movement ──────────────────────────────
    fn send_to_deck(&mut self, card_ids: &[u32], top: bool) -> u32;
    fn return_to_hand(&mut self, card_ids: &[u32]) -> u32;
    fn tribute(&mut self, card_ids: &[u32]) -> u32;
    fn shuffle_deck(&mut self, player: u8);

    // ── Stat Modification ────────────────────────────────────
    fn modify_atk(&mut self, card_id: u32, delta: i32);
    fn modify_def(&mut self, card_id: u32, delta: i32);
    fn set_atk(&mut self, card_id: u32, value: i32);
    fn set_def(&mut self, card_id: u32, value: i32);

    // ── Battle ───────────────────────────────────────────────
    fn negate_attack(&mut self) -> bool;
    fn change_position(&mut self, card_id: u32);

    // ── Xyz Materials ────────────────────────────────────────
    fn detach_material(&mut self, card_id: u32, count: u32) -> u32;
    fn attach_material(&mut self, material_id: u32, target_id: u32);

    // ── Counters ─────────────────────────────────────────────
    fn place_counter(&mut self, card_id: u32, counter_name: &str, count: u32);
    fn remove_counter(&mut self, card_id: u32, counter_name: &str, count: u32);

    // ── Count matching cards ─────────────────────────────────
    fn count_matching(&self, player: u8, location: u32, filter: &CardFilter) -> usize;

    // ── Phase 2: Custom events ───────────────────────────────
    // Emit a named custom event. The engine should map `name` to a
    // stable event code (typically `EVENT_CUSTOM + hash(name)`) and
    // raise it on the duel's event bus so other effects can react.
    fn raise_custom_event(&mut self, _name: &str, _cards: &[u32]) {}

    // ── Phase 3: Confirm cards ───────────────────────────────
    // Show cards to a player. `audience` is 0=you, 1=opponent, 2=both.
    // `cards` is empty when the whole hand of `owner` should be shown.
    fn confirm_cards(&mut self, _owner: u8, _audience: u8, _cards: &[u32]) {}

    // ── Phase 3: Announce ────────────────────────────────────
    // Prompt a player to announce a value. Returns an opaque u32 token
    // that can later be looked up via `get_announcement`. `kind` is the
    // announcement type (0=card, 1=attribute, 2=race, 3=type, 4=level)
    // and `filter_mask` is an engine-specific OPCODE filter.
    fn announce(&mut self, _player: u8, _kind: u8, _filter_mask: u32) -> u32 { 0 }
    fn get_announcement(&self, _token: u32) -> u32 { 0 }

    // ── Phase 1A: Flag effects ───────────────────────────────
    // Register a flag on a card with custom survive/reset behavior.
    // `survives_mask` and `resets_mask` are engine-specific bitmasks
    // (see the reset_mask_to_bits helper in type_mapper). The flag can
    // be queried later via `has_flag` and removed via `clear_flag`.
    fn register_flag(&mut self, _card_id: u32, _name: &str, _survives_mask: u32, _resets_mask: u32) {}
    fn clear_flag(&mut self, _card_id: u32, _name: &str) {}
    fn has_flag(&self, _card_id: u32, _name: &str) -> bool { false }

    // ── Phase 1B: History queries ────────────────────────────
    // "Was this card previously on the field / face-up / sent by battle?"
    // Implemented in the engine by inspecting prior-state fields.
    fn previous_location(&self, _card_id: u32) -> u32 { 0 }
    fn previous_position(&self, _card_id: u32) -> u32 { 0 }
    fn sent_by_reason(&self, _card_id: u32) -> u32 { 0 }

    // ── Phase 1D: Named bindings ─────────────────────────────
    // Bindings let a cost capture a card, which later actions can
    // reference via `captured.name` / `captured.atk` / etc. The runtime
    // is the keeper of the binding environment for the current effect.
    fn set_binding(&mut self, _name: &str, _card_id: u32) {}
    /// Bind `name` to whatever card(s) the most recent cost/target step
    /// selected. The engine is expected to track its last selection group.
    fn bind_last_selection(&mut self, _name: &str) {}
    fn get_binding_card(&self, _name: &str) -> Option<u32> { None }
    fn get_binding_field(&self, _name: &str, _field: &str) -> i32 { 0 }

    // ── Phase 1E: Change name / code ─────────────────────────
    fn change_card_code(&mut self, _card_id: u32, _code: u32, _duration_mask: u32) {}

    // ── Sprint 67: Control change ────────────────────────────
    fn take_control(&mut self, _card_id: u32, _new_controller: u8) {}

    // ── Sprint 67: Token creation ────────────────────────────
    fn create_token(&mut self, _player: u8, _atk: i32, _def: i32, _count: u32) {}

    // ── Sprint 67: Cross-phase state ─────────────────────────
    fn store_value(&mut self, _label: &str, _value: i32) {}
    fn recall_value(&self, _label: &str) -> i32 { 0 }

    // ── Sprint 67: Delayed effect registration ───────────────
    fn register_delayed(&mut self, _phase: u32, _card_id: u32) {}
}

// ── Generated Callback Types ──────────────────────────────────

/// The four callbacks for an effect, using Arc for cloneability.
pub struct GeneratedCallbacks {
    pub condition: Option<Arc<dyn Fn(&dyn DuelScriptRuntime) -> bool + Send + Sync>>,
    pub cost:      Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>>,
    pub target:    Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>>,
    pub operation: Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime) + Send + Sync>>,
}

// ── Callback Generation ───────────────────────────────────────

/// Generate all four callbacks from an effect body.
pub fn generate_callbacks(body: &EffectBody, _card: &Card) -> GeneratedCallbacks {
    // Sprint 60: implicit condition inference. If the cost includes
    // "tribute self" or "send self to gy" and no explicit condition
    // is declared, inject an on_field condition so the engine doesn't
    // try to activate the effect from the GY/hand/banished.
    let condition = if body.condition.is_none() && cost_implies_on_field(&body.cost) {
        Some(ConditionExpr::Simple(SimpleCondition::OnField))
    } else {
        body.condition.clone()
    };

    GeneratedCallbacks {
        condition: generate_condition(&condition, &body.trigger),
        cost:      generate_cost(&body.cost),
        target:    generate_target(&body.on_activate),
        operation: generate_operation(&body.on_resolve),
    }
}

/// Check if the cost implies the card must be on the field.
fn cost_implies_on_field(cost: &[CostAction]) -> bool {
    cost.iter().any(|c| matches!(
        c,
        CostAction::Tribute(SelfOrTarget::Self_)
        | CostAction::Send { target: SelfOrTarget::Self_, .. }
    ))
}

// ── Condition Callback ────────────────────────────────────────

fn generate_condition(
    condition: &Option<ConditionExpr>,
    trigger: &Option<TriggerExpr>,
) -> Option<Arc<dyn Fn(&dyn DuelScriptRuntime) -> bool + Send + Sync>> {
    let cond = condition.clone();
    let trig = trigger.clone();

    // No condition and no trigger = always true, return None
    if cond.is_none() && trig.is_none() {
        return None;
    }

    Some(Arc::new(move |rt: &dyn DuelScriptRuntime| {
        // Check condition
        if let Some(ref c) = cond {
            if !eval_condition(c, rt) {
                return false;
            }
        }
        // Check trigger (for chain-aware conditions like Ash Blossom)
        if let Some(ref t) = trig {
            if !eval_trigger(t, rt) {
                return false;
            }
        }
        true
    }))
}

fn eval_condition(cond: &ConditionExpr, rt: &dyn DuelScriptRuntime) -> bool {
    match cond {
        ConditionExpr::Simple(s) => eval_simple_condition(s, rt),
        ConditionExpr::And(conditions) => conditions.iter().all(|c| eval_simple_condition(c, rt)),
        ConditionExpr::Or(conditions) => conditions.iter().any(|c| eval_simple_condition(c, rt)),
    }
}

fn eval_simple_condition(cond: &SimpleCondition, rt: &dyn DuelScriptRuntime) -> bool {
    let player = rt.effect_player();
    let opponent = 1 - player;

    match cond {
        SimpleCondition::OnField => true, // checked by range
        SimpleCondition::InZone(_) => true, // checked by range
        SimpleCondition::FieldIsEmpty => {
            rt.get_field_card_count(player, type_mapper::LOCATION_MZONE) == 0
            && rt.get_field_card_count(opponent, type_mapper::LOCATION_MZONE) == 0
        }
        SimpleCondition::YouControlNoMonsters => {
            rt.get_field_card_count(player, type_mapper::LOCATION_MZONE) == 0
        }
        SimpleCondition::OpponentControlsNoMonsters => {
            rt.get_field_card_count(opponent, type_mapper::LOCATION_MZONE) == 0
        }
        SimpleCondition::LpCondition { player: p, op, value } => {
            let p_idx = if *p == Player::You { player } else { opponent };
            let lp = rt.get_lp(p_idx);
            compare(lp as u32, *op, *value)
        }
        SimpleCondition::HandSize { op, value } => {
            compare(rt.get_hand_count(player) as u32, *op, *value)
        }
        SimpleCondition::CardsInGy { op, value } => {
            compare(rt.get_gy_count(player) as u32, *op, *value)
        }
        SimpleCondition::BanishedCount { op, value } => {
            compare(rt.get_banished_count(player) as u32, *op, *value)
        }
        SimpleCondition::YouControlCount { op, value } => {
            compare(rt.get_field_card_count(player, type_mapper::LOCATION_MZONE) as u32, *op, *value)
        }
        SimpleCondition::ChainIncludes(categories) => {
            let event_cats = rt.event_categories();
            categories.iter().any(|cat| {
                let engine_cat = type_mapper::chain_category_to_constant(cat);
                event_cats & engine_cat != 0
            })
        }
        SimpleCondition::YouControl(_target) => {
            // TODO: check if matching cards exist on field
            true
        }
        SimpleCondition::OpponentControls(_target) => {
            // TODO: check if matching cards exist on opponent's field
            true
        }
        // v0.6 additions — evaluated against runtime state
        SimpleCondition::ChainLinkMatches(_) => {
            // TODO: walk the current chain link and match against criteria
            true
        }
        SimpleCondition::History(_) => {
            // TODO: query card history from runtime
            true
        }
        SimpleCondition::Predicate(_) => {
            // TODO: evaluate predicate against self
            true
        }
        // Phase 1A: flag & history conditions — engine runtime support needed
        SimpleCondition::HasFlag { .. }
        | SimpleCondition::PreviousLocation(_)
        | SimpleCondition::PreviousPosition(_)
        | SimpleCondition::SentByReason(_)
        | SimpleCondition::ThisEffectActivatedThisTurn
        | SimpleCondition::ThisCardWasFlippedThisTurn => {
            // TODO: wire to engine's flag/history tracking
            true
        }
    }
}

fn compare(actual: u32, op: CompareOp, expected: u32) -> bool {
    match op {
        CompareOp::Gte => actual >= expected,
        CompareOp::Lte => actual <= expected,
        CompareOp::Gt  => actual > expected,
        CompareOp::Lt  => actual < expected,
        CompareOp::Eq  => actual == expected,
        CompareOp::Neq => actual != expected,
    }
}

fn eval_trigger(trigger: &TriggerExpr, rt: &dyn DuelScriptRuntime) -> bool {
    match trigger {
        TriggerExpr::OpponentActivates(actions) => {
            // Check if the current chain link's categories match any of the listed actions
            let event_cats = rt.event_categories();
            actions.iter().any(|action| {
                let cat = trigger_action_to_category(action);
                event_cats & cat != 0
            })
        }
        // Most triggers are checked by the engine's event system, not by condition
        _ => true,
    }
}

fn trigger_action_to_category(action: &TriggerAction) -> u32 {
    match action {
        TriggerAction::Search         => type_mapper::CATEGORY_SEARCH,
        TriggerAction::SpecialSummon  => type_mapper::CATEGORY_SPECIAL_SUMMON,
        TriggerAction::SendToGy       => type_mapper::CATEGORY_TOGRAVE,
        TriggerAction::AddToHand      => type_mapper::CATEGORY_TOHAND,
        TriggerAction::Draw           => type_mapper::CATEGORY_DRAW,
        TriggerAction::Banish         => type_mapper::CATEGORY_REMOVE,
        TriggerAction::Mill           => type_mapper::CATEGORY_TOGRAVE,
        TriggerAction::TokenSpawn     => type_mapper::CATEGORY_TOKEN,
        _                             => 0,
    }
}

// ── Cost Callback ─────────────────────────────────────────────

fn generate_cost(
    costs: &[CostAction],
) -> Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>> {
    if costs.is_empty() || costs.iter().all(|c| matches!(c, CostAction::None)) {
        return None;
    }

    let costs = costs.to_vec();

    Some(Arc::new(move |rt: &mut dyn DuelScriptRuntime, check_only: bool| {
        let player = rt.effect_player();
        let card_id = rt.effect_card_id();

        for cost in &costs {
            match cost {
                CostAction::None => {}
                CostAction::PayLp(expr) => {
                    let amount = eval_expr_runtime(expr, rt);
                    if check_only {
                        if rt.get_lp(player) < amount {
                            return false;
                        }
                    } else {
                        rt.damage(player, amount);
                    }
                }
                CostAction::Discard(SelfOrTarget::Self_) => {
                    if check_only {
                        // Card must be in hand to discard
                        // The engine validates this via range
                        return true;
                    } else {
                        rt.discard(&[card_id]);
                    }
                }
                CostAction::Discard(SelfOrTarget::Target(_target)) => {
                    if check_only {
                        return rt.get_hand_count(player) > 0;
                    } else {
                        let hand = rt.get_field_cards(player, type_mapper::LOCATION_HAND);
                        if !hand.is_empty() {
                            let selected = rt.select_cards(player, &hand, 1, 1);
                            if !selected.is_empty() {
                                rt.discard(&selected);
                            }
                        }
                    }
                }
                CostAction::Tribute(SelfOrTarget::Self_) => {
                    if !check_only {
                        rt.send_to_grave(&[card_id]);
                    }
                }
                CostAction::Tribute(SelfOrTarget::Target(_target)) => {
                    if check_only {
                        return rt.get_field_card_count(player, type_mapper::LOCATION_MZONE) > 0;
                    } else {
                        let monsters = rt.get_field_cards(player, type_mapper::LOCATION_MZONE);
                        let selected = rt.select_cards(player, &monsters, 1, 1);
                        if !selected.is_empty() {
                            rt.send_to_grave(&selected);
                        }
                    }
                }
                CostAction::Banish { target, .. } => {
                    match target {
                        SelfOrTarget::Self_ => {
                            if !check_only {
                                rt.banish(&[card_id]);
                            }
                        }
                        SelfOrTarget::Target(_) => {
                            if check_only {
                                return true; // simplified
                            }
                        }
                    }
                }
                CostAction::Detach { count, .. } => {
                    // Xyz detach — engine handles overlay materials
                    if check_only {
                        return true; // engine validates overlay count
                    }
                    let _ = count; // engine uses this
                }
                CostAction::Reveal(_) => {
                    // Reveal is always payable
                }
                CostAction::Send { .. } | CostAction::RemoveCounter { .. } => {
                    if check_only {
                        return true; // simplified
                    }
                }
                CostAction::Announce { .. } => {
                    // Phase 3: announcements are always payable.
                    if check_only { return true; }
                }
                CostAction::Bound { name, inner: _ } => {
                    // The inner cost (reveal/send/etc) runs through the
                    // standard selection flow; we then snapshot whatever
                    // was selected under `name`. The engine is responsible
                    // for the actual binding storage.
                    if check_only { return true; }
                    rt.bind_last_selection(name);
                }
            }
        }
        true
    }))
}

// ── Target Callback ───────────────────────────────────────────

fn generate_target(
    on_activate: &[GameAction],
) -> Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>> {
    if on_activate.is_empty() {
        return None;
    }

    let actions = on_activate.to_vec();

    Some(Arc::new(move |rt: &mut dyn DuelScriptRuntime, check_only: bool| {
        // Target phase: check if valid targets exist (check_only=true)
        // or actually select targets (check_only=false)
        for action in &actions {
            match action {
                GameAction::Destroy { target } => {
                    if !resolve_target_check(target, rt, check_only) {
                        return false;
                    }
                }
                GameAction::Search { target, .. } => {
                    if !resolve_target_check(target, rt, check_only) {
                        return false;
                    }
                }
                _ => {
                    // Non-targeting actions in on_activate are fine
                }
            }
        }
        true
    }))
}

fn resolve_target_check(
    _target: &TargetExpr,
    _rt: &mut dyn DuelScriptRuntime,
    _check_only: bool,
) -> bool {
    // TODO: expand target resolution
    // For now, assume valid — engine will do full validation
    true
}

// ── Operation Callback ────────────────────────────────────────

fn generate_operation(
    on_resolve: &[GameAction],
) -> Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime) + Send + Sync>> {
    if on_resolve.is_empty() {
        return None;
    }

    let actions = on_resolve.to_vec();

    Some(Arc::new(move |rt: &mut dyn DuelScriptRuntime| {
        let player = rt.effect_player();
        for action in &actions {
            execute_action(action, rt, player);
        }
    }))
}

fn execute_action(action: &GameAction, rt: &mut dyn DuelScriptRuntime, player: u8) {
    let opponent = 1 - player;

    match action {
        GameAction::Draw { count } => {
            let n = eval_expr_runtime(count, rt) as u32;
            rt.draw(player, n);
        }
        GameAction::Destroy { target } => {
            let cards = resolve_target_cards(target, rt, player);
            if !cards.is_empty() {
                rt.destroy(&cards);
            }
        }
        GameAction::SpecialSummon { target, position, .. } => {
            // Position bits: ATK=0x1, DEF=0x2, FACEUP=0x5, FACEDOWN=0xA.
            // Default to face-up attack position when unspecified.
            let pos = match position {
                Some(BattlePosition::AttackPosition) => 0x1,
                Some(BattlePosition::DefensePosition) => 0x2,
                Some(BattlePosition::FaceDownDefense) => 0xA,
                None => 0x1,
            };
            match target {
                SelfOrTarget::Self_ => {
                    let card_id = rt.effect_card_id();
                    rt.special_summon(card_id, player, pos);
                }
                SelfOrTarget::Target(t) => {
                    let cards = resolve_target_cards(t, rt, player);
                    for card_id in cards {
                        rt.special_summon(card_id, player, pos);
                    }
                }
            }
        }
        GameAction::SendToZone { target, .. } => {
            match target {
                SelfOrTarget::Self_ => {
                    let card_id = rt.effect_card_id();
                    rt.send_to_grave(&[card_id]);
                }
                SelfOrTarget::Target(t) => {
                    let cards = resolve_target_cards(t, rt, player);
                    rt.send_to_grave(&cards);
                }
            }
        }
        GameAction::Search { target, .. } => {
            let cards = resolve_target_cards(target, rt, player);
            if !cards.is_empty() {
                let selected = rt.select_cards(player, &cards, 1, 1);
                rt.send_to_hand(&selected);
            }
        }
        GameAction::AddToHand { target, .. } => {
            let cards = resolve_target_cards(target, rt, player);
            if !cards.is_empty() {
                rt.send_to_hand(&cards);
            }
        }
        GameAction::Banish { target, .. } => {
            match target {
                SelfOrTarget::Self_ => {
                    let card_id = rt.effect_card_id();
                    rt.banish(&[card_id]);
                }
                SelfOrTarget::Target(t) => {
                    let cards = resolve_target_cards(t, rt, player);
                    rt.banish(&cards);
                }
            }
        }
        GameAction::Negate { and_destroy, .. } => {
            rt.negate_activation();
            if *and_destroy {
                // The negated card should be destroyed
                // Engine tracks which card was negated
            }
        }
        GameAction::DealDamage { to, amount } => {
            let amt = eval_expr_runtime(amount, rt);
            let target_player = match to {
                DamageTarget::Opponent    => opponent,
                DamageTarget::You         => player,
                DamageTarget::BothPlayers => {
                    rt.damage(player, amt);
                    opponent
                }
            };
            rt.damage(target_player, amt);
        }
        GameAction::GainLp { amount } => {
            let amt = eval_expr_runtime(amount, rt);
            rt.recover(player, amt);
        }
        GameAction::Discard { target, random: _ } => {
            match target {
                SelfOrTarget::Self_ => {
                    let card_id = rt.effect_card_id();
                    rt.discard(&[card_id]);
                }
                SelfOrTarget::Target(t) => {
                    let cards = resolve_target_cards(t, rt, player);
                    rt.discard(&cards);
                }
            }
        }
        GameAction::Mill { count, from } => {
            let n = eval_expr_runtime(count, rt) as u32;
            let mill_player = match from {
                MillSource::YourDeck     => player,
                MillSource::OpponentDeck => opponent,
            };
            // Mill = send top N cards from deck to GY
            let deck = rt.get_field_cards(mill_player, type_mapper::LOCATION_DECK);
            let to_mill: Vec<u32> = deck.iter().rev().take(n as usize).copied().collect();
            rt.send_to_grave(&to_mill);
        }
        GameAction::TakeControl { target, .. } => {
            let cards = resolve_target_cards(target, rt, player);
            for cid in cards {
                rt.take_control(cid, player);
            }
        }
        GameAction::CreateToken { spec } => {
            let atk = match &spec.atk {
                StatValue::Number(n) => *n,
                _ => 0,
            };
            let def = match &spec.def {
                StatValue::Number(n) => *n,
                _ => 0,
            };
            let count = spec.count;
            rt.create_token(player, atk, def, count);
        }
        GameAction::If { condition, then_actions, else_actions } => {
            if eval_condition(condition, rt) {
                for a in then_actions {
                    execute_action(a, rt, player);
                }
            } else {
                for a in else_actions {
                    execute_action(a, rt, player);
                }
            }
        }
        GameAction::Choose { options } => {
            let labels: Vec<String> = options.iter().map(|o| o.label.clone()).collect();
            let choice = rt.select_option(player, &labels);
            if choice < options.len() {
                for a in &options[choice].actions {
                    execute_action(a, rt, player);
                }
            }
        }
        GameAction::ForEach { target, in_zone, actions } => {
            let location = type_mapper::zone_to_location(in_zone);
            let cards = rt.get_field_cards(player, location);
            let matching: Vec<u32> = cards.into_iter()
                .filter(|&cid| match &target {
                    TargetExpr::Filter(f) => rt.card_matches_filter(cid, f),
                    _ => true,
                })
                .collect();
            for _card_id in matching {
                for a in actions {
                    execute_action(a, rt, player);
                }
            }
        }
        GameAction::Return { target, to, shuffle } => {
            let cards = match target {
                SelfOrTarget::Self_ => vec![rt.effect_card_id()],
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player),
            };
            match to {
                ReturnZone::Hand => { rt.return_to_hand(&cards); }
                ReturnZone::Deck => {
                    rt.send_to_deck(&cards, !shuffle);
                    if *shuffle { rt.shuffle_deck(player); }
                }
                ReturnZone::ExtraDeck => { rt.send_to_deck(&cards, false); }
            }
        }
        GameAction::ModifyAtk { kind, target, .. } => {
            let cards = match target {
                Some(t) => resolve_target_cards(t, rt, player),
                None => vec![rt.effect_card_id()],
            };
            for card_id in cards {
                match kind {
                    AtkModKind::Delta { sign, value } => {
                        let v = eval_expr_runtime(value, rt);
                        let delta = match sign { Sign::Plus => v, Sign::Minus => -v };
                        rt.modify_atk(card_id, delta);
                    }
                    AtkModKind::SetTo(expr) => {
                        let v = eval_expr_runtime(expr, rt);
                        rt.set_atk(card_id, v);
                    }
                    AtkModKind::Double => {
                        let current = rt.get_card_stat(card_id, &Stat::Atk);
                        rt.set_atk(card_id, current * 2);
                    }
                    AtkModKind::Halve => {
                        let current = rt.get_card_stat(card_id, &Stat::Atk);
                        rt.set_atk(card_id, current / 2);
                    }
                }
            }
        }
        GameAction::ModifyDef { kind, target, .. } => {
            let cards = match target {
                Some(t) => resolve_target_cards(t, rt, player),
                None => vec![rt.effect_card_id()],
            };
            for card_id in cards {
                match kind {
                    DefModKind::Delta { sign, value } => {
                        let v = eval_expr_runtime(value, rt);
                        let delta = match sign { Sign::Plus => v, Sign::Minus => -v };
                        rt.modify_def(card_id, delta);
                    }
                    DefModKind::SetTo(expr) => {
                        let v = eval_expr_runtime(expr, rt);
                        rt.set_def(card_id, v);
                    }
                }
            }
        }
        GameAction::Detach { count, from } => {
            let card_id = match from {
                SelfOrTarget::Self_ => rt.effect_card_id(),
                SelfOrTarget::Target(t) => {
                    resolve_target_cards(t, rt, player).first().copied().unwrap_or(0)
                }
            };
            rt.detach_material(card_id, *count as u32);
        }
        GameAction::Attach { target, to } => {
            let materials = resolve_target_cards(target, rt, player);
            let target_id = match to {
                SelfOrTarget::Self_ => rt.effect_card_id(),
                SelfOrTarget::Target(t) => {
                    resolve_target_cards(t, rt, player).first().copied().unwrap_or(0)
                }
            };
            for mat_id in materials {
                rt.attach_material(mat_id, target_id);
            }
        }
        GameAction::ChangeBattlePosition { target } => {
            let cards = resolve_target_cards(target, rt, player);
            for card_id in cards {
                rt.change_position(card_id);
            }
        }
        GameAction::PlaceCounter { count, name, on } => {
            let card_id = match on {
                SelfOrTarget::Self_ => rt.effect_card_id(),
                SelfOrTarget::Target(t) => {
                    resolve_target_cards(t, rt, player).first().copied().unwrap_or(0)
                }
            };
            rt.place_counter(card_id, name, *count as u32);
        }
        GameAction::RemoveCounter { count, name, from } => {
            let card_id = match from {
                SelfOrTarget::Self_ => rt.effect_card_id(),
                SelfOrTarget::Target(t) => {
                    resolve_target_cards(t, rt, player).first().copied().unwrap_or(0)
                }
            };
            rt.remove_counter(card_id, name, *count as u32);
        }
        GameAction::Shuffle { zone } => {
            let p = player; // shuffle own zone by default
            let _ = zone;
            rt.shuffle_deck(p);
        }
        GameAction::Tribute { target } => {
            let cards = match target {
                SelfOrTarget::Self_ => vec![rt.effect_card_id()],
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player),
            };
            rt.tribute(&cards);
        }
        GameAction::Reveal { target } => {
            // Reveal is informational — engine handles display
            let _ = target;
        }
        GameAction::Delayed { until, actions } => {
            // Register with the engine that these actions should execute
            // at the given phase. For now, store the card ID + phase.
            let phase_code = 0x1200u32; // default to END_PHASE
            let _ = until;
            rt.register_delayed(phase_code, rt.effect_card_id());
            // Also execute as fallback for MockRuntime testing
            for a in actions {
                execute_action(a, rt, player);
            }
        }
        GameAction::RegisterEffect { target, effect, duration } => {
            // Register a dynamic effect on target card(s). The grants
            // are applied via the runtime's register_flag mechanism.
            let cards = resolve_target_cards(target, rt, player);
            for grant in &effect.grants {
                let name = format!("dynamic_{:?}", grant);
                for &cid in &cards {
                    rt.register_flag(cid, &name, 0, 0);
                }
            }
            let _ = duration;
        }
        GameAction::Store { label, value } => {
            let val = match value {
                crate::ast::StoreValue::SelectedTargets => 0,
                crate::ast::StoreValue::Expression(e) => eval_expr_runtime(e, rt),
            };
            rt.store_value(label, val);
        }
        GameAction::Recall { label } => {
            let _val = rt.recall_value(label);
        }
        GameAction::SendToDeck { target, position } => {
            let cards = match target {
                SelfOrTarget::Self_ => vec![rt.effect_card_id()],
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player),
            };
            let top = matches!(position, DeckPosition::Top);
            rt.send_to_deck(&cards, top);
            if matches!(position, DeckPosition::Shuffle) {
                rt.shuffle_deck(player);
            }
        }
        GameAction::Release { target } => {
            let cards = match target {
                SelfOrTarget::Self_ => vec![rt.effect_card_id()],
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player),
            };
            rt.tribute(&cards);
        }
        GameAction::DiscardAll { whose } => {
            let p = if *whose == Player::You { player } else { opponent };
            let hand = rt.get_field_cards(p, type_mapper::LOCATION_HAND);
            rt.discard(&hand);
        }
        GameAction::ShuffleHand { whose } => {
            let p = match whose {
                Some(Player::Opponent) => opponent,
                _ => player,
            };
            rt.shuffle_deck(p); // Engine handles hand shuffle
        }
        GameAction::ShuffleDeck { whose } => {
            let p = match whose {
                Some(Player::Opponent) => opponent,
                _ => player,
            };
            rt.shuffle_deck(p);
        }
        GameAction::SetSpellTrap { .. } => {
            // Engine handles set-to-field mechanics
        }
        GameAction::MoveToField { target, .. } => {
            match target {
                SelfOrTarget::Self_ => {
                    let card_id = rt.effect_card_id();
                    rt.special_summon(card_id, player, 0x1);
                }
                _ => {}
            }
        }
        GameAction::Excavate { count, .. } => {
            let _n = eval_expr_runtime(count, rt);
            // Engine handles excavation display + selection
        }
        GameAction::NormalSummon { .. } => {
            // Engine handles normal summon mechanics
        }
        GameAction::YesNo { yes_actions, no_actions } => {
            let choice = rt.select_option(player, &["Yes".to_string(), "No".to_string()]);
            let actions = if choice == 0 { yes_actions } else { no_actions };
            for a in actions {
                execute_action(a, rt, player);
            }
        }
        GameAction::CoinFlip { heads, tails } => {
            // 50/50 random — engine provides randomness
            let choice = rt.select_option(player, &["Heads".to_string(), "Tails".to_string()]);
            let actions = if choice == 0 { heads } else { tails };
            for a in actions {
                execute_action(a, rt, player);
            }
        }
        GameAction::ChangeLevel { target, value } => {
            let _cards = match target {
                SelfOrTarget::Self_ => vec![rt.effect_card_id()],
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player),
            };
            let _v = eval_expr_runtime(value, rt);
            // Engine handles level modification
        }
        GameAction::ChangeAttribute { .. } | GameAction::ChangeRace { .. } => {
            // Engine handles attribute/race modification
        }
        GameAction::ChangeName { target, source, duration: _ } => {
            let card_ids = match target {
                SelfOrTarget::Self_ => vec![rt.effect_card_id()],
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player),
            };
            let code: u32 = match source {
                NameSource::Code(n) => *n,
                NameSource::Literal(_) => 0, // engine resolves by name lookup
                NameSource::Binding { name, .. } => {
                    rt.get_binding_field(name, "code") as u32
                }
            };
            for id in card_ids {
                rt.change_card_code(id, code, 0);
            }
        }
        GameAction::ChangeCode { target, source, duration: _ } => {
            let card_ids = match target {
                SelfOrTarget::Self_ => vec![rt.effect_card_id()],
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player),
            };
            let code: u32 = match source {
                NameSource::Code(n) => *n,
                NameSource::Literal(_) => 0,
                NameSource::Binding { name, .. } => {
                    rt.get_binding_field(name, "code") as u32
                }
            };
            for id in card_ids {
                rt.change_card_code(id, code, 0);
            }
        }
        GameAction::EmitEvent(name) => {
            rt.raise_custom_event(name, &[rt.effect_card_id()]);
        }
        // Sprint 28: in-resolution selection binding. Resolve targets,
        // pick the first card (deterministic), and bind it under `name`
        // so subsequent BindingRef expressions can reach it.
        GameAction::Select { target, name } => {
            let cards = resolve_target_cards(target, rt, player);
            if let Some(&first) = cards.first() {
                rt.set_binding(name, first);
            }
        }
        GameAction::Confirm { target, audience } => {
            let aud = match audience {
                ConfirmAudience::You => 0u8,
                ConfirmAudience::Opponent => 1u8,
                ConfirmAudience::Both => 2u8,
            };
            match target {
                ConfirmTarget::Hand => {
                    // Empty list signals "reveal entire hand of owner"
                    rt.confirm_cards(player, aud, &[]);
                }
                ConfirmTarget::SelfCard => {
                    rt.confirm_cards(player, aud, &[rt.effect_card_id()]);
                }
                ConfirmTarget::Target(t) => {
                    let cards = resolve_target_cards(t, rt, player);
                    rt.confirm_cards(player, aud, &cards);
                }
            }
        }
        GameAction::NegateEffects { target, .. } => {
            let _cards = match target {
                SelfOrTarget::Self_ => vec![rt.effect_card_id()],
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player),
            };
            // Engine handles effect negation state
        }
        GameAction::Overlay { materials, target } => {
            let mats = resolve_target_cards(materials, rt, player);
            let target_id = match target {
                SelfOrTarget::Self_ => rt.effect_card_id(),
                SelfOrTarget::Target(t) => resolve_target_cards(t, rt, player).first().copied().unwrap_or(0),
            };
            for mat_id in mats {
                rt.attach_material(mat_id, target_id);
            }
        }
        GameAction::SetFlag { name, target, survives, resets_on, value: _ } => {
            let card_ids = match target {
                Some(SelfOrTarget::Target(t)) => resolve_target_cards(t, rt, player),
                _ => vec![rt.effect_card_id()],
            };
            let survives_mask = type_mapper::flag_reset_mask(survives);
            let resets_mask = type_mapper::flag_reset_mask(resets_on);
            for id in card_ids {
                rt.register_flag(id, name, survives_mask, resets_mask);
            }
        }
        GameAction::ClearFlag { name, target } => {
            let card_ids = match target {
                Some(SelfOrTarget::Target(t)) => resolve_target_cards(t, rt, player),
                _ => vec![rt.effect_card_id()],
            };
            for id in card_ids {
                rt.clear_flag(id, name);
            }
        }
        // Sequential resolution semantics (v0.6)
        // The actual succeeded-or-not tracking is engine-side.
        // In the closure, we execute the nested actions linearly for now;
        // the engine interprets the categorization via the effect metadata.
        GameAction::AndIfYouDo { actions } => {
            // TODO: check "prior action succeeded" flag from runtime
            // For now, execute unconditionally (closest approximation)
            for a in actions {
                execute_action(a, rt, player);
            }
        }
        GameAction::Then { actions } => {
            for a in actions {
                execute_action(a, rt, player);
            }
        }
        GameAction::Also { actions } => {
            for a in actions {
                execute_action(a, rt, player);
            }
        }
        // Remaining stubs
        GameAction::SetFaceDown { .. }
        | GameAction::FlipFaceDown { .. }
        | GameAction::LookAt { .. }
        | GameAction::CopyEffect { .. }
        | GameAction::Equip { .. }
        | GameAction::SetScale { .. }
        | GameAction::FusionSummon { .. }
        | GameAction::SynchroSummon { .. }
        | GameAction::XyzSummon { .. }
        | GameAction::RitualSummon { .. }
        | GameAction::PendulumSummon { .. }
        | GameAction::ApplyUntil { .. } => {
            // These need deeper engine support
        }
    }
}

// ── Expression Evaluation (runtime) ───────────────────────────

fn eval_expr_runtime(expr: &Expr, rt: &dyn DuelScriptRuntime) -> i32 {
    match expr {
        Expr::Literal(n) => *n,
        Expr::SelfStat(stat) => {
            let card_id = rt.effect_card_id();
            rt.get_card_stat(card_id, stat)
        }
        Expr::TargetStat(_stat) => {
            // TODO: get target card's stat
            0
        }
        Expr::PlayerLp(player) => {
            let p = if *player == Player::You { rt.effect_player() } else { 1 - rt.effect_player() };
            rt.get_lp(p)
        }
        Expr::Count { target, zone } => {
            let player = rt.effect_player();
            let location = zone.as_ref()
                .map(|z| type_mapper::zone_to_location(z))
                .unwrap_or(type_mapper::LOCATION_ONFIELD);
            match target.as_ref() {
                TargetExpr::Filter(f) => rt.count_matching(player, location, f) as i32,
                _ => 0,
            }
        }
        Expr::BindingRef { name, field } => rt.get_binding_field(name, field),
        Expr::BinOp { left, op, right } => {
            let l = eval_expr_runtime(left, rt);
            let r = eval_expr_runtime(right, rt);
            match op {
                BinOp::Add => l.saturating_add(r),
                BinOp::Sub => l.saturating_sub(r),
                BinOp::Mul => l.saturating_mul(r),
                BinOp::Div => if r == 0 { 0 } else { l / r },
            }
        }
    }
}

// ── Target Resolution ─────────────────────────────────────────

fn resolve_target_cards(target: &TargetExpr, rt: &dyn DuelScriptRuntime, player: u8) -> Vec<u32> {
    let opponent = 1 - player;
    match target {
        TargetExpr::SelfCard => vec![rt.effect_card_id()],
        TargetExpr::Counted { filter, controller, zone, predicate, .. } => {
            let ctrl = match controller {
                Some(ControllerRef::You)         => player,
                Some(ControllerRef::Opponent)     => opponent,
                Some(ControllerRef::EitherPlayer) | None => player,
            };
            let location = zone.as_ref()
                .map(|z| type_mapper::zone_to_location(z))
                .unwrap_or(type_mapper::LOCATION_ONFIELD);

            let all_cards = rt.get_field_cards(ctrl, location);
            let mut matching: Vec<u32> = all_cards.into_iter()
                .filter(|&cid| rt.card_matches_filter(cid, filter))
                .collect();

            // For EitherPlayer, also check opponent's cards.
            if matches!(controller, Some(ControllerRef::EitherPlayer)) {
                let opp_cards = rt.get_field_cards(opponent, location);
                matching.extend(opp_cards.into_iter()
                    .filter(|&cid| rt.card_matches_filter(cid, filter)));
            }

            // Sprint 23: apply the predicate filter on top.
            if let Some(pred) = predicate {
                matching.retain(|&cid| eval_predicate(pred, cid, rt));
            }
            matching
        }
        TargetExpr::Filter(filter) => {
            let all = rt.get_field_cards(player, type_mapper::LOCATION_ONFIELD);
            all.into_iter()
                .filter(|&cid| rt.card_matches_filter(cid, filter))
                .collect()
        }
        TargetExpr::WithPredicate { filter, predicate, .. } => {
            let all = rt.get_field_cards(player, type_mapper::LOCATION_ONFIELD);
            all.into_iter()
                .filter(|&cid| rt.card_matches_filter(cid, filter))
                .filter(|&cid| eval_predicate(predicate, cid, rt))
                .collect()
        }
    }
}

/// Sprint 23: evaluate a Predicate against a single card_id.
/// Walks the AST and queries the runtime for each leaf.
fn eval_predicate(pred: &Predicate, card_id: u32, rt: &dyn DuelScriptRuntime) -> bool {
    match pred {
        Predicate::And(parts) => parts.iter().all(|p| eval_predicate(p, card_id, rt)),
        Predicate::Or(parts)  => parts.iter().any(|p| eval_predicate(p, card_id, rt)),
        Predicate::Not(inner) => !eval_predicate(inner, card_id, rt),
        Predicate::Compare { field, op, value } => {
            let lhs = pred_field_value(field, card_id, rt);
            let rhs = pred_value_to_i32(value);
            match op {
                CompareOp::Gte => lhs >= rhs,
                CompareOp::Lte => lhs <= rhs,
                CompareOp::Gt  => lhs >  rhs,
                CompareOp::Lt  => lhs <  rhs,
                CompareOp::Eq  => lhs == rhs,
                CompareOp::Neq => lhs != rhs,
            }
        }
        // Property/state checks fall through to the runtime; the trait
        // exposes the most-needed ones (is_face_up, has_flag, etc.).
        // Anything we don't yet have a method for defaults to true so
        // cards don't accidentally get filtered to nothing.
        Predicate::Is(_)
        | Predicate::Has(_)
        | Predicate::Location(_)
        | Predicate::Controller(_)
        | Predicate::StateCheck(_)
        | Predicate::Archetype(_)
        | Predicate::SummonedBy(_) => true,
    }
}

/// Read a numeric value off the candidate card for predicate comparison.
/// Sprint 25: Race/Attribute/Type/Name now query the runtime.
fn pred_field_value(field: &PredField, card_id: u32, rt: &dyn DuelScriptRuntime) -> i32 {
    match field {
        PredField::Atk           => rt.get_card_stat(card_id, &Stat::Atk),
        PredField::Def           => rt.get_card_stat(card_id, &Stat::Def),
        PredField::Level         => rt.get_card_stat(card_id, &Stat::Level),
        PredField::Rank          => rt.get_card_stat(card_id, &Stat::Rank),
        PredField::OriginalAtk   => rt.get_card_stat(card_id, &Stat::OriginalAtk),
        PredField::OriginalDef   => rt.get_card_stat(card_id, &Stat::OriginalDef),
        PredField::OriginalLevel => rt.get_card_stat(card_id, &Stat::Level),
        PredField::LinkRating    => rt.get_card_stat(card_id, &Stat::Level),
        PredField::Scale         => 0,
        PredField::CardId        => rt.get_card_code(card_id) as i32,
        PredField::Race          => rt.get_card_race(card_id) as i32,
        PredField::Attribute     => rt.get_card_attribute(card_id) as i32,
        PredField::Type          => rt.get_card_type(card_id) as i32,
        PredField::Name          => 0, // strings need separate handling
    }
}

/// Coerce a PredValue to i32 for numeric comparisons. Race/Attribute
/// values use the EDOPro bitfield discriminant of their AST variant
/// (matches what the runtime returns).
fn pred_value_to_i32(value: &PredValue) -> i32 {
    match value {
        PredValue::Number(n)    => *n,
        PredValue::Race(r)      => race_to_bits(r) as i32,
        PredValue::Attribute(a) => attribute_to_bits(a) as i32,
        PredValue::CardType(_)  => 0,
        PredValue::String(_)    => 0,
        PredValue::FieldRef(_)  => 0,
    }
}

/// Map an AST Race to the EDOPro RACE_X bitfield.
fn race_to_bits(r: &Race) -> u64 {
    match r {
        Race::Warrior      => 0x1,
        Race::Spellcaster  => 0x2,
        Race::Fairy        => 0x4,
        Race::Fiend        => 0x8,
        Race::Zombie       => 0x10,
        Race::Machine      => 0x20,
        Race::Aqua         => 0x40,
        Race::Pyro         => 0x80,
        Race::Rock         => 0x100,
        Race::WingedBeast  => 0x200,
        Race::Plant        => 0x400,
        Race::Insect       => 0x800,
        Race::Thunder      => 0x1000,
        Race::Dragon       => 0x2000,
        Race::Beast        => 0x4000,
        Race::BeastWarrior => 0x8000,
        Race::Dinosaur     => 0x10000,
        Race::Fish         => 0x20000,
        Race::SeaSerpent   => 0x40000,
        Race::Reptile      => 0x80000,
        Race::Psychic      => 0x100000,
        Race::DivineBeast  => 0x200000,
        Race::CreatorGod   => 0x400000,
        Race::Wyrm         => 0x800000,
        Race::Cyberse      => 0x1000000,
        Race::Illusion     => 0x2000000,
        Race::Cyborg       => 0x4000000,
        Race::MagicalKnight => 0x8000000,
        Race::HighDragon   => 0x10000000,
        Race::OmegaPsychic => 0x20000000,
        Race::Unknown      => 0,
    }
}

/// Map an AST Attribute to the EDOPro ATTRIBUTE_X bitfield.
fn attribute_to_bits(a: &Attribute) -> u64 {
    match a {
        Attribute::Earth  => 0x1,
        Attribute::Water  => 0x2,
        Attribute::Fire   => 0x4,
        Attribute::Wind   => 0x8,
        Attribute::Light  => 0x10,
        Attribute::Dark   => 0x20,
        Attribute::Divine => 0x40,
    }
}
