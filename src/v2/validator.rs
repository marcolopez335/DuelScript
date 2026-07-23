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
    check_target_references(&ctx, errors);
    check_fusion_extra_pools(&ctx, errors);
    check_restrict_qualifiers(&ctx, errors);
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
    /// Continuous Spell, Continuous Trap, Field Spell, or Equip Spell —
    /// cards whose activation just places the card in its zone, with the
    /// real work done by passive grants. The "Effect 1" activation block
    /// on these cards is a placeholder with no resolve content.
    is_permanent_passive: bool,
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
        let is_permanent_passive = types.iter().any(|t| matches!(t,
            CardType::ContinuousSpell | CardType::FieldSpell
          | CardType::EquipSpell      | CardType::ContinuousTrap
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
            is_permanent_passive,
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
        // Effect must have resolve or choose.
        // Exception: on permanent-passive cards (Continuous Spell/Trap, Field
        // Spell, Equip Spell), the activation effect just places the card in
        // its zone — its resolve is genuinely empty and the real work is done
        // by `passive` blocks. We skip the error when the effect has no
        // trigger and no cost (cost without resolve already triggers a warn).
        if effect.resolve.is_empty() && effect.choose.is_none() {
            let is_passive_activation = ctx.is_permanent_passive
                && effect.trigger.is_none()
                && effect.cost.is_empty();
            if !is_passive_activation {
                errors.push(err(ctx.name(), &format!(
                    "Effect '{}' must have a resolve or choose block", effect.name
                )));
            }
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

// ── Bare-target references ──────────────────────────────────
//
// `target` in a resolve body refers to the card(s) bound by the effect's
// target declaration or choose block. Without either, the reference
// resolves to nothing at runtime — a silent mis-aim (the bug class shipped
// by pre-oracle translator phases).
fn check_target_references(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    for effect in &ctx.card.effects {
        if effect.target.is_some() || effect.choose.is_some() {
            continue;
        }
        let mut scan = TargetScan::default();
        scan_actions(&effect.resolve, &mut scan);
        // An inline `choose` action binds its own selection — exempt.
        if scan.uses_target && !scan.has_choose {
            errors.push(warn(ctx.name(), &format!(
                "Effect '{}' references `target` but declares no target/choose block",
                effect.name
            )));
        }
    }
}

// ── Fusion extra-pool shape (T38 S5) ────────────────────────
//
// The `plus <selector>` clause describes an EXTRA material pool — a class
// of cards (filter + location) merged into the default hand/field pool
// before material selection (lua `extrafil`). Shorthand/binding selectors
// (`self`, `target`, `searched`, a named binding, …) name a single
// already-resolved card; they cannot describe a pool, and the compiler
// has no filter/location to hand to the runtime seam. Structured
// (parenthesized) selectors are required. A count-limited pool
// (`(1, …)` / `(2+, …)`) is legal but the quantity is not carried
// through the runtime seam (count-limited pools are the lua `fcheck`
// class, out of S5 scope) — warn so the mis-fit is visible. Likewise
// `where` predicates and position filters on the pool: DSL-carried (the
// corpus's natural archetype spelling IS the where-clause form), but
// `ExtraMaterialPool` mirrors only filter + location today — warn so the
// author sees the engine-side gap instead of a silent drop.
fn check_fusion_extra_pools(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    let mut check = |actions: &[Action], block_name: &str| {
        walk_actions(actions, &mut |a| {
            if let Action::FusionSummon { extra_materials: Some(pool), .. } = a {
                match pool {
                    Selector::Counted { quantity, position, where_clause, .. } => {
                        if !matches!(quantity, Quantity::All) {
                            errors.push(warn(ctx.name(), &format!(
                                "'{}': fusion_summon `plus` pool quantity is not carried \
                                 through the runtime seam — use `all`", block_name
                            )));
                        }
                        if where_clause.is_some() || position.is_some() {
                            errors.push(warn(ctx.name(), &format!(
                                "'{}': fusion_summon `plus` pool `where`/position filters \
                                 are DSL-carried but not yet mirrored into the runtime \
                                 pool spec — the engine sees filter + location only", block_name
                            )));
                        }
                    }
                    _ => errors.push(err(ctx.name(), &format!(
                        "'{}': fusion_summon `plus` pool must be a structured \
                         (parenthesized) selector — shorthand selectors name a single \
                         resolved card and cannot describe a material pool", block_name
                    ))),
                }
            }
        });
    };
    for effect in &ctx.card.effects {
        check(&effect.resolve, &effect.name);
        if let Some(choose) = &effect.choose {
            for opt in &choose.options {
                check(&opt.resolve, &effect.name);
            }
        }
    }
    // Replacement blocks (`do { action+ }`) accept the full action grammar
    // too — without this walk a shorthand pool inside a replacement would
    // skip the structured-selector error and hit the compiler's defensive
    // drop (the exact silent-drop path the error exists to prevent).
    for repl in &ctx.card.replacements {
        let name = repl.name.as_deref().unwrap_or("replacement");
        check(&repl.actions, name);
    }
}

// ── Restrict qualifier shape (T38 S2) ───────────────────────
//
// The `from <zone>` / `except (…)` clauses qualify WHICH cards a player
// restriction covers — they only make sense where a per-card dimension
// exists. Battle Phase restrictions have no card at all; normal monster
// summons/sets have exactly one source (the hand), so a source-zone
// dimension — whether the action-level `from` clause OR a from-zone
// exempt atom — is either redundant or contradictory there. Both are
// author errors, not warnings: the compiler would forward a qualifier
// the engine can never evaluate (the exact silent-drop path the S5 pool
// checks exist to prevent). `cannot_set_spells_traps` is NOT in the
// hand-only family: EFFECT_CANNOT_SSET also gates effect-driven Sets
// from non-hand zones, and the corpus carries the qualified shape
// (c88851326 splimit `c:IsLocation(LOCATION_HAND)` — "cannot Set from
// the hand", deck Sets stay legal). Non-zone `except` atoms stay legal
// on every summon/set/activate family member — the lua corpus carries
// `not c:IsRace(…)` splimits on EFFECT_CANNOT_SUMMON too (c38576155;
// zero IsLocation splimits across all 25 CANNOT_SUMMON / 4 CANNOT_MSET
// SetTarget bodies).
//
// Same-zone overlap (`from X except (from X)` where the from-zone atom
// is a whole and-term) makes every scoped card exempt — the action is
// an engine-side no-op. Warning, not error, matching the S5 house rule
// for spellings that are silently inert at the seam.
fn check_restrict_qualifiers(ctx: &Ctx, errors: &mut Vec<ValidationError>) {
    use super::ast::PlayerRestriction as PR;
    let mut check = |actions: &[Action], block_name: &str| {
        walk_actions(actions, &mut |a| {
            if let Action::Restrict { restriction, from_zone, except, .. } = a {
                let qualified = from_zone.is_some() || except.is_some();
                if qualified
                    && matches!(restriction, PR::CannotConductBattlePhase | PR::SkipBattlePhase)
                {
                    errors.push(err(ctx.name(), &format!(
                        "'{}': restrict `from`/`except` qualifiers apply per card — \
                         Battle Phase restrictions take none", block_name
                    )));
                }
                let except_has_from_zone = except.as_ref().is_some_and(|e|
                    e.terms.iter().any(|t|
                        t.atoms.iter().any(|at| matches!(at, ExemptAtom::FromZone(_)))));
                if (from_zone.is_some() || except_has_from_zone)
                    && matches!(restriction, PR::CannotNormalSummon | PR::CannotSetMonsters)
                {
                    errors.push(err(ctx.name(), &format!(
                        "'{}': restrict source-zone qualifiers (`from <zone>` or a \
                         from-zone exempt atom) have no dimension here — normal monster \
                         summons/sets only ever come from the hand", block_name
                    )));
                }
                if let (Some(fz), Some(e)) = (from_zone, except) {
                    if e.terms.iter().any(|t|
                        t.atoms.len() == 1
                            && matches!(&t.atoms[0], ExemptAtom::FromZone(z) if z == fz))
                    {
                        errors.push(warn(ctx.name(), &format!(
                            "'{}': `from` and a whole-term `except (from …)` name the \
                             same zone — every card the restriction scopes to is \
                             exempt, so the action is a no-op", block_name
                        )));
                    }
                }
            }
        });
    };
    for effect in &ctx.card.effects {
        check(&effect.resolve, &effect.name);
        if let Some(choose) = &effect.choose {
            for opt in &choose.options {
                check(&opt.resolve, &effect.name);
            }
        }
    }
    for repl in &ctx.card.replacements {
        let name = repl.name.as_deref().unwrap_or("replacement");
        check(&repl.actions, name);
    }
}

/// Depth-first walk over an action list, recursing into every nested
/// action container (`if`, `coin_flip`, `dice_roll`, `delayed`,
/// `and_if_you_do`, `then`, `also`, `for_each`, `install_watcher`,
/// inline `choose`).
fn walk_actions(actions: &[Action], visit: &mut impl FnMut(&Action)) {
    for action in actions {
        visit(action);
        match action {
            Action::CoinFlip { heads, tails } => {
                walk_actions(heads, visit);
                walk_actions(tails, visit);
            }
            Action::If { then, otherwise, .. } => {
                walk_actions(then, visit);
                walk_actions(otherwise, visit);
            }
            Action::DiceRoll(body)
            | Action::Delayed { body, .. }
            | Action::AndIfYouDo(body)
            | Action::Then(body)
            | Action::Also(body)
            | Action::ForEach { body, .. } => walk_actions(body, visit),
            Action::InstallWatcher { check, .. } => walk_actions(check, visit),
            Action::Choose(block) => {
                for opt in &block.options {
                    walk_actions(&opt.resolve, visit);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct TargetScan {
    uses_target: bool,
    has_choose: bool,
}

fn scan_selector(s: &Selector, scan: &mut TargetScan) {
    if matches!(s, Selector::Target) {
        scan.uses_target = true;
    }
}

fn scan_condition(c: &Condition, scan: &mut TargetScan) {
    let atoms: &[ConditionAtom] = match c {
        Condition::And(v) | Condition::Or(v) => v,
        Condition::Single(a) => std::slice::from_ref(a),
    };
    for a in atoms {
        scan_condition_atom(a, scan);
    }
}

fn scan_condition_atom(a: &ConditionAtom, scan: &mut TargetScan) {
    match a {
        ConditionAtom::Not(inner) => scan_condition_atom(inner, scan),
        ConditionAtom::Controls(_, sel) => scan_selector(sel, scan),
        _ => {}
    }
}

fn scan_actions(actions: &[Action], scan: &mut TargetScan) {
    for action in actions {
        match action {
            Action::Discard(s)
            | Action::Destroy(s)
            | Action::Banish(s, _, _)
            | Action::Send(s, _)
            | Action::Return(s, _)
            | Action::Search(s, _)
            | Action::AddToHand(s, _)
            | Action::SpecialSummon(s, _, _)
            | Action::NormalSummon(s)
            | Action::Set(s, _)
            | Action::FlipDown(s)
            | Action::ChangePosition(s, _)
            | Action::TakeControl(s, _)
            | Action::NegateEffects(s, _)
            | Action::ModifyStat(_, s, _, _, _)
            | Action::SetStat(_, s, _, _)
            | Action::ChangeLevel(s, _)
            | Action::ChangeAttribute(s, _)
            | Action::ChangeRace(s, _)
            | Action::ChangeName(s, _, _)
            | Action::SetScale(s, _)
            | Action::Detach(_, s)
            | Action::PlaceCounter(_, _, s)
            | Action::RemoveCounter(_, _, s)
            | Action::Reveal(s)
            | Action::LookAt(s, _)
            | Action::Grant(s, _, _)
            | Action::SwapStats(s) => scan_selector(s, scan),
            Action::RitualSummon { target, materials, .. }
            | Action::SynchroSummon { target, materials }
            | Action::XyzSummon { target, materials } => {
                scan_selector(target, scan);
                if let Some(m) = materials {
                    scan_selector(m, scan);
                }
            }
            Action::FusionSummon { target, materials, extra_materials, .. } => {
                scan_selector(target, scan);
                if let Some(m) = materials {
                    scan_selector(m, scan);
                }
                if let Some(m) = extra_materials {
                    scan_selector(m, scan);
                }
            }
            Action::Equip(a, b)
            | Action::Attach(a, b)
            | Action::LinkTo(a, b)
            | Action::SwapControl(a, b) => {
                scan_selector(a, scan);
                scan_selector(b, scan);
            }
            Action::CoinFlip { heads, tails } => {
                scan_actions(heads, scan);
                scan_actions(tails, scan);
            }
            Action::DiceRoll(body)
            | Action::Delayed { body, .. }
            | Action::AndIfYouDo(body)
            | Action::Then(body)
            | Action::Also(body) => scan_actions(body, scan),
            Action::If { condition, then, otherwise } => {
                scan_condition(condition, scan);
                scan_actions(then, scan);
                scan_actions(otherwise, scan);
            }
            Action::ForEach { selector, body, .. } => {
                scan_selector(selector, scan);
                scan_actions(body, scan);
            }
            Action::InstallWatcher { check, .. } => scan_actions(check, scan),
            Action::Choose(_) => scan.has_choose = true,
            Action::Draw(_)
            | Action::Negate(_)
            | Action::Damage(_, _)
            | Action::GainLp(_)
            | Action::PayLp(_)
            | Action::CreateToken(_)
            | Action::Mill(_, _)
            | Action::Excavate(_, _)
            | Action::ShuffleDeck(_)
            | Action::ShuffleHand(_)
            | Action::Announce(_, _)
            // T36: player-scoped — no card selector to scan.
            | Action::Restrict { .. }
            // T37: player-scoped — no card selector to scan.
            | Action::DamageRule { .. } => {}
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
    fn test_overlay_counter_passive_valid() {
        // T34: overlay/counter stat-ref passives pass validation clean —
        // no bare-target warnings, no missing-resolve errors.
        let source = r#"
card "Overlay Counter Valid Test" {
    id: 1
    type: Xyz Monster
    attribute: DARK
    race: Beast
    rank: 7
    atk: 700
    def: 2500

    summon {
        cannot_normal_summon
        xyz materials: (2, monster, where level == 7)
    }

    passive "Material Boost" {
        scope: self
        modifier: atk + self.overlay_count * 700
    }

    passive "Counter Boost" {
        scope: self
        modifier: atk + self.counter("Spell Counter") * 300
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
        assert_eq!(report.warning_count(), 0, "warnings: {:?}", report.errors);
    }

    #[test]
    fn test_fusion_plus_pool_valid() {
        // T38 S5: structured all-quantity plus pool + the other proc
        // clauses validate clean.
        let source = r#"
card "Fusion Plus Valid Test" {
    id: 1
    type: Normal Spell

    effect "Fuse" {
        speed: 1
        resolve {
            fusion_summon (1, fusion monster) plus (all, monster, you control, in gy) including self sending_materials_to deck
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
        assert_eq!(report.warning_count(), 0, "warnings: {:?}", report.errors);
    }

    #[test]
    fn test_restrict_qualifiers_valid() {
        // T38 S2: from/except on the summon + activate families validate
        // clean — including except on cannot_normal_summon (the c38576155
        // corpus shape) and from on the activate family (activation
        // location).
        let source = r#"
card "Restrict Qualifier Valid Test" {
    id: 1
    type: Normal Trap

    effect "Summon Limits" {
        speed: 2
        mandatory
        resolve {
            restrict you cannot_special_summon from extra_deck except (is_synchro) this_turn
            restrict opponent cannot_normal_summon except (race == Fairy) this_turn
            restrict both_players cannot_activate from gy end_of_turn
            restrict you cannot_set_monsters except (archetype == "Shaddoll") this_turn
            restrict you cannot_set_spells_traps from hand this_turn
        }
    }
}
"#;
        // The last line is the c88851326 corpus shape: EFFECT_CANNOT_SSET
        // with splimit `c:IsLocation(LOCATION_HAND)` — "cannot Set from the
        // hand" (effect-driven deck Sets stay legal), so SSET is NOT in the
        // hand-only family.
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
        assert_eq!(report.warning_count(), 0, "warnings: {:?}", report.errors);
    }

    #[test]
    fn test_restrict_qualifier_warns_on_same_zone_overlap() {
        // T38 S2 review minor: `from X except (from X)` with a whole-term
        // from-zone atom exempts every scoped card — engine-side no-op,
        // warn per the S5 inert-spelling house rule. A from-zone atom
        // and-composed with other atoms does NOT swallow the restriction
        // and stays clean.
        let source = r#"
card "Restrict Overlap Test" {
    id: 1
    type: Normal Trap

    effect "Lockdown" {
        speed: 2
        mandatory
        resolve {
            restrict you cannot_special_summon from extra_deck except (from extra_deck) this_turn
            restrict you cannot_special_summon from extra_deck except (is_synchro and from extra_deck) this_turn
            restrict you cannot_special_summon from extra_deck except (from gy) this_turn
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
        assert_eq!(report.warning_count(), 1, "warnings: {:?}", report.errors);
        assert!(report.errors.iter().any(|e| e.message.contains("no-op")),
            "unexpected messages: {:?}", report.errors);
    }

    #[test]
    fn test_restrict_qualifier_errors_in_nested_containers() {
        // T38 S2 review minor: the S5 sibling check needed a post-review
        // fix for exactly this — pin the choose-option and replacement
        // walks so a refactor can't silently drop them.
        let source = r#"
card "Restrict Container Test" {
    id: 1
    type: Normal Trap

    effect "Pick" {
        speed: 2
        mandatory
        choose {
            option "Lock" {
                resolve {
                    restrict opponent skip_battle_phase from extra_deck this_turn
                }
            }
            option "Draw" {
                resolve {
                    draw 1
                }
            }
        }
    }

    replacement "Lock Instead" {
        instead_of: destroyed
        do {
            restrict opponent cannot_conduct_battle_phase except (is_synchro) this_turn
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        let bp_errors: Vec<_> = report.errors.iter()
            .filter(|e| e.message.contains("Battle Phase"))
            .collect();
        assert_eq!(bp_errors.len(), 2, "errors: {:?}", report.errors);
        assert!(bp_errors.iter().any(|e| e.message.contains("Lock Instead")),
            "replacement name missing: {:?}", bp_errors);
    }

    #[test]
    fn test_restrict_qualifier_rejects_battle_phase_qualifiers() {
        // T38 S2: Battle Phase restrictions have no per-card dimension —
        // any qualifier is an author error.
        let source = r#"
card "Restrict BP Qualifier Test" {
    id: 1
    type: Normal Trap

    effect "Lockdown" {
        speed: 2
        mandatory
        resolve {
            restrict opponent cannot_conduct_battle_phase except (is_synchro) this_turn
            restrict opponent skip_battle_phase from extra_deck this_turn
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 2, "errors: {:?}", report.errors);
        assert!(report.errors.iter().all(|e| e.message.contains("Battle Phase")),
            "unexpected messages: {:?}", report.errors);
    }

    #[test]
    fn test_restrict_qualifier_rejects_source_zones_on_hand_only_restrictions() {
        // T38 S2: normal monster summons/sets only come from the hand — a
        // source-zone dimension is redundant or contradictory whether it
        // arrives as the `from` clause OR smuggled in as a from-zone
        // exempt atom. Non-zone `except` atoms stay legal (the valid-test
        // covers race/archetype exemptions on this family).
        let source = r#"
card "Restrict From Hand-Only Test" {
    id: 1
    type: Normal Trap

    effect "Lockdown" {
        speed: 2
        mandatory
        resolve {
            restrict you cannot_normal_summon from deck this_turn
            restrict you cannot_set_monsters from deck this_turn
            restrict you cannot_normal_summon except (from extra_deck) this_turn
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 3, "errors: {:?}", report.errors);
        assert!(report.errors.iter().all(|e| e.message.contains("come from the hand")),
            "unexpected messages: {:?}", report.errors);
    }

    #[test]
    fn test_fusion_plus_pool_rejects_shorthand_selector() {
        // T38 S5: a shorthand selector names a single resolved card —
        // it cannot describe a material pool (no filter/location for the
        // runtime seam).
        let source = r#"
card "Fusion Plus Shorthand Test" {
    id: 1
    type: Normal Spell

    effect "Fuse" {
        speed: 1
        resolve {
            fusion_summon (1, fusion monster) plus searched
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 1, "errors: {:?}", report.errors);
        assert!(report.errors[0].message.contains("structured"),
            "unexpected message: {}", report.errors[0].message);
    }

    #[test]
    fn test_fusion_plus_pool_warns_on_counted_quantity() {
        // T38 S5: count-limited pools are the lua fcheck class — legal
        // grammar, but the quantity is not carried through the runtime
        // seam, so the validator surfaces the mis-fit as a warning.
        let source = r#"
card "Fusion Plus Quantity Test" {
    id: 1
    type: Normal Spell

    effect "Fuse" {
        speed: 1
        resolve {
            fusion_summon (1, fusion monster) plus (1, monster, you control, in gy)
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
        assert_eq!(report.warning_count(), 1, "warnings: {:?}", report.errors);
    }

    #[test]
    fn test_fusion_plus_pool_warns_on_where_clause() {
        // T38 S5 review minor 1: `where` predicates on the plus pool are
        // DSL-carried (the corpus's natural archetype spelling) but not
        // yet mirrored into ExtraMaterialPool — the validator must
        // surface the engine-side gap, same shape as the quantity warn.
        let source = r#"
card "Fusion Plus Where Test" {
    id: 1
    type: Normal Spell

    effect "Fuse" {
        speed: 1
        resolve {
            fusion_summon (1, fusion monster) plus (all, monster, you control, in gy, where archetype == "Shaddoll")
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
        assert_eq!(report.warning_count(), 1, "warnings: {:?}", report.errors);
        assert!(report.errors[0].message.contains("not yet mirrored"),
            "unexpected message: {}", report.errors[0].message);
    }

    #[test]
    fn test_fusion_plus_pool_shorthand_in_replacement_errors() {
        // T38 S5 review minor 2: replacement blocks accept the full
        // action grammar — a shorthand pool inside `do { … }` must hit
        // the structured-selector error, not the compiler's silent drop.
        let source = r#"
card "Fusion Plus Replacement Test" {
    id: 1
    type: Normal Spell

    effect "Place" {
        speed: 1
        resolve {
            draw 1
        }
    }

    replacement "Fuse Instead" {
        instead_of: destroyed
        do {
            fusion_summon (1, fusion monster) plus searched
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 1, "errors: {:?}", report.errors);
        let msg = &report.errors.iter().find(|e| matches!(e.severity, Severity::Error))
            .expect("expected an error").message;
        assert!(msg.contains("structured"), "unexpected message: {}", msg);
        assert!(msg.contains("Fuse Instead"), "replacement name missing from: {}", msg);
    }

    #[test]
    fn test_restrict_action_valid() {
        // T36: restrict has no card selector — it must not trip the
        // target-scan invariants (bare-target warning, must-have-resolve ok).
        let source = r#"
card "Restrict Valid Test" {
    id: 1
    type: Normal Trap

    effect "Lockdown" {
        speed: 2
        mandatory
        resolve {
            restrict opponent cannot_special_summon this_turn
            restrict both_players cannot_activate_spells_traps end_of_turn
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
        assert_eq!(report.warning_count(), 0, "warnings: {:?}", report.errors);
    }

    #[test]
    fn test_damage_rule_action_valid() {
        // T37: damage_rule has no card selector — it must not trip the
        // target-scan invariants (bare-target warning, must-have-resolve ok).
        let source = r#"
card "Damage Rule Valid Test" {
    id: 1
    type: Normal Trap

    effect "Shield" {
        speed: 2
        mandatory
        resolve {
            damage_rule you no_battle_damage this_turn
            damage_rule both_players halve_effect_damage end_of_turn
        }
    }
}
"#;
        let file = parse_v2(source).unwrap();
        let report = validate_v2(&file);
        assert_eq!(report.error_count(), 0, "errors: {:?}", report.errors);
        assert_eq!(report.warning_count(), 0, "warnings: {:?}", report.errors);
    }

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

    fn bare_target_card(target: Option<TargetDecl>, resolve: Vec<Action>) -> File {
        File {
            cards: vec![Card {
                name: "Bare Target".into(),
                fields: CardFields {
                    card_types: vec![CardType::NormalSpell],
                    ..Default::default()
                },
                summon: None,
                effects: vec![Effect {
                    name: "Effect 1".into(),
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
                    target,
                    cost: vec![],
                    resolve,
                    choose: None,
                }],
                passives: vec![],
                restrictions: vec![],
                replacements: vec![],
                redirects: vec![],
            }],
        }
    }

    #[test]
    fn test_bare_target_without_declaration_warns() {
        let file = bare_target_card(None, vec![Action::Destroy(Selector::Target)]);
        let report = validate_v2(&file);
        assert!(
            report.errors.iter().any(|e| e.severity == Severity::Warning
                && e.message.contains("references `target`")),
            "expected bare-target warning; got: {:?}", report.errors
        );
    }

    #[test]
    fn test_bare_target_nested_in_then_warns() {
        let file = bare_target_card(
            None,
            vec![Action::Then(vec![Action::Banish(Selector::Target, None, false)])],
        );
        let report = validate_v2(&file);
        assert!(report.errors.iter().any(|e| e.message.contains("references `target`")));
    }

    #[test]
    fn test_target_with_declaration_no_warning() {
        let file = bare_target_card(
            Some(TargetDecl {
                selector: Selector::Counted {
                    quantity: Quantity::Exact(1),
                    filter: CardFilter { name: None, kind: CardFilterKind::Card },
                    controller: None,
                    zone: None,
                    position: None,
                    where_clause: None,
                },
                binding: None,
            }),
            vec![Action::Destroy(Selector::Target)],
        );
        let report = validate_v2(&file);
        assert!(
            !report.errors.iter().any(|e| e.message.contains("references `target`")),
            "declared target must not warn; got: {:?}", report.errors
        );
    }

    /// Permanent-passive cards (Continuous Spell/Trap, Field Spell, Equip Spell)
    /// have an activation effect whose resolve is genuinely empty — the card
    /// is just placed in its zone, and `passive` blocks do the real work.
    /// The validator must not require a resolve for these.
    #[test]
    fn test_permanent_passive_activation_allows_empty_resolve() {
        let mk_card = |ct: CardType, name: &str| Card {
            name: name.into(),
            fields: CardFields {
                card_types: vec![ct],
                ..Default::default()
            },
            summon: None,
            effects: vec![Effect {
                name: "Effect 1".into(),
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
                resolve: vec![],
                choose: None,
            }],
            passives: vec![],
            restrictions: vec![],
            replacements: vec![],
            redirects: vec![],
        };
        for ct in [
            CardType::ContinuousSpell,
            CardType::ContinuousTrap,
            CardType::FieldSpell,
            CardType::EquipSpell,
        ] {
            let file = File { cards: vec![mk_card(ct.clone(), &format!("{:?}", ct))] };
            let report = validate_v2(&file);
            assert!(
                !report.errors.iter().any(|e| e.message.contains("resolve or choose")),
                "{:?} should not require resolve on activation effect; got {:?}",
                ct, report.errors
            );
        }
    }

    /// The relax must NOT extend to non-permanent spells/traps (Normal,
    /// Quick-Play, Counter Trap) — those cards genuinely need resolve content.
    #[test]
    fn test_non_permanent_spells_still_need_resolve() {
        let mk_card = |ct: CardType, name: &str| Card {
            name: name.into(),
            fields: CardFields {
                card_types: vec![ct],
                ..Default::default()
            },
            summon: None,
            effects: vec![Effect {
                name: "Effect 1".into(),
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
                resolve: vec![],
                choose: None,
            }],
            passives: vec![],
            restrictions: vec![],
            replacements: vec![],
            redirects: vec![],
        };
        for ct in [
            CardType::NormalSpell,
            CardType::QuickPlaySpell,
            CardType::NormalTrap,
            CardType::CounterTrap,
        ] {
            let file = File { cards: vec![mk_card(ct.clone(), &format!("{:?}", ct))] };
            let report = validate_v2(&file);
            assert!(
                report.errors.iter().any(|e| e.message.contains("resolve or choose")),
                "{:?} should still require resolve; got {:?}",
                ct, report.errors
            );
        }
    }

    /// Even on permanent-passive cards, an effect with a *trigger* (e.g.
    /// "if X is destroyed") is a real ignition/trigger effect — it must
    /// have a resolve.
    #[test]
    fn test_permanent_passive_with_trigger_still_needs_resolve() {
        let file = File {
            cards: vec![Card {
                name: "Triggered Continuous".into(),
                fields: CardFields {
                    card_types: vec![CardType::ContinuousSpell],
                    ..Default::default()
                },
                summon: None,
                effects: vec![Effect {
                    name: "Effect 1".into(),
                    speed: Some(1),
                    frequency: None,
                    mandatory: false,
                    simultaneous: false,
                    timing: None,
                    trigger: Some(Trigger::EndPhase),
                    who: None,
                    condition: None,
                    activate_from: vec![],
                    damage_step: None,
                    target: None,
                    cost: vec![],
                    resolve: vec![],
                    choose: None,
                }],
                passives: vec![],
                restrictions: vec![],
                replacements: vec![],
                redirects: vec![],
            }],
        };
        let report = validate_v2(&file);
        assert!(
            report.errors.iter().any(|e| e.message.contains("resolve or choose")),
            "trigger on Continuous Spell should still require resolve; got {:?}",
            report.errors
        );
    }
}
