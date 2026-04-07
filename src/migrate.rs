// ============================================================
// DuelScript Migration Tool — migrate.rs
//
// Smart migrator: reads ProjectIgnis Lua card scripts and
// generates .ds files by extracting SetType/SetCategory/SetCode
// patterns. Uses BabelCdb for card stats when available.
//
// Usage:
//   let result = generate_from_lua(lua_source, 55144522, "Pot of Greed");
//   let batch = migrate_directory(lua_dir, cdb_reader);
// ============================================================

use std::{
    fs,
    path::Path,
};

// ── Public API ────────────────────────────────────────────────

#[derive(Debug)]
pub struct MigrationResult {
    pub passcode:    u64,
    pub card_name:   String,
    pub ds_content:  String,
    pub confidence:  Confidence,
    pub effect_count: usize,
    pub notes:       Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Full,   // All effects fully mapped
    High,   // Most effects mapped, minor TODOs
    Medium, // Structure correct, some effects need review
    Low,    // Skeleton only
}

impl Confidence {
    pub fn label(&self) -> &'static str {
        match self {
            Confidence::Full   => "FULL",
            Confidence::High   => "HIGH",
            Confidence::Medium => "MEDIUM",
            Confidence::Low    => "LOW",
        }
    }
}

/// Generate a .ds file from a Lua script, optionally with CDB stats.
pub fn generate_from_lua(
    lua_source: &str,
    passcode: u64,
    card_name: &str,
) -> MigrationResult {
    generate_from_lua_with_cdb(lua_source, passcode, card_name, None)
}

/// Generate a .ds file from a Lua script with CDB data for card stats.
pub fn generate_from_lua_with_cdb(
    lua_source: &str,
    passcode: u64,
    card_name: &str,
    cdb_card: Option<&crate::cdb::CdbCard>,
) -> MigrationResult {
    let effects = extract_effects(lua_source);
    let meta = extract_card_meta(lua_source);
    let mut ds = String::new();
    let notes = Vec::new();
    let mut all_mapped = true;

    // Header
    ds.push_str(&format!("// {}\n", card_name));
    ds.push_str(&format!("// Migrated from c{}.lua\n\n", passcode));
    ds.push_str(&format!("card \"{}\" {{\n", card_name));
    ds.push_str(&format!("    password: {}\n", passcode));

    // Card stats from CDB or placeholder
    if let Some(cdb) = cdb_card {
        ds.push_str(&format!("    type: {}\n", cdb.ds_type_line()));
        if cdb.is_monster() {
            ds.push_str(&format!("    attribute: {}\n", cdb.attribute_name()));
            ds.push_str(&format!("    race: {}\n", cdb.race_name()));
            if cdb.is_xyz() {
                ds.push_str(&format!("    rank: {}\n", cdb.actual_level()));
            } else if cdb.is_link() {
                ds.push_str(&format!("    link: {}\n", cdb.actual_level()));
            } else {
                ds.push_str(&format!("    level: {}\n", cdb.actual_level()));
            }
            if cdb.is_pendulum() {
                ds.push_str(&format!("    scale: {}\n", cdb.pendulum_scale()));
            }
            ds.push_str(&format!("    atk: {}\n", cdb.atk_str()));
            if !cdb.is_link() {
                ds.push_str(&format!("    def: {}\n", cdb.def_str()));
            }
        }
    } else {
        ds.push_str("    // TODO: type, attribute, race, level, atk, def from CDB\n");
    }
    ds.push('\n');

    // Summon procedures
    if meta.has_xyz { ds.push_str("    materials {\n        require: 2+ monster\n        same_level: true\n        method: xyz\n    }\n\n"); }
    if meta.has_synchro { ds.push_str("    materials {\n        require: 1 tuner monster\n        require: 1+ non-tuner monster\n        method: synchro\n    }\n\n"); }
    if meta.has_link { ds.push_str("    materials {\n        require: 2+ effect monster\n        method: link\n    }\n\n"); }
    if meta.has_fusion { ds.push_str("    materials {\n        require: 2+ monster\n        method: fusion\n    }\n\n"); }
    if meta.has_revive_limit {
        ds.push_str("    summon_condition {\n        cannot_normal_summon: true\n    }\n\n");
    }

    // Effects
    for (i, eff) in effects.iter().enumerate() {
        // Skip summon proc effects (they're handled by materials block)
        if eff.code_raw == "EFFECT_SPSUMMON_PROC" || eff.code_raw.contains("946") || eff.code_raw.contains("948") || eff.code_raw.contains("950") {
            continue;
        }

        ds.push_str(&format!("    effect \"Effect {}\" {{\n", i + 1));

        // Speed
        let speed = determine_speed(&eff.type_raw);
        ds.push_str(&format!("        speed: {}\n", speed));

        // Frequency
        if let Some(ref cl) = eff.count_limit {
            if cl.contains(",id") || cl.contains(", id") {
                ds.push_str("        once_per_turn: hard\n");
            } else if cl.contains(",0") || cl.contains(", 0") {
                ds.push_str("        once_per_turn: soft\n");
            } else {
                ds.push_str("        once_per_turn: hard\n");
            }
        }

        // Optional
        if eff.type_raw.contains("TRIGGER_O") || eff.type_raw.contains("QUICK_O") {
            ds.push_str("        optional: true\n");
        }

        // Trigger
        if let Some(trigger) = determine_trigger(&eff.code_raw) {
            ds.push_str(&format!("        trigger: {}\n", trigger));
        }

        // Range condition
        if eff.range_raw.contains("LOCATION_HAND") && eff.type_raw.contains("QUICK") {
            // Hand trap
        } else if eff.range_raw.contains("LOCATION_GRAVE") {
            ds.push_str("        condition: in_gy\n");
        }

        // Cost
        if let Some(cost) = determine_cost(&eff.cost_key, lua_source) {
            ds.push_str(&format!("        cost {{\n            {}\n        }}\n", cost));
        }

        // On resolve
        let actions = determine_actions(&eff.operation_key, lua_source, &eff.category_raw);
        ds.push_str("        on_resolve {\n");
        if !actions.is_empty() {
            for action in &actions {
                ds.push_str(&format!("            {}\n", action));
            }
        } else {
            // Must have at least one action for the grammar to parse
            ds.push_str("            reveal self\n");
            all_mapped = false;
        }
        ds.push_str("        }\n");

        ds.push_str("    }\n\n");
    }

    ds.push_str("}\n");

    let effect_count = effects.iter()
        .filter(|e| e.code_raw != "EFFECT_SPSUMMON_PROC" && !e.code_raw.contains("946") && !e.code_raw.contains("948") && !e.code_raw.contains("950"))
        .count();

    let confidence = if effect_count == 0 && (meta.has_xyz || meta.has_synchro || meta.has_link || meta.has_fusion) {
        Confidence::Medium // Summon procedure only
    } else if all_mapped && effect_count > 0 {
        Confidence::High
    } else if effect_count > 0 {
        Confidence::Medium
    } else {
        Confidence::Low
    };

    MigrationResult {
        passcode,
        card_name: card_name.to_string(),
        ds_content: ds,
        confidence,
        effect_count,
        notes,
    }
}

/// Batch migrate all .lua files in a directory.
pub fn migrate_directory(lua_dir: &Path) -> Vec<MigrationResult> {
    let mut results = Vec::new();

    let Ok(entries) = fs::read_dir(lua_dir) else { return results };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if !name.starts_with('c') || !name.ends_with(".lua") { continue; }

        let id_str = name.trim_start_matches('c').trim_end_matches(".lua");
        let Ok(passcode) = id_str.parse::<u64>() else { continue };

        let Ok(source) = fs::read_to_string(&path) else { continue };

        // Extract card name from first comment line
        let card_name = source.lines()
            .find(|l| l.starts_with("--") && !l.starts_with("---"))
            .and_then(|l| l.strip_prefix("--"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("Card {}", passcode));

        // Skip the Japanese name line if there's an English name on the next line
        let card_name = source.lines()
            .filter(|l| l.starts_with("--") && !l.starts_with("---"))
            .nth(1)
            .and_then(|l| l.strip_prefix("--"))
            .map(|s| s.trim().to_string())
            .unwrap_or(card_name);

        results.push(generate_from_lua(&source, passcode, &card_name));
    }

    results
}

// ── Effect Extraction ─────────────────────────────────────────

#[derive(Debug, Default)]
struct ExtractedEffect {
    type_raw:      String,
    category_raw:  String,
    code_raw:      String,
    property_raw:  String,
    range_raw:     String,
    count_limit:   Option<String>,
    cost_key:      Option<String>,
    target_key:    Option<String>,
    condition_key: Option<String>,
    operation_key: Option<String>,
}

#[derive(Debug, Default)]
struct CardMeta {
    has_xyz: bool,
    has_synchro: bool,
    has_link: bool,
    has_fusion: bool,
    has_ritual: bool,
    has_revive_limit: bool,
}

fn extract_card_meta(source: &str) -> CardMeta {
    let mut meta = CardMeta::default();
    for line in source.lines() {
        let l = line.trim();
        if l.contains("Xyz.AddProcedure")     { meta.has_xyz = true; }
        if l.contains("Synchro.AddProcedure") { meta.has_synchro = true; }
        if l.contains("Link.AddProcedure")    { meta.has_link = true; }
        if l.contains("Fusion.AddProc")       { meta.has_fusion = true; }
        if l.contains("Ritual.AddProcedure")  { meta.has_ritual = true; }
        if l.contains("EnableReviveLimit")    { meta.has_revive_limit = true; }
    }
    meta
}

fn extract_effects(source: &str) -> Vec<ExtractedEffect> {
    let mut effects = Vec::new();
    let mut current: Option<ExtractedEffect> = None;

    for line in source.lines() {
        let l = line.trim();

        // New effect starts with Effect.CreateEffect
        if l.contains("Effect.CreateEffect") {
            if let Some(eff) = current.take() {
                effects.push(eff);
            }
            current = Some(ExtractedEffect::default());
        }

        if let Some(ref mut eff) = current {
            if l.contains(":SetType(") {
                eff.type_raw = extract_parenthesized(l);
            }
            if l.contains(":SetCategory(") {
                eff.category_raw = extract_parenthesized(l);
            }
            if l.contains(":SetCode(") {
                eff.code_raw = extract_parenthesized(l);
            }
            if l.contains(":SetProperty(") {
                eff.property_raw = extract_parenthesized(l);
            }
            if l.contains(":SetRange(") {
                eff.range_raw = extract_parenthesized(l);
            }
            if l.contains(":SetCountLimit(") {
                eff.count_limit = Some(extract_parenthesized(l));
            }
            if l.contains(":SetCost(") {
                eff.cost_key = Some(extract_parenthesized(l));
            }
            if l.contains(":SetTarget(") {
                eff.target_key = Some(extract_parenthesized(l));
            }
            if l.contains(":SetCondition(") {
                eff.condition_key = Some(extract_parenthesized(l));
            }
            if l.contains(":SetOperation(") {
                eff.operation_key = Some(extract_parenthesized(l));
            }
        }

        // RegisterEffect ends the current effect
        if l.contains("RegisterEffect") {
            if let Some(eff) = current.take() {
                effects.push(eff);
            }
        }
    }

    if let Some(eff) = current {
        effects.push(eff);
    }

    effects
}

fn extract_parenthesized(line: &str) -> String {
    if let Some(start) = line.find('(') {
        if let Some(end) = line[start..].find(')') {
            return line[start+1..start+end].trim().to_string();
        }
    }
    String::new()
}

// ── Mapping Helpers ───────────────────────────────────────────

fn determine_speed(type_raw: &str) -> &'static str {
    if type_raw.contains("QUICK") { "spell_speed_2" }
    else { "spell_speed_1" }
}

fn determine_trigger(code_raw: &str) -> Option<&'static str> {
    if code_raw.contains("EVENT_CHAINING")         { return Some("opponent_activates [search | special_summon | send_to_gy | draw]"); }
    if code_raw.contains("EVENT_SUMMON")            { return Some("when_summoned"); }
    if code_raw.contains("EVENT_SPSUMMON_SUCCESS")  { return Some("when_summoned by_special_summon"); }
    if code_raw.contains("EVENT_DESTROYED")         { return Some("when_destroyed"); }
    if code_raw.contains("EVENT_TO_GRAVE")          { return Some("when_sent_to gy"); }
    if code_raw.contains("EVENT_ATTACK_ANNOUNCE")   { return Some("when attack_declared"); }
    if code_raw.contains("EVENT_BE_BATTLE_TARGET")  { return Some("when_attacked"); }
    if code_raw.contains("EVENT_FLIP")              { return Some("when_flipped"); }
    if code_raw.contains("PHASE_END")               { return Some("during_end_phase"); }
    if code_raw.contains("PHASE_STANDBY")           { return Some("during_standby_phase"); }
    if code_raw.contains("EVENT_FREE_CHAIN")        { return None; } // No trigger
    None
}

fn determine_cost(cost_key: &Option<String>, source: &str) -> Option<String> {
    let key = cost_key.as_ref()?;
    if key.contains("SelfDiscard") || key.contains("Cost.Discard") {
        return Some("discard self".to_string());
    }
    if key.contains("DetachFromSelf") {
        // Try to extract count
        let count = if key.contains("(2") { "2" } else { "1" };
        return Some(format!("detach {} overlay_unit from self", count));
    }
    if key.contains("SelfToGrave") {
        return Some("send self to gy".to_string());
    }
    if key.contains("SelfBanish") {
        return Some("banish self".to_string());
    }
    if key.contains("PayLp") || key.contains("PayLP") {
        // Try to extract amount from Cost.PayLp(N)
        if let Some(start) = key.find("PayLp(").or(key.find("PayLP(")) {
            let rest = &key[start + 6..];
            if let Some(end) = rest.find(')') {
                let amount = &rest[..end];
                return Some(format!("pay_lp {}", amount));
            }
        }
        return Some("pay_lp 1000 // TODO: verify amount".to_string());
    }
    // Check for inline cost functions
    if key.starts_with("s.") {
        // Scan the source for the cost function to detect patterns
        let func_name = key.trim_start_matches("s.");
        if let Some(body) = find_function_body(source, func_name) {
            if body.contains("PayLPCost") && body.contains("GetLP") {
                return Some("pay_lp your_lp / 2".to_string());
            }
            if body.contains("Discard") {
                return Some("discard self // TODO: verify target".to_string());
            }
            if body.contains("RemoveOverlayCard") {
                return Some("detach 1 overlay_unit from self".to_string());
            }
        }
    }
    None
}

fn determine_actions(operation_key: &Option<String>, source: &str, category_raw: &str) -> Vec<String> {
    let mut actions = Vec::new();

    // First check category flags for hints
    if category_raw.contains("CATEGORY_DRAW") {
        actions.push("draw 2 // TODO: verify count".to_string());
    }
    if category_raw.contains("CATEGORY_DESTROY") && !category_raw.contains("DISABLE") {
        actions.push("destroy (1, card, opponent controls) // TODO: verify target".to_string());
    }
    if category_raw.contains("CATEGORY_SPECIAL_SUMMON") {
        actions.push("special_summon (1, monster) from gy // TODO: verify".to_string());
    }
    if category_raw.contains("CATEGORY_NEGATE") {
        if category_raw.contains("CATEGORY_DESTROY") {
            actions.push("negate activation and destroy".to_string());
        } else {
            actions.push("negate activation".to_string());
        }
    }
    if category_raw.contains("CATEGORY_DISABLE") && !category_raw.contains("SUMMON") {
        actions.push("negate effect".to_string());
    }
    if category_raw.contains("CATEGORY_DISABLE_SUMMON") {
        if category_raw.contains("CATEGORY_DESTROY") {
            actions.push("negate summon and destroy".to_string());
        } else {
            actions.push("negate summon".to_string());
        }
    }

    // If we got actions from categories, we're done
    if !actions.is_empty() {
        return actions;
    }

    // Try scanning the operation function
    if let Some(key) = operation_key {
        if key.starts_with("s.") {
            let func_name = key.trim_start_matches("s.");
            if let Some(body) = find_function_body(source, func_name) {
                if body.contains("Duel.Draw") { actions.push("draw 2 // TODO: verify count".to_string()); }
                if body.contains("Duel.Destroy") { actions.push("destroy (1+, card, either_player controls) // TODO: verify".to_string()); }
                if body.contains("Duel.SpecialSummon") { actions.push("special_summon self from gy // TODO: verify".to_string()); }
                if body.contains("Duel.NegateAttack") { actions.push("negate attack".to_string()); }
                if body.contains("Duel.NegateEffect") { actions.push("negate effect".to_string()); }
                if body.contains("Duel.NegateActivation") { actions.push("negate activation".to_string()); }
                if body.contains("Duel.Remove") { actions.push("banish (1, card) // TODO: verify".to_string()); }
                if body.contains("Duel.SendtoHand") { actions.push("add_to_hand (1, card) from gy // TODO: verify".to_string()); }
                if body.contains("Duel.Damage") { actions.push("deal_damage to opponent: 1000 // TODO: verify".to_string()); }
                if body.contains("Duel.Recover") { actions.push("gain_lp: 1000 // TODO: verify".to_string()); }
            }
        } else if key.contains("NegateAttack") {
            actions.push("negate attack".to_string());
        }
    }

    actions
}

fn find_function_body(source: &str, func_name: &str) -> Option<String> {
    let pattern = format!("function s.{}(", func_name);
    let lines: Vec<&str> = source.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        if line.contains(&pattern) {
            // Collect lines until next "end" at the same indentation
            let mut body = String::new();
            let mut depth = 1;
            for j in (i+1)..lines.len() {
                let l = lines[j].trim();
                if l.starts_with("function ") { break; } // Next function
                if l == "end" {
                    depth -= 1;
                    if depth == 0 { break; }
                }
                if l.contains("if ") || l.contains("for ") || l.contains("while ") {
                    depth += 1;
                }
                body.push_str(l);
                body.push('\n');
            }
            return Some(body);
        }
    }
    None
}
