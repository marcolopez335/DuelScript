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
pub trait DuelScriptRuntime: Send + Sync {
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
    GeneratedCallbacks {
        condition: generate_condition(&body.condition, &body.trigger),
        cost:      generate_cost(&body.cost),
        target:    generate_target(&body.on_activate),
        operation: generate_operation(&body.on_resolve),
    }
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
        GameAction::SpecialSummon { target, .. } => {
            match target {
                SelfOrTarget::Self_ => {
                    let card_id = rt.effect_card_id();
                    rt.special_summon(card_id, player, 0x1); // face-up attack
                }
                SelfOrTarget::Target(_) => {
                    // TODO: resolve target and summon
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
        GameAction::Discard { target } => {
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
            let _cards = resolve_target_cards(target, rt, player);
            // TODO: implement control change via engine
        }
        GameAction::CreateToken { spec } => {
            // TODO: token creation via engine
            let _ = spec;
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
            // TODO: register a future trigger effect at the given phase
            // For now, execute immediately as a placeholder
            let _ = until;
            for a in actions {
                execute_action(a, rt, player);
            }
        }
        GameAction::RegisterEffect { target, effect, duration } => {
            // TODO: dynamically register a new continuous/restriction effect
            let _ = (target, effect, duration);
        }
        GameAction::Store { label, value } => {
            // TODO: store state for cross-phase persistence
            let _ = (label, value);
        }
        GameAction::Recall { label } => {
            // TODO: recall stored state
            let _ = label;
        }
        // Remaining stubs — need deeper engine integration
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
            // These need deeper engine support — will be implemented as cards need them
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
        TargetExpr::Counted { filter, controller, zone, .. } => {
            let ctrl = match controller {
                Some(ControllerRef::You)         => player,
                Some(ControllerRef::Opponent)     => opponent,
                Some(ControllerRef::EitherPlayer) | None => player,
            };
            let location = zone.as_ref()
                .map(|z| type_mapper::zone_to_location(z))
                .unwrap_or(type_mapper::LOCATION_ONFIELD);

            let all_cards = rt.get_field_cards(ctrl, location);
            let matching: Vec<u32> = all_cards.into_iter()
                .filter(|&cid| rt.card_matches_filter(cid, filter))
                .collect();

            // For EitherPlayer, also check opponent
            if matches!(controller, Some(ControllerRef::EitherPlayer)) {
                let mut combined = matching;
                let opp_cards = rt.get_field_cards(opponent, location);
                combined.extend(opp_cards.into_iter()
                    .filter(|&cid| rt.card_matches_filter(cid, filter)));
                combined
            } else {
                matching
            }
        }
        TargetExpr::Filter(filter) => {
            // Unscoped filter — search player's field
            let all = rt.get_field_cards(player, type_mapper::LOCATION_ONFIELD);
            all.into_iter()
                .filter(|&cid| rt.card_matches_filter(cid, filter))
                .collect()
        }
    }
}
