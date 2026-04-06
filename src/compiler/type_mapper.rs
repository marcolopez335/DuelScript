// ============================================================
// DuelScript Type Mapper — compiler/type_mapper.rs
// Maps DSL-level types to engine u32 bitfield constants.
//
// These constants match the yugaioh engine (and EDOPro/YGOPro)
// constant definitions exactly.
// ============================================================

use crate::ast::*;
use super::CountLimit;

// ── Engine Constants ──────────────────────────────────────────
// ALL values from EDOPro/YGOPro constant.lua — the canonical source.
// These MUST match exactly or Lua comparison will fail.

// Effect types
pub const EFFECT_TYPE_SINGLE:     u32 = 0x1;
pub const EFFECT_TYPE_FIELD:      u32 = 0x2;
pub const EFFECT_TYPE_EQUIP:      u32 = 0x4;
pub const EFFECT_TYPE_ACTIVATE:   u32 = 0x10;
pub const EFFECT_TYPE_FLIP:       u32 = 0x20;
pub const EFFECT_TYPE_IGNITION:   u32 = 0x40;
pub const EFFECT_TYPE_TRIGGER_O:  u32 = 0x80;
pub const EFFECT_TYPE_QUICK_O:    u32 = 0x100;
pub const EFFECT_TYPE_TRIGGER_F:  u32 = 0x200;
pub const EFFECT_TYPE_QUICK_F:    u32 = 0x400;
pub const EFFECT_TYPE_CONTINUOUS: u32 = 0x800;
pub const EFFECT_TYPE_XMATERIAL:  u32 = 0x1000;

// Categories
pub const CATEGORY_DESTROY:        u32 = 0x1;
pub const CATEGORY_RELEASE:        u32 = 0x2;
pub const CATEGORY_REMOVE:         u32 = 0x4;
pub const CATEGORY_TOHAND:         u32 = 0x8;
pub const CATEGORY_TODECK:         u32 = 0x10;
pub const CATEGORY_TOGRAVE:        u32 = 0x20;
pub const CATEGORY_DECKDES:        u32 = 0x40;
pub const CATEGORY_HANDES:         u32 = 0x80;
pub const CATEGORY_SUMMON:         u32 = 0x100;
pub const CATEGORY_SPECIAL_SUMMON: u32 = 0x200;
pub const CATEGORY_TOKEN:          u32 = 0x400;
pub const CATEGORY_POSITION:       u32 = 0x1000;
pub const CATEGORY_CONTROL:        u32 = 0x2000;
pub const CATEGORY_DISABLE:        u32 = 0x4000;
pub const CATEGORY_DISABLE_SUMMON: u32 = 0x8000;
pub const CATEGORY_DRAW:           u32 = 0x10000;
pub const CATEGORY_SEARCH:         u32 = 0x20000;
pub const CATEGORY_EQUIP:          u32 = 0x40000;
pub const CATEGORY_DAMAGE:         u32 = 0x80000;
pub const CATEGORY_RECOVER:        u32 = 0x100000;
pub const CATEGORY_ATKCHANGE:      u32 = 0x200000;
pub const CATEGORY_DEFCHANGE:      u32 = 0x400000;
pub const CATEGORY_COUNTER:        u32 = 0x800000;
pub const CATEGORY_NEGATE:         u32 = 0x10000000;
pub const CATEGORY_FUSION_SUMMON:  u32 = 0x40000000;

// Events
pub const EVENT_FLIP:              u32 = 1001;
pub const EVENT_FREE_CHAIN:        u32 = 1002;
pub const EVENT_DESTROY:           u32 = 1010;
pub const EVENT_REMOVE:            u32 = 1011;
pub const EVENT_TO_HAND:           u32 = 1012;
pub const EVENT_TO_GRAVE:          u32 = 1014;
pub const EVENT_RELEASE:           u32 = 1017;
pub const EVENT_CHAINING:          u32 = 1027;
pub const EVENT_DESTROYED:         u32 = 1029;
pub const EVENT_SUMMON_SUCCESS:    u32 = 1100;
pub const EVENT_FLIP_SUMMON_SUCCESS: u32 = 1101;
pub const EVENT_SPSUMMON_SUCCESS:  u32 = 1102;
pub const EVENT_SUMMON:            u32 = 1103;
pub const EVENT_FLIP_SUMMON:       u32 = 1104;
pub const EVENT_SPSUMMON:          u32 = 1105;
pub const EVENT_ATTACK_ANNOUNCE:   u32 = 1130;
pub const EVENT_BE_BATTLE_TARGET:  u32 = 1131;
pub const EVENT_PREDRAW:           u32 = 1113;

// Phase events: EVENT_PHASE + PHASE_*
pub const EVENT_PHASE:             u32 = 0x1000;
pub const PHASE_DRAW:              u32 = 0x1;
pub const PHASE_STANDBY:           u32 = 0x2;
pub const PHASE_MAIN1:             u32 = 0x4;
pub const PHASE_BATTLE:            u32 = 0x80;
pub const PHASE_MAIN2:             u32 = 0x100;
pub const PHASE_END:               u32 = 0x200;

// Locations
pub const LOCATION_DECK:    u32 = 0x1;
pub const LOCATION_HAND:    u32 = 0x2;
pub const LOCATION_MZONE:   u32 = 0x4;
pub const LOCATION_SZONE:   u32 = 0x8;
pub const LOCATION_GRAVE:   u32 = 0x10;
pub const LOCATION_REMOVED: u32 = 0x20;
pub const LOCATION_EXTRA:   u32 = 0x40;
pub const LOCATION_FZONE:   u32 = 0x100;
pub const LOCATION_PZONE:   u32 = 0x200;
pub const LOCATION_ONFIELD: u32 = LOCATION_MZONE | LOCATION_SZONE;

// Property flags
pub const EFFECT_FLAG_CARD_TARGET:   u32 = 0x10;
pub const EFFECT_FLAG_PLAYER_TARGET: u32 = 0x800;
pub const EFFECT_FLAG_DAMAGE_STEP:   u32 = 0x4000;
pub const EFFECT_FLAG_DELAY:         u32 = 0x10000;
pub const EFFECT_FLAG_SINGLE_RANGE:  u32 = 0x20000;

// ── Mapping Functions ─────────────────────────────────────────

/// Map DSL effect body + card type → engine effect_type bitfield
pub fn effect_type_flags(body: &EffectBody, card: &Card) -> u32 {
    // Spell/Trap cards: their effects use EFFECT_TYPE_ACTIVATE
    // This covers: Normal Spells, Counter Traps (even with triggers like Solemn),
    // Continuous Spells/Traps activation, Quick-Play Spells, etc.
    // The only exception is Quick-Play monster effects on Spell/Trap cards,
    // but those don't exist in standard YGO rules.
    if card.is_spell() || card.is_trap() {
        return EFFECT_TYPE_ACTIVATE;
    }

    // Quick effect (Spell Speed 2) — monster quick effects
    if body.speed == SpellSpeed::SpellSpeed2 {
        if body.trigger.is_some() {
            return if body.optional { EFFECT_TYPE_QUICK_O } else { EFFECT_TYPE_QUICK_F };
        }
        // Monster quick effect without explicit trigger
        return EFFECT_TYPE_QUICK_O;
    }

    // Triggered effect
    if body.trigger.is_some() {
        return if body.optional { EFFECT_TYPE_TRIGGER_O } else { EFFECT_TYPE_TRIGGER_F };
    }

    // Default: Ignition (manual activation during main phase)
    if card.is_monster() {
        return EFFECT_TYPE_IGNITION;
    }

    EFFECT_TYPE_ACTIVATE
}

/// Scan on_resolve actions to determine CATEGORY_* flags
pub fn categories_from_actions(actions: &[GameAction]) -> u32 {
    let mut cats = 0u32;
    for action in actions {
        cats |= category_for_action(action);
    }
    cats
}

fn category_for_action(action: &GameAction) -> u32 {
    match action {
        GameAction::Draw { .. }            => CATEGORY_DRAW,
        GameAction::Destroy { .. }         => CATEGORY_DESTROY,
        GameAction::SpecialSummon { .. }   => CATEGORY_SPECIAL_SUMMON,
        GameAction::Search { .. }          => CATEGORY_SEARCH,
        GameAction::AddToHand { .. }       => CATEGORY_TOHAND,
        GameAction::SendToZone { .. }      => CATEGORY_TOGRAVE,
        GameAction::Banish { .. }          => CATEGORY_REMOVE,
        GameAction::Return { .. }          => CATEGORY_TOHAND,
        GameAction::DealDamage { .. }      => CATEGORY_DAMAGE,
        GameAction::GainLp { .. }          => CATEGORY_RECOVER,
        GameAction::Negate { what, and_destroy } => {
            // "negate activation" = CATEGORY_NEGATE (Solemn Judgment)
            // "negate effect" = CATEGORY_DISABLE (Ash Blossom)
            // "negate summon" = CATEGORY_DISABLE_SUMMON
            // "negate attack" = no category (engine handles attack negation)
            let base = match what {
                Some(NegateTarget::Activation) => CATEGORY_NEGATE,
                Some(NegateTarget::Summon)     => CATEGORY_DISABLE_SUMMON,
                Some(NegateTarget::Attack)     => 0,
                _                               => CATEGORY_DISABLE,
            };
            if *and_destroy { base | CATEGORY_DESTROY } else { base }
        }
        GameAction::TakeControl { .. }     => CATEGORY_CONTROL,
        GameAction::ModifyAtk { .. }       => CATEGORY_ATKCHANGE,
        GameAction::ModifyDef { .. }       => CATEGORY_DEFCHANGE,
        GameAction::CreateToken { .. }     => CATEGORY_TOKEN,
        GameAction::Equip { .. }           => CATEGORY_EQUIP,
        GameAction::PlaceCounter { .. }    => CATEGORY_COUNTER,
        GameAction::RemoveCounter { .. }   => CATEGORY_COUNTER,
        GameAction::FusionSummon { .. }    => CATEGORY_FUSION_SUMMON | CATEGORY_SPECIAL_SUMMON,
        GameAction::SynchroSummon { .. }   => CATEGORY_SPECIAL_SUMMON,
        GameAction::XyzSummon { .. }       => CATEGORY_SPECIAL_SUMMON,
        GameAction::RitualSummon { .. }    => CATEGORY_SPECIAL_SUMMON,
        GameAction::PendulumSummon { .. }  => CATEGORY_SPECIAL_SUMMON,
        GameAction::Mill { .. }            => CATEGORY_TOGRAVE,
        GameAction::Discard { .. }         => CATEGORY_TOGRAVE,
        GameAction::Tribute { .. }         => CATEGORY_RELEASE,
        GameAction::ChangeBattlePosition { .. } => CATEGORY_POSITION,
        GameAction::Shuffle { .. }         => CATEGORY_TODECK,
        GameAction::If { then_actions, else_actions, .. } => {
            categories_from_actions(then_actions) | categories_from_actions(else_actions)
        }
        GameAction::Choose { options } => {
            options.iter().fold(0u32, |acc, opt| acc | categories_from_actions(&opt.actions))
        }
        GameAction::ForEach { actions, .. } => categories_from_actions(actions),
        _ => 0,
    }
}

/// Map DSL trigger expression → engine EVENT_* code
pub fn trigger_to_event_code(trigger: &Option<TriggerExpr>) -> u32 {
    match trigger {
        None => EVENT_FREE_CHAIN,
        Some(t) => match t {
            TriggerExpr::OpponentActivates(_)                 => EVENT_CHAINING,
            TriggerExpr::WhenSummoned(None)                   => EVENT_SUMMON_SUCCESS,
            TriggerExpr::WhenSummoned(Some(SummonMethod::ByNormalSummon)) => EVENT_SUMMON_SUCCESS,
            TriggerExpr::WhenSummoned(Some(SummonMethod::BySpecialSummon)) => EVENT_SPSUMMON_SUCCESS,
            TriggerExpr::WhenSummoned(Some(SummonMethod::ByFlipSummon)) => EVENT_FLIP_SUMMON,
            TriggerExpr::WhenSummoned(Some(_))                => EVENT_SPSUMMON_SUCCESS,
            TriggerExpr::WhenTributeSummoned(_)               => EVENT_SUMMON_SUCCESS,
            TriggerExpr::WhenTributed(_)                      => EVENT_SUMMON_SUCCESS,
            TriggerExpr::WhenDestroyed(_)                     => EVENT_DESTROYED,
            TriggerExpr::WhenSentTo { .. }                    => EVENT_TO_GRAVE,
            TriggerExpr::WhenFlipped                          => EVENT_FLIP,
            TriggerExpr::WhenAttacked                         => EVENT_BE_BATTLE_TARGET,
            TriggerExpr::OnNthSummon(_)                       => EVENT_SPSUMMON_SUCCESS,
            TriggerExpr::DuringStandbyPhase(_)                => EVENT_PHASE + PHASE_STANDBY,
            TriggerExpr::DuringEndPhase                       => EVENT_PHASE + PHASE_END,
            TriggerExpr::DuringPhase(phase) => match phase {
                Phase::DrawPhase        => EVENT_PHASE + PHASE_DRAW,
                Phase::StandbyPhase     => EVENT_PHASE + PHASE_STANDBY,
                Phase::MainPhase1       => EVENT_PHASE + PHASE_MAIN1,
                Phase::BattlePhase      => EVENT_PHASE + PHASE_BATTLE,
                Phase::MainPhase2       => EVENT_PHASE + PHASE_MAIN2,
                Phase::EndPhase         => EVENT_PHASE + PHASE_END,
                _                       => EVENT_FREE_CHAIN,
            },
            TriggerExpr::WhenBattleDestroyed          => 1029, // EVENT_BATTLE_DESTROYED
            TriggerExpr::WhenDestroysByBattle         => 1030, // EVENT_BATTLE_DESTROYING (approximate)
            TriggerExpr::WhenLeavesField              => 1015, // EVENT_LEAVE_FIELD
            TriggerExpr::WhenUsedAsMaterial(_)        => 1108, // EVENT_BE_MATERIAL
            TriggerExpr::WhenBattleDamage(_)          => 1111, // EVENT_BATTLE_DAMAGE (approximate)
            TriggerExpr::WhenBanished(_)              => EVENT_REMOVE,
            TriggerExpr::WhenAction(action) => match action {
                TriggerAction::AttackDeclared => EVENT_ATTACK_ANNOUNCE,
                _ => EVENT_FREE_CHAIN,
            },
        },
    }
}

/// Determine EFFECT_FLAG_* property bits from the effect body
pub fn property_flags(body: &EffectBody) -> u32 {
    let mut flags = 0u32;

    // CARD_TARGET is set when the effect targets specific cards during activation
    // (on_activate phase). Effects that affect groups on resolution (like Dark Hole)
    // do NOT target — only effects with explicit target selection in on_activate do.
    if !body.on_activate.is_empty() && body.on_activate.iter().any(action_targets_card) {
        flags |= EFFECT_FLAG_CARD_TARGET;
    }

    // Check for timing qualifier that implies DELAY flag
    if body.timing == TimingQualifier::If {
        flags |= EFFECT_FLAG_DELAY;
    }

    flags
}

fn action_targets_card(action: &GameAction) -> bool {
    matches!(action,
        GameAction::Destroy { .. }
      | GameAction::Banish { .. }
      | GameAction::Return { .. }
      | GameAction::TakeControl { .. }
      | GameAction::Equip { .. }
      | GameAction::Search { .. }
      | GameAction::SpecialSummon { .. }
    )
}

/// Determine where this effect can be activated from
pub fn activation_range(body: &EffectBody, card: &Card) -> u32 {
    // If the effect has a condition that specifies a zone, use that
    if let Some(ConditionExpr::Simple(SimpleCondition::InZone(zone))) = &body.condition {
        return zone_to_location(zone);
    }

    // Check trigger for hand-trap patterns — monsters that activate from hand
    // Extra Deck monsters (Synchro, Xyz, Link, Fusion) activate from MZONE
    if let Some(TriggerExpr::OpponentActivates(_)) = &body.trigger {
        if body.speed == SpellSpeed::SpellSpeed2 && card.is_monster() && !card.is_extra_deck() {
            return LOCATION_HAND;
        }
    }

    // Default by card type
    // Activated spells/traps don't set explicit range in the engine —
    // the Lua scripts leave it at 0 and the engine handles activation from hand/field
    if card.is_spell() || card.is_trap() {
        0 // engine handles spell/trap activation location
    } else {
        LOCATION_MZONE
    }
}

/// Map DSL Zone → engine LOCATION_* constant
pub fn zone_to_location(zone: &Zone) -> u32 {
    match zone {
        Zone::Hand            => LOCATION_HAND,
        Zone::Deck            => LOCATION_DECK,
        Zone::Graveyard       => LOCATION_GRAVE,
        Zone::Banished        => LOCATION_REMOVED,
        Zone::ExtraDeck       => LOCATION_EXTRA,
        Zone::ExtraDeckFaceUp => LOCATION_EXTRA,
        Zone::MonsterZone     => LOCATION_MZONE,
        Zone::SpellTrapZone   => LOCATION_SZONE,
        Zone::Field           => LOCATION_ONFIELD,
        Zone::FieldZone       => LOCATION_FZONE,
        Zone::PendulumZone    => LOCATION_PZONE,
        Zone::ExtraMonsterZone => LOCATION_MZONE,
        Zone::TopOfDeck       => LOCATION_DECK,
        Zone::BottomOfDeck    => LOCATION_DECK,
    }
}

/// Map DSL Frequency → engine CountLimit
pub fn frequency_to_count_limit(freq: &Frequency, card_id: u32) -> Option<CountLimit> {
    match freq {
        Frequency::Unlimited => None,
        Frequency::OncePerTurn(opt) => Some(CountLimit {
            count: 1,
            code: match opt {
                OptKind::Soft => 0,
                OptKind::Hard => card_id,
            },
        }),
        Frequency::TwicePerTurn => Some(CountLimit {
            count: 2,
            code: card_id,
        }),
        Frequency::OncePerDuel => Some(CountLimit {
            count: 1,
            code: card_id | 0x10000000, // Engine convention for per-duel
        }),
        Frequency::EachTurn => Some(CountLimit {
            count: 1,
            code: 0, // Soft — resets each turn
        }),
    }
}

/// Map a ChainCategory to engine CATEGORY_* constant
pub fn chain_category_to_constant(cat: &ChainCategory) -> u32 {
    match cat {
        ChainCategory::Search       => CATEGORY_SEARCH,
        ChainCategory::SpecialSummon => CATEGORY_SPECIAL_SUMMON,
        ChainCategory::SendToGy     => CATEGORY_TOGRAVE,
        ChainCategory::AddToHand    => CATEGORY_TOHAND,
        ChainCategory::Draw         => CATEGORY_DRAW,
        ChainCategory::Banish       => CATEGORY_REMOVE,
        ChainCategory::Mill         => CATEGORY_TOGRAVE,
        ChainCategory::Destroy      => CATEGORY_DESTROY,
        ChainCategory::Negate       => CATEGORY_NEGATE,
    }
}
