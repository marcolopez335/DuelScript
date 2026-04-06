// ============================================================
// Sprint 4 Integration Tests — Complex card patterns
// ============================================================

use duelscript::parse;
use duelscript::compiler::{compile_card, CompiledCard};
use duelscript::compiler::type_mapper::*;

fn parse_and_compile(source: &str) -> CompiledCard {
    let file = parse(source).unwrap_or_else(|e| panic!("parse failed: {}", e));
    assert!(!file.cards.is_empty(), "no cards parsed");
    compile_card(&file.cards[0])
}

// ── Number 39: Utopia ─────────────────────────────────────────

#[test]
fn test_utopia_parses() {
    let source = include_str!("../cards/official/c84013237.ds");
    let file = parse(source).expect("Utopia should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "Number 39: Utopia");
    assert_eq!(card.password, Some(84013237));
    assert!(card.materials.is_some(), "Utopia should have materials");
    assert_eq!(card.effects.len(), 2, "Utopia should have 2 effects");
}

#[test]
fn test_utopia_compiles() {
    let source = include_str!("../cards/official/c84013237.ds");
    let compiled = parse_and_compile(source);
    // 2 summoning procedure effects + 2 card effects = 4
    assert_eq!(compiled.effects.len(), 4);

    // Effects 0-1: Summoning procedure (check + SPSUMMON_PROC)
    assert_eq!(compiled.effects[0].code, 946, "Check effect for Xyz");
    assert_eq!(compiled.effects[1].code, 34, "EFFECT_SPSUMMON_PROC");

    // Effect 2: Negate Attack — FIELD | TRIGGER_O (monster watching the field)
    let e1 = &compiled.effects[2];
    assert_eq!(e1.effect_type, EFFECT_TYPE_FIELD | EFFECT_TYPE_TRIGGER_O,
        "Negate Attack should be FIELD|TRIGGER_O, got {:#x}", e1.effect_type);
    assert_eq!(e1.range, LOCATION_MZONE);

    // Effect 3: Self Destroy — SINGLE | TRIGGER_F (watches this card only)
    let e2 = &compiled.effects[3];
    assert_eq!(e2.effect_type, EFFECT_TYPE_SINGLE | EFFECT_TYPE_TRIGGER_F,
        "Self Destroy should be SINGLE|TRIGGER_F, got {:#x}", e2.effect_type);
}

// ── Solemn Judgment ───────────────────────────────────────────

#[test]
fn test_solemn_judgment_parses() {
    let source = include_str!("../cards/official/c41420027.ds");
    let file = parse(source).expect("Solemn Judgment should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "Solemn Judgment");
    assert_eq!(card.effects.len(), 2, "Solemn should have 2 effects (negate summon + negate activation)");
}

#[test]
fn test_solemn_judgment_compiles() {
    let source = include_str!("../cards/official/c41420027.ds");
    let compiled = parse_and_compile(source);

    // Effect 1: Negate Summon
    let e1 = &compiled.effects[0];
    // Counter Trap = EFFECT_TYPE_ACTIVATE with spell_speed_3
    assert_eq!(e1.effect_type, EFFECT_TYPE_ACTIVATE);
    // Effect 1 negates summon: CATEGORY_DISABLE_SUMMON + CATEGORY_DESTROY
    assert!(e1.category & CATEGORY_DISABLE_SUMMON != 0,
        "should have DISABLE_SUMMON category, got {:#x}", e1.category);
    assert!(e1.category & CATEGORY_DESTROY != 0, "should have DESTROY category");

    // Cost should include pay_lp (dynamic expr: your_lp / 2)
    assert!(e1.callbacks.cost.is_some(), "should have cost callback");
}

// ── Jurassic World (Field Spell) ──────────────────────────────

#[test]
fn test_jurassic_world_parses() {
    let source = include_str!("../cards/official/c10080320.ds");
    let file = parse(source).expect("Jurassic World should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "Jurassic World");
    assert!(card.continuous_effects.len() >= 1, "should have continuous effect");
}

// ── Skill Drain (Continuous Trap) ─────────────────────────────

#[test]
fn test_skill_drain_parses() {
    let source = include_str!("../cards/official/c82732705.ds");
    let file = parse(source).expect("Skill Drain should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "Skill Drain");
    assert!(card.continuous_effects.len() >= 1, "should have continuous effect");
}

// ── Magi Magi (Choose) ───────────────────────────────────────

#[test]
fn test_magi_magi_parses() {
    let source = include_str!("../cards/official/c10000030.ds");
    let file = parse(source).expect("Magi Magi should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "Magi Magi Magician Gal");
    assert!(card.materials.is_some(), "should have materials");
    assert_eq!(card.effects.len(), 1);

    // The effect's on_resolve should contain a Choose action
    let actions = &card.effects[0].body.on_resolve;
    assert!(!actions.is_empty(), "should have on_resolve actions");
}

#[test]
fn test_magi_magi_compiles() {
    let source = include_str!("../cards/official/c10000030.ds");
    let compiled = parse_and_compile(source);

    // 2 proc effects + 1 card effect = 3
    assert_eq!(compiled.effects.len(), 3);

    // Skip proc effects (0, 1) — card effect is at index 2
    let e1 = &compiled.effects[2];
    // Ignition effect (spell speed 1, no trigger, monster)
    assert_eq!(e1.effect_type, EFFECT_TYPE_IGNITION);
    assert_eq!(e1.range, LOCATION_MZONE);
    // Ignition effects have category=0 (Lua sets category dynamically in target function)
    assert_eq!(e1.category, 0, "Ignition should have category=0, got {:#x}", e1.category);
}
