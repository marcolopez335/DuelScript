// ============================================================
// DuelScript v2 Compiler
// Compiles v2 AST into engine-consumable effect metadata.
//
// Reuses engine constants from crate::compiler::type_mapper.
// Callback generation (closures) is deferred — this module
// produces the metadata (effect_type, category, code, etc.)
// that the engine needs to register effects.
// ============================================================

use std::sync::Arc;
use super::ast::*;
use super::constants as tm;
use super::runtime::{DuelScriptRuntime, Stat};

// ── Output Types ────────────────────────────────────────────

#[derive(Debug)]
pub struct CompiledCardV2 {
    pub card_id: u64,
    pub name: String,
    pub effects: Vec<CompiledEffectV2>,
}

pub struct CompiledEffectV2 {
    pub label: String,
    pub effect_type: u32,
    pub category: u32,
    pub code: u32,
    pub property: u32,
    pub range: u32,
    pub count_limit: Option<(u32, u32)>,
    pub condition: Option<Arc<dyn Fn(&dyn DuelScriptRuntime) -> bool + Send + Sync>>,
    pub cost: Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>>,
    pub target: Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>>,
    pub operation: Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime) + Send + Sync>>,
}

impl std::fmt::Debug for CompiledEffectV2 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("CompiledEffectV2")
            .field("label", &self.label)
            .field("effect_type", &self.effect_type)
            .field("category", &self.category)
            .field("code", &self.code)
            .finish()
    }
}

// ── Entry Point ─────────────────────────────────────────────

pub fn compile_card_v2(card: &Card) -> CompiledCardV2 {
    let card_id = card.fields.id.unwrap_or(0);
    let mut effects = Vec::new();

    // Summon procedures (materials, special summon procedures)
    if let Some(ref summon) = card.summon {
        effects.extend(compile_summon(summon, card));
    }

    // Passive effects (continuous)
    for passive in &card.passives {
        effects.push(compile_passive(passive, card));
    }

    // Activated/trigger effects
    for effect in &card.effects {
        effects.extend(compile_effect(effect, card));
    }

    // Restriction blocks
    for restriction in &card.restrictions {
        effects.push(compile_restriction(restriction, card));
    }

    // Replacement blocks
    for replacement in &card.replacements {
        effects.push(compile_replacement(replacement, card));
    }

    CompiledCardV2 { card_id, name: card.name.clone(), effects }
}

// ── Card Classification ─────────────────────────────────────

fn is_monster(card: &Card) -> bool {
    card.fields.card_types.iter().any(|t| matches!(t,
        CardType::NormalMonster | CardType::EffectMonster | CardType::RitualMonster
      | CardType::FusionMonster | CardType::SynchroMonster | CardType::XyzMonster
      | CardType::LinkMonster | CardType::PendulumMonster
    ))
}

fn is_spell_trap(card: &Card) -> bool { !is_monster(card) }

fn is_extra_deck(card: &Card) -> bool {
    card.fields.card_types.iter().any(|t| matches!(t,
        CardType::FusionMonster | CardType::SynchroMonster
      | CardType::XyzMonster | CardType::LinkMonster
    ))
}

fn activation_range(card: &Card) -> u32 {
    let types = &card.fields.card_types;
    if types.contains(&CardType::FieldSpell) {
        tm::LOCATION_FZONE
    } else if types.iter().any(|t| matches!(t,
        CardType::NormalSpell | CardType::QuickPlaySpell | CardType::ContinuousSpell
      | CardType::EquipSpell | CardType::RitualSpell
      | CardType::NormalTrap | CardType::CounterTrap | CardType::ContinuousTrap
    )) {
        tm::LOCATION_SZONE
    } else if is_monster(card) {
        tm::LOCATION_MZONE
    } else {
        0
    }
}

// ── Summon Block Compilation ────────────────────────────────

fn compile_summon(summon: &SummonBlock, card: &Card) -> Vec<CompiledEffectV2> {
    let mut effects = Vec::new();
    let is_xyz = card.fields.card_types.contains(&CardType::XyzMonster);

    let has_materials = summon.fusion_materials.is_some()
        || summon.synchro_materials.is_some()
        || summon.xyz_materials.is_some()
        || summon.link_materials.is_some();

    if has_materials {
        // Xyz gets an extra check effect
        if is_xyz {
            effects.push(CompiledEffectV2 {
                label: "Xyz Check".into(),
                effect_type: tm::EFFECT_TYPE_SINGLE,
                category: 0,
                code: 946,
                property: 0,
                range: 0,
                count_limit: None,
                condition: None, cost: None, target: None, operation: None,
            });
        }

        // Summoning procedure effect
        effects.push(CompiledEffectV2 {
            label: "Summon Procedure".into(),
            effect_type: tm::EFFECT_TYPE_FIELD,
            category: 0,
            code: 34,
            property: 0,
            range: tm::LOCATION_EXTRA,
            count_limit: None,
            condition: None, cost: None, target: None, operation: None,
        });
    }

    // Special summon procedure (e.g., Lava Golem)
    if summon.special_summon_procedure.is_some() && !has_materials {
        effects.push(CompiledEffectV2 {
            label: "Special Summon Procedure".into(),
            effect_type: tm::EFFECT_TYPE_FIELD,
            category: tm::CATEGORY_SPECIAL_SUMMON,
            code: 34,
            property: 0,
            range: tm::LOCATION_HAND,
            count_limit: None,
            condition: None, cost: None, target: None, operation: None,
        });
    }

    // cannot_normal_summon → spsummon condition flag
    if summon.cannot_normal_summon && is_extra_deck(card) {
        // Extra deck monsters inherently can't be normal summoned
    } else if summon.cannot_normal_summon {
        effects.push(CompiledEffectV2 {
            label: "Cannot Normal Summon".into(),
            effect_type: tm::EFFECT_TYPE_SINGLE,
            category: 0,
            code: 42,
            property: 0,
            range: 0,
            count_limit: None,
            condition: None, cost: None, target: None, operation: None,
        });
    }

    effects
}

// ── Passive (Continuous) Effect Compilation ──────────────────

fn compile_passive(passive: &Passive, card: &Card) -> CompiledEffectV2 {
    let effect_type = match passive.scope {
        Some(Scope::Field) => tm::EFFECT_TYPE_FIELD,
        _ => tm::EFFECT_TYPE_SINGLE,
    };

    // Determine code from what the passive does
    let code = if !passive.modifiers.is_empty() {
        let first = &passive.modifiers[0];
        match first.stat {
            StatName::Atk => 100, // EFFECT_UPDATE_ATTACK
            StatName::Def => 104, // EFFECT_UPDATE_DEFENSE
        }
    } else if passive.negate_effects {
        2 // EFFECT_DISABLE
    } else if !passive.grants.is_empty() {
        grant_to_code(&passive.grants[0])
    } else if passive.set_atk.is_some() {
        110 // EFFECT_SET_ATTACK
    } else if passive.set_def.is_some() {
        114 // EFFECT_SET_DEFENSE
    } else {
        0
    };

    let range = if is_monster(card) { tm::LOCATION_MZONE } else { tm::LOCATION_SZONE };

    let category = if passive.negate_effects { tm::CATEGORY_DISABLE } else { 0 };

    CompiledEffectV2 {
        label: passive.name.clone(),
        effect_type: effect_type | tm::EFFECT_TYPE_CONTINUOUS,
        category,
        code,
        property: 0,
        range,
        count_limit: None,
        condition: None, cost: None, target: None, operation: None,
    }
}

fn grant_to_code(grant: &GrantAbility) -> u32 {
    match grant {
        GrantAbility::CannotBeDestroyed(Some(DestroyBy::Battle))      => 30, // EFFECT_INDESTRUCTABLE_BATTLE
        GrantAbility::CannotBeDestroyed(Some(DestroyBy::Effect))       => 31, // EFFECT_INDESTRUCTABLE_EFFECT
        GrantAbility::CannotBeDestroyed(Some(DestroyBy::CardEffect))   => 31,
        GrantAbility::CannotBeDestroyed(None)                          => 30,
        GrantAbility::CannotBeTargeted(_)                              => 8,  // EFFECT_CANNOT_BE_EFFECT_TARGET
        GrantAbility::CannotAttack                                     => 16, // EFFECT_CANNOT_ATTACK
        GrantAbility::CannotAttackDirectly                             => 18, // EFFECT_CANNOT_ATTACK_ANNOUNCE
        GrantAbility::CannotChangePosition                             => 20, // EFFECT_CANNOT_CHANGE_POSITION
        GrantAbility::CannotBeTributed                                 => 24, // EFFECT_CANNOT_BE_TRIBUTE
        GrantAbility::CannotBeUsedAsMaterial                           => 25, // EFFECT_CANNOT_BE_SYNCHRO_MATERIAL (approx)
        GrantAbility::CannotActivate(_)                                => 2,  // EFFECT_DISABLE
        GrantAbility::CannotNormalSummon                               => 52, // EFFECT_CANNOT_SUMMON
        GrantAbility::CannotSpecialSummon                              => 56, // EFFECT_CANNOT_SPECIAL_SUMMON
        GrantAbility::Piercing                                         => 96, // EFFECT_PIERCE
        GrantAbility::DirectAttack                                     => 17, // EFFECT_DIRECT_ATTACK
        GrantAbility::DoubleAttack                                     => 19, // EFFECT_DOUBLE_ATTACK
        GrantAbility::TripleAttack                                     => 19, // no separate triple in EDOPro; use DOUBLE_ATTACK code
        GrantAbility::AttackAllMonsters                                => 21, // EFFECT_ATTACK_ALL
        GrantAbility::MustAttack                                       => 22, // EFFECT_MUST_ATTACK
        GrantAbility::ImmuneToTargeting                                => 8,  // shares EFFECT_CANNOT_BE_EFFECT_TARGET
        GrantAbility::UnaffectedBy(src) => match src {
            UnaffectedSource::Spells         => 32,
            UnaffectedSource::Traps          => 33,
            UnaffectedSource::Monsters       => 34,
            UnaffectedSource::Effects        => 35,
            UnaffectedSource::OpponentEffects => 36,
        },
    }
}

/// Maps GrantAbility to an engine effect code, used for runtime grant actions.
fn grant_ability_to_code(ability: &GrantAbility) -> u32 {
    grant_to_code(ability)
}

/// Maps a phase name to an engine phase constant.
fn phase_name_to_code(phase: &PhaseName) -> u32 {
    match phase {
        PhaseName::Draw => tm::PHASE_DRAW,
        PhaseName::Standby => tm::PHASE_STANDBY,
        PhaseName::Main1 => tm::PHASE_MAIN1,
        PhaseName::Battle => tm::PHASE_BATTLE,
        PhaseName::Main2 => tm::PHASE_MAIN2,
        PhaseName::End => tm::PHASE_END,
        PhaseName::Damage | PhaseName::DamageCalculation => tm::PHASE_BATTLE,
    }
}

/// Maps an optional Duration to a numeric code for the engine.
fn duration_to_code(dur: &Option<Duration>) -> u32 {
    match dur {
        Some(Duration::ThisTurn) | Some(Duration::EndOfTurn) => 1,
        Some(Duration::EndPhase) => 2,
        Some(Duration::WhileOnField) | Some(Duration::WhileFaceUp) => 3,
        Some(Duration::Permanently) => 0,
        None => 0,
        _ => 1,
    }
}

// ── Activated Effect Compilation ────────────────────────────

fn compile_effect(effect: &Effect, card: &Card) -> Vec<CompiledEffectV2> {
    let speed = effect.speed.unwrap_or(1);
    let has_trigger = effect.trigger.is_some();

    // Determine effect_type
    let effect_type = if is_spell_trap(card) {
        tm::EFFECT_TYPE_ACTIVATE
    } else if speed >= 2 {
        // Speed 2+ on monsters = Quick Effect (even with triggers)
        tm::EFFECT_TYPE_QUICK_O
    } else if has_trigger {
        if effect.mandatory {
            tm::EFFECT_TYPE_TRIGGER_F
        } else {
            tm::EFFECT_TYPE_TRIGGER_O
        }
    } else {
        tm::EFFECT_TYPE_IGNITION
    };

    // For monster trigger effects, add SINGLE (self-trigger) or FIELD (watches field)
    // Quick effects don't need this — they activate from the field directly
    let is_trigger_type = effect_type & (tm::EFFECT_TYPE_TRIGGER_O | tm::EFFECT_TYPE_TRIGGER_F) != 0;
    let effect_type = if is_monster(card) && is_trigger_type && has_trigger {
        let is_self_trigger = matches!(&effect.trigger, Some(
            Trigger::DestroyedByBattle | Trigger::DestroyedByEffect
          | Trigger::Destroyed(_) | Trigger::Targeted
          | Trigger::Flipped | Trigger::FlipSummoned
          | Trigger::TributeSummoned | Trigger::NormalSummoned
          | Trigger::SentTo(_, _) | Trigger::LeavesField
          | Trigger::Banished
          | Trigger::DirectAttackDamage
        ));
        if is_self_trigger {
            effect_type | tm::EFFECT_TYPE_SINGLE
        } else {
            effect_type | tm::EFFECT_TYPE_FIELD
        }
    } else {
        effect_type
    };

    // Event code
    let code = if effect_type == tm::EFFECT_TYPE_IGNITION {
        0 // ignition effects have code=0
    } else {
        trigger_to_event_code(&effect.trigger)
    };

    // Range
    let range = if effect_type & tm::EFFECT_TYPE_SINGLE != 0
        && effect_type & (tm::EFFECT_TYPE_TRIGGER_O | tm::EFFECT_TYPE_TRIGGER_F) != 0
    {
        0 // self-trigger range is 0
    } else {
        activation_range(card)
    };

    // Category from actions
    let category = if effect_type == tm::EFFECT_TYPE_IGNITION {
        0 // ignition: engine sets category dynamically
    } else {
        categories_from_actions(&effect.resolve)
    };

    // Property
    let mut property = 0u32;
    if effect.target.is_some() {
        property |= tm::EFFECT_FLAG_CARD_TARGET;
    }

    // Count limit
    let card_id = card.fields.id.unwrap_or(0);
    let count_limit = frequency_to_count_limit(&effect.frequency, card_id);

    // Generate callbacks
    let (condition, cost, target, operation) = build_effect_callbacks(effect, card);

    // Check if this is a "negate summon" pattern — needs expansion to 3 events
    if is_spell_trap(card) && matches!(&effect.trigger, Some(Trigger::SummonAttempt)) {
        let events = [tm::EVENT_SUMMON, tm::EVENT_FLIP_SUMMON, tm::EVENT_SPSUMMON];
        return events.iter().map(|&evt| CompiledEffectV2 {
            label: effect.name.clone(),
            effect_type,
            category,
            code: evt,
            property,
            range,
            count_limit,
            condition: condition.clone(),
            cost: cost.clone(),
            target: target.clone(),
            operation: operation.clone(),
        }).collect();
    }

    vec![CompiledEffectV2 {
        label: effect.name.clone(),
        effect_type,
        category,
        code,
        property,
        range,
        count_limit,
        condition,
        cost,
        target,
        operation,
    }]
}

// ── Restriction Compilation ─────────────────────────────────

fn compile_restriction(restriction: &Restriction, card: &Card) -> CompiledEffectV2 {
    let code = restriction.abilities.first()
        .map(|a| grant_to_code(a))
        .unwrap_or(0);

    CompiledEffectV2 {
        label: restriction.name.clone().unwrap_or_else(|| "restriction".into()),
        effect_type: tm::EFFECT_TYPE_SINGLE | tm::EFFECT_TYPE_CONTINUOUS,
        category: 0,
        code,
        property: 0,
        range: if is_monster(card) { tm::LOCATION_MZONE } else { tm::LOCATION_SZONE },
        count_limit: None,
        condition: None, cost: None, target: None, operation: None,
    }
}

// ── Replacement Compilation ─────────────────────────────────

fn compile_replacement(replacement: &Replacement, card: &Card) -> CompiledEffectV2 {
    let code = match &replacement.instead_of {
        ReplaceableEvent::DestroyedByBattle | ReplaceableEvent::DestroyedByEffect
      | ReplaceableEvent::Destroyed => 0x1000014, // EFFECT_DESTROY_REPLACE
        ReplaceableEvent::SentToGy => 0x1000015,
        ReplaceableEvent::Banished => 0x1000016,
        ReplaceableEvent::ReturnedToHand => 0x1000017,
        ReplaceableEvent::ReturnedToDeck => 0x1000018,
        ReplaceableEvent::LeavesField => 0x1000014,
    };

    CompiledEffectV2 {
        label: replacement.name.clone().unwrap_or_else(|| "replacement".into()),
        effect_type: tm::EFFECT_TYPE_SINGLE | tm::EFFECT_TYPE_CONTINUOUS,
        category: tm::CATEGORY_DESTROY,
        code,
        property: 0,
        range: if is_monster(card) { tm::LOCATION_MZONE } else { tm::LOCATION_SZONE },
        count_limit: None,
        condition: None, cost: None, target: None, operation: None,
    }
}

// ── Trigger → Event Code ────────────────────────────────────

fn trigger_to_event_code(trigger: &Option<Trigger>) -> u32 {
    match trigger {
        None => tm::EVENT_FREE_CHAIN,
        Some(t) => match t {
            Trigger::Summoned(_) | Trigger::SpecialSummoned(_) => tm::EVENT_SPSUMMON_SUCCESS,
            Trigger::NormalSummoned | Trigger::TributeSummoned => tm::EVENT_SUMMON_SUCCESS,
            Trigger::FlipSummoned => tm::EVENT_FLIP_SUMMON_SUCCESS,
            Trigger::Flipped => tm::EVENT_FLIP,
            Trigger::Destroyed(_) | Trigger::DestroyedByBattle | Trigger::DestroyedByEffect
                => tm::EVENT_DESTROYED,
            Trigger::DestroysByBattle => tm::EVENT_ATTACK_ANNOUNCE,
            Trigger::SentTo(Zone::Gy, _) => tm::EVENT_TO_GRAVE,
            Trigger::SentTo(_, _) => 0,
            Trigger::LeavesField => tm::EVENT_TO_GRAVE,
            Trigger::Banished => tm::EVENT_REMOVE,
            Trigger::ReturnedTo(_) => tm::EVENT_TO_HAND,
            Trigger::AttackDeclared | Trigger::OpponentAttackDeclared
                => tm::EVENT_ATTACK_ANNOUNCE,
            Trigger::Attacked => tm::EVENT_BE_BATTLE_TARGET,
            Trigger::BattleDamage(_) | Trigger::DirectAttackDamage => 1132, // EVENT_BATTLE_DAMAGE
            Trigger::DamageCalculation => 1134, // EVENT_DAMAGE_CALCULATING
            Trigger::StandbyPhase(_) => tm::EVENT_PHASE | tm::PHASE_STANDBY,
            Trigger::EndPhase => tm::EVENT_PHASE | tm::PHASE_END,
            Trigger::DrawPhase => tm::EVENT_PHASE | tm::PHASE_DRAW,
            Trigger::MainPhase => tm::EVENT_PHASE | tm::PHASE_MAIN1,
            Trigger::BattlePhase => tm::EVENT_PHASE | tm::PHASE_BATTLE,
            Trigger::SummonAttempt => tm::EVENT_SUMMON,
            Trigger::SpellTrapActivated | Trigger::OpponentActivates(_)
                => tm::EVENT_CHAINING,
            Trigger::ChainLink => tm::EVENT_CHAINING,
            Trigger::Targeted => 0, // special engine handling
            Trigger::UsedAsMaterial(_) => tm::EVENT_RELEASE,
            Trigger::PositionChanged => 0, // engine-specific; no standard event
            Trigger::ControlChanged  => 0, // engine-specific
            Trigger::Equipped        => 0, // engine-specific
            Trigger::Unequipped      => 0, // engine-specific
            Trigger::Custom(_)       => 0, // user-defined event
        }
    }
}

// ── Action → Category Flags ─────────────────────────────────

fn categories_from_actions(actions: &[Action]) -> u32 {
    let mut cat = 0u32;
    for action in actions {
        cat |= action_category(action);
    }
    cat
}

fn action_category(action: &Action) -> u32 {
    match action {
        Action::Draw(_) => tm::CATEGORY_DRAW,
        Action::Discard(_) => tm::CATEGORY_HANDES,
        Action::Destroy(_) => tm::CATEGORY_DESTROY,
        Action::Banish(_, _, _) => tm::CATEGORY_REMOVE,
        Action::Send(_, Zone::Gy) => tm::CATEGORY_TOGRAVE,
        Action::Send(_, _) => 0,
        Action::Return(_, ReturnDest::Hand) => tm::CATEGORY_TOHAND,
        Action::Return(_, ReturnDest::Deck(_)) => tm::CATEGORY_TODECK,
        Action::Return(_, ReturnDest::ExtraDeck) => tm::CATEGORY_TODECK,
        Action::Search(_, _) => tm::CATEGORY_SEARCH,
        Action::AddToHand(_, _) => tm::CATEGORY_TOHAND,
        Action::SpecialSummon(_, _, _) => tm::CATEGORY_SPECIAL_SUMMON,
        Action::NormalSummon(_) => tm::CATEGORY_SUMMON,
        Action::Damage(_, _) => tm::CATEGORY_DAMAGE,
        Action::GainLp(_) => tm::CATEGORY_RECOVER,
        Action::PayLp(_) => 0,
        Action::Negate(_) => tm::CATEGORY_NEGATE,
        Action::NegateEffects(_, _) => tm::CATEGORY_DISABLE,
        Action::CreateToken(_) => tm::CATEGORY_TOKEN,
        Action::TakeControl(_, _) => tm::CATEGORY_CONTROL,
        Action::ChangePosition(_, _) => tm::CATEGORY_POSITION,
        Action::Equip(_, _) => tm::CATEGORY_EQUIP,
        Action::ModifyStat(StatName::Atk, _, _, _, _) => tm::CATEGORY_ATKCHANGE,
        Action::ModifyStat(StatName::Def, _, _, _, _) => tm::CATEGORY_DEFCHANGE,
        Action::SetStat(StatName::Atk, _, _, _) => tm::CATEGORY_ATKCHANGE,
        Action::SetStat(StatName::Def, _, _, _) => tm::CATEGORY_DEFCHANGE,
        Action::PlaceCounter(_, _, _) | Action::RemoveCounter(_, _, _) => tm::CATEGORY_COUNTER,
        Action::Mill(_, _) => tm::CATEGORY_DECKDES,
        // Compound actions — recurse
        Action::If { then, otherwise, .. } => {
            categories_from_actions(then) | categories_from_actions(otherwise)
        }
        Action::Choose(block) => {
            block.options.iter().fold(0, |acc, opt| acc | categories_from_actions(&opt.resolve))
        }
        Action::Then(actions) | Action::Also(actions) | Action::AndIfYouDo(actions)
            => categories_from_actions(actions),
        Action::Grant(_, _, _)         => 0,
        // Compound actions handled separately above or not categorised
        Action::ForEach { body, .. }   => categories_from_actions(body),
        Action::Delayed { body, .. }   => categories_from_actions(body),
        Action::InstallWatcher { .. }  => 0,
        Action::Set(_, _)              => 0,
        Action::FlipDown(_)            => 0,
        Action::ChangeLevel(_, _)      => 0,
        Action::ChangeAttribute(_, _)  => 0,
        Action::ChangeRace(_, _)       => 0,
        Action::ChangeName(_, _, _)    => 0,
        Action::SetScale(_, _)         => 0,
        Action::Attach(_, _)           => 0,
        Action::Detach(_, _)           => 0,
        Action::LookAt(_, _)           => 0,
        Action::Excavate(_, _)         => 0,
        Action::ShuffleDeck(_)         => 0,
        Action::Reveal(_)              => 0,
        Action::Announce(_, _)         => 0,
        Action::CoinFlip { heads, tails } => {
            categories_from_actions(heads) | categories_from_actions(tails)
        }
        Action::DiceRoll(branches)     => categories_from_actions(branches),
        Action::LinkTo(_, _)           => 0,
        Action::RitualSummon { .. }    => tm::CATEGORY_SPECIAL_SUMMON,
        Action::FusionSummon { .. }    => tm::CATEGORY_SPECIAL_SUMMON | tm::CATEGORY_FUSION_SUMMON,
        Action::SynchroSummon { .. }   => tm::CATEGORY_SPECIAL_SUMMON,
        Action::XyzSummon { .. }       => tm::CATEGORY_SPECIAL_SUMMON,
    }
}

// ── Frequency → Count Limit ─────────────────────────────────

fn frequency_to_count_limit(freq: &Option<Frequency>, card_id: u64) -> Option<(u32, u32)> {
    match freq {
        None => None,
        Some(Frequency::OncePerTurn(OptKind::Hard)) => Some((1, card_id as u32)),
        Some(Frequency::OncePerTurn(OptKind::Soft)) => Some((1, 0)),
        Some(Frequency::TwicePerTurn) => Some((2, card_id as u32)),
        Some(Frequency::OncePerDuel) => Some((1, card_id as u32 | 0x10000000)),
    }
}

// ── Expression Evaluation ────────────────────────────────────

fn eval_v2_expr(expr: &Expr, rt: &dyn DuelScriptRuntime) -> i32 {
    match expr {
        Expr::Literal(n) => *n,
        Expr::Half => rt.get_lp(rt.effect_player()) / 2,
        Expr::StatRef(entity, field) => {
            let card_id = match entity.as_str() {
                "self" => rt.effect_card_id(),
                _ => rt.effect_card_id(), // fallback
            };
            stat_field_to_value(field, card_id, rt)
        }
        Expr::BindingRef(name, field) => {
            rt.get_binding_field(name, &format!("{:?}", field).to_lowercase())
        }
        Expr::PlayerLp(owner) => {
            let player = match owner {
                LpOwner::Your | LpOwner::Controller => rt.effect_player(),
                LpOwner::Opponent => 1 - rt.effect_player(),
            };
            rt.get_lp(player)
        }
        Expr::Count(selector) => {
            resolve_v2_selector(selector, rt, rt.effect_player()).len() as i32
        }
        Expr::BinOp { left, op, right } => {
            let l = eval_v2_expr(left, rt);
            let r = eval_v2_expr(right, rt);
            match op {
                BinOp::Add => l.saturating_add(r),
                BinOp::Sub => l.saturating_sub(r),
                BinOp::Mul => l.saturating_mul(r),
                BinOp::Div => if r == 0 { 0 } else { l / r },
            }
        }
    }
}

fn stat_field_to_value(field: &StatField, card_id: u32, rt: &dyn DuelScriptRuntime) -> i32 {
    let stat = match field {
        StatField::Atk | StatField::BaseAtk | StatField::OriginalAtk
            => &Stat::Atk,
        StatField::Def | StatField::BaseDef | StatField::OriginalDef
            => &Stat::Def,
        StatField::Level => &Stat::Level,
        StatField::Rank => &Stat::Rank,
        _ => return 0,
    };
    rt.get_card_stat(card_id, stat)
}

// ── Selector Resolution ─────────────────────────────────────

fn resolve_v2_selector(sel: &Selector, rt: &dyn DuelScriptRuntime, player: u8) -> Vec<u32> {
    let opponent = 1 - player;
    match sel {
        Selector::SelfCard => vec![rt.effect_card_id()],
        Selector::Counted { controller, zone, .. } => {
            let ctrl_player = match controller {
                Some(Controller::You) => Some(player),
                Some(Controller::Opponent) => Some(opponent),
                Some(Controller::Either) | None => None, // both
            };
            let location = match zone {
                Some(ZoneFilter::In(z)) | Some(ZoneFilter::From(z)) => zone_to_location(z),
                Some(ZoneFilter::OnField(_)) => tm::LOCATION_ONFIELD,
                None => tm::LOCATION_ONFIELD,
            };
            let mut cards = Vec::new();
            if let Some(p) = ctrl_player {
                cards.extend(rt.get_field_cards(p, location));
            } else {
                cards.extend(rt.get_field_cards(player, location));
                cards.extend(rt.get_field_cards(opponent, location));
            }
            cards
        }
        _ => vec![], // Target, Searched, etc. — engine tracks these
    }
}

fn zone_to_location(zone: &Zone) -> u32 {
    match zone {
        Zone::Hand           => tm::LOCATION_HAND,
        Zone::Field          => tm::LOCATION_ONFIELD,
        Zone::Deck           => tm::LOCATION_DECK,
        Zone::ExtraDeck
        | Zone::ExtraDeckFaceUp => tm::LOCATION_EXTRA,
        Zone::Gy             => tm::LOCATION_GRAVE,
        Zone::Banished       => tm::LOCATION_REMOVED,
        Zone::MonsterZone    => tm::LOCATION_MZONE,
        Zone::SpellTrapZone  => tm::LOCATION_SZONE,
        Zone::FieldZone      => tm::LOCATION_FZONE,
        Zone::PendulumZone   => tm::LOCATION_PZONE,
        Zone::ExtraMonsterZone => tm::LOCATION_MZONE, // extra monster zone is still a monster zone
        Zone::Overlay        => 0, // overlay/xyz materials have no location constant
        Zone::Equipped       => tm::LOCATION_SZONE, // equipped cards live in spell/trap zone
        Zone::TopOfDeck
        | Zone::BottomOfDeck => tm::LOCATION_DECK,
    }
}

fn player_who_to_idx(who: &PlayerWho, player: u8) -> u8 {
    match who {
        PlayerWho::You | PlayerWho::Controller => player,
        PlayerWho::Opponent => 1 - player,
        PlayerWho::Both     => player, // caller handles both
        PlayerWho::Owner    => player, // owner == controller in most contexts
        PlayerWho::Summoner => player, // summoner == controller in most contexts
    }
}

fn attribute_to_engine(attr: &Attribute) -> u32 {
    match attr {
        Attribute::Light  => 0x10,
        Attribute::Dark   => 0x20,
        Attribute::Fire   => 0x40,
        Attribute::Water  => 0x80,
        Attribute::Earth  => 0x100,
        Attribute::Wind   => 0x200,
        Attribute::Divine => 0x400,
    }
}

fn race_to_engine(race: &Race) -> u32 {
    match race {
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
        Race::Wyrm         => 0x800000,
        Race::Cyberse      => 0x1000000,
        Race::Illusion     => 0x2000000,
    }
}

// ── Callback Generation ─────────────────────────────────────

fn gen_condition(effect: &Effect) -> Option<Arc<dyn Fn(&dyn DuelScriptRuntime) -> bool + Send + Sync>> {
    let cond = effect.condition.clone();
    let trigger = effect.trigger.clone();

    if cond.is_none() && trigger.is_none() {
        return None;
    }

    Some(Arc::new(move |rt: &dyn DuelScriptRuntime| {
        if let Some(ref c) = cond {
            if !eval_v2_condition(c, rt) {
                return false;
            }
        }
        // Trigger-based conditions (e.g., opponent_activates checks event categories)
        if let Some(Trigger::OpponentActivates(ref cats)) = trigger {
            if !cats.is_empty() {
                let event_cats = rt.event_categories();
                let matched = cats.iter().any(|cat| {
                    let engine_cat = category_to_engine(cat);
                    event_cats & engine_cat != 0
                });
                if !matched { return false; }
            }
        }
        true
    }))
}

fn eval_v2_condition(cond: &Condition, rt: &dyn DuelScriptRuntime) -> bool {
    match cond {
        Condition::Single(atom) => eval_v2_condition_atom(atom, rt),
        Condition::And(atoms) => atoms.iter().all(|a| eval_v2_condition_atom(a, rt)),
        Condition::Or(atoms) => atoms.iter().any(|a| eval_v2_condition_atom(a, rt)),
    }
}

fn eval_v2_condition_atom(atom: &ConditionAtom, rt: &dyn DuelScriptRuntime) -> bool {
    let player = rt.effect_player();
    let opponent = 1 - player;
    match atom {
        ConditionAtom::Not(inner) => !eval_v2_condition_atom(inner, rt),
        ConditionAtom::OnField => true,
        ConditionAtom::InGy | ConditionAtom::InHand | ConditionAtom::InBanished => true,
        ConditionAtom::LpCompare(op, expr) => {
            compare_i32(rt.get_lp(player), op, eval_v2_expr(expr, rt))
        }
        ConditionAtom::HandSize(op, expr) => {
            compare_i32(rt.get_hand_count(player) as i32, op, eval_v2_expr(expr, rt))
        }
        ConditionAtom::CardsInGy(op, expr) => {
            compare_i32(rt.get_gy_count(player) as i32, op, eval_v2_expr(expr, rt))
        }
        ConditionAtom::Controls(who, _sel) => {
            let p = player_who_to_idx(who, player);
            rt.get_field_card_count(p, tm::LOCATION_MZONE) > 0
        }
        ConditionAtom::NoCardsOnField(_, owner) => {
            let p = match owner {
                FieldOwner::Your => player,
                FieldOwner::Opponent => opponent,
                FieldOwner::Either => return
                    rt.get_field_card_count(player, tm::LOCATION_MZONE) == 0
                    && rt.get_field_card_count(opponent, tm::LOCATION_MZONE) == 0,
            };
            rt.get_field_card_count(p, tm::LOCATION_MZONE) == 0
        }
        ConditionAtom::OpponentLpCompare(op, expr) => {
            compare_i32(rt.get_lp(opponent), op, eval_v2_expr(expr, rt))
        }
        ConditionAtom::CardsInBanished(op, expr) => {
            compare_i32(rt.get_banished_count(player) as i32, op, eval_v2_expr(expr, rt))
        }
        ConditionAtom::SelfState(state) => {
            let card_id = rt.effect_card_id();
            match state {
                CardState::SummonedThisTurn  => rt.was_summoned_this_turn(card_id),
                CardState::AttackedThisTurn  => rt.has_attacked_this_turn(card_id),
                CardState::FlippedThisTurn   => rt.was_flipped_this_turn(card_id),
                CardState::ActivatedThisTurn => rt.was_summoned_this_turn(card_id), // proxy
                CardState::FaceUp            => rt.is_face_up(card_id),
                CardState::FaceDown          => rt.is_face_down(card_id),
                CardState::InAttackPosition  => rt.is_attack_position(card_id),
                CardState::InDefensePosition => rt.is_defense_position(card_id),
            }
        }
        ConditionAtom::PhaseIs(phase) => {
            rt.get_current_phase() == phase_name_to_code(phase)
        }
        ConditionAtom::ChainIncludes(cats) => {
            cats.iter().any(|cat| {
                let engine_cat = category_to_engine(cat);
                rt.chain_includes_category(engine_cat)
            })
        }
        ConditionAtom::HasCounter(name, _target) => {
            let card_id = rt.effect_card_id();
            rt.has_counter(card_id, name)
        }
        ConditionAtom::HasFlag(name) => {
            let card_id = rt.effect_card_id();
            rt.has_flag(card_id, name)
        }
    }
}

fn compare_i32(actual: i32, op: &CompareOp, expected: i32) -> bool {
    match op {
        CompareOp::Gte => actual >= expected,
        CompareOp::Lte => actual <= expected,
        CompareOp::Eq => actual == expected,
        CompareOp::Neq => actual != expected,
        CompareOp::Gt => actual > expected,
        CompareOp::Lt => actual < expected,
    }
}

fn category_to_engine(cat: &Category) -> u32 {
    match cat {
        Category::Search               => tm::CATEGORY_SEARCH,
        Category::SpecialSummon        => tm::CATEGORY_SPECIAL_SUMMON,
        Category::SendToGy             => tm::CATEGORY_TOGRAVE,
        Category::AddToHand            => tm::CATEGORY_TOHAND,
        Category::Draw                 => tm::CATEGORY_DRAW,
        Category::Banish               => tm::CATEGORY_REMOVE,
        Category::Destroy              => tm::CATEGORY_DESTROY,
        Category::Negate               => tm::CATEGORY_NEGATE,
        Category::Mill                 => tm::CATEGORY_DECKDES,
        Category::ActivateSpell        => 0,
        Category::ActivateTrap         => 0,
        Category::ActivateMonsterEffect => 0,
        Category::NormalSummon         => tm::CATEGORY_SUMMON,
        Category::FusionSummon
        | Category::SynchroSummon
        | Category::XyzSummon
        | Category::LinkSummon
        | Category::RitualSummon       => tm::CATEGORY_SPECIAL_SUMMON,
        Category::AttackDeclared       => 0,
    }
}

fn gen_cost(costs: &[CostAction], card_id: u64) -> Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>> {
    if costs.is_empty() {
        return None;
    }
    let costs = costs.to_vec();
    let _cid = card_id as u32;

    Some(Arc::new(move |rt: &mut dyn DuelScriptRuntime, check_only: bool| {
        let player = rt.effect_player();
        for cost in &costs {
            match cost {
                CostAction::PayLp(expr) => {
                    let amount = eval_v2_expr(expr, rt);
                    if check_only {
                        if rt.get_lp(player) < amount { return false; }
                    } else {
                        rt.damage(player, amount);
                    }
                }
                CostAction::Discard(sel, binding) => {
                    if check_only {
                        return rt.get_hand_count(player) > 0;
                    }
                    let hand = rt.get_field_cards(player, tm::LOCATION_HAND);
                    if !hand.is_empty() {
                        let selected = rt.select_cards(player, &hand, 1, 1);
                        if !selected.is_empty() {
                            rt.discard(&selected);
                            if let Some(name) = binding {
                                rt.bind_last_selection(name);
                            }
                        }
                    }
                    let _ = sel; // selector used for engine-level filtering
                }
                CostAction::Tribute(sel, binding) => {
                    if check_only {
                        return rt.get_field_card_count(player, tm::LOCATION_MZONE) > 0;
                    }
                    let monsters = rt.get_field_cards(player, tm::LOCATION_MZONE);
                    if !monsters.is_empty() {
                        let selected = rt.select_cards(player, &monsters, 1, 1);
                        rt.tribute(&selected);
                        if let Some(name) = binding {
                            rt.bind_last_selection(name);
                        }
                    }
                    let _ = sel;
                }
                CostAction::Detach(count, _sel) => {
                    if check_only { return true; }
                    let card_id = rt.effect_card_id();
                    rt.detach_material(card_id, *count as u32);
                }
                CostAction::Banish(sel, _zone, binding) => {
                    if check_only { return true; }
                    let player = rt.effect_player();
                    // Gather candidates from hand and field
                    let mut candidates = rt.get_field_cards(player, tm::LOCATION_HAND);
                    candidates.extend(rt.get_field_cards(player, tm::LOCATION_MZONE));
                    let _ = sel; // filter hint for engine-level use
                    if !candidates.is_empty() {
                        let selected = rt.select_cards(player, &candidates, 1, 1);
                        rt.banish(&selected);
                        if let Some(name) = binding {
                            rt.bind_last_selection(name);
                        }
                    }
                }
                CostAction::Send(sel, zone, binding) => {
                    if check_only { return true; }
                    let player = rt.effect_player();
                    let loc = zone_to_location(zone);
                    let mut candidates = rt.get_field_cards(player, tm::LOCATION_HAND);
                    candidates.extend(rt.get_field_cards(player, tm::LOCATION_MZONE));
                    let _ = sel;
                    if !candidates.is_empty() {
                        let selected = rt.select_cards(player, &candidates, 1, 1);
                        if loc == tm::LOCATION_GRAVE {
                            rt.send_to_grave(&selected);
                        } else {
                            rt.send_to_deck(&selected, true);
                        }
                        if let Some(name) = binding {
                            rt.bind_last_selection(name);
                        }
                    }
                }
                CostAction::RemoveCounter(name, count, sel) => {
                    if check_only { return true; }
                    let player = rt.effect_player();
                    let card_id = rt.effect_card_id();
                    let candidates = rt.get_field_cards(player, tm::LOCATION_MZONE);
                    let _ = sel;
                    let target = candidates.first().copied().unwrap_or(card_id);
                    rt.remove_counter(target, name, *count as u32);
                }
                CostAction::Reveal(sel) => {
                    if check_only { return true; }
                    let player = rt.effect_player();
                    let cards = rt.get_field_cards(player, tm::LOCATION_HAND);
                    let _ = sel;
                    rt.reveal(&cards);
                }
                CostAction::Announce(what, binding) => {
                    if check_only { return true; }
                    let player = rt.effect_player();
                    let kind: u8 = match what {
                        AnnounceWhat::Type      => 3,
                        AnnounceWhat::Attribute => 1,
                        AnnounceWhat::Race      => 2,
                        AnnounceWhat::Level     => 4,
                        AnnounceWhat::Card      => 0,
                    };
                    let token = rt.announce(player, kind, 0);
                    if let Some(name) = binding {
                        rt.set_binding(name, token);
                    }
                }
                CostAction::None => {}
            }
        }
        true
    }))
}

fn gen_target(target: &Option<TargetDecl>) -> Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>> {
    let target = target.clone()?;

    Some(Arc::new(move |rt: &mut dyn DuelScriptRuntime, check_only: bool| {
        let player = rt.effect_player();
        let cards = resolve_v2_selector(&target.selector, rt, player);
        if check_only {
            return !cards.is_empty();
        }
        if !cards.is_empty() {
            let min = 1;
            let max = 1;
            let selected = rt.select_cards(player, &cards, min, max);
            rt.set_targets(&selected);
        }
        true
    }))
}

fn gen_operation(actions: &[Action]) -> Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime) + Send + Sync>> {
    if actions.is_empty() {
        return None;
    }
    let actions = actions.to_vec();

    Some(Arc::new(move |rt: &mut dyn DuelScriptRuntime| {
        let player = rt.effect_player();
        for action in &actions {
            execute_v2_action(action, rt, player);
        }
    }))
}

fn execute_v2_action(action: &Action, rt: &mut dyn DuelScriptRuntime, player: u8) {
    match action {
        Action::Draw(expr) => {
            let n = eval_v2_expr(expr, rt) as u32;
            rt.draw(player, n);
        }
        Action::Discard(sel) => {
            let cards = resolve_v2_selector(sel, rt, player);
            if !cards.is_empty() {
                rt.discard(&cards);
            }
        }
        Action::Destroy(sel) => {
            let cards = resolve_v2_selector(sel, rt, player);
            if !cards.is_empty() {
                rt.destroy(&cards);
            }
        }
        Action::Send(sel, zone) => {
            let cards = resolve_v2_selector(sel, rt, player);
            match zone {
                Zone::Gy => { rt.send_to_grave(&cards); }
                Zone::Hand => { rt.send_to_hand(&cards); }
                Zone::Deck => { rt.send_to_deck(&cards, true); }
                _ => {}
            }
        }
        Action::Banish(sel, _, _) => {
            let cards = resolve_v2_selector(sel, rt, player);
            rt.banish(&cards);
        }
        Action::Search(sel, _zone) => {
            let cards = resolve_v2_selector(sel, rt, player);
            if !cards.is_empty() {
                let selected = rt.select_cards(player, &cards, 1, 1);
                rt.send_to_hand(&selected);
            }
        }
        Action::AddToHand(sel, _) => {
            let cards = resolve_v2_selector(sel, rt, player);
            if !cards.is_empty() {
                rt.send_to_hand(&cards);
            }
        }
        Action::SpecialSummon(sel, _, pos) => {
            let pos_val = match pos {
                Some(BattlePosition::Attack) => 0x1,
                Some(BattlePosition::Defense) => 0x2,
                Some(BattlePosition::FaceDownDefense) => 0xA,
                None => 0x1,
            };
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.special_summon(card_id, player, pos_val);
            }
        }
        Action::Negate(and_destroy) => {
            rt.negate_activation();
            if *and_destroy {
                rt.negate_effect();
            }
        }
        Action::NegateEffects(sel, _) => {
            let _ = resolve_v2_selector(sel, rt, player);
            rt.negate_effect();
        }
        Action::Damage(who, expr) => {
            let amount = eval_v2_expr(expr, rt);
            let target = player_who_to_idx(who, player);
            rt.damage(target, amount);
        }
        Action::GainLp(expr) => {
            let amount = eval_v2_expr(expr, rt);
            rt.recover(player, amount);
        }
        Action::PayLp(expr) => {
            let amount = eval_v2_expr(expr, rt);
            rt.damage(player, amount);
        }
        Action::FlipDown(sel) => {
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.change_position(card_id);
            }
        }
        Action::ChangePosition(sel, _) => {
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.change_position(card_id);
            }
        }
        Action::TakeControl(sel, _) => {
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.take_control(card_id, player);
            }
        }
        Action::Equip(card_sel, target_sel) => {
            let equip_cards = resolve_v2_selector(card_sel, rt, player);
            let target_cards = resolve_v2_selector(target_sel, rt, player);
            if let (Some(&equip_id), Some(&target_id)) = (equip_cards.first(), target_cards.first()) {
                rt.equip_card(equip_id, target_id);
            }
        }
        Action::ModifyStat(stat, sel, is_negative, expr, _) => {
            let cards = resolve_v2_selector(sel, rt, player);
            let val = eval_v2_expr(expr, rt);
            let delta = if *is_negative { -val } else { val };
            for card_id in cards {
                match stat {
                    StatName::Atk => rt.modify_atk(card_id, delta),
                    StatName::Def => rt.modify_def(card_id, delta),
                }
            }
        }
        Action::SetStat(stat, sel, expr, _) => {
            let cards = resolve_v2_selector(sel, rt, player);
            let val = eval_v2_expr(expr, rt);
            for card_id in cards {
                match stat {
                    StatName::Atk => rt.set_atk(card_id, val),
                    StatName::Def => rt.set_def(card_id, val),
                }
            }
        }
        Action::CreateToken(spec) => {
            let atk = match &spec.atk { StatVal::Number(n) => *n, _ => 0 };
            let def = match &spec.def { StatVal::Number(n) => *n, _ => 0 };
            rt.create_token(player, atk, def, spec.count);
        }
        Action::Return(sel, dest) => {
            let cards = resolve_v2_selector(sel, rt, player);
            match dest {
                ReturnDest::Hand => { rt.return_to_hand(&cards); }
                ReturnDest::Deck(pos) => {
                    let top = !matches!(pos, Some(DeckPosition::Bottom));
                    rt.send_to_deck(&cards, top);
                    if matches!(pos, Some(DeckPosition::Shuffle)) {
                        rt.shuffle_deck(player);
                    }
                }
                ReturnDest::ExtraDeck => { rt.send_to_deck(&cards, false); }
            }
        }
        Action::Grant(sel, ability, duration) => {
            let cards = resolve_v2_selector(sel, rt, player);
            let grant_code = grant_ability_to_code(ability);
            let dur_code = duration_to_code(duration);
            for card_id in cards {
                rt.register_grant(card_id, grant_code, dur_code);
            }
        }
        Action::If { condition, then, otherwise } => {
            if eval_v2_condition(condition, rt) {
                for a in then { execute_v2_action(a, rt, player); }
            } else {
                for a in otherwise { execute_v2_action(a, rt, player); }
            }
        }
        Action::Then(actions) | Action::Also(actions) | Action::AndIfYouDo(actions) => {
            for a in actions { execute_v2_action(a, rt, player); }
        }
        Action::ChangeLevel(sel, expr) => {
            let cards = resolve_v2_selector(sel, rt, player);
            let val = eval_v2_expr(expr, rt) as u32;
            for card_id in cards {
                rt.change_level(card_id, val);
            }
        }
        Action::ChangeAttribute(sel, attr) => {
            let cards = resolve_v2_selector(sel, rt, player);
            let attr_val = attribute_to_engine(attr);
            for card_id in cards {
                rt.change_attribute(card_id, attr_val);
            }
        }
        Action::ChangeRace(sel, race) => {
            let cards = resolve_v2_selector(sel, rt, player);
            let race_val = race_to_engine(race);
            for card_id in cards {
                rt.change_race(card_id, race_val);
            }
        }
        Action::ChangeName(sel, name, _duration) => {
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.change_name(card_id, name, 0);
            }
        }
        Action::SetScale(sel, expr) => {
            let cards = resolve_v2_selector(sel, rt, player);
            let val = eval_v2_expr(expr, rt) as u32;
            for card_id in cards {
                rt.set_scale(card_id, val);
            }
        }
        Action::PlaceCounter(name, count, sel) => {
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.place_counter(card_id, name, *count as u32);
            }
        }
        Action::RemoveCounter(name, count, sel) => {
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.remove_counter(card_id, name, *count as u32);
            }
        }
        Action::Attach(material_sel, target_sel) => {
            let materials = resolve_v2_selector(material_sel, rt, player);
            let targets = resolve_v2_selector(target_sel, rt, player);
            if let Some(&target_id) = targets.first() {
                for mat_id in materials {
                    rt.attach_material(mat_id, target_id);
                }
            }
        }
        Action::Detach(count, sel) => {
            let cards = resolve_v2_selector(sel, rt, player);
            if let Some(&card_id) = cards.first() {
                rt.detach_material(card_id, *count as u32);
            }
        }
        Action::Mill(expr, owner) => {
            let count = eval_v2_expr(expr, rt) as u32;
            let target_player = match owner {
                Some(DeckOwner::Opponents) => 1 - player,
                _ => player,
            };
            rt.mill(target_player, count);
        }
        Action::Excavate(expr, owner) => {
            let count = eval_v2_expr(expr, rt) as u32;
            let target_player = match owner {
                DeckOwner::Yours => player,
                DeckOwner::Opponents => 1 - player,
            };
            let _ = rt.excavate(target_player, count);
        }
        Action::ShuffleDeck(owner) => {
            let target_player = match owner {
                Some(DeckOwner::Opponents) => 1 - player,
                _ => player,
            };
            rt.shuffle_deck(target_player);
        }
        Action::Reveal(sel) => {
            let cards = resolve_v2_selector(sel, rt, player);
            rt.reveal(&cards);
        }
        Action::LookAt(sel, _zone) => {
            let cards = resolve_v2_selector(sel, rt, player);
            rt.look_at(player, &cards);
        }
        Action::Announce(what, _binding) => {
            let kind: u8 = match what {
                AnnounceWhat::Type      => 3,
                AnnounceWhat::Attribute => 1,
                AnnounceWhat::Race      => 2,
                AnnounceWhat::Level     => 4,
                AnnounceWhat::Card      => 0,
            };
            rt.announce(player, kind, 0);
        }
        Action::CoinFlip { heads, tails } => {
            let result = rt.coin_flip(player);
            let actions = if result { heads } else { tails };
            for a in actions {
                execute_v2_action(a, rt, player);
            }
        }
        Action::DiceRoll(branches) => {
            let result = rt.dice_roll(player) as usize;
            if !branches.is_empty() {
                let idx = if result > 0 && result <= branches.len() { result - 1 } else { 0 };
                execute_v2_action(&branches[idx], rt, player);
            }
        }
        Action::NormalSummon(sel) => {
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.normal_summon(card_id, player);
            }
        }
        Action::Set(sel, _zone) => {
            let cards = resolve_v2_selector(sel, rt, player);
            for card_id in cards {
                rt.set_card(card_id, player);
            }
        }
        Action::ForEach { selector, zone: _, body } => {
            // Resolve the set of cards and iterate; execute body for each
            let cards = resolve_v2_selector(selector, rt, player);
            for card_id in cards {
                // Set the card as the current target so body actions can reference it
                rt.set_targets(&[card_id]);
                for a in body {
                    execute_v2_action(a, rt, player);
                }
            }
        }
        Action::Choose(block) => {
            let labels: Vec<String> = block.options.iter()
                .map(|o| o.label.clone())
                .collect();
            let chosen = rt.select_option(player, &labels);
            if let Some(option) = block.options.get(chosen) {
                for a in &option.resolve {
                    execute_v2_action(a, rt, player);
                }
            }
        }
        Action::Delayed { until, body } => {
            let phase_code = phase_name_to_code(until);
            let card_id = rt.effect_card_id();
            rt.register_delayed(phase_code, card_id);
            // In mock/test context also execute body immediately so tests can observe
            for a in body {
                execute_v2_action(a, rt, player);
            }
        }
        Action::InstallWatcher { name, .. } => {
            // Signal the engine to register a watcher; engine handles trigger/duration logic
            rt.raise_custom_event(&format!("install_watcher:{}", name), &[]);
        }
        Action::LinkTo(_, _) => {
            // LinkTo is engine-internal (set link arrows); no runtime method yet
        }
        Action::RitualSummon { target, materials, .. } => {
            let targets = resolve_v2_selector(target, rt, player);
            let mats = materials.as_ref().map(|m| resolve_v2_selector(m, rt, player)).unwrap_or_default();
            if let Some(&card_id) = targets.first() {
                rt.ritual_summon(card_id, player, &mats);
            }
        }
        Action::FusionSummon { target, materials } => {
            let targets = resolve_v2_selector(target, rt, player);
            let mats = materials.as_ref().map(|m| resolve_v2_selector(m, rt, player)).unwrap_or_default();
            if let Some(&card_id) = targets.first() {
                rt.fusion_summon(card_id, player, &mats);
            }
        }
        Action::SynchroSummon { target, materials } => {
            let targets = resolve_v2_selector(target, rt, player);
            let mats = materials.as_ref().map(|m| resolve_v2_selector(m, rt, player)).unwrap_or_default();
            if let Some(&card_id) = targets.first() {
                rt.synchro_summon(card_id, player, &mats);
            }
        }
        Action::XyzSummon { target, materials } => {
            let targets = resolve_v2_selector(target, rt, player);
            let mats = materials.as_ref().map(|m| resolve_v2_selector(m, rt, player)).unwrap_or_default();
            if let Some(&card_id) = targets.first() {
                rt.xyz_summon(card_id, player, &mats);
            }
        }
    }
}

// ── Wire Callbacks Into Compiled Effects ─────────────────────

fn build_effect_callbacks(effect: &Effect, card: &Card) -> (
    Option<Arc<dyn Fn(&dyn DuelScriptRuntime) -> bool + Send + Sync>>,
    Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>>,
    Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime, bool) -> bool + Send + Sync>>,
    Option<Arc<dyn Fn(&mut dyn DuelScriptRuntime) + Send + Sync>>,
) {
    let card_id = card.fields.id.unwrap_or(0);
    (
        gen_condition(effect),
        gen_cost(&effect.cost, card_id),
        gen_target(&effect.target),
        gen_operation(&effect.resolve),
    )
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::parser::parse_v2;

    fn compile(path: &str) -> CompiledCardV2 {
        let source = std::fs::read_to_string(path).unwrap();
        let file = parse_v2(&source).unwrap();
        compile_card_v2(&file.cards[0])
    }

    #[test]
    fn test_pot_of_greed_compile() {
        let c = compile("cards/goat/pot_of_greed.ds");
        assert_eq!(c.card_id, 55144522);
        assert_eq!(c.effects.len(), 1);
        let e = &c.effects[0];
        assert_eq!(e.effect_type, tm::EFFECT_TYPE_ACTIVATE);
        assert_eq!(e.category, tm::CATEGORY_DRAW);
        assert_eq!(e.code, tm::EVENT_FREE_CHAIN);
        assert_eq!(e.range, tm::LOCATION_SZONE);
    }

    #[test]
    fn test_raigeki_compile() {
        let c = compile("cards/goat/raigeki.ds");
        assert_eq!(c.effects.len(), 1);
        let e = &c.effects[0];
        assert_eq!(e.effect_type, tm::EFFECT_TYPE_ACTIVATE);
        assert_eq!(e.category, tm::CATEGORY_DESTROY);
        assert_eq!(e.code, tm::EVENT_FREE_CHAIN);
    }

    #[test]
    fn test_mirror_force_compile() {
        let c = compile("cards/goat/mirror_force.ds");
        assert_eq!(c.effects.len(), 1);
        let e = &c.effects[0];
        assert_eq!(e.effect_type, tm::EFFECT_TYPE_ACTIVATE);
        assert_eq!(e.category, tm::CATEGORY_DESTROY);
        assert_eq!(e.code, tm::EVENT_ATTACK_ANNOUNCE);
        assert_eq!(e.range, tm::LOCATION_SZONE);
    }

    #[test]
    fn test_solemn_judgment_compile() {
        let c = compile("cards/goat/solemn_judgment.ds");
        // Solemn expands to 3 effects (summon + flip_summon + spsummon)
        assert_eq!(c.effects.len(), 3);
        assert_eq!(c.effects[0].code, tm::EVENT_SUMMON);
        assert_eq!(c.effects[1].code, tm::EVENT_FLIP_SUMMON);
        assert_eq!(c.effects[2].code, tm::EVENT_SPSUMMON);
        assert_eq!(c.effects[0].category, tm::CATEGORY_NEGATE);
    }

    #[test]
    fn test_sangan_compile() {
        let c = compile("cards/goat/sangan.ds");
        assert_eq!(c.effects.len(), 1);
        let e = &c.effects[0];
        // Mandatory trigger on monster
        assert_ne!(e.effect_type & tm::EFFECT_TYPE_TRIGGER_F, 0);
        assert_ne!(e.effect_type & tm::EFFECT_TYPE_SINGLE, 0); // self-trigger
        assert_eq!(e.code, tm::EVENT_TO_GRAVE);
        assert_eq!(e.count_limit, Some((1, 26202165)));
        assert_eq!(e.range, 0); // self-trigger → range 0
    }

    #[test]
    fn test_lava_golem_compile() {
        let c = compile("cards/goat/lava_golem.ds");
        // Special summon procedure + cannot normal summon + effect
        assert!(c.effects.len() >= 2);
        // Find the triggered effect
        let burn = c.effects.iter().find(|e| e.label == "Burn").unwrap();
        assert_ne!(burn.effect_type & tm::EFFECT_TYPE_TRIGGER_F, 0); // mandatory
        assert_eq!(burn.code, tm::EVENT_PHASE | tm::PHASE_STANDBY);
        assert_eq!(burn.category, tm::CATEGORY_DAMAGE);
    }

    #[test]
    fn test_book_of_moon_compile() {
        let c = compile("cards/goat/book_of_moon.ds");
        assert_eq!(c.effects.len(), 1);
        let e = &c.effects[0];
        assert_eq!(e.effect_type, tm::EFFECT_TYPE_ACTIVATE);
        assert_ne!(e.property & tm::EFFECT_FLAG_CARD_TARGET, 0); // targets
        assert_eq!(e.range, tm::LOCATION_SZONE);
    }

    #[test]
    fn test_graceful_charity_compile() {
        let c = compile("cards/goat/graceful_charity.ds");
        assert_eq!(c.effects.len(), 1);
        let e = &c.effects[0];
        assert_eq!(e.category, tm::CATEGORY_DRAW | tm::CATEGORY_HANDES);
    }

    #[test]
    fn test_jinzo_compile() {
        let c = compile("cards/goat/jinzo.ds");
        // summon (tributes: 1) + passive
        let passive = c.effects.iter().find(|e| e.label == "Trap Lockdown").unwrap();
        assert_ne!(passive.effect_type & tm::EFFECT_TYPE_FIELD, 0);
        assert_ne!(passive.effect_type & tm::EFFECT_TYPE_CONTINUOUS, 0);
        assert_eq!(passive.category, tm::CATEGORY_DISABLE);
        assert_eq!(passive.range, tm::LOCATION_MZONE);
    }

    #[test]
    fn test_dark_paladin_compile() {
        let c = compile("cards/goat/dark_paladin.ds");
        // summon proc + passive + negate effect
        let negate = c.effects.iter().find(|e| e.label == "Negate Spell").unwrap();
        assert_eq!(negate.effect_type, tm::EFFECT_TYPE_QUICK_O); // speed 2 monster = Quick Effect
        assert_eq!(negate.code, tm::EVENT_CHAINING);
        assert_eq!(negate.category, tm::CATEGORY_NEGATE);

        let boost = c.effects.iter().find(|e| e.label == "Dragon Power").unwrap();
        assert_ne!(boost.effect_type & tm::EFFECT_TYPE_CONTINUOUS, 0);
        assert_eq!(boost.code, 100); // EFFECT_UPDATE_ATTACK
    }

    #[test]
    fn test_spirit_reaper_compile() {
        let c = compile("cards/goat/spirit_reaper.ds");
        // passive (indestructible) + 2 triggered effects
        let passive = c.effects.iter().find(|e| e.label == "Indestructible").unwrap();
        assert_ne!(passive.effect_type & tm::EFFECT_TYPE_SINGLE, 0);
        assert_eq!(passive.code, 30); // EFFECT_INDESTRUCTABLE_BATTLE

        let selfdestruct = c.effects.iter().find(|e| e.label == "Self-Destruct").unwrap();
        assert_ne!(selfdestruct.effect_type & tm::EFFECT_TYPE_TRIGGER_F, 0); // mandatory

        let rip = c.effects.iter().find(|e| e.label == "Hand Rip").unwrap();
        assert_eq!(rip.category, tm::CATEGORY_HANDES);
    }

    // ── Callback Tests (execute against MockRuntime) ───────────

    #[test]
    fn test_pot_of_greed_executes() {
        use super::super::mock_runtime::MockRuntime;
        let c = compile("cards/goat/pot_of_greed.ds");
        let effect = &c.effects[0];
        let mut rt = MockRuntime::new();
        rt.effect_player = 0;
        rt.effect_card_id = 55144522;
        (effect.operation.as_ref().unwrap())(&mut rt);
        assert!(rt.was_called_with("draw", "count=2"));
    }

    #[test]
    fn test_solemn_cost_pays_half_lp() {
        use super::super::mock_runtime::MockRuntime;
        let c = compile("cards/goat/solemn_judgment.ds");
        let effect = &c.effects[0];
        let mut rt = MockRuntime::new();
        rt.effect_player = 0;
        rt.effect_card_id = 41420027;
        let cost_fn = effect.cost.as_ref().unwrap();
        assert!(cost_fn(&mut rt, true));
        cost_fn(&mut rt, false);
        assert_eq!(rt.get_lp(0), 4000);
    }

    #[test]
    fn test_negate_and_destroy_executes() {
        use super::super::mock_runtime::MockRuntime;
        let c = compile("cards/goat/solemn_judgment.ds");
        let effect = &c.effects[0];
        let mut rt = MockRuntime::new();
        rt.effect_player = 0;
        rt.effect_card_id = 41420027;
        (effect.operation.as_ref().unwrap())(&mut rt);
        assert!(rt.was_called_with("negate_activation", ""));
    }

    #[test]
    fn test_lava_golem_damage_executes() {
        use super::super::mock_runtime::MockRuntime;
        let c = compile("cards/goat/lava_golem.ds");
        let burn = c.effects.iter().find(|e| e.label == "Burn").unwrap();
        let mut rt = MockRuntime::new();
        rt.effect_player = 0;
        rt.effect_card_id = 102380;
        (burn.operation.as_ref().unwrap())(&mut rt);
        assert!(rt.was_called_with("damage", "1000"));
    }

    #[test]
    fn test_graceful_charity_executes() {
        use super::super::mock_runtime::MockRuntime;
        let c = compile("cards/goat/graceful_charity.ds");
        let effect = &c.effects[0];
        let mut rt = MockRuntime::new();
        rt.effect_player = 0;
        rt.effect_card_id = 79571449;
        (effect.operation.as_ref().unwrap())(&mut rt);
        assert!(rt.was_called_with("draw", "count=3"));
    }

    #[test]
    fn test_thestalos_compile() {
        let c = compile("cards/goat/thestalos.ds");
        let eff = c.effects.iter().find(|e| e.label == "Discard and Burn").unwrap();
        assert_ne!(eff.effect_type & tm::EFFECT_TYPE_TRIGGER_F, 0);
        assert_eq!(eff.code, tm::EVENT_SUMMON_SUCCESS);
        assert_eq!(eff.category, tm::CATEGORY_HANDES);
    }

    #[test]
    fn test_change_level_executes() {
        use super::super::mock_runtime::{MockRuntime, CardSnapshot};
        const CARD_ID: u32 = 99001;
        let src = r#"
card "Level Changer" {
    id: 99001
    type: Effect Monster
    attribute: DARK
    race: Spellcaster
    level: 6
    atk: 2000
    def: 1500

    effect "Level Down" {
        speed: 1
        resolve {
            change_level self to 4
        }
    }
}
"#;
        let file = parse_v2(src).unwrap();
        let compiled = compile_card_v2(&file.cards[0]);
        let effect = compiled.effects.iter().find(|e| e.label == "Level Down").unwrap();

        let mut rt = MockRuntime::new();
        rt.effect_player = 0;
        rt.effect_card_id = CARD_ID;
        // register the card so SelfCard selector can find its id
        rt.state.add_card(CardSnapshot::monster(CARD_ID, "Level Changer", 2000, 1500, 6));
        rt.state.players[0].field_monsters.push(CARD_ID);

        (effect.operation.as_ref().unwrap())(&mut rt);
        assert!(rt.was_called_with("change_level", "level=4"),
            "expected change_level call; calls: {}", rt.dump_calls());
    }

    #[test]
    fn test_complex_card_all_effects_execute() {
        use super::super::mock_runtime::{MockRuntime, CardSnapshot};

        const CARD_ID: u32 = 99999999;
        const OPP_MONSTER: u32 = 12345;

        let c = compile("cards/goat/test_complex.ds");
        assert_eq!(c.card_id, 99999999);
        assert_eq!(c.effects.len(), 3, "expected 3 effects, got {}: {:?}", c.effects.len(), c.effects);

        // --- Effect 0: Counter Play ---
        // Cost: remove_counter; resolve: destroy opponent monster then draw
        {
            let eff = &c.effects[0];
            assert_eq!(eff.label, "Counter Play");

            // Execute cost callback
            if let Some(ref cost_fn) = eff.cost {
                let mut rt = MockRuntime::new();
                rt.effect_player = 0;
                rt.effect_card_id = CARD_ID;
                // Put self on field so remove_counter can find a target
                rt.state.add_card(CardSnapshot::monster(CARD_ID, "Test Complex Card", 2500, 2100, 7));
                rt.state.players[0].field_monsters.push(CARD_ID);
                cost_fn(&mut rt, false);
                assert!(rt.was_called_with("remove_counter", "Spell Counter"),
                    "Counter Play cost: expected remove_counter; calls: {}", rt.dump_calls());
            } else {
                panic!("Counter Play effect missing cost callback");
            }

            // Execute operation callback
            if let Some(ref op) = eff.operation {
                let mut rt = MockRuntime::new();
                rt.effect_player = 0;
                rt.effect_card_id = CARD_ID;
                // Put an opponent monster on field so destroy selector finds something
                rt.state.add_card(CardSnapshot::monster(OPP_MONSTER, "Opp Monster", 1800, 1200, 4));
                rt.state.players[1].field_monsters.push(OPP_MONSTER);
                // Give player 0 a deck card so draw works
                rt.state.players[0].deck.push(42u32);
                op(&mut rt);
                let log = rt.dump_calls();
                assert!(!log.is_empty(), "Counter Play operation produced no calls");
            } else {
                panic!("Counter Play effect missing operation callback");
            }
        }

        // --- Effect 1: Mill Effect ---
        // Condition: lp >= 1000; resolve: mill 2
        {
            let eff = &c.effects[1];
            assert_eq!(eff.label, "Mill Effect");

            // Evaluate condition (LP is 8000 by default, >= 1000 should pass)
            if let Some(ref cond_fn) = eff.condition {
                let mut rt = MockRuntime::new();
                rt.effect_player = 0;
                rt.effect_card_id = CARD_ID;
                let result = cond_fn(&rt);
                assert!(result, "Mill Effect condition should be true at 8000 LP");
            } else {
                panic!("Mill Effect missing condition callback");
            }

            // Execute operation
            if let Some(ref op) = eff.operation {
                let mut rt = MockRuntime::new();
                rt.effect_player = 0;
                rt.effect_card_id = CARD_ID;
                // Give player a deck to mill
                rt.state.players[0].deck = vec![1, 2, 3, 4];
                op(&mut rt);
                assert!(rt.was_called_with("mill", "count=2"),
                    "Mill Effect: expected mill count=2; calls: {}", rt.dump_calls());
            } else {
                panic!("Mill Effect missing operation callback");
            }
        }

        // --- Effect 2: Level Change ---
        // resolve: change_level self to 4, modify_atk self + 500
        {
            let eff = &c.effects[2];
            assert_eq!(eff.label, "Level Change");

            if let Some(ref op) = eff.operation {
                let mut rt = MockRuntime::new();
                rt.effect_player = 0;
                rt.effect_card_id = CARD_ID;
                rt.state.add_card(CardSnapshot::monster(CARD_ID, "Test Complex Card", 2500, 2100, 7));
                rt.state.players[0].field_monsters.push(CARD_ID);
                op(&mut rt);
                let log = rt.dump_calls();
                assert!(rt.was_called_with("change_level", "level=4"),
                    "Level Change: expected change_level level=4; calls: {}", log);
                assert!(rt.was_called_with("modify_atk", "delta=500"),
                    "Level Change: expected modify_atk delta=500; calls: {}", log);
            } else {
                panic!("Level Change effect missing operation callback");
            }
        }
    }

    #[test]
    fn test_ritual_summon_action_parses_and_executes() {
        use super::super::mock_runtime::{MockRuntime, CardSnapshot};
        let source = r#"
            card "Test Ritual" {
                id: 12345
                type: Ritual Spell

                effect "Summon" {
                    speed: 1
                    resolve {
                        ritual_summon (1, monster) using (1+, monster, you control) where total_level >= 8
                    }
                }
            }
        "#;
        let file = parse_v2(source).unwrap();
        let compiled = compile_card_v2(&file.cards[0]);
        assert_eq!(compiled.effects.len(), 1);
        let mut rt = MockRuntime::new();
        rt.effect_card_id = 12345;
        rt.effect_player = 0;
        // Provide a monster on the field so the selector can resolve something
        rt.state.add_card(CardSnapshot::monster(12345, "Test Ritual", 0, 0, 8));
        rt.state.players[0].field_monsters.push(12345);
        if let Some(ref op) = compiled.effects[0].operation {
            op(&mut rt);
        }
        let log = rt.dump_calls();
        assert!(log.contains("ritual_summon"), "Expected ritual_summon, got: {}", log);
    }
}
