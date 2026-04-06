// ============================================================
// Sprint 5 Integration Tests — Continuous, Equip, Synchro, Link
// ============================================================

use duelscript::parse;
use duelscript::compiler::{compile_card, CompiledCard};
use duelscript::compiler::type_mapper::*;

fn parse_and_compile(source: &str) -> CompiledCard {
    let file = parse(source).unwrap_or_else(|e| panic!("parse failed: {}", e));
    assert!(!file.cards.is_empty(), "no cards parsed");
    compile_card(&file.cards[0])
}

// ── United We Stand (Equip Spell) ─────────────────────────────

#[test]
fn test_united_we_stand_parses() {
    let source = include_str!("../cards/official/c56747793.ds");
    let file = parse(source).expect("United We Stand should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "United We Stand");
    assert_eq!(card.password, Some(56747793));
    assert!(!card.equip_effects.is_empty(), "should have equip_effect block");

    let eq = &card.equip_effects[0];
    assert!(!eq.while_equipped.is_empty(), "should have while_equipped modifiers");
}

#[test]
fn test_united_we_stand_compiles() {
    let source = include_str!("../cards/official/c56747793.ds");
    let compiled = parse_and_compile(source);

    // Should have at least one effect (the equip effect)
    assert!(!compiled.effects.is_empty(), "should compile equip effects");

    let e = &compiled.effects[0];
    assert_eq!(e.effect_type, EFFECT_TYPE_EQUIP,
        "should be EQUIP type, got {:#x}", e.effect_type);
}

// ── Stardust Dragon (Synchro Monster) ─────────────────────────

#[test]
fn test_stardust_dragon_parses() {
    let source = include_str!("../cards/official/c44508094.ds");
    let file = parse(source).expect("Stardust Dragon should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "Stardust Dragon");
    assert_eq!(card.password, Some(44508094));
    assert!(card.materials.is_some(), "should have materials block");
    assert_eq!(card.effects.len(), 2, "should have 2 effects");

    // Verify materials
    let mats = card.materials.as_ref().unwrap();
    assert_eq!(mats.slots.len(), 2, "should have 2 material slots (1 tuner + 1+ non-tuner)");
}

#[test]
fn test_stardust_dragon_compiles() {
    let source = include_str!("../cards/official/c44508094.ds");
    let compiled = parse_and_compile(source);

    assert_eq!(compiled.card_id, 44508094);
    // 1 synchro proc + 2 card effects = 3
    assert!(compiled.effects.len() >= 3, "expected at least 3 effects, got {}", compiled.effects.len());

    // Skip proc effects — card effects start at index 1
    let e1 = &compiled.effects[1];
    assert_eq!(e1.effect_type, EFFECT_TYPE_QUICK_O,
        "Negate should be QUICK_O, got {:#x}", e1.effect_type);
    assert!(e1.category & CATEGORY_NEGATE != 0, "should have NEGATE");
    assert!(e1.category & CATEGORY_DESTROY != 0, "should have DESTROY");
    assert_eq!(e1.range, LOCATION_MZONE);

    // Effect 2: Revive Self — FIELD | TRIGGER_O (triggered from GY during End Phase)
    let e2 = &compiled.effects[2];
    assert_eq!(e2.effect_type, EFFECT_TYPE_FIELD | EFFECT_TYPE_TRIGGER_O,
        "Revive should be FIELD|TRIGGER_O, got {:#x}", e2.effect_type);
    assert!(e2.category & CATEGORY_SPECIAL_SUMMON != 0, "should have SPECIAL_SUMMON");
}

// ── Decode Talker (Link Monster) ──────────────────────────────

#[test]
fn test_decode_talker_parses() {
    let source = include_str!("../cards/official/c1861629.ds");
    let file = parse(source).expect("Decode Talker should parse");
    let card = &file.cards[0];
    assert_eq!(card.name, "Decode Talker");
    assert_eq!(card.password, Some(1861629));
    assert_eq!(card.link, Some(3));
    assert_eq!(card.link_arrows.len(), 3, "should have 3 link arrows");

    // Materials
    assert!(card.materials.is_some(), "should have materials");

    // Should have 1 continuous effect + 1 triggered effect
    assert!(!card.continuous_effects.is_empty(), "should have continuous effect");
    assert_eq!(card.effects.len(), 1, "should have 1 triggered effect");
}

#[test]
fn test_decode_talker_compiles() {
    let source = include_str!("../cards/official/c1861629.ds");
    let compiled = parse_and_compile(source);

    assert_eq!(compiled.card_id, 1861629);
    // 1 link proc + 1 continuous + 1 triggered = 3
    assert!(compiled.effects.len() >= 3,
        "should have at least 3 effects, got {}", compiled.effects.len());

    // Find the continuous effect (EFFECT_TYPE_FIELD or SINGLE)
    let continuous = compiled.effects.iter()
        .find(|e| e.effect_type & EFFECT_TYPE_FIELD != 0 || e.effect_type & EFFECT_TYPE_SINGLE != 0);
    assert!(continuous.is_some(), "should have a continuous effect");

    // Find the quick effect (negate)
    let negate = compiled.effects.iter()
        .find(|e| e.effect_type == EFFECT_TYPE_QUICK_O);
    assert!(negate.is_some(), "should have a quick negate effect");
    let negate = negate.unwrap();
    assert!(negate.category & CATEGORY_NEGATE != 0);
    assert!(negate.category & CATEGORY_DESTROY != 0);
}

// ── Cross-card: verify all Sprint 3+4+5 cards still parse ────

#[test]
fn test_all_official_cards_parse() {
    let cards = vec![
        ("c55144522.ds", "Pot of Greed"),
        ("c53129443.ds", "Dark Hole"),
        ("c83764718.ds", "Monster Reborn"),
        ("c14558127.ds", "Ash Blossom & Joyous Spring"),
        ("c84013237.ds", "Number 39: Utopia"),
        ("c41420027.ds", "Solemn Judgment"),
        ("c82732705.ds", "Skill Drain"),
        ("c10080320.ds", "Jurassic World"),
        ("c10000030.ds", "Magi Magi Magician Gal"),
        ("c56747793.ds", "United We Stand"),
        ("c44508094.ds", "Stardust Dragon"),
        ("c1861629.ds",  "Decode Talker"),
    ];

    let dir = std::path::Path::new("cards/official");
    for (file, expected_name) in &cards {
        let path = dir.join(file);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("can't read {}: {}", file, e));
        let parsed = parse(&source)
            .unwrap_or_else(|e| panic!("{} failed to parse: {}", file, e));
        assert!(!parsed.cards.is_empty(), "{} parsed to 0 cards", file);
        assert_eq!(parsed.cards[0].name, *expected_name, "{} wrong name", file);
    }
}

#[test]
fn test_link_arrows_parse_minimal() {
    let source = r#"card "Test" {
    type: Link Monster
    link: 3
    atk: 2300
    attribute: DARK
    race: Cyberse
    password: 1
    link_arrows: [top, bottom_left, bottom_right]
}"#;
    let file = parse(source).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 3);
}

#[test]
fn test_link_arrows_bare() {
    let source = r#"card "T" { link_arrows: [top] }"#;
    let file = parse(source).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 1);
}

#[test]
fn test_link_arrows_after_materials() {
    let source = r#"card "T" {
    type: Link Monster
    link: 3
    atk: 2300
    attribute: DARK
    race: Cyberse
    password: 1

    materials {
        require: 2+ effect monster
        method: link
    }

    link_arrows: [top, bottom_left, bottom_right]
}"#;
    let file = parse(source).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 3);
}

#[test]
fn test_link_arrows_after_materials_no_method() {
    let source = r#"card "T" {
    type: Link Monster
    link: 3
    atk: 2300
    attribute: DARK
    race: Cyberse
    password: 1
    materials {
        require: 2+ effect monster
    }
    link_arrows: [top, bottom_left, bottom_right]
}"#;
    let file = parse(source).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 3);
}

#[test]
fn test_link_arrows_after_simple_materials() {
    let source = r#"card "T" {
    type: Link Monster
    link: 2
    atk: 1000
    attribute: DARK
    race: Cyberse
    password: 1
    materials {
        require: 2 monster
    }
    link_arrows: [top]
}"#;
    let file = parse(source).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 1);
}

#[test]
fn test_materials_2plus_effect_monster() {
    let source = r#"card "T" {
    type: Link Monster
    link: 2
    atk: 1000
    attribute: DARK
    race: Cyberse
    password: 1
    materials {
        require: 2+ effect monster
    }
    link_arrows: [top]
}"#;
    let file = parse(source).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 1);
}

#[test]
fn test_materials_with_method_link() {
    let source = r#"card "T" {
    type: Link Monster
    link: 2
    atk: 1000
    attribute: DARK
    race: Cyberse
    password: 1
    materials {
        require: 2+ effect monster
        method: link
    }
    link_arrows: [top]
}"#;
    let file = parse(source).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 1);
}

#[test]
fn test_decode_talker_exact() {
    let source = include_str!("../cards/official/c1861629.ds");
    // Try to parse just the first few fields
    let file = parse(source);
    match file {
        Ok(f) => println!("Parsed {} cards", f.cards.len()),
        Err(e) => panic!("parse error: {}", e),
    }
}

#[test]
fn test_decode_progressively() {
    // Step 1: materials + link_arrows only
    let s1 = r#"card "T" {
    type: Link Monster | Effect Monster
    attribute: DARK
    race: Cyberse
    link: 3
    atk: 2300
    password: 1
    materials {
        require: 2+ effect monster
        method: link
    }
    link_arrows: [top, bottom_left, bottom_right]
}"#;
    parse(s1).unwrap_or_else(|e| panic!("Step 1 failed: {}", e));

    // Step 2: add summon_condition
    let s2 = r#"card "T" {
    type: Link Monster | Effect Monster
    attribute: DARK
    race: Cyberse
    link: 3
    atk: 2300
    password: 1
    materials {
        require: 2+ effect monster
        method: link
    }
    link_arrows: [top, bottom_left, bottom_right]
    summon_condition {
        cannot_normal_summon: true
    }
}"#;
    parse(s2).unwrap_or_else(|e| panic!("Step 2 failed: {}", e));
}

#[test]
fn test_link_type_pipe() {
    let s = r#"card "T" {
    type: Link Monster | Effect Monster
    link: 2
    atk: 1000
    attribute: DARK
    race: Cyberse
    password: 1
    materials {
        require: 2+ effect monster
        method: link
    }
    link_arrows: [top]
}"#;
    let file = parse(s).unwrap_or_else(|e| panic!("parse error: {}", e));
    println!("types: {:?}", file.cards[0].card_types);
}

#[test]
fn test_link_method_arrows_combined() {
    let s = r#"card "T" {
    type: Link Monster | Effect Monster
    attribute: DARK
    race: Cyberse
    link: 3
    atk: 2300
    password: 1861629
    materials {
        require: 2+ effect monster
        method: link
    }
    link_arrows: [top, bottom_left, bottom_right]
}"#;
    let file = parse(s).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 3);
}

#[test]
fn test_password_long() {
    let s = r#"card "T" {
    type: Link Monster
    link: 2
    atk: 1000
    attribute: DARK
    race: Cyberse
    password: 1861629
    materials {
        require: 2+ effect monster
        method: link
    }
    link_arrows: [top]
}"#;
    let file = parse(s).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].password, Some(1861629));
}

#[test]
fn test_three_arrows() {
    let s = r#"card "T" {
    type: Link Monster
    link: 3
    atk: 2300
    attribute: DARK
    race: Cyberse
    password: 1861629
    materials {
        require: 2+ effect monster
        method: link
    }
    link_arrows: [top, bottom_left, bottom_right]
}"#;
    let file = parse(s).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 3);
}

#[test]
fn test_two_arrows() {
    let s = r#"card "T" {
    type: Link Monster
    link: 2
    atk: 1000
    attribute: DARK
    race: Cyberse
    password: 1
    materials { require: 2+ effect monster method: link }
    link_arrows: [top, bottom_left]
}"#;
    let file = parse(s).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 2);
}

#[test]
fn test_bottom_right_arrow() {
    let s = r#"card "T" { link_arrows: [bottom_right] }"#;
    let file = parse(s).unwrap_or_else(|e| panic!("parse error: {}", e));
    assert_eq!(file.cards[0].link_arrows.len(), 1);
}
