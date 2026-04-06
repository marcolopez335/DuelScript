// ============================================================
// DuelScript Parser v0.5 — parser.rs
// Converts pest parse tree into DuelScript AST nodes
// ============================================================

use pest::Parser;
use pest::iterators::Pair;
use pest_derive::Parser;
use crate::ast::*;

use std::fmt;

#[derive(Parser)]
#[grammar = "grammar/duelscript.pest"]
pub struct DuelScriptParser;

// ── Error Type ────────────────────────────────────────────────

#[derive(Debug)]
pub enum ParseError {
    PestError(String),
    MissingField(&'static str),
    InvalidValue(&'static str),
    UnknownCardType(String),
    UnknownAttribute(String),
    UnknownRace(String),
    UnknownZone(String),
    UnknownRule(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::PestError(e)        => write!(f, "Parse error: {}", e),
            ParseError::MissingField(field) => write!(f, "Missing field: {}", field),
            ParseError::InvalidValue(val)   => write!(f, "Invalid value: {}", val),
            ParseError::UnknownCardType(t)  => write!(f, "Unknown card type: {}", t),
            ParseError::UnknownAttribute(a) => write!(f, "Unknown attribute: {}", a),
            ParseError::UnknownRace(r)      => write!(f, "Unknown race: {}", r),
            ParseError::UnknownZone(z)      => write!(f, "Unknown zone: {}", z),
            ParseError::UnknownRule(r)      => write!(f, "Unknown rule: {}", r),
        }
    }
}

impl std::error::Error for ParseError {}

// ── Entry Point ───────────────────────────────────────────────

pub fn parse(source: &str) -> Result<DuelScriptFile, ParseError> {
    let pairs = DuelScriptParser::parse(Rule::file, source)
        .map_err(|e| ParseError::PestError(e.to_string()))?;

    let mut cards = Vec::new();
    for pair in pairs {
        // The top-level `file` rule contains card+ — iterate into it
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::card {
                cards.push(parse_card(inner)?);
            }
        }
    }
    Ok(DuelScriptFile { cards })
}

// ── Card ──────────────────────────────────────────────────────

fn parse_card(pair: Pair<Rule>) -> Result<Card, ParseError> {
    let mut inner = pair.into_inner();
    let name = parse_string(inner.next().ok_or(ParseError::MissingField("card name"))?);

    let mut card = Card {
        name,
        card_types: vec![],
        attribute: None,
        stats: Stats::default(),
        race: None,
        level: None,
        rank: None,
        link: None,
        scale: None,
        flavor: None,
        password: None,
        archetypes: vec![],
        link_arrows: vec![],
        summon_conditions: vec![],
        materials: None,
        counter_system: None,
        pendulum_effect: None,
        effects: vec![],
        continuous_effects: vec![],
        replacement_effects: vec![],
        equip_effects: vec![],
        win_condition: None,
        raw_effects: vec![],
    };

    let body = inner.next().ok_or(ParseError::MissingField("card body"))?;
    for item in body.into_inner() {
        // card_body contains card_body_item* — unwrap the item wrapper
        let field = if item.as_rule() == Rule::card_body_item {
            item.into_inner().next().ok_or(ParseError::MissingField("card body item"))?
        } else {
            item
        };
        match field.as_rule() {
            Rule::card_field             => parse_card_field(field, &mut card)?,
            Rule::archetype_decl         => card.archetypes = parse_archetype_decl(field)?,
            Rule::summon_condition_block  => card.summon_conditions = parse_summon_condition_block(field)?,
            Rule::materials_block        => card.materials = Some(parse_materials_block(field)?),
            Rule::counter_system_block   => card.counter_system = Some(parse_counter_system(field)?),
            Rule::link_arrows_decl       => card.link_arrows = parse_link_arrows(field)?,
            Rule::pendulum_block         => {
                let body_pair = field.into_inner().next().ok_or(ParseError::MissingField("pendulum body"))?;
                card.pendulum_effect = Some(parse_effect_body(body_pair)?);
            }
            Rule::raw_effect_block       => card.raw_effects.push(parse_raw_effect_block(field)?),
            Rule::effect_block           => card.effects.push(parse_effect_block(field)?),
            Rule::continuous_effect_block => card.continuous_effects.push(parse_continuous_effect(field)?),
            Rule::replacement_effect_block => card.replacement_effects.push(parse_replacement_effect(field)?),
            Rule::equip_effect_block     => card.equip_effects.push(parse_equip_effect(field)?),
            Rule::win_condition_block    => card.win_condition = Some(parse_win_condition(field)?),
            _ => {}
        }
    }
    Ok(card)
}

fn parse_card_field(pair: Pair<Rule>, card: &mut Card) -> Result<(), ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("card field"))?;
    match inner.as_rule() {
        Rule::type_decl => {
            card.card_types = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::card_type)
                .map(parse_card_type)
                .collect::<Result<Vec<_>, _>>()?;
        }
        Rule::attribute_decl => {
            let attr = inner.into_inner()
                .find(|p| p.as_rule() == Rule::attribute)
                .ok_or(ParseError::MissingField("attribute"))?;
            card.attribute = Some(parse_attribute(attr)?);
        }
        Rule::stat_decl => {
            let text = inner.as_str();
            let mut parts = inner.into_inner();
            // The stat name is embedded in the rule text
            let val = parts.next().map(parse_stat_value).transpose()?;
            if text.starts_with("atk") {
                card.stats.atk = val;
            } else {
                card.stats.def = val;
            }
        }
        Rule::race_decl => {
            let race = inner.into_inner()
                .find(|p| p.as_rule() == Rule::race)
                .ok_or(ParseError::MissingField("race"))?;
            card.race = Some(parse_race(race)?);
        }
        Rule::level_decl => {
            card.level = Some(parse_unsigned(inner.into_inner().next()
                .ok_or(ParseError::MissingField("level"))?)?);
        }
        Rule::rank_decl => {
            card.rank = Some(parse_unsigned(inner.into_inner().next()
                .ok_or(ParseError::MissingField("rank"))?)?);
        }
        Rule::link_decl => {
            card.link = Some(parse_unsigned(inner.into_inner().next()
                .ok_or(ParseError::MissingField("link"))?)?);
        }
        Rule::scale_decl => {
            card.scale = Some(parse_unsigned(inner.into_inner().next()
                .ok_or(ParseError::MissingField("scale"))?)?);
        }
        Rule::flavor_decl => {
            card.flavor = inner.into_inner().next().map(parse_string);
        }
        Rule::password_decl => {
            card.password = Some(parse_unsigned(inner.into_inner().next()
                .ok_or(ParseError::MissingField("password"))?)?);
        }
        _ => {}
    }
    Ok(())
}

// ─�� Archetype ────────────────────────────────���────────────────

fn parse_archetype_decl(pair: Pair<Rule>) -> Result<Vec<String>, ParseError> {
    Ok(pair.into_inner()
        .filter(|p| p.as_rule() == Rule::string)
        .map(parse_string)
        .collect())
}

// ── Summon Conditions ─────────────────────────────────────────

fn parse_summon_condition_block(pair: Pair<Rule>) -> Result<Vec<SummonRule>, ParseError> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::summon_rule)
        .map(parse_summon_rule)
        .collect()
}

fn parse_summon_rule(pair: Pair<Rule>) -> Result<SummonRule, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("summon rule"))?;
    match inner.as_rule() {
        Rule::tributes_required_rule => {
            let n = parse_unsigned(inner.into_inner().next()
                .ok_or(ParseError::MissingField("tribute count"))?)?;
            Ok(SummonRule::TributesRequired(n))
        }
        Rule::cannot_normal_summon_rule => Ok(SummonRule::CannotNormalSummon),
        Rule::cannot_special_summon_rule => Ok(SummonRule::CannotSpecialSummon),
        Rule::special_summon_only_rule => Ok(SummonRule::SpecialSummonOnly),
        Rule::summon_once_per_turn_rule => Ok(SummonRule::SummonOncePerTurn),
        Rule::must_be_summoned_by_rule => {
            let source = inner.into_inner().next()
                .ok_or(ParseError::MissingField("summon source"))?;
            Ok(SummonRule::MustBeSummonedBy(parse_summon_source(source)?))
        }
        Rule::tribute_material_rule => {
            let filter = inner.into_inner().next()
                .ok_or(ParseError::MissingField("tribute material filter"))?;
            Ok(SummonRule::TributeMaterial(parse_card_filter(filter)?))
        }
        Rule::special_summon_from_rule => {
            let zones = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::zone)
                .map(parse_zone)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SummonRule::SpecialSummonFrom(zones))
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_summon_source(pair: Pair<Rule>) -> Result<SummonSource, ParseError> {
    let text = pair.as_str();
    match text {
        "own_effect"   => Ok(SummonSource::OwnEffect),
        "ritual_spell" => Ok(SummonSource::RitualSpell),
        "fusion_spell" => Ok(SummonSource::FusionSpell),
        _ => {
            // Check for "specific_card" pattern or summon_method
            let mut inner = pair.into_inner();
            if let Some(child) = inner.next() {
                if child.as_rule() == Rule::string {
                    return Ok(SummonSource::SpecificCard(parse_string(child)));
                }
                if child.as_rule() == Rule::summon_method {
                    return Ok(SummonSource::Method(parse_summon_method(child)?));
                }
            }
            Err(ParseError::UnknownRule(text.to_string()))
        }
    }
}

// ── Materials ─────────────────────────────────────────────────

fn parse_materials_block(pair: Pair<Rule>) -> Result<MaterialsBlock, ParseError> {
    let mut block = MaterialsBlock {
        slots: vec![],
        constraints: vec![],
        alternatives: vec![],
    };
    let body = pair.into_inner().next().ok_or(ParseError::MissingField("materials body"))?;
    for child in body.into_inner() {
        match child.as_rule() {
            Rule::material_slot         => block.slots.push(parse_material_slot(child)?),
            Rule::material_constraint   => block.constraints.push(parse_material_constraint(child)?),
            Rule::alternative_materials => block.alternatives.push(parse_alternative_materials(child)?),
            _ => {}
        }
    }
    Ok(block)
}

fn parse_material_slot(pair: Pair<Rule>) -> Result<MaterialSlot, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("material slot inner"))?;
    match inner.as_rule() {
        Rule::named_material_slot => {
            let names = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::string)
                .map(parse_string)
                .collect();
            Ok(MaterialSlot::Named(names))
        }
        Rule::generic_material_slot => {
            let mut count = 1u32;
            let mut count_or_more = false;
            let mut qualifiers = vec![];
            let mut attribute = None;
            let mut race = None;
            let mut level = None;
            let mut extra_deck_type = None;
            let mut filter = CardFilter::Monster;

            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::material_count => {
                        let text = child.as_str();
                        count_or_more = text.ends_with('+');
                        let num_str = text.trim_end_matches('+');
                        count = num_str.parse().unwrap_or(1);
                    }
                    Rule::material_qualifier => {
                        qualifiers.push(parse_material_qualifier(child)?);
                    }
                    Rule::attribute => attribute = Some(parse_attribute(child)?),
                    Rule::race => race = Some(parse_race(child)?),
                    Rule::level_constraint => level = Some(parse_level_constraint(child)?),
                    Rule::extra_deck_type => {
                        extra_deck_type = Some(match child.as_str() {
                            "synchro" => ExtraDeckType::Synchro,
                            "fusion"  => ExtraDeckType::Fusion,
                            "xyz"     => ExtraDeckType::Xyz,
                            "link"    => ExtraDeckType::Link,
                            "ritual"  => ExtraDeckType::Ritual,
                            _         => ExtraDeckType::Fusion,
                        });
                    }
                    Rule::card_filter => filter = parse_card_filter(child)?,
                    _ => {}
                }
            }
            Ok(MaterialSlot::Generic(GenericMaterialSlot {
                count, count_or_more, qualifiers, attribute, race, level, extra_deck_type, filter,
            }))
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_material_qualifier(pair: Pair<Rule>) -> Result<MaterialQualifier, ParseError> {
    match pair.as_str() {
        "tuner"      => Ok(MaterialQualifier::Tuner),
        "non-tuner"  => Ok(MaterialQualifier::NonTuner),
        "non-token"  => Ok(MaterialQualifier::NonToken),
        "non-special" => Ok(MaterialQualifier::NonSpecial),
        "non-fusion" => Ok(MaterialQualifier::NonFusion),
        "non-synchro" => Ok(MaterialQualifier::NonSynchro),
        "non-xyz"    => Ok(MaterialQualifier::NonXyz),
        "non-link"   => Ok(MaterialQualifier::NonLink),
        other        => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_level_constraint(pair: Pair<Rule>) -> Result<LevelConstraint, ParseError> {
    let text = pair.as_str();
    let nums: Vec<u32> = pair.into_inner()
        .filter(|p| p.as_rule() == Rule::unsigned)
        .map(|p| p.as_str().parse().unwrap_or(0))
        .collect();

    if text.starts_with("min_level") {
        Ok(LevelConstraint::Min(nums[0]))
    } else if text.starts_with("max_level") {
        Ok(LevelConstraint::Max(nums[0]))
    } else if nums.len() == 2 {
        Ok(LevelConstraint::Range(nums[0], nums[1]))
    } else {
        Ok(LevelConstraint::Exact(nums[0]))
    }
}

fn parse_material_constraint(pair: Pair<Rule>) -> Result<MaterialConstraint, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("material constraint"))?;
    match inner.as_rule() {
        Rule::same_level_constraint     => Ok(MaterialConstraint::SameLevel),
        Rule::same_attribute_constraint => Ok(MaterialConstraint::SameAttribute),
        Rule::same_race_constraint      => Ok(MaterialConstraint::SameRace),
        Rule::must_include_constraint   => {
            let name = inner.into_inner().find(|p| p.as_rule() == Rule::string)
                .map(parse_string).unwrap_or_default();
            Ok(MaterialConstraint::MustInclude(name))
        }
        Rule::cannot_use_constraint => {
            let target = inner.into_inner().next()
                .ok_or(ParseError::MissingField("cannot use target"))?;
            let mct = match target.as_str() {
                "token"    => MaterialCannotTarget::Token,
                "fusion"   => MaterialCannotTarget::Fusion,
                "synchro"  => MaterialCannotTarget::Synchro,
                "xyz"      => MaterialCannotTarget::Xyz,
                "link"     => MaterialCannotTarget::Link,
                "pendulum" => MaterialCannotTarget::Pendulum,
                other      => MaterialCannotTarget::Named(other.trim_matches('"').to_string()),
            };
            Ok(MaterialConstraint::CannotUse(mct))
        }
        Rule::summon_method_constraint => {
            let method = inner.into_inner().next()
                .ok_or(ParseError::MissingField("summon method type"))?;
            let smt = match method.as_str() {
                "fusion"  => SummonMethodType::Fusion,
                "synchro" => SummonMethodType::Synchro,
                "xyz"     => SummonMethodType::Xyz,
                "link"    => SummonMethodType::Link,
                "ritual"  => SummonMethodType::Ritual,
                _         => SummonMethodType::Fusion,
            };
            Ok(MaterialConstraint::Method(smt))
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_alternative_materials(pair: Pair<Rule>) -> Result<AlternativeMaterials, ParseError> {
    let mut alt = AlternativeMaterials { slots: vec![], constraints: vec![] };
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::material_slot       => alt.slots.push(parse_material_slot(child)?),
            Rule::material_constraint => alt.constraints.push(parse_material_constraint(child)?),
            _ => {}
        }
    }
    Ok(alt)
}

// ── Counter System ────────────────────────────────────────────

fn parse_counter_system(pair: Pair<Rule>) -> Result<CounterSystem, ParseError> {
    let mut name = String::new();
    let mut placed_when = None;
    let mut max = None;
    let mut effects = vec![];

    let body = pair.into_inner().next().ok_or(ParseError::MissingField("counter body"))?;
    for child in body.into_inner() {
        match child.as_rule() {
            Rule::counter_name_decl => {
                name = child.into_inner().find(|p| p.as_rule() == Rule::string)
                    .map(parse_string).unwrap_or_default();
            }
            Rule::counter_placed_when => {
                if let Some(t) = child.into_inner().find(|p| p.as_rule() == Rule::trigger_expr) {
                    placed_when = Some(parse_trigger_expr(t)?);
                }
            }
            Rule::counter_max_decl => {
                let val_str = child.into_inner().next().map(|p| p.as_str().to_string()).unwrap_or_default();
                max = Some(if val_str == "none" {
                    CounterMax::Unlimited
                } else {
                    CounterMax::Limited(val_str.parse().unwrap_or(0))
                });
            }
            Rule::effect_block => effects.push(parse_effect_block(child)?),
            _ => {}
        }
    }
    Ok(CounterSystem { name, placed_when, max, effects })
}

// ── Link Arrows ───────────────────────────────────────────────

fn parse_link_arrows(pair: Pair<Rule>) -> Result<Vec<LinkArrow>, ParseError> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::link_arrow)
        .map(|p| match p.as_str() {
            "top_left"     => Ok(LinkArrow::TopLeft),
            "top"          => Ok(LinkArrow::Top),
            "top_right"    => Ok(LinkArrow::TopRight),
            "left"         => Ok(LinkArrow::Left),
            "right"        => Ok(LinkArrow::Right),
            "bottom_left"  => Ok(LinkArrow::BottomLeft),
            "bottom"       => Ok(LinkArrow::Bottom),
            "bottom_right" => Ok(LinkArrow::BottomRight),
            other          => Err(ParseError::UnknownRule(other.to_string())),
        })
        .collect()
}

// ── Effect Block ──────────────────────────────────────────────

/// v0.6: Parse a raw_effect block that declares explicit engine bitfields.
/// Used by the transpiler to preserve exact Lua metadata.
fn parse_raw_effect_block(pair: Pair<Rule>) -> Result<RawEffect, ParseError> {
    let mut raw = RawEffect {
        name: None,
        effect_type: 0,
        category: 0,
        code: 0,
        property: 0,
        range: 0,
        count_limit: None,
        cost: vec![],
        on_activate: vec![],
        on_resolve: vec![],
    };

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::string => raw.name = Some(parse_string(child)),
            Rule::raw_effect_body => {
                for inner in child.into_inner() {
                    match inner.as_rule() {
                        Rule::raw_field => {
                            let text = inner.as_str();
                            let nums: Vec<u32> = inner.into_inner()
                                .filter(|p| p.as_rule() == Rule::unsigned)
                                .map(|p| p.as_str().parse().unwrap_or(0))
                                .collect();

                            if text.starts_with("effect_type") { raw.effect_type = nums[0]; }
                            else if text.starts_with("category") { raw.category = nums[0]; }
                            else if text.starts_with("code") { raw.code = nums[0]; }
                            else if text.starts_with("property") { raw.property = nums[0]; }
                            else if text.starts_with("range") { raw.range = nums[0]; }
                            else if text.starts_with("count_limit") && nums.len() >= 2 {
                                raw.count_limit = Some((nums[0], nums[1]));
                            }
                        }
                        Rule::cost_clause => {
                            for action in inner.into_inner() {
                                if action.as_rule() == Rule::cost_action {
                                    raw.cost.push(parse_cost_action(action)?);
                                }
                            }
                        }
                        Rule::on_activate_clause => {
                            for action in inner.into_inner() {
                                if action.as_rule() == Rule::game_action {
                                    raw.on_activate.push(parse_game_action(action)?);
                                }
                            }
                        }
                        Rule::on_resolve_clause => {
                            for action in inner.into_inner() {
                                if action.as_rule() == Rule::game_action {
                                    raw.on_resolve.push(parse_game_action(action)?);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(raw)
}

fn parse_effect_block(pair: Pair<Rule>) -> Result<Effect, ParseError> {
    let mut name = None;
    let mut body = EffectBody::default();

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::string      => name = Some(parse_string(child)),
            Rule::effect_body => body = parse_effect_body(child)?,
            _ => {}
        }
    }
    Ok(Effect { name, body })
}

fn parse_effect_body(pair: Pair<Rule>) -> Result<EffectBody, ParseError> {
    let mut body = EffectBody::default();
    for item in pair.into_inner() {
        // Unwrap effect_body_clause wrapper if present
        let clause = if item.as_rule() == Rule::effect_body_clause {
            item.into_inner().next().ok_or(ParseError::MissingField("effect body clause"))?
        } else {
            item
        };
        match clause.as_rule() {
            Rule::speed_decl => {
                if let Some(sp) = clause.into_inner().find(|p| p.as_rule() == Rule::spell_speed) {
                    body.speed = parse_spell_speed(sp)?;
                }
            }
            Rule::frequency_decl => body.frequency = parse_frequency(clause)?,
            Rule::optional_decl => {
                body.optional = clause.into_inner().next()
                    .map(|p| p.as_str() == "true").unwrap_or(false);
            }
            Rule::activate_from_clause => {
                body.activate_from = clause.into_inner()
                    .filter(|p| p.as_rule() == Rule::zone)
                    .map(parse_zone)
                    .collect::<Result<Vec<_>, _>>()?;
            }
            Rule::damage_step_decl => {
                body.damage_step = clause.into_inner().next()
                    .map(|p| p.as_str() == "true").unwrap_or(false);
            }
            Rule::timing_qualifier_decl => {
                if let Some(tq) = clause.into_inner().find(|p| p.as_rule() == Rule::timing_qualifier) {
                    body.timing = match tq.as_str() {
                        "if" => TimingQualifier::If,
                        _    => TimingQualifier::When,
                    };
                    body.timing_explicit = true;
                }
            }
            Rule::condition_clause => {
                if let Some(c) = clause.into_inner().find(|p| p.as_rule() == Rule::condition_expr) {
                    body.condition = Some(parse_condition_expr(c)?);
                }
            }
            Rule::trigger_clause => {
                if let Some(t) = clause.into_inner().find(|p| p.as_rule() == Rule::trigger_expr) {
                    body.trigger = Some(parse_trigger_expr(t)?);
                }
            }
            Rule::cost_clause => {
                for action in clause.into_inner() {
                    if action.as_rule() == Rule::cost_action {
                        body.cost.push(parse_cost_action(action)?);
                    }
                }
            }
            Rule::on_activate_clause => {
                for action in clause.into_inner() {
                    if action.as_rule() == Rule::game_action {
                        body.on_activate.push(parse_game_action(action)?);
                    }
                }
            }
            Rule::on_resolve_clause => {
                for action in clause.into_inner() {
                    if action.as_rule() == Rule::game_action {
                        body.on_resolve.push(parse_game_action(action)?);
                    }
                }
            }
            Rule::restriction_block => {
                body.restrictions.extend(parse_restriction_block(clause)?);
            }
            _ => {}
        }
    }
    Ok(body)
}

// ── Continuous Effect ─────────────────────────────────────────

fn parse_continuous_effect(pair: Pair<Rule>) -> Result<ContinuousEffect, ParseError> {
    let mut name = None;
    let mut while_cond = None;
    let mut apply_to = None;
    let mut modifiers = vec![];
    let mut restrictions = vec![];
    let mut cannots = vec![];

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::string => name = Some(parse_string(child)),
            Rule::continuous_body => {
                for inner in child.into_inner() {
                    match inner.as_rule() {
                        Rule::while_clause => {
                            if let Some(c) = inner.into_inner().find(|p| p.as_rule() == Rule::condition_expr) {
                                while_cond = Some(parse_condition_expr(c)?);
                            }
                        }
                        Rule::apply_to_clause => {
                            if let Some(t) = inner.into_inner().find(|p| p.as_rule() == Rule::target_expr) {
                                apply_to = Some(parse_target_expr(t)?);
                            }
                        }
                        Rule::modifier_list => {
                            for m in inner.into_inner() {
                                if m.as_rule() == Rule::modifier_decl {
                                    modifiers.push(parse_modifier_decl(m)?);
                                }
                            }
                        }
                        Rule::restriction_block => {
                            restrictions.extend(parse_restriction_block(inner)?);
                        }
                        Rule::cannot_block => {
                            cannots.push(parse_cannot_block(inner)?);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(ContinuousEffect { name, while_cond, apply_to, modifiers, restrictions, cannots })
}

fn parse_modifier_decl(pair: Pair<Rule>) -> Result<ModifierDecl, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("modifier decl"))?;
    match inner.as_rule() {
        Rule::atk_modifier_decl => {
            let mut sign = Sign::Plus;
            let mut value = Expr::lit(0);
            let mut duration = None;
            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::plus_minus => sign = if child.as_str() == "+" { Sign::Plus } else { Sign::Minus },
                    Rule::expr       => value = parse_expr(child)?,
                    Rule::duration   => duration = Some(parse_duration(child)?),
                    _ => {}
                }
            }
            Ok(ModifierDecl::Atk { sign, value, duration })
        }
        Rule::def_modifier_decl => {
            let mut sign = Sign::Plus;
            let mut value = Expr::lit(0);
            let mut duration = None;
            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::plus_minus => sign = if child.as_str() == "+" { Sign::Plus } else { Sign::Minus },
                    Rule::expr       => value = parse_expr(child)?,
                    Rule::duration   => duration = Some(parse_duration(child)?),
                    _ => {}
                }
            }
            Ok(ModifierDecl::Def { sign, value, duration })
        }
        Rule::level_modifier_decl => {
            let mut sign = Sign::Plus;
            let mut value = Expr::lit(0);
            let mut duration = None;
            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::plus_minus => sign = if child.as_str() == "+" { Sign::Plus } else { Sign::Minus },
                    Rule::expr       => value = parse_expr(child)?,
                    Rule::duration   => duration = Some(parse_duration(child)?),
                    _ => {}
                }
            }
            Ok(ModifierDecl::Level { sign, value, duration })
        }
        Rule::flag_modifier_decl => {
            let ability = inner.into_inner().next()
                .ok_or(ParseError::MissingField("granted ability"))?;
            Ok(ModifierDecl::Grant(parse_granted_ability(ability)?))
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_granted_ability(pair: Pair<Rule>) -> Result<GrantedAbility, ParseError> {
    match pair.as_str() {
        "piercing"                       => Ok(GrantedAbility::Piercing),
        "double_attack"                  => Ok(GrantedAbility::DoubleAttack),
        "direct_attack"                  => Ok(GrantedAbility::DirectAttack),
        "cannot_be_destroyed_by_battle"  => Ok(GrantedAbility::CannotBeDestroyedByBattle),
        "cannot_be_destroyed_by_effect"  => Ok(GrantedAbility::CannotBeDestroyedByEffect),
        "unaffected_by_spell_effects"    => Ok(GrantedAbility::UnaffectedBySpellEffects),
        "unaffected_by_trap_effects"     => Ok(GrantedAbility::UnaffectedByTrapEffects),
        "unaffected_by_monster_effects"  => Ok(GrantedAbility::UnaffectedByMonsterEffects),
        "unaffected_by_card_effects"     => Ok(GrantedAbility::UnaffectedByCardEffects),
        "immune_to_targeting"            => Ok(GrantedAbility::ImmuneToTargeting),
        "cannot_activate_effects"        => Ok(GrantedAbility::CannotActivateEffects),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

// ── Replacement Effect ────────────────────────────────────────

fn parse_replacement_effect(pair: Pair<Rule>) -> Result<ReplacementEffect, ParseError> {
    let mut name = None;
    let mut instead_of = ReplaceableEvent::DestroyedByAny;
    let mut do_actions = vec![];

    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::string => name = Some(parse_string(child)),
            Rule::replacement_body => {
                for inner in child.into_inner() {
                    match inner.as_rule() {
                        Rule::instead_of_clause => {
                            if let Some(event) = inner.into_inner().find(|p| p.as_rule() == Rule::replaceable_event) {
                                instead_of = parse_replaceable_event(event)?;
                            }
                        }
                        Rule::do_clause => {
                            for action in inner.into_inner() {
                                if action.as_rule() == Rule::game_action {
                                    do_actions.push(parse_game_action(action)?);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(ReplacementEffect { name, instead_of, do_actions })
}

fn parse_replaceable_event(pair: Pair<Rule>) -> Result<ReplaceableEvent, ParseError> {
    match pair.as_str() {
        "destroyed_by_battle"  => Ok(ReplaceableEvent::DestroyedByBattle),
        "destroyed_by_effect"  => Ok(ReplaceableEvent::DestroyedByEffect),
        "destroyed_by_any"     => Ok(ReplaceableEvent::DestroyedByAny),
        "sent_to_gy_by_effect" => Ok(ReplaceableEvent::SentToGyByEffect),
        "sent_to_gy_by_battle" => Ok(ReplaceableEvent::SentToGyByBattle),
        "sent_to_gy"           => Ok(ReplaceableEvent::SentToGy),
        "banished"             => Ok(ReplaceableEvent::Banished),
        "returned_to_hand"     => Ok(ReplaceableEvent::ReturnedToHand),
        "returned_to_deck"     => Ok(ReplaceableEvent::ReturnedToDeck),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

// ── Equip Effect ──────────────────────────────────────────────

fn parse_equip_effect(pair: Pair<Rule>) -> Result<EquipEffect, ParseError> {
    let mut target = TargetExpr::SelfCard;
    let mut while_equipped = vec![];
    let mut on_equipped_destroyed = vec![];
    let mut on_unequipped = vec![];

    let body = pair.into_inner().next().ok_or(ParseError::MissingField("equip body"))?;
    for child in body.into_inner() {
        match child.as_rule() {
            Rule::equip_target_clause => {
                if let Some(t) = child.into_inner().find(|p| p.as_rule() == Rule::target_expr) {
                    target = parse_target_expr(t)?;
                }
            }
            Rule::while_equipped_block => {
                for inner in child.into_inner() {
                    match inner.as_rule() {
                        Rule::modifier_decl => {
                            while_equipped.push(WhileEquippedClause::Modifier(parse_modifier_decl(inner)?));
                        }
                        Rule::cannot_block => {
                            while_equipped.push(WhileEquippedClause::Cannot(parse_cannot_block(inner)?));
                        }
                        _ => {}
                    }
                }
            }
            Rule::on_equipped_destroyed_block => {
                for inner in child.into_inner() {
                    if inner.as_rule() == Rule::game_action {
                        on_equipped_destroyed.push(parse_game_action(inner)?);
                    }
                }
            }
            Rule::on_unequipped_block => {
                for inner in child.into_inner() {
                    if inner.as_rule() == Rule::game_action {
                        on_unequipped.push(parse_game_action(inner)?);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(EquipEffect { target, while_equipped, on_equipped_destroyed, on_unequipped })
}

// ── Win Condition ─────────────────────────────────────────────

fn parse_win_condition(pair: Pair<Rule>) -> Result<WinCondition, ParseError> {
    let mut trigger = WinTrigger::AllPiecesInHand;
    let mut result = WinResult::WinDuel;

    let body = pair.into_inner().next().ok_or(ParseError::MissingField("win body"))?;
    for child in body.into_inner() {
        match child.as_rule() {
            Rule::win_trigger_clause => {
                if let Some(wt) = child.into_inner().find(|p| p.as_rule() == Rule::win_trigger) {
                    trigger = parse_win_trigger(wt)?;
                }
            }
            Rule::win_action => {
                let text = child.into_inner().next().map(|p| p.as_str()).unwrap_or("win_duel");
                result = match text {
                    "lose_duel" => WinResult::LoseDuel,
                    "draw_duel" => WinResult::DrawDuel,
                    _           => WinResult::WinDuel,
                };
            }
            _ => {}
        }
    }
    Ok(WinCondition { trigger, result })
}

fn parse_win_trigger(pair: Pair<Rule>) -> Result<WinTrigger, ParseError> {
    let text = pair.as_str();
    if text.starts_with("all_pieces_in_hand") {
        Ok(WinTrigger::AllPiecesInHand)
    } else if text.starts_with("opponent_cannot_draw") {
        Ok(WinTrigger::OpponentCannotDraw)
    } else if text.starts_with("turn_count") {
        let n = pair.into_inner().find(|p| p.as_rule() == Rule::unsigned)
            .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
        Ok(WinTrigger::TurnCount(n))
    } else if text.starts_with("specific_cards_on_field") {
        let cards = pair.into_inner()
            .filter(|p| p.as_rule() == Rule::string)
            .map(parse_string)
            .collect();
        Ok(WinTrigger::SpecificCardsOnField(cards))
    } else {
        Err(ParseError::UnknownRule(text.to_string()))
    }
}

// ─�� Frequency ─────────────────────────────────────────────────

fn parse_frequency(pair: Pair<Rule>) -> Result<Frequency, ParseError> {
    let text = pair.as_str();
    if text.starts_with("once_per_turn") {
        let opt = pair.into_inner()
            .find(|p| p.as_rule() == Rule::opt_kind)
            .map(|p| match p.as_str() {
                "soft" => OptKind::Soft,
                _      => OptKind::Hard,
            })
            .unwrap_or(OptKind::Hard);
        Ok(Frequency::OncePerTurn(opt))
    } else if text.starts_with("twice_per_turn") {
        Ok(Frequency::TwicePerTurn)
    } else if text.starts_with("once_per_duel") {
        Ok(Frequency::OncePerDuel)
    } else if text.starts_with("each_turn") {
        Ok(Frequency::EachTurn)
    } else {
        Ok(Frequency::Unlimited)
    }
}

// ── Expression ────────────────────────────────────────────────

fn parse_expr(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    let mut children: Vec<Pair<Rule>> = pair.into_inner().collect();

    if children.is_empty() {
        return Ok(Expr::lit(0));
    }

    // Parse first atom
    let first = children.remove(0);
    let mut result = parse_expr_atom(first)?;

    // Parse (op atom) pairs
    while children.len() >= 2 {
        let op_pair = children.remove(0);
        let atom_pair = children.remove(0);
        let op = match op_pair.as_str() {
            "+" => BinOp::Add,
            "-" => BinOp::Sub,
            "*" => BinOp::Mul,
            "/" => BinOp::Div,
            _   => BinOp::Add,
        };
        let right = parse_expr_atom(atom_pair)?;
        result = Expr::BinOp {
            left: Box::new(result),
            op,
            right: Box::new(right),
        };
    }

    Ok(result)
}

fn parse_expr_atom(pair: Pair<Rule>) -> Result<Expr, ParseError> {
    match pair.as_rule() {
        Rule::expr_atom => {
            let inner = pair.into_inner().next().ok_or(ParseError::MissingField("expr atom"))?;
            parse_expr_atom(inner)
        }
        Rule::expr => parse_expr(pair),
        Rule::unsigned => {
            let n: i32 = pair.as_str().parse().map_err(|_| ParseError::InvalidValue("unsigned"))?;
            Ok(Expr::lit(n))
        }
        Rule::count_expr => {
            let mut target = TargetExpr::SelfCard;
            let mut zone = None;
            for child in pair.into_inner() {
                match child.as_rule() {
                    Rule::target_expr => target = parse_target_expr(child)?,
                    Rule::zone        => zone = Some(parse_zone(child)?),
                    _ => {}
                }
            }
            Ok(Expr::Count { target: Box::new(target), zone })
        }
        Rule::self_stat_expr => {
            let text = pair.as_str();
            let stat = if text.ends_with("atk") { Stat::Atk }
                else if text.ends_with("def") { Stat::Def }
                else if text.ends_with("level") { Stat::Level }
                else { Stat::Rank };
            Ok(Expr::SelfStat(stat))
        }
        Rule::target_stat_expr => {
            let text = pair.as_str();
            let stat = if text.ends_with("atk") { Stat::Atk }
                else if text.ends_with("def") { Stat::Def }
                else { Stat::Level };
            Ok(Expr::TargetStat(stat))
        }
        Rule::player_lp_expr => {
            let player = if pair.as_str() == "your_lp" { Player::You } else { Player::Opponent };
            Ok(Expr::PlayerLp(player))
        }
        _ => Err(ParseError::UnknownRule(pair.as_str().to_string())),
    }
}

// ── Cost Actions ──────────────────────────────────────────────

fn parse_cost_action(pair: Pair<Rule>) -> Result<CostAction, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("cost action"))?;
    match inner.as_rule() {
        Rule::pay_lp_cost => {
            let expr = inner.into_inner().find(|p| p.as_rule() == Rule::expr)
                .ok_or(ParseError::MissingField("lp amount"))?;
            Ok(CostAction::PayLp(parse_expr(expr)?))
        }
        Rule::discard_cost => {
            Ok(CostAction::Discard(parse_self_or_target(inner)?))
        }
        Rule::tribute_cost => {
            Ok(CostAction::Tribute(parse_self_or_target(inner)?))
        }
        Rule::banish_cost => {
            let target = parse_self_or_target_first(inner.clone())?;
            let zone = inner.into_inner().find(|p| p.as_rule() == Rule::zone)
                .map(parse_zone).transpose()?;
            Ok(CostAction::Banish { target, from: zone })
        }
        Rule::send_cost => {
            let target = parse_self_or_target_first(inner.clone())?;
            let zone = inner.into_inner().find(|p| p.as_rule() == Rule::zone)
                .ok_or(ParseError::MissingField("send zone"))
                .and_then(parse_zone)?;
            Ok(CostAction::Send { target, to: zone })
        }
        Rule::remove_counter_cost => {
            let mut parts = inner.into_inner();
            let count = parse_unsigned(parts.next().ok_or(ParseError::MissingField("counter count"))?)?;
            let name = parts.find(|p| p.as_rule() == Rule::string).map(parse_string).unwrap_or_default();
            Ok(CostAction::RemoveCounter { count, name, from: SelfOrTarget::Self_ })
        }
        Rule::detach_cost => {
            let count = inner.into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(1)).unwrap_or(1);
            Ok(CostAction::Detach { count, from: SelfOrTarget::Self_ })
        }
        Rule::reveal_cost => {
            Ok(CostAction::Reveal(parse_self_or_target(inner)?))
        }
        _ => {
            if inner.as_str() == "none" {
                Ok(CostAction::None)
            } else {
                Err(ParseError::UnknownRule(inner.as_str().to_string()))
            }
        }
    }
}

// ── Game Actions ──────────────────────────────────────────────

fn parse_game_action(pair: Pair<Rule>) -> Result<GameAction, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("game action"))?;

    match inner.as_rule() {
        Rule::draw_action => {
            let expr = inner.into_inner().find(|p| p.as_rule() == Rule::expr)
                .ok_or(ParseError::MissingField("draw count"))?;
            Ok(GameAction::Draw { count: parse_expr(expr)? })
        }
        Rule::special_summon_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let from = inner.clone().into_inner().find(|p| p.as_rule() == Rule::zone)
                .map(parse_zone).transpose()?;
            let position = inner.into_inner().find(|p| p.as_rule() == Rule::battle_position)
                .map(parse_battle_position).transpose()?;
            Ok(GameAction::SpecialSummon { target, from, position })
        }
        Rule::negate_action => {
            let mut what = None;
            let mut and_destroy = false;
            let text = inner.as_str();
            and_destroy = text.contains("and") && text.contains("destroy");
            for child in inner.into_inner() {
                if child.as_rule() == Rule::negate_target {
                    what = Some(match child.as_str() {
                        "trigger"    => NegateTarget::Trigger,
                        "effect"     => NegateTarget::Effect,
                        "activation" => NegateTarget::Activation,
                        "summon"     => NegateTarget::Summon,
                        "attack"     => NegateTarget::Attack,
                        _            => NegateTarget::Effect,
                    });
                }
            }
            Ok(GameAction::Negate { what, and_destroy })
        }
        Rule::destroy_action => {
            let target = inner.into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("destroy target"))?;
            Ok(GameAction::Destroy { target: parse_target_expr(target)? })
        }
        Rule::send_to_zone_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let zone = inner.into_inner().find(|p| p.as_rule() == Rule::zone)
                .ok_or(ParseError::MissingField("send zone"))
                .and_then(parse_zone)?;
            Ok(GameAction::SendToZone { target, zone })
        }
        Rule::search_action => {
            let mut parts = inner.into_inner();
            let target = parse_target_expr(parts.find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("search target"))?)?;
            let from = parse_zone(parts.find(|p| p.as_rule() == Rule::zone)
                .ok_or(ParseError::MissingField("search zone"))?)?;
            Ok(GameAction::Search { target, from })
        }
        Rule::add_to_hand_action => {
            let mut children = inner.into_inner();
            let target = parse_target_expr(children.find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("add target"))?)?;
            let from = parse_zone(children.find(|p| p.as_rule() == Rule::zone)
                .ok_or(ParseError::MissingField("add from zone"))?)?;
            Ok(GameAction::AddToHand { target, from })
        }
        Rule::atk_modifier_action => parse_atk_modifier_action(inner),
        Rule::def_modifier_action => parse_def_modifier_action(inner),
        Rule::banish_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let from = inner.clone().into_inner().find(|p| p.as_rule() == Rule::zone)
                .map(parse_zone).transpose()?;
            let face_down = inner.as_str().contains("face_down");
            Ok(GameAction::Banish { target, from, face_down })
        }
        Rule::return_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let text = inner.as_str();
            let to = if text.contains("extra_deck") { ReturnZone::ExtraDeck }
                else if text.contains("deck") { ReturnZone::Deck }
                else { ReturnZone::Hand };
            let shuffle = text.contains("shuffle");
            Ok(GameAction::Return { target, to, shuffle })
        }
        Rule::set_face_down_action => {
            let target = inner.into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("set target"))?;
            Ok(GameAction::SetFaceDown { target: parse_target_expr(target)? })
        }
        Rule::flip_face_down_action => {
            let target = inner.into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("flip target"))?;
            Ok(GameAction::FlipFaceDown { target: parse_target_expr(target)? })
        }
        Rule::change_battle_position_action => {
            let target = inner.into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("position target"))?;
            Ok(GameAction::ChangeBattlePosition { target: parse_target_expr(target)? })
        }
        Rule::take_control_action => {
            let target = inner.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("control target"))?;
            let text = inner.as_str();
            let duration = if text.contains("end_phase") { Some(TakeControlDuration::EndPhase) }
                else if text.contains("end_of_turn") { Some(TakeControlDuration::EndOfTurn) }
                else { None };
            Ok(GameAction::TakeControl { target: parse_target_expr(target)?, duration })
        }
        Rule::place_counter_action => {
            let count = inner.clone().into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(1)).unwrap_or(1);
            let name = inner.clone().into_inner().find(|p| p.as_rule() == Rule::string)
                .map(parse_string).unwrap_or_default();
            let on = parse_self_or_target_last(inner)?;
            Ok(GameAction::PlaceCounter { count, name, on })
        }
        Rule::remove_counter_action => {
            let count = inner.clone().into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(1)).unwrap_or(1);
            let name = inner.clone().into_inner().find(|p| p.as_rule() == Rule::string)
                .map(parse_string).unwrap_or_default();
            let from = parse_self_or_target_last(inner)?;
            Ok(GameAction::RemoveCounter { count, name, from })
        }
        Rule::look_at_action => {
            let target = inner.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("look target"))?;
            let from = inner.into_inner().find(|p| p.as_rule() == Rule::zone)
                .map(parse_zone).transpose()?;
            Ok(GameAction::LookAt { target: parse_target_expr(target)?, from })
        }
        Rule::reveal_action => {
            Ok(GameAction::Reveal { target: parse_self_or_target(inner)? })
        }
        Rule::copy_effect_action => {
            let target = inner.into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("copy target"))?;
            Ok(GameAction::CopyEffect { from: parse_target_expr(target)? })
        }
        Rule::equip_action => {
            let targets: Vec<_> = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::target_expr)
                .collect();
            if targets.len() < 2 {
                return Err(ParseError::MissingField("equip targets"));
            }
            Ok(GameAction::Equip {
                card: parse_target_expr(targets[0].clone())?,
                to: parse_target_expr(targets[1].clone())?,
            })
        }
        Rule::detach_action => {
            let count = inner.clone().into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(1)).unwrap_or(1);
            let from = parse_self_or_target_last(inner)?;
            Ok(GameAction::Detach { count, from })
        }
        Rule::attach_action => {
            let target = inner.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("attach target"))?;
            let to = parse_self_or_target_last(inner)?;
            Ok(GameAction::Attach { target: parse_target_expr(target)?, to })
        }
        Rule::fusion_summon_action => parse_summon_action(inner, |t, m| GameAction::FusionSummon { target: t, materials: m }),
        Rule::synchro_summon_action => parse_summon_action(inner, |t, m| GameAction::SynchroSummon { target: t, materials: m }),
        Rule::xyz_summon_action => parse_summon_action(inner, |t, m| GameAction::XyzSummon { target: t, materials: m }),
        Rule::ritual_summon_action => parse_summon_action(inner, |t, m| GameAction::RitualSummon { target: t, materials: m }),
        Rule::pendulum_summon_action => {
            let target = inner.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("pendulum target"))?;
            let zones = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::zone)
                .map(parse_zone)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(GameAction::PendulumSummon { targets: parse_target_expr(target)?, from: zones })
        }
        Rule::token_action => {
            let spec = parse_token_body(inner)?;
            Ok(GameAction::CreateToken { spec })
        }
        Rule::damage_action => {
            let text = inner.as_str();
            let to = if text.contains("both_players") { DamageTarget::BothPlayers }
                else if text.contains("opponent") { DamageTarget::Opponent }
                else { DamageTarget::You };
            let expr = inner.into_inner().find(|p| p.as_rule() == Rule::expr)
                .ok_or(ParseError::MissingField("damage amount"))?;
            Ok(GameAction::DealDamage { to, amount: parse_expr(expr)? })
        }
        Rule::gain_lp_action => {
            let expr = inner.into_inner().find(|p| p.as_rule() == Rule::expr)
                .ok_or(ParseError::MissingField("gain amount"))?;
            Ok(GameAction::GainLp { amount: parse_expr(expr)? })
        }
        Rule::shuffle_action => {
            let zone = inner.into_inner().find(|p| p.as_rule() == Rule::zone)
                .ok_or(ParseError::MissingField("shuffle zone"))?;
            Ok(GameAction::Shuffle { zone: parse_zone(zone)? })
        }
        Rule::mill_action => {
            let expr = inner.clone().into_inner().find(|p| p.as_rule() == Rule::expr)
                .ok_or(ParseError::MissingField("mill count"))?;
            let text = inner.as_str();
            let from = if text.contains("opponent_deck") { MillSource::OpponentDeck }
                else { MillSource::YourDeck };
            Ok(GameAction::Mill { count: parse_expr(expr)?, from })
        }
        Rule::discard_action => {
            Ok(GameAction::Discard { target: parse_self_or_target(inner)? })
        }
        Rule::tribute_action => {
            Ok(GameAction::Tribute { target: parse_self_or_target(inner)? })
        }
        Rule::set_scale_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let expr = inner.into_inner().find(|p| p.as_rule() == Rule::expr)
                .ok_or(ParseError::MissingField("scale value"))?;
            Ok(GameAction::SetScale { target, value: parse_expr(expr)? })
        }
        Rule::if_action => {
            let mut condition = ConditionExpr::Simple(SimpleCondition::OnField);
            let mut then_actions = vec![];
            let mut else_actions = vec![];
            let mut in_else = false;

            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::condition_expr => condition = parse_condition_expr(child)?,
                    Rule::game_action => {
                        let action = parse_game_action(child)?;
                        if in_else { else_actions.push(action); }
                        else { then_actions.push(action); }
                    }
                    _ => {
                        // "else" keyword
                        if child.as_str() == "else" { in_else = true; }
                    }
                }
            }
            Ok(GameAction::If { condition, then_actions, else_actions })
        }
        Rule::each_action => {
            let target = inner.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("each target"))?;
            let zone = inner.clone().into_inner().find(|p| p.as_rule() == Rule::zone)
                .ok_or(ParseError::MissingField("each zone"))?;
            let actions = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::game_action)
                .map(parse_game_action)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(GameAction::ForEach {
                target: parse_target_expr(target)?,
                in_zone: parse_zone(zone)?,
                actions,
            })
        }
        Rule::apply_until_action => {
            let actions = inner.clone().into_inner()
                .filter(|p| p.as_rule() == Rule::game_action)
                .map(parse_game_action)
                .collect::<Result<Vec<_>, _>>()?;
            let duration = inner.into_inner().find(|p| p.as_rule() == Rule::duration)
                .ok_or(ParseError::MissingField("apply duration"))
                .and_then(parse_duration)?;
            Ok(GameAction::ApplyUntil { actions, duration })
        }
        Rule::choose_action => {
            let options = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::choice_option)
                .map(|opt| {
                    let mut label = String::new();
                    let mut actions = vec![];
                    for child in opt.into_inner() {
                        match child.as_rule() {
                            Rule::string      => label = parse_string(child),
                            Rule::game_action => actions.push(parse_game_action(child).unwrap_or(GameAction::Draw { count: Expr::lit(0) })),
                            _ => {}
                        }
                    }
                    Ok(ChoiceOption { label, actions })
                })
                .collect::<Result<Vec<_>, ParseError>>()?;
            Ok(GameAction::Choose { options })
        }
        Rule::delayed_action => {
            let phase = inner.clone().into_inner().find(|p| p.as_rule() == Rule::phase)
                .ok_or(ParseError::MissingField("delayed phase"))?;
            let actions = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::game_action)
                .map(parse_game_action)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(GameAction::Delayed { until: parse_phase(phase)?, actions })
        }
        Rule::register_effect_action => {
            let target = inner.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("register target"))?;
            let mut modifiers = vec![];
            let mut grants = vec![];
            let mut restrictions = vec![];
            let mut duration = None;

            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::modifier_decl      => modifiers.push(parse_modifier_decl(child)?),
                    Rule::flag_modifier_decl => {
                        if let Some(g) = child.into_inner().next() {
                            grants.push(parse_granted_ability(g)?);
                        }
                    }
                    Rule::restriction_block  => restrictions.extend(parse_restriction_block(child)?),
                    Rule::duration           => duration = Some(parse_duration(child)?),
                    _ => {}
                }
            }
            Ok(GameAction::RegisterEffect {
                target: parse_target_expr(target)?,
                effect: Box::new(InlineEffect { modifiers, grants, restrictions }),
                duration,
            })
        }
        Rule::store_action => {
            let label = inner.clone().into_inner().find(|p| p.as_rule() == Rule::string)
                .map(parse_string).unwrap_or_default();
            let text = inner.as_str();
            let value = if text.contains("selected_targets") {
                StoreValue::SelectedTargets
            } else {
                let expr = inner.into_inner().find(|p| p.as_rule() == Rule::expr)
                    .map(parse_expr).transpose()?.unwrap_or(Expr::lit(0));
                StoreValue::Expression(expr)
            };
            Ok(GameAction::Store { label, value })
        }
        Rule::recall_action => {
            let label = inner.into_inner().find(|p| p.as_rule() == Rule::string)
                .map(parse_string).unwrap_or_default();
            Ok(GameAction::Recall { label })
        }
        Rule::send_to_deck_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let text = inner.as_str();
            let position = if text.contains("top") { DeckPosition::Top }
                else if text.contains("bottom") { DeckPosition::Bottom }
                else { DeckPosition::Shuffle };
            Ok(GameAction::SendToDeck { target, position })
        }
        Rule::release_action => {
            Ok(GameAction::Release { target: parse_self_or_target(inner)? })
        }
        Rule::discard_all_action => {
            let whose = if inner.as_str().contains("opponent") { Player::Opponent } else { Player::You };
            Ok(GameAction::DiscardAll { whose })
        }
        Rule::shuffle_hand_action => {
            let text = inner.as_str();
            let whose = if text.contains("opponents") { Some(Player::Opponent) }
                else if text.contains("yours") { Some(Player::You) }
                else { None };
            Ok(GameAction::ShuffleHand { whose })
        }
        Rule::shuffle_deck_action => {
            let text = inner.as_str();
            let whose = if text.contains("opponents") { Some(Player::Opponent) }
                else if text.contains("yours") { Some(Player::You) }
                else { None };
            Ok(GameAction::ShuffleDeck { whose })
        }
        Rule::change_position_action => {
            // change_position (target) to position
            let target = inner.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("change_position target"))?;
            Ok(GameAction::ChangeBattlePosition { target: parse_target_expr(target)? })
        }
        Rule::set_spell_trap_action => {
            // Use existing SetFaceDown since they're essentially the same
            let target = parse_self_or_target_first(inner)?;
            match target {
                SelfOrTarget::Self_ => Ok(GameAction::SetFaceDown { target: TargetExpr::SelfCard }),
                SelfOrTarget::Target(t) => Ok(GameAction::SetFaceDown { target: t }),
            }
        }
        Rule::equip_to_action => {
            let targets: Vec<_> = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::target_expr)
                .collect();
            if targets.len() < 2 {
                return Err(ParseError::MissingField("equip targets"));
            }
            Ok(GameAction::Equip {
                card: parse_target_expr(targets[0].clone())?,
                to: parse_target_expr(targets[1].clone())?,
            })
        }
        Rule::overlay_action => {
            let mats = inner.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
                .ok_or(ParseError::MissingField("overlay materials"))?;
            let target = parse_self_or_target_last(inner)?;
            Ok(GameAction::Overlay {
                materials: parse_target_expr(mats)?,
                target,
            })
        }
        Rule::move_to_field_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let position = inner.into_inner().find(|p| p.as_rule() == Rule::battle_position)
                .map(parse_battle_position).transpose()?;
            Ok(GameAction::MoveToField { target, position })
        }
        Rule::excavate_action => {
            let expr = inner.clone().into_inner().find(|p| p.as_rule() == Rule::expr)
                .ok_or(ParseError::MissingField("excavate count"))?;
            let text = inner.as_str();
            let from = if text.contains("opponent") { MillSource::OpponentDeck } else { MillSource::YourDeck };
            Ok(GameAction::Excavate { count: parse_expr(expr)?, from })
        }
        Rule::normal_summon_action => {
            Ok(GameAction::NormalSummon { target: parse_self_or_target(inner)? })
        }
        Rule::yes_no_action => {
            let mut yes_actions = vec![];
            let mut no_actions = vec![];
            let mut in_else = false;
            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::game_action => {
                        let action = parse_game_action(child)?;
                        if in_else { no_actions.push(action); } else { yes_actions.push(action); }
                    }
                    _ => if child.as_str() == "else" { in_else = true; }
                }
            }
            Ok(GameAction::YesNo { yes_actions, no_actions })
        }
        Rule::coin_flip_action => {
            let mut heads = vec![];
            let mut tails = vec![];
            let mut in_tails = false;
            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::game_action => {
                        let action = parse_game_action(child)?;
                        if in_tails { tails.push(action); } else { heads.push(action); }
                    }
                    _ => if child.as_str() == "tails" { in_tails = true; }
                }
            }
            Ok(GameAction::CoinFlip { heads, tails })
        }
        Rule::change_level_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let expr = inner.into_inner().find(|p| p.as_rule() == Rule::expr)
                .ok_or(ParseError::MissingField("level value"))?;
            Ok(GameAction::ChangeLevel { target, value: parse_expr(expr)? })
        }
        Rule::change_attribute_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let attr = inner.into_inner().find(|p| p.as_rule() == Rule::attribute)
                .ok_or(ParseError::MissingField("attribute"))?;
            Ok(GameAction::ChangeAttribute { target, attribute: parse_attribute(attr)? })
        }
        Rule::change_race_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let race = inner.into_inner().find(|p| p.as_rule() == Rule::race)
                .ok_or(ParseError::MissingField("race"))?;
            Ok(GameAction::ChangeRace { target, race: parse_race(race)? })
        }
        Rule::negate_effects_action => {
            let target = parse_self_or_target_first(inner.clone())?;
            let duration = inner.into_inner().find(|p| p.as_rule() == Rule::duration)
                .map(parse_duration).transpose()?;
            Ok(GameAction::NegateEffects { target, duration })
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_atk_modifier_action(pair: Pair<Rule>) -> Result<GameAction, ParseError> {
    let text = pair.as_str();
    if text.starts_with("double_atk") {
        let target = pair.into_inner().find(|p| p.as_rule() == Rule::target_expr)
            .map(parse_target_expr).transpose()?;
        return Ok(GameAction::ModifyAtk { kind: AtkModKind::Double, target, duration: None });
    }
    if text.starts_with("halve_atk") {
        let target = pair.into_inner().find(|p| p.as_rule() == Rule::target_expr)
            .map(parse_target_expr).transpose()?;
        return Ok(GameAction::ModifyAtk { kind: AtkModKind::Halve, target, duration: None });
    }
    if text.starts_with("set_atk") {
        let target = pair.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
            .map(parse_target_expr).transpose()?;
        let expr = pair.into_inner().find(|p| p.as_rule() == Rule::expr)
            .ok_or(ParseError::MissingField("atk value"))?;
        return Ok(GameAction::ModifyAtk {
            kind: AtkModKind::SetTo(parse_expr(expr)?),
            target, duration: None,
        });
    }
    // Delta: modifier: atk +/- expr
    let mut sign = Sign::Plus;
    let mut value = Expr::lit(0);
    let mut target = None;
    let mut duration = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::plus_minus  => sign = if child.as_str() == "+" { Sign::Plus } else { Sign::Minus },
            Rule::expr        => value = parse_expr(child)?,
            Rule::target_expr => target = Some(parse_target_expr(child)?),
            Rule::duration    => duration = Some(parse_duration(child)?),
            _ => {}
        }
    }
    Ok(GameAction::ModifyAtk { kind: AtkModKind::Delta { sign, value }, target, duration })
}

fn parse_def_modifier_action(pair: Pair<Rule>) -> Result<GameAction, ParseError> {
    let text = pair.as_str();
    if text.starts_with("set_def") {
        let target = pair.clone().into_inner().find(|p| p.as_rule() == Rule::target_expr)
            .map(parse_target_expr).transpose()?;
        let expr = pair.into_inner().find(|p| p.as_rule() == Rule::expr)
            .ok_or(ParseError::MissingField("def value"))?;
        return Ok(GameAction::ModifyDef {
            kind: DefModKind::SetTo(parse_expr(expr)?),
            target, duration: None,
        });
    }
    let mut sign = Sign::Plus;
    let mut value = Expr::lit(0);
    let mut target = None;
    let mut duration = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::plus_minus  => sign = if child.as_str() == "+" { Sign::Plus } else { Sign::Minus },
            Rule::expr        => value = parse_expr(child)?,
            Rule::target_expr => target = Some(parse_target_expr(child)?),
            Rule::duration    => duration = Some(parse_duration(child)?),
            _ => {}
        }
    }
    Ok(GameAction::ModifyDef { kind: DefModKind::Delta { sign, value }, target, duration })
}

fn parse_summon_action<F>(pair: Pair<Rule>, f: F) -> Result<GameAction, ParseError>
where F: FnOnce(TargetExpr, Vec<TargetExpr>) -> GameAction
{
    let mut target = TargetExpr::SelfCard;
    let mut materials = vec![];
    let mut found_target = false;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::target_expr if !found_target => {
                target = parse_target_expr(child)?;
                found_target = true;
            }
            Rule::material_list => {
                materials = child.into_inner()
                    .filter(|p| p.as_rule() == Rule::target_expr)
                    .map(parse_target_expr)
                    .collect::<Result<Vec<_>, _>>()?;
            }
            _ => {}
        }
    }
    Ok(f(target, materials))
}

fn parse_token_body(pair: Pair<Rule>) -> Result<TokenSpec, ParseError> {
    let mut spec = TokenSpec {
        name: None, attribute: None, race: None,
        atk: StatValue::Number(0), def: StatValue::Number(0),
        count: 1, position: None,
    };
    let body = pair.into_inner().next().ok_or(ParseError::MissingField("token body"))?;
    for child in body.into_inner() {
        match child.as_rule() {
            Rule::string    => spec.name = Some(parse_string(child)),
            Rule::attribute => spec.attribute = Some(parse_attribute(child)?),
            Rule::race      => spec.race = Some(parse_race(child)?),
            Rule::stat_val  => {
                // This is simplified — real impl checks atk/def ordering
                let val = parse_stat_value(child)?;
                if spec.atk == StatValue::Number(0) && spec.def == StatValue::Number(0) {
                    spec.atk = val;
                } else {
                    spec.def = val;
                }
            }
            Rule::unsigned  => spec.count = child.as_str().parse().unwrap_or(1),
            Rule::battle_position => spec.position = Some(parse_battle_position(child)?),
            _ => {}
        }
    }
    Ok(spec)
}

// ── Restrictions ──────────────────────────────────────────────

fn parse_restriction_block(pair: Pair<Rule>) -> Result<Vec<RestrictionRule>, ParseError> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::restriction_rule)
        .map(parse_restriction_rule)
        .collect()
}

fn parse_restriction_rule(pair: Pair<Rule>) -> Result<RestrictionRule, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("restriction rule"))?;
    match inner.as_rule() {
        Rule::cannot_block => Ok(RestrictionRule::Cannot(parse_cannot_block(inner)?)),
        Rule::must_block   => {
            let must = match inner.into_inner().next().map(|p| p.as_str()) {
                Some("attack_if_able")           => MustBlock::AttackIfAble,
                Some("attack_all_monsters")      => MustBlock::AttackAllMonsters,
                Some("change_to_attack_position") => MustBlock::ChangeToAttackPosition,
                _                                 => MustBlock::AttackIfAble,
            };
            Ok(RestrictionRule::Must(must))
        }
        Rule::limit_block => {
            let text = inner.as_str();
            let n = inner.into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(1)).unwrap_or(1);
            let limit = if text.contains("attacks_per_turn") {
                LimitBlock::AttacksPerTurn(n)
            } else {
                LimitBlock::SpecialSummonsPerTurn(n)
            };
            Ok(RestrictionRule::Limit(limit))
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_cannot_block(pair: Pair<Rule>) -> Result<CannotBlock, ParseError> {
    let mut action = CannotAction::BeDestroyed;
    let mut scope = None;
    for child in pair.into_inner() {
        match child.as_rule() {
            Rule::cannot_action => {
                action = match child.as_str() {
                    "be_targeted"            => CannotAction::BeTargeted,
                    "be_destroyed"           => CannotAction::BeDestroyed,
                    "be_negated"             => CannotAction::BeNegated,
                    "be_banished"            => CannotAction::BeBanished,
                    "be_returned"            => CannotAction::BeReturned,
                    "change_battle_position" => CannotAction::ChangeBattlePosition,
                    "be_tributed"            => CannotAction::BeTributed,
                    "attack"                 => CannotAction::Attack,
                    "attack_directly"        => CannotAction::AttackDirectly,
                    "activate_effects"       => CannotAction::ActivateEffects,
                    "special_summon"         => CannotAction::SpecialSummon,
                    _                        => CannotAction::BeDestroyed,
                };
            }
            Rule::restriction_scope => {
                scope = Some(parse_restriction_scope(child)?);
            }
            _ => {}
        }
    }
    Ok(CannotBlock { action, scope })
}

fn parse_restriction_scope(pair: Pair<Rule>) -> Result<RestrictionScope, ParseError> {
    match pair.as_str() {
        "battle"                => Ok(RestrictionScope::Battle),
        "card_effects"          => Ok(RestrictionScope::CardEffects),
        "spell_effects"         => Ok(RestrictionScope::SpellEffects),
        "trap_effects"          => Ok(RestrictionScope::TrapEffects),
        "monster_effects"       => Ok(RestrictionScope::MonsterEffects),
        "opponent_card_effects" => Ok(RestrictionScope::OpponentCardEffects),
        "your_card_effects"     => Ok(RestrictionScope::YourCardEffects),
        "any"                   => Ok(RestrictionScope::Any),
        other                   => Err(ParseError::UnknownRule(other.to_string())),
    }
}

// ── Conditions ────────────────────────────────────────────────

fn parse_condition_expr(pair: Pair<Rule>) -> Result<ConditionExpr, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("condition expr"))?;
    match inner.as_rule() {
        Rule::composite_condition => {
            let mut conditions = vec![];
            let mut op_is_and = true;
            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::simple_condition => conditions.push(parse_simple_condition(child)?),
                    _ => {
                        if child.as_str() == "or" { op_is_and = false; }
                    }
                }
            }
            if op_is_and {
                Ok(ConditionExpr::And(conditions))
            } else {
                Ok(ConditionExpr::Or(conditions))
            }
        }
        Rule::simple_condition => {
            Ok(ConditionExpr::Simple(parse_simple_condition(inner)?))
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_simple_condition(pair: Pair<Rule>) -> Result<SimpleCondition, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("simple condition"))?;
    match inner.as_rule() {
        Rule::chain_includes_condition => {
            let categories = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::chain_category)
                .map(|p| match p.as_str() {
                    "search"         => ChainCategory::Search,
                    "special_summon" => ChainCategory::SpecialSummon,
                    "send_to_gy"     => ChainCategory::SendToGy,
                    "add_to_hand"    => ChainCategory::AddToHand,
                    "draw"           => ChainCategory::Draw,
                    "banish"         => ChainCategory::Banish,
                    "mill"           => ChainCategory::Mill,
                    "destroy"        => ChainCategory::Destroy,
                    "negate"         => ChainCategory::Negate,
                    _                => ChainCategory::Search,
                })
                .collect();
            Ok(SimpleCondition::ChainIncludes(categories))
        }
        Rule::zone_condition => {
            let zone_text = inner.as_str().trim_start_matches("in_");
            // Reconstruct zone from the text after "in_"
            let zone = match zone_text {
                "hand"        => Zone::Hand,
                "gy"          => Zone::Graveyard,
                "graveyard"   => Zone::Graveyard,
                "banished"    => Zone::Banished,
                "deck"        => Zone::Deck,
                _             => Zone::Hand,
            };
            Ok(SimpleCondition::InZone(zone))
        }
        Rule::field_condition => Ok(SimpleCondition::OnField),
        Rule::board_condition => {
            let text = inner.as_str();
            if text.starts_with("you_control_no_monsters") {
                Ok(SimpleCondition::YouControlNoMonsters)
            } else if text.starts_with("opponent_controls_no_monsters") {
                Ok(SimpleCondition::OpponentControlsNoMonsters)
            } else if text.starts_with("field_is_empty") {
                Ok(SimpleCondition::FieldIsEmpty)
            } else if text.starts_with("you_control") {
                let target = inner.into_inner().find(|p| p.as_rule() == Rule::target_expr)
                    .ok_or(ParseError::MissingField("you_control target"))?;
                Ok(SimpleCondition::YouControl(parse_target_expr(target)?))
            } else if text.starts_with("opponent_controls") {
                let target = inner.into_inner().find(|p| p.as_rule() == Rule::target_expr)
                    .ok_or(ParseError::MissingField("opponent_controls target"))?;
                Ok(SimpleCondition::OpponentControls(parse_target_expr(target)?))
            } else {
                Ok(SimpleCondition::FieldIsEmpty)
            }
        }
        Rule::lp_condition => {
            let text = inner.as_str();
            let player = if text.starts_with("your_lp") { Player::You } else { Player::Opponent };
            let op = inner.clone().into_inner().find(|p| p.as_rule() == Rule::compare_op)
                .map(parse_compare_op).transpose()?.unwrap_or(CompareOp::Gte);
            let value = inner.into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
            Ok(SimpleCondition::LpCondition { player, op, value })
        }
        Rule::hand_size_condition => {
            let op = inner.clone().into_inner().find(|p| p.as_rule() == Rule::compare_op)
                .map(parse_compare_op).transpose()?.unwrap_or(CompareOp::Gte);
            let value = inner.into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
            Ok(SimpleCondition::HandSize { op, value })
        }
        Rule::gy_count_condition => {
            let op = inner.clone().into_inner().find(|p| p.as_rule() == Rule::compare_op)
                .map(parse_compare_op).transpose()?.unwrap_or(CompareOp::Gte);
            let value = inner.into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
            Ok(SimpleCondition::CardsInGy { op, value })
        }
        Rule::card_count_condition => {
            let op = inner.clone().into_inner().find(|p| p.as_rule() == Rule::compare_op)
                .map(parse_compare_op).transpose()?.unwrap_or(CompareOp::Gte);
            let value = inner.into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
            Ok(SimpleCondition::YouControlCount { op, value })
        }
        Rule::banished_count_condition => {
            let op = inner.clone().into_inner().find(|p| p.as_rule() == Rule::compare_op)
                .map(parse_compare_op).transpose()?.unwrap_or(CompareOp::Gte);
            let value = inner.into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
            Ok(SimpleCondition::BanishedCount { op, value })
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

// ── Triggers ──────────────────────────────────────────────────

fn parse_trigger_expr(pair: Pair<Rule>) -> Result<TriggerExpr, ParseError> {
    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("trigger expr"))?;
    match inner.as_rule() {
        Rule::opponent_activates_trigger => {
            let actions = inner.into_inner()
                .filter(|p| p.as_rule() == Rule::trigger_action)
                .map(parse_trigger_action)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TriggerExpr::OpponentActivates(actions))
        }
        Rule::when_summoned_trigger => {
            let method = inner.into_inner().find(|p| p.as_rule() == Rule::summon_method)
                .map(parse_summon_method).transpose()?;
            Ok(TriggerExpr::WhenSummoned(method))
        }
        Rule::when_tribute_summoned_trigger => {
            let filter = inner.into_inner().find(|p| p.as_rule() == Rule::tribute_filter)
                .and_then(|p| p.into_inner().next())
                .map(parse_card_filter).transpose()?;
            Ok(TriggerExpr::WhenTributeSummoned(filter))
        }
        Rule::when_tributed_trigger => {
            let text = inner.as_str();
            let for_what = if text.contains("summon") { Some(TributeFor::Summon) }
                else if text.contains("cost") { Some(TributeFor::Cost) }
                else if text.contains("any") { Some(TributeFor::Any) }
                else { None };
            Ok(TriggerExpr::WhenTributed(for_what))
        }
        Rule::when_destroyed_trigger => {
            let cause = inner.into_inner().find(|p| p.as_rule() == Rule::destruction_cause)
                .map(parse_destruction_cause).transpose()?;
            Ok(TriggerExpr::WhenDestroyed(cause))
        }
        Rule::when_sent_to_trigger => {
            let zone = inner.clone().into_inner().find(|p| p.as_rule() == Rule::zone)
                .ok_or(ParseError::MissingField("sent_to zone")).and_then(parse_zone)?;
            let cause = inner.into_inner().find(|p| p.as_rule() == Rule::destruction_cause)
                .map(parse_destruction_cause).transpose()?;
            Ok(TriggerExpr::WhenSentTo { zone, cause })
        }
        Rule::when_flipped_trigger  => Ok(TriggerExpr::WhenFlipped),
        Rule::when_attacked_trigger => Ok(TriggerExpr::WhenAttacked),
        Rule::when_battle_destroyed_trigger   => Ok(TriggerExpr::WhenBattleDestroyed),
        Rule::when_destroys_by_battle_trigger => Ok(TriggerExpr::WhenDestroysByBattle),
        Rule::when_leaves_field_trigger       => Ok(TriggerExpr::WhenLeavesField),
        Rule::when_used_as_material_trigger   => {
            let method = inner.into_inner().next().map(|p| match p.as_str() {
                "fusion"  => SummonMethodType::Fusion,
                "synchro" => SummonMethodType::Synchro,
                "xyz"     => SummonMethodType::Xyz,
                "link"    => SummonMethodType::Link,
                "ritual"  => SummonMethodType::Ritual,
                _         => SummonMethodType::Fusion,
            });
            Ok(TriggerExpr::WhenUsedAsMaterial(method))
        }
        Rule::when_battle_damage_trigger => {
            let player = inner.into_inner().next().map(|p| match p.as_str() {
                "you"      => Player::You,
                "opponent" => Player::Opponent,
                _          => Player::You,
            });
            Ok(TriggerExpr::WhenBattleDamage(player))
        }
        Rule::when_banished_trigger => {
            let cause = inner.into_inner().find(|p| p.as_rule() == Rule::destruction_cause)
                .map(parse_destruction_cause).transpose()?;
            Ok(TriggerExpr::WhenBanished(cause))
        }
        Rule::on_nth_summon_trigger => {
            let n = inner.into_inner().find(|p| p.as_rule() == Rule::unsigned)
                .map(|p| p.as_str().parse().unwrap_or(1)).unwrap_or(1);
            Ok(TriggerExpr::OnNthSummon(n))
        }
        Rule::standby_phase_trigger => {
            let text = inner.as_str();
            let owner = if text.contains("yours") { Some(PhaseOwner::Yours) }
                else if text.contains("opponents") { Some(PhaseOwner::Opponents) }
                else if text.contains("either") { Some(PhaseOwner::Either) }
                else { None };
            Ok(TriggerExpr::DuringStandbyPhase(owner))
        }
        Rule::end_phase_trigger     => Ok(TriggerExpr::DuringEndPhase),
        Rule::during_phase_trigger  => {
            let phase = inner.into_inner().find(|p| p.as_rule() == Rule::phase)
                .ok_or(ParseError::MissingField("phase"))?;
            Ok(TriggerExpr::DuringPhase(parse_phase(phase)?))
        }
        Rule::when_action_trigger => {
            let action = inner.into_inner().find(|p| p.as_rule() == Rule::trigger_action)
                .ok_or(ParseError::MissingField("trigger action"))?;
            Ok(TriggerExpr::WhenAction(parse_trigger_action(action)?))
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_trigger_action(pair: Pair<Rule>) -> Result<TriggerAction, ParseError> {
    match pair.as_str() {
        "search"                  => Ok(TriggerAction::Search),
        "special_summon"          => Ok(TriggerAction::SpecialSummon),
        "send_to_gy"              => Ok(TriggerAction::SendToGy),
        "add_to_hand"             => Ok(TriggerAction::AddToHand),
        "draw"                    => Ok(TriggerAction::Draw),
        "banish"                  => Ok(TriggerAction::Banish),
        "mill"                    => Ok(TriggerAction::Mill),
        "token_spawn"             => Ok(TriggerAction::TokenSpawn),
        "activate_spell"          => Ok(TriggerAction::ActivateSpell),
        "activate_trap"           => Ok(TriggerAction::ActivateTrap),
        "activate_monster_effect" => Ok(TriggerAction::ActivateMonsterEffect),
        "fusion_summon"           => Ok(TriggerAction::FusionSummon),
        "synchro_summon"          => Ok(TriggerAction::SynchroSummon),
        "xyz_summon"              => Ok(TriggerAction::XyzSummon),
        "link_summon"             => Ok(TriggerAction::LinkSummon),
        "ritual_summon"           => Ok(TriggerAction::RitualSummon),
        "normal_summon"           => Ok(TriggerAction::NormalSummon),
        "set_card"                => Ok(TriggerAction::SetCard),
        "change_battle_position"  => Ok(TriggerAction::ChangeBattlePosition),
        "take_damage"             => Ok(TriggerAction::TakeDamage),
        "gain_lp"                 => Ok(TriggerAction::GainLp),
        "attack_declared"         => Ok(TriggerAction::AttackDeclared),
        other                     => Err(ParseError::UnknownRule(other.to_string())),
    }
}

// ── Shared Helpers ────────────────────────────────────────────

fn parse_string(pair: Pair<Rule>) -> String {
    pair.as_str().trim_matches('"').to_string()
}

fn parse_unsigned(pair: Pair<Rule>) -> Result<u32, ParseError> {
    pair.as_str().parse().map_err(|_| ParseError::InvalidValue("unsigned"))
}

fn parse_stat_value(pair: Pair<Rule>) -> Result<StatValue, ParseError> {
    let s = pair.as_str();
    if s == "?" { Ok(StatValue::Variable) }
    else { s.parse::<i32>().map(StatValue::Number).map_err(|_| ParseError::InvalidValue("stat value")) }
}

impl PartialEq for StatValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (StatValue::Variable, StatValue::Variable) => true,
            (StatValue::Number(a), StatValue::Number(b)) => a == b,
            _ => false,
        }
    }
}

fn parse_card_type(pair: Pair<Rule>) -> Result<CardType, ParseError> {
    match pair.as_str() {
        "Normal Monster"   => Ok(CardType::NormalMonster),
        "Effect Monster"   => Ok(CardType::EffectMonster),
        "Ritual Monster"   => Ok(CardType::RitualMonster),
        "Fusion Monster"   => Ok(CardType::FusionMonster),
        "Synchro Monster"  => Ok(CardType::SynchroMonster),
        "Xyz Monster"      => Ok(CardType::XyzMonster),
        "Link Monster"     => Ok(CardType::LinkMonster),
        "Pendulum Monster" => Ok(CardType::PendulumMonster),
        "Tuner"            => Ok(CardType::Tuner),
        "Synchro Tuner"    => Ok(CardType::SynchroTuner),
        "Gemini"           => Ok(CardType::Gemini),
        "Union"            => Ok(CardType::Union),
        "Spirit"           => Ok(CardType::Spirit),
        "Flip"             => Ok(CardType::Flip),
        "Toon"             => Ok(CardType::Toon),
        "Normal Spell"     => Ok(CardType::NormalSpell),
        "Quick-Play Spell" => Ok(CardType::QuickPlaySpell),
        "Continuous Spell" => Ok(CardType::ContinuousSpell),
        "Equip Spell"      => Ok(CardType::EquipSpell),
        "Field Spell"      => Ok(CardType::FieldSpell),
        "Ritual Spell"     => Ok(CardType::RitualSpell),
        "Normal Trap"      => Ok(CardType::NormalTrap),
        "Counter Trap"     => Ok(CardType::CounterTrap),
        "Continuous Trap"  => Ok(CardType::ContinuousTrap),
        other => Err(ParseError::UnknownCardType(other.to_string())),
    }
}

fn parse_attribute(pair: Pair<Rule>) -> Result<Attribute, ParseError> {
    match pair.as_str() {
        "LIGHT"  => Ok(Attribute::Light),
        "DARK"   => Ok(Attribute::Dark),
        "FIRE"   => Ok(Attribute::Fire),
        "WATER"  => Ok(Attribute::Water),
        "EARTH"  => Ok(Attribute::Earth),
        "WIND"   => Ok(Attribute::Wind),
        "DIVINE" => Ok(Attribute::Divine),
        other    => Err(ParseError::UnknownAttribute(other.to_string())),
    }
}

fn parse_race(pair: Pair<Rule>) -> Result<Race, ParseError> {
    match pair.as_str() {
        "Dragon"       => Ok(Race::Dragon),
        "Spellcaster"  => Ok(Race::Spellcaster),
        "Zombie"       => Ok(Race::Zombie),
        "Warrior"      => Ok(Race::Warrior),
        "Beast-Warrior"=> Ok(Race::BeastWarrior),
        "Beast"        => Ok(Race::Beast),
        "Winged Beast" => Ok(Race::WingedBeast),
        "Fiend"        => Ok(Race::Fiend),
        "Fairy"        => Ok(Race::Fairy),
        "Insect"       => Ok(Race::Insect),
        "Dinosaur"     => Ok(Race::Dinosaur),
        "Reptile"      => Ok(Race::Reptile),
        "Fish"         => Ok(Race::Fish),
        "Sea Serpent"  => Ok(Race::SeaSerpent),
        "Aqua"         => Ok(Race::Aqua),
        "Pyro"         => Ok(Race::Pyro),
        "Thunder"      => Ok(Race::Thunder),
        "Rock"         => Ok(Race::Rock),
        "Plant"        => Ok(Race::Plant),
        "Machine"      => Ok(Race::Machine),
        "Psychic"      => Ok(Race::Psychic),
        "Divine-Beast" => Ok(Race::DivineBeast),
        "Wyrm"         => Ok(Race::Wyrm),
        "Cyberse"      => Ok(Race::Cyberse),
        other          => Err(ParseError::UnknownRace(other.to_string())),
    }
}

fn parse_spell_speed(pair: Pair<Rule>) -> Result<SpellSpeed, ParseError> {
    match pair.as_str() {
        "spell_speed_1" => Ok(SpellSpeed::SpellSpeed1),
        "spell_speed_2" => Ok(SpellSpeed::SpellSpeed2),
        "spell_speed_3" => Ok(SpellSpeed::SpellSpeed3),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_zone(pair: Pair<Rule>) -> Result<Zone, ParseError> {
    match pair.as_str() {
        "hand"               => Ok(Zone::Hand),
        "field"              => Ok(Zone::Field),
        "graveyard" | "gy"   => Ok(Zone::Graveyard),
        "banished" | "exile" => Ok(Zone::Banished),
        "deck"               => Ok(Zone::Deck),
        "extra_deck"         => Ok(Zone::ExtraDeck),
        "extra_deck_face_up" => Ok(Zone::ExtraDeckFaceUp),
        "spell_trap_zone"    => Ok(Zone::SpellTrapZone),
        "monster_zone"       => Ok(Zone::MonsterZone),
        "extra_monster_zone" => Ok(Zone::ExtraMonsterZone),
        "top_of_deck"        => Ok(Zone::TopOfDeck),
        "bottom_of_deck"     => Ok(Zone::BottomOfDeck),
        "pendulum_zone"      => Ok(Zone::PendulumZone),
        "field_zone"         => Ok(Zone::FieldZone),
        other => Err(ParseError::UnknownZone(other.to_string())),
    }
}

fn parse_duration(pair: Pair<Rule>) -> Result<Duration, ParseError> {
    match pair.as_str() {
        "until_end_of_turn"       => Ok(Duration::UntilEndOfTurn),
        "until_end_phase"         => Ok(Duration::UntilEndPhase),
        "until_end_of_damage_step"=> Ok(Duration::UntilEndOfDamageStep),
        "until_next_turn"         => Ok(Duration::UntilNextTurn),
        "permanently"             => Ok(Duration::Permanently),
        "this_turn"               => Ok(Duration::ThisTurn),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_phase(pair: Pair<Rule>) -> Result<Phase, ParseError> {
    match pair.as_str() {
        "draw_phase"          => Ok(Phase::DrawPhase),
        "standby_phase"       => Ok(Phase::StandbyPhase),
        "main_phase_1"        => Ok(Phase::MainPhase1),
        "damage_step"         => Ok(Phase::DamageStep),
        "damage_calculation"  => Ok(Phase::DamageCalculation),
        "battle_phase"        => Ok(Phase::BattlePhase),
        "main_phase_2"        => Ok(Phase::MainPhase2),
        "end_phase"           => Ok(Phase::EndPhase),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_compare_op(pair: Pair<Rule>) -> Result<CompareOp, ParseError> {
    match pair.as_str() {
        ">=" => Ok(CompareOp::Gte),
        "<=" => Ok(CompareOp::Lte),
        ">"  => Ok(CompareOp::Gt),
        "<"  => Ok(CompareOp::Lt),
        "==" => Ok(CompareOp::Eq),
        "!=" => Ok(CompareOp::Neq),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_summon_method(pair: Pair<Rule>) -> Result<SummonMethod, ParseError> {
    match pair.as_str() {
        "by_normal_summon"  => Ok(SummonMethod::ByNormalSummon),
        "by_special_summon" => Ok(SummonMethod::BySpecialSummon),
        "by_flip_summon"    => Ok(SummonMethod::ByFlipSummon),
        "by_ritual_summon"  => Ok(SummonMethod::ByRitualSummon),
        "by_fusion_summon"  => Ok(SummonMethod::ByFusionSummon),
        "by_synchro_summon" => Ok(SummonMethod::BySynchroSummon),
        "by_xyz_summon"     => Ok(SummonMethod::ByXyzSummon),
        "by_link_summon"    => Ok(SummonMethod::ByLinkSummon),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_destruction_cause(pair: Pair<Rule>) -> Result<DestructionCause, ParseError> {
    match pair.as_str() {
        "battle"          => Ok(DestructionCause::Battle),
        "card_effect"     => Ok(DestructionCause::CardEffect),
        "your_effect"     => Ok(DestructionCause::YourEffect),
        "opponent_effect" => Ok(DestructionCause::OpponentEffect),
        "any"             => Ok(DestructionCause::Any),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_battle_position(pair: Pair<Rule>) -> Result<BattlePosition, ParseError> {
    match pair.as_str() {
        "attack_position"    => Ok(BattlePosition::AttackPosition),
        "defense_position"   => Ok(BattlePosition::DefensePosition),
        "face_down_defense"  => Ok(BattlePosition::FaceDownDefense),
        other => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_target_expr(pair: Pair<Rule>) -> Result<TargetExpr, ParseError> {
    if pair.as_str() == "self" {
        return Ok(TargetExpr::SelfCard);
    }

    let inner = pair.into_inner().next().ok_or(ParseError::MissingField("target inner"))?;

    if inner.as_str() == "self" {
        return Ok(TargetExpr::SelfCard);
    }

    match inner.as_rule() {
        Rule::counted_target => {
            let mut count = 1u32;
            let mut count_or_more = false;
            let mut filter = CardFilter::Card;
            let mut controller = None;
            let mut zone = None;
            let mut qualifiers = vec![];

            for child in inner.into_inner() {
                match child.as_rule() {
                    Rule::unsigned => {
                        count = child.as_str().parse().unwrap_or(1);
                        // Check for "+" in the parent text
                    }
                    Rule::card_filter => filter = parse_card_filter(child)?,
                    Rule::controller_ref => controller = Some(parse_controller_ref(child)?),
                    Rule::zone => zone = Some(parse_zone(child)?),
                    Rule::target_qualifier => qualifiers.push(parse_target_qualifier(child)?),
                    _ => {
                        // Check for "+" count_or_more marker
                        if child.as_str() == "+" { count_or_more = true; }
                    }
                }
            }
            Ok(TargetExpr::Counted { count, count_or_more, filter, controller, zone, qualifiers, predicate: None })
        }
        Rule::filter_target => {
            let filter = inner.into_inner().next()
                .ok_or(ParseError::MissingField("filter target"))?;
            Ok(TargetExpr::Filter(parse_card_filter(filter)?))
        }
        Rule::card_filter => {
            Ok(TargetExpr::Filter(parse_card_filter(inner)?))
        }
        _ => Err(ParseError::UnknownRule(inner.as_str().to_string())),
    }
}

fn parse_card_filter(pair: Pair<Rule>) -> Result<CardFilter, ParseError> {
    let text = pair.as_str();
    match text {
        "monster"            => Ok(CardFilter::Monster),
        "spell"              => Ok(CardFilter::Spell),
        "trap"               => Ok(CardFilter::Trap),
        "card"               => Ok(CardFilter::Card),
        "token"              => Ok(CardFilter::Token),
        "non-token monster"  => Ok(CardFilter::NonTokenMonster),
        "tuner monster"      => Ok(CardFilter::TunerMonster),
        "non-tuner monster"  => Ok(CardFilter::NonTunerMonster),
        "normal monster"     => Ok(CardFilter::NormalMonster),
        "effect monster"     => Ok(CardFilter::EffectMonster),
        "fusion monster"     => Ok(CardFilter::FusionMonster),
        "synchro monster"    => Ok(CardFilter::SynchroMonster),
        "xyz monster"        => Ok(CardFilter::XyzMonster),
        "link monster"       => Ok(CardFilter::LinkMonster),
        "ritual monster"     => Ok(CardFilter::RitualMonster),
        _ => {
            let trimmed = text.trim_matches('"');
            if trimmed.ends_with(" monster") {
                let name = trimmed.trim_end_matches(" monster").trim_matches('"');
                Ok(CardFilter::ArchetypeMonster(name.to_string()))
            } else if trimmed.ends_with(" card") {
                let name = trimmed.trim_end_matches(" card").trim_matches('"');
                Ok(CardFilter::ArchetypeCard(name.to_string()))
            } else {
                Ok(CardFilter::NamedCard(trimmed.to_string()))
            }
        }
    }
}

fn parse_controller_ref(pair: Pair<Rule>) -> Result<ControllerRef, ParseError> {
    match pair.as_str() {
        "you"           => Ok(ControllerRef::You),
        "opponent"      => Ok(ControllerRef::Opponent),
        "either_player" => Ok(ControllerRef::EitherPlayer),
        other           => Err(ParseError::UnknownRule(other.to_string())),
    }
}

fn parse_target_qualifier(pair: Pair<Rule>) -> Result<TargetQualifier, ParseError> {
    let text = pair.as_str();
    match text {
        "face_up"                    => Ok(TargetQualifier::FaceUp),
        "face_down"                  => Ok(TargetQualifier::FaceDown),
        "in_attack_position"         => Ok(TargetQualifier::InAttackPosition),
        "in_defense_position"        => Ok(TargetQualifier::InDefensePosition),
        "that_was_normal_summoned"   => Ok(TargetQualifier::ThatWasNormalSummoned),
        "that_was_special_summoned"  => Ok(TargetQualifier::ThatWasSpecialSummoned),
        "other_than_self"            => Ok(TargetQualifier::OtherThanSelf),
        _ => {
            // Compound qualifiers like "with_atk >= 2000"
            let mut inner = pair.into_inner();
            if text.starts_with("with_counter") {
                let name = inner.find(|p| p.as_rule() == Rule::string)
                    .map(parse_string).unwrap_or_default();
                return Ok(TargetQualifier::WithCounter(name));
            }
            if text.starts_with("with_atk") {
                let op = inner.clone().find(|p| p.as_rule() == Rule::compare_op)
                    .map(parse_compare_op).transpose()?.unwrap_or(CompareOp::Gte);
                let val = inner.find(|p| p.as_rule() == Rule::unsigned)
                    .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
                return Ok(TargetQualifier::WithAtk(op, val));
            }
            if text.starts_with("with_def") {
                let op = inner.clone().find(|p| p.as_rule() == Rule::compare_op)
                    .map(parse_compare_op).transpose()?.unwrap_or(CompareOp::Gte);
                let val = inner.find(|p| p.as_rule() == Rule::unsigned)
                    .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
                return Ok(TargetQualifier::WithDef(op, val));
            }
            if text.starts_with("with_level") {
                let op = inner.clone().find(|p| p.as_rule() == Rule::compare_op)
                    .map(parse_compare_op).transpose()?.unwrap_or(CompareOp::Gte);
                let val = inner.find(|p| p.as_rule() == Rule::unsigned)
                    .map(|p| p.as_str().parse().unwrap_or(0)).unwrap_or(0);
                return Ok(TargetQualifier::WithLevel(op, val));
            }
            if text.starts_with("of_attribute") {
                let attr = inner.find(|p| p.as_rule() == Rule::attribute)
                    .ok_or(ParseError::MissingField("attribute qualifier"))?;
                return Ok(TargetQualifier::OfAttribute(parse_attribute(attr)?));
            }
            if text.starts_with("of_race") {
                let race = inner.find(|p| p.as_rule() == Rule::race)
                    .ok_or(ParseError::MissingField("race qualifier"))?;
                return Ok(TargetQualifier::OfRace(parse_race(race)?));
            }
            if text.starts_with("of_archetype") {
                let name = inner.find(|p| p.as_rule() == Rule::string)
                    .map(parse_string).unwrap_or_default();
                return Ok(TargetQualifier::OfArchetype(name));
            }
            Err(ParseError::UnknownRule(text.to_string()))
        }
    }
}

// ── Self/Target helpers ───────────────────────────────────────

fn parse_self_or_target(pair: Pair<Rule>) -> Result<SelfOrTarget, ParseError> {
    for child in pair.into_inner() {
        if child.as_str() == "self" {
            return Ok(SelfOrTarget::Self_);
        }
        if child.as_rule() == Rule::target_expr {
            return Ok(SelfOrTarget::Target(parse_target_expr(child)?));
        }
    }
    Ok(SelfOrTarget::Self_)
}

fn parse_self_or_target_first(pair: Pair<Rule>) -> Result<SelfOrTarget, ParseError> {
    for child in pair.into_inner() {
        if child.as_str() == "self" {
            return Ok(SelfOrTarget::Self_);
        }
        if child.as_rule() == Rule::target_expr {
            return Ok(SelfOrTarget::Target(parse_target_expr(child)?));
        }
    }
    Ok(SelfOrTarget::Self_)
}

fn parse_self_or_target_last(pair: Pair<Rule>) -> Result<SelfOrTarget, ParseError> {
    let mut last = SelfOrTarget::Self_;
    for child in pair.into_inner() {
        if child.as_str() == "self" {
            last = SelfOrTarget::Self_;
        } else if child.as_rule() == Rule::target_expr {
            last = SelfOrTarget::Target(parse_target_expr(child)?);
        }
    }
    Ok(last)
}
