// ============================================================
// v0.6 Feature Tests — verify new constructs parse correctly
// ============================================================

use duelscript::parse;

#[test]
fn test_and_if_you_do_parses() {
    let source = r#"card "Test Card" {
    type: Normal Spell
    password: 1

    effect "Combo" {
        speed: spell_speed_1
        on_resolve {
            destroy (1, card, opponent controls)
            and_if_you_do {
                draw 1
            }
        }
    }
}"#;
    let file = parse(source).expect("Should parse and_if_you_do");
    assert_eq!(file.cards.len(), 1);
    assert_eq!(file.cards[0].effects.len(), 1);
    assert_eq!(file.cards[0].effects[0].body.on_resolve.len(), 2);
}

#[test]
fn test_then_parses() {
    let source = r#"card "Test Card" {
    type: Normal Spell
    password: 2

    effect "Sequential" {
        speed: spell_speed_1
        on_resolve {
            banish (1, card)
            then {
                draw 1
            }
        }
    }
}"#;
    let file = parse(source).expect("Should parse then");
    assert_eq!(file.cards[0].effects[0].body.on_resolve.len(), 2);
}

#[test]
fn test_also_parses() {
    let source = r#"card "Test Card" {
    type: Normal Spell
    password: 3

    effect "Simultaneous" {
        speed: spell_speed_1
        on_resolve {
            destroy (1, card)
            also {
                deal_damage to opponent: 500
            }
        }
    }
}"#;
    let file = parse(source).expect("Should parse also");
    assert_eq!(file.cards[0].effects[0].body.on_resolve.len(), 2);
}

#[test]
fn test_new_battle_modifiers_parse() {
    let source = r#"card "Battle Beast" {
    type: Effect Monster
    attribute: FIRE
    race: Warrior
    level: 4
    atk: 2000
    def: 1000
    password: 4

    continuous_effect "Combat Abilities" {
        while: on_field
        apply_to: self
        grant: piercing
        grant: attack_twice
        grant: cannot_be_targeted_by_opponent
        grant: cannot_be_destroyed_by_battle
        grant: must_attack_if_able
    }
}"#;
    let file = parse(source).expect("Should parse new battle modifiers");
    let ce = &file.cards[0].continuous_effects[0];
    assert!(ce.modifiers.len() >= 5, "should have 5+ modifiers");
}

#[test]
fn test_raw_effect_parses() {
    let source = r#"card "Pot of Greed" {
    type: Normal Spell
    password: 55144522

    raw_effect "Draw 2" {
        effect_type: 16
        category: 65536
        code: 1002
        property: 2048
        on_resolve {
            draw 2
        }
    }
}"#;
    let file = parse(source).expect("Should parse raw_effect");
    assert_eq!(file.cards[0].raw_effects.len(), 1);
    let raw = &file.cards[0].raw_effects[0];
    assert_eq!(raw.effect_type, 16);
    assert_eq!(raw.category, 65536);
    assert_eq!(raw.code, 1002);
    assert_eq!(raw.property, 2048);
}

#[test]
fn test_nested_sequential_resolution() {
    let source = r#"card "Chain" {
    type: Normal Spell
    password: 5

    effect "Nested" {
        speed: spell_speed_1
        on_resolve {
            search (1, "Dragon" monster) from deck
            and_if_you_do {
                add_to_hand (1, card) from gy
                then {
                    draw 1
                }
            }
        }
    }
}"#;
    let file = parse(source).expect("Should parse nested sequential");
    assert_eq!(file.cards.len(), 1);
}
