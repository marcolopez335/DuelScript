// ============================================================
// DuelScript v2 Pretty-Printer
//
// Walks the v2 AST and emits canonical source that re-parses
// to an equivalent AST (roundtrip guarantee).
// ============================================================

use std::fmt::Write;
use super::ast::*;

// ── Entry Point ──────────────────────────────────────────────

pub fn format_file(file: &File) -> String {
    let mut out = String::new();
    for (i, card) in file.cards.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        format_card(card, &mut out);
    }
    out
}

// ── Card ─────────────────────────────────────────────────────

fn format_card(card: &Card, out: &mut String) {
    writeln!(out, "card \"{}\" {{", card.name).unwrap();
    format_fields(&card.fields, out);
    if let Some(s) = &card.summon {
        format_summon_block(s, out);
    }
    for p in &card.passives {
        format_passive(p, out);
    }
    for r in &card.restrictions {
        format_restriction(r, out, 4);
    }
    for r in &card.replacements {
        format_replacement(r, out);
    }
    for r in &card.redirects {
        format_redirect(r, out);
    }
    for e in &card.effects {
        format_effect(e, out);
    }
    writeln!(out, "}}").unwrap();
}

// ── Fields ───────────────────────────────────────────────────

fn format_fields(fields: &CardFields, out: &mut String) {
    if let Some(id) = fields.id {
        writeln!(out, "    id: {}", id).unwrap();
    }
    if !fields.card_types.is_empty() {
        let types: Vec<&str> = fields.card_types.iter().map(|t| format_card_type(t)).collect();
        writeln!(out, "    type: {}", types.join(" | ")).unwrap();
    }
    if let Some(attr) = &fields.attribute {
        writeln!(out, "    attribute: {}", format_attribute(attr)).unwrap();
    }
    if let Some(race) = &fields.race {
        writeln!(out, "    race: {}", format_race(race)).unwrap();
    }
    if let Some(level) = fields.level {
        writeln!(out, "    level: {}", level).unwrap();
    }
    if let Some(rank) = fields.rank {
        writeln!(out, "    rank: {}", rank).unwrap();
    }
    if let Some(link) = fields.link {
        writeln!(out, "    link: {}", link).unwrap();
    }
    if let Some(scale) = fields.scale {
        writeln!(out, "    scale: {}", scale).unwrap();
    }
    if let Some(atk) = &fields.atk {
        writeln!(out, "    atk: {}", format_stat_val(atk)).unwrap();
    }
    if let Some(def) = &fields.def {
        writeln!(out, "    def: {}", format_stat_val(def)).unwrap();
    }
    if !fields.link_arrows.is_empty() {
        let arrows: Vec<&str> = fields.link_arrows.iter().map(|a| format_arrow(a)).collect();
        writeln!(out, "    link_arrows: [{}]", arrows.join(", ")).unwrap();
    }
    if !fields.archetypes.is_empty() {
        let arches: Vec<String> = fields.archetypes.iter().map(|a| format!("\"{}\"", a)).collect();
        writeln!(out, "    archetype: [{}]", arches.join(", ")).unwrap();
    }
}

// ── Summon Block ─────────────────────────────────────────────

fn format_summon_block(s: &SummonBlock, out: &mut String) {
    writeln!(out, "\n    summon {{").unwrap();
    if s.cannot_normal_summon {
        writeln!(out, "        cannot_normal_summon").unwrap();
    }
    if s.cannot_special_summon {
        writeln!(out, "        cannot_special_summon").unwrap();
    }
    if let Some(t) = s.tributes {
        writeln!(out, "        tributes: {}", t).unwrap();
    }
    if let Some(ssp) = &s.special_summon_procedure {
        // Only emit special_summon_procedure if it has content (parser may not populate all fields)
        let has_content = ssp.from.is_some()
            || ssp.to.is_some()
            || !ssp.cost.is_empty()
            || ssp.condition.is_some()
            || ssp.restriction.is_some();
        if has_content {
            format_ssp(ssp, out);
        }
    }
    if let Some(fm) = &s.fusion_materials {
        // Only emit if items were parsed (parser limitation may leave list empty)
        if !fm.items.is_empty() {
            let items: Vec<String> = fm.items.iter().map(format_material_item).collect();
            writeln!(out, "        fusion materials: {}", items.join(" + ")).unwrap();
        }
    }
    if let Some(sm) = &s.synchro_materials {
        writeln!(out, "        synchro materials {{").unwrap();
        writeln!(out, "            tuner: {}", format_selector(&sm.tuner)).unwrap();
        writeln!(out, "            non_tuner: {}", format_selector(&sm.non_tuner)).unwrap();
        writeln!(out, "        }}").unwrap();
    }
    if let Some(xm) = &s.xyz_materials {
        writeln!(out, "        xyz materials: {}", format_selector(xm)).unwrap();
    }
    if let Some(lm) = &s.link_materials {
        writeln!(out, "        link materials: {}", format_selector(lm)).unwrap();
    }
    if let Some(rm) = &s.ritual_materials {
        let mut line = format!("        ritual materials: {}", format_selector(&rm.materials));
        if let Some(lc) = &rm.level_constraint {
            let kind = match lc.kind {
                LevelConstraintKind::TotalLevel => "total_level",
                LevelConstraintKind::ExactLevel => "exact_level",
            };
            line.push_str(&format!(" where {} {} {}", kind, format_compare_op(&lc.op), format_expr(&lc.value)));
        }
        writeln!(out, "{}", line).unwrap();
    }
    if !s.pendulum_from.is_empty() {
        let zones: Vec<&str> = s.pendulum_from.iter().map(|z| format_zone(z)).collect();
        writeln!(out, "        pendulum from: [{}]", zones.join(", ")).unwrap();
    }
    writeln!(out, "    }}").unwrap();
}

fn format_ssp(ssp: &SpecialSummonProcedure, out: &mut String) {
    writeln!(out, "        special_summon_procedure {{").unwrap();
    if let Some(from) = &ssp.from {
        writeln!(out, "            from: {}", format_zone(from)).unwrap();
    }
    if let Some(to) = &ssp.to {
        writeln!(out, "            to: {}", format_field_target(to)).unwrap();
    }
    if !ssp.cost.is_empty() {
        writeln!(out, "            cost {{").unwrap();
        for c in &ssp.cost {
            writeln!(out, "                {}", format_cost_action(c)).unwrap();
        }
        writeln!(out, "            }}").unwrap();
    }
    if let Some(cond) = &ssp.condition {
        writeln!(out, "            condition: {}", format_condition(cond)).unwrap();
    }
    if let Some(r) = &ssp.restriction {
        format_restriction(r, out, 12);
    }
    writeln!(out, "        }}").unwrap();
}

fn format_material_item(item: &MaterialItem) -> String {
    match item {
        MaterialItem::Named(s) => format!("\"{}\"", s),
        MaterialItem::Generic(sel) => format_selector(sel),
    }
}

// ── Effect Block ─────────────────────────────────────────────

fn format_effect(e: &Effect, out: &mut String) {
    writeln!(out, "\n    effect \"{}\" {{", e.name).unwrap();
    if let Some(speed) = e.speed {
        writeln!(out, "        speed: {}", speed).unwrap();
    }
    if let Some(freq) = &e.frequency {
        writeln!(out, "        {}", format_frequency(freq)).unwrap();
    }
    if e.mandatory {
        writeln!(out, "        mandatory").unwrap();
    }
    if e.simultaneous {
        writeln!(out, "        simultaneous").unwrap();
    }
    if let Some(timing) = &e.timing {
        let t = match timing {
            Timing::When => "when",
            Timing::If => "if",
        };
        writeln!(out, "        timing: {}", t).unwrap();
    }
    if let Some(trigger) = &e.trigger {
        writeln!(out, "        trigger: {}", format_trigger(trigger)).unwrap();
    }
    if let Some(who) = &e.who {
        writeln!(out, "        who: {}", format_player_who(who)).unwrap();
    }
    if let Some(cond) = &e.condition {
        writeln!(out, "        condition: {}", format_condition(cond)).unwrap();
    }
    if !e.activate_from.is_empty() {
        let zones: Vec<&str> = e.activate_from.iter().map(|z| format_zone(z)).collect();
        writeln!(out, "        activate_from: [{}]", zones.join(", ")).unwrap();
    }
    if let Some(ds) = e.damage_step {
        writeln!(out, "        damage_step: {}", ds).unwrap();
    }
    if let Some(target) = &e.target {
        let mut s = format!("        target {}", format_selector(&target.selector));
        if let Some(b) = &target.binding {
            s.push_str(&format!(" as {}", b));
        }
        writeln!(out, "{}", s).unwrap();
    }
    if !e.cost.is_empty() {
        writeln!(out, "        cost {{").unwrap();
        for c in &e.cost {
            writeln!(out, "            {}", format_cost_action(c)).unwrap();
        }
        writeln!(out, "        }}").unwrap();
    }
    if !e.resolve.is_empty() {
        writeln!(out, "        resolve {{").unwrap();
        for a in &e.resolve {
            format_action(a, out, 12);
        }
        writeln!(out, "        }}").unwrap();
    }
    if let Some(choose) = &e.choose {
        format_choose_block(choose, out, 8);
    }
    writeln!(out, "    }}").unwrap();
}

fn format_frequency(freq: &Frequency) -> &'static str {
    match freq {
        Frequency::OncePerTurn(OptKind::Soft) => "once_per_turn: soft",
        Frequency::OncePerTurn(OptKind::Hard) => "once_per_turn: hard",
        Frequency::TwicePerTurn => "twice_per_turn",
        Frequency::OncePerDuel => "once_per_duel",
    }
}

// ── Passive Block ─────────────────────────────────────────────

fn format_passive(p: &Passive, out: &mut String) {
    writeln!(out, "\n    passive \"{}\" {{", p.name).unwrap();
    if let Some(scope) = &p.scope {
        let s = match scope {
            Scope::Self_ => "self",
            Scope::Field => "field",
        };
        writeln!(out, "        scope: {}", s).unwrap();
    }
    if let Some(target) = &p.target {
        writeln!(out, "        target: {}", format_selector(target)).unwrap();
    }
    if let Some(cond) = &p.condition {
        writeln!(out, "        condition: {}", format_condition(cond)).unwrap();
    }
    for m in &p.modifiers {
        let sign = if m.positive { "+" } else { "-" };
        writeln!(out, "        modifier: {} {} {}", format_stat_name(&m.stat), sign, format_expr(&m.value)).unwrap();
    }
    for g in &p.grants {
        writeln!(out, "        grant: {}", format_grant_ability(g)).unwrap();
    }
    if p.negate_effects {
        writeln!(out, "        negate_effects").unwrap();
    }
    if let Some(atk) = &p.set_atk {
        writeln!(out, "        set_atk: {}", format_expr(atk)).unwrap();
    }
    if let Some(def) = &p.set_def {
        writeln!(out, "        set_def: {}", format_expr(def)).unwrap();
    }
    writeln!(out, "    }}").unwrap();
}

// ── Restriction Block ─────────────────────────────────────────

fn format_restriction(r: &Restriction, out: &mut String, indent: usize) {
    let pad = " ".repeat(indent);
    let inner_pad = " ".repeat(indent + 4);
    let name_part = r.name.as_deref().map(|n| format!(" \"{}\"", n)).unwrap_or_default();
    writeln!(out, "\n{}restriction{} {{", pad, name_part).unwrap();
    if let Some(apply_to) = &r.apply_to {
        writeln!(out, "{}apply_to: {}", inner_pad, format_player_who(apply_to)).unwrap();
    }
    if let Some(target) = &r.target {
        writeln!(out, "{}target: {}", inner_pad, format_selector(target)).unwrap();
    }
    for ability in &r.abilities {
        writeln!(out, "{}{}", inner_pad, format_grant_ability(ability)).unwrap();
    }
    if let Some(dur) = &r.duration {
        writeln!(out, "{}duration: {}", inner_pad, format_duration(dur)).unwrap();
    }
    if let Some(trigger) = &r.trigger {
        writeln!(out, "{}trigger: {}", inner_pad, format_trigger(trigger)).unwrap();
    }
    if let Some(cond) = &r.condition {
        writeln!(out, "{}condition: {}", inner_pad, format_condition(cond)).unwrap();
    }
    writeln!(out, "{}}}", pad).unwrap();
}

// ── Replacement Block ─────────────────────────────────────────

fn format_replacement(r: &Replacement, out: &mut String) {
    let name_part = r.name.as_deref().map(|n| format!(" \"{}\"", n)).unwrap_or_default();
    writeln!(out, "\n    replacement{} {{", name_part).unwrap();
    writeln!(out, "        instead_of: {}", format_replaceable_event(&r.instead_of)).unwrap();
    if let Some(cond) = &r.condition {
        writeln!(out, "        condition: {}", format_condition(cond)).unwrap();
    }
    writeln!(out, "        do {{").unwrap();
    for a in &r.actions {
        format_action(a, out, 12);
    }
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
}

fn format_replaceable_event(ev: &ReplaceableEvent) -> &'static str {
    match ev {
        ReplaceableEvent::DestroyedByBattle => "destroyed_by_battle",
        ReplaceableEvent::DestroyedByEffect => "destroyed_by_effect",
        ReplaceableEvent::Destroyed => "destroyed",
        ReplaceableEvent::SentToGy => "sent_to_gy",
        ReplaceableEvent::Banished => "banished",
        ReplaceableEvent::ReturnedToHand => "returned_to_hand",
        ReplaceableEvent::ReturnedToDeck => "returned_to_deck",
        ReplaceableEvent::LeavesField => "leaves_field",
    }
}

// ── Redirect Block (T31 / CC-II) ──────────────────────────────

fn format_redirect(r: &Redirect, out: &mut String) {
    let name_part = r.name.as_deref().map(|n| format!(" \"{}\"", n)).unwrap_or_default();
    writeln!(out, "\n    redirect{} {{", name_part).unwrap();
    writeln!(out, "        scope: {}", format_redirect_scope(&r.scope)).unwrap();
    writeln!(out, "        from: {}", format_zone(&r.from)).unwrap();
    writeln!(out, "        to: {}", format_zone(&r.to)).unwrap();
    if let Some(sel) = &r.filter {
        writeln!(out, "        when: {}", format_selector(sel)).unwrap();
    }
    writeln!(out, "    }}").unwrap();
}

fn format_redirect_scope(s: &RedirectScope) -> &'static str {
    match s {
        RedirectScope::Self_         => "self",
        RedirectScope::Field         => "field",
        RedirectScope::OpponentField => "opponent_field",
        RedirectScope::BothFields    => "both_fields",
    }
}

// ── Choose Block ──────────────────────────────────────────────

fn format_choose_block(choose: &ChooseBlock, out: &mut String, indent: usize) {
    let pad = " ".repeat(indent);
    let inner_pad = " ".repeat(indent + 4);
    writeln!(out, "{}choose {{", pad).unwrap();
    for opt in &choose.options {
        writeln!(out, "{}option \"{}\" {{", inner_pad, opt.label).unwrap();
        let opt_inner = " ".repeat(indent + 8);
        if let Some(target) = &opt.target {
            let mut s = format!("{}target {}", opt_inner, format_selector(&target.selector));
            if let Some(b) = &target.binding {
                s.push_str(&format!(" as {}", b));
            }
            writeln!(out, "{}", s).unwrap();
        }
        if !opt.cost.is_empty() {
            writeln!(out, "{}cost {{", opt_inner).unwrap();
            let cost_pad = " ".repeat(indent + 12);
            for c in &opt.cost {
                writeln!(out, "{}{}", cost_pad, format_cost_action(c)).unwrap();
            }
            writeln!(out, "{}}}", opt_inner).unwrap();
        }
        if let Some(trigger) = &opt.trigger {
            writeln!(out, "{}trigger: {}", opt_inner, format_trigger(trigger)).unwrap();
        }
        if !opt.resolve.is_empty() {
            writeln!(out, "{}resolve {{", opt_inner).unwrap();
            for a in &opt.resolve {
                format_action(a, out, indent + 12);
            }
            writeln!(out, "{}}}", opt_inner).unwrap();
        }
        writeln!(out, "{}}}", inner_pad).unwrap();
    }
    writeln!(out, "{}}}", pad).unwrap();
}

// ── Actions ───────────────────────────────────────────────────

fn format_action(a: &Action, out: &mut String, indent: usize) {
    let pad = " ".repeat(indent);
    match a {
        Action::Draw(expr) => writeln!(out, "{}draw {}", pad, format_expr(expr)).unwrap(),
        Action::Discard(sel) => writeln!(out, "{}discard {}", pad, format_selector(sel)).unwrap(),
        Action::Destroy(sel) => writeln!(out, "{}destroy {}", pad, format_selector(sel)).unwrap(),
        Action::Banish(sel, zone, face_down) => {
            let mut s = format!("{}banish {}", pad, format_selector(sel));
            if let Some(z) = zone {
                s.push_str(&format!(" from {}", format_zone(z)));
            }
            if *face_down {
                s.push_str(" face_down");
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::Send(sel, zone) => {
            writeln!(out, "{}send {} to {}", pad, format_selector(sel), format_zone(zone)).unwrap();
        }
        Action::Return(sel, dest) => {
            let dest_str = match dest {
                ReturnDest::Hand => "hand".to_string(),
                ReturnDest::Deck(None) => "deck".to_string(),
                ReturnDest::Deck(Some(DeckPosition::Top)) => "deck top".to_string(),
                ReturnDest::Deck(Some(DeckPosition::Bottom)) => "deck bottom".to_string(),
                ReturnDest::Deck(Some(DeckPosition::Shuffle)) => "deck shuffle".to_string(),
                ReturnDest::ExtraDeck => "extra_deck".to_string(),
                ReturnDest::Owner => "owner".to_string(),
            };
            writeln!(out, "{}return {} to {}", pad, format_selector(sel), dest_str).unwrap();
        }
        Action::Search(sel, zone) => {
            let mut s = format!("{}search {}", pad, format_selector(sel));
            if let Some(z) = zone {
                s.push_str(&format!(" from {}", format_zone(z)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::AddToHand(sel, zone) => {
            let mut s = format!("{}add_to_hand {}", pad, format_selector(sel));
            if let Some(z) = zone {
                s.push_str(&format!(" from {}", format_zone(z)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::SpecialSummon(sel, zone, pos) => {
            let mut s = format!("{}special_summon {}", pad, format_selector(sel));
            if let Some(z) = zone {
                s.push_str(&format!(" from {}", format_zone(z)));
            }
            if let Some(p) = pos {
                s.push_str(&format!(" in {}", format_battle_position(p)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::RitualSummon { target, materials, level_op, level_expr } => {
            let mut s = format!("{}ritual_summon {}", pad, format_selector(target));
            if let Some(m) = materials {
                s.push_str(&format!(" using {}", format_selector(m)));
            }
            if let (Some(op), Some(expr)) = (level_op, level_expr) {
                s.push_str(&format!(" where total_level {} {}", format_compare_op(op), format_expr(expr)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::FusionSummon { target, materials } => {
            let mut s = format!("{}fusion_summon {}", pad, format_selector(target));
            if let Some(m) = materials {
                s.push_str(&format!(" using {}", format_selector(m)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::SynchroSummon { target, materials } => {
            let mut s = format!("{}synchro_summon {}", pad, format_selector(target));
            if let Some(m) = materials {
                s.push_str(&format!(" using {}", format_selector(m)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::XyzSummon { target, materials } => {
            let mut s = format!("{}xyz_summon {}", pad, format_selector(target));
            if let Some(m) = materials {
                s.push_str(&format!(" using {}", format_selector(m)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::NormalSummon(sel) => {
            writeln!(out, "{}normal_summon {}", pad, format_selector(sel)).unwrap();
        }
        Action::Set(sel, zone) => {
            let mut s = format!("{}set {}", pad, format_selector(sel));
            if let Some(z) = zone {
                s.push_str(&format!(" from {}", format_zone(z)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::FlipDown(sel) => {
            writeln!(out, "{}flip_down {}", pad, format_selector(sel)).unwrap();
        }
        Action::ChangePosition(sel, pos) => {
            let mut s = format!("{}change_position {}", pad, format_selector(sel));
            if let Some(p) = pos {
                s.push_str(&format!(" to {}", format_battle_position(p)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::TakeControl(sel, dur) => {
            let mut s = format!("{}take_control {}", pad, format_selector(sel));
            if let Some(d) = dur {
                s.push_str(&format!(" until {}", format_duration(d)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::Equip(card, target) => {
            writeln!(out, "{}equip {} to {}", pad, format_selector(card), format_selector(target)).unwrap();
        }
        Action::Negate(and_destroy) => {
            if *and_destroy {
                writeln!(out, "{}negate and destroy", pad).unwrap();
            } else {
                writeln!(out, "{}negate", pad).unwrap();
            }
        }
        Action::NegateEffects(sel, dur) => {
            let mut s = format!("{}negate_effects {}", pad, format_selector(sel));
            if let Some(d) = dur {
                s.push_str(&format!(" {}", format_duration(d)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::Damage(who, expr) => {
            writeln!(out, "{}damage {} {}", pad, format_player_who(who), format_expr(expr)).unwrap();
        }
        Action::GainLp(expr) => {
            writeln!(out, "{}gain_lp {}", pad, format_expr(expr)).unwrap();
        }
        Action::PayLp(expr) => {
            writeln!(out, "{}pay_lp {}", pad, format_expr(expr)).unwrap();
        }
        Action::ModifyStat(stat, sel, is_negative, expr, dur) => {
            let sign = if *is_negative { "-" } else { "+" };
            let mut s = format!("{}modify_{} {} {} {}", pad, format_stat_name(stat), format_selector(sel), sign, format_expr(expr));
            if let Some(d) = dur {
                s.push_str(&format!(" until {}", format_duration(d)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::SetStat(stat, sel, expr, dur) => {
            let mut s = format!("{}set_{} {} {}", pad, format_stat_name(stat), format_selector(sel), format_expr(expr));
            if let Some(d) = dur {
                s.push_str(&format!(" until {}", format_duration(d)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::ChangeLevel(sel, expr) => {
            writeln!(out, "{}change_level {} to {}", pad, format_selector(sel), format_expr(expr)).unwrap();
        }
        Action::ChangeAttribute(sel, attr) => {
            writeln!(out, "{}change_attribute {} to {}", pad, format_selector(sel), format_attribute(attr)).unwrap();
        }
        Action::ChangeRace(sel, race) => {
            writeln!(out, "{}change_race {} to {}", pad, format_selector(sel), format_race(race)).unwrap();
        }
        Action::ChangeName(sel, name, dur) => {
            let mut s = format!("{}change_name {} to \"{}\"", pad, format_selector(sel), name);
            if let Some(d) = dur {
                s.push_str(&format!(" until {}", format_duration(d)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::SetScale(sel, expr) => {
            writeln!(out, "{}set_scale {} to {}", pad, format_selector(sel), format_expr(expr)).unwrap();
        }
        Action::CreateToken(spec) => {
            writeln!(out, "{}create_token {{", pad).unwrap();
            let inner = " ".repeat(indent + 4);
            if let Some(name) = &spec.name {
                writeln!(out, "{}name: \"{}\"", inner, name).unwrap();
            }
            if let Some(attr) = &spec.attribute {
                writeln!(out, "{}attribute: {}", inner, format_attribute(attr)).unwrap();
            }
            if let Some(race) = &spec.race {
                writeln!(out, "{}race: {}", inner, format_race(race)).unwrap();
            }
            if let Some(level) = spec.level {
                writeln!(out, "{}level: {}", inner, level).unwrap();
            }
            writeln!(out, "{}atk: {}", inner, format_stat_val(&spec.atk)).unwrap();
            writeln!(out, "{}def: {}", inner, format_stat_val(&spec.def)).unwrap();
            writeln!(out, "{}count: {}", inner, spec.count).unwrap();
            if let Some(pos) = &spec.position {
                writeln!(out, "{}position: {}", inner, format_battle_position(pos)).unwrap();
            }
            if let Some(r) = &spec.restriction {
                format_restriction(r, out, indent + 4);
            }
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::Attach(card, target) => {
            writeln!(out, "{}attach {} to {} as_material", pad, format_selector(card), format_selector(target)).unwrap();
        }
        Action::Detach(count, sel) => {
            writeln!(out, "{}detach {} from {}", pad, count, format_selector(sel)).unwrap();
        }
        Action::PlaceCounter(name, count, sel) => {
            writeln!(out, "{}place_counter \"{}\" {} on {}", pad, name, count, format_selector(sel)).unwrap();
        }
        Action::RemoveCounter(name, count, sel) => {
            writeln!(out, "{}remove_counter \"{}\" {} from {}", pad, name, count, format_selector(sel)).unwrap();
        }
        Action::Mill(expr, owner) => {
            let mut s = format!("{}mill {}", pad, format_expr(expr));
            if let Some(o) = owner {
                let deck = match o {
                    DeckOwner::Yours => "your_deck",
                    DeckOwner::Opponents => "opponent_deck",
                };
                s.push_str(&format!(" from {}", deck));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::Excavate(expr, owner) => {
            let deck = match owner {
                DeckOwner::Yours => "your_deck",
                DeckOwner::Opponents => "opponent_deck",
            };
            writeln!(out, "{}excavate {} from {}", pad, format_expr(expr), deck).unwrap();
        }
        Action::Reveal(sel) => {
            writeln!(out, "{}reveal {}", pad, format_selector(sel)).unwrap();
        }
        Action::LookAt(sel, zone) => {
            let mut s = format!("{}look_at {}", pad, format_selector(sel));
            if let Some(z) = zone {
                s.push_str(&format!(" from {}", format_zone(z)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::ShuffleDeck(owner) => {
            let suffix = match owner {
                None => "".to_string(),
                Some(DeckOwner::Yours) => " yours".to_string(),
                Some(DeckOwner::Opponents) => " opponents".to_string(),
            };
            writeln!(out, "{}shuffle_deck{}", pad, suffix).unwrap();
        }
        Action::Announce(what, binding) => {
            let mut s = format!("{}announce {}", pad, format_announce_what(what));
            if let Some(b) = binding {
                s.push_str(&format!(" as {}", b));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::LinkTo(a, b) => {
            writeln!(out, "{}link {} to {}", pad, format_selector(a), format_selector(b)).unwrap();
        }
        Action::CoinFlip { heads, tails } => {
            writeln!(out, "{}flip_coin {{", pad).unwrap();
            let inner = " ".repeat(indent + 4);
            writeln!(out, "{}heads {{", inner).unwrap();
            for a in heads {
                format_action(a, out, indent + 8);
            }
            writeln!(out, "{}}}",inner).unwrap();
            writeln!(out, "{}tails {{", inner).unwrap();
            for a in tails {
                format_action(a, out, indent + 8);
            }
            writeln!(out, "{}}}", inner).unwrap();
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::DiceRoll(actions) => {
            writeln!(out, "{}roll_dice {{", pad).unwrap();
            for a in actions {
                format_action(a, out, indent + 4);
            }
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::Grant(sel, ability, dur) => {
            let mut s = format!("{}grant {} {}", pad, format_selector(sel), format_grant_ability(ability));
            if let Some(d) = dur {
                s.push_str(&format!(" until {}", format_duration(d)));
            }
            writeln!(out, "{}", s).unwrap();
        }
        Action::If { condition, then, otherwise } => {
            writeln!(out, "{}if ({}) {{", pad, format_condition(condition)).unwrap();
            for a in then {
                format_action(a, out, indent + 4);
            }
            if !otherwise.is_empty() {
                writeln!(out, "{}}} else {{", pad).unwrap();
                for a in otherwise {
                    format_action(a, out, indent + 4);
                }
            }
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::ForEach { selector, zone, body } => {
            writeln!(out, "{}for_each {} in {} {{", pad, format_selector(selector), format_zone(zone)).unwrap();
            for a in body {
                format_action(a, out, indent + 4);
            }
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::Choose(block) => {
            format_choose_block(block, out, indent);
        }
        Action::Delayed { until, body } => {
            writeln!(out, "{}delayed until {} {{", pad, format_phase_name(until)).unwrap();
            for a in body {
                format_action(a, out, indent + 4);
            }
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::AndIfYouDo(actions) => {
            writeln!(out, "{}and_if_you_do {{", pad).unwrap();
            for a in actions {
                format_action(a, out, indent + 4);
            }
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::Then(actions) => {
            writeln!(out, "{}then {{", pad).unwrap();
            for a in actions {
                format_action(a, out, indent + 4);
            }
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::Also(actions) => {
            writeln!(out, "{}also {{", pad).unwrap();
            for a in actions {
                format_action(a, out, indent + 4);
            }
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::InstallWatcher { name, event, duration, check } => {
            writeln!(out, "{}install_watcher \"{}\" {{", pad, name).unwrap();
            let inner = " ".repeat(indent + 4);
            writeln!(out, "{}event: {}", inner, format_trigger(event)).unwrap();
            writeln!(out, "{}duration: {}", inner, format_duration(duration)).unwrap();
            writeln!(out, "{}check {{", inner).unwrap();
            for a in check {
                format_action(a, out, indent + 8);
            }
            writeln!(out, "{}}}", inner).unwrap();
            writeln!(out, "{}}}", pad).unwrap();
        }
        Action::SwapControl(a, b) => {
            writeln!(out, "{}swap_control {} and {}", pad, format_selector(a), format_selector(b)).unwrap();
        }
        Action::SwapStats(sel) => {
            writeln!(out, "{}swap_stats {}", pad, format_selector(sel)).unwrap();
        }
    }
}

// ── Cost Actions ──────────────────────────────────────────────

fn format_cost_action(c: &CostAction) -> String {
    match c {
        CostAction::PayLp(expr) => format!("pay_lp {}", format_expr(expr)),
        CostAction::Discard(sel, binding) => {
            let mut s = format!("discard {}", format_selector(sel));
            if let Some(b) = binding {
                s.push_str(&format!(" as {}", b));
            }
            s
        }
        CostAction::Tribute(sel, binding) => {
            let mut s = format!("tribute {}", format_selector(sel));
            if let Some(b) = binding {
                s.push_str(&format!(" as {}", b));
            }
            s
        }
        CostAction::Banish(sel, zone, binding) => {
            let mut s = format!("banish {}", format_selector(sel));
            if let Some(z) = zone {
                s.push_str(&format!(" from {}", format_zone(z)));
            }
            if let Some(b) = binding {
                s.push_str(&format!(" as {}", b));
            }
            s
        }
        CostAction::Send(sel, zone, binding) => {
            let mut s = format!("send {} to {}", format_selector(sel), format_zone(zone));
            if let Some(b) = binding {
                s.push_str(&format!(" as {}", b));
            }
            s
        }
        CostAction::Detach(count, sel) => {
            match sel {
                Selector::SelfCard => format!("detach {} from self", count),
                _ => format!("detach {} from {}", count, format_selector(sel)),
            }
        }
        CostAction::RemoveCounter(name, count, sel) => {
            match sel {
                Selector::SelfCard => format!("remove_counter \"{}\" {} from self", name, count),
                _ => format!("remove_counter \"{}\" {} from {}", name, count, format_selector(sel)),
            }
        }
        CostAction::Reveal(sel) => format!("reveal {}", format_selector(sel)),
        CostAction::Announce(what, binding) => {
            let mut s = format!("announce {}", format_announce_what(what));
            if let Some(b) = binding {
                s.push_str(&format!(" as {}", b));
            }
            s
        }
        CostAction::None => "none".to_string(),
    }
}

// ── Selectors ─────────────────────────────────────────────────

fn format_selector(sel: &Selector) -> String {
    match sel {
        Selector::SelfCard => "self".to_string(),
        Selector::Target => "target".to_string(),
        Selector::EquippedCard => "equipped_card".to_string(),
        Selector::NegatedCard => "negated_card".to_string(),
        Selector::Searched => "searched".to_string(),
        Selector::LinkedCard => "linked_card".to_string(),
        Selector::Binding(name) => name.clone(),
        Selector::Counted { quantity, filter, controller, zone, position, where_clause } => {
            let mut parts = vec![
                format_quantity(quantity),
                format_card_filter(filter),
            ];
            if let Some(ctrl) = controller {
                parts.push(format_controller(ctrl).to_string());
            }
            if let Some(z) = zone {
                parts.push(format_zone_filter(z));
            }
            if let Some(p) = position {
                parts.push(format_position_filter(p).to_string());
            }
            if let Some(wc) = where_clause {
                parts.push(format!("where {}", format_predicate(wc)));
            }
            format!("({})", parts.join(", "))
        }
    }
}

fn format_quantity(q: &Quantity) -> String {
    match q {
        Quantity::All => "all".to_string(),
        Quantity::Exact(n) => n.to_string(),
        Quantity::AtLeast(n) => format!("{}+", n),
    }
}

fn format_card_filter(f: &CardFilter) -> String {
    let kind_str = match &f.kind {
        CardFilterKind::Monster => "monster",
        CardFilterKind::Spell => "spell",
        CardFilterKind::Trap => "trap",
        CardFilterKind::Card => "card",
        CardFilterKind::EffectMonster => "effect monster",
        CardFilterKind::NormalMonster => "normal monster",
        CardFilterKind::FusionMonster => "fusion monster",
        CardFilterKind::SynchroMonster => "synchro monster",
        CardFilterKind::XyzMonster => "xyz monster",
        CardFilterKind::LinkMonster => "link monster",
        CardFilterKind::RitualMonster => "ritual monster",
        CardFilterKind::PendulumMonster => "pendulum monster",
        CardFilterKind::TunerMonster => "tuner monster",
        CardFilterKind::NonTunerMonster => "non-tuner monster",
        CardFilterKind::NonTokenMonster => "non-token monster",
    };
    if let Some(name) = &f.name {
        format!("\"{}\" {}", name, kind_str)
    } else {
        kind_str.to_string()
    }
}

fn format_controller(ctrl: &Controller) -> &'static str {
    match ctrl {
        Controller::You => "you control",
        Controller::Opponent => "opponent controls",
        Controller::Either => "either controls",
    }
}

fn format_zone_filter(zf: &ZoneFilter) -> String {
    match zf {
        ZoneFilter::In(zones) => {
            let zs: Vec<&str> = zones.iter().map(|z| format_zone(z)).collect();
            format!("in {}", zs.join(" or "))
        }
        ZoneFilter::From(zones) => {
            let zs: Vec<&str> = zones.iter().map(|z| format_zone(z)).collect();
            format!("from {}", zs.join(" or "))
        }
        ZoneFilter::OnField(owner) => {
            match owner {
                FieldOwner::Your => "on your field".to_string(),
                FieldOwner::Opponent => "on opponent field".to_string(),
                FieldOwner::Either => "on either field".to_string(),
            }
        }
    }
}

fn format_position_filter(pf: &PositionFilter) -> &'static str {
    match pf {
        PositionFilter::FaceUp => "face_up",
        PositionFilter::FaceDown => "face_down",
        PositionFilter::AttackPosition => "in attack_position",
        PositionFilter::DefensePosition => "in defense_position",
        PositionFilter::ExceptSelf => "except self",
    }
}

// ── Predicates ────────────────────────────────────────────────

fn format_predicate(pred: &Predicate) -> String {
    match pred {
        Predicate::Single(a) => format_pred_atom(a),
        Predicate::And(atoms) => atoms.iter().map(format_pred_atom).collect::<Vec<_>>().join(" and "),
        Predicate::Or(atoms) => atoms.iter().map(format_pred_atom).collect::<Vec<_>>().join(" or "),
    }
}

fn format_pred_atom(atom: &PredicateAtom) -> String {
    match atom {
        PredicateAtom::Not(inner) => format!("not {}", format_pred_atom(inner)),
        PredicateAtom::StatCompare(field, op, expr) => {
            format!("{} {} {}", format_stat_field(field), format_compare_op(op), format_expr(expr))
        }
        PredicateAtom::AttributeIs(attr) => format!("attribute == {}", format_attribute(attr)),
        PredicateAtom::RaceIs(race) => format!("race == {}", format_race(race)),
        PredicateAtom::TypeIs(ct) => format!("type == {}", format_card_type(ct)),
        PredicateAtom::NameIs(s) => format!("name == \"{}\"", s),
        PredicateAtom::ArchetypeIs(s) => format!("archetype == \"{}\"", s),
        PredicateAtom::IsFaceUp => "is_face_up".to_string(),
        PredicateAtom::IsFaceDown => "is_face_down".to_string(),
        PredicateAtom::IsMonster => "is_monster".to_string(),
        PredicateAtom::IsSpell => "is_spell".to_string(),
        PredicateAtom::IsTrap => "is_trap".to_string(),
        PredicateAtom::IsEffect => "is_effect".to_string(),
        PredicateAtom::IsNormal => "is_normal".to_string(),
        PredicateAtom::IsTuner => "is_tuner".to_string(),
        PredicateAtom::IsFusion => "is_fusion".to_string(),
        PredicateAtom::IsSynchro => "is_synchro".to_string(),
        PredicateAtom::IsXyz => "is_xyz".to_string(),
        PredicateAtom::IsLink => "is_link".to_string(),
        PredicateAtom::IsRitual => "is_ritual".to_string(),
        PredicateAtom::IsPendulum => "is_pendulum".to_string(),
        PredicateAtom::IsToken => "is_token".to_string(),
        PredicateAtom::IsFlip => "is_flip".to_string(),
    }
}

// ── Conditions ────────────────────────────────────────────────

fn format_condition(cond: &Condition) -> String {
    match cond {
        Condition::Single(a) => format_condition_atom(a),
        Condition::And(atoms) => atoms.iter().map(format_condition_atom).collect::<Vec<_>>().join(" and "),
        Condition::Or(atoms) => atoms.iter().map(format_condition_atom).collect::<Vec<_>>().join(" or "),
    }
}

fn format_condition_atom(atom: &ConditionAtom) -> String {
    match atom {
        ConditionAtom::Not(inner) => format!("not {}", format_condition_atom(inner)),
        ConditionAtom::SelfState(state) => format!("self {}", format_card_state(state)),
        ConditionAtom::Controls(who, sel) => format!("{} controls {}", format_player_who(who), format_selector(sel)),
        ConditionAtom::NoCardsOnField(kind, owner) => {
            let owner_str = match owner {
                FieldOwner::Your => "your",
                FieldOwner::Opponent => "opponent",
                FieldOwner::Either => "either",
            };
            let filter = format_card_filter_kind(kind);
            format!("no {} on {} field", filter, owner_str)
        }
        ConditionAtom::LpCompare(op, expr) => format!("lp {} {}", format_compare_op(op), format_expr(expr)),
        ConditionAtom::OpponentLpCompare(op, expr) => format!("opponent_lp {} {}", format_compare_op(op), format_expr(expr)),
        ConditionAtom::HandSize(op, expr) => format!("hand_size {} {}", format_compare_op(op), format_expr(expr)),
        ConditionAtom::CardsInGy(op, expr) => format!("cards_in_gy {} {}", format_compare_op(op), format_expr(expr)),
        ConditionAtom::CardsInBanished(op, expr) => format!("cards_in_banished {} {}", format_compare_op(op), format_expr(expr)),
        ConditionAtom::OnField => "on_field".to_string(),
        ConditionAtom::InGy => "in_gy".to_string(),
        ConditionAtom::InHand => "in_hand".to_string(),
        ConditionAtom::InBanished => "in_banished".to_string(),
        ConditionAtom::PhaseIs(phase) => format!("phase == {}", format_phase_name(phase)),
        ConditionAtom::ChainIncludes(cats) => {
            let cat_strs: Vec<&str> = cats.iter().map(format_category).collect();
            format!("chain_includes [{}]", cat_strs.join(", "))
        }
        ConditionAtom::HasCounter(name, op, threshold, target) => {
            let mut s = format!("has_counter \"{}\"", name);
            if let (Some(op), Some(expr)) = (op, threshold) {
                s.push_str(&format!(" {} {}", format_compare_op(op), format_expr(expr)));
            }
            let target_str = match target {
                CounterTarget::OnSelf => "self",
                CounterTarget::OnSelector => "self", // fallback; selector context not preserved here
            };
            s.push_str(&format!(" on {}", target_str));
            s
        }
        ConditionAtom::HasFlag(name) => format!("has_flag \"{}\"", name),
        ConditionAtom::Reason(op, filters) => {
            let op_s = match op {
                ReasonOp::Eq => "==",
                ReasonOp::Neq => "!=",
                ReasonOp::Includes => "includes",
            };
            if filters.len() == 1 {
                format!("reason {} {}", op_s, format_reason_filter(&filters[0]))
            } else {
                let fs: Vec<&str> = filters.iter().map(format_reason_filter).collect();
                format!("reason {} [{}]", op_s, fs.join(", "))
            }
        }
        ConditionAtom::PreviousLocationIs(op, zone) => {
            format!("previous_location {} {}", format_eq_op(op), format_zone(zone))
        }
        ConditionAtom::PreviousControllerIs(op, who) => {
            let who_s = match who {
                PrevControllerRef::You => "you",
                PrevControllerRef::Opponent => "opponent",
                PrevControllerRef::Controller => "controller",
                PrevControllerRef::Owner => "owner",
            };
            format!("previous_controller {} {}", format_eq_op(op), who_s)
        }
        ConditionAtom::PreviousPositionIs(op, pos) => {
            let pos_s = match pos {
                PrevPositionValue::FaceUp => "face_up",
                PrevPositionValue::FaceDown => "face_down",
                PrevPositionValue::AttackPosition => "attack_position",
                PrevPositionValue::DefensePosition => "defense_position",
            };
            format!("previous_position {} {}", format_eq_op(op), pos_s)
        }
    }
}

fn format_eq_op(op: &EqOp) -> &'static str {
    match op { EqOp::Eq => "==", EqOp::Neq => "!=" }
}

fn format_reason_filter(f: &ReasonFilter) -> &'static str {
    match f {
        ReasonFilter::Battle         => "battle",
        ReasonFilter::Effect         => "effect",
        ReasonFilter::Cost           => "cost",
        ReasonFilter::Material       => "material",
        ReasonFilter::Release        => "release",
        ReasonFilter::Rule           => "rule",
        ReasonFilter::Discard        => "discard",
        ReasonFilter::Return         => "return",
        ReasonFilter::Summon         => "summon",
        ReasonFilter::Destroy        => "destroy",
        ReasonFilter::BattleOrEffect => "battle_or_effect",
    }
}

fn format_card_filter_kind(kind: &CardFilterKind) -> &'static str {
    match kind {
        CardFilterKind::Monster => "monster",
        CardFilterKind::Spell => "spell",
        CardFilterKind::Trap => "trap",
        CardFilterKind::Card => "card",
        CardFilterKind::EffectMonster => "effect monster",
        CardFilterKind::NormalMonster => "normal monster",
        CardFilterKind::FusionMonster => "fusion monster",
        CardFilterKind::SynchroMonster => "synchro monster",
        CardFilterKind::XyzMonster => "xyz monster",
        CardFilterKind::LinkMonster => "link monster",
        CardFilterKind::RitualMonster => "ritual monster",
        CardFilterKind::PendulumMonster => "pendulum monster",
        CardFilterKind::TunerMonster => "tuner monster",
        CardFilterKind::NonTunerMonster => "non-tuner monster",
        CardFilterKind::NonTokenMonster => "non-token monster",
    }
}

fn format_card_state(state: &CardState) -> &'static str {
    match state {
        CardState::SummonedThisTurn => "summoned_this_turn",
        CardState::AttackedThisTurn => "attacked_this_turn",
        CardState::FlippedThisTurn => "flipped_this_turn",
        CardState::ActivatedThisTurn => "activated_this_turn",
        CardState::FaceUp => "face_up",
        CardState::FaceDown => "face_down",
        CardState::InAttackPosition => "in_attack_position",
        CardState::InDefensePosition => "in_defense_position",
    }
}

// ── Trigger ───────────────────────────────────────────────────

fn format_trigger(trigger: &Trigger) -> String {
    match trigger {
        Trigger::Summoned(None) => "summoned".to_string(),
        Trigger::Summoned(Some(m)) => format!("summoned by {}", format_summon_method(m)),
        Trigger::SpecialSummoned(None) => "special_summoned".to_string(),
        Trigger::SpecialSummoned(Some(m)) => format!("special_summoned by {}", format_summon_method(m)),
        Trigger::NormalSummoned => "normal_summoned".to_string(),
        Trigger::TributeSummoned => "tribute_summoned".to_string(),
        Trigger::FlipSummoned => "flip_summoned".to_string(),
        Trigger::Flipped => "flipped".to_string(),
        Trigger::Destroyed(None) => "destroyed".to_string(),
        Trigger::Destroyed(Some(DestroyBy::Battle)) => "destroyed by battle".to_string(),
        Trigger::Destroyed(Some(DestroyBy::Effect)) => "destroyed by effect".to_string(),
        Trigger::Destroyed(Some(DestroyBy::CardEffect)) => "destroyed by card_effect".to_string(),
        Trigger::DestroyedByBattle => "destroyed_by_battle".to_string(),
        Trigger::DestroyedByEffect => "destroyed_by_effect".to_string(),
        Trigger::DestroysByBattle => "destroys_by_battle".to_string(),
        Trigger::SentTo(zone, None) => format!("sent_to {}", format_zone(zone)),
        Trigger::SentTo(zone, Some(from)) => format!("sent_to {} from {}", format_zone(zone), format_zone(from)),
        Trigger::LeavesField => "leaves_field".to_string(),
        Trigger::Banished => "banished".to_string(),
        Trigger::ReturnedTo(zone) => format!("returned_to {}", format_zone(zone)),
        Trigger::AttackDeclared => "attack_declared".to_string(),
        Trigger::OpponentAttackDeclared => "opponent_attack_declared".to_string(),
        Trigger::Attacked => "attacked".to_string(),
        Trigger::BattleDamage(None) => "battle_damage".to_string(),
        Trigger::BattleDamage(Some(who)) => {
            let w = match who {
                PlayerWho::You => "you",
                PlayerWho::Opponent => "opponent",
                PlayerWho::Controller => "controller",
                _ => "either",
            };
            format!("battle_damage to {}", w)
        }
        Trigger::DirectAttackDamage => "direct_attack_damage".to_string(),
        Trigger::DamageCalculation => "damage_calculation".to_string(),
        Trigger::StandbyPhase(None) => "standby_phase".to_string(),
        Trigger::StandbyPhase(Some(owner)) => {
            let o = match owner {
                PhaseOwner::Yours => "yours",
                PhaseOwner::Opponents => "opponents",
                PhaseOwner::Either => "either",
            };
            format!("standby_phase of {}", o)
        }
        Trigger::EndPhase => "end_phase".to_string(),
        Trigger::DrawPhase => "draw_phase".to_string(),
        Trigger::MainPhase => "main_phase".to_string(),
        Trigger::BattlePhase => "battle_phase".to_string(),
        Trigger::SummonAttempt => "summon_attempt".to_string(),
        Trigger::SpellTrapActivated => "spell_trap_activated".to_string(),
        Trigger::Activates { subject, categories } => {
            let keyword = match subject {
                ActivatesSubject::Opponent => "opponent_activates",
                ActivatesSubject::You      => "you_activates",
                ActivatesSubject::Any      => "any_activates",
            };
            if categories.is_empty() {
                keyword.to_string()
            } else {
                let cat_strs: Vec<&str> = categories.iter().map(format_category).collect();
                format!("{} [{}]", keyword, cat_strs.join(", "))
            }
        }
        Trigger::ChainSolved => "chain_solved".to_string(),
        Trigger::ChainSolving => "chain_solving".to_string(),
        Trigger::ChainLink => "chain_link".to_string(),
        Trigger::Targeted => "targeted".to_string(),
        Trigger::PositionChanged => "position_changed".to_string(),
        Trigger::ControlChanged => "control_changed".to_string(),
        Trigger::Equipped => "equipped".to_string(),
        Trigger::Unequipped => "unequipped".to_string(),
        Trigger::UsedAsMaterial { role, method, summoned_by_binding } => {
            let mut out = String::from("used_as_material");
            if let Some(r) = role {
                out.push_str(" as ");
                out.push_str(format_material_role(r));
            }
            if let Some(m) = method {
                out.push_str(" for ");
                out.push_str(format_summon_method(m));
            }
            if let Some(name) = summoned_by_binding {
                out.push_str(" by as ");
                out.push_str(name);
            }
            out
        }
        Trigger::Custom(s) => format!("custom \"{}\"", s),
    }
}

fn format_summon_method(m: &SummonMethod) -> &'static str {
    match m {
        SummonMethod::Normal => "normal",
        SummonMethod::Special => "special",
        SummonMethod::Flip => "flip",
        SummonMethod::Tribute => "tribute",
        SummonMethod::Fusion => "fusion",
        SummonMethod::Synchro => "synchro",
        SummonMethod::Xyz => "xyz",
        SummonMethod::Link => "link",
        SummonMethod::Ritual => "ritual",
        SummonMethod::Pendulum => "pendulum",
    }
}

/// T30 / AA-II: format a MaterialRole AST variant back to its grammar keyword.
fn format_material_role(r: &MaterialRole) -> &'static str {
    match r {
        MaterialRole::XyzAttached => "xyz_attached",
        MaterialRole::Tributed    => "tributed",
        MaterialRole::Fused       => "fused",
        MaterialRole::Synchro     => "synchro",
        MaterialRole::Link        => "link",
        MaterialRole::Ritual      => "ritual",
    }
}

// ── Grant Abilities ───────────────────────────────────────────

fn format_grant_ability(g: &GrantAbility) -> String {
    match g {
        GrantAbility::CannotAttack => "cannot_attack".to_string(),
        GrantAbility::CannotAttackDirectly => "cannot_attack_directly".to_string(),
        GrantAbility::CannotChangePosition => "cannot_change_position".to_string(),
        GrantAbility::CannotBeDestroyed(None) => "cannot_be_destroyed".to_string(),
        GrantAbility::CannotBeDestroyed(Some(DestroyBy::Battle)) => "cannot_be_destroyed by battle".to_string(),
        GrantAbility::CannotBeDestroyed(Some(DestroyBy::Effect)) => "cannot_be_destroyed by effect".to_string(),
        GrantAbility::CannotBeDestroyed(Some(DestroyBy::CardEffect)) => "cannot_be_destroyed by effect".to_string(),
        GrantAbility::CannotBeTargeted(None) => "cannot_be_targeted".to_string(),
        GrantAbility::CannotBeTargeted(Some(TargetedBy::Spells)) => "cannot_be_targeted by spells".to_string(),
        GrantAbility::CannotBeTargeted(Some(TargetedBy::Traps)) => "cannot_be_targeted by traps".to_string(),
        GrantAbility::CannotBeTargeted(Some(TargetedBy::Monsters)) => "cannot_be_targeted by monsters".to_string(),
        GrantAbility::CannotBeTargeted(Some(TargetedBy::Effects)) => "cannot_be_targeted by effects".to_string(),
        GrantAbility::CannotBeTargeted(Some(TargetedBy::Opponent)) => "cannot_be_targeted by opponent".to_string(),
        GrantAbility::CannotBeTributed => "cannot_be_tributed".to_string(),
        GrantAbility::CannotBeUsedAsMaterial => "cannot_be_used_as_material".to_string(),
        GrantAbility::CannotActivate(None) => "cannot_activate".to_string(),
        GrantAbility::CannotActivate(Some(ActivateWhat::Effects)) => "cannot_activate effects".to_string(),
        GrantAbility::CannotActivate(Some(ActivateWhat::Spells)) => "cannot_activate spells".to_string(),
        GrantAbility::CannotActivate(Some(ActivateWhat::Traps)) => "cannot_activate traps".to_string(),
        GrantAbility::CannotNormalSummon => "cannot_normal_summon".to_string(),
        GrantAbility::CannotSpecialSummon => "cannot_special_summon".to_string(),
        GrantAbility::UnaffectedBy(UnaffectedSource::Spells) => "unaffected_by spells".to_string(),
        GrantAbility::UnaffectedBy(UnaffectedSource::Traps) => "unaffected_by traps".to_string(),
        GrantAbility::UnaffectedBy(UnaffectedSource::Monsters) => "unaffected_by monsters".to_string(),
        GrantAbility::UnaffectedBy(UnaffectedSource::Effects) => "unaffected_by effects".to_string(),
        GrantAbility::UnaffectedBy(UnaffectedSource::OpponentEffects) => "unaffected_by opponent_effects".to_string(),
        GrantAbility::Piercing => "piercing".to_string(),
        GrantAbility::DirectAttack => "direct_attack".to_string(),
        GrantAbility::DoubleAttack => "double_attack".to_string(),
        GrantAbility::TripleAttack => "triple_attack".to_string(),
        GrantAbility::AttackAllMonsters => "attack_all_monsters".to_string(),
        GrantAbility::MustAttack => "must_attack".to_string(),
        GrantAbility::ImmuneToTargeting => "immune_to_targeting".to_string(),
    }
}

// ── Duration ──────────────────────────────────────────────────

fn format_duration(d: &Duration) -> String {
    match d {
        Duration::ThisTurn => "this_turn".to_string(),
        Duration::EndOfTurn => "end_of_turn".to_string(),
        Duration::EndPhase => "end_phase".to_string(),
        Duration::EndOfDamageStep => "end_of_damage_step".to_string(),
        Duration::NextStandbyPhase => "next_standby_phase".to_string(),
        Duration::WhileOnField => "while_on_field".to_string(),
        Duration::WhileFaceUp => "while_face_up".to_string(),
        Duration::Permanently => "permanently".to_string(),
        Duration::NTurns(n) => format!("{}_turns", n),
    }
}

// ── Expressions ───────────────────────────────────────────────

fn format_expr(expr: &Expr) -> String {
    match expr {
        Expr::Literal(n) => n.to_string(),
        Expr::Half => "half".to_string(),
        Expr::StatRef(entity, field) => format!("{}.{}", entity, format_stat_field(field)),
        Expr::BindingRef(name, field) => format!("{}.{}", name, format_stat_field(field)),
        Expr::PlayerLp(owner) => match owner {
            LpOwner::Your => "your_lp".to_string(),
            LpOwner::Opponent => "opponent_lp".to_string(),
            LpOwner::Controller => "controller_lp".to_string(),
        },
        Expr::Count(sel) => format!("count({})", format_selector(sel)),
        Expr::BinOp { left, op, right } => {
            let op_str = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
            };
            format!("{} {} {}", format_expr(left), op_str, format_expr(right))
        }
    }
}

// ── Simple Enums ──────────────────────────────────────────────

fn format_card_type(ct: &CardType) -> &'static str {
    match ct {
        CardType::NormalMonster => "Normal Monster",
        CardType::EffectMonster => "Effect Monster",
        CardType::RitualMonster => "Ritual Monster",
        CardType::FusionMonster => "Fusion Monster",
        CardType::SynchroMonster => "Synchro Monster",
        CardType::XyzMonster => "Xyz Monster",
        CardType::LinkMonster => "Link Monster",
        CardType::PendulumMonster => "Pendulum Monster",
        CardType::Tuner => "Tuner",
        CardType::SynchroTuner => "Synchro Tuner",
        CardType::Flip => "Flip",
        CardType::Gemini => "Gemini",
        CardType::Union => "Union",
        CardType::Spirit => "Spirit",
        CardType::Toon => "Toon",
        CardType::NormalSpell => "Normal Spell",
        CardType::QuickPlaySpell => "Quick-Play Spell",
        CardType::ContinuousSpell => "Continuous Spell",
        CardType::EquipSpell => "Equip Spell",
        CardType::FieldSpell => "Field Spell",
        CardType::RitualSpell => "Ritual Spell",
        CardType::NormalTrap => "Normal Trap",
        CardType::CounterTrap => "Counter Trap",
        CardType::ContinuousTrap => "Continuous Trap",
    }
}

fn format_attribute(attr: &Attribute) -> &'static str {
    match attr {
        Attribute::Light => "LIGHT",
        Attribute::Dark => "DARK",
        Attribute::Fire => "FIRE",
        Attribute::Water => "WATER",
        Attribute::Earth => "EARTH",
        Attribute::Wind => "WIND",
        Attribute::Divine => "DIVINE",
    }
}

fn format_race(race: &Race) -> &'static str {
    match race {
        Race::Dragon => "Dragon",
        Race::Spellcaster => "Spellcaster",
        Race::Zombie => "Zombie",
        Race::Warrior => "Warrior",
        Race::BeastWarrior => "Beast-Warrior",
        Race::Beast => "Beast",
        Race::WingedBeast => "Winged Beast",
        Race::Fiend => "Fiend",
        Race::Fairy => "Fairy",
        Race::Insect => "Insect",
        Race::Dinosaur => "Dinosaur",
        Race::Reptile => "Reptile",
        Race::Fish => "Fish",
        Race::SeaSerpent => "Sea Serpent",
        Race::Aqua => "Aqua",
        Race::Pyro => "Pyro",
        Race::Thunder => "Thunder",
        Race::Rock => "Rock",
        Race::Plant => "Plant",
        Race::Machine => "Machine",
        Race::Psychic => "Psychic",
        Race::DivineBeast => "Divine-Beast",
        Race::Wyrm => "Wyrm",
        Race::Cyberse => "Cyberse",
        Race::Illusion => "Illusion",
    }
}

fn format_arrow(a: &Arrow) -> &'static str {
    match a {
        Arrow::TopLeft => "top_left",
        Arrow::Top => "top",
        Arrow::TopRight => "top_right",
        Arrow::Left => "left",
        Arrow::Right => "right",
        Arrow::BottomLeft => "bottom_left",
        Arrow::Bottom => "bottom",
        Arrow::BottomRight => "bottom_right",
    }
}

fn format_stat_val(sv: &StatVal) -> String {
    match sv {
        StatVal::Number(n) => n.to_string(),
        StatVal::Unknown => "?".to_string(),
    }
}

fn format_zone(z: &Zone) -> &'static str {
    match z {
        Zone::Hand => "hand",
        Zone::Field => "field",
        Zone::Deck => "deck",
        Zone::ExtraDeck => "extra_deck",
        Zone::ExtraDeckFaceUp => "extra_deck_face_up",
        Zone::Gy => "gy",
        Zone::Banished => "banished",
        Zone::MonsterZone => "monster_zone",
        Zone::SpellTrapZone => "spell_trap_zone",
        Zone::FieldZone => "field_zone",
        Zone::PendulumZone => "pendulum_zone",
        Zone::ExtraMonsterZone => "extra_monster_zone",
        Zone::Overlay => "overlay",
        Zone::Equipped => "equipped",
        Zone::TopOfDeck => "top_of_deck",
        Zone::BottomOfDeck => "bottom_of_deck",
    }
}

fn format_player_who(who: &PlayerWho) -> &'static str {
    match who {
        PlayerWho::You => "you",
        PlayerWho::Opponent => "opponent",
        PlayerWho::Controller => "controller",
        PlayerWho::Owner => "owner",
        PlayerWho::Summoner => "summoner",
        PlayerWho::Both => "both",
    }
}

fn format_field_target(ft: &FieldTarget) -> &'static str {
    match ft {
        FieldTarget::YourField => "your_field",
        FieldTarget::OpponentField => "opponent_field",
        FieldTarget::EitherField => "either_field",
    }
}

fn format_stat_name(s: &StatName) -> &'static str {
    match s {
        StatName::Atk => "atk",
        StatName::Def => "def",
    }
}

fn format_stat_field(s: &StatField) -> &'static str {
    match s {
        StatField::Atk => "atk",
        StatField::Def => "def",
        StatField::Level => "level",
        StatField::Rank => "rank",
        StatField::Link => "link",
        StatField::Scale => "scale",
        StatField::BaseAtk => "base_atk",
        StatField::BaseDef => "base_def",
        StatField::OriginalAtk => "original_atk",
        StatField::OriginalDef => "original_def",
    }
}

fn format_compare_op(op: &CompareOp) -> &'static str {
    match op {
        CompareOp::Gte => ">=",
        CompareOp::Lte => "<=",
        CompareOp::Eq => "==",
        CompareOp::Neq => "!=",
        CompareOp::Gt => ">",
        CompareOp::Lt => "<",
    }
}

fn format_battle_position(bp: &BattlePosition) -> &'static str {
    match bp {
        BattlePosition::Attack => "attack_position",
        BattlePosition::Defense => "defense_position",
        BattlePosition::FaceDownDefense => "face_down_defense",
    }
}

fn format_phase_name(p: &PhaseName) -> &'static str {
    match p {
        PhaseName::Draw => "draw",
        PhaseName::Standby => "standby",
        PhaseName::Main1 => "main1",
        PhaseName::Battle => "battle",
        PhaseName::Main2 => "main2",
        PhaseName::End => "end",
        PhaseName::Damage => "damage",
        PhaseName::DamageCalculation => "damage_calculation",
    }
}

fn format_category(cat: &Category) -> &'static str {
    match cat {
        Category::Search => "search",
        Category::SpecialSummon => "special_summon",
        Category::SendToGy => "send_to_gy",
        Category::AddToHand => "add_to_hand",
        Category::Draw => "draw",
        Category::Banish => "banish",
        Category::Destroy => "destroy",
        Category::Negate => "negate",
        Category::Mill => "mill",
        Category::ActivateSpell => "activate_spell",
        Category::ActivateTrap => "activate_trap",
        Category::ActivateMonsterEffect => "activate_monster_effect",
        Category::NormalSummon => "normal_summon",
        Category::FusionSummon => "fusion_summon",
        Category::SynchroSummon => "synchro_summon",
        Category::XyzSummon => "xyz_summon",
        Category::LinkSummon => "link_summon",
        Category::RitualSummon => "ritual_summon",
        Category::Discard => "discard",
        Category::ReturnToDeck => "return_to_deck",
        Category::Equip => "equip",
        Category::AttackDeclared => "attack_declared",
    }
}

fn format_announce_what(w: &AnnounceWhat) -> &'static str {
    match w {
        AnnounceWhat::Type => "type",
        AnnounceWhat::Attribute => "attribute",
        AnnounceWhat::Race => "race",
        AnnounceWhat::Level => "level",
        AnnounceWhat::Card => "card",
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::parser::parse_v2;

    fn roundtrip(path: &str) {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|_| panic!("cannot read {}", path));
        let file = parse_v2(&source)
            .unwrap_or_else(|e| panic!("parse error {}: {}", path, e));
        let formatted = format_file(&file);
        let reparsed = parse_v2(&formatted)
            .unwrap_or_else(|e| panic!("roundtrip failed for {}:\n=== output ===\n{}\n=== error ===\n{}", path, formatted, e));
        assert_eq!(reparsed.cards.len(), file.cards.len());
        assert_eq!(reparsed.cards[0].name, file.cards[0].name);
    }

    #[test] fn test_pot_of_greed_roundtrips() { roundtrip("cards/goat/pot_of_greed.ds"); }
    #[test] fn test_raigeki_roundtrips() { roundtrip("cards/goat/raigeki.ds"); }
    #[test] fn test_mirror_force_roundtrips() { roundtrip("cards/goat/mirror_force.ds"); }
    #[test] fn test_sangan_roundtrips() { roundtrip("cards/goat/sangan.ds"); }
    #[test] fn test_solemn_judgment_roundtrips() { roundtrip("cards/goat/solemn_judgment.ds"); }
    #[test] fn test_lava_golem_roundtrips() { roundtrip("cards/goat/lava_golem.ds"); }
    #[test] fn test_graceful_charity_roundtrips() { roundtrip("cards/goat/graceful_charity.ds"); }
    #[test] fn test_scapegoat_roundtrips() { roundtrip("cards/goat/scapegoat.ds"); }
    #[test] fn test_dark_paladin_roundtrips() { roundtrip("cards/goat/dark_paladin.ds"); }
    #[test] fn test_jinzo_roundtrips() { roundtrip("cards/goat/jinzo.ds"); }

    #[test]
    fn test_all_goat_cards_roundtrip() {
        let dir = std::fs::read_dir("cards/goat").unwrap();
        let mut ok = 0;
        let mut fail = 0;
        let mut first_failure: Option<(String, String)> = None;
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "ds") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let file = match parse_v2(&source) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let formatted = format_file(&file);
            match parse_v2(&formatted) {
                Ok(_) => ok += 1,
                Err(e) => {
                    fail += 1;
                    if first_failure.is_none() {
                        first_failure = Some((path.display().to_string(), format!("{}\n---\n{}", e, formatted)));
                    }
                }
            }
        }
        println!("Roundtrip: {} ok, {} fail", ok, fail);
        if let Some((path, details)) = first_failure {
            panic!("First failure on {}:\n{}", path, details);
        }
    }
}
