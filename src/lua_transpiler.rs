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
                // Only use numeric count; fallback to 1 for variable names
                let count = self.args.get(1)
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(1);
                Some(format!("draw {}", count))
            }
            "Destroy" => Some("destroy (1+, card, either_player controls)".to_string()),
            "Remove" => Some("banish (1+, card)".to_string()),
            "SendtoGrave" | "SendToGrave" => Some("send (1, card) to gy".to_string()),
            "SendtoHand" | "SendToHand" => Some("add_to_hand (1, card) from gy".to_string()),
            "SendtoDeck" | "SendToDeck" => Some("return (1, card) to deck shuffle".to_string()),
            "SpecialSummon" => Some("special_summon (1, monster) from gy".to_string()),
            "NegateEffect" => Some("negate effect".to_string()),
            "NegateActivation" => Some("negate activation".to_string()),
            "NegateAttack" => Some("negate attack".to_string()),
            "NegateSummon" => Some("negate summon".to_string()),
            "Damage" => {
                let amount = self.args.get(1)
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(1000);
                Some(format!("deal_damage to opponent: {}", amount))
            }
            "Recover" => {
                let amount = self.args.get(1)
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(1000);
                Some(format!("gain_lp: {}", amount))
            }
            "DiscardDeck" => {
                let count = self.args.get(1)
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(1);
                Some(format!("mill {}", count))
            }
            "Release" => Some("tribute (1, monster, you controls)".to_string()),
            "ChangePosition" => Some("change_battle_position (1, monster)".to_string()),
            "SSet" => Some("set (1, card)".to_string()),
            "Equip" => Some("equip (1, card) to (1, monster)".to_string()),
            "Overlay" => Some("attach (1, card) to self as_material".to_string()),
            "CreateToken" => Some("create_token { atk: 0 def: 0 }".to_string()),
            "GetControl" => Some("take_control of (1, monster, opponent controls)".to_string()),
            "DiscardHand" => Some("discard (1, card)".to_string()),
            "ShuffleHand" => Some("shuffle deck".to_string()),
            "ShuffleDeck" => Some("shuffle deck".to_string()),
            "Discard" => Some("discard (1, card)".to_string()),
            "MoveToField" => Some("special_summon (1, monster) from gy".to_string()),
            "PayLPCost" => None, // pay_lp belongs in cost blocks, not on_resolve
            "SelectYesNo" => None, // Engine handles player choice
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

    // Header — escape quotes in card name
    let safe_name = card_name.replace('"', "'");
    ds.push_str(&format!("// {}\n", safe_name));
    ds.push_str(&format!("// Transpiled from c{}.lua\n\n", passcode));
    ds.push_str(&format!("card \"{}\" {{\n", safe_name));
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

    // v0.6: Emit raw_effect blocks with exact Lua bitfields
    // This preserves the exact effect_type/category/code/range/count_limit
    // from the Lua script, bypassing type_mapper inference entirely.
    for (i, effect) in effects.iter().enumerate() {
        // Skip summon proc effects (handled by materials block)
        if effect.code.as_deref() == Some("EFFECT_SPSUMMON_PROC") { continue; }
        if effect.code.as_deref().map(|c| c.contains("946") || c.contains("948") || c.contains("950")).unwrap_or(false) { continue; }

        let effect_type = resolve_lua_constant_expr(effect.effect_type.as_deref().unwrap_or("0"));
        let category    = resolve_lua_constant_expr(effect.category.as_deref().unwrap_or("0"));
        let code        = resolve_lua_constant_expr(effect.code.as_deref().unwrap_or("0"));
        let property    = resolve_lua_constant_expr(effect.property.as_deref().unwrap_or("0"));
        let range       = resolve_lua_constant_expr(effect.range.as_deref().unwrap_or("0"));

        ds.push_str(&format!("    raw_effect \"Effect {}\" {{\n", i + 1));
        if effect_type != 0 { ds.push_str(&format!("        effect_type: {}\n", effect_type)); }
        if category != 0    { ds.push_str(&format!("        category: {}\n", category)); }
        if code != 0        { ds.push_str(&format!("        code: {}\n", code)); }
        if property != 0    { ds.push_str(&format!("        property: {}\n", property)); }
        if range != 0       { ds.push_str(&format!("        range: {}\n", range)); }

        if let Some(ref cl) = effect.count_limit {
            // Parse "(1,id)" or "(1, id)" or "(1)" etc.
            let cleaned = cl.trim();
            let parts: Vec<&str> = cleaned.split(',').map(|s| s.trim()).collect();
            let count: u32 = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(1);
            let code_val: u32 = parts.get(1).map(|s| {
                if *s == "id" { passcode as u32 } else { s.parse().unwrap_or(0) }
            }).unwrap_or(0);
            ds.push_str(&format!("        count_limit: ({}, {})\n", count, code_val));
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

/// Resolve a Lua constant expression like "EFFECT_TYPE_ACTIVATE+EFFECT_TYPE_IGNITION"
/// into a single u32. Supports + and | (bitwise OR) and common constants.
pub fn resolve_lua_constant_expr(expr: &str) -> u32 {
    let cleaned = expr.trim();
    if cleaned.is_empty() { return 0; }

    let mut total = 0u32;
    for part in cleaned.split(|c| c == '+' || c == '|') {
        let t = part.trim();
        if t.is_empty() { continue; }
        if let Ok(n) = t.parse::<u32>() {
            total |= n;
            continue;
        }
        // Handle hex
        if let Some(hex) = t.strip_prefix("0x") {
            if let Ok(n) = u32::from_str_radix(hex, 16) {
                total |= n;
                continue;
            }
        }
        total |= lookup_lua_constant(t);
    }
    total
}

pub fn lookup_lua_constant(name: &str) -> u32 {
    match name {
        // Effect types
        "EFFECT_TYPE_SINGLE" => 0x1, "EFFECT_TYPE_FIELD" => 0x2, "EFFECT_TYPE_EQUIP" => 0x4,
        "EFFECT_TYPE_ACTIONS" => 0x8, "EFFECT_TYPE_ACTIVATE" => 0x10, "EFFECT_TYPE_FLIP" => 0x20,
        "EFFECT_TYPE_IGNITION" => 0x40, "EFFECT_TYPE_TRIGGER_O" => 0x80, "EFFECT_TYPE_QUICK_O" => 0x100,
        "EFFECT_TYPE_TRIGGER_F" => 0x200, "EFFECT_TYPE_QUICK_F" => 0x400, "EFFECT_TYPE_CONTINUOUS" => 0x800,
        "EFFECT_TYPE_XMATERIAL" => 0x1000, "EFFECT_TYPE_GRANT" => 0x2000, "EFFECT_TYPE_TARGET" => 0x4000,

        // Categories
        "CATEGORY_DESTROY" => 0x1, "CATEGORY_RELEASE" => 0x2, "CATEGORY_REMOVE" => 0x4,
        "CATEGORY_TOHAND" => 0x8, "CATEGORY_TODECK" => 0x10, "CATEGORY_TOGRAVE" => 0x20,
        "CATEGORY_DECKDES" => 0x40, "CATEGORY_HANDES" => 0x80, "CATEGORY_SUMMON" => 0x100,
        "CATEGORY_SPECIAL_SUMMON" => 0x200, "CATEGORY_TOKEN" => 0x400, "CATEGORY_FLIP" => 0x800,
        "CATEGORY_POSITION" => 0x1000, "CATEGORY_CONTROL" => 0x2000, "CATEGORY_DISABLE" => 0x4000,
        "CATEGORY_DISABLE_SUMMON" => 0x8000, "CATEGORY_DRAW" => 0x10000, "CATEGORY_SEARCH" => 0x20000,
        "CATEGORY_EQUIP" => 0x40000, "CATEGORY_DAMAGE" => 0x80000, "CATEGORY_RECOVER" => 0x100000,
        "CATEGORY_ATKCHANGE" => 0x200000, "CATEGORY_DEFCHANGE" => 0x400000, "CATEGORY_COUNTER" => 0x800000,
        "CATEGORY_COIN" => 0x1000000, "CATEGORY_DICE" => 0x2000000, "CATEGORY_LEAVE_GRAVE" => 0x4000000,
        "CATEGORY_LVCHANGE" => 0x8000000, "CATEGORY_NEGATE" => 0x10000000, "CATEGORY_ANNOUNCE" => 0x20000000,
        "CATEGORY_FUSION_SUMMON" => 0x40000000,

        // Events
        "EVENT_STARTUP" => 1000, "EVENT_FLIP" => 1001, "EVENT_FREE_CHAIN" => 1002,
        "EVENT_DESTROY" => 1010, "EVENT_REMOVE" => 1011, "EVENT_TO_HAND" => 1012,
        "EVENT_TO_DECK" => 1013, "EVENT_TO_GRAVE" => 1014, "EVENT_LEAVE_FIELD" => 1015,
        "EVENT_CHANGE_POS" => 1016, "EVENT_RELEASE" => 1017, "EVENT_DISCARD" => 1018,
        "EVENT_CHAIN_SOLVING" => 1020, "EVENT_CHAIN_ACTIVATING" => 1021, "EVENT_CHAIN_SOLVED" => 1022,
        "EVENT_CHAIN_NEGATED" => 1024, "EVENT_CHAIN_DISABLED" => 1025, "EVENT_CHAIN_END" => 1026,
        "EVENT_CHAINING" => 1027, "EVENT_BECOME_TARGET" => 1028, "EVENT_DESTROYED" => 1029,
        "EVENT_MOVE" => 1030, "EVENT_LEAVE_GRAVE" => 1031, "EVENT_ADJUST" => 1040,
        "EVENT_BREAK_EFFECT" => 1050, "EVENT_SUMMON_SUCCESS" => 1100,
        "EVENT_FLIP_SUMMON_SUCCESS" => 1101, "EVENT_SPSUMMON_SUCCESS" => 1102,
        "EVENT_SUMMON" => 1103, "EVENT_FLIP_SUMMON" => 1104, "EVENT_SPSUMMON" => 1105,
        "EVENT_MSET" => 1106, "EVENT_SSET" => 1107, "EVENT_BE_MATERIAL" => 1108,
        "EVENT_BE_PRE_MATERIAL" => 1109, "EVENT_DRAW" => 1110, "EVENT_DAMAGE" => 1111,
        "EVENT_RECOVER" => 1112, "EVENT_PREDRAW" => 1113, "EVENT_SUMMON_NEGATED" => 1114,
        "EVENT_FLIP_SUMMON_NEGATED" => 1115, "EVENT_SPSUMMON_NEGATED" => 1116,
        "EVENT_CONTROL_CHANGED" => 1120, "EVENT_EQUIP" => 1121,
        "EVENT_ATTACK_ANNOUNCE" => 1130, "EVENT_BE_BATTLE_TARGET" => 1131,
        "EVENT_BATTLE_START" => 1132, "EVENT_BATTLE_CONFIRM" => 1133,
        "EVENT_PRE_DAMAGE_CALCULATE" => 1134, "EVENT_DAMAGE_STEP_END" => 1136,
        "EVENT_BATTLED" => 1137, "EVENT_BATTLE_DAMAGE" => 1138,
        "EVENT_BATTLE_DESTROYING" => 1139, "EVENT_BATTLE_DESTROYED" => 1140,
        "EVENT_ATTACK_DISABLED" => 1141, "EVENT_PHASE" => 0x1000, "EVENT_PHASE_START" => 0x2000,

        // Phases
        "PHASE_DRAW" => 0x1, "PHASE_STANDBY" => 0x2, "PHASE_MAIN1" => 0x4,
        "PHASE_BATTLE_START" => 0x8, "PHASE_BATTLE_STEP" => 0x10, "PHASE_DAMAGE" => 0x20,
        "PHASE_DAMAGE_CAL" => 0x40, "PHASE_BATTLE" => 0x80, "PHASE_MAIN2" => 0x100,
        "PHASE_END" => 0x200,

        // Locations
        "LOCATION_DECK" => 0x1, "LOCATION_HAND" => 0x2, "LOCATION_MZONE" => 0x4,
        "LOCATION_SZONE" => 0x8, "LOCATION_GRAVE" => 0x10, "LOCATION_REMOVED" => 0x20,
        "LOCATION_EXTRA" => 0x40, "LOCATION_FZONE" => 0x100, "LOCATION_PZONE" => 0x200,
        "LOCATION_ONFIELD" => 0xc, "LOCATION_OVERLAY" => 0x80,

        // Effect flags
        "EFFECT_FLAG_INITIAL" => 0x1, "EFFECT_FLAG_FUNC_VALUE" => 0x2,
        "EFFECT_FLAG_COUNT_LIMIT" => 0x4, "EFFECT_FLAG_FIELD_ONLY" => 0x8,
        "EFFECT_FLAG_CARD_TARGET" => 0x10, "EFFECT_FLAG_IGNORE_RANGE" => 0x20,
        "EFFECT_FLAG_ABSOLUTE_TARGET" => 0x40, "EFFECT_FLAG_IGNORE_IMMUNE" => 0x80,
        "EFFECT_FLAG_SET_AVAILABLE" => 0x100, "EFFECT_FLAG_CANNOT_NEGATE" => 0x200,
        "EFFECT_FLAG_CANNOT_DISABLE" => 0x400, "EFFECT_FLAG_PLAYER_TARGET" => 0x800,
        "EFFECT_FLAG_BOTH_SIDE" => 0x1000, "EFFECT_FLAG_COPY_INHERIT" => 0x2000,
        "EFFECT_FLAG_DAMAGE_STEP" => 0x4000, "EFFECT_FLAG_DAMAGE_CAL" => 0x8000,
        "EFFECT_FLAG_DELAY" => 0x10000, "EFFECT_FLAG_SINGLE_RANGE" => 0x20000,
        "EFFECT_FLAG_UNCOPYABLE" => 0x40000, "EFFECT_FLAG_OATH" => 0x80000,
        "EFFECT_FLAG_SPSUM_PARAM" => 0x100000, "EFFECT_FLAG_REPEAT" => 0x200000,
        "EFFECT_FLAG_NO_TURN_RESET" => 0x400000, "EFFECT_FLAG_EVENT_PLAYER" => 0x800000,
        "EFFECT_FLAG_OWNER_RELATE" => 0x1000000, "EFFECT_FLAG_CANNOT_INACTIVATE" => 0x2000000,
        "EFFECT_FLAG_CLIENT_HINT" => 0x4000000, "EFFECT_FLAG_CONTINUOUS_TARGET" => 0x8000000,
        "EFFECT_FLAG_LIMIT_ZONE" => 0x10000000, "EFFECT_FLAG_IMMEDIATELY_APPLY" => 0x80000000,

        // Effect codes (common ones)
        "EFFECT_DISABLE" => 2, "EFFECT_UPDATE_ATTACK" => 100, "EFFECT_UPDATE_DEFENSE" => 104,
        "EFFECT_SPSUMMON_CONDITION" => 30, "EFFECT_REVIVE_LIMIT" => 31, "EFFECT_SPSUMMON_PROC" => 34,
        "EFFECT_SPSUMMON_PROC_G" => 320, "EFFECT_CANNOT_SUMMON" => 50,
        "EFFECT_CANNOT_FLIP_SUMMON" => 51, "EFFECT_CANNOT_SPECIAL_SUMMON" => 52,
        "EFFECT_CANNOT_MSET" => 53, "EFFECT_CANNOT_SSET" => 54,
        "EFFECT_CANNOT_CHANGE_POSITION" => 56, "EFFECT_CANNOT_BE_EFFECT_TARGET" => 60,
        "EFFECT_CANNOT_ATTACK" => 62, "EFFECT_CANNOT_ATTACK_ANNOUNCE" => 63,
        "EFFECT_INDESTRUCTABLE" => 65, "EFFECT_INDESTRUCTABLE_BATTLE" => 66,
        "EFFECT_INDESTRUCTABLE_EFFECT" => 67, "EFFECT_CANNOT_BE_BATTLE_TARGET" => 68,
        "EFFECT_CANNOT_ACTIVATE" => 75, "EFFECT_DISABLE_EFFECT" => 76,
        "EFFECT_CANNOT_TRIGGER" => 78, "EFFECT_PIERCE" => 80,
        "EFFECT_DIRECT_ATTACK" => 82, "EFFECT_EXTRA_ATTACK" => 84,
        "EFFECT_SET_ATTACK" => 91, "EFFECT_SET_ATTACK_FINAL" => 92,
        "EFFECT_SET_BASE_ATTACK" => 93, "EFFECT_SWAP_ATTACK_FINAL" => 97,
        "EFFECT_UPDATE_LEVEL" => 110, "EFFECT_CHANGE_LEVEL" => 113,
        "EFFECT_CHANGE_ATTRIBUTE" => 121, "EFFECT_CHANGE_CODE" => 129,
        "EFFECT_DESTROY_REPLACE" => 202, "EFFECT_SEND_REPLACE" => 203,
        "EFFECT_LEAVE_FIELD_REDIRECT" => 205, "EFFECT_TO_GRAVE_REDIRECT" => 206,
        "EFFECT_IMMUNE_EFFECT" => 308, "EFFECT_EQUIP_LIMIT" => 311,
        "EFFECT_MATERIAL_CHECK" => 312, "EFFECT_CANNOT_DISABLE_SPSUMMON" => 77,
        "EFFECT_CANNOT_BE_FUSION_MATERIAL" => 310, "EFFECT_ADD_TYPE" => 118,
        "EFFECT_REMOVE_TYPE" => 119, "EFFECT_ADD_RACE" => 120,
        "EFFECT_REMOVE_RACE" => 122, "EFFECT_ADD_ATTRIBUTE" => 123,
        "EFFECT_REMOVE_ATTRIBUTE" => 124,

        _ => 0,
    }
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
    if cost_key.contains("SelfToDeck")   { return Some("send self to deck".to_string()); }
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
