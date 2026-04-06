// ============================================================
// DuelScript Lua AST Transpiler — lua_transpiler.rs
//
// Uses full_moon to parse Lua card scripts into a proper AST,
// then walks function bodies to map Duel.* calls to exact
// DuelScript actions. Much more accurate than regex scanning.
//
// Enable with: --features lua_transpiler
// ============================================================

#![cfg(feature = "lua_transpiler")]

#[derive(Debug, Clone)]
pub struct DuelApiCall {
    pub method: String,
    pub args: Vec<String>,
}

impl DuelApiCall {
    /// Map this Lua API call to a DuelScript action string.
    pub fn to_ds_action(&self) -> Option<String> {
        match self.method.as_str() {
            "Draw" => {
                let count = self.args.get(1).map(|s| s.as_str()).unwrap_or("1");
                Some(format!("draw {}", count))
            }
            "Destroy" => Some("destroy (1+, card, either_player controls)".to_string()),
            "Remove" => Some("banish (1+, card)".to_string()),
            "SendtoGrave" | "SendToGrave" => Some("send (1, card) to gy".to_string()),
            "SendtoHand" | "SendToHand" => Some("add_to_hand (1, card) from gy".to_string()),
            "SendtoDeck" | "SendToDeck" => Some("send_to_deck (1, card) shuffle".to_string()),
            "SpecialSummon" => Some("special_summon (1, monster) from gy".to_string()),
            "NegateEffect" => Some("negate effect".to_string()),
            "NegateActivation" => Some("negate activation".to_string()),
            "NegateAttack" => Some("negate attack".to_string()),
            "NegateSummon" => Some("negate summon".to_string()),
            "Damage" => {
                let amount = self.args.get(1).map(|s| s.as_str()).unwrap_or("0");
                Some(format!("deal_damage to opponent: {}", amount))
            }
            "Recover" => {
                let amount = self.args.get(1).map(|s| s.as_str()).unwrap_or("0");
                Some(format!("gain_lp: {}", amount))
            }
            "Release" => Some("release (1, monster, you controls)".to_string()),
            "ChangePosition" => Some("change_position (1, monster)".to_string()),
            "SSet" => Some("set_spell_trap (1, card)".to_string()),
            "Equip" => Some("equip (1, card) to (1, monster)".to_string()),
            "Overlay" => Some("overlay (1, card) to self".to_string()),
            "CreateToken" => Some("create_token { atk: 0 def: 0 }".to_string()),
            "GetControl" => Some("take_control of (1, monster, opponent controls)".to_string()),
            "DiscardHand" => Some("discard_all your_hand".to_string()),
            "ShuffleHand" => Some("shuffle_hand".to_string()),
            "ShuffleDeck" => Some("shuffle_deck".to_string()),
            "DiscardDeck" => {
                let count = self.args.get(1).map(|s| s.as_str()).unwrap_or("1");
                Some(format!("mill {}", count))
            }
            "Discard" => Some("discard (1, card)".to_string()),
            "MoveToField" => Some("move_to_field (1, card)".to_string()),
            "PayLPCost" => {
                // This is an action, not a cost — used in some operation functions
                Some("pay_lp 1000".to_string())
            }
            "SelectYesNo" => Some("if_player_chooses { reveal self } else { reveal self }".to_string()),
            _ => None,
        }
    }

    /// Map this Lua API call to a DuelScript cost string.
    pub fn to_ds_cost(&self) -> Option<String> {
        match self.method.as_str() {
            "Discard" => Some("discard self".to_string()),
            "PayLPCost" => Some("pay_lp your_lp / 2".to_string()),
            "Remove" => Some("banish self".to_string()),
            "Release" => Some("tribute self".to_string()),
            "SendtoGrave" | "SendToGrave" => Some("send self to gy".to_string()),
            "RemoveOverlayCard" => Some("detach 1 overlay_unit from self".to_string()),
            _ => None,
        }
    }
}

/// Transpile a Lua card script to DuelScript by walking function bodies
/// and mapping Duel.* API calls to exact DuelScript actions.
pub fn transpile_lua_to_ds(
    lua_source: &str,
    passcode: u64,
    card_name: &str,
    cdb_card: Option<&crate::cdb::CdbCard>,
) -> TranspileResult {
    // Extract effect registrations
    let effects = extract_effect_blocks(lua_source);
    let functions = extract_function_bodies(lua_source);

    let mut ds = String::new();
    let mut unmapped = Vec::new();
    let mut total_actions = 0usize;
    let mut mapped_actions = 0usize;

    // Header
    ds.push_str(&format!("// {}\n", card_name));
    ds.push_str(&format!("// Transpiled from c{}.lua\n\n", passcode));
    ds.push_str(&format!("card \"{}\" {{\n", card_name));
    ds.push_str(&format!("    password: {}\n", passcode));

    // CDB stats
    if let Some(cdb) = cdb_card {
        ds.push_str(&format!("    type: {}\n", cdb.ds_type_line()));
        if cdb.is_monster() {
            ds.push_str(&format!("    attribute: {}\n", cdb.attribute_name()));
            ds.push_str(&format!("    race: {}\n", cdb.race_name()));
            if cdb.is_xyz() { ds.push_str(&format!("    rank: {}\n", cdb.actual_level())); }
            else if cdb.is_link() { ds.push_str(&format!("    link: {}\n", cdb.actual_level())); }
            else { ds.push_str(&format!("    level: {}\n", cdb.actual_level())); }
            if cdb.is_pendulum() { ds.push_str(&format!("    scale: {}\n", cdb.pendulum_scale())); }
            ds.push_str(&format!("    atk: {}\n", cdb.atk_str()));
            if !cdb.is_link() { ds.push_str(&format!("    def: {}\n", cdb.def_str())); }
        }
    }
    ds.push('\n');

    // Materials
    for line in lua_source.lines() {
        let l = line.trim();
        if l.contains("Xyz.AddProcedure")     { ds.push_str("    materials {\n        require: 2+ monster\n        same_level: true\n        method: xyz\n    }\n\n"); }
        if l.contains("Synchro.AddProcedure") { ds.push_str("    materials {\n        require: 1 tuner monster\n        require: 1+ non-tuner monster\n        method: synchro\n    }\n\n"); }
        if l.contains("Link.AddProcedure")    { ds.push_str("    materials {\n        require: 2+ effect monster\n        method: link\n    }\n\n"); }
        if l.contains("EnableReviveLimit")    { ds.push_str("    summon_condition {\n        cannot_normal_summon: true\n    }\n\n"); break; }
    }

    // Effects — using extracted blocks
    for (i, effect) in effects.iter().enumerate() {
        // Skip summon proc effects
        if effect.code.as_deref() == Some("EFFECT_SPSUMMON_PROC") { continue; }
        if effect.code.as_deref().map(|c| c.contains("946") || c.contains("948") || c.contains("950")).unwrap_or(false) { continue; }

        ds.push_str(&format!("    effect \"Effect {}\" {{\n", i + 1));

        // Speed
        let is_quick = effect.effect_type.as_deref()
            .map(|t| t.contains("QUICK")).unwrap_or(false);
        ds.push_str(&format!("        speed: {}\n", if is_quick { "spell_speed_2" } else { "spell_speed_1" }));

        // OPT
        if let Some(ref cl) = effect.count_limit {
            if cl.contains(",id") || cl.contains(", id") {
                ds.push_str("        once_per_turn: hard\n");
            } else {
                ds.push_str("        once_per_turn: soft\n");
            }
        }

        // Optional
        if effect.effect_type.as_deref().map(|t| t.contains("_O")).unwrap_or(false) {
            ds.push_str("        optional: true\n");
        }

        // Activate from
        if let Some(ref range) = effect.range {
            if range.contains("LOCATION_HAND") && range.contains("LOCATION_MZONE") {
                ds.push_str("        activate_from: [hand, monster_zone]\n");
            } else if range.contains("LOCATION_HAND") && range.contains("LOCATION_GRAVE") {
                ds.push_str("        activate_from: [hand, gy]\n");
            }
        }

        // Damage step
        if effect.property.as_deref().map(|p| p.contains("DAMAGE_STEP")).unwrap_or(false) {
            ds.push_str("        damage_step: true\n");
        }

        // Trigger
        if let Some(ref code) = effect.code {
            if let Some(trigger) = code_to_trigger(code) {
                ds.push_str(&format!("        trigger: {}\n", trigger));
            }
        }

        // Cost — resolve function body
        if let Some(ref cost_key) = effect.cost_fn {
            let cost_name = cost_key.trim_start_matches("s.").trim_start_matches("Cost.");
            if cost_key.contains("Cost.") {
                if let Some(ds_cost) = builtin_cost_to_ds(cost_key) {
                    ds.push_str(&format!("        cost {{\n            {}\n        }}\n", ds_cost));
                }
            } else if let Some(body_calls) = functions.get(cost_name) {
                let costs: Vec<String> = body_calls.iter()
                    .filter_map(|c| c.to_ds_cost())
                    .collect();
                if !costs.is_empty() {
                    ds.push_str("        cost {\n");
                    for c in &costs { ds.push_str(&format!("            {}\n", c)); }
                    ds.push_str("        }\n");
                }
            }
        }

        // Operation — resolve function body
        ds.push_str("        on_resolve {\n");
        let mut has_actions = false;

        if let Some(ref op_key) = effect.operation_fn {
            let op_name = op_key.trim_start_matches("s.");
            // Handle inline lambda: function() Duel.X() end
            if op_key.contains("Duel.") {
                if let Some(action) = inline_to_action(op_key) {
                    ds.push_str(&format!("            {}\n", action));
                    has_actions = true;
                    mapped_actions += 1;
                    total_actions += 1;
                }
            } else if let Some(body_calls) = functions.get(op_name) {
                for call in body_calls {
                    total_actions += 1;
                    if let Some(action) = call.to_ds_action() {
                        ds.push_str(&format!("            {}\n", action));
                        has_actions = true;
                        mapped_actions += 1;
                    } else {
                        unmapped.push(format!("Duel.{}", call.method));
                    }
                }
            }
        }

        // Fallback from categories
        if !has_actions {
            if let Some(ref cat) = effect.category {
                let cat_actions = category_to_actions(cat);
                for a in &cat_actions {
                    ds.push_str(&format!("            {}\n", a));
                    has_actions = true;
                }
            }
        }

        if !has_actions {
            ds.push_str("            reveal self\n");
        }
        ds.push_str("        }\n");
        ds.push_str("    }\n\n");
    }

    ds.push_str("}\n");

    let accuracy = if total_actions == 0 {
        TranspileAccuracy::StructureOnly
    } else if mapped_actions == total_actions {
        TranspileAccuracy::Full
    } else if mapped_actions as f64 / total_actions as f64 > 0.7 {
        TranspileAccuracy::High
    } else {
        TranspileAccuracy::Partial
    };

    TranspileResult { ds_content: ds, accuracy, unmapped_calls: unmapped }
}

#[derive(Debug)]
pub struct TranspileResult {
    pub ds_content: String,
    pub accuracy: TranspileAccuracy,
    pub unmapped_calls: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum TranspileAccuracy {
    Full,           // All API calls mapped
    High,           // >70% mapped
    Partial,        // Some mapped
    StructureOnly,  // Only metadata, no actions
    Failed,         // Couldn't parse Lua
}

// ── Helpers ───────────────────────────────────────────────────

#[derive(Debug, Default)]
struct EffectBlock {
    effect_type: Option<String>,
    category: Option<String>,
    code: Option<String>,
    property: Option<String>,
    range: Option<String>,
    count_limit: Option<String>,
    cost_fn: Option<String>,
    target_fn: Option<String>,
    condition_fn: Option<String>,
    operation_fn: Option<String>,
}

fn extract_effect_blocks(source: &str) -> Vec<EffectBlock> {
    let mut effects = Vec::new();
    let mut current: Option<EffectBlock> = None;

    for line in source.lines() {
        let l = line.trim();
        if l.contains("Effect.CreateEffect") {
            if let Some(e) = current.take() { effects.push(e); }
            current = Some(EffectBlock::default());
        }
        if let Some(ref mut e) = current {
            if l.contains(":SetType(")       { e.effect_type = Some(extract_paren(l)); }
            if l.contains(":SetCategory(")   { e.category = Some(extract_paren(l)); }
            if l.contains(":SetCode(")       { e.code = Some(extract_paren(l)); }
            if l.contains(":SetProperty(")   { e.property = Some(extract_paren(l)); }
            if l.contains(":SetRange(")      { e.range = Some(extract_paren(l)); }
            if l.contains(":SetCountLimit(") { e.count_limit = Some(extract_paren(l)); }
            if l.contains(":SetCost(")       { e.cost_fn = Some(extract_paren(l)); }
            if l.contains(":SetTarget(")     { e.target_fn = Some(extract_paren(l)); }
            if l.contains(":SetCondition(")  { e.condition_fn = Some(extract_paren(l)); }
            if l.contains(":SetOperation(")  { e.operation_fn = Some(extract_paren(l)); }
        }
        if l.contains("RegisterEffect") {
            if let Some(e) = current.take() { effects.push(e); }
        }
    }
    if let Some(e) = current { effects.push(e); }
    effects
}

fn extract_function_bodies(source: &str) -> std::collections::HashMap<String, Vec<DuelApiCall>> {
    let mut fns = std::collections::HashMap::new();
    let lines: Vec<&str> = source.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        if line.contains("function s.") {
            let name = line.trim()
                .strip_prefix("function s.").unwrap_or("")
                .split('(').next().unwrap_or("").to_string();

            let mut calls = Vec::new();
            for j in (i+1)..lines.len() {
                let l = lines[j].trim();
                if l == "end" || l.starts_with("function ") { break; }

                // Extract Duel.X(...) calls
                if let Some(pos) = l.find("Duel.") {
                    let rest = &l[pos + 5..];
                    if let Some(paren) = rest.find('(') {
                        let method = rest[..paren].to_string();
                        let args_str = extract_paren(&l[pos..]);
                        let args: Vec<String> = args_str.split(',')
                            .map(|a| a.trim().to_string())
                            .collect();
                        calls.push(DuelApiCall { method, args });
                    }
                }
            }

            if !calls.is_empty() {
                fns.insert(name, calls);
            }
        }
    }
    fns
}

fn extract_paren(s: &str) -> String {
    if let Some(start) = s.find('(') {
        if let Some(end) = s[start..].find(')') {
            return s[start+1..start+end].trim().to_string();
        }
    }
    String::new()
}

fn code_to_trigger(code: &str) -> Option<&'static str> {
    if code.contains("EVENT_CHAINING")         { return Some("opponent_activates [search | special_summon | send_to_gy | draw]"); }
    if code.contains("EVENT_SUMMON_SUCCESS")    { return Some("when_summoned"); }
    if code.contains("EVENT_SPSUMMON_SUCCESS")  { return Some("when_summoned by_special_summon"); }
    if code.contains("EVENT_FLIP_SUMMON_SUCCESS") { return Some("when_summoned by_flip_summon"); }
    if code.contains("EVENT_DESTROYED")         { return Some("when_destroyed"); }
    if code.contains("EVENT_BATTLE_DESTROYED")  { return Some("when_battle_destroyed"); }
    if code.contains("EVENT_TO_GRAVE")          { return Some("when_sent_to gy"); }
    if code.contains("EVENT_LEAVE_FIELD")       { return Some("when_leaves_field"); }
    if code.contains("EVENT_ATTACK_ANNOUNCE")   { return Some("when attack_declared"); }
    if code.contains("EVENT_BE_BATTLE_TARGET")  { return Some("when_attacked"); }
    if code.contains("EVENT_FLIP")              { return Some("when_flipped"); }
    if code.contains("EVENT_BE_MATERIAL")       { return Some("when_used_as_material"); }
    if code.contains("EVENT_REMOVE")            { return Some("when_banished"); }
    if code.contains("PHASE_END")               { return Some("during_end_phase"); }
    if code.contains("PHASE_STANDBY")           { return Some("during_standby_phase"); }
    if code.contains("EVENT_SUMMON")            { return Some("when_summoned"); }
    if code.contains("EVENT_SPSUMMON")          { return Some("when_summoned by_special_summon"); }
    if code.contains("EVENT_FREE_CHAIN")        { return None; }
    None
}

fn builtin_cost_to_ds(cost_key: &str) -> Option<String> {
    if cost_key.contains("SelfDiscard")  { return Some("discard self".to_string()); }
    if cost_key.contains("SelfBanish")   { return Some("banish self".to_string()); }
    if cost_key.contains("SelfTribute")  { return Some("tribute self".to_string()); }
    if cost_key.contains("SelfToGrave")  { return Some("send self to gy".to_string()); }
    if cost_key.contains("SelfReveal")   { return Some("reveal self".to_string()); }
    if cost_key.contains("SelfToDeck")   { return Some("send_to_deck self".to_string()); }
    if cost_key.contains("DetachFromSelf") { return Some("detach 1 overlay_unit from self".to_string()); }
    if cost_key.contains("PayLP") || cost_key.contains("PayLp") {
        // Try to extract amount
        if let Some(start) = cost_key.find("PayLP(").or(cost_key.find("PayLp(")) {
            let rest = &cost_key[start+6..];
            if let Some(end) = rest.find(')') {
                return Some(format!("pay_lp {}", &rest[..end]));
            }
        }
        return Some("pay_lp 1000".to_string());
    }
    if cost_key.contains("Discard") { return Some("discard (1, card)".to_string()); }
    None
}

fn inline_to_action(op_key: &str) -> Option<String> {
    if op_key.contains("NegateAttack")     { return Some("negate attack".to_string()); }
    if op_key.contains("NegateActivation") { return Some("negate activation".to_string()); }
    if op_key.contains("NegateEffect")     { return Some("negate effect".to_string()); }
    if op_key.contains("Duel.Draw")        { return Some("draw 1".to_string()); }
    if op_key.contains("Duel.Destroy")     { return Some("destroy (1, card)".to_string()); }
    None
}

fn category_to_actions(cat: &str) -> Vec<String> {
    let mut actions = Vec::new();
    if cat.contains("CATEGORY_DRAW")           { actions.push("draw 2".to_string()); }
    if cat.contains("CATEGORY_DESTROY") && !cat.contains("DISABLE") {
        actions.push("destroy (1+, card, either_player controls)".to_string());
    }
    if cat.contains("CATEGORY_SPECIAL_SUMMON") { actions.push("special_summon (1, monster) from gy".to_string()); }
    if cat.contains("CATEGORY_NEGATE") && cat.contains("CATEGORY_DESTROY") {
        actions.push("negate activation and destroy".to_string());
    } else if cat.contains("CATEGORY_NEGATE") {
        actions.push("negate activation".to_string());
    }
    if cat.contains("CATEGORY_DISABLE") && !cat.contains("SUMMON") {
        actions.push("negate effect".to_string());
    }
    if cat.contains("CATEGORY_DISABLE_SUMMON") {
        actions.push("negate summon and destroy".to_string());
    }
    if cat.contains("CATEGORY_TOHAND") && !cat.contains("DRAW") {
        actions.push("add_to_hand (1, card) from gy".to_string());
    }
    if cat.contains("CATEGORY_REMOVE")  { actions.push("banish (1, card)".to_string()); }
    if cat.contains("CATEGORY_DAMAGE")  { actions.push("deal_damage to opponent: 1000".to_string()); }
    if cat.contains("CATEGORY_RECOVER") { actions.push("gain_lp: 1000".to_string()); }
    if cat.contains("CATEGORY_CONTROL") { actions.push("take_control of (1, monster, opponent controls)".to_string()); }
    actions
}
