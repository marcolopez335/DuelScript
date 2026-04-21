// ============================================================
// DuelScript CDB Reader — cdb.rs
//
// Reads ProjectIgnis BabelCdb SQLite3 files (.cdb) into
// typed Rust structs. Links with DuelScript .ds files via
// card passcode.
//
// BabelCdb schema:
//   texts(id, name, desc, str1..str16)
//   datas(id, ot, alias, setcode, type, atk, def,
//         level, race, attribute, category)
//
// Usage:
//   let cdb = CdbReader::open("cards.cdb")?;
//   let card = cdb.get(95474755)?;  // Number 89 by passcode
//   let all  = cdb.all_cards();
// ============================================================

use std::{
    collections::HashMap,
    path::Path,
};

// ── Public Types ──────────────────────────────────────────────

/// A card record loaded from a BabelCdb SQLite file.
/// Provides the raw stat data that DuelScript .ds files
/// declare in structured form.
#[derive(Debug, Clone)]
pub struct CdbCard {
    /// Card passcode — the primary key linking CDB ↔ .ds files
    pub id:        u64,
    pub name:      String,
    pub desc:      String,

    // Stats
    pub atk:       i32,   // -2 = ? (variable)
    pub def:       i32,   // -2 = ? (variable), -1 = link (no def)
    pub level:     u32,   // also encodes rank/link/scale for those types
    pub race:      u64,   // bitmask — use race_name() to decode
    pub attribute: u64,   // bitmask — use attribute_name() to decode
    pub card_type: u64,   // bitmask — use type_names() to decode

    // Meta
    pub ot:        u32,   // OT = TCG/OCG/Both/Pre-release flag
    pub alias:     u64,   // alternate artwork alias passcode (0 if none)
    pub setcode:   u64,   // archetype setcode bitmask
    pub category:  u64,   // effect category bitmask

    // Flavor / extra strings
    pub strings:   Vec<String>,
}

impl CdbCard {
    // ── Derived helpers ───────────────────────────────────────

    pub fn is_monster(&self)  -> bool { self.card_type & 0x1    != 0 }
    pub fn is_spell(&self)    -> bool { self.card_type & 0x2    != 0 }
    pub fn is_trap(&self)     -> bool { self.card_type & 0x4    != 0 }
    pub fn is_effect(&self)   -> bool { self.card_type & 0x20   != 0 }
    pub fn is_fusion(&self)   -> bool { self.card_type & 0x40   != 0 }
    pub fn is_ritual(&self)   -> bool { self.card_type & 0x80   != 0 }
    pub fn is_tuner(&self)    -> bool { self.card_type & 0x1000 != 0 }
    pub fn is_synchro(&self)  -> bool { self.card_type & 0x2000 != 0 }
    pub fn is_xyz(&self)      -> bool { self.card_type & 0x800000 != 0 }
    pub fn is_link(&self)     -> bool { self.card_type & 0x4000000 != 0 }
    pub fn is_pendulum(&self) -> bool { self.card_type & 0x1000000 != 0 }
    pub fn is_normal(&self)   -> bool { self.card_type & 0x10   != 0 }
    pub fn is_flip(&self)     -> bool { self.card_type & 0x200  != 0 }
    pub fn is_gemini(&self)   -> bool { self.card_type & 0x400  != 0 }
    pub fn is_union(&self)    -> bool { self.card_type & 0x800  != 0 }
    pub fn is_spirit(&self)   -> bool { self.card_type & 0x2000000 != 0 }
    pub fn is_toon(&self)     -> bool { self.card_type & 0x8000 != 0 }
    pub fn is_counter_trap(&self) -> bool { self.card_type & 0x1000000 != 0 && self.is_trap() }
    pub fn is_quick_play(&self)   -> bool { self.card_type & 0x10000  != 0 }
    pub fn is_continuous(&self)   -> bool { self.card_type & 0x20000  != 0 }
    pub fn is_equip(&self)        -> bool { self.card_type & 0x40000  != 0 }
    pub fn is_field(&self)        -> bool { self.card_type & 0x80000  != 0 }

    pub fn is_extra_deck(&self) -> bool {
        self.is_fusion() || self.is_synchro() || self.is_xyz() || self.is_link()
    }

    /// ATK as a displayable string ("?" for variable)
    pub fn atk_str(&self) -> String {
        if self.atk == -2 { "?".to_string() } else { self.atk.to_string() }
    }

    /// DEF as a displayable string ("?" for variable, "—" for Link)
    pub fn def_str(&self) -> String {
        match self.def {
            -2 => "?".to_string(),
            -1 => "—".to_string(),
            n  => n.to_string(),
        }
    }

    /// Pendulum scale (upper 16 bits of level field)
    pub fn pendulum_scale(&self) -> u32 {
        if self.is_pendulum() { (self.level >> 24) & 0xFF } else { 0 }
    }

    /// Actual level/rank/link rating (lower 8 bits of level field)
    pub fn actual_level(&self) -> u32 {
        self.level & 0xFF
    }

    /// Human-readable attribute name
    pub fn attribute_name(&self) -> &'static str {
        match self.attribute {
            0x1  => "EARTH",
            0x2  => "WATER",
            0x4  => "FIRE",
            0x8  => "WIND",
            0x10 => "LIGHT",
            0x20 => "DARK",
            0x40 => "DIVINE",
            _    => "UNKNOWN",
        }
    }

    /// Human-readable race name
    pub fn race_name(&self) -> &'static str {
        match self.race {
            0x1        => "Warrior",
            0x2        => "Spellcaster",
            0x4        => "Fairy",
            0x8        => "Fiend",
            0x10       => "Zombie",
            0x20       => "Machine",
            0x40       => "Aqua",
            0x80       => "Pyro",
            0x100      => "Rock",
            0x200      => "Winged Beast",
            0x400      => "Plant",
            0x800      => "Insect",
            0x1000     => "Thunder",
            0x2000     => "Dragon",
            0x4000     => "Beast",
            0x8000     => "Beast-Warrior",
            0x10000    => "Dinosaur",
            0x20000    => "Fish",
            0x40000    => "Sea Serpent",
            0x80000    => "Reptile",
            0x100000   => "Psychic",
            0x200000   => "Divine-Beast",
            0x400000   => "Creator-God",
            0x800000   => "Wyrm",
            0x1000000  => "Cyberse",
            _          => "Unknown",
        }
    }

    /// All card type labels
    pub fn type_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.is_monster()  { names.push("Monster"); }
        if self.is_spell()    { names.push("Spell"); }
        if self.is_trap()     { names.push("Trap"); }
        if self.is_normal()   { names.push("Normal"); }
        if self.is_effect()   { names.push("Effect"); }
        if self.is_fusion()   { names.push("Fusion"); }
        if self.is_ritual()   { names.push("Ritual"); }
        if self.is_synchro()  { names.push("Synchro"); }
        if self.is_xyz()      { names.push("Xyz"); }
        if self.is_link()     { names.push("Link"); }
        if self.is_pendulum() { names.push("Pendulum"); }
        if self.is_tuner()    { names.push("Tuner"); }
        if self.is_flip()     { names.push("Flip"); }
        if self.is_gemini()   { names.push("Gemini"); }
        if self.is_union()    { names.push("Union"); }
        if self.is_spirit()   { names.push("Spirit"); }
        if self.is_toon()     { names.push("Toon"); }
        if self.is_quick_play()  { names.push("Quick-Play"); }
        if self.is_continuous()  { names.push("Continuous"); }
        if self.is_equip()       { names.push("Equip"); }
        if self.is_field()       { names.push("Field"); }
        names
    }

    /// OT region
    pub fn region(&self) -> CdbRegion {
        match self.ot {
            0x1  => CdbRegion::Ocg,
            0x2  => CdbRegion::Tcg,
            0x3  => CdbRegion::Both,
            0x100 => CdbRegion::Prerelease,
            _    => CdbRegion::Unknown,
        }
    }

    /// Generate a DuelScript skeleton .ds file from CDB data.
    /// The engine can use this as a starting point — effects
    /// still need to be filled in manually or via migration tool.
    pub fn to_ds_skeleton(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!("// Generated from BabelCdb passcode {}\n", self.id));
        out.push_str(&format!("// {}\n", self.region().label()));
        out.push_str("// Effects require manual scripting — see docs/V2_LANGUAGE_REFERENCE.md\n\n");

        out.push_str(&format!("card \"{}\" {{\n", self.name));
        out.push_str(&format!("    id: {}\n\n", self.id));

        // Type line
        let types = self.ds_type_line();
        out.push_str(&format!("    type: {}\n", types));

        // Monster fields
        if self.is_monster() {
            out.push_str(&format!("    attribute: {}\n", self.attribute_name()));
            out.push_str(&format!("    race: {}\n", self.race_name()));

            if self.is_xyz() {
                out.push_str(&format!("    rank: {}\n", self.actual_level()));
            } else if self.is_link() {
                out.push_str(&format!("    link: {}\n", self.actual_level()));
            } else {
                out.push_str(&format!("    level: {}\n", self.actual_level()));
            }

            if self.is_pendulum() {
                out.push_str(&format!("    scale: {}\n", self.pendulum_scale()));
            }

            out.push_str(&format!("    atk: {}\n", self.atk_str()));
            if !self.is_link() {
                out.push_str(&format!("    def: {}\n", self.def_str()));
            }
        }

        // Flavor for normal monsters — v2 has no `flavor:` field, so emit as comments.
        if self.is_normal() && !self.desc.is_empty() {
            out.push_str("\n    // Flavor text:\n");
            for line in self.desc.lines() {
                out.push_str(&format!("    // {}\n", line));
            }
        }

        // Effect stub — v2 requires a labeled effect block.
        if self.is_effect() || (!self.is_monster() && !self.is_normal()) {
            out.push_str("\n    // TODO: translate effect\n");
            out.push_str("    // Original text:\n");
            for line in self.desc.lines() {
                out.push_str(&format!("    // {}\n", line));
            }
            out.push_str("\n    effect \"Effect 1\" {\n");
            out.push_str("        speed: 1\n");
            out.push_str("        // TODO: add condition / cost / resolve\n");
            out.push_str("    }\n");
        }

        out.push_str("}\n");
        out
    }

    /// Sprint 15: Decode the link arrow bitmask (stored in `def` for
    /// link monsters in BabelCdb) into the DSL's named arrows.
    /// Returns an empty Vec for non-link cards.
    pub fn link_arrow_names(&self) -> Vec<&'static str> {
        if !self.is_link() { return Vec::new(); }
        // EDOPro LINK_MARKER bits stored in `def`:
        //   0x001 BL, 0x002 B,  0x004 BR
        //   0x008 L,            0x020 R
        //   0x040 TL, 0x080 T,  0x100 TR
        let mask = self.def as u32;
        let mut out = Vec::new();
        if mask & 0x040 != 0 { out.push("top_left"); }
        if mask & 0x080 != 0 { out.push("top"); }
        if mask & 0x100 != 0 { out.push("top_right"); }
        if mask & 0x008 != 0 { out.push("left"); }
        if mask & 0x020 != 0 { out.push("right"); }
        if mask & 0x001 != 0 { out.push("bottom_left"); }
        if mask & 0x002 != 0 { out.push("bottom"); }
        if mask & 0x004 != 0 { out.push("bottom_right"); }
        out
    }

    pub fn ds_type_line(&self) -> String {
        let mut parts = Vec::new();

        if self.is_fusion()   { parts.push("Fusion Monster"); }
        else if self.is_synchro() { parts.push("Synchro Monster"); }
        else if self.is_xyz() { parts.push("Xyz Monster"); }
        else if self.is_link(){ parts.push("Link Monster"); }
        else if self.is_ritual() && self.is_monster() { parts.push("Ritual Monster"); }
        else if self.is_normal() { parts.push("Normal Monster"); }
        else if self.is_effect() { parts.push("Effect Monster"); }
        // Pendulum is a property, not a base type — `Pendulum Monster`
        // alone fails validation because pendulums must also be either
        // Normal or Effect monsters. Emit it as a secondary qualifier.
        if self.is_pendulum() && self.is_monster() { parts.push("Pendulum Monster"); }
        // Backstop: any monster card that didn't match a base type yet
        // gets "Effect Monster" so the validator accepts it.
        if self.is_monster() && parts.is_empty() {
            parts.push("Effect Monster");
        }
        // ── spells/traps below — pendulum check above doesn't apply
        if self.is_spell() {
            if self.is_ritual()    { parts.push("Ritual Spell"); }
            else if self.is_quick_play() { parts.push("Quick-Play Spell"); }
            else if self.is_continuous() { parts.push("Continuous Spell"); }
            else if self.is_equip()      { parts.push("Equip Spell"); }
            else if self.is_field()      { parts.push("Field Spell"); }
            else                         { parts.push("Normal Spell"); }
        } else if self.is_trap() {
            if self.is_counter_trap()   { parts.push("Counter Trap"); }
            else if self.is_continuous(){ parts.push("Continuous Trap"); }
            else                        { parts.push("Normal Trap"); }
        }

        if self.is_tuner() && self.is_monster() { parts.push("Tuner"); }
        if self.is_flip()    { parts.push("Flip"); }
        if self.is_gemini()  { parts.push("Gemini"); }
        if self.is_union()   { parts.push("Union"); }
        if self.is_spirit()  { parts.push("Spirit"); }
        if self.is_toon()    { parts.push("Toon"); }

        parts.join(" | ")
    }
}

// ── Region ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CdbRegion {
    Ocg, Tcg, Both, Prerelease, Unknown,
}

impl CdbRegion {
    pub fn label(&self) -> &'static str {
        match self {
            CdbRegion::Ocg        => "OCG",
            CdbRegion::Tcg        => "TCG",
            CdbRegion::Both       => "TCG/OCG",
            CdbRegion::Prerelease => "Pre-release",
            CdbRegion::Unknown    => "Unknown",
        }
    }
}

// ── CDB Reader ────────────────────────────────────────────────

/// Reads a BabelCdb SQLite3 .cdb file.
/// Uses raw SQLite queries — no ORM dependency.
pub struct CdbReader {
    cards: HashMap<u64, CdbCard>,
}

impl CdbReader {
    /// Open and read a .cdb file into memory.
    /// Requires the `rusqlite` feature to be enabled.
    ///
    /// In your Cargo.toml:
    ///   [dependencies]
    ///   rusqlite = { version = "0.31", features = ["bundled"] }
    #[cfg(feature = "cdb")]
    pub fn open(path: &Path) -> Result<Self, CdbError> {
        use rusqlite::{Connection, OpenFlags};

        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).map_err(|e| CdbError::SqliteError(e.to_string()))?;

        let mut cards = HashMap::new();

        // Join texts + datas on id
        let mut stmt = conn.prepare(
            "SELECT
               d.id, d.ot, d.alias, d.setcode,
               d.type, d.atk, d.def, d.level, d.race, d.attribute, d.category,
               t.name, t.desc,
               t.str1,  t.str2,  t.str3,  t.str4,
               t.str5,  t.str6,  t.str7,  t.str8,
               t.str9,  t.str10, t.str11, t.str12,
               t.str13, t.str14, t.str15, t.str16
             FROM datas d
             JOIN texts t ON d.id = t.id
             ORDER BY d.id"
        ).map_err(|e| CdbError::SqliteError(e.to_string()))?;

        let rows = stmt.query_map([], |row| {
            let strings: Vec<String> = (13..29usize)
                .map(|i| row.get::<_, String>(i).unwrap_or_default())
                .filter(|s| !s.is_empty())
                .collect();

            Ok(CdbCard {
                id:        row.get::<_, i64>(0)? as u64,
                ot:        row.get::<_, i64>(1)? as u32,
                alias:     row.get::<_, i64>(2)? as u64,
                setcode:   row.get::<_, i64>(3)? as u64,
                card_type: row.get::<_, i64>(4)? as u64,
                atk:       row.get::<_, i64>(5)? as i32,
                def:       row.get::<_, i64>(6)? as i32,
                level:     row.get::<_, i64>(7)? as u32,
                race:      row.get::<_, i64>(8)? as u64,
                attribute: row.get::<_, i64>(9)? as u64,
                category:  row.get::<_, i64>(10)? as u64,
                name:      row.get(11)?,
                desc:      row.get(12)?,
                strings,
            })
        }).map_err(|e| CdbError::SqliteError(e.to_string()))?;

        for row in rows {
            let card = row.map_err(|e| CdbError::SqliteError(e.to_string()))?;
            cards.insert(card.id, card);
        }

        Ok(Self { cards })
    }

    /// Stub open for when the cdb feature is not enabled.
    /// Returns an error — enable the "cdb" feature to use.
    #[cfg(not(feature = "cdb"))]
    pub fn open(_path: &Path) -> Result<Self, CdbError> {
        Err(CdbError::FeatureNotEnabled)
    }

    /// Look up a card by passcode.
    pub fn get(&self, passcode: u64) -> Option<&CdbCard> {
        self.cards.get(&passcode)
    }

    /// All cards in the database.
    pub fn all_cards(&self) -> Vec<&CdbCard> {
        let mut cards: Vec<_> = self.cards.values().collect();
        cards.sort_by_key(|c| c.id);
        cards
    }

    /// All monster cards.
    pub fn monsters(&self) -> Vec<&CdbCard> {
        self.cards.values().filter(|c| c.is_monster()).collect()
    }

    /// Search by name (case-insensitive substring).
    pub fn search_name(&self, query: &str) -> Vec<&CdbCard> {
        let q = query.to_lowercase();
        self.cards.values()
            .filter(|c| c.name.to_lowercase().contains(&q))
            .collect()
    }

    /// Cards belonging to an archetype setcode.
    pub fn by_setcode(&self, setcode: u64) -> Vec<&CdbCard> {
        self.cards.values()
            .filter(|c| c.setcode & setcode != 0)
            .collect()
    }

    /// Total cards loaded.
    pub fn len(&self) -> usize { self.cards.len() }

    pub fn is_empty(&self) -> bool { self.cards.is_empty() }

    // ── Generation ────────────────────────────────────────────

    /// Generate .ds skeleton files for cards that don't have one yet.
    /// Returns a Vec of (passcode, ds_content) pairs.
    pub fn generate_missing_skeletons(
        &self,
        existing_dir: &Path,
    ) -> Vec<(u64, String)> {
        let mut missing = Vec::new();

        for card in self.all_cards() {
            // Skip alternate artworks (alias within 10 of id)
            if card.alias != 0 && card.alias.abs_diff(card.id) <= 10 {
                continue;
            }

            let filename = format!("c{}.ds", card.id);
            let filepath = existing_dir.join(&filename);

            if !filepath.exists() {
                missing.push((card.id, card.to_ds_skeleton()));
            }
        }

        missing
    }
}

// ── Merged Card ───────────────────────────────────────────────

/// A card with both CDB stat data AND parsed DuelScript v2 behavior.
/// This is the complete picture your engine works with.
#[derive(Debug)]
pub struct MergedCard {
    /// Raw stat data from BabelCdb
    pub cdb:  CdbCard,
    /// Parsed v2 behavior from .ds file (None if not yet scripted)
    pub ds:   Option<std::sync::Arc<crate::v2::ast::Card>>,
}

impl MergedCard {
    /// Whether this card has a DuelScript behavior file.
    pub fn has_script(&self) -> bool { self.ds.is_some() }

    /// Whether this card only has CDB data (not yet ported to DuelScript).
    pub fn is_cdb_only(&self) -> bool { self.ds.is_none() }

    /// Display name — prefers DS name, falls back to CDB name.
    pub fn name(&self) -> &str {
        self.ds.as_ref().map_or(&self.cdb.name, |d| &d.name)
    }
}

/// Merge a CDB reader and a map of v2 DuelScript cards (keyed by passcode)
/// into a unified view of all cards. Cards in CDB but not in DS
/// are included with ds=None (not yet ported).
pub fn merge_databases(
    cdb: &CdbReader,
    ds:  &std::collections::HashMap<u32, std::sync::Arc<crate::v2::ast::Card>>,
) -> Vec<MergedCard> {
    let mut merged = Vec::new();

    for cdb_card in cdb.all_cards() {
        let ds_card = ds.get(&(cdb_card.id as u32)).cloned();
        merged.push(MergedCard {
            cdb: cdb_card.clone(),
            ds:  ds_card,
        });
    }

    merged.sort_by_key(|m| m.cdb.id);
    merged
}

// ── Error ─────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CdbError {
    SqliteError(String),
    FileNotFound(String),
    FeatureNotEnabled,
}

impl std::fmt::Display for CdbError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CdbError::SqliteError(e)  => write!(f, "SQLite error: {}", e),
            CdbError::FileNotFound(p) => write!(f, "CDB file not found: {}", p),
            CdbError::FeatureNotEnabled => write!(f, "Enable the 'cdb' feature in Cargo.toml: rusqlite = {{ version = \"0.31\", features = [\"bundled\"] }}"),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::parser::parse_v2;

    fn bare_card(card_type: u64) -> CdbCard {
        CdbCard {
            id:        12345,
            name:      "Test Card".to_string(),
            desc:      "Some flavor text.\nSecond line.".to_string(),
            atk:       1500,
            def:       1200,
            level:     4,
            race:      0x1, // Warrior
            attribute: 0x20, // DARK
            card_type,
            ot:        0x1,
            alias:     0,
            setcode:   0,
            category:  0,
            strings:   Vec::new(),
        }
    }

    #[test]
    fn skeleton_parses_normal_monster() {
        // type bits: monster (0x1) | normal (0x10)
        let card = bare_card(0x1 | 0x10);
        let src = card.to_ds_skeleton();
        parse_v2(&src).unwrap_or_else(|e| panic!("skeleton did not parse:\n{src}\n\nerror: {e}"));
    }

    #[test]
    fn skeleton_parses_effect_monster() {
        // type bits: monster (0x1) | effect (0x20)
        let card = bare_card(0x1 | 0x20);
        let src = card.to_ds_skeleton();
        parse_v2(&src).unwrap_or_else(|e| panic!("skeleton did not parse:\n{src}\n\nerror: {e}"));
    }

    #[test]
    fn skeleton_parses_spell() {
        // type bits: spell (0x2)
        let card = bare_card(0x2);
        let src = card.to_ds_skeleton();
        parse_v2(&src).unwrap_or_else(|e| panic!("skeleton did not parse:\n{src}\n\nerror: {e}"));
    }
}

impl std::error::Error for CdbError {}
