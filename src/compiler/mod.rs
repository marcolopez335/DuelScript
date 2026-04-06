// ============================================================
// DuelScript Compiler — compiler/mod.rs
// Compiles parsed AST into engine-consumable effect definitions.
//
// This module is engine-agnostic at the interface level but
// provides concrete compilation for the yugaioh engine via
// the CompiledCard / CompiledEffect types.
// ============================================================

pub mod type_mapper;
pub mod expr_eval;
pub mod callback_gen;

use crate::ast::*;

/// A fully compiled card ready for engine consumption.
pub struct CompiledCard {
    pub card_id: u32,
    pub name: String,
    pub effects: Vec<CompiledEffect>,
}

/// A single compiled effect with all metadata resolved to engine constants.
pub struct CompiledEffect {
    /// Engine effect type flags (e.g., EFFECT_TYPE_ACTIVATE | EFFECT_TYPE_IGNITION)
    pub effect_type: u32,
    /// Engine category flags (e.g., CATEGORY_DESTROY | CATEGORY_DRAW)
    pub category: u32,
    /// Engine event code (e.g., EVENT_FREE_CHAIN, EVENT_CHAINING)
    pub code: u32,
    /// Engine property flags (e.g., EFFECT_FLAG_CARD_TARGET)
    pub property: u32,
    /// Activation range (e.g., LOCATION_HAND | LOCATION_MZONE)
    pub range: u32,
    /// Once-per-turn / once-per-duel limits
    pub count_limit: Option<CountLimit>,
    /// Generated callbacks for condition/cost/target/operation
    pub callbacks: callback_gen::GeneratedCallbacks,
    /// The original AST effect body (retained for debugging/inspection)
    pub source: EffectBody,
}

/// Count limit for effect activation frequency.
#[derive(Debug, Clone)]
pub struct CountLimit {
    /// How many times per period (1 for OPT, 2 for twice-per-turn)
    pub count: u32,
    /// 0 = soft OPT (can re-activate if negated), card_id = hard OPT
    pub code: u32,
}

/// Compile a parsed Card AST into a CompiledCard.
///
/// This resolves all DSL-level types into engine-level u32 bitfields
/// and prepares effect metadata. Callback generation (the closures
/// that actually execute effects) is handled separately.
pub fn compile_card(card: &Card) -> CompiledCard {
    let card_id = card.password.unwrap_or(0);
    let mut effects = Vec::new();

    // Summoning procedure effects — Extra Deck monsters need these
    if let Some(ref mats) = card.materials {
        effects.extend(compile_summoning_procedures(mats, card));
    }

    // EnableReviveLimit — Extra Deck monsters that can only be Special Summoned
    // by their proper method first
    if card.is_extra_deck() {
        effects.extend(compile_revive_limit(card));
    }

    // For spells/traps: ACTIVATE effects first, then continuous
    // For monsters: continuous first, then triggered/quick
    // This matches Lua registration order in initial_effect()
    if card.is_spell() || card.is_trap() {
        for effect in &card.effects {
            effects.extend(compile_effect_expanded(effect, card));
        }
        for ce in &card.continuous_effects {
            effects.push(compile_continuous_effect(ce, card));
        }
    } else {
        for ce in &card.continuous_effects {
            effects.push(compile_continuous_effect(ce, card));
        }
        for effect in &card.effects {
            effects.extend(compile_effect_expanded(effect, card));
        }
    }

    for re in &card.replacement_effects {
        effects.push(compile_replacement_effect(re, card));
    }

    for eq in &card.equip_effects {
        effects.push(compile_equip_effect(eq, card));
    }

    CompiledCard {
        card_id,
        name: card.name.clone(),
        effects,
    }
}

/// Compile materials block into summoning procedure effects.
/// Mirrors what Xyz/Synchro/Link/Fusion.AddProcedure do in Lua.
/// - Xyz.AddProcedure → 2 effects (check + proc)
/// - Synchro/Link/Fusion.AddProcedure → 1 effect (proc only)
fn compile_summoning_procedures(_mats: &MaterialsBlock, card: &Card) -> Vec<CompiledEffect> {
    let body = EffectBody::default();
    let is_xyz = card.card_types.contains(&CardType::XyzMonster);

    let mut effects = Vec::new();

    // Xyz has an extra check effect (code=946)
    if is_xyz {
        effects.push(CompiledEffect {
            effect_type: type_mapper::EFFECT_TYPE_SINGLE,
            category: 0,
            code: 946,
            property: 0,
            range: 0,
            count_limit: None,
            callbacks: callback_gen::GeneratedCallbacks {
                condition: None, cost: None, target: None, operation: None,
            },
            source: body.clone(),
        });
    }

    // All summon types get the SPSUMMON_PROC effect
    effects.push(CompiledEffect {
        effect_type: type_mapper::EFFECT_TYPE_FIELD,
        category: 0,
        code: 34, // EFFECT_SPSUMMON_PROC
        property: 0,
        range: type_mapper::LOCATION_EXTRA,
        count_limit: None,
        callbacks: callback_gen::GeneratedCallbacks {
            condition: None, cost: None, target: None, operation: None,
        },
        source: body,
    });

    effects
}

/// Compile EnableReviveLimit — Extra Deck monsters that need proper summon first.
/// The Lua engine stubs EnableReviveLimit as a no-op, so we don't generate
/// effects for it. If the engine implements it later, add effects here.
fn compile_revive_limit(_card: &Card) -> Vec<CompiledEffect> {
    // Engine currently stubs EnableReviveLimit — no effects needed
    vec![]
}

fn compile_continuous_effect(ce: &ContinuousEffect, card: &Card) -> CompiledEffect {
    // Continuous effects are EFFECT_TYPE_FIELD (affects other cards) or
    // EFFECT_TYPE_SINGLE (affects only this card)
    let effect_type = if ce.apply_to.is_some() {
        type_mapper::EFFECT_TYPE_FIELD
    } else {
        type_mapper::EFFECT_TYPE_SINGLE
    };

    // Determine the code based on modifiers — uses EDOPro EFFECT_* codes
    let code = if ce.modifiers.iter().any(|m| matches!(m, ModifierDecl::Atk { .. })) {
        100 // EFFECT_UPDATE_ATTACK
    } else if ce.modifiers.iter().any(|m| matches!(m, ModifierDecl::Def { .. })) {
        104 // EFFECT_UPDATE_DEFENSE
    } else if ce.modifiers.iter().any(|m| matches!(m, ModifierDecl::Grant(_))) {
        2 // EFFECT_DISABLE
    } else {
        0
    };

    // Range: where the card must be for this effect to apply
    let range = if card.is_spell() || card.is_trap() {
        type_mapper::LOCATION_SZONE
    } else {
        type_mapper::LOCATION_MZONE
    };
    // Field spells use LOCATION_FZONE
    let range = if card.card_types.contains(&CardType::FieldSpell) {
        type_mapper::LOCATION_FZONE
    } else {
        range
    };

    // Build a synthetic EffectBody for callbacks (continuous effects don't have
    // the standard condition/cost/target/operation, but we synthesize one)
    let body = EffectBody::default();

    CompiledEffect {
        effect_type,
        category: 0, // continuous effects don't have categories
        code,
        property: 0,
        range,
        count_limit: None,
        callbacks: callback_gen::GeneratedCallbacks {
            condition: None,
            cost: None,
            target: None,
            operation: None,
        },
        source: body,
    }
}

fn compile_replacement_effect(re: &ReplacementEffect, card: &Card) -> CompiledEffect {
    // Replacement effects intercept events before they happen
    let code = match &re.instead_of {
        ReplaceableEvent::DestroyedByBattle  => 0x1000014, // EFFECT_DESTROY_REPLACE
        ReplaceableEvent::DestroyedByEffect  => 0x1000014,
        ReplaceableEvent::DestroyedByAny     => 0x1000014,
        ReplaceableEvent::SentToGy           => 0x1000015, // EFFECT_SEND_REPLACE
        ReplaceableEvent::SentToGyByEffect   => 0x1000015,
        ReplaceableEvent::SentToGyByBattle   => 0x1000015,
        ReplaceableEvent::Banished           => 0x1000016,
        ReplaceableEvent::ReturnedToHand     => 0x1000017,
        ReplaceableEvent::ReturnedToDeck     => 0x1000018,
    };

    let range = if card.is_monster() {
        type_mapper::LOCATION_MZONE
    } else {
        type_mapper::LOCATION_SZONE
    };

    let body = EffectBody::default();

    CompiledEffect {
        effect_type: type_mapper::EFFECT_TYPE_SINGLE | type_mapper::EFFECT_TYPE_CONTINUOUS,
        category: type_mapper::CATEGORY_DESTROY, // replacement usually involves destruction
        code,
        property: 0,
        range,
        count_limit: None,
        callbacks: callback_gen::GeneratedCallbacks {
            condition: None,
            cost: None,
            target: None,
            operation: None,
        },
        source: body,
    }
}

fn compile_equip_effect(eq: &EquipEffect, card: &Card) -> CompiledEffect {
    let _ = eq; // Equip effects are complex — the modifiers are applied as EFFECT_TYPE_EQUIP

    let body = EffectBody::default();

    CompiledEffect {
        effect_type: type_mapper::EFFECT_TYPE_EQUIP,
        category: type_mapper::CATEGORY_EQUIP,
        code: 0, // Equip effects use value codes like EFFECT_UPDATE_ATTACK
        property: 0,
        range: type_mapper::LOCATION_SZONE,
        count_limit: None,
        callbacks: callback_gen::GeneratedCallbacks {
            condition: None,
            cost: None,
            target: None,
            operation: None,
        },
        source: body,
    }
}

/// Some effects expand into multiple CompiledEffects.
/// E.g., Counter Traps with "when_summoned" need 3 effects for
/// EVENT_SUMMON, EVENT_FLIP_SUMMON, EVENT_SPSUMMON (like Lua's Clone pattern).
fn compile_effect_expanded(effect: &Effect, card: &Card) -> Vec<CompiledEffect> {
    // Check if this is a "negate summon" pattern on a trap (like Solemn Judgment)
    if (card.is_trap() || card.is_spell()) {
        if let Some(TriggerExpr::WhenSummoned(None)) = &effect.body.trigger {
            // Expand to 3 events: summon, flip summon, special summon
            let events = vec![
                type_mapper::EVENT_SUMMON,
                type_mapper::EVENT_FLIP_SUMMON,
                type_mapper::EVENT_SPSUMMON,
            ];
            return events.iter().map(|&event_code| {
                let mut ce = compile_effect(effect, card);
                ce.code = event_code;
                ce
            }).collect();
        }
    }

    vec![compile_effect(effect, card)]
}

fn compile_effect(effect: &Effect, card: &Card) -> CompiledEffect {
    let body = &effect.body;
    let mut effect_type = type_mapper::effect_type_flags(body, card);
    let mut code = type_mapper::trigger_to_event_code(&body.trigger);

    // Ignition effects (monster, no trigger, speed 1) have code=0, not EVENT_FREE_CHAIN
    if effect_type == type_mapper::EFFECT_TYPE_IGNITION {
        code = 0;
    }

    // Monster trigger effects: FIELD (watches the field) vs SINGLE (watches this card)
    if card.is_monster() && body.trigger.is_some()
        && (effect_type == type_mapper::EFFECT_TYPE_TRIGGER_O
            || effect_type == type_mapper::EFFECT_TYPE_TRIGGER_F)
    {
        let is_self_trigger = matches!(&body.trigger, Some(
            TriggerExpr::WhenAttacked
            | TriggerExpr::WhenDestroyed(_)
            | TriggerExpr::WhenSentTo { .. }
            | TriggerExpr::WhenFlipped
            | TriggerExpr::WhenTributed(_)
            | TriggerExpr::WhenTributeSummoned(_)
        ));
        if is_self_trigger {
            effect_type |= type_mapper::EFFECT_TYPE_SINGLE;
        } else {
            effect_type |= type_mapper::EFFECT_TYPE_FIELD;
        }
    }

    // SINGLE trigger effects have range=0 (inherent to the card itself)
    let range = if effect_type & type_mapper::EFFECT_TYPE_SINGLE != 0
        && effect_type & (type_mapper::EFFECT_TYPE_TRIGGER_O | type_mapper::EFFECT_TYPE_TRIGGER_F) != 0
    {
        0
    } else {
        type_mapper::activation_range(body, card)
    };

    // Ignition effects: Lua scripts don't set category at registration time —
    // they set it dynamically in the target function. So category=0 for ignition.
    let category = if effect_type == type_mapper::EFFECT_TYPE_IGNITION {
        0
    } else {
        type_mapper::categories_from_actions(&body.on_resolve)
    };

    CompiledEffect {
        effect_type,
        category,
        code,
        property: type_mapper::property_flags(body),
        range,
        count_limit: type_mapper::frequency_to_count_limit(&body.frequency, card.password.unwrap_or(0)),
        callbacks: callback_gen::generate_callbacks(body, card),
        source: body.clone(),
    }
}
