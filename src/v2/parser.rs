// ============================================================
// DuelScript v2 Parser
// Converts pest parse tree (duelscript.pest) into v2 AST
// ============================================================

use pest::Parser;
use pest::iterators::Pair;
use pest_derive::Parser;
use super::ast::*;
use std::fmt;

#[derive(Parser)]
#[grammar = "grammar/duelscript.pest"]
pub struct V2Parser;

// ── Error Type ──────────────────────────────────────────────

#[derive(Debug)]
pub enum V2ParseError {
    PestError(String),
    MissingField(&'static str),
    InvalidValue(String),
    UnknownRule(String),
}

impl fmt::Display for V2ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            V2ParseError::PestError(e) => write!(f, "Parse error: {}", e),
            V2ParseError::MissingField(s) => write!(f, "Missing field: {}", s),
            V2ParseError::InvalidValue(s) => write!(f, "Invalid value: {}", s),
            V2ParseError::UnknownRule(s) => write!(f, "Unknown rule: {}", s),
        }
    }
}

impl std::error::Error for V2ParseError {}

// ── Helpers ─────────────────────────────────────────────────

fn strip_quotes(s: &str) -> String {
    s.trim_matches('"').to_string()
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Entry Point ─────────────────────────────────────────────

pub fn parse_v2(source: &str) -> Result<File, V2ParseError> {
    let pairs = V2Parser::parse(Rule::file, source)
        .map_err(|e| V2ParseError::PestError(e.to_string()))?;

    let mut cards = Vec::new();
    for pair in pairs {
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::card => cards.push(parse_card(inner)?),
                Rule::EOI => {}
                _ => {}
            }
        }
    }
    Ok(File { cards })
}

// ── Card ────────────────────────────────────────────────────

fn parse_card(pair: Pair<Rule>) -> Result<Card, V2ParseError> {
    let mut inner = pair.into_inner();
    let name = strip_quotes(inner.next().unwrap().as_str());
    let card_body = inner.next().unwrap();

    let mut fields = CardFields::default();
    let mut summon = None;
    let mut effects = Vec::new();
    let mut passives = Vec::new();
    let mut restrictions = Vec::new();
    let mut replacements = Vec::new();

    for card_item in card_body.into_inner() {
        let item = card_item.into_inner().next().unwrap();
        match item.as_rule() {
            Rule::field_decl => parse_field_decl(item, &mut fields)?,
            Rule::summon_block => summon = Some(parse_summon_block(item)?),
            Rule::effect_block => effects.push(parse_effect_block(item)?),
            Rule::passive_block => passives.push(parse_passive_block(item)?),
            Rule::restriction_block => restrictions.push(parse_restriction_block(item)?),
            Rule::replacement_block => replacements.push(parse_replacement_block(item)?),
            _ => {}
        }
    }

    Ok(Card { name, fields, summon, effects, passives, restrictions, replacements })
}

// ── Field Declarations ──────────────────────────────────────

fn parse_field_decl(pair: Pair<Rule>, fields: &mut CardFields) -> Result<(), V2ParseError> {
    let text = pair.as_str().trim();
    let field_name = text.split(':').next().unwrap_or("").trim();
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    match field_name {
        "id" => {
            fields.id = Some(inner[0].as_str().parse::<u64>()
                .map_err(|_| V2ParseError::InvalidValue("id".into()))?);
        }
        "type" => {
            for p in &inner {
                if p.as_rule() == Rule::card_type {
                    fields.card_types.push(parse_card_type(p.as_str().trim())?);
                }
            }
        }
        "attribute" => {
            fields.attribute = Some(parse_attribute(inner[0].as_str().trim())?);
        }
        "race" => {
            fields.race = Some(parse_race(inner[0].as_str().trim())?);
        }
        "level" => {
            fields.level = Some(inner[0].as_str().parse::<u32>()
                .map_err(|_| V2ParseError::InvalidValue("level".into()))?);
        }
        "rank" => {
            fields.rank = Some(inner[0].as_str().parse::<u32>()
                .map_err(|_| V2ParseError::InvalidValue("rank".into()))?);
        }
        "link_arrows" => {
            for p in &inner {
                if p.as_rule() == Rule::arrow {
                    fields.link_arrows.push(parse_arrow(p.as_str().trim())?);
                }
            }
        }
        "link" => {
            fields.link = Some(inner[0].as_str().parse::<u32>()
                .map_err(|_| V2ParseError::InvalidValue("link".into()))?);
        }
        "scale" => {
            fields.scale = Some(inner[0].as_str().parse::<u32>()
                .map_err(|_| V2ParseError::InvalidValue("scale".into()))?);
        }
        "atk" => {
            fields.atk = Some(parse_stat_val(inner[0].as_str().trim())?);
        }
        "def" => {
            fields.def = Some(parse_stat_val(inner[0].as_str().trim())?);
        }
        "archetype" => {
            for p in &inner {
                if p.as_rule() == Rule::string {
                    fields.archetypes.push(strip_quotes(p.as_str()));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ── Summon Block ────────────────────────────────────────────

fn parse_summon_block(pair: Pair<Rule>) -> Result<SummonBlock, V2ParseError> {
    let mut sb = SummonBlock {
        cannot_normal_summon: false,
        cannot_special_summon: false,
        tributes: None,
        special_summon_procedure: None,
        fusion_materials: None,
        synchro_materials: None,
        xyz_materials: None,
        link_materials: None,
        ritual_materials: None,
        pendulum_from: vec![],
    };

    for item in pair.into_inner() {
        let text = normalize_ws(item.as_str());
        let mut item_inner = item.into_inner();

        if text == "cannot_normal_summon" {
            sb.cannot_normal_summon = true;
        } else if text == "cannot_special_summon" {
            sb.cannot_special_summon = true;
        } else if text.starts_with("tributes") {
            if let Some(p) = item_inner.next() {
                sb.tributes = Some(p.as_str().parse::<u32>()
                    .map_err(|_| V2ParseError::InvalidValue("tributes".into()))?);
            }
        } else if text.starts_with("special_summon_procedure") {
            sb.special_summon_procedure = Some(parse_ssp(item_inner)?);
        } else if text.starts_with("fusion") {
            sb.fusion_materials = Some(parse_material_list(item_inner)?);
        } else if text.starts_with("synchro") {
            sb.synchro_materials = Some(parse_synchro_materials(item_inner)?);
        } else if text.starts_with("xyz") {
            if let Some(p) = item_inner.next() {
                sb.xyz_materials = Some(parse_selector(p)?);
            }
        } else if text.starts_with("link materials") {
            if let Some(p) = item_inner.next() {
                sb.link_materials = Some(parse_selector(p)?);
            }
        } else if text.starts_with("ritual") {
            sb.ritual_materials = Some(parse_ritual_materials(item_inner)?);
        } else if text.starts_with("pendulum") {
            for p in item_inner {
                if p.as_rule() == Rule::zone {
                    sb.pendulum_from.push(parse_zone(p.as_str().trim())?);
                }
            }
        }
    }
    Ok(sb)
}

fn parse_ssp(pairs: pest::iterators::Pairs<Rule>) -> Result<SpecialSummonProcedure, V2ParseError> {
    let mut ssp = SpecialSummonProcedure {
        from: None,
        to: None,
        cost: vec![],
        condition: None,
        restriction: None,
    };

    for item in pairs {
        let text = normalize_ws(item.as_str());

        if text.starts_with("from") {
            if let Some(p) = item.into_inner().next() {
                ssp.from = Some(parse_zone(p.as_str().trim())?);
            }
        } else if text.starts_with("to") {
            if let Some(p) = item.into_inner().next() {
                ssp.to = Some(parse_field_target(p.as_str().trim())?);
            }
        } else if text.starts_with("cost") {
            ssp.cost = parse_cost_block(item)?;
        } else if text.starts_with("condition") {
            if let Some(p) = item.into_inner().next() {
                ssp.condition = Some(parse_condition(p)?);
            }
        } else if text.starts_with("restriction") {
            ssp.restriction = Some(parse_restriction_block(item)?);
        }
    }
    Ok(ssp)
}

fn parse_material_list(pairs: pest::iterators::Pairs<Rule>) -> Result<MaterialList, V2ParseError> {
    let mut items = Vec::new();
    for p in pairs {
        match p.as_rule() {
            Rule::material_item => {
                let inner = p.into_inner().next().unwrap();
                match inner.as_rule() {
                    Rule::string => items.push(MaterialItem::Named(strip_quotes(inner.as_str()))),
                    Rule::selector => items.push(MaterialItem::Generic(parse_selector(inner)?)),
                    _ => {}
                }
            }
            Rule::material_list => {
                for mi in p.into_inner() {
                    let inner = mi.into_inner().next().unwrap();
                    match inner.as_rule() {
                        Rule::string => items.push(MaterialItem::Named(strip_quotes(inner.as_str()))),
                        Rule::selector => items.push(MaterialItem::Generic(parse_selector(inner)?)),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(MaterialList { items })
}

fn parse_synchro_materials(pairs: pest::iterators::Pairs<Rule>) -> Result<SynchroMaterials, V2ParseError> {
    let mut tuner = None;
    let mut non_tuner = None;
    for p in pairs {
        let text = normalize_ws(p.as_str());
        let mut inner = p.into_inner();
        if text.starts_with("tuner") && !text.starts_with("non_tuner") {
            if let Some(sel) = inner.next() {
                tuner = Some(parse_selector(sel)?);
            }
        } else if text.starts_with("non_tuner") {
            if let Some(sel) = inner.next() {
                non_tuner = Some(parse_selector(sel)?);
            }
        }
    }
    Ok(SynchroMaterials {
        tuner: tuner.ok_or(V2ParseError::MissingField("synchro tuner"))?,
        non_tuner: non_tuner.ok_or(V2ParseError::MissingField("synchro non_tuner"))?,
    })
}

fn parse_ritual_materials(mut pairs: pest::iterators::Pairs<Rule>) -> Result<RitualMaterials, V2ParseError> {
    let materials = parse_selector(pairs.next().ok_or(V2ParseError::MissingField("ritual materials"))?)?;
    let level_constraint = pairs.next().map(|p| parse_level_constraint(p)).transpose()?;
    Ok(RitualMaterials { materials, level_constraint })
}

fn parse_level_constraint(pair: Pair<Rule>) -> Result<LevelConstraint, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let mut inner = pair.into_inner();

    let kind = if text.contains("total_level") {
        LevelConstraintKind::TotalLevel
    } else {
        LevelConstraintKind::ExactLevel
    };

    let op = parse_compare_op(inner.next().ok_or(V2ParseError::MissingField("compare_op"))?.as_str().trim())?;
    let value = parse_expr(inner.next().ok_or(V2ParseError::MissingField("level_constraint value"))?)?;

    Ok(LevelConstraint { kind, op, value })
}

// ── Effect Block ────────────────────────────────────────────

fn parse_effect_block(pair: Pair<Rule>) -> Result<Effect, V2ParseError> {
    let mut inner = pair.into_inner();
    let name = strip_quotes(inner.next().unwrap().as_str());

    let mut effect = Effect {
        name,
        speed: None,
        frequency: None,
        mandatory: false,
        timing: None,
        trigger: None,
        who: None,
        condition: None,
        activate_from: vec![],
        damage_step: None,
        target: None,
        cost: vec![],
        resolve: vec![],
        choose: None,
    };

    for item_pair in inner {
        let item = item_pair.into_inner().next().unwrap();
        match item.as_rule() {
            Rule::speed_decl => {
                let text = item.as_str().trim();
                let speed_str = text.split(':').last().unwrap().trim();
                effect.speed = Some(speed_str.parse::<u8>()
                    .map_err(|_| V2ParseError::InvalidValue("speed".into()))?);
            }
            Rule::frequency_decl => {
                effect.frequency = Some(parse_frequency(item.as_str().trim())?);
            }
            Rule::mandatory_decl => {
                effect.mandatory = true;
            }
            Rule::timing_decl => {
                let text = item.as_str().trim();
                if text.contains("when") {
                    effect.timing = Some(Timing::When);
                } else {
                    effect.timing = Some(Timing::If);
                }
            }
            Rule::trigger_decl => {
                let trigger_expr = item.into_inner().next().unwrap();
                effect.trigger = Some(parse_trigger(trigger_expr)?);
            }
            Rule::who_decl => {
                let pw = item.into_inner().next().unwrap();
                effect.who = Some(parse_player_who(pw.as_str().trim())?);
            }
            Rule::condition_decl => {
                let cond = item.into_inner().next().unwrap();
                effect.condition = Some(parse_condition(cond)?);
            }
            Rule::activate_from_decl => {
                for z in item.into_inner() {
                    if z.as_rule() == Rule::zone {
                        effect.activate_from.push(parse_zone(z.as_str().trim())?);
                    }
                }
            }
            Rule::damage_step_decl => {
                let text = item.as_str().trim();
                effect.damage_step = Some(text.contains("true"));
            }
            Rule::target_decl => {
                effect.target = Some(parse_target_decl(item)?);
            }
            Rule::cost_block => {
                effect.cost = parse_cost_block(item)?;
            }
            Rule::resolve_block => {
                effect.resolve = parse_action_list(item)?;
            }
            Rule::choose_block => {
                effect.choose = Some(parse_choose_block(item)?);
            }
            _ => {}
        }
    }
    Ok(effect)
}

fn parse_frequency(text: &str) -> Result<Frequency, V2ParseError> {
    let normalized = normalize_ws(text);
    if normalized.starts_with("once_per_turn") {
        let kind = if normalized.contains("soft") { OptKind::Soft } else { OptKind::Hard };
        Ok(Frequency::OncePerTurn(kind))
    } else if normalized.starts_with("once_per_duel") {
        Ok(Frequency::OncePerDuel)
    } else if normalized.starts_with("twice_per_turn") {
        Ok(Frequency::TwicePerTurn)
    } else {
        Err(V2ParseError::UnknownRule(format!("frequency: {}", text)))
    }
}

// ── Passive Block ───────────────────────────────────────────

fn parse_passive_block(pair: Pair<Rule>) -> Result<Passive, V2ParseError> {
    let mut inner = pair.into_inner();
    let name = strip_quotes(inner.next().unwrap().as_str());

    let mut passive = Passive {
        name,
        scope: None,
        target: None,
        condition: None,
        modifiers: vec![],
        grants: vec![],
        negate_effects: false,
        set_atk: None,
        set_def: None,
    };

    for item in inner {
        let text = normalize_ws(item.as_str());
        let sub: Vec<Pair<Rule>> = item.into_inner().collect();

        // Literal-only items (no sub-rules)
        if text.starts_with("scope") {
            passive.scope = Some(if text.contains("field") { Scope::Field } else { Scope::Self_ });
        } else if text == "negate_effects" {
            passive.negate_effects = true;
        } else if let Some(pi) = sub.into_iter().next() {
            // Items with named sub-rules
            match pi.as_rule() {
                Rule::modifier_decl => {
                    passive.modifiers.push(parse_modifier_decl(pi)?);
                }
                Rule::grant_decl => {
                    let ga = pi.into_inner().next().unwrap();
                    passive.grants.push(parse_grant_ability(ga.as_str().trim())?);
                }
                Rule::selector => {
                    passive.target = Some(parse_selector(pi)?);
                }
                Rule::condition_expr => {
                    passive.condition = Some(parse_condition(pi)?);
                }
                Rule::expr => {
                    if text.starts_with("set_atk") {
                        passive.set_atk = Some(parse_expr(pi)?);
                    } else {
                        passive.set_def = Some(parse_expr(pi)?);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(passive)
}

fn parse_modifier_decl(pair: Pair<Rule>) -> Result<Modifier, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let mut inner = pair.into_inner();
    let stat = parse_stat_name(inner.next().unwrap().as_str().trim())?;
    let positive = text.contains('+');
    let expr = parse_expr(inner.next().unwrap())?;
    Ok(Modifier { stat, positive, value: expr })
}

// ── Restriction Block ───────────────────────────────────────

fn parse_restriction_block(pair: Pair<Rule>) -> Result<Restriction, V2ParseError> {
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    let mut name = None;
    let mut start = 0;

    // First inner might be a string (name) or restriction_item
    if !inner.is_empty() && inner[0].as_rule() == Rule::string {
        name = Some(strip_quotes(inner[0].as_str()));
        start = 1;
    }

    let mut restriction = Restriction {
        name,
        apply_to: None,
        target: None,
        abilities: vec![],
        duration: None,
        trigger: None,
        condition: None,
    };

    for item in &inner[start..] {
        let text = normalize_ws(item.as_str());
        let item_inner: Vec<Pair<Rule>> = item.clone().into_inner().collect();

        if text.starts_with("apply_to") {
            if let Some(p) = item_inner.first() {
                restriction.apply_to = Some(parse_player_who(p.as_str().trim())?);
            }
        } else if text.starts_with("target") {
            if let Some(p) = item_inner.first() {
                restriction.target = Some(parse_selector(p.clone())?);
            }
        } else if text.starts_with("duration") {
            if let Some(p) = item_inner.first() {
                restriction.duration = Some(parse_duration(p.as_str().trim())?);
            }
        } else if text.starts_with("trigger") {
            if let Some(p) = item_inner.first() {
                restriction.trigger = Some(parse_trigger(p.clone())?);
            }
        } else if text.starts_with("condition") {
            if let Some(p) = item_inner.first() {
                restriction.condition = Some(parse_condition(p.clone())?);
            }
        } else {
            // Try as grant_ability
            if let Ok(ga) = parse_grant_ability(&text) {
                restriction.abilities.push(ga);
            }
        }
    }
    Ok(restriction)
}

// ── Replacement Block ───────────────────────────────────────

fn parse_replacement_block(pair: Pair<Rule>) -> Result<Replacement, V2ParseError> {
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    let mut name = None;
    let mut start = 0;

    if !inner.is_empty() && inner[0].as_rule() == Rule::string {
        name = Some(strip_quotes(inner[0].as_str()));
        start = 1;
    }

    let mut instead_of = None;
    let mut actions = Vec::new();
    let mut condition = None;

    for item in &inner[start..] {
        let text = normalize_ws(item.as_str());
        let item_inner: Vec<Pair<Rule>> = item.clone().into_inner().collect();

        if text.starts_with("instead_of") {
            if let Some(p) = item_inner.first() {
                instead_of = Some(parse_replaceable_event(p.as_str().trim())?);
            }
        } else if text.starts_with("do") {
            for p in &item_inner {
                if p.as_rule() == Rule::action {
                    actions.push(parse_action(p.clone())?);
                }
            }
        } else if text.starts_with("condition") {
            if let Some(p) = item_inner.first() {
                condition = Some(parse_condition(p.clone())?);
            }
        }
    }

    Ok(Replacement {
        name,
        instead_of: instead_of.ok_or(V2ParseError::MissingField("instead_of"))?,
        actions,
        condition,
    })
}

// ── Choose Block ────────────────────────────────────────────

fn parse_choose_block(pair: Pair<Rule>) -> Result<ChooseBlock, V2ParseError> {
    let mut options = Vec::new();
    for p in pair.into_inner() {
        if p.as_rule() == Rule::option_block {
            options.push(parse_option_block(p)?);
        }
    }
    Ok(ChooseBlock { options })
}

fn parse_option_block(pair: Pair<Rule>) -> Result<ChooseOption, V2ParseError> {
    let mut inner = pair.into_inner();
    let label = strip_quotes(inner.next().unwrap().as_str());

    let mut option = ChooseOption {
        label,
        target: None,
        cost: vec![],
        trigger: None,
        resolve: vec![],
    };

    for item in inner {
        let sub = item.into_inner().next().unwrap();
        match sub.as_rule() {
            Rule::target_decl => option.target = Some(parse_target_decl(sub)?),
            Rule::cost_block => option.cost = parse_cost_block(sub)?,
            Rule::trigger_decl => {
                let te = sub.into_inner().next().unwrap();
                option.trigger = Some(parse_trigger(te)?);
            }
            Rule::resolve_block => option.resolve = parse_action_list(sub)?,
            _ => {}
        }
    }
    Ok(option)
}

// ── Target Declaration ──────────────────────────────────────

fn parse_target_decl(pair: Pair<Rule>) -> Result<TargetDecl, V2ParseError> {
    let mut inner = pair.into_inner();
    let selector = parse_selector(inner.next().unwrap())?;
    let binding = inner.next()
        .filter(|p| p.as_rule() == Rule::ident || p.as_rule() == Rule::binding)
        .map(|p| {
            if p.as_rule() == Rule::binding {
                p.into_inner().next().unwrap().as_str().to_string()
            } else {
                p.as_str().to_string()
            }
        });
    Ok(TargetDecl { selector, binding })
}

// ── Costs ───────────────────────────────────────────────────

fn parse_cost_block(pair: Pair<Rule>) -> Result<Vec<CostAction>, V2ParseError> {
    let mut costs = Vec::new();
    for p in pair.into_inner() {
        if p.as_rule() == Rule::cost_action {
            costs.push(parse_cost_action(p)?);
        }
    }
    Ok(costs)
}

fn parse_cost_action(pair: Pair<Rule>) -> Result<CostAction, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    if text.starts_with("pay_lp") {
        let expr = parse_expr(inner.into_iter().next().unwrap())?;
        Ok(CostAction::PayLp(expr))
    } else if text.starts_with("discard") {
        let mut it = inner.into_iter();
        let sel = parse_selector(it.next().unwrap())?;
        let binding = it.next()
            .filter(|p| p.as_rule() == Rule::binding)
            .map(|p| p.into_inner().next().unwrap().as_str().to_string());
        Ok(CostAction::Discard(sel, binding))
    } else if text.starts_with("tribute") {
        let mut it = inner.into_iter();
        let sel = parse_selector(it.next().unwrap())?;
        let binding = it.next()
            .filter(|p| p.as_rule() == Rule::binding)
            .map(|p| p.into_inner().next().unwrap().as_str().to_string());
        Ok(CostAction::Tribute(sel, binding))
    } else if text.starts_with("banish") {
        let mut it = inner.into_iter();
        let sel = parse_selector(it.next().unwrap())?;
        let zone = it.next()
            .filter(|p| p.as_rule() == Rule::zone)
            .map(|p| parse_zone(p.as_str().trim()))
            .transpose()?;
        let binding = it.next()
            .filter(|p| p.as_rule() == Rule::binding)
            .map(|p| p.into_inner().next().unwrap().as_str().to_string());
        Ok(CostAction::Banish(sel, zone, binding))
    } else if text.starts_with("send") {
        let mut it = inner.into_iter();
        let sel = parse_selector(it.next().unwrap())?;
        let zone = parse_zone(it.next().unwrap().as_str().trim())?;
        let binding = it.next()
            .filter(|p| p.as_rule() == Rule::binding)
            .map(|p| p.into_inner().next().unwrap().as_str().to_string());
        Ok(CostAction::Send(sel, zone, binding))
    } else if text.starts_with("detach") {
        let mut it = inner.into_iter();
        let count = it.next().unwrap().as_str().parse::<u32>()
            .map_err(|_| V2ParseError::InvalidValue("detach count".into()))?;
        let sel = match it.next() {
            Some(p) => parse_selector(p)?,
            None => Selector::SelfCard, // "from self" — self is a literal
        };
        Ok(CostAction::Detach(count, sel))
    } else if text.starts_with("remove_counter") {
        let mut it = inner.into_iter();
        let counter_name = strip_quotes(it.next().unwrap().as_str());
        let count = it.next().unwrap().as_str().parse::<u32>()
            .map_err(|_| V2ParseError::InvalidValue("counter count".into()))?;
        // Grammar: "from" ~ ("self" | selector) — when "self" is the literal
        // alternative it produces no inner pair, so fall back to SelfCard.
        let sel = match it.next() {
            Some(p) => parse_selector(p)?,
            None => Selector::SelfCard,
        };
        Ok(CostAction::RemoveCounter(counter_name, count, sel))
    } else if text.starts_with("reveal") {
        let sel = parse_selector(inner.into_iter().next().unwrap())?;
        Ok(CostAction::Reveal(sel))
    } else if text.starts_with("announce") {
        let mut it = inner.into_iter();
        let what = parse_announce_what(it.next().unwrap().as_str().trim())?;
        let binding = it.next()
            .filter(|p| p.as_rule() == Rule::binding)
            .map(|p| p.into_inner().next().unwrap().as_str().to_string());
        Ok(CostAction::Announce(what, binding))
    } else if text == "none" {
        Ok(CostAction::None)
    } else {
        Err(V2ParseError::UnknownRule(format!("cost_action: {}", text)))
    }
}

// ── Selectors ───────────────────────────────────────────────

fn parse_selector(pair: Pair<Rule>) -> Result<Selector, V2ParseError> {
    let text = pair.as_str().trim().to_string();
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    // Keyword selectors (no inner pairs — string literals)
    if inner.is_empty() {
        return match text.as_str() {
            "self" => Ok(Selector::SelfCard),
            "target" => Ok(Selector::Target),
            "equipped_card" => Ok(Selector::EquippedCard),
            "negated_card" => Ok(Selector::NegatedCard),
            "searched" => Ok(Selector::Searched),
            "linked_card" => Ok(Selector::LinkedCard),
            _ => Err(V2ParseError::UnknownRule(format!("selector: {}", text))),
        };
    }

    // Named binding (ident)
    if inner[0].as_rule() == Rule::ident {
        return Ok(Selector::Binding(inner[0].as_str().to_string()));
    }

    // Counted selector: (quantity, card_filter, ...)
    if inner[0].as_rule() == Rule::quantity {
        return parse_counted_selector(inner);
    }

    Err(V2ParseError::UnknownRule(format!("selector: {}", text)))
}

fn parse_counted_selector(inner: Vec<Pair<Rule>>) -> Result<Selector, V2ParseError> {
    let mut quantity = Quantity::Exact(1);
    let mut filter = CardFilter { name: None, kind: CardFilterKind::Card };
    let mut controller = None;
    let mut zone = None;
    let mut position = None;
    let mut where_clause = None;

    for pair in inner {
        match pair.as_rule() {
            Rule::quantity => quantity = parse_quantity(pair)?,
            Rule::card_filter => filter = parse_card_filter(pair)?,
            Rule::controller => controller = Some(parse_controller(pair.as_str().trim())?),
            Rule::zone_filter => zone = Some(parse_zone_filter(pair)?),
            Rule::position_filter => position = Some(parse_position_filter(pair.as_str().trim())?),
            Rule::where_clause => {
                let pred = pair.into_inner().next().unwrap();
                where_clause = Some(parse_predicate(pred)?);
            }
            _ => {}
        }
    }

    Ok(Selector::Counted { quantity, filter, controller, zone, position, where_clause })
}

fn parse_quantity(pair: Pair<Rule>) -> Result<Quantity, V2ParseError> {
    let text = pair.as_str().trim();
    if text == "all" {
        Ok(Quantity::All)
    } else if text.ends_with('+') {
        let n = text.trim_end_matches('+').parse::<u32>()
            .map_err(|_| V2ParseError::InvalidValue("quantity".into()))?;
        Ok(Quantity::AtLeast(n))
    } else {
        let n = text.parse::<u32>()
            .map_err(|_| V2ParseError::InvalidValue("quantity".into()))?;
        Ok(Quantity::Exact(n))
    }
}

fn parse_card_filter(pair: Pair<Rule>) -> Result<CardFilter, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    // Named filter: "Dark Magician" monster
    if let Some(name_pair) = inner.first().filter(|p| p.as_rule() == Rule::string) {
        let name = strip_quotes(name_pair.as_str());
        let kind = if text.ends_with("monster") { CardFilterKind::Monster } else { CardFilterKind::Card };
        return Ok(CardFilter { name: Some(name), kind });
    }

    // Bare keyword filter
    let kind = match text.as_str() {
        "non-tuner monster" => CardFilterKind::NonTunerMonster,
        "tuner monster" => CardFilterKind::TunerMonster,
        "non-token monster" => CardFilterKind::NonTokenMonster,
        "fusion monster" => CardFilterKind::FusionMonster,
        "synchro monster" => CardFilterKind::SynchroMonster,
        "xyz monster" => CardFilterKind::XyzMonster,
        "link monster" => CardFilterKind::LinkMonster,
        "ritual monster" => CardFilterKind::RitualMonster,
        "pendulum monster" => CardFilterKind::PendulumMonster,
        "effect monster" => CardFilterKind::EffectMonster,
        "normal monster" => CardFilterKind::NormalMonster,
        "monster" => CardFilterKind::Monster,
        "spell" => CardFilterKind::Spell,
        "trap" => CardFilterKind::Trap,
        "card" => CardFilterKind::Card,
        _ => return Err(V2ParseError::UnknownRule(format!("card_filter: {}", text))),
    };
    Ok(CardFilter { name: None, kind })
}

fn parse_zone_list(pair: Pair<Rule>) -> Result<Vec<Zone>, V2ParseError> {
    let mut zones = Vec::new();
    for p in pair.into_inner() {
        if p.as_rule() == Rule::zone {
            zones.push(parse_zone(p.as_str().trim())?);
        }
    }
    Ok(zones)
}

fn parse_zone_filter(pair: Pair<Rule>) -> Result<ZoneFilter, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let mut inner = pair.into_inner();

    if text.starts_with("in ") {
        let zone_list_pair = inner.next().unwrap();
        let zones = parse_zone_list(zone_list_pair)?;
        Ok(ZoneFilter::In(zones))
    } else if text.starts_with("from ") {
        let zone_list_pair = inner.next().unwrap();
        let zones = parse_zone_list(zone_list_pair)?;
        Ok(ZoneFilter::From(zones))
    } else if text.starts_with("on ") {
        let owner = if text.contains("your") { FieldOwner::Your }
            else if text.contains("opponent") { FieldOwner::Opponent }
            else { FieldOwner::Either };
        Ok(ZoneFilter::OnField(owner))
    } else {
        Err(V2ParseError::UnknownRule(format!("zone_filter: {}", text)))
    }
}

// ── Predicates ──────────────────────────────────────────────

fn parse_predicate(pair: Pair<Rule>) -> Result<Predicate, V2ParseError> {
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    let mut atoms = Vec::new();
    let mut conjunction = None;

    for p in &inner {
        match p.as_rule() {
            Rule::pred_atom => atoms.push(parse_pred_atom(p.clone())?),
            Rule::conjunction => {
                conjunction = Some(p.as_str().trim().to_string());
            }
            _ => {}
        }
    }

    if atoms.len() == 1 {
        Ok(Predicate::Single(atoms.remove(0)))
    } else if conjunction.as_deref() == Some("or") {
        Ok(Predicate::Or(atoms))
    } else {
        Ok(Predicate::And(atoms))
    }
}

fn parse_pred_atom(pair: Pair<Rule>) -> Result<PredicateAtom, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    // "not" prefix
    if text.starts_with("not ") {
        let sub = inner.into_iter().next().unwrap();
        return Ok(PredicateAtom::Not(Box::new(parse_pred_atom(sub)?)));
    }

    // Parenthesized predicate
    if !inner.is_empty() && inner[0].as_rule() == Rule::predicate {
        let pred = parse_predicate(inner.into_iter().next().unwrap())?;
        return match pred {
            Predicate::Single(a) => Ok(a),
            Predicate::And(atoms) => Ok(PredicateAtom::Not(Box::new(atoms.into_iter().next().unwrap()))),
            _ => Err(V2ParseError::UnknownRule("nested predicate".into())),
        };
    }

    // stat_field compare_op expr
    if !inner.is_empty() && inner[0].as_rule() == Rule::stat_field {
        let field = parse_stat_field(inner[0].as_str().trim())?;
        let op = parse_compare_op(inner[1].as_str().trim())?;
        let expr = parse_expr(inner[2].clone())?;
        return Ok(PredicateAtom::StatCompare(field, op, expr));
    }

    // Keyword predicates
    if !inner.is_empty() && inner[0].as_rule() == Rule::attribute {
        return Ok(PredicateAtom::AttributeIs(parse_attribute(inner[0].as_str().trim())?));
    }
    if !inner.is_empty() && inner[0].as_rule() == Rule::race {
        return Ok(PredicateAtom::RaceIs(parse_race(inner[0].as_str().trim())?));
    }
    if !inner.is_empty() && inner[0].as_rule() == Rule::card_type {
        return Ok(PredicateAtom::TypeIs(parse_card_type(inner[0].as_str().trim())?));
    }
    if !inner.is_empty() && inner[0].as_rule() == Rule::string {
        let s = strip_quotes(inner[0].as_str());
        if text.contains("name") { return Ok(PredicateAtom::NameIs(s)); }
        if text.contains("archetype") { return Ok(PredicateAtom::ArchetypeIs(s)); }
    }

    // Boolean predicates
    match text.as_str() {
        "is_face_up" => Ok(PredicateAtom::IsFaceUp),
        "is_face_down" => Ok(PredicateAtom::IsFaceDown),
        "is_monster" => Ok(PredicateAtom::IsMonster),
        "is_spell" => Ok(PredicateAtom::IsSpell),
        "is_trap" => Ok(PredicateAtom::IsTrap),
        "is_effect" => Ok(PredicateAtom::IsEffect),
        "is_normal" => Ok(PredicateAtom::IsNormal),
        "is_tuner" => Ok(PredicateAtom::IsTuner),
        "is_fusion" => Ok(PredicateAtom::IsFusion),
        "is_synchro" => Ok(PredicateAtom::IsSynchro),
        "is_xyz" => Ok(PredicateAtom::IsXyz),
        "is_link" => Ok(PredicateAtom::IsLink),
        "is_ritual" => Ok(PredicateAtom::IsRitual),
        "is_pendulum" => Ok(PredicateAtom::IsPendulum),
        "is_token" => Ok(PredicateAtom::IsToken),
        "is_flip" => Ok(PredicateAtom::IsFlip),
        _ => Err(V2ParseError::UnknownRule(format!("pred_atom: {}", text))),
    }
}

// ── Conditions ──────────────────────────────────────────────

fn parse_condition(pair: Pair<Rule>) -> Result<Condition, V2ParseError> {
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    let mut atoms = Vec::new();
    let mut conjunction = None;

    for p in &inner {
        match p.as_rule() {
            Rule::condition_atom => atoms.push(parse_condition_atom(p.clone())?),
            Rule::conjunction => {
                conjunction = Some(p.as_str().trim().to_string());
            }
            _ => {}
        }
    }

    if atoms.len() == 1 {
        Ok(Condition::Single(atoms.remove(0)))
    } else if conjunction.as_deref() == Some("or") {
        Ok(Condition::Or(atoms))
    } else {
        Ok(Condition::And(atoms))
    }
}

fn parse_condition_atom(pair: Pair<Rule>) -> Result<ConditionAtom, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    // "not" prefix
    if text.starts_with("not ") {
        let sub = inner.into_iter().next().unwrap();
        return Ok(ConditionAtom::Not(Box::new(parse_condition_atom(sub)?)));
    }

    // self + card_state
    if text.starts_with("self ") && !inner.is_empty() && inner[0].as_rule() == Rule::card_state {
        return Ok(ConditionAtom::SelfState(parse_card_state(inner[0].as_str().trim())?));
    }

    // player controls selector
    if !inner.is_empty() && inner[0].as_rule() == Rule::player_who {
        let who = parse_player_who(inner[0].as_str().trim())?;
        let sel = parse_selector(inner[1].clone())?;
        return Ok(ConditionAtom::Controls(who, sel));
    }

    // no card_filter on field
    if text.starts_with("no ") && !inner.is_empty() && inner[0].as_rule() == Rule::card_filter {
        let filter = parse_card_filter(inner[0].clone())?;
        let owner = if text.contains("your") { FieldOwner::Your }
            else if text.contains("opponent") { FieldOwner::Opponent }
            else { FieldOwner::Either };
        return Ok(ConditionAtom::NoCardsOnField(filter.kind, owner));
    }

    // Comparisons: lp, opponent_lp, hand_size, etc.
    if text.starts_with("lp ") && !inner.is_empty() && inner[0].as_rule() == Rule::compare_op {
        let op = parse_compare_op(inner[0].as_str().trim())?;
        let expr = parse_expr(inner[1].clone())?;
        return Ok(ConditionAtom::LpCompare(op, expr));
    }
    if text.starts_with("opponent_lp") {
        let op = parse_compare_op(inner[0].as_str().trim())?;
        let expr = parse_expr(inner[1].clone())?;
        return Ok(ConditionAtom::OpponentLpCompare(op, expr));
    }
    if text.starts_with("hand_size") {
        let op = parse_compare_op(inner[0].as_str().trim())?;
        let expr = parse_expr(inner[1].clone())?;
        return Ok(ConditionAtom::HandSize(op, expr));
    }
    if text.starts_with("cards_in_gy") {
        let op = parse_compare_op(inner[0].as_str().trim())?;
        let expr = parse_expr(inner[1].clone())?;
        return Ok(ConditionAtom::CardsInGy(op, expr));
    }
    if text.starts_with("cards_in_banished") {
        let op = parse_compare_op(inner[0].as_str().trim())?;
        let expr = parse_expr(inner[1].clone())?;
        return Ok(ConditionAtom::CardsInBanished(op, expr));
    }

    // phase check
    if text.starts_with("phase") && !inner.is_empty() && inner[0].as_rule() == Rule::phase_name {
        return Ok(ConditionAtom::PhaseIs(parse_phase_name(inner[0].as_str().trim())?));
    }

    // chain_includes
    if text.starts_with("chain_includes") {
        let mut cats = Vec::new();
        for p in &inner {
            if p.as_rule() == Rule::category {
                cats.push(parse_category(p.as_str().trim())?);
            }
        }
        return Ok(ConditionAtom::ChainIncludes(cats));
    }

    // has_counter
    if text.starts_with("has_counter") {
        let counter = strip_quotes(inner[0].as_str());
        let mut op: Option<CompareOp> = None;
        let mut threshold: Option<Expr> = None;
        for p in inner.iter().skip(1) {
            match p.as_rule() {
                Rule::compare_op => op = Some(parse_compare_op(p.as_str().trim())?),
                Rule::expr => threshold = Some(parse_expr(p.clone())?),
                _ => {}
            }
        }
        let target = if text.contains("self") { CounterTarget::OnSelf } else { CounterTarget::OnSelector };
        return Ok(ConditionAtom::HasCounter(counter, op, threshold, target));
    }

    // has_flag
    if text.starts_with("has_flag") {
        let flag = strip_quotes(inner[0].as_str());
        return Ok(ConditionAtom::HasFlag(flag));
    }

    // Simple location checks
    match text.as_str() {
        "on_field" => Ok(ConditionAtom::OnField),
        "in_gy" => Ok(ConditionAtom::InGy),
        "in_hand" => Ok(ConditionAtom::InHand),
        "in_banished" => Ok(ConditionAtom::InBanished),
        _ => Err(V2ParseError::UnknownRule(format!("condition_atom: {}", text))),
    }
}

// ── Triggers ────────────────────────────────────────────────

fn parse_trigger(pair: Pair<Rule>) -> Result<Trigger, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    // Keyword-only triggers
    if inner.is_empty() {
        return match text.as_str() {
            "summoned" => Ok(Trigger::Summoned(None)),
            "special_summoned" => Ok(Trigger::SpecialSummoned(None)),
            "normal_summoned" => Ok(Trigger::NormalSummoned),
            "tribute_summoned" => Ok(Trigger::TributeSummoned),
            "flip_summoned" => Ok(Trigger::FlipSummoned),
            "flipped" => Ok(Trigger::Flipped),
            "destroyed" => Ok(Trigger::Destroyed(None)),
            "destroyed_by_battle" => Ok(Trigger::DestroyedByBattle),
            "destroyed_by_effect" => Ok(Trigger::DestroyedByEffect),
            "destroys_by_battle" => Ok(Trigger::DestroysByBattle),
            "leaves_field" => Ok(Trigger::LeavesField),
            "banished" => Ok(Trigger::Banished),
            "attack_declared" => Ok(Trigger::AttackDeclared),
            "opponent_attack_declared" => Ok(Trigger::OpponentAttackDeclared),
            "attacked" => Ok(Trigger::Attacked),
            "battle_damage" => Ok(Trigger::BattleDamage(None)),
            "direct_attack_damage" => Ok(Trigger::DirectAttackDamage),
            "damage_calculation" => Ok(Trigger::DamageCalculation),
            "standby_phase" => Ok(Trigger::StandbyPhase(None)),
            "end_phase" => Ok(Trigger::EndPhase),
            "draw_phase" => Ok(Trigger::DrawPhase),
            "main_phase" => Ok(Trigger::MainPhase),
            "battle_phase" => Ok(Trigger::BattlePhase),
            "summon_attempt" => Ok(Trigger::SummonAttempt),
            "spell_trap_activated" => Ok(Trigger::SpellTrapActivated),
            "opponent_activates" => Ok(Trigger::OpponentActivates(vec![])),
            "chain_link" => Ok(Trigger::ChainLink),
            "targeted" => Ok(Trigger::Targeted),
            "position_changed" => Ok(Trigger::PositionChanged),
            "control_changed" => Ok(Trigger::ControlChanged),
            "equipped" => Ok(Trigger::Equipped),
            "unequipped" => Ok(Trigger::Unequipped),
            _ => Err(V2ParseError::UnknownRule(format!("trigger: {}", text))),
        };
    }

    // Triggers with qualifiers
    if text.starts_with("summoned") && !text.starts_with("summon_attempt") {
        let method = inner.into_iter()
            .find(|p| p.as_rule() == Rule::summon_qualifier)
            .map(|sq| parse_summon_method(sq.into_inner().next().unwrap().as_str().trim()))
            .transpose()?;
        return Ok(Trigger::Summoned(method));
    }

    if text.starts_with("special_summoned") {
        let method = inner.into_iter()
            .find(|p| p.as_rule() == Rule::summon_qualifier)
            .map(|sq| parse_summon_method(sq.into_inner().next().unwrap().as_str().trim()))
            .transpose()?;
        return Ok(Trigger::SpecialSummoned(method));
    }

    if text.starts_with("destroyed ") {
        let qual = inner.into_iter()
            .find(|p| p.as_rule() == Rule::destroy_qualifier)
            .map(|dq| {
                let dtext = normalize_ws(dq.as_str());
                if dtext.contains("battle") { Ok(DestroyBy::Battle) }
                else if dtext.contains("card_effect") { Ok(DestroyBy::CardEffect) }
                else { Ok(DestroyBy::Effect) }
            })
            .transpose()?;
        return Ok(Trigger::Destroyed(qual));
    }

    if text.starts_with("sent_to") {
        let mut it = inner.into_iter();
        let zone = parse_zone(it.next().unwrap().as_str().trim())?;
        let from = it.next()
            .filter(|p| p.as_rule() == Rule::from_qualifier)
            .map(|fq| parse_zone(fq.into_inner().next().unwrap().as_str().trim()))
            .transpose()?;
        return Ok(Trigger::SentTo(zone, from));
    }

    if text.starts_with("returned_to") {
        let zone = parse_zone(inner[0].as_str().trim())?;
        return Ok(Trigger::ReturnedTo(zone));
    }

    if text.starts_with("battle_damage") {
        let who = inner.into_iter()
            .find(|p| p.as_rule() == Rule::damage_qualifier)
            .map(|dq| {
                let dt = normalize_ws(dq.as_str());
                if dt.contains("you") { Ok(PlayerWho::You) }
                else if dt.contains("opponent") { Ok(PlayerWho::Opponent) }
                else if dt.contains("controller") { Ok(PlayerWho::Controller) }
                else { Ok(PlayerWho::Both) }
            })
            .transpose()?;
        return Ok(Trigger::BattleDamage(who));
    }

    if text.starts_with("standby_phase") {
        let owner = inner.into_iter()
            .find(|p| p.as_rule() == Rule::phase_qualifier)
            .map(|pq| {
                let pt = normalize_ws(pq.as_str());
                if pt.contains("yours") { Ok(PhaseOwner::Yours) }
                else if pt.contains("opponents") { Ok(PhaseOwner::Opponents) }
                else { Ok(PhaseOwner::Either) }
            })
            .transpose()?;
        return Ok(Trigger::StandbyPhase(owner));
    }

    if text.starts_with("opponent_activates") {
        let mut cats = Vec::new();
        for p in &inner {
            if p.as_rule() == Rule::category_list {
                for c in p.clone().into_inner() {
                    if c.as_rule() == Rule::category {
                        cats.push(parse_category(c.as_str().trim())?);
                    }
                }
            }
        }
        return Ok(Trigger::OpponentActivates(cats));
    }

    if text.starts_with("used_as_material") {
        let method = inner.into_iter()
            .find(|p| p.as_rule() == Rule::summon_method)
            .map(|sm| parse_summon_method(sm.as_str().trim()))
            .transpose()?;
        return Ok(Trigger::UsedAsMaterial(method));
    }

    // Custom trigger
    if text.starts_with("custom") {
        let s = strip_quotes(inner[0].as_str());
        return Ok(Trigger::Custom(s));
    }

    Err(V2ParseError::UnknownRule(format!("trigger: {}", text)))
}

// ── Actions ─────────────────────────────────────────────────

fn parse_action_list(pair: Pair<Rule>) -> Result<Vec<Action>, V2ParseError> {
    let mut actions = Vec::new();
    for p in pair.into_inner() {
        if p.as_rule() == Rule::action {
            actions.push(parse_action(p)?);
        }
    }
    Ok(actions)
}

fn parse_action(pair: Pair<Rule>) -> Result<Action, V2ParseError> {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::draw_action => {
            let expr = parse_expr(inner.into_inner().next().unwrap())?;
            Ok(Action::Draw(expr))
        }
        Rule::discard_action => {
            let sel = parse_selector(inner.into_inner().next().unwrap())?;
            Ok(Action::Discard(sel))
        }
        Rule::destroy_action => {
            let sel = parse_selector(inner.into_inner().next().unwrap())?;
            Ok(Action::Destroy(sel))
        }
        Rule::banish_action => {
            let text = normalize_ws(inner.as_str());
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let zone = it.next()
                .filter(|p| p.as_rule() == Rule::zone)
                .map(|p| parse_zone(p.as_str().trim()))
                .transpose()?;
            let face_down = text.contains("face_down");
            Ok(Action::Banish(sel, zone, face_down))
        }
        Rule::send_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let zone = parse_zone(it.next().unwrap().as_str().trim())?;
            Ok(Action::Send(sel, zone))
        }
        Rule::return_action => {
            let text = normalize_ws(inner.as_str());
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let dest = if text.contains("hand") {
                ReturnDest::Hand
            } else if text.contains("extra_deck") {
                ReturnDest::ExtraDeck
            } else {
                let pos = it.next()
                    .filter(|p| p.as_rule() == Rule::deck_position)
                    .map(|p| match p.as_str().trim() {
                        "top" => DeckPosition::Top,
                        "bottom" => DeckPosition::Bottom,
                        _ => DeckPosition::Shuffle,
                    });
                ReturnDest::Deck(pos)
            };
            Ok(Action::Return(sel, dest))
        }
        Rule::search_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let zone = it.next()
                .filter(|p| p.as_rule() == Rule::zone)
                .map(|p| parse_zone(p.as_str().trim()))
                .transpose()?;
            Ok(Action::Search(sel, zone))
        }
        Rule::add_to_hand_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let zone = it.next()
                .filter(|p| p.as_rule() == Rule::zone)
                .map(|p| parse_zone(p.as_str().trim()))
                .transpose()?;
            Ok(Action::AddToHand(sel, zone))
        }
        Rule::special_summon_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let mut zone = None;
            let mut pos = None;
            for p in it {
                match p.as_rule() {
                    Rule::zone => zone = Some(parse_zone(p.as_str().trim())?),
                    Rule::battle_position => pos = Some(parse_battle_position(p.as_str().trim())?),
                    _ => {}
                }
            }
            Ok(Action::SpecialSummon(sel, zone, pos))
        }
        Rule::ritual_summon_action => {
            let mut target: Option<Selector> = None;
            let mut materials: Option<Selector> = None;
            let mut level_op: Option<CompareOp> = None;
            let mut level_expr: Option<Expr> = None;
            for p in inner.into_inner() {
                match p.as_rule() {
                    Rule::selector => {
                        if target.is_none() {
                            target = Some(parse_selector(p)?);
                        } else {
                            materials = Some(parse_selector(p)?);
                        }
                    }
                    Rule::compare_op => level_op = Some(parse_compare_op(p.as_str().trim())?),
                    Rule::expr => level_expr = Some(parse_expr(p)?),
                    _ => {}
                }
            }
            Ok(Action::RitualSummon {
                target: target.ok_or(V2ParseError::MissingField("ritual_summon target"))?,
                materials,
                level_op,
                level_expr,
            })
        }
        Rule::fusion_summon_action => {
            let mut it = inner.into_inner();
            let target = parse_selector(it.next().ok_or(V2ParseError::MissingField("fusion_summon target"))?)?;
            let materials = it.next()
                .filter(|p| p.as_rule() == Rule::selector)
                .map(|p| parse_selector(p))
                .transpose()?;
            Ok(Action::FusionSummon { target, materials })
        }
        Rule::synchro_summon_action => {
            let mut it = inner.into_inner();
            let target = parse_selector(it.next().ok_or(V2ParseError::MissingField("synchro_summon target"))?)?;
            let materials = it.next()
                .filter(|p| p.as_rule() == Rule::selector)
                .map(|p| parse_selector(p))
                .transpose()?;
            Ok(Action::SynchroSummon { target, materials })
        }
        Rule::xyz_summon_action => {
            let mut it = inner.into_inner();
            let target = parse_selector(it.next().ok_or(V2ParseError::MissingField("xyz_summon target"))?)?;
            let materials = it.next()
                .filter(|p| p.as_rule() == Rule::selector)
                .map(|p| parse_selector(p))
                .transpose()?;
            Ok(Action::XyzSummon { target, materials })
        }
        Rule::normal_summon_action => {
            let sel = parse_selector(inner.into_inner().next().unwrap())?;
            Ok(Action::NormalSummon(sel))
        }
        Rule::set_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let zone = it.next()
                .filter(|p| p.as_rule() == Rule::zone)
                .map(|p| parse_zone(p.as_str().trim()))
                .transpose()?;
            Ok(Action::Set(sel, zone))
        }
        Rule::flip_down_action => {
            let sel = parse_selector(inner.into_inner().next().unwrap())?;
            Ok(Action::FlipDown(sel))
        }
        Rule::change_position_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let pos = it.next()
                .filter(|p| p.as_rule() == Rule::battle_position)
                .map(|p| parse_battle_position(p.as_str().trim()))
                .transpose()?;
            Ok(Action::ChangePosition(sel, pos))
        }
        Rule::take_control_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let dur = it.next()
                .filter(|p| p.as_rule() == Rule::duration)
                .map(|p| parse_duration(p.as_str().trim()))
                .transpose()?;
            Ok(Action::TakeControl(sel, dur))
        }
        Rule::equip_action => {
            let mut it = inner.into_inner();
            let card = parse_selector(it.next().unwrap())?;
            let target = parse_selector(it.next().unwrap())?;
            Ok(Action::Equip(card, target))
        }
        Rule::negate_action => {
            let text = normalize_ws(inner.as_str());
            let and_destroy = text.contains("destroy");
            Ok(Action::Negate(and_destroy))
        }
        Rule::negate_effects_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let dur = it.next()
                .filter(|p| p.as_rule() == Rule::duration)
                .map(|p| parse_duration(p.as_str().trim()))
                .transpose()?;
            Ok(Action::NegateEffects(sel, dur))
        }
        Rule::damage_action => {
            let mut it = inner.into_inner();
            let who = parse_player_who(it.next().unwrap().as_str().trim())?;
            let amount = parse_expr(it.next().unwrap())?;
            Ok(Action::Damage(who, amount))
        }
        Rule::gain_lp_action => {
            let expr = parse_expr(inner.into_inner().next().unwrap())?;
            Ok(Action::GainLp(expr))
        }
        Rule::pay_lp_action => {
            let expr = parse_expr(inner.into_inner().next().unwrap())?;
            Ok(Action::PayLp(expr))
        }
        Rule::modify_stat_action => {
            let text = normalize_ws(inner.as_str());
            let mut it = inner.into_inner();
            let stat = parse_stat_name(it.next().unwrap().as_str().trim())?;
            let sel = parse_selector(it.next().unwrap())?;
            let positive = text.contains('+');
            let expr = parse_expr(it.next().unwrap())?;
            let dur = it.next()
                .filter(|p| p.as_rule() == Rule::duration)
                .map(|p| parse_duration(p.as_str().trim()))
                .transpose()?;
            Ok(Action::ModifyStat(stat, sel, !positive, expr, dur))
        }
        Rule::set_stat_action => {
            let mut it = inner.into_inner();
            let stat = parse_stat_name(it.next().unwrap().as_str().trim())?;
            let sel = parse_selector(it.next().unwrap())?;
            let expr = parse_expr(it.next().unwrap())?;
            let dur = it.next()
                .filter(|p| p.as_rule() == Rule::duration)
                .map(|p| parse_duration(p.as_str().trim()))
                .transpose()?;
            Ok(Action::SetStat(stat, sel, expr, dur))
        }
        Rule::change_property_action => parse_change_property(inner),
        Rule::create_token_action => parse_create_token(inner),
        Rule::attach_action => {
            let mut it = inner.into_inner();
            let card = parse_selector(it.next().unwrap())?;
            let target = parse_selector(it.next().unwrap())?;
            Ok(Action::Attach(card, target))
        }
        Rule::detach_action => {
            let mut it = inner.into_inner();
            let count = it.next().unwrap().as_str().parse::<u32>()
                .map_err(|_| V2ParseError::InvalidValue("detach count".into()))?;
            let sel = parse_selector(it.next().unwrap())?;
            Ok(Action::Detach(count, sel))
        }
        Rule::counter_action => {
            let text = normalize_ws(inner.as_str());
            let mut it = inner.into_inner();
            let name = strip_quotes(it.next().unwrap().as_str());
            let count = it.next().unwrap().as_str().parse::<u32>()
                .map_err(|_| V2ParseError::InvalidValue("counter count".into()))?;
            let sel = parse_selector(it.next().unwrap())?;
            if text.starts_with("place") {
                Ok(Action::PlaceCounter(name, count, sel))
            } else {
                Ok(Action::RemoveCounter(name, count, sel))
            }
        }
        Rule::mill_action => {
            let mut it = inner.into_inner();
            let amount = parse_expr(it.next().unwrap())?;
            let owner = it.next().map(|p| {
                let t = p.as_str().trim();
                if t.contains("opponent") { DeckOwner::Opponents } else { DeckOwner::Yours }
            });
            Ok(Action::Mill(amount, owner))
        }
        Rule::excavate_action => {
            let text = normalize_ws(inner.as_str());
            let mut it = inner.into_inner();
            let amount = parse_expr(it.next().unwrap())?;
            let owner = if text.contains("opponent") { DeckOwner::Opponents } else { DeckOwner::Yours };
            Ok(Action::Excavate(amount, owner))
        }
        Rule::reveal_action => {
            let sel = parse_selector(inner.into_inner().next().unwrap())?;
            Ok(Action::Reveal(sel))
        }
        Rule::look_at_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let zone = it.next()
                .filter(|p| p.as_rule() == Rule::zone)
                .map(|p| parse_zone(p.as_str().trim()))
                .transpose()?;
            Ok(Action::LookAt(sel, zone))
        }
        Rule::shuffle_action => {
            let text = normalize_ws(inner.as_str());
            let owner = if text.contains("opponents") { Some(DeckOwner::Opponents) }
                else if text.contains("yours") || text.contains("both") { Some(DeckOwner::Yours) }
                else { None };
            Ok(Action::ShuffleDeck(owner))
        }
        Rule::announce_action => {
            let mut it = inner.into_inner();
            let what = parse_announce_what(it.next().unwrap().as_str().trim())?;
            let binding = it.next()
                .filter(|p| p.as_rule() == Rule::binding)
                .map(|p| p.into_inner().next().unwrap().as_str().to_string());
            Ok(Action::Announce(what, binding))
        }
        Rule::coin_flip_action => {
            let mut heads = Vec::new();
            let mut tails = Vec::new();
            let mut in_heads = true;
            for p in inner.into_inner() {
                if p.as_rule() == Rule::action {
                    if in_heads { heads.push(parse_action(p)?); }
                    else { tails.push(parse_action(p)?); }
                } else {
                    // Switch from heads to tails on encountering non-action
                    if !heads.is_empty() { in_heads = false; }
                }
            }
            Ok(Action::CoinFlip { heads, tails })
        }
        Rule::dice_roll_action => {
            let mut actions = Vec::new();
            for p in inner.into_inner() {
                if p.as_rule() == Rule::action {
                    actions.push(parse_action(p)?);
                }
            }
            Ok(Action::DiceRoll(actions))
        }
        Rule::grant_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let ability = parse_grant_ability(it.next().unwrap().as_str().trim())?;
            let dur = it.next()
                .filter(|p| p.as_rule() == Rule::duration)
                .map(|p| parse_duration(p.as_str().trim()))
                .transpose()?;
            Ok(Action::Grant(sel, ability, dur))
        }
        Rule::link_action => {
            let mut it = inner.into_inner();
            let a = parse_selector(it.next().unwrap())?;
            let b = parse_selector(it.next().unwrap())?;
            Ok(Action::LinkTo(a, b))
        }
        Rule::if_action => {
            let mut it = inner.into_inner();
            let cond = parse_condition(it.next().unwrap())?;
            let mut then_actions = Vec::new();
            let mut else_actions = Vec::new();
            let mut in_else = false;
            for p in it {
                if p.as_rule() == Rule::action {
                    if in_else { else_actions.push(parse_action(p)?); }
                    else { then_actions.push(parse_action(p)?); }
                } else {
                    if !then_actions.is_empty() { in_else = true; }
                }
            }
            Ok(Action::If {
                condition: cond,
                then: then_actions,
                otherwise: else_actions,
            })
        }
        Rule::for_each_action => {
            let mut it = inner.into_inner();
            let sel = parse_selector(it.next().unwrap())?;
            let zone = parse_zone(it.next().unwrap().as_str().trim())?;
            let mut body = Vec::new();
            for p in it {
                if p.as_rule() == Rule::action {
                    body.push(parse_action(p)?);
                }
            }
            Ok(Action::ForEach { selector: sel, zone, body })
        }
        Rule::choose_action => {
            let block = parse_choose_block(inner)?;
            Ok(Action::Choose(block))
        }
        Rule::delayed_action => {
            let mut it = inner.into_inner();
            let phase = parse_phase_name(it.next().unwrap().as_str().trim())?;
            let mut body = Vec::new();
            for p in it {
                if p.as_rule() == Rule::action {
                    body.push(parse_action(p)?);
                }
            }
            Ok(Action::Delayed { until: phase, body })
        }
        Rule::and_if_you_do_action => {
            let mut actions = Vec::new();
            for p in inner.into_inner() {
                if p.as_rule() == Rule::action {
                    actions.push(parse_action(p)?);
                }
            }
            Ok(Action::AndIfYouDo(actions))
        }
        Rule::then_action => {
            let mut actions = Vec::new();
            for p in inner.into_inner() {
                if p.as_rule() == Rule::action {
                    actions.push(parse_action(p)?);
                }
            }
            Ok(Action::Then(actions))
        }
        Rule::also_action => {
            let mut actions = Vec::new();
            for p in inner.into_inner() {
                if p.as_rule() == Rule::action {
                    actions.push(parse_action(p)?);
                }
            }
            Ok(Action::Also(actions))
        }
        Rule::install_watcher_action => {
            let mut it = inner.into_inner();
            let name = strip_quotes(it.next().unwrap().as_str());
            let mut event = None;
            let mut duration = Duration::EndOfTurn;
            let mut check = Vec::new();
            for p in it {
                match p.as_rule() {
                    Rule::trigger_expr => event = Some(parse_trigger(p)?),
                    Rule::duration => duration = parse_duration(p.as_str().trim())?,
                    Rule::action => check.push(parse_action(p)?),
                    _ => {}
                }
            }
            Ok(Action::InstallWatcher {
                name,
                event: event.ok_or(V2ParseError::MissingField("watcher event"))?,
                duration,
                check,
            })
        }
        _ => Err(V2ParseError::UnknownRule(format!("action: {:?}", inner.as_rule()))),
    }
}

fn parse_change_property(pair: Pair<Rule>) -> Result<Action, V2ParseError> {
    let text = normalize_ws(pair.as_str());
    let mut it = pair.into_inner();

    if text.starts_with("change_level") {
        let sel = parse_selector(it.next().unwrap())?;
        let expr = parse_expr(it.next().unwrap())?;
        Ok(Action::ChangeLevel(sel, expr))
    } else if text.starts_with("change_rank") {
        let sel = parse_selector(it.next().unwrap())?;
        let expr = parse_expr(it.next().unwrap())?;
        Ok(Action::ChangeLevel(sel, expr)) // reuse ChangeLevel for rank
    } else if text.starts_with("change_attribute") {
        let sel = parse_selector(it.next().unwrap())?;
        let attr = parse_attribute(it.next().unwrap().as_str().trim())?;
        Ok(Action::ChangeAttribute(sel, attr))
    } else if text.starts_with("change_race") {
        let sel = parse_selector(it.next().unwrap())?;
        let race = parse_race(it.next().unwrap().as_str().trim())?;
        Ok(Action::ChangeRace(sel, race))
    } else if text.starts_with("change_name") {
        let sel = parse_selector(it.next().unwrap())?;
        let name = strip_quotes(it.next().unwrap().as_str());
        let dur = it.next()
            .filter(|p| p.as_rule() == Rule::duration)
            .map(|p| parse_duration(p.as_str().trim()))
            .transpose()?;
        Ok(Action::ChangeName(sel, name, dur))
    } else if text.starts_with("set_scale") {
        let sel = parse_selector(it.next().unwrap())?;
        let expr = parse_expr(it.next().unwrap())?;
        Ok(Action::SetScale(sel, expr))
    } else {
        Err(V2ParseError::UnknownRule(format!("change_property: {}", text)))
    }
}

fn parse_create_token(pair: Pair<Rule>) -> Result<Action, V2ParseError> {
    let mut token = TokenSpec {
        name: None,
        attribute: None,
        race: None,
        level: None,
        atk: StatVal::Number(0),
        def: StatVal::Number(0),
        count: 1,
        position: None,
        restriction: None,
    };

    for item in pair.into_inner() {
        let text = normalize_ws(item.as_str());
        let field = text.split(':').next().unwrap_or("").trim();
        let mut item_inner = item.into_inner();

        match field {
            "name" => token.name = Some(strip_quotes(item_inner.next().unwrap().as_str())),
            "attribute" => token.attribute = Some(parse_attribute(item_inner.next().unwrap().as_str().trim())?),
            "race" => token.race = Some(parse_race(item_inner.next().unwrap().as_str().trim())?),
            "level" => token.level = Some(item_inner.next().unwrap().as_str().parse::<u32>()
                .map_err(|_| V2ParseError::InvalidValue("token level".into()))?),
            "atk" => token.atk = parse_stat_val(item_inner.next().unwrap().as_str().trim())?,
            "def" => token.def = parse_stat_val(item_inner.next().unwrap().as_str().trim())?,
            "count" => token.count = item_inner.next().unwrap().as_str().parse::<u32>()
                .map_err(|_| V2ParseError::InvalidValue("token count".into()))?,
            "position" => token.position = Some(parse_battle_position(item_inner.next().unwrap().as_str().trim())?),
            _ => {
                // Restriction block inside token
                if text.starts_with("restriction") {
                    // Re-parse from the item pair — but we already consumed it.
                    // For now, skip token restrictions.
                }
            }
        }
    }

    Ok(Action::CreateToken(token))
}

// ── Expressions ─────────────────────────────────────────────

fn parse_expr(pair: Pair<Rule>) -> Result<Expr, V2ParseError> {
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    if inner.is_empty() {
        return Err(V2ParseError::MissingField("expression"));
    }

    let mut expr = parse_expr_atom(inner[0].clone())?;

    let mut i = 1;
    while i + 1 < inner.len() {
        let op = parse_binop(inner[i].as_str().trim())?;
        let right = parse_expr_atom(inner[i + 1].clone())?;
        expr = Expr::BinOp {
            left: Box::new(expr),
            op,
            right: Box::new(right),
        };
        i += 2;
    }

    Ok(expr)
}

fn parse_expr_atom(pair: Pair<Rule>) -> Result<Expr, V2ParseError> {
    let text = pair.as_str().trim();
    let inner: Vec<Pair<Rule>> = pair.into_inner().collect();

    // "half" keyword (no inner pairs)
    if text == "half" {
        return Ok(Expr::Half);
    }

    if inner.is_empty() {
        // Shouldn't happen for anything but "half"
        return Err(V2ParseError::UnknownRule(format!("expr_atom: {}", text)));
    }

    let first = &inner[0];
    match first.as_rule() {
        Rule::expr => parse_expr(first.clone()),
        Rule::count_expr => {
            let sel = parse_selector(first.clone().into_inner().next().unwrap())?;
            Ok(Expr::Count(Box::new(sel)))
        }
        Rule::stat_ref => {
            // Could be StatRef (self.atk) or BindingRef (tributed.level)
            let has_ident = first.clone().into_inner().any(|p| p.as_rule() == Rule::ident);
            let stat_inner: Vec<Pair<Rule>> = first.clone().into_inner().collect();
            let field_pair = stat_inner.iter().find(|p| p.as_rule() == Rule::stat_field).unwrap();
            let field = parse_stat_field(field_pair.as_str().trim())?;

            if has_ident {
                let ident = stat_inner.iter().find(|p| p.as_rule() == Rule::ident).unwrap();
                Ok(Expr::BindingRef(ident.as_str().to_string(), field))
            } else {
                let entity = first.as_str().split('.').next().unwrap().trim();
                Ok(Expr::StatRef(entity.to_string(), field))
            }
        }
        Rule::binding_ref => {
            // Unreachable in practice (stat_ref catches it), but handle anyway
            let br_inner: Vec<Pair<Rule>> = first.clone().into_inner().collect();
            let ident = br_inner.iter().find(|p| p.as_rule() == Rule::ident).unwrap();
            let field = br_inner.iter().find(|p| p.as_rule() == Rule::stat_field).unwrap();
            Ok(Expr::BindingRef(ident.as_str().to_string(), parse_stat_field(field.as_str().trim())?))
        }
        Rule::player_ref => {
            match first.as_str().trim() {
                "your_lp" => Ok(Expr::PlayerLp(LpOwner::Your)),
                "opponent_lp" => Ok(Expr::PlayerLp(LpOwner::Opponent)),
                "controller_lp" => Ok(Expr::PlayerLp(LpOwner::Controller)),
                _ => Err(V2ParseError::UnknownRule(format!("player_ref: {}", first.as_str()))),
            }
        }
        Rule::unsigned => {
            let val = first.as_str().parse::<i32>()
                .map_err(|_| V2ParseError::InvalidValue(first.as_str().into()))?;
            Ok(Expr::Literal(val))
        }
        _ => Err(V2ParseError::UnknownRule(format!("expr_atom: {:?}", first.as_rule()))),
    }
}

// ── Simple Enum Parsers ─────────────────────────────────────

fn parse_card_type(text: &str) -> Result<CardType, V2ParseError> {
    match text {
        "Normal Monster" => Ok(CardType::NormalMonster),
        "Effect Monster" => Ok(CardType::EffectMonster),
        "Ritual Monster" => Ok(CardType::RitualMonster),
        "Fusion Monster" => Ok(CardType::FusionMonster),
        "Synchro Monster" => Ok(CardType::SynchroMonster),
        "Xyz Monster" => Ok(CardType::XyzMonster),
        "Link Monster" => Ok(CardType::LinkMonster),
        "Pendulum Monster" => Ok(CardType::PendulumMonster),
        "Tuner" => Ok(CardType::Tuner),
        "Synchro Tuner" => Ok(CardType::SynchroTuner),
        "Flip" => Ok(CardType::Flip),
        "Gemini" => Ok(CardType::Gemini),
        "Union" => Ok(CardType::Union),
        "Spirit" => Ok(CardType::Spirit),
        "Toon" => Ok(CardType::Toon),
        "Normal Spell" => Ok(CardType::NormalSpell),
        "Quick-Play Spell" => Ok(CardType::QuickPlaySpell),
        "Continuous Spell" => Ok(CardType::ContinuousSpell),
        "Equip Spell" => Ok(CardType::EquipSpell),
        "Field Spell" => Ok(CardType::FieldSpell),
        "Ritual Spell" => Ok(CardType::RitualSpell),
        "Normal Trap" => Ok(CardType::NormalTrap),
        "Counter Trap" => Ok(CardType::CounterTrap),
        "Continuous Trap" => Ok(CardType::ContinuousTrap),
        _ => Err(V2ParseError::UnknownRule(format!("card_type: {}", text))),
    }
}

fn parse_attribute(text: &str) -> Result<Attribute, V2ParseError> {
    match text {
        "LIGHT" => Ok(Attribute::Light),
        "DARK" => Ok(Attribute::Dark),
        "FIRE" => Ok(Attribute::Fire),
        "WATER" => Ok(Attribute::Water),
        "EARTH" => Ok(Attribute::Earth),
        "WIND" => Ok(Attribute::Wind),
        "DIVINE" => Ok(Attribute::Divine),
        _ => Err(V2ParseError::UnknownRule(format!("attribute: {}", text))),
    }
}

fn parse_race(text: &str) -> Result<Race, V2ParseError> {
    match text {
        "Dragon" => Ok(Race::Dragon),
        "Spellcaster" => Ok(Race::Spellcaster),
        "Zombie" => Ok(Race::Zombie),
        "Warrior" => Ok(Race::Warrior),
        "Beast-Warrior" => Ok(Race::BeastWarrior),
        "Beast" => Ok(Race::Beast),
        "Winged Beast" => Ok(Race::WingedBeast),
        "Fiend" => Ok(Race::Fiend),
        "Fairy" => Ok(Race::Fairy),
        "Insect" => Ok(Race::Insect),
        "Dinosaur" => Ok(Race::Dinosaur),
        "Reptile" => Ok(Race::Reptile),
        "Fish" => Ok(Race::Fish),
        "Sea Serpent" => Ok(Race::SeaSerpent),
        "Aqua" => Ok(Race::Aqua),
        "Pyro" => Ok(Race::Pyro),
        "Thunder" => Ok(Race::Thunder),
        "Rock" => Ok(Race::Rock),
        "Plant" => Ok(Race::Plant),
        "Machine" => Ok(Race::Machine),
        "Psychic" => Ok(Race::Psychic),
        "Divine-Beast" => Ok(Race::DivineBeast),
        "Wyrm" => Ok(Race::Wyrm),
        "Cyberse" => Ok(Race::Cyberse),
        "Illusion" => Ok(Race::Illusion),
        _ => Err(V2ParseError::UnknownRule(format!("race: {}", text))),
    }
}

fn parse_zone(text: &str) -> Result<Zone, V2ParseError> {
    match text {
        "hand" => Ok(Zone::Hand),
        "field" => Ok(Zone::Field),
        "deck" => Ok(Zone::Deck),
        "extra_deck" => Ok(Zone::ExtraDeck),
        "extra_deck_face_up" => Ok(Zone::ExtraDeckFaceUp),
        "gy" | "graveyard" => Ok(Zone::Gy),
        "banished" => Ok(Zone::Banished),
        "monster_zone" => Ok(Zone::MonsterZone),
        "spell_trap_zone" => Ok(Zone::SpellTrapZone),
        "field_zone" => Ok(Zone::FieldZone),
        "pendulum_zone" => Ok(Zone::PendulumZone),
        "extra_monster_zone" => Ok(Zone::ExtraMonsterZone),
        "overlay" => Ok(Zone::Overlay),
        "equipped" => Ok(Zone::Equipped),
        "top_of_deck" => Ok(Zone::TopOfDeck),
        "bottom_of_deck" => Ok(Zone::BottomOfDeck),
        _ => Err(V2ParseError::UnknownRule(format!("zone: {}", text))),
    }
}

fn parse_duration(text: &str) -> Result<Duration, V2ParseError> {
    let normalized = normalize_ws(text);
    match normalized.as_str() {
        "this_turn" => Ok(Duration::ThisTurn),
        "end_of_turn" => Ok(Duration::EndOfTurn),
        "end_phase" => Ok(Duration::EndPhase),
        "end_of_damage_step" => Ok(Duration::EndOfDamageStep),
        "next_standby_phase" => Ok(Duration::NextStandbyPhase),
        "while_on_field" => Ok(Duration::WhileOnField),
        "while_face_up" => Ok(Duration::WhileFaceUp),
        "permanently" => Ok(Duration::Permanently),
        _ => {
            // N_turns pattern
            if normalized.ends_with("_turns") {
                let n = normalized.trim_end_matches("_turns").parse::<u32>()
                    .map_err(|_| V2ParseError::InvalidValue(format!("duration: {}", text)))?;
                Ok(Duration::NTurns(n))
            } else {
                Err(V2ParseError::UnknownRule(format!("duration: {}", text)))
            }
        }
    }
}

fn parse_battle_position(text: &str) -> Result<BattlePosition, V2ParseError> {
    match normalize_ws(text).as_str() {
        "attack_position" => Ok(BattlePosition::Attack),
        "defense_position" => Ok(BattlePosition::Defense),
        "face_down_defense" => Ok(BattlePosition::FaceDownDefense),
        _ => Err(V2ParseError::UnknownRule(format!("battle_position: {}", text))),
    }
}

fn parse_arrow(text: &str) -> Result<Arrow, V2ParseError> {
    match text {
        "top_left" => Ok(Arrow::TopLeft),
        "top" => Ok(Arrow::Top),
        "top_right" => Ok(Arrow::TopRight),
        "left" => Ok(Arrow::Left),
        "right" => Ok(Arrow::Right),
        "bottom_left" => Ok(Arrow::BottomLeft),
        "bottom" => Ok(Arrow::Bottom),
        "bottom_right" => Ok(Arrow::BottomRight),
        _ => Err(V2ParseError::UnknownRule(format!("arrow: {}", text))),
    }
}

fn parse_stat_val(text: &str) -> Result<StatVal, V2ParseError> {
    if text == "?" {
        Ok(StatVal::Unknown)
    } else {
        let v = text.parse::<i32>()
            .map_err(|_| V2ParseError::InvalidValue(format!("stat_val: {}", text)))?;
        Ok(StatVal::Number(v))
    }
}

fn parse_field_target(text: &str) -> Result<FieldTarget, V2ParseError> {
    match text {
        "your_field" => Ok(FieldTarget::YourField),
        "opponent_field" => Ok(FieldTarget::OpponentField),
        "either_field" => Ok(FieldTarget::EitherField),
        _ => Err(V2ParseError::UnknownRule(format!("field_target: {}", text))),
    }
}

fn parse_player_who(text: &str) -> Result<PlayerWho, V2ParseError> {
    match text {
        "you" => Ok(PlayerWho::You),
        "opponent" => Ok(PlayerWho::Opponent),
        "controller" => Ok(PlayerWho::Controller),
        "owner" => Ok(PlayerWho::Owner),
        "summoner" => Ok(PlayerWho::Summoner),
        "both" => Ok(PlayerWho::Both),
        _ => Err(V2ParseError::UnknownRule(format!("player_who: {}", text))),
    }
}

fn parse_controller(text: &str) -> Result<Controller, V2ParseError> {
    let normalized = normalize_ws(text);
    if normalized.starts_with("you") { Ok(Controller::You) }
    else if normalized.starts_with("opponent") { Ok(Controller::Opponent) }
    else { Ok(Controller::Either) }
}

fn parse_position_filter(text: &str) -> Result<PositionFilter, V2ParseError> {
    let normalized = normalize_ws(text);
    match normalized.as_str() {
        "face_up" => Ok(PositionFilter::FaceUp),
        "face_down" => Ok(PositionFilter::FaceDown),
        "in attack_position" => Ok(PositionFilter::AttackPosition),
        "in defense_position" => Ok(PositionFilter::DefensePosition),
        "except self" => Ok(PositionFilter::ExceptSelf),
        _ => Err(V2ParseError::UnknownRule(format!("position_filter: {}", text))),
    }
}

fn parse_stat_field(text: &str) -> Result<StatField, V2ParseError> {
    match text {
        "atk" => Ok(StatField::Atk),
        "def" => Ok(StatField::Def),
        "level" => Ok(StatField::Level),
        "rank" => Ok(StatField::Rank),
        "link" => Ok(StatField::Link),
        "scale" => Ok(StatField::Scale),
        "base_atk" => Ok(StatField::BaseAtk),
        "base_def" => Ok(StatField::BaseDef),
        "original_atk" => Ok(StatField::OriginalAtk),
        "original_def" => Ok(StatField::OriginalDef),
        _ => Err(V2ParseError::UnknownRule(format!("stat_field: {}", text))),
    }
}

fn parse_stat_name(text: &str) -> Result<StatName, V2ParseError> {
    match text {
        "atk" => Ok(StatName::Atk),
        "def" => Ok(StatName::Def),
        _ => Err(V2ParseError::UnknownRule(format!("stat_name: {}", text))),
    }
}

fn parse_compare_op(text: &str) -> Result<CompareOp, V2ParseError> {
    match text {
        ">=" => Ok(CompareOp::Gte),
        "<=" => Ok(CompareOp::Lte),
        "==" => Ok(CompareOp::Eq),
        "!=" => Ok(CompareOp::Neq),
        ">" => Ok(CompareOp::Gt),
        "<" => Ok(CompareOp::Lt),
        _ => Err(V2ParseError::UnknownRule(format!("compare_op: {}", text))),
    }
}

fn parse_binop(text: &str) -> Result<BinOp, V2ParseError> {
    match text {
        "+" => Ok(BinOp::Add),
        "-" => Ok(BinOp::Sub),
        "*" => Ok(BinOp::Mul),
        "/" => Ok(BinOp::Div),
        _ => Err(V2ParseError::UnknownRule(format!("binop: {}", text))),
    }
}

fn parse_grant_ability(text: &str) -> Result<GrantAbility, V2ParseError> {
    let normalized = normalize_ws(text);
    match normalized.as_str() {
        "cannot_attack" => Ok(GrantAbility::CannotAttack),
        "cannot_attack_directly" => Ok(GrantAbility::CannotAttackDirectly),
        "cannot_change_position" => Ok(GrantAbility::CannotChangePosition),
        "cannot_be_destroyed" => Ok(GrantAbility::CannotBeDestroyed(None)),
        "cannot_be_destroyed by battle" => Ok(GrantAbility::CannotBeDestroyed(Some(DestroyBy::Battle))),
        "cannot_be_destroyed by effect" => Ok(GrantAbility::CannotBeDestroyed(Some(DestroyBy::Effect))),
        "cannot_be_targeted" => Ok(GrantAbility::CannotBeTargeted(None)),
        "cannot_be_targeted by spells" => Ok(GrantAbility::CannotBeTargeted(Some(TargetedBy::Spells))),
        "cannot_be_targeted by traps" => Ok(GrantAbility::CannotBeTargeted(Some(TargetedBy::Traps))),
        "cannot_be_targeted by monsters" => Ok(GrantAbility::CannotBeTargeted(Some(TargetedBy::Monsters))),
        "cannot_be_targeted by effects" => Ok(GrantAbility::CannotBeTargeted(Some(TargetedBy::Effects))),
        "cannot_be_targeted by opponent" => Ok(GrantAbility::CannotBeTargeted(Some(TargetedBy::Opponent))),
        "cannot_be_tributed" => Ok(GrantAbility::CannotBeTributed),
        "cannot_be_used_as_material" => Ok(GrantAbility::CannotBeUsedAsMaterial),
        "cannot_activate" => Ok(GrantAbility::CannotActivate(None)),
        "cannot_activate effects" => Ok(GrantAbility::CannotActivate(Some(ActivateWhat::Effects))),
        "cannot_activate spells" => Ok(GrantAbility::CannotActivate(Some(ActivateWhat::Spells))),
        "cannot_activate traps" => Ok(GrantAbility::CannotActivate(Some(ActivateWhat::Traps))),
        "cannot_normal_summon" => Ok(GrantAbility::CannotNormalSummon),
        "cannot_special_summon" => Ok(GrantAbility::CannotSpecialSummon),
        "piercing" => Ok(GrantAbility::Piercing),
        "direct_attack" => Ok(GrantAbility::DirectAttack),
        "double_attack" => Ok(GrantAbility::DoubleAttack),
        "triple_attack" => Ok(GrantAbility::TripleAttack),
        "attack_all_monsters" => Ok(GrantAbility::AttackAllMonsters),
        "must_attack" => Ok(GrantAbility::MustAttack),
        "immune_to_targeting" => Ok(GrantAbility::ImmuneToTargeting),
        s if s.starts_with("unaffected_by") => {
            if s.contains("spells") { Ok(GrantAbility::UnaffectedBy(UnaffectedSource::Spells)) }
            else if s.contains("traps") { Ok(GrantAbility::UnaffectedBy(UnaffectedSource::Traps)) }
            else if s.contains("opponent_effects") { Ok(GrantAbility::UnaffectedBy(UnaffectedSource::OpponentEffects)) }
            else if s.contains("monsters") { Ok(GrantAbility::UnaffectedBy(UnaffectedSource::Monsters)) }
            else { Ok(GrantAbility::UnaffectedBy(UnaffectedSource::Effects)) }
        }
        _ => Err(V2ParseError::UnknownRule(format!("grant_ability: {}", text))),
    }
}

fn parse_card_state(text: &str) -> Result<CardState, V2ParseError> {
    match text {
        "summoned_this_turn" => Ok(CardState::SummonedThisTurn),
        "attacked_this_turn" => Ok(CardState::AttackedThisTurn),
        "flipped_this_turn" => Ok(CardState::FlippedThisTurn),
        "activated_this_turn" => Ok(CardState::ActivatedThisTurn),
        "face_up" => Ok(CardState::FaceUp),
        "face_down" => Ok(CardState::FaceDown),
        "in_attack_position" => Ok(CardState::InAttackPosition),
        "in_defense_position" => Ok(CardState::InDefensePosition),
        _ => Err(V2ParseError::UnknownRule(format!("card_state: {}", text))),
    }
}

fn parse_phase_name(text: &str) -> Result<PhaseName, V2ParseError> {
    match text {
        "draw" => Ok(PhaseName::Draw),
        "standby" => Ok(PhaseName::Standby),
        "main1" => Ok(PhaseName::Main1),
        "battle" => Ok(PhaseName::Battle),
        "main2" => Ok(PhaseName::Main2),
        "end" => Ok(PhaseName::End),
        "damage" => Ok(PhaseName::Damage),
        "damage_calculation" => Ok(PhaseName::DamageCalculation),
        _ => Err(V2ParseError::UnknownRule(format!("phase_name: {}", text))),
    }
}

fn parse_category(text: &str) -> Result<Category, V2ParseError> {
    match text {
        "search" => Ok(Category::Search),
        "special_summon" => Ok(Category::SpecialSummon),
        "send_to_gy" => Ok(Category::SendToGy),
        "add_to_hand" => Ok(Category::AddToHand),
        "draw" => Ok(Category::Draw),
        "banish" => Ok(Category::Banish),
        "destroy" => Ok(Category::Destroy),
        "negate" => Ok(Category::Negate),
        "mill" => Ok(Category::Mill),
        "activate_spell" => Ok(Category::ActivateSpell),
        "activate_trap" => Ok(Category::ActivateTrap),
        "activate_monster_effect" => Ok(Category::ActivateMonsterEffect),
        "normal_summon" => Ok(Category::NormalSummon),
        "fusion_summon" => Ok(Category::FusionSummon),
        "synchro_summon" => Ok(Category::SynchroSummon),
        "xyz_summon" => Ok(Category::XyzSummon),
        "link_summon" => Ok(Category::LinkSummon),
        "ritual_summon" => Ok(Category::RitualSummon),
        "attack_declared" => Ok(Category::AttackDeclared),
        _ => Err(V2ParseError::UnknownRule(format!("category: {}", text))),
    }
}

fn parse_summon_method(text: &str) -> Result<SummonMethod, V2ParseError> {
    match text {
        "normal" => Ok(SummonMethod::Normal),
        "special" => Ok(SummonMethod::Special),
        "flip" => Ok(SummonMethod::Flip),
        "tribute" => Ok(SummonMethod::Tribute),
        "fusion" => Ok(SummonMethod::Fusion),
        "synchro" => Ok(SummonMethod::Synchro),
        "xyz" => Ok(SummonMethod::Xyz),
        "link" => Ok(SummonMethod::Link),
        "ritual" => Ok(SummonMethod::Ritual),
        "pendulum" => Ok(SummonMethod::Pendulum),
        _ => Err(V2ParseError::UnknownRule(format!("summon_method: {}", text))),
    }
}

fn parse_announce_what(text: &str) -> Result<AnnounceWhat, V2ParseError> {
    match text {
        "type" => Ok(AnnounceWhat::Type),
        "attribute" => Ok(AnnounceWhat::Attribute),
        "race" => Ok(AnnounceWhat::Race),
        "level" => Ok(AnnounceWhat::Level),
        "card" => Ok(AnnounceWhat::Card),
        _ => Err(V2ParseError::UnknownRule(format!("announce_what: {}", text))),
    }
}

fn parse_replaceable_event(text: &str) -> Result<ReplaceableEvent, V2ParseError> {
    match text {
        "destroyed_by_battle" => Ok(ReplaceableEvent::DestroyedByBattle),
        "destroyed_by_effect" => Ok(ReplaceableEvent::DestroyedByEffect),
        "destroyed" => Ok(ReplaceableEvent::Destroyed),
        "sent_to_gy" => Ok(ReplaceableEvent::SentToGy),
        "banished" => Ok(ReplaceableEvent::Banished),
        "returned_to_hand" => Ok(ReplaceableEvent::ReturnedToHand),
        "returned_to_deck" => Ok(ReplaceableEvent::ReturnedToDeck),
        "leaves_field" => Ok(ReplaceableEvent::LeavesField),
        _ => Err(V2ParseError::UnknownRule(format!("replaceable_event: {}", text))),
    }
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pot_of_greed() {
        let source = include_str!("../../cards/goat/pot_of_greed.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards.len(), 1);
        let card = &file.cards[0];
        assert_eq!(card.name, "Pot of Greed");
        assert_eq!(card.fields.id, Some(55144522));
        assert_eq!(card.fields.card_types, vec![CardType::NormalSpell]);
        assert_eq!(card.effects.len(), 1);
        assert_eq!(card.effects[0].name, "Draw 2");
        assert_eq!(card.effects[0].speed, Some(1));
        assert_eq!(card.effects[0].resolve.len(), 1);
    }

    #[test]
    fn test_lava_golem() {
        let source = include_str!("../../cards/goat/lava_golem.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards.len(), 1);
        let card = &file.cards[0];
        assert_eq!(card.name, "Lava Golem");
        assert_eq!(card.fields.id, Some(102380));
        assert_eq!(card.fields.attribute, Some(Attribute::Fire));
        assert_eq!(card.fields.race, Some(Race::Fiend));
        assert!(card.summon.is_some());
        let summon = card.summon.as_ref().unwrap();
        assert!(summon.cannot_normal_summon);
        assert!(summon.special_summon_procedure.is_some());
        assert_eq!(card.effects.len(), 1);
        assert!(card.effects[0].mandatory);
        assert_eq!(card.effects[0].who, Some(PlayerWho::Controller));
    }

    #[test]
    fn test_mirror_force() {
        let source = include_str!("../../cards/goat/mirror_force.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards.len(), 1);
        let card = &file.cards[0];
        assert_eq!(card.name, "Mirror Force");
        assert_eq!(card.fields.card_types, vec![CardType::NormalTrap]);
        assert_eq!(card.effects.len(), 1);
        assert_eq!(card.effects[0].speed, Some(2));
        assert_eq!(card.effects[0].resolve.len(), 1);
    }

    #[test]
    fn test_sangan() {
        let source = include_str!("../../cards/goat/sangan.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards.len(), 1);
        let card = &file.cards[0];
        assert_eq!(card.name, "Sangan");
        assert_eq!(card.fields.attribute, Some(Attribute::Dark));
        assert_eq!(card.effects.len(), 1);
        assert!(card.effects[0].mandatory);
        assert!(card.effects[0].frequency.is_some());
        assert_eq!(card.effects[0].resolve.len(), 2);
    }

    #[test]
    fn test_solemn_judgment() {
        let source = include_str!("../../cards/goat/solemn_judgment.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards.len(), 1);
        let card = &file.cards[0];
        assert_eq!(card.name, "Solemn Judgment");
        assert_eq!(card.fields.card_types, vec![CardType::CounterTrap]);
        assert_eq!(card.effects.len(), 1);
        assert_eq!(card.effects[0].speed, Some(3));
        assert_eq!(card.effects[0].cost.len(), 1);
        assert_eq!(card.effects[0].resolve.len(), 1);
    }

    #[test]
    fn test_official_error_report() {
        let dir = match std::fs::read_dir("cards/official") {
            Ok(d) => d,
            Err(_) => return, // skip if dir doesn't exist
        };
        let mut error_samples: std::collections::HashMap<String, (usize, String)> = std::collections::HashMap::new();
        let mut ok = 0;
        let mut fail = 0;
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "ds") { continue; }
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            match parse_v2(&src) {
                Ok(_) => ok += 1,
                Err(e) => {
                    fail += 1;
                    let msg = e.to_string();
                    let key = if let Some(pos) = msg.find("expected") {
                        msg[pos..].lines().next().unwrap_or("?").to_string()
                    } else {
                        "other".to_string()
                    };
                    let entry = error_samples.entry(key).or_insert((0, path.to_string_lossy().to_string()));
                    entry.0 += 1;
                }
            }
        }
        println!("\nofficial parse: {} ok, {} fail", ok, fail);
        let mut sorted: Vec<_> = error_samples.into_iter().collect();
        sorted.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        for (msg, (count, example)) in sorted.iter().take(10) {
            println!("  {:>5} | {} (e.g. {})", count, msg, example);
        }
        // Don't assert — this is informational
    }

    #[test]
    fn test_raigeki() {
        let source = include_str!("../../cards/goat/raigeki.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards[0].name, "Raigeki");
        assert_eq!(file.cards[0].effects[0].resolve.len(), 1);
    }

    #[test]
    fn test_heavy_storm() {
        let source = include_str!("../../cards/goat/heavy_storm.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards[0].name, "Heavy Storm");
        assert_eq!(file.cards[0].effects[0].resolve.len(), 2); // destroy spell + destroy trap
    }

    #[test]
    fn test_book_of_moon() {
        let source = include_str!("../../cards/goat/book_of_moon.ds");
        let file = parse_v2(source).unwrap();
        let card = &file.cards[0];
        assert_eq!(card.name, "Book of Moon");
        assert_eq!(card.fields.card_types, vec![CardType::QuickPlaySpell]);
        assert!(card.effects[0].target.is_some());
        assert_eq!(card.effects[0].speed, Some(2));
    }

    #[test]
    fn test_waboku() {
        let source = include_str!("../../cards/goat/waboku.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards[0].name, "Waboku");
        assert_eq!(file.cards[0].fields.card_types, vec![CardType::NormalTrap]);
        assert_eq!(file.cards[0].effects[0].resolve.len(), 1);
    }

    #[test]
    fn test_graceful_charity() {
        let source = include_str!("../../cards/goat/graceful_charity.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards[0].name, "Graceful Charity");
        assert_eq!(file.cards[0].effects[0].resolve.len(), 2); // draw + discard
    }

    #[test]
    fn test_scapegoat() {
        let source = include_str!("../../cards/goat/scapegoat.ds");
        let file = parse_v2(source).unwrap();
        assert_eq!(file.cards[0].name, "Scapegoat");
        assert_eq!(file.cards[0].effects[0].speed, Some(2));
    }

    #[test]
    fn test_spirit_reaper() {
        let source = include_str!("../../cards/goat/spirit_reaper.ds");
        let file = parse_v2(source).unwrap();
        let card = &file.cards[0];
        assert_eq!(card.name, "Spirit Reaper");
        assert_eq!(card.passives.len(), 1);
        assert_eq!(card.effects.len(), 2);
        assert!(card.effects[0].mandatory);
    }

    #[test]
    fn test_airknight_parshath() {
        let source = include_str!("../../cards/goat/airknight_parshath.ds");
        let file = parse_v2(source).unwrap();
        let card = &file.cards[0];
        assert_eq!(card.name, "Airknight Parshath");
        assert!(card.summon.is_some());
        assert_eq!(card.passives.len(), 1);
        assert_eq!(card.effects.len(), 1);
    }

    #[test]
    fn test_gravekeepers_spy() {
        let source = include_str!("../../cards/goat/gravekeepers_spy.ds");
        let file = parse_v2(source).unwrap();
        let card = &file.cards[0];
        assert_eq!(card.name, "Gravekeeper's Spy");
        assert!(card.fields.card_types.contains(&CardType::Flip));
        assert_eq!(card.effects.len(), 1);
    }

    #[test]
    fn test_jinzo() {
        let source = include_str!("../../cards/goat/jinzo.ds");
        let file = parse_v2(source).unwrap();
        let card = &file.cards[0];
        assert_eq!(card.name, "Jinzo");
        assert!(card.summon.is_some());
        assert_eq!(card.passives.len(), 1);
        assert!(card.passives[0].negate_effects);
    }

    #[test]
    fn test_thestalos() {
        let source = include_str!("../../cards/goat/thestalos.ds");
        let file = parse_v2(source).unwrap();
        let card = &file.cards[0];
        assert_eq!(card.name, "Thestalos the Firestorm Monarch");
        assert!(card.effects[0].mandatory);
        assert_eq!(card.effects[0].resolve.len(), 1);
    }

    #[test]
    fn test_dark_paladin() {
        let source = include_str!("../../cards/goat/dark_paladin.ds");
        let file = parse_v2(source).unwrap();
        let card = &file.cards[0];
        assert_eq!(card.name, "Dark Paladin");
        assert!(card.fields.card_types.contains(&CardType::FusionMonster));
        assert!(card.summon.is_some());
        assert_eq!(card.effects.len(), 1);
        assert_eq!(card.passives.len(), 1);
        assert_eq!(card.effects[0].cost.len(), 1);
    }
}
