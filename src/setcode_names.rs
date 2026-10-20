// ============================================================
// setcode_names — `SET_*` archetype constants → archetype names
//
// T39 S3 (selector filter fidelity). The lua corpus names archetypes
// by constant (`c:IsSetCard(SET_KOZMO)`); the DSL names them by the
// string the compiler's `ArchetypeIs` predicate expects (`archetype
// == "Kozmo"`). No strings.conf ships with the workspace, so the name
// is derived from the cdb: the constant's words, matched case-
// insensitively (with optional space / hyphen separators) against the
// names of the archetype's member cards, with the spelling the cards
// use. A constant resolves only when the match covers at least 85% of
// its members (`MIN_COVERAGE`) — the pre-existing curated table in
// `lua_ast` takes precedence, and `OVERRIDES` below pins the handful
// of constants whose words differ from the printed archetype name
// (`SET_TELLARKNIGHT` → "tellarknight", `SET_FA` → "F.A.").
//
// Pure text helpers — no lua or sqlite dependency — so the derivation
// is unit-testable against fixtures.
// ============================================================

use std::collections::BTreeMap;

/// Minimum fraction of an archetype's member cards whose name must
/// contain the derived string.
pub const MIN_COVERAGE: f64 = 0.85;

/// Constants whose words are not the printed archetype name. Checked
/// by hand against the cdb member names (see the PR table).
const OVERRIDES: &[(&str, &str)] = &[
    ("SET_TELLARKNIGHT", "tellarknight"),
    ("SET_STELLARKNIGHT", "Stellarknight"),
    ("SET_GISHKI", "Gishki"),
    ("SET_MAGNET_WARRIOR", "Magnet Warrior"),
    ("SET_RESCUE_ACE", "Rescue-ACE"),
    ("SET_NOBLE_ARMS", "Noble Arms"),
    ("SET_PURRELY", "Purrely"),
    ("SET_RITUAL_BEAST", "Ritual Beast"),
    ("SET_THE_PHANTOM_KNIGHTS", "The Phantom Knights"),
    ("SET_FA", "F.A."),
    ("SET_RB", "R.B."),
    ("SET_TG", "T.G."),
    ("SET_UA", "U.A."),
    ("SET_POSSESSED", "Possessed"),
    ("SET_ROKKET", "rokket"),
    ("SET_VENDREAD", "Vendread"),
    ("SET_HERO", "HERO"),
    ("SET_LSWARM", "lswarm"),
    ("SET_CODE_TALKER", "Code Talker"),
    ("SET_IGNISTER", "@Ignister"),
];

/// `SET_NAME = 0x…` lines of `archetype_setcode_constants.lua`.
pub fn parse_setcode_constants(lua: &str) -> Vec<(String, u64)> {
    parse_constants(lua, "SET_")
        .into_iter()
        .map(|(n, v)| (n, v))
        .collect()
}

/// `CARD_NAME = <passcode>` lines of `card_counter_constants.lua`.
pub fn parse_card_constants(lua: &str) -> Vec<(String, u32)> {
    parse_constants(lua, "CARD_")
        .into_iter()
        .filter_map(|(n, v)| u32::try_from(v).ok().map(|v| (n, v)))
        .collect()
}

fn parse_constants(lua: &str, prefix: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for line in lua.lines() {
        let line = line.trim();
        if !line.starts_with(prefix) { continue; }
        let Some((name, value)) = line.split_once('=') else { continue };
        let name = name.trim();
        if !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            continue;
        }
        let value = value.split("--").next().unwrap_or("").trim();
        let parsed = match value.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            None => value.parse::<u64>().ok(),
        };
        if let Some(v) = parsed {
            out.push((name.to_string(), v));
        }
    }
    out
}

/// ocgcore `card::is_set_card`: a card's packed setcode (four 16-bit
/// chunks) belongs to `set` when a chunk's base (low 12 bits) equals
/// the set's base and the chunk carries every sub-archetype bit (high
/// 4 bits) the set carries.
pub fn is_member(card_setcode: u64, set: u64) -> bool {
    (0..4).any(|k| {
        let chunk = (card_setcode >> (16 * k)) & 0xffff;
        chunk != 0
            && (chunk & 0xfff) == (set & 0xfff)
            && (chunk & set & 0xf000) == (set & 0xf000)
    })
}

/// One cdb card as the derivation sees it.
pub struct CardRow<'a> {
    pub setcode: u64,
    pub card_type: u64,
    pub name: &'a str,
}

/// Derive `(constant, archetype name)` pairs. Tokens (TYPE_TOKEN =
/// 0x4000) are not members. Constants with no member, or whose
/// derived string covers fewer than `MIN_COVERAGE` of the members,
/// are left out — the translator then treats the atom as unmapped.
pub fn derive_archetype_names(consts: &[(String, u64)], cards: &[CardRow<'_>]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, code) in consts {
        if let Some((_, pinned)) = OVERRIDES.iter().find(|(c, _)| c == name) {
            out.push((name.clone(), pinned.to_string()));
            continue;
        }
        let members: Vec<&str> = cards
            .iter()
            .filter(|c| c.card_type & 0x4000 == 0 && is_member(c.setcode, *code))
            .map(|c| c.name)
            .collect();
        if members.is_empty() { continue; }
        let words: Vec<String> = name
            .trim_start_matches("SET_")
            .split('_')
            .filter(|w| !w.is_empty())
            .map(|w| w.to_ascii_lowercase())
            .collect();
        if words.is_empty() { continue; }
        let mut found: BTreeMap<String, usize> = BTreeMap::new();
        for m in &members {
            if let Some(s) = find_words(m, &words) {
                *found.entry(s).or_insert(0) += 1;
            }
        }
        // Coverage counts every spelling the match produced
        // ("Mathmech" / "mathmech"); the most common spelling is the
        // archetype name.
        let matched: usize = found.values().sum();
        let Some((best, _)) = found.into_iter().max_by_key(|(s, n)| (*n, std::cmp::Reverse(s.clone()))) else { continue };
        if best.len() < 2 { continue; }
        if (matched as f64) / (members.len() as f64) < MIN_COVERAGE { continue; }
        out.push((name.clone(), best));
    }
    out
}

/// Find `words` in `name` in order, case-insensitively, each pair
/// joined by nothing, one space, or one hyphen. Returns the matched
/// span with the name's own spelling.
fn find_words(name: &str, words: &[String]) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let lb = lower.as_bytes();
    'start: for start in 0..lb.len() {
        if !lower.is_char_boundary(start) { continue; }
        let mut pos = start;
        for (wi, w) in words.iter().enumerate() {
            if wi > 0 && pos < lb.len() && (lb[pos] == b' ' || lb[pos] == b'-') {
                pos += 1;
            }
            if !lower.is_char_boundary(pos) || !lower[pos..].starts_with(w.as_str()) {
                continue 'start;
            }
            pos += w.len();
        }
        // ASCII lowercasing preserves byte offsets, so the span is a
        // char boundary of `name` too.
        return Some(name[start..pos].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows<'a>(v: &'a [(u64, &'a str)]) -> Vec<CardRow<'a>> {
        v.iter().map(|(sc, n)| CardRow { setcode: *sc, card_type: 0x21, name: n }).collect()
    }

    #[test]
    fn parses_hex_and_decimal_constants() {
        let lua = "-- header\nSET_KOZMO = 0xd2\nSET_R_GENEX = 0x1002 -- sub\nlocal x = 1\n";
        assert_eq!(parse_setcode_constants(lua), vec![
            ("SET_KOZMO".to_string(), 0xd2),
            ("SET_R_GENEX".to_string(), 0x1002),
        ]);
        let cards = "CARD_POLYMERIZATION = 24094653\nCARD_DARK_MAGICIAN=46986414\n";
        assert_eq!(parse_card_constants(cards), vec![
            ("CARD_POLYMERIZATION".to_string(), 24094653),
            ("CARD_DARK_MAGICIAN".to_string(), 46986414),
        ]);
    }

    #[test]
    fn membership_follows_ocgcore_sub_archetype_rule() {
        assert!(is_member(0x2, 0x2));          // Genex is Genex
        assert!(is_member(0x1002, 0x2));       // R-Genex is Genex
        assert!(!is_member(0x2, 0x1002));      // Genex is not R-Genex
        assert!(is_member(0x00d2_0002, 0x2));  // second chunk
        assert!(!is_member(0, 0x2));
    }

    #[test]
    fn derives_the_card_spelling_with_separators() {
        let consts = vec![
            ("SET_FIRE_FORMATION".to_string(), 0x7c),
            ("SET_KOZMO".to_string(), 0xd2),
            ("SET_WIND_UP".to_string(), 0x55),
        ];
        let cards = rows(&[
            (0x7c, "Fire Formation - Tenki"),
            (0x7c, "Ultimate Fire Formation - Seito"),
            (0xd2, "Kozmo Farmgirl"),
            (0xd2, "Kozmoll Wickedwitch"),
            (0x55, "Wind-Up Zenmaines"),
            (0x55, "Wind-Up Factory"),
        ]);
        let got = derive_archetype_names(&consts, &cards);
        assert_eq!(got, vec![
            ("SET_FIRE_FORMATION".to_string(), "Fire Formation".to_string()),
            ("SET_KOZMO".to_string(), "Kozmo".to_string()),
            ("SET_WIND_UP".to_string(), "Wind-Up".to_string()),
        ]);
    }

    #[test]
    fn low_coverage_and_tokens_are_left_out() {
        let consts = vec![("SET_GLADIATOR".to_string(), 0x19), ("SET_EMPTY".to_string(), 0x99)];
        let mut cards = rows(&[
            (0x19, "Gladiator Beast Laquari"),
            (0x19, "Test Ape"),
            (0x19, "Test Tiger"),
        ]);
        // A token named after the archetype never counts.
        cards.push(CardRow { setcode: 0x19, card_type: 0x4021, name: "Gladiator Token" });
        assert!(derive_archetype_names(&consts, &cards).is_empty());
    }

    #[test]
    fn overrides_win_regardless_of_members() {
        let consts = vec![("SET_FA".to_string(), 0x11c)];
        let cards = rows(&[(0x11c, "F.A. Test Drive")]);
        assert_eq!(derive_archetype_names(&consts, &cards), vec![("SET_FA".to_string(), "F.A.".to_string())]);
    }
}
