// ============================================================
// DuelScript Validator — validator.rs
// Catches illegal card definitions before the engine sees them.
// Run after parsing — validate(&file) returns a Vec<ValidationError>.
// ============================================================

use crate::ast::*;

// ── Public API ────────────────────────────────────────────────

/// Validate an entire parsed file. Returns all errors found across all cards.
pub fn validate(file: &DuelScriptFile) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for card in &file.cards {
        validate_card(card, &mut errors);
    }
    errors
}

/// Validate a single card.
pub fn validate_card(card: &Card, errors: &mut Vec<ValidationError>) {
    let ctx = CardCtx::new(card);

    check_required_fields(&ctx, errors);
    check_stat_ranges(&ctx, errors);
    check_level_tribute_consistency(&ctx, errors);
    check_type_consistency(&ctx, errors);
    check_link_arrows(&ctx, errors);
    check_materials(&ctx, errors);
    check_spell_speeds(&ctx, errors);
    check_once_per_turn_on_spells_traps(&ctx, errors);
    check_once_per_duel(&ctx, errors);
    check_cost_validity(&ctx, errors);
    check_trigger_validity(&ctx, errors);
    check_summon_conditions(&ctx, errors);
    check_replacement_effect_validity(&ctx, errors);
    check_equip_effect_validity(&ctx, errors);
    check_win_condition_validity(&ctx, errors);
    check_counter_system_validity(&ctx, errors);
    check_continuous_effect_validity(&ctx, errors);
    check_flip_effects(&ctx, errors);
    check_flag_references(&ctx, errors);
}

// ── Phase 1B: Flip effects must live on Flip monsters ────────
fn check_flip_effects(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    if ctx.card.flip_effects.is_empty() { return; }
    let is_flip_monster = ctx.card.card_types.iter().any(|t| matches!(t, CardType::Flip));
    if !is_flip_monster {
        errors.push(warn(
            &ctx.card.name,
            "flip_effect block defined on a non-Flip monster — add 'Flip' to the type declaration",
        ));
    }
}

// ── Phase 1A: has_flag should reference a name that set_flag uses ────
fn check_flag_references(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    let mut set_names: std::collections::HashSet<String> = Default::default();
    let mut referenced: Vec<String> = vec![];

    fn walk_actions(actions: &[GameAction], set_names: &mut std::collections::HashSet<String>) {
        for a in actions {
            if let GameAction::SetFlag { name, .. } = a {
                set_names.insert(name.clone());
            }
        }
    }
    fn walk_simple(s: &SimpleCondition, out: &mut Vec<String>) {
        if let SimpleCondition::HasFlag { name, .. } = s {
            out.push(name.clone());
        }
    }
    fn walk_cond(cond: &ConditionExpr, out: &mut Vec<String>) {
        match cond {
            ConditionExpr::Simple(s) => walk_simple(s, out),
            ConditionExpr::And(list) | ConditionExpr::Or(list) => {
                for c in list { walk_simple(c, out); }
            }
        }
    }

    for e in &ctx.card.effects {
        walk_actions(&e.body.on_activate, &mut set_names);
        walk_actions(&e.body.on_resolve, &mut set_names);
        if let Some(c) = &e.body.condition { walk_cond(c, &mut referenced); }
    }
    for fe in &ctx.card.flip_effects {
        walk_actions(&fe.on_activate, &mut set_names);
        walk_actions(&fe.on_resolve, &mut set_names);
        if let Some(c) = &fe.condition { walk_cond(c, &mut referenced); }
    }

    for name in &referenced {
        if !set_names.contains(name) {
            errors.push(warn(
                &ctx.card.name,
                &format!("has_flag \"{}\" referenced but no set_flag \"{}\" in this card — flag may come from another card or is a typo", name, name),
            ));
        }
    }
}

// ── Card Context ──────────────────────────────────────────────

struct CardCtx<'a> {
    card: &'a Card,
    is_monster: bool,
    is_spell: bool,
    is_trap: bool,
    is_extra_deck: bool,
    is_link: bool,
    is_xyz: bool,
    is_pendulum: bool,
    is_normal_monster: bool,
    #[allow(dead_code)]
    is_effect_monster: bool,
}

impl<'a> CardCtx<'a> {
    fn new(card: &'a Card) -> Self {
        let types = &card.card_types;
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
            is_monster,
            is_spell,
            is_trap,
            is_extra_deck,
            is_link:           types.contains(&CardType::LinkMonster),
            is_xyz:            types.contains(&CardType::XyzMonster),
            is_pendulum:       types.contains(&CardType::PendulumMonster),
            is_normal_monster: types.contains(&CardType::NormalMonster),
            is_effect_monster: types.contains(&CardType::EffectMonster),
            card,
        }
    }

    fn name(&self) -> &str { &self.card.name }
}

// ── Checks ────────────────────────────────────────────────────

fn check_required_fields(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    // All cards must have at least one type
    if ctx.card.card_types.is_empty() {
        errors.push(err(ctx.name(), "Card must declare at least one type"));
    }

    // Monsters must have ATK
    if ctx.is_monster && ctx.card.stats.atk.is_none() {
        errors.push(err(ctx.name(), "Monster must declare 'atk'"));
    }

    // Non-link monsters should have DEF (links do not)
    if ctx.is_monster && !ctx.is_link && ctx.card.stats.def.is_none() {
        errors.push(err(ctx.name(), "Non-Link monster must declare 'def'"));
    }

    // Link monsters must NOT have DEF
    if ctx.is_link && ctx.card.stats.def.is_some() {
        errors.push(err(ctx.name(), "Link monsters do not have DEF — remove 'def' declaration"));
    }

    // Monsters must have race
    if ctx.is_monster && ctx.card.race.is_none() {
        errors.push(err(ctx.name(), "Monster must declare 'race'"));
    }

    // Monsters must have attribute
    if ctx.is_monster && ctx.card.attribute.is_none() {
        errors.push(err(ctx.name(), "Monster must declare 'attribute'"));
    }

    // Non-Xyz non-Link monsters must have level
    if ctx.is_monster && !ctx.is_extra_deck && !ctx.is_xyz {
        if ctx.card.level.is_none() {
            errors.push(err(ctx.name(), "Monster must declare 'level' (or 'rank' for Xyz, 'link' for Link)"));
        }
    }

    // Xyz monsters must have rank not level
    if ctx.is_xyz && ctx.card.level.is_some() {
        errors.push(err(ctx.name(), "Xyz monsters use 'rank', not 'level'"));
    }
    if ctx.is_xyz && ctx.card.rank.is_none() {
        errors.push(err(ctx.name(), "Xyz monster must declare 'rank'"));
    }

    // Link monsters must have link rating not level
    if ctx.is_link && ctx.card.level.is_some() {
        errors.push(err(ctx.name(), "Link monsters use 'link', not 'level'"));
    }
    if ctx.is_link && ctx.card.link.is_none() {
        errors.push(err(ctx.name(), "Link monster must declare 'link' rating"));
    }

    // Pendulum must have scale
    if ctx.is_pendulum && ctx.card.scale.is_none() {
        errors.push(err(ctx.name(), "Pendulum monster must declare 'scale'"));
    }

    // Normal monsters should have flavor text
    if ctx.is_normal_monster && ctx.card.flavor.is_none() {
        errors.push(warn(ctx.name(), "Normal monsters should have 'flavor' text"));
    }

    // Normal monsters should have no effects
    if ctx.is_normal_monster && !ctx.card.effects.is_empty() {
        errors.push(err(ctx.name(), "Normal monsters cannot have effect blocks"));
    }
}

fn check_stat_ranges(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    if let Some(StatValue::Number(atk)) = &ctx.card.stats.atk {
        if *atk < 0 {
            errors.push(err(ctx.name(), "ATK cannot be negative"));
        }
        if *atk > 5000 {
            errors.push(warn(ctx.name(), &format!("ATK of {} is unusually high — is this intentional?", atk)));
        }
    }
    if let Some(StatValue::Number(def)) = &ctx.card.stats.def {
        if *def < 0 {
            errors.push(err(ctx.name(), "DEF cannot be negative"));
        }
    }
    if let Some(level) = ctx.card.level {
        if level > 12 {
            errors.push(warn(ctx.name(), &format!("Level {} is unusually high — max standard is 12", level)));
        }
        if level == 0 {
            errors.push(err(ctx.name(), "Level cannot be 0"));
        }
    }
    if let Some(rank) = ctx.card.rank {
        if rank > 13 {
            errors.push(warn(ctx.name(), &format!("Rank {} exceeds known Xyz ranks", rank)));
        }
    }
    if let Some(link) = ctx.card.link {
        if link == 0 {
            errors.push(err(ctx.name(), "Link rating cannot be 0"));
        }
        if link > 8 {
            errors.push(warn(ctx.name(), &format!("Link rating of {} is unusually high", link)));
        }
    }
}

fn check_level_tribute_consistency(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    // Only matters for main deck monsters that aren't special-summon-only
    if !ctx.is_monster || ctx.is_extra_deck {
        return;
    }

    let level = match ctx.card.level {
        Some(l) => l,
        None => return,
    };

    // Check summon_conditions declares the right tribute count
    if !ctx.card.summon_conditions.is_empty() {
        for rule in &ctx.card.summon_conditions {
            if let SummonRule::TributesRequired(n) = rule {
                let expected = expected_tributes(level);
                if let Some(exp) = expected {
                    if *n != exp {
                        errors.push(err(
                            ctx.name(),
                            &format!(
                                "Level {} monster should require {} tribute(s), but declares {}",
                                level, exp, n
                            ),
                        ));
                    }
                }
            }
        }
    } else if level >= 5 && !is_special_summon_only(ctx) {
        // Level 5+ with no summon_condition — warn
        errors.push(warn(
            ctx.name(),
            &format!(
                "Level {} monster should declare 'summon_conditions' with 'tributes_required: {}'",
                level,
                expected_tributes(level).unwrap_or(1)
            ),
        ));
    }
}

fn expected_tributes(level: u32) -> Option<u32> {
    match level {
        1..=4 => None, // No tribute
        5..=6 => Some(1),
        7..=12 => Some(2),
        _ => None,
    }
}

fn is_special_summon_only(ctx: &CardCtx) -> bool {
    ctx.card.summon_conditions.iter().any(|r| matches!(r, SummonRule::SpecialSummonOnly))
}

fn check_type_consistency(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    // Can't be both a spell and a monster
    if ctx.is_monster && ctx.is_spell {
        errors.push(err(ctx.name(), "Card cannot be both a Monster and a Spell type"));
    }
    if ctx.is_monster && ctx.is_trap {
        errors.push(err(ctx.name(), "Card cannot be both a Monster and a Trap type"));
    }
    if ctx.is_spell && ctx.is_trap {
        errors.push(err(ctx.name(), "Card cannot be both a Spell and a Trap type"));
    }

    // Tuner must be a monster
    if ctx.card.card_types.contains(&CardType::Tuner) && !ctx.is_monster {
        errors.push(err(ctx.name(), "'Tuner' subtype requires a monster type"));
    }

    // Pendulum must declare a base monster type. Any of the canonical
    // monster categories satisfies it (Fusion Pendulum, Synchro Pendulum,
    // Xyz Pendulum are all valid card categories in real Yu-Gi-Oh).
    if ctx.is_pendulum {
        let has_base = ctx.card.card_types.iter().any(|t| matches!(t,
            CardType::NormalMonster | CardType::EffectMonster
            | CardType::FusionMonster | CardType::SynchroMonster
            | CardType::XyzMonster | CardType::RitualMonster
        ));
        if !has_base {
            errors.push(err(ctx.name(),
                "Pendulum Monster must also declare a base monster type \
                 (Normal/Effect/Fusion/Synchro/Xyz/Ritual)"));
        }
    }

    // Counter Trap can only be spell speed 3
    for effect in &ctx.card.effects {
        if ctx.card.card_types.contains(&CardType::CounterTrap) {
            if effect.body.speed != SpellSpeed::SpellSpeed3 {
                errors.push(err(
                    ctx.name(),
                    "Counter Trap must use 'speed: spell_speed_3'",
                ));
            }
        }
    }
}

fn check_link_arrows(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    if !ctx.is_link { return; }

    let arrows = &ctx.card.link_arrows;
    let link_rating = ctx.card.link.unwrap_or(0) as usize;

    if arrows.is_empty() {
        errors.push(err(ctx.name(), "Link monster must declare 'link_arrows'"));
        return;
    }

    // Number of arrows must equal link rating
    if arrows.len() != link_rating {
        errors.push(err(
            ctx.name(),
            &format!(
                "Link {} monster must have exactly {} link arrow(s), found {}",
                link_rating, link_rating, arrows.len()
            ),
        ));
    }

    // No duplicate arrows
    let mut seen = std::collections::HashSet::new();
    for arrow in arrows {
        if !seen.insert(format!("{:?}", arrow)) {
            errors.push(err(ctx.name(), &format!("Duplicate link arrow: {:?}", arrow)));
        }
    }
}

fn check_materials(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    // Extra deck monsters (Fusion/Synchro/Xyz/Link) should declare materials.
    // Sprint 57: ritual monsters no longer need materials — the ritual
    // SPELL defines the summoning conditions. Materials on a ritual
    // monster is still accepted for backwards compat but not required.
    if ctx.is_extra_deck && ctx.card.materials.is_none() {
        errors.push(warn(ctx.name(), "Extra deck monster should declare a 'materials' block"));
    }

    // Main deck non-ritual monsters should NOT have materials
    if !ctx.is_extra_deck
        && !ctx.card.card_types.contains(&CardType::RitualMonster)
        && ctx.card.materials.is_some()
    {
        errors.push(err(ctx.name(), "Only Extra Deck and Ritual monsters should declare 'materials'"));
    }

    // Xyz must not require tuner
    if ctx.is_xyz {
        if let Some(mats) = &ctx.card.materials {
            for slot in &mats.slots {
                if let MaterialSlot::Generic(g) = slot {
                    if g.qualifiers.contains(&MaterialQualifier::Tuner) {
                        errors.push(err(ctx.name(), "Xyz monsters cannot require Tuner materials"));
                    }
                }
            }
        }
    }
}

fn check_spell_speeds(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    for effect in &ctx.card.effects {
        let speed = &effect.body.speed;

        // Spell cards can only be spell speed 1 (unless Quick-Play)
        if ctx.is_spell {
            let is_quickplay = ctx.card.card_types.contains(&CardType::QuickPlaySpell);
            let is_counter   = ctx.card.card_types.contains(&CardType::CounterTrap);
            match speed {
                SpellSpeed::SpellSpeed3 if !is_counter => {
                    errors.push(err(ctx.name(), "Only Counter Traps can use spell_speed_3"));
                }
                SpellSpeed::SpellSpeed2 if !is_quickplay => {
                    errors.push(err(ctx.name(), "Only Quick-Play Spells can use spell_speed_2 — regular Spells are spell_speed_1"));
                }
                _ => {}
            }
        }

        // Trap effects are at least spell speed 2
        if ctx.is_trap {
            if *speed == SpellSpeed::SpellSpeed1 {
                errors.push(err(ctx.name(), "Trap effects must be at least spell_speed_2"));
            }
        }

        // Counter traps must be spell speed 3
        if ctx.card.card_types.contains(&CardType::CounterTrap) {
            if *speed != SpellSpeed::SpellSpeed3 {
                errors.push(err(ctx.name(), "Counter Trap effects must be spell_speed_3"));
            }
        }

        // Optional trigger effects MUST explicitly declare timing (when/if)
        // "when" = can miss timing (strict), "if" = cannot miss timing (lenient)
        // This directly affects SEGOC chain ordering in the engine
        if effect.body.optional && effect.body.trigger.is_some()
            && effect.body.speed == SpellSpeed::SpellSpeed1
            && !effect.body.timing_explicit
        {
            errors.push(warn(
                ctx.name(),
                "Optional trigger effect should explicitly declare 'timing: when' or 'timing: if'. \
                 'when' effects can miss timing; 'if' effects cannot. Defaulting to 'when'.",
            ));
        }

        // negate activation in a speed 1 effect is illegal
        for action in &effect.body.on_resolve {
            if let GameAction::Negate { what: Some(NegateTarget::Activation), .. } = action {
                if *speed == SpellSpeed::SpellSpeed1 {
                    errors.push(err(
                        ctx.name(),
                        "'negate activation' requires at least spell_speed_2 — change speed or use 'negate effect'",
                    ));
                }
            }
        }
    }
}

fn check_once_per_turn_on_spells_traps(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    if !ctx.is_spell && !ctx.is_trap { return; }

    for effect in &ctx.card.effects {
        // once_per_duel on a spell/trap is almost always nonsensical
        if effect.body.frequency == Frequency::OncePerDuel {
            errors.push(warn(
                ctx.name(),
                "'once_per_duel' on a Spell/Trap card is unusual — cards leave the field, making this meaningless. Did you mean 'once_per_turn'?",
            ));
        }
    }
}

fn check_once_per_duel(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    // Once per duel + once per turn is contradictory
    for effect in &ctx.card.effects {
        if effect.body.frequency == Frequency::OncePerDuel
            && matches!(effect.body.frequency, Frequency::OncePerTurn(_))
        {
            errors.push(err(ctx.name(), "Effect cannot be both 'once_per_turn' and 'once_per_duel'"));
        }
    }
}

fn check_cost_validity(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    for effect in &ctx.card.effects {
        for cost in &effect.body.cost {
            match cost {
                // Detach is only valid on Xyz monsters
                CostAction::Detach { .. } if !ctx.is_xyz => {
                    errors.push(err(
                        ctx.name(),
                        "'detach overlay_unit' cost is only valid on Xyz monsters",
                    ));
                }
                // Sprint 60: tribute self → compiler auto-injects on_field
                // condition, so no warning needed. The compiler handles it.
                CostAction::Tribute(SelfOrTarget::Self_) => {}
                // Pay LP must be > 0
                CostAction::PayLp(expr) => {
                    if let Expr::Literal(0) = expr {
                        errors.push(err(ctx.name(), "'pay_lp 0' is a no-op — remove or use a positive value"));
                    }
                }
                _ => {}
            }
        }
    }
}

fn condition_implies_on_field(cond: &ConditionExpr) -> bool {
    match cond {
        ConditionExpr::Simple(SimpleCondition::OnField) => true,
        ConditionExpr::And(conditions) | ConditionExpr::Or(conditions) => {
            conditions.iter().any(|c| matches!(c, SimpleCondition::OnField))
        }
        _ => false,
    }
}

fn check_trigger_validity(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    for effect in &ctx.card.effects {
        let Some(trigger) = &effect.body.trigger else { continue };

        match trigger {
            // when_tribute_summoned only valid for monsters
            TriggerExpr::WhenTributeSummoned { .. } if !ctx.is_monster => {
                errors.push(err(ctx.name(), "'when_tribute_summoned' trigger only applies to monsters"));
            }
            // when_destroyed only valid if card can be on the field
            TriggerExpr::WhenDestroyed(_) if ctx.is_spell || ctx.is_trap => {
                // Spells/Traps can be destroyed — this is ok, just unusual
                errors.push(warn(
                    ctx.name(),
                    "'when_destroyed' on a Spell/Trap is unusual — are you sure this is the intended timing?",
                ));
            }
            // when_attacked only valid for monsters
            TriggerExpr::WhenAttacked if !ctx.is_monster => {
                errors.push(err(ctx.name(), "'when_attacked' trigger only applies to monsters"));
            }
            // when_flipped only valid for Flip monsters
            TriggerExpr::WhenFlipped if !ctx.card.card_types.contains(&CardType::Flip) => {
                errors.push(warn(
                    ctx.name(),
                    "'when_flipped' trigger is only standard on Flip monsters — is this intentional?",
                ));
            }
            _ => {}
        }

        // Hand traps: condition must include in_hand
        if is_hand_trap(effect) {
            let has_hand_condition = effect.body.condition.as_ref().map_or(false, |c| {
                condition_contains_zone(c, &Zone::Hand)
            });
            if !has_hand_condition {
                errors.push(warn(
                    ctx.name(),
                    "Effect with 'opponent_activates' trigger should include 'condition: in_hand' for hand trap clarity",
                ));
            }
        }
    }
}

fn is_hand_trap(effect: &Effect) -> bool {
    matches!(&effect.body.trigger, Some(TriggerExpr::OpponentActivates(_)))
        && effect.body.condition.as_ref().map_or(false, |c| condition_contains_zone(c, &Zone::Hand))
}

fn condition_contains_zone(cond: &ConditionExpr, zone: &Zone) -> bool {
    match cond {
        ConditionExpr::Simple(SimpleCondition::InZone(z)) => z == zone,
        ConditionExpr::And(cs) | ConditionExpr::Or(cs) => {
            cs.iter().any(|c| matches!(c, SimpleCondition::InZone(z) if z == zone))
        }
        _ => false,
    }
}

fn check_summon_conditions(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    let sc = &ctx.card.summon_conditions;
    if sc.is_empty() { return; }

    // summon_conditions on a spell/trap makes no sense
    if ctx.is_spell || ctx.is_trap {
        errors.push(err(ctx.name(), "'summon_condition' block is only valid on monster cards"));
        return;
    }

    // cannot_special_summon AND special_summon_only is a contradiction
    let has_cannot_special = sc.iter().any(|r| matches!(r, SummonRule::CannotSpecialSummon));
    let has_special_only   = sc.iter().any(|r| matches!(r, SummonRule::SpecialSummonOnly));
    if has_cannot_special && has_special_only {
        errors.push(err(
            ctx.name(),
            "'cannot_special_summon' and 'special_summon_only' are contradictory",
        ));
    }

    // If special_summon_only, should not declare tributes_required
    if has_special_only {
        if sc.iter().any(|r| matches!(r, SummonRule::TributesRequired(_))) {
            errors.push(err(
                ctx.name(),
                "'special_summon_only' cards do not use tribute — remove 'tributes_required'",
            ));
        }
    }

    // Tribute material filter without tributes_required
    let has_tribute_filter   = sc.iter().any(|r| matches!(r, SummonRule::TributeMaterial(_)));
    let has_tributes_required = sc.iter().any(|r| matches!(r, SummonRule::TributesRequired(_)));
    if has_tribute_filter && !has_tributes_required {
        errors.push(err(
            ctx.name(),
            "'tribute_material' requires 'tributes_required' to also be declared",
        ));
    }
}

fn check_replacement_effect_validity(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    for re in &ctx.card.replacement_effects {
        // Replacement effects on one-shot spells/traps are unusual.
        // Continuous Spells, Equip Spells, Field Spells, and Continuous
        // Traps routinely host self-protection effects. Additionally,
        // many Normal Spells use the "banish self instead of being
        // destroyed" pattern (e.g. Neos Fusion) — that's idiomatic, not
        // unusual, so we exempt single-action `banish self` replacements.
        let is_persistent_spell_trap = ctx.card.card_types.iter().any(|t| matches!(
            t,
            CardType::ContinuousSpell | CardType::EquipSpell
                | CardType::FieldSpell | CardType::ContinuousTrap
        ));
        let is_banish_self_idiom = re.do_actions.len() == 1
            && matches!(
                &re.do_actions[0],
                GameAction::Banish { target: SelfOrTarget::Self_, .. }
            );
        if (ctx.is_spell || ctx.is_trap)
            && !is_persistent_spell_trap
            && !is_banish_self_idiom
        {
            errors.push(warn(
                ctx.name(),
                "Replacement effects on Spells/Traps are unusual — verify this is the intended behavior",
            ));
        }

        // Cannot have empty do: block
        if re.do_actions.is_empty() {
            errors.push(err(ctx.name(), "Replacement effect 'do' block cannot be empty"));
        }

        // Pendulum return to extra deck instead of destroyed — standard pattern, just check it's right
        if matches!(re.instead_of, ReplaceableEvent::DestroyedByAny | ReplaceableEvent::DestroyedByEffect) {
            let returns_to_extra = re.do_actions.iter().any(|a| {
                matches!(a, GameAction::Return { to: ReturnZone::ExtraDeck, .. })
            });
            if ctx.is_pendulum && !returns_to_extra {
                errors.push(warn(
                    ctx.name(),
                    "Pendulum 'replacement_effect' on destruction usually returns 'self' to 'extra_deck' — verify action",
                ));
            }
        }
    }
}

fn check_equip_effect_validity(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    if ctx.card.equip_effects.is_empty() { return; }

    // equip_effect only makes sense on equip spells or monsters that equip themselves
    let is_equip_spell = ctx.card.card_types.contains(&CardType::EquipSpell);
    if !is_equip_spell && !ctx.is_monster {
        errors.push(err(ctx.name(), "'equip_effect' is only valid on Equip Spells or Effect Monsters"));
    }
}

fn check_win_condition_validity(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    let Some(wc) = &ctx.card.win_condition else { return };

    // all_pieces_in_hand makes no sense on a non-monster
    if matches!(wc.trigger, WinTrigger::AllPiecesInHand) && !ctx.is_monster {
        errors.push(err(ctx.name(), "'all_pieces_in_hand' win condition only applies to monster cards (e.g. Exodia)"));
    }
}

fn check_counter_system_validity(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    let Some(cs) = &ctx.card.counter_system else { return };

    if cs.name.is_empty() {
        errors.push(err(ctx.name(), "Counter system must declare a 'name'"));
    }

    // Counter effects need valid speeds like any other effect
    for effect in &cs.effects {
        if effect.body.speed == SpellSpeed::SpellSpeed3 && !ctx.card.card_types.contains(&CardType::CounterTrap) {
            errors.push(err(
                ctx.name(),
                "Counter system effects cannot be spell_speed_3 unless the card is a Counter Trap",
            ));
        }
    }
}

fn check_continuous_effect_validity(ctx: &CardCtx, errors: &mut Vec<ValidationError>) {
    for ce in &ctx.card.continuous_effects {
        if ce.modifiers.is_empty() && ce.restrictions.is_empty() && ce.cannots.is_empty() {
            errors.push(warn(
                ctx.name(),
                "Continuous effect block has no modifiers, restrictions, or cannots — it does nothing",
            ));
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────

fn err(card: &str, msg: &str) -> ValidationError {
    ValidationError {
        card_name: card.to_string(),
        message:   msg.to_string(),
        severity:  Severity::Error,
    }
}

fn warn(card: &str, msg: &str) -> ValidationError {
    ValidationError {
        card_name: card.to_string(),
        message:   msg.to_string(),
        severity:  Severity::Warning,
    }
}

// ── Error Types ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub card_name: String,
    pub message:   String,
    pub severity:  Severity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,   // Must fix — engine should refuse this card
    Warning, // Should fix — unusual but not illegal
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let tag = match self.severity {
            Severity::Error   => "ERROR",
            Severity::Warning => "WARN ",
        };
        write!(f, "[{}] {}: {}", tag, self.card_name, self.message)
    }
}

// ── Validation Report ─────────────────────────────────────────

#[derive(Debug)]
pub struct ValidationReport {
    pub errors:   Vec<ValidationError>,
    pub warnings: Vec<ValidationError>,
}

impl ValidationReport {
    pub fn from(all: Vec<ValidationError>) -> Self {
        let (errors, warnings) = all.into_iter().partition(|e| e.severity == Severity::Error);
        Self { errors, warnings }
    }

    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn print(&self) {
        if self.errors.is_empty() && self.warnings.is_empty() {
            println!("✓ All cards valid.");
            return;
        }
        for e in &self.errors   { println!("{}", e); }
        for w in &self.warnings { println!("{}", w); }
        println!(
            "\n{} error(s), {} warning(s)",
            self.errors.len(),
            self.warnings.len()
        );
    }
}
