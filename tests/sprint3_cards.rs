// ============================================================
// Sprint 3 Integration Tests — Parse + compile real cards
// Verifies .ds files produce correct engine-level bitfields.
// ============================================================

use duelscript::parse;
use duelscript::compiler::{compile_card, CompiledCard};
use duelscript::compiler::type_mapper::*;

// ── Helper ────────────────────────────────────────────────────

fn parse_and_compile(source: &str) -> CompiledCard {
    let file = parse(source).expect("parse failed");
    assert_eq!(file.cards.len(), 1, "expected exactly 1 card");
    compile_card(&file.cards[0])
}

// ── Pot of Greed ──────────────────────────────────────────────

#[test]
fn test_pot_of_greed_parses() {
    let source = include_str!("../cards/official/c55144522.ds");
    let file = parse(source).expect("Pot of Greed should parse");
    assert_eq!(file.cards.len(), 1);
    assert_eq!(file.cards[0].name, "Pot of Greed");
    assert_eq!(file.cards[0].password, Some(55144522));
    assert_eq!(file.cards[0].effects.len(), 1);
}

#[test]
fn test_pot_of_greed_compiles() {
    let source = include_str!("../cards/official/c55144522.ds");
    let compiled = parse_and_compile(source);

    assert_eq!(compiled.card_id, 55144522);
    assert_eq!(compiled.effects.len(), 1);

    let effect = &compiled.effects[0];
    // Lua: SetType(EFFECT_TYPE_ACTIVATE) = 0x10
    assert_eq!(effect.effect_type, EFFECT_TYPE_ACTIVATE);
    // Lua: SetCategory(CATEGORY_DRAW) = 0x20
    assert_eq!(effect.category, CATEGORY_DRAW);
    // Lua: SetCode(EVENT_FREE_CHAIN) = 1002
    assert_eq!(effect.code, EVENT_FREE_CHAIN);
    // Activated spells: range=0 (engine handles activation location)
    assert_eq!(effect.range, 0);
    // No OPT
    assert!(effect.count_limit.is_none());
}

// ── Dark Hole ─────────────────────────────────────────────────

#[test]
fn test_dark_hole_parses() {
    let source = include_str!("../cards/official/c53129443.ds");
    let file = parse(source).expect("Dark Hole should parse");
    assert_eq!(file.cards[0].name, "Dark Hole");
    assert_eq!(file.cards[0].effects.len(), 1);
}

#[test]
fn test_dark_hole_compiles() {
    let source = include_str!("../cards/official/c53129443.ds");
    let compiled = parse_and_compile(source);

    let effect = &compiled.effects[0];
    // Lua: SetType(EFFECT_TYPE_ACTIVATE)
    assert_eq!(effect.effect_type, EFFECT_TYPE_ACTIVATE);
    // Lua: SetCategory(CATEGORY_DESTROY)
    assert_eq!(effect.category, CATEGORY_DESTROY);
    // Lua: SetCode(EVENT_FREE_CHAIN)
    assert_eq!(effect.code, EVENT_FREE_CHAIN);
    assert_eq!(effect.range, 0);
}

// ── Monster Reborn ────────────────────────────────────────────

#[test]
fn test_monster_reborn_parses() {
    let source = include_str!("../cards/official/c83764718.ds");
    let file = parse(source).expect("Monster Reborn should parse");
    assert_eq!(file.cards[0].name, "Monster Reborn");
    assert_eq!(file.cards[0].effects.len(), 1);
}

#[test]
fn test_monster_reborn_compiles() {
    let source = include_str!("../cards/official/c83764718.ds");
    let compiled = parse_and_compile(source);

    let effect = &compiled.effects[0];
    // Lua: SetType(EFFECT_TYPE_ACTIVATE)
    assert_eq!(effect.effect_type, EFFECT_TYPE_ACTIVATE);
    // Lua: SetCategory(CATEGORY_SPECIAL_SUMMON)
    assert_eq!(effect.category, CATEGORY_SPECIAL_SUMMON);
    // Lua: SetCode(EVENT_FREE_CHAIN)
    assert_eq!(effect.code, EVENT_FREE_CHAIN);
    // Lua: SetProperty(EFFECT_FLAG_CARD_TARGET) — should have targeting flag
    assert!(effect.property & EFFECT_FLAG_CARD_TARGET != 0,
        "Monster Reborn should have CARD_TARGET property, got {:#x}", effect.property);
    assert_eq!(effect.range, 0);
}

// ── Ash Blossom ───────────────────────────────────────────────

#[test]
fn test_ash_blossom_parses() {
    let source = include_str!("../cards/official/c14558127.ds");
    let file = parse(source).expect("Ash Blossom should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "Ash Blossom & Joyous Spring");
    assert_eq!(card.password, Some(14558127));
    assert_eq!(card.effects.len(), 1);

    let effect = &card.effects[0].body;
    // Should have a condition (chain_link_includes)
    assert!(effect.condition.is_some(), "Ash should have a condition");
    // Should have a trigger (opponent_activates)
    assert!(effect.trigger.is_some(), "Ash should have a trigger");
    // Should have cost (discard self)
    assert!(!effect.cost.is_empty(), "Ash should have a cost");
    // Should have on_resolve (negate)
    assert!(!effect.on_resolve.is_empty(), "Ash should have on_resolve");
}

#[test]
fn test_ash_blossom_compiles() {
    let source = include_str!("../cards/official/c14558127.ds");
    let compiled = parse_and_compile(source);

    assert_eq!(compiled.card_id, 14558127);
    let effect = &compiled.effects[0];

    // Lua: SetType(EFFECT_TYPE_QUICK_O) — hand trap, speed 2, optional trigger
    assert_eq!(effect.effect_type, EFFECT_TYPE_QUICK_O,
        "Ash should be QUICK_O, got {:#x}", effect.effect_type);
    // Lua: SetCode(EVENT_CHAINING) — triggers on opponent's chain link
    assert_eq!(effect.code, EVENT_CHAINING);
    // Lua: SetRange(LOCATION_HAND) — activates from hand
    assert_eq!(effect.range, LOCATION_HAND);
    // Lua: SetCountLimit(1, id) — hard OPT
    assert!(effect.count_limit.is_some(), "Ash should have count limit");
    let cl = effect.count_limit.as_ref().unwrap();
    assert_eq!(cl.count, 1);
    assert_eq!(cl.code, 14558127, "Hard OPT code should be card ID");

    // Should have callbacks generated
    assert!(effect.callbacks.condition.is_some(), "Should have condition callback");
    assert!(effect.callbacks.cost.is_some(), "Should have cost callback");
    assert!(effect.callbacks.operation.is_some(), "Should have operation callback");
}

#[test]
fn test_minimal_parse() {
    let source = r#"card "Test" {
    type: Normal Spell
    password: 12345

    effect "Test" {
        speed: spell_speed_1
        on_resolve {
            draw 2
        }
    }
}"#;
    let file = parse(source).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards.len(), 1, "Expected 1 card, got {}", file.cards.len());
}
