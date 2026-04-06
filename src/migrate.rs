// ============================================================
// DuelScript Migration Tool — migrate.rs
//
// Reads ProjectIgnis Lua card scripts and generates .ds
// skeletons. Automates what it can, flags what needs manual
// work. Designed to port the 12,000+ CardScripts over time.
//
// Usage (CLI):
//   duelscript migrate official/c14558127.lua
//   duelscript migrate official/                  (batch)
//   duelscript migrate-cdb cards.cdb cards/official/
// ============================================================

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

// ── Public API ────────────────────────────────────────────────

/// Result of migrating a single Lua script
#[derive(Debug)]
pub struct MigrationResult {
    pub passcode:    u64,
    pub source_path: PathBuf,
    pub output_path: PathBuf,
    pub ds_content:  String,
    pub confidence:  Confidence,
    pub notes:       Vec<MigrationNote>,
}

/// How confident we are in the migration output
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confidence {
    /// Fully auto-migrated — normal monster, vanilla spell/trap
    Full,
    /// Mostly auto-migrated — common effect pattern recognized  
    High,
    /// Skeleton generated — structure correct, effects need review
    Medium,
    /// Passcode extracted only — complex script, manual work needed
    Low,
}

impl Confidence {
    pub fn label(&self) -> &'static str {
        match self {
            Confidence::Full   => "FULL   ",
            Confidence::High   => "HIGH   ",
            Confidence::Medium => "MEDIUM ",
            Confidence::Low    => "LOW    ",
        }
    }
}

/// A note attached to a migration result
#[derive(Debug, Clone)]
pub struct MigrationNote {
    pub kind:    NoteKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteKind {
    AutoMigrated,  // Successfully mapped to DuelScript keyword
    NeedsReview,   // Pattern recognized but needs human verification
    Unknown,       // Could not map — manual scripting required
    Warning,       // Possible issue detected
}

// ── Lua Pattern Detector ──────────────────────────────────────

/// Patterns we recognize from ProjectIgnis Lua scripts.
/// Each maps to a DuelScript construct.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum LuaPattern {
    // Effect types
    IgnitionEffect,
    TriggerEffect,
    QuickEffect,
    ContinuousEffect,
    FieldEffect,

    // Common operations
    DrawCards,
    SpecialSummon,
    SearchDeck,
    NegateEffect,
    NegateAndDestroy,
    Destroy,
    Banish,
    SendToGy,
    ReturnToHand,
    ReturnToDeck,

    // Costs
    DetachCost,
    DiscardCost,
    TributeCost,
    PayLpCost,
    BanishCost,

    // Conditions
    HandCondition,
    FieldCondition,
    GraveyardCondition,

    // Frequency
    OncePerTurn,
    OncePerDuel,

    // Summon procedures
    XyzSummon,
    SynchroSummon,
    FusionSummon,
    LinkSummon,
    RitualSummon,

    // Special
    OpponentActivates,
    CounterLimit,
    ReviveLimit,
    EnableReviveLimit,
}

// ── Lua Script Analyzer ───────────────────────────────────────

struct LuaAnalyzer<'a> {
    source:   &'a str,
    patterns: HashMap<LuaPattern, Vec<usize>>, // pattern → line numbers
    passcode: u64,
}

impl<'a> LuaAnalyzer<'a> {
    fn new(source: &'a str, passcode: u64) -> Self {
        let mut analyzer = Self {
            source,
            patterns: HashMap::new(),
            passcode,
        };
        analyzer.scan();
        analyzer
    }

    fn scan(&mut self) {
        for (lineno, line) in self.source.lines().enumerate() {
            self.detect_line(line, lineno + 1);
        }
    }

    fn detect_line(&mut self, line: &str, lineno: usize) {
        let l = line.trim();

        // Effect types
        if l.contains("EFFECT_TYPE_IGNITION")  { self.mark(LuaPattern::IgnitionEffect, lineno); }
        if l.contains("EFFECT_TYPE_TRIGGER_O") || l.contains("EFFECT_TYPE_TRIGGER_F") {
            self.mark(LuaPattern::TriggerEffect, lineno);
        }
        if l.contains("EFFECT_TYPE_QUICK_O")   { self.mark(LuaPattern::QuickEffect, lineno); }
        if l.contains("EFFECT_TYPE_CONTINUOUS") { self.mark(LuaPattern::ContinuousEffect, lineno); }
        if l.contains("EFFECT_TYPE_FIELD")      { self.mark(LuaPattern::FieldEffect, lineno); }

        // Operations
        if l.contains("Duel.Draw")              { self.mark(LuaPattern::DrawCards, lineno); }
        if l.contains("Duel.SpecialSummon")     { self.mark(LuaPattern::SpecialSummon, lineno); }
        if l.contains("LOCATION_DECK") && l.contains("Duel.Hint") {
            self.mark(LuaPattern::SearchDeck, lineno);
        }
        if l.contains("Duel.NegateEffect") || l.contains("re:IsHasType(EFFECT_NEGATE)") {
            self.mark(LuaPattern::NegateEffect, lineno);
        }
        if l.contains("Duel.Destroy") && l.contains("negate") {
            self.mark(LuaPattern::NegateAndDestroy, lineno);
        }
        if l.contains("Duel.Destroy")           { self.mark(LuaPattern::Destroy, lineno); }
        if l.contains("Duel.Remove")            { self.mark(LuaPattern::Banish, lineno); }
        if l.contains("Duel.SendtoGrave") || l.contains("Duel.SendToGrave") {
            self.mark(LuaPattern::SendToGy, lineno);
        }
        if l.contains("LOCATION_HAND") && l.contains("Duel.SendtoGrave") {
            self.mark(LuaPattern::ReturnToHand, lineno);
        }
        if l.contains("LOCATION_DECK") && l.contains("Duel.SendtoGrave") {
            self.mark(LuaPattern::ReturnToDeck, lineno);
        }

        // Costs
        if l.contains("Cost.Detach") || l.contains("DetachFromSelf") {
            self.mark(LuaPattern::DetachCost, lineno);
        }
        if l.contains("Cost.Discard") || l.contains("Duel.Discard") {
            self.mark(LuaPattern::DiscardCost, lineno);
        }
        if l.contains("Cost.Tribute") || l.contains("Duel.Tribute") {
            self.mark(LuaPattern::TributeCost, lineno);
        }
        if l.contains("Cost.PayLp") || l.contains("Duel.DamageStep") {
            self.mark(LuaPattern::PayLpCost, lineno);
        }
        if l.contains("Cost.Banish")            { self.mark(LuaPattern::BanishCost, lineno); }

        // Conditions
        if l.contains("LOCATION_HAND")          { self.mark(LuaPattern::HandCondition, lineno); }
        if l.contains("LOCATION_MZONE") || l.contains("LOCATION_SZONE") {
            self.mark(LuaPattern::FieldCondition, lineno);
        }
        if l.contains("LOCATION_GRAVE")         { self.mark(LuaPattern::GraveyardCondition, lineno); }

        // Frequency
        if l.contains("SetCountLimit(1)") || l.contains("SetCountLimit(1,") {
            self.mark(LuaPattern::OncePerTurn, lineno);
        }
        if l.contains("SetCountLimit(1,id)") || l.contains("RESET_DUEL") {
            self.mark(LuaPattern::OncePerDuel, lineno);
        }

        // Summon procedures
        if l.contains("Xyz.AddProcedure")       { self.mark(LuaPattern::XyzSummon, lineno); }
        if l.contains("Synchro.AddProcedure")   { self.mark(LuaPattern::SynchroSummon, lineno); }
        if l.contains("Fusion.AddProcedure")    { self.mark(LuaPattern::FusionSummon, lineno); }
        if l.contains("Link.AddProcedure")      { self.mark(LuaPattern::LinkSummon, lineno); }
        if l.contains("Ritual.AddProcedure")    { self.mark(LuaPattern::RitualSummon, lineno); }

        // Special
        if l.contains("opponent_activates") || l.contains("EVENT_CHAIN_SOLVING") {
            self.mark(LuaPattern::OpponentActivates, lineno);
        }
        if l.contains("EnableReviveLimit")      { self.mark(LuaPattern::EnableReviveLimit, lineno); }
    }

    fn mark(&mut self, pattern: LuaPattern, lineno: usize) {
        self.patterns.entry(pattern).or_default().push(lineno);
    }

    fn has(&self, p: &LuaPattern) -> bool {
        self.patterns.contains_key(p)
    }

    fn count_effects(&self) -> usize {
        // Count RegisterEffect calls as a proxy for effect count
        self.source.lines()
            .filter(|l| l.contains("RegisterEffect"))
            .count()
    }

    fn extract_effect_count_limit(&self) -> u32 {
        if self.has(&LuaPattern::OncePerDuel) { return 1; }
        if self.has(&LuaPattern::OncePerTurn) { return 1; }
        0
    }
}

// ── DS Generator ──────────────────────────────────────────────

struct DsGenerator {
    analyzer: LuaAnalyzer<'static>, // lifetime simplified for generation
    passcode: u64,
    card_name: String,
}

/// Generate a .ds skeleton from a Lua script source and card name.
pub fn generate_from_lua(
    lua_source: &str,
    passcode:   u64,
    card_name:  &str,
) -> MigrationResult {
    let mut notes = Vec::new();
    let mut ds    = String::new();
    let mut confidence = Confidence::Low;

    // Static analysis of the Lua source
    let mut patterns: HashMap<LuaPattern, bool> = HashMap::new();
    let mut effect_count = 0usize;

    for line in lua_source.lines() {
        let l = line.trim();

        macro_rules! detect {
            ($pat:expr, $needle:expr) => {
                if l.contains($needle) { patterns.insert($pat, true); }
            };
        }

        detect!(LuaPattern::XyzSummon,       "Xyz.AddProcedure");
        detect!(LuaPattern::SynchroSummon,   "Synchro.AddProcedure");
        detect!(LuaPattern::FusionSummon,    "Fusion.AddProcedure");
        detect!(LuaPattern::LinkSummon,      "Link.AddProcedure");
        detect!(LuaPattern::RitualSummon,    "Ritual.AddProcedure");
        detect!(LuaPattern::EnableReviveLimit, "EnableReviveLimit");
        detect!(LuaPattern::DetachCost,      "DetachFromSelf");
        detect!(LuaPattern::DiscardCost,     "Cost.Discard");
        detect!(LuaPattern::TributeCost,     "Cost.Tribute");
        detect!(LuaPattern::PayLpCost,       "Cost.PayLp");
        detect!(LuaPattern::BanishCost,      "Cost.Banish");
        detect!(LuaPattern::DrawCards,       "Duel.Draw");
        detect!(LuaPattern::Destroy,         "Duel.Destroy");
        detect!(LuaPattern::Banish,          "Duel.Remove");
        detect!(LuaPattern::SendToGy,        "Duel.SendtoGrave");
        detect!(LuaPattern::SpecialSummon,   "Duel.SpecialSummon");
        detect!(LuaPattern::NegateEffect,    "Duel.NegateEffect");
        detect!(LuaPattern::OncePerTurn,     "SetCountLimit(1)");
        detect!(LuaPattern::OncePerDuel,     "RESET_DUEL");
        detect!(LuaPattern::HandCondition,   "LOCATION_HAND");
        detect!(LuaPattern::FieldCondition,  "LOCATION_MZONE");
        detect!(LuaPattern::GraveyardCondition, "LOCATION_GRAVE");
        detect!(LuaPattern::IgnitionEffect,  "EFFECT_TYPE_IGNITION");
        detect!(LuaPattern::TriggerEffect,   "EFFECT_TYPE_TRIGGER");
        detect!(LuaPattern::QuickEffect,     "EFFECT_TYPE_QUICK");
        detect!(LuaPattern::ContinuousEffect,"EFFECT_TYPE_CONTINUOUS");

        if l.contains("RegisterEffect") { effect_count += 1; }
    }

    // ── Header ─────────────────────────────────────────────────
    ds.push_str(&format!("// Migrated from c{}.lua\n", passcode));
    ds.push_str("// Review all effect blocks before use — see LANGUAGE_REFERENCE.md\n\n");
    ds.push_str(&format!("card \"{}\" {{\n", card_name));
    ds.push_str(&format!("  password: {}\n\n", passcode));
    ds.push_str("  // TODO: add type, attribute, race, level, atk, def from BabelCdb\n");
    ds.push_str("  // Run: duelscript merge-cdb cards.cdb cards/ to auto-populate\n\n");

    notes.push(MigrationNote {
        kind:    NoteKind::NeedsReview,
        message: "Card stats not populated — run merge-cdb or fill from BabelCdb".to_string(),
    });

    // ── Summon procedure ───────────────────────────────────────
    if patterns.contains_key(&LuaPattern::XyzSummon) {
        ds.push_str("  materials {\n");
        ds.push_str("    // TODO: add Xyz material requirements\n");
        ds.push_str("    require: 2+ monster\n");
        ds.push_str("    same_level: true\n");
        ds.push_str("  }\n\n");
        notes.push(MigrationNote {
            kind:    NoteKind::AutoMigrated,
            message: "Xyz summon procedure detected — materials block generated".to_string(),
        });
        confidence = Confidence::Medium;
    }
    if patterns.contains_key(&LuaPattern::SynchroSummon) {
        ds.push_str("  materials {\n");
        ds.push_str("    // TODO: add Synchro material requirements\n");
        ds.push_str("    require: 1 tuner monster\n");
        ds.push_str("    require: 1+ non-tuner monster\n");
        ds.push_str("  }\n\n");
        notes.push(MigrationNote {
            kind:    NoteKind::AutoMigrated,
            message: "Synchro summon procedure detected — materials block generated".to_string(),
        });
        confidence = Confidence::Medium;
    }
    if patterns.contains_key(&LuaPattern::FusionSummon) {
        ds.push_str("  materials {\n");
        ds.push_str("    // TODO: add Fusion material requirements\n");
        ds.push_str("    require: 2+ monster\n");
        ds.push_str("    method: fusion\n");
        ds.push_str("  }\n\n");
        notes.push(MigrationNote {
            kind:    NoteKind::AutoMigrated,
            message: "Fusion summon procedure detected — materials block generated".to_string(),
        });
        confidence = Confidence::Medium;
    }
    if patterns.contains_key(&LuaPattern::LinkSummon) {
        ds.push_str("  // TODO: add link_arrows declaration\n");
        ds.push_str("  materials {\n");
        ds.push_str("    // TODO: add Link material requirements\n");
        ds.push_str("    require: 2+ monster\n");
        ds.push_str("  }\n\n");
        notes.push(MigrationNote {
            kind: NoteKind::AutoMigrated,
            message: "Link summon procedure detected — materials block generated".to_string(),
        });
        confidence = Confidence::Medium;
    }
    if patterns.contains_key(&LuaPattern::EnableReviveLimit) {
        ds.push_str("  summon_condition {\n");
        ds.push_str("    must_be_summoned_by: own_effect\n");
        ds.push_str("  }\n\n");
        notes.push(MigrationNote {
            kind:    NoteKind::AutoMigrated,
            message: "EnableReviveLimit detected → summon_condition: must_be_summoned_by own_effect".to_string(),
        });
    }

    // ── Effects ────────────────────────────────────────────────
    let freq = if patterns.contains_key(&LuaPattern::OncePerDuel) {
        "  once_per_duel: true\n"
    } else if patterns.contains_key(&LuaPattern::OncePerTurn) {
        "  once_per_turn: true\n"
    } else {
        ""
    };

    let condition = if patterns.contains_key(&LuaPattern::HandCondition)
        && patterns.contains_key(&LuaPattern::TriggerEffect)
    {
        "  condition: in_hand\n"
    } else if patterns.contains_key(&LuaPattern::GraveyardCondition) {
        "  condition: in_gy\n"
    } else if patterns.contains_key(&LuaPattern::FieldCondition) {
        "  condition: on_field\n"
    } else {
        ""
    };

    let speed = if patterns.contains_key(&LuaPattern::QuickEffect) {
        "  speed: spell_speed_2\n"
    } else {
        "  speed: spell_speed_1\n"
    };

    for i in 0..effect_count.max(1) {
        ds.push_str(&format!("  effect \"Effect {}\" {{\n", i + 1));
        ds.push_str(speed);
        ds.push_str(freq);
        ds.push_str(condition);

        // Cost block
        ds.push_str("    cost {\n");
        if patterns.contains_key(&LuaPattern::DetachCost) {
            ds.push_str("      detach 1 overlay_unit from self\n");
            notes.push(MigrationNote {
                kind:    NoteKind::AutoMigrated,
                message: "DetachFromSelf → detach 1 overlay_unit from self".to_string(),
            });
        } else if patterns.contains_key(&LuaPattern::DiscardCost) {
            ds.push_str("      // TODO: discard (1, card) OR discard self\n");
            notes.push(MigrationNote {
                kind:    NoteKind::NeedsReview,
                message: "Cost.Discard detected — verify target".to_string(),
            });
        } else if patterns.contains_key(&LuaPattern::PayLpCost) {
            ds.push_str("      // TODO: pay_lp N\n");
            notes.push(MigrationNote {
                kind:    NoteKind::NeedsReview,
                message: "Cost.PayLp detected — add LP amount".to_string(),
            });
        } else if patterns.contains_key(&LuaPattern::TributeCost) {
            ds.push_str("      // TODO: tribute (1, monster, you controls)\n");
            notes.push(MigrationNote {
                kind:    NoteKind::NeedsReview,
                message: "Cost.Tribute detected — verify tribute target".to_string(),
            });
        } else {
            ds.push_str("      none\n");
        }
        ds.push_str("    }\n\n");

        // Resolution block
        ds.push_str("    on_resolve {\n");
        if patterns.contains_key(&LuaPattern::DrawCards) {
            ds.push_str("      draw 1 // TODO: verify count\n");
            notes.push(MigrationNote {
                kind: NoteKind::AutoMigrated,
                message: "Duel.Draw → draw N (verify count)".to_string(),
            });
        }
        if patterns.contains_key(&LuaPattern::NegateEffect) {
            ds.push_str("      negate effect\n");
            notes.push(MigrationNote {
                kind: NoteKind::AutoMigrated,
                message: "Duel.NegateEffect → negate effect".to_string(),
            });
        }
        if patterns.contains_key(&LuaPattern::Destroy) {
            ds.push_str("      destroy (1, card, opponent controls) // TODO: verify target\n");
            notes.push(MigrationNote {
                kind: NoteKind::NeedsReview,
                message: "Duel.Destroy detected — verify target expression".to_string(),
            });
        }
        if patterns.contains_key(&LuaPattern::Banish) {
            ds.push_str("      banish (1, card) from field // TODO: verify zone and target\n");
            notes.push(MigrationNote {
                kind: NoteKind::NeedsReview,
                message: "Duel.Remove detected — verify banish source and target".to_string(),
            });
        }
        if patterns.contains_key(&LuaPattern::SpecialSummon) {
            ds.push_str("      special_summon self from gy // TODO: verify source zone\n");
            notes.push(MigrationNote {
                kind: NoteKind::NeedsReview,
                message: "Duel.SpecialSummon detected — verify target and zone".to_string(),
            });
        }
        if patterns.contains_key(&LuaPattern::SendToGy) {
            ds.push_str("      send (1, card) to gy // TODO: verify target\n");
            notes.push(MigrationNote {
                kind: NoteKind::NeedsReview,
                message: "Duel.SendtoGrave detected — verify target".to_string(),
            });
        }
        if !patterns.contains_key(&LuaPattern::DrawCards)
            && !patterns.contains_key(&LuaPattern::NegateEffect)
            && !patterns.contains_key(&LuaPattern::Destroy)
            && !patterns.contains_key(&LuaPattern::Banish)
            && !patterns.contains_key(&LuaPattern::SpecialSummon)
            && !patterns.contains_key(&LuaPattern::SendToGy)
        {
            ds.push_str("      // TODO: translate effect resolution\n");
            notes.push(MigrationNote {
                kind:    NoteKind::Unknown,
                message: "Resolution actions not auto-detected — manual translation required".to_string(),
            });
        }
        ds.push_str("    }\n");
        ds.push_str("  }\n\n");
    }

    ds.push_str("}\n");

    // Determine final confidence
    if notes.iter().all(|n| n.kind == NoteKind::AutoMigrated) {
        confidence = Confidence::High;
    } else if notes.iter().any(|n| n.kind == NoteKind::Unknown) {
        confidence = Confidence::Low;
    } else {
        confidence = Confidence::Medium;
    }

    // Deduplicate notes
    notes.dedup_by(|a, b| a.message == b.message);

    MigrationResult {
        passcode,
        source_path: PathBuf::from(format!("c{}.lua", passcode)),
        output_path: PathBuf::from(format!("c{}.ds", passcode)),
        ds_content: ds,
        confidence,
        notes,
    }
}

// ── Batch Migration ───────────────────────────────────────────

/// Migrate all .lua files in a directory to .ds skeletons.
/// Returns one MigrationResult per file processed.
pub fn migrate_directory(
    lua_dir:    &Path,
    output_dir: &Path,
    overwrite:  bool,
) -> Vec<MigrationResult> {
    let mut results = Vec::new();

    let entries = match fs::read_dir(lua_dir) {
        Ok(e)  => e,
        Err(e) => {
            eprintln!("Cannot read directory {}: {}", lua_dir.display(), e);
            return results;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "lua") {
            // Extract passcode from filename: c12345678.lua → 12345678
            let stem = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");

            if !stem.starts_with('c') { continue; }

            let passcode: u64 = match stem[1..].parse() {
                Ok(p)  => p,
                Err(_) => continue,
            };

            let out_path = output_dir.join(format!("c{}.ds", passcode));
            if out_path.exists() && !overwrite { continue; }

            let source = match fs::read_to_string(&path) {
                Ok(s)  => s,
                Err(_) => continue,
            };

            // card_name would normally come from CDB — use passcode as placeholder
            let result = generate_from_lua(
                &source,
                passcode,
                &format!("Card #{}", passcode), // replace with CDB name
            );

            // Write output
            if let Some(parent) = out_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&out_path, &result.ds_content);

            results.push(result);
        }
    }

    results
}

// ── Report ────────────────────────────────────────────────────

pub fn print_migration_report(results: &[MigrationResult]) {
    let total  = results.len();
    let full   = results.iter().filter(|r| r.confidence == Confidence::Full).count();
    let high   = results.iter().filter(|r| r.confidence == Confidence::High).count();
    let medium = results.iter().filter(|r| r.confidence == Confidence::Medium).count();
    let low    = results.iter().filter(|r| r.confidence == Confidence::Low).count();

    println!("\n── DuelScript Migration Report ──────────────────────");
    println!("  Total migrated:  {}", total);
    println!("  Full confidence: {} ({:.0}%)", full,   pct(full,   total));
    println!("  High confidence: {} ({:.0}%)", high,   pct(high,   total));
    println!("  Medium:          {} ({:.0}%)", medium, pct(medium, total));
    println!("  Needs work:      {} ({:.0}%)", low,    pct(low,    total));
    println!("─────────────────────────────────────────────────────\n");

    for result in results.iter().filter(|r| r.confidence == Confidence::Low) {
        println!("  [NEEDS WORK] c{}.ds", result.passcode);
        for note in result.notes.iter().filter(|n| n.kind == NoteKind::Unknown) {
            println!("    ✗ {}", note.message);
        }
    }
}

fn pct(n: usize, total: usize) -> f64 {
    if total == 0 { 0.0 } else { n as f64 / total as f64 * 100.0 }
}
