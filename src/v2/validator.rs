// ============================================================
// DuelScript v2 Validator
// Catches semantic errors after parsing. Returns errors+warnings.
// ============================================================

use super::ast::*;
use std::fmt;

// ── Public API ──────────────────────────────────────────────

pub fn validate_v2(file: &File) -> ValidationReport {
    let mut report = ValidationReport { errors: vec![] };
    for card in &file.cards {
        validate_card(card, &mut report.errors);
    }
    report
}

pub fn validate_card(card: &Card, errors: &mut Vec<ValidationError>) {
    let ctx = Ctx::new(card);
    check_required_fields(&ctx, errors);
    check_stat_ranges(&ctx, errors);
    check_type_consistency(&ctx, errors);
    check_level_tribute_consistency(&ctx, errors);
    check_link_arrows(&ctx, errors);
    check_summon_block(&ctx, errors);
    check_effect_blocks(&ctx, errors);
    check_spell_speeds(&ctx, errors);
    check_passive_blocks(&ctx, errors);
    check_restriction_blocks(&ctx, errors);
    check_replacement_blocks(&ctx, errors);
    check_redirect_blocks(&ctx, errors);
}

// ── Types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub errors: Vec<ValidationError>,
}

impl ValidationReport {
    pub fn has_errors(&self) -> bool {
        self.errors.iter().any(|e| e.severity == Severity::Error)
    }
    pub fn error_count(&self) -> usize {
        self.errors.iter().filter(|e| e.severity == Severity::Error).count()
    }
    pub fn warning_count(&self) -> usize {
        self.errors.iter().filter(|e| e.severity == Severity::Warning).count()
    }
}

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub card: String,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity { Error, Warning }

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let tag = match self.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARN",
        };
        write!(f, "[{}] {}: {}", tag, self.card, self.message)
    }
}

fn err(card: &str, msg: &str) -> ValidationError {
    ValidationError { card: card.to_string(), message: msg.to_string(), severity: Severity::Error }
}

fn warn(card: &str, msg: &str) -> ValidationError {
    ValidationError { card: card.to_string(), message: msg.to_string(), severity: Severity::Warning }
}

// ── Card Context ────────────────────────────────────────────

struct Ctx<'a> {
    card: &'a Card,
    is_monster: bool,
    is_spell: bool,
    is_trap: bool,
    is_extra_deck: bool,
    is_link: bool,
    is_xyz: bool,
    is_pendulum: bool,
    is_normal_monster: bool,
    is_ritual: bool,
    is_counter_trap: bool,
    is_quickplay: bool,
}

impl<'a> Ctx<'a> {
    fn new(card: &'a Card) -> Self {
        let types = &card.fields.card_types;
        let is_monster = types.iter().any(|t| matches!(t,
            CardType::NormalMonster | CardType::EffectMonster | CardType::RitualMonster
          | CardType::FusionMonster | CardType::SynchroMonster | CardType::XyzMonster
          | CardType::LinkMonster   | CardType::PendulumMonster
        ));
        let is_spell = types.iter().any(|t| matches!(t,
            CardType::NormalSpell | CardType::QuickPlaySpell | CardType::ContinuousSpell
          | CardType::EquipSpell  | CardType::FieldSpell     | CardType::RitualSpell
        ));
        let is_trap = types.iter().any(|t| matches!(t,
            CardType::NormalTrap | CardType::CounterTrap | CardType::ContinuousTrap
        ));
        let is_extra_deck = types.iter().any(|t| matches!(t,
            CardType::FusionMonster | CardType::SynchroMonster
          | CardType::XyzMonster   | CardType::LinkMonster
        ));
        Self {
            card, is_monster, is_spell, is_trap, is_extra_deck,
            is_link: types.contains(&CardType::LinkMonster),
            is_xyz: types.contains(&CardType::XyzMonster),
            is_pendulum: types.contains(&CardType::PendulumMonster),
            is_normal_monster: types.contains(&CardType::NormalMonster),
            is_ritual: types.contains(&CardType::RitualMonster),
            is_counter_trap: types.contains(&CardType::CounterTrap),
            is_quickplay: types.contains(&CardType::QuickPlaySpell),
        }
    }

    fn name(&self) -> &str { &self.card.name }

    #[allow(dead_code)]
    fn is_special_summon_only(&self) -> bool {
        self.card.summon.as_ref().map_or(false, |s| s.cannot_normal_summon)
    }
}

// ── Checks ──────────────────────────────────────────────────

fn check_required_fields(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    let f = &ctx.card.fields;

    if f.card_types.is_empty() {
        errors.push(err(ctx.name(), "Card must declare at least one type"));
    }

    if ctx.is_monster {
        if f.atk.is_none() {
            errors.push(err(ctx.name(), "Monster must declare 'atk'"));
        }
        if !ctx.is_link && f.def.is_none() {
            errors.push(err(ctx.name(), "Non-Link monster must declare 'def'"));
        }
        if ctx.is_link && f.def.is_some() {
            errors.push(err(ctx.name(), "Link monsters do not have DEF"));
        }
        if f.race.is_none() {
            errors.push(err(ctx.name(), "Monster must declare 'race'"));
        }
        if f.attribute.is_none() {
            errors.push(err(ctx.name(), "Monster must declare 'attribute'"));
        }

        // Level/rank/link
        if !ctx.is_xyz && !ctx.is_link && !ctx.is_extra_deck && f.level.is_none() {
            errors.push(err(ctx.name(), "Monster must declare 'level'"));
        }
        if ctx.is_xyz {
            if f.rank.is_none() {
                errors.push(err(ctx.name(), "Xyz monster must declare 'rank'"));
            }
            if f.level.is_some() {
                errors.push(err(ctx.name(), "Xyz monsters use 'rank', not 'level'"));
            }
        }
        if ctx.is_link {
            if f.link.is_none() {
                errors.push(err(ctx.name(), "Link monster must declare 'link' rating"));
            }
            if f.level.is_some() {
                errors.push(err(ctx.name(), "Link monsters use 'link', not 'level'"));
            }
        }
        if ctx.is_pendulum && f.scale.is_none() {
            errors.push(err(ctx.name(), "Pendulum monster must declare 'scale'"));
        }
    }

    // Normal monsters should have no effects
    if ctx.is_normal_monster && !ctx.card.effects.is_empty() {
        errors.push(err(ctx.name(), "Normal monsters cannot have effect blocks"));
    }
}

fn check_stat_ranges(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    let f = &ctx.card.fields;

    if let Some(StatVal::Number(atk)) = &f.atk {
        if *atk < 0 {
            errors.push(err(ctx.name(), "ATK cannot be negative"));
        }
        if *atk > 5000 {
            errors.push(warn(ctx.name(), &format!("ATK {} is unusually high", atk)));
        }
    }
    if let Some(StatVal::Number(def)) = &f.def {
        if *def < 0 {
            errors.push(err(ctx.name(), "DEF cannot be negative"));
        }
    }
    if let Some(level) = f.level {
        if level == 0 { errors.push(err(ctx.name(), "Level cannot be 0")); }
        if level > 12 { errors.push(warn(ctx.name(), &format!("Level {} is unusually high", level))); }
    }
    if let Some(rank) = f.rank {
        if rank > 13 { errors.push(warn(ctx.name(), &format!("Rank {} exceeds known Xyz ranks", rank))); }
    }
    if let Some(link) = f.link {
        if link == 0 { errors.push(err(ctx.name(), "Link rating cannot be 0")); }
        if link > 8 { errors.push(warn(ctx.name(), &format!("Link rating {} is unusually high", link))); }
    }
}

fn check_type_consistency(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    if ctx.is_monster && ctx.is_spell {
        errors.push(err(ctx.name(), "Card cannot be both Monster and Spell"));
    }
    if ctx.is_monster && ctx.is_trap {
        errors.push(err(ctx.name(), "Card cannot be both Monster and Trap"));
    }
    if ctx.is_spell && ctx.is_trap {
        errors.push(err(ctx.name(), "Card cannot be both Spell and Trap"));
    }

    // Tuner requires monster base type
    let types = &ctx.card.fields.card_types;
    if types.contains(&CardType::Tuner) && !ctx.is_monster {
        errors.push(err(ctx.name(), "'Tuner' requires a monster type"));
    }

    // Pendulum requires a base monster type
    if ctx.is_pendulum {
        let has_base = types.iter().any(|t| matches!(t,
            CardType::NormalMonster | CardType::EffectMonster
          | CardType::FusionMonster | CardType::SynchroMonster
          | CardType::XyzMonster | CardType::RitualMonster
        ));
        if !has_base {
            errors.push(err(ctx.name(), "Pendulum Monster must also declare a base monster type"));
        }
    }
}

fn check_level_tribute_consistency(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    if !ctx.is_monster || ctx.is_extra_deck { return; }

    let level = match ctx.card.fields.level {
        Some(l) => l,
        None => return,
    };

    let summon = match &ctx.card.summon {
        Some(s) => s,
        None => {
            // Level 5+ without summon block should warn (needs tributes)
            if level >= 5 {
                errors.push(warn(ctx.name(), &format!(
                    "Level {} monster should declare a summon block with tributes: {}",
                    level, if level <= 6 { 1 } else { 2 }
                )));
            }
            return;
        }
    };

    if summon.cannot_normal_summon { return; } // special summon only

    if let Some(tributes) = summon.tributes {
        let expected = match level {
            5..=6 => Some(1u32),
            7..=12 => Some(2),
            _ => None,
        };
        if let Some(exp) = expected {
            if tributes != exp {
                errors.push(err(ctx.name(), &format!(
                    "Level {} needs {} tribute(s), declared {}",
                    level, exp, tributes
                )));
            }
        }
    } else if level >= 5 {
        errors.push(warn(ctx.name(), &format!(
            "Level {} monster should declare tributes: {}",
            level, if level <= 6 { 1 } else { 2 }
        )));
    }
}

fn check_link_arrows(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    if !ctx.is_link { return; }

    let arrows = &ctx.card.fields.link_arrows;
    let rating = ctx.card.fields.link.unwrap_or(0) as usize;

    if arrows.is_empty() {
        errors.push(err(ctx.name(), "Link monster must declare link_arrows"));
        return;
    }
    if arrows.len() != rating {
        errors.push(err(ctx.name(), &format!(
            "Link {} needs {} arrow(s), found {}", rating, rating, arrows.len()
        )));
    }

    // No duplicates
    let mut seen = std::collections::HashSet::new();
    for arrow in arrows {
        if !seen.insert(format!("{:?}", arrow)) {
            errors.push(err(ctx.name(), &format!("Duplicate link arrow: {:?}", arrow)));
        }
    }
}

fn check_summon_block(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    let summon = match &ctx.card.summon {
        Some(s) => s,
        None => return,
    };

    if !ctx.is_monster {
        errors.push(err(ctx.name(), "Summon block only applies to monsters"));
    }

    // Fusion materials on non-Fusion monster
    if summon.fusion_materials.is_some() {
        let types = &ctx.card.fields.card_types;
        if !types.contains(&CardType::FusionMonster) {
            errors.push(err(ctx.name(), "Fusion materials declared on non-Fusion monster"));
        }
    }
    // Synchro materials on non-Synchro
    if summon.synchro_materials.is_some() {
        if !ctx.card.fields.card_types.contains(&CardType::SynchroMonster) {
            errors.push(err(ctx.name(), "Synchro materials declared on non-Synchro monster"));
        }
    }
    // Xyz materials on non-Xyz
    if summon.xyz_materials.is_some() {
        if !ctx.is_xyz {
            errors.push(err(ctx.name(), "Xyz materials declared on non-Xyz monster"));
        }
    }
    // Link materials on non-Link
    if summon.link_materials.is_some() {
        if !ctx.is_link {
            errors.push(err(ctx.name(), "Link materials declared on non-Link monster"));
        }
    }
    // Ritual materials on non-Ritual (note: ritual spells define materials, but monsters can too)
    if summon.ritual_materials.is_some() {
        if !ctx.is_ritual {
            errors.push(warn(ctx.name(), "Ritual materials on non-Ritual card — usually the Ritual Spell defines these"));
        }
    }

    // Extra deck monsters should have materials
    if ctx.is_extra_deck {
        let has_materials = summon.fusion_materials.is_some()
            || summon.synchro_materials.is_some()
            || summon.xyz_materials.is_some()
            || summon.link_materials.is_some();
        if !has_materials && summon.special_summon_procedure.is_none() {
            errors.push(warn(ctx.name(), "Extra deck monster should declare materials or a special summon procedure"));
        }
    }
}

fn check_effect_blocks(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    for effect in &ctx.card.effects {
        // Effect must have resolve or choose
        if effect.resolve.is_empty() && effect.choose.is_none() {
            errors.push(err(ctx.name(), &format!(
                "Effect '{}' must have a resolve or choose block", effect.name
            )));
        }

        // Can't have both resolve and choose
        if !effect.resolve.is_empty() && effect.choose.is_some() {
            errors.push(err(ctx.name(), &format!(
                "Effect '{}' cannot have both resolve and choose blocks", effect.name
            )));
        }

        // Speed must be 1-3
        if let Some(speed) = effect.speed {
            if speed < 1 || speed > 3 {
                errors.push(err(ctx.name(), &format!(
                    "Effect '{}': speed must be 1, 2, or 3", effect.name
                )));
            }
        }

        // Cost with no resolve/choose is suspicious
        if !effect.cost.is_empty() && effect.resolve.is_empty() && effect.choose.is_none() {
            errors.push(warn(ctx.name(), &format!(
                "Effect '{}' has a cost but no resolve/choose", effect.name
            )));
        }
    }
}

fn check_spell_speeds(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    for effect in &ctx.card.effects {
        let speed = match effect.speed {
            Some(s) => s,
            None => continue,
        };

        // Counter Trap must be speed 3
        if ctx.is_counter_trap && speed != 3 {
            errors.push(err(ctx.name(), &format!(
                "Effect '{}': Counter Trap must use speed: 3", effect.name
            )));
        }

        // Traps must be speed 2+
        if ctx.is_trap && speed < 2 {
            errors.push(err(ctx.name(), &format!(
                "Effect '{}': Trap effects must be speed 2 or higher", effect.name
            )));
        }

        // Non-Quick-Play spells must be speed 1
        if ctx.is_spell && !ctx.is_quickplay && speed > 1 {
            errors.push(err(ctx.name(), &format!(
                "Effect '{}': Non-Quick-Play Spells must use speed: 1", effect.name
            )));
        }

        // Speed 3 only for Counter Traps
        if speed == 3 && !ctx.is_counter_trap {
            errors.push(err(ctx.name(), &format!(
                "Effect '{}': speed 3 is only for Counter Traps", effect.name
            )));
        }

        // Negate at speed 1 is suspicious — but only for card types where
        // speed 2+ is actually permitted. Non-Quick-Play Spells are locked
        // to speed 1 by the rule above, so a negate on them is a migration
        // signal (the real card is probably mis-typed) rather than a speed
        // bug we can fix here. Keep the warning off those to avoid the
        // contradictory "raise to 2+" / "must be 1" pairing.
        let can_be_quick = !ctx.is_spell || ctx.is_quickplay;
        if can_be_quick {
            for action in &effect.resolve {
                if let Action::Negate(_) = action {
                    if speed == 1 {
                        errors.push(warn(ctx.name(), &format!(
                            "Effect '{}': negate at speed 1 — should be speed 2+", effect.name
                        )));
                    }
                }
            }
        }

        // Optional trigger effects should declare timing
        if !effect.mandatory && effect.trigger.is_some() && speed == 1 && effect.timing.is_none() {
            errors.push(warn(ctx.name(), &format!(
                "Effect '{}': optional trigger should declare timing: when or if", effect.name
            )));
        }
    }
}

fn check_passive_blocks(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    for passive in &ctx.card.passives {
        let has_content = !passive.modifiers.is_empty()
            || !passive.grants.is_empty()
            || passive.negate_effects
            || passive.set_atk.is_some()
            || passive.set_def.is_some();

        if !has_content {
            errors.push(err(ctx.name(), &format!(
                "Passive '{}' has no modifiers, grants, or effects", passive.name
            )));
        }
    }
}

fn check_restriction_blocks(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    for restriction in &ctx.card.restrictions {
        if restriction.abilities.is_empty() {
            let name = restriction.name.as_deref().unwrap_or("unnamed");
            errors.push(err(ctx.name(), &format!(
                "Restriction '{}' declares no abilities", name
            )));
        }
    }
}

fn check_replacement_blocks(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    for replacement in &ctx.card.replacements {
        if replacement.actions.is_empty() {
            let name = replacement.name.as_deref().unwrap_or("unnamed");
            errors.push(warn(ctx.name(), &format!(
                "Replacement '{}' has no actions in do block", name
            )));
        }
    }
}

// T31 / CC-II — redirect block semantic checks.
//
// (a) `from` and `to` zones must be distinct — redirecting GY → GY is a
//     no-op and almost certainly a typo.
// (b) `scope: self` + `from: field` is nonsensical (the card is the
//     *source* of the redirect; "self moving off the field" is the event
//     the replacement block models, not this block). Warn.
// (c) CCC-II: `from:` and `to:` must be terminal card-location zones —
//     hand / deck / gy / banished / extra_deck. On-field placement zones
//     (monster_zone, spell_trap_zone, field, pendulum_zone, etc.) and
//     the overlay zone describe in-play state, not leave-field routing
//     endpoints. The engine consumer only honours the terminal set; any
//     other value stays latent and silently mismatches.
fn check_redirect_blocks(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    for redirect in &ctx.card.redirects {
        let name = redirect.name.as_deref().unwrap_or("unnamed");

        if redirect.from == redirect.to {
            errors.push(err(ctx.name(), &format!(
                "Redirect '{}' has identical from/to zones ({:?}) — no-op",
                name, redirect.from
            )));
        }

        if matches!(redirect.scope, RedirectScope::Self_)
            && matches!(redirect.from, Zone::Field)
        {
            errors.push(warn(ctx.name(), &format!(
                "Redirect '{}' uses `scope: self` with `from: field` — \
                 use a `replacement` block for per-card leave-field events",
                name
            )));
        }

        if !is_terminal_redirect_zone(&redirect.from) {
            errors.push(err(ctx.name(), &format!(
                "Redirect '{}' has non-terminal `from:` zone ({:?}) — \
                 must be one of hand/deck/gy/banished/extra_deck",
                name, redirect.from
            )));
        }
        if !is_terminal_redirect_zone(&redirect.to) {
            errors.push(err(ctx.name(), &format!(
                "Redirect '{}' has non-terminal `to:` zone ({:?}) — \
                 must be one of hand/deck/gy/banished/extra_deck",
                name, redirect.to
            )));
        }
    }
}

/// CCC-II: a redirect's `from:` and `to:` must describe terminal card
/// locations (where a card "lives" once routed), not on-field placement
/// zones or the overlay/equipped anchors. This keeps the grammar aligned
/// with the engine consumer which only understands these five targets.
fn is_terminal_redirect_zone(zone: &Zone) -> bool {
    matches!(
        zone,
        Zone::Hand | Zone::Deck | Zone::Gy | Zone::Banished
        | Zone::ExtraDeck | Zone::ExtraDeckFaceUp
    )
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::parser::parse_v2;

    #[test]
    fn test_pot_of_greed_valid() {
        let source = include_str!("../../cards/goat/pot_of_greed.ds");
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
    }

    #[test]
    fn test_lava_golem_valid() {
        let source = include_str!("../../cards/goat/lava_golem.ds");
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
    }

    #[test]
    fn test_mirror_force_valid() {
        let source = include_str!("../../cards/goat/mirror_force.ds");
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
    }

    #[test]
    fn test_sangan_valid() {
        let source = include_str!("../../cards/goat/sangan.ds");
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
    }

    #[test]
    fn test_solemn_judgment_valid() {
        let source = include_str!("../../cards/goat/solemn_judgment.ds");
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
    }

    #[test]
    fn test_missing_atk() {
        let file = File {
            cards: vec![Card {
                name: "Bad Monster".into(),
                fields: CardFields {
                    card_types: vec![CardType::EffectMonster],
                    attribute: Some(Attribute::Dark),
                    race: Some(Race::Fiend),
                    level: Some(4),
                    ..Default::default()
                },
                summon: None,
                effects: vec![],
                passives: vec![],
                restrictions: vec![],
                replacements: vec![],
                redirects: vec![],
            }],
        };
        let report = validate_v2(&file);
        assert!(report.has_errors());
        assert!(report.errors.iter().any(|e| e.message.contains("atk")));
    }

    #[test]
    fn test_monster_spell_conflict() {
        let file = File {
            cards: vec![Card {
                name: "Impossible Card".into(),
                fields: CardFields {
                    card_types: vec![CardType::EffectMonster, CardType::NormalSpell],
                    ..Default::default()
                },
                summon: None,
                effects: vec![],
                passives: vec![],
                restrictions: vec![],
                replacements: vec![],
                redirects: vec![],
            }],
        };
        let report = validate_v2(&file);
        assert!(report.errors.iter().any(|e| e.message.contains("Monster and Spell")));
    }

    #[test]
    fn test_counter_trap_speed() {
        let file = File {
            cards: vec![Card {
                name: "Bad Counter".into(),
                fields: CardFields {
                    card_types: vec![CardType::CounterTrap],
                    ..Default::default()
                },
                summon: None,
                effects: vec![Effect {
                    name: "Negate".into(),
                    speed: Some(2), // wrong — should be 3
                    frequency: None,
                    mandatory: false,
                    simultaneous: false,
                    timing: None,
                    trigger: None,
                    who: None,
                    condition: None,
                    activate_from: vec![],
                    damage_step: None,
                    target: None,
                    cost: vec![],
                    resolve: vec![Action::Negate(false)],
                    choose: None,
                }],
                passives: vec![],
                restrictions: vec![],
                replacements: vec![],
                redirects: vec![],
            }],
        };
        let report = validate_v2(&file);
        assert!(report.errors.iter().any(|e| e.message.contains("Counter Trap must use speed: 3")));
    }

    #[test]
    fn test_all_v2_cards_parse_and_validate() {
        let dir = std::fs::read_dir("cards/goat").unwrap();
        let mut count = 0;
        let mut failures = Vec::new();
        for entry in dir {
            let path = entry.unwrap().path();
            if path.extension().map_or(false, |e| e == "ds") {
                let path_str = path.to_string_lossy().to_string();
                let source = std::fs::read_to_string(&path).unwrap();
                match parse_v2(&source) {
                    Ok(file) => {
                        let report = validate_v2(&file);
                        if report.has_errors() {
                            let errs: Vec<_> = report.errors.iter()
                                .filter(|e| e.severity == Severity::Error)
                                .map(|e| e.to_string())
                                .collect();
                            failures.push(format!("{}: {}", path_str, errs.join("; ")));
                        }
                    }
                    Err(e) => {
                        failures.push(format!("{}: PARSE ERROR: {}", path_str, e));
                    }
                }
                count += 1;
            }
        }
        assert!(failures.is_empty(),
            "\n{} of {} cards failed:\n{}", failures.len(), count, failures.join("\n"));
        assert!(count >= 80, "Expected at least 80 v2 cards, found {}", count);
    }

    #[test]
    fn test_effect_needs_resolve() {
        let file = File {
            cards: vec![Card {
                name: "Empty Effect".into(),
                fields: CardFields {
                    card_types: vec![CardType::NormalSpell],
                    ..Default::default()
                },
                summon: None,
                effects: vec![Effect {
                    name: "Nothing".into(),
                    speed: Some(1),
                    frequency: None,
                    mandatory: false,
                    simultaneous: false,
                    timing: None,
                    trigger: None,
                    who: None,
                    condition: None,
                    activate_from: vec![],
                    damage_step: None,
                    target: None,
                    cost: vec![],
                    resolve: vec![], // empty!
                    choose: None,
                }],
                passives: vec![],
                restrictions: vec![],
                replacements: vec![],
                redirects: vec![],
            }],
        };
        let report = validate_v2(&file);
        assert!(report.errors.iter().any(|e| e.message.contains("resolve or choose")));
    }
}
