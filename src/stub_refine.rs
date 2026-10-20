// ============================================================
// stub_refine — T38 stub refinement (apply "Pass S") decision helpers.
//
// The corpus generator wrote one placeholder line per lua `SetCategory`
// bit it could spell — `special_summon (all, card, either controls)`,
// `send (all, card, either controls) to gy`, … — into otherwise-empty
// resolve blocks. The lines validate (every card on the field is a
// legal selector), so `duelscript check` never flags them and the apply
// passes, which only fill EMPTY resolves, never rewrite them. Pass S
// replaces a resolve whose body is exactly ONE such line when the
// block's owner handler produces a complete fill under every Pass A
// gate AND the rendered lines state the same category.
//
// Everything here is a pure function of text, so it lives in the lib
// (unit-tested under `cargo test --lib`); the apply integration is in
// src/bin/lua_translate.rs.
// ============================================================

/// The generator's placeholder selector — every stub line carries it
/// verbatim, directly after the verb.
pub const STUB_SELECTOR: &str = "(all, card, either controls)";

/// A card-moving category, keyed to the lua `CATEGORY_*` bit the
/// generator spelled the stub from. Two DSL spellings can share one
/// category (see [`SPELLINGS`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    SpecialSummon,
    ToHand,
    Destroy,
    ToGrave,
    ToDeck,
    ToExtra,
    Remove,
    Control,
    Discard,
}

impl Category {
    /// Report label (matches the stub's own spelling).
    pub fn name(self) -> &'static str {
        match self {
            Category::SpecialSummon => "special_summon",
            Category::ToHand => "add_to_hand",
            Category::Destroy => "destroy",
            Category::ToGrave => "send-to-gy",
            Category::ToDeck => "return-to-deck",
            Category::ToExtra => "send-to-extra_deck",
            Category::Remove => "banish",
            Category::Control => "take_control",
            Category::Discard => "discard",
        }
    }

    /// Card-text phrases that evidence this category. Used ONLY for
    /// rider lines (a rendered line of a category OTHER than the
    /// stub's) — the stub's own category needs no text evidence, the
    /// lua `SetCategory` already declared it. Matched case-insensitively
    /// against the card's full `desc`.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Category::SpecialSummon => &["special summon"],
            Category::ToHand => &[
                "to the hand", "to your hand", "to their hand", "owner's hand",
                "to the hands",
            ],
            Category::Destroy => &["destroy"],
            Category::ToGrave => &["to the gy", "to the graveyard"],
            Category::ToDeck => &["to the deck", "into the deck", "to the main deck"],
            Category::ToExtra => &["to the extra deck"],
            Category::Remove => &["banish"],
            Category::Control => &["take control", "gain control"],
            Category::Discard => &["discard"],
        }
    }
}

/// Spelling → category table. A spelling is the action verb, plus
/// ` to <dest>` for the `send` / `return` family (destination = the
/// zone token after the LAST ` to `). The generator and the translator
/// spell two categories differently; the table is the documented
/// synonym set, each entry verified against real cards + card text
/// (PR body, "Synonym table"):
///
/// | lua category       | generator stub       | translator rendering | evidence |
/// |--------------------|----------------------|----------------------|----------|
/// | `CATEGORY_TOHAND`  | `return … to hand`   | `add_to_hand …`      | c10805153 Witchcrafter Collaboration ("add this card from your GY to your hand"), c11759079 Dual Avatar Feet - Kokoku ("add 1 'Dual Avatar' Trap from your Deck to your hand"), c1186447 / c13533678 / c13597785 (deck searches) — `Duel.SendtoHand` renders `add_to_hand` whatever the source zone |
/// | `CATEGORY_TODECK`  | `return … to deck`   | `send … to deck`     | c11548522 Outstanding Dog Marron ("shuffle this card into the Deck"), c11868731, c12948099 ("shuffle all monsters … into the Deck"), c14001430 ("shuffle … from your GY into the Deck") — `Duel.SendtoDeck` renders `send … to deck` |
///
/// NOT admitted (no evidence in the refill set, so no entry):
/// `return … to extra_deck` (0 renderings), `fusion_summon` /
/// `ritual_summon` / `synchro_summon` helper lines as `special_summon`
/// (0 renderings), `return … to owner` / `… to field`.
pub const SPELLINGS: &[(&str, Category)] = &[
    ("special_summon", Category::SpecialSummon),
    ("add_to_hand", Category::ToHand),
    ("return to hand", Category::ToHand),
    ("destroy", Category::Destroy),
    ("send to gy", Category::ToGrave),
    ("send to graveyard", Category::ToGrave),
    ("send to deck", Category::ToDeck),
    ("return to deck", Category::ToDeck),
    ("send to extra_deck", Category::ToExtra),
    ("banish", Category::Remove),
    ("take_control", Category::Control),
    ("discard", Category::Discard),
];

/// Rider verbs — rendered lines outside the category vocabulary that
/// may accompany the stub's category when the card text mentions them
/// (any keyword, case-insensitive). Verbs absent from this table
/// (`grant`, `install_watcher`, `delayed`, `damage_rule`, `announce`,
/// `place_counter`, `draw`, `damage`, …) are NOT admitted: a body
/// carrying one skips and counts as a category mismatch.
pub const RIDERS: &[(&str, &[&str])] = &[
    ("restrict", &["cannot"]),
    ("change_position", &["position"]),
    ("attach", &["attach"]),
    ("modify_level", &["level"]),
    ("set_level", &["level"]),
    ("modify_atk", &["atk"]),
    ("set_atk", &["atk"]),
    ("modify_def", &["def"]),
    ("set_def", &["def"]),
    ("set", &["set"]),
    ("shuffle_deck", &["shuffle"]),
    ("shuffle_hand", &["shuffle"]),
    ("change_attribute", &["attribute"]),
    ("normal_summon", &["normal summon"]),
];

/// How a rendered action line classifies for the category match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineClass {
    /// A category-vocabulary line (see [`SPELLINGS`]).
    Category(Category),
    /// A rider verb (see [`RIDERS`]) with its card-text keywords.
    Rider(&'static [&'static str]),
    /// Anything else — never admitted.
    Unknown,
}

/// The spelling key of a rendered line: its verb, plus ` to <dest>`
/// for `send` / `return`. The destination is the first token after the
/// last ` to ` (`return self to deck shuffle` → `return to deck`).
fn spelling_key(line: &str) -> Option<String> {
    let line = line.trim();
    let verb = line.split_whitespace().next()?;
    if verb == "send" || verb == "return" {
        let (_, tail) = line.rsplit_once(" to ")?;
        let dest = tail.split_whitespace().next()?;
        return Some(format!("{} to {}", verb, dest));
    }
    Some(verb.to_string())
}

/// Classify one rendered action line by its verb (+ destination).
pub fn line_class(line: &str) -> LineClass {
    let Some(key) = spelling_key(line) else { return LineClass::Unknown };
    if let Some((_, cat)) = SPELLINGS.iter().find(|(k, _)| *k == key) {
        return LineClass::Category(*cat);
    }
    if let Some((_, kws)) = RIDERS.iter().find(|(k, _)| *k == key) {
        return LineClass::Rider(kws);
    }
    LineClass::Unknown
}

/// Parse one trimmed line as a generator category stub: `<verb>
/// (all, card, either controls)[ to <dest>]`, nothing else on the line.
pub fn stub_line_category(line: &str) -> Option<Category> {
    let line = line.trim();
    let (verb, rest) = line.split_once(' ')?;
    let rest = rest.strip_prefix(STUB_SELECTOR)?;
    let key = match rest {
        "" => verb.to_string(),
        _ => {
            let dest = rest.strip_prefix(" to ")?;
            if dest.is_empty() || dest.contains(char::is_whitespace) {
                return None;
            }
            format!("{} to {}", verb, dest)
        }
    };
    SPELLINGS.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
}

/// The category of a resolve body that is EXACTLY one stub line —
/// `None` when the body is empty, has any other line (a second stub,
/// a real action, a comment), or the one line isn't a stub.
pub fn lone_stub_category(body: &str) -> Option<Category> {
    let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    stub_line_category(first)
}

/// The category-match verdict for replacing a `stub` with `rendered`
/// (the owner handler's action lines, trimmed): at least one rendered
/// line has the stub's category, AND every other line is either the
/// same category, another category the card text mentions, or a rider
/// the card text mentions. `desc` is the card's full text (`None` when
/// no cdb is loaded — then no rider or foreign-category line is
/// admitted). Conservative by construction: an unknown verb refuses.
pub fn category_match(stub: Category, rendered: &[String], desc: Option<&str>) -> bool {
    let desc_lc = desc.map(str::to_lowercase);
    let mentions = |kws: &[&str]| {
        desc_lc
            .as_deref()
            .map(|d| kws.iter().any(|k| d.contains(k)))
            .unwrap_or(false)
    };
    let mut has_stub_category = false;
    for line in rendered {
        match line_class(line) {
            LineClass::Category(c) if c == stub => has_stub_category = true,
            LineClass::Category(c) => {
                if !mentions(c.keywords()) {
                    return false;
                }
            }
            LineClass::Rider(kws) => {
                if !mentions(kws) {
                    return false;
                }
            }
            LineClass::Unknown => return false,
        }
    }
    has_stub_category
}

/// Pass S refusal for handler calls whose CURRENT translation is known
/// to be lossy — found by the hand-verification pass over the first
/// refinement run. Pass A ships these shapes on empty resolves (a
/// pre-existing policy with its own retrofit obligation — PR body,
/// "Backlog"); a stub rewrite must not add more. `(method, args)` is
/// one `Duel.*` call of the owner handler; returns the reason to skip.
///
/// - `Duel.LinkSummon` / `FusionSummon` / `RitualSummon` — the S7
///   legacy generic mapping renders a materials-consuming procedure as
///   a plain `special_summon … from extra_deck` (c89019964 Unchained
///   Soul of Disaster).
/// - `Duel.SpecialSummon[Step]` whose target-player arg (4th) is not
///   `tp` — summoning to the OPPONENT's field renders as `special_summon
///   self` (c39829561 Destiny HERO - Departed).
/// - `Duel.SendtoDeck` with `SEQ_DECKTOP` / `SEQ_DECKBOTTOM` — the
///   placement renders as a plain `send … to deck` (c33413638
///   Cockroach Knight).
/// - a group-building call (`SelectMatchingCard`, `GetMatchingGroup`,
///   `SelectTarget`, …) whose location args aren't single `LOCATION_*`
///   literals (or `0`) — compound `LOCATION_A|LOCATION_B` and variable
///   locations both render a ZONELESS selector, which the compiler
///   reads as on-field (c18895832 System Down: `MZONE|GRAVE` → `banish
///   (all, card, opponent controls)`; c34235530 Shi En: `DECK|GRAVE` →
///   `add_to_hand (1, card, you control)`; c40390147 Starliege Lord
///   Galaxion: `loc` from a label → `special_summon (1, card, you
///   control)`).
/// - the same calls with an exclusion arg other than `nil` / the
///   handler itself — a variable exclusion drops silently (c20073910
///   Amaterasu: `exc` → "banish all OTHER cards" loses `except self`).
pub fn lossy_call(method: &str, args: &[String]) -> Option<&'static str> {
    match method {
        "Duel.LinkSummon" | "Duel.FusionSummon" | "Duel.RitualSummon" => {
            return Some("legacy extra-deck summon mapping");
        }
        "Duel.SpecialSummon" | "Duel.SpecialSummonStep" => {
            if args.get(3).map(|a| a.trim() != "tp").unwrap_or(false) {
                return Some("special summon to a non-tp field");
            }
        }
        "Duel.SendtoDeck" => {
            if let Some(seq) = args.get(2) {
                if !matches!(seq.trim(), "nil" | "SEQ_DECKSHUFFLE" | "2") {
                    return Some("deck top/bottom placement");
                }
            }
        }
        _ => {}
    }
    // (first location arg index, exclusion arg index) per group builder.
    let shape = match method {
        "Duel.SelectMatchingCard" | "Duel.SelectTarget" => Some((3, 7)),
        "Duel.GetMatchingGroup" | "Duel.GetMatchingGroupCount"
        | "Duel.GetFirstMatchingCard" => Some((2, 4)),
        "Duel.GetFieldGroup" | "Duel.GetFieldGroupCount" => Some((1, usize::MAX)),
        _ => None,
    };
    if let Some((loc, ex)) = shape {
        for i in [loc, loc + 1] {
            if args.get(i).map(|a| !literal_location(a)).unwrap_or(false) {
                return Some("non-literal LOCATION argument");
            }
        }
        if let Some(e) = args.get(ex) {
            if !matches!(e.trim(), "nil" | "c" | "e:GetHandler()") {
                return Some("non-literal exclusion argument");
            }
        }
    }
    None
}

/// `0` or exactly one `LOCATION_*` identifier — no operators, no
/// variables.
fn literal_location(arg: &str) -> bool {
    let a = arg.trim();
    a == "0"
        || (a.strip_prefix("LOCATION_")
            .map(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
            .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn lossy_calls_refuse_the_hand_review_shapes() {
        assert!(lossy_call("Duel.LinkSummon", &a(&["tp", "sc", "nil", "mg", "2", "2"])).is_some());
        assert!(lossy_call("Duel.FusionSummon", &a(&["tp", "sc", "nil"])).is_some());
        assert!(lossy_call("Duel.RitualSummon", &a(&["tp", "sc", "nil"])).is_some());
        // opponent's field
        assert!(lossy_call(
            "Duel.SpecialSummon",
            &a(&["e:GetHandler()", "0", "tp", "1-tp", "false", "false", "POS_FACEUP_ATTACK"])
        ).is_some());
        assert_eq!(lossy_call(
            "Duel.SpecialSummon",
            &a(&["c", "0", "tp", "tp", "false", "false", "POS_FACEUP"])
        ), None);
        // deck placement
        assert!(lossy_call("Duel.SendtoDeck", &a(&["c", "nil", "SEQ_DECKTOP", "REASON_EFFECT"])).is_some());
        assert!(lossy_call("Duel.SendtoDeck", &a(&["c", "nil", "SEQ_DECKBOTTOM", "REASON_EFFECT"])).is_some());
        assert!(lossy_call("Duel.SendtoDeck", &a(&["c", "nil", "1", "REASON_EFFECT"])).is_some());
        assert_eq!(lossy_call("Duel.SendtoDeck", &a(&["c", "nil", "SEQ_DECKSHUFFLE", "REASON_EFFECT"])), None);
        assert_eq!(lossy_call("Duel.SendtoDeck", &a(&["g", "nil", "2", "REASON_EFFECT"])), None);
        // compound locations
        assert!(lossy_call(
            "Duel.SelectMatchingCard",
            &a(&["tp", "s.thfilter", "tp", "LOCATION_DECK|LOCATION_GRAVE", "0", "1", "1", "nil"])
        ).is_some());
        assert!(lossy_call(
            "Duel.GetMatchingGroup",
            &a(&["s.filter", "tp", "0", "LOCATION_MZONE|LOCATION_GRAVE", "nil"])
        ).is_some());
        assert!(lossy_call(
            "Duel.GetMatchingGroup",
            &a(&["s.filter", "tp", "LOCATION_MZONE+LOCATION_SZONE", "0", "nil"])
        ).is_some());
        assert_eq!(lossy_call(
            "Duel.SelectMatchingCard",
            &a(&["tp", "s.thfilter", "tp", "LOCATION_DECK", "0", "1", "1", "nil"])
        ), None);
        assert_eq!(lossy_call(
            "Duel.GetMatchingGroup",
            &a(&["aux.TRUE", "tp", "LOCATION_ONFIELD", "LOCATION_ONFIELD", "nil"])
        ), None);
        assert_eq!(lossy_call("Duel.Destroy", &a(&["g", "REASON_EFFECT"])), None);
        // variable location (label-dispatched zone) and variable exclusion
        assert!(lossy_call(
            "Duel.SelectMatchingCard",
            &a(&["tp", "s.spfilter", "tp", "loc", "0", "1", "1", "nil", "e", "tp"])
        ).is_some());
        assert!(lossy_call(
            "Duel.GetMatchingGroup",
            &a(&["Card.IsAbleToRemove", "tp", "LOCATION_ONFIELD", "LOCATION_ONFIELD", "exc"])
        ).is_some());
        assert_eq!(lossy_call(
            "Duel.GetMatchingGroup",
            &a(&["s.rmfilter", "tp", "LOCATION_MZONE", "LOCATION_MZONE", "e:GetHandler()"])
        ), None);
        assert_eq!(lossy_call(
            "Duel.SelectTarget",
            &a(&["tp", "nil", "tp", "LOCATION_MZONE", "LOCATION_MZONE", "1", "1", "c"])
        ), None);
        // a trailing extra arg after a nil exclusion is fine
        assert_eq!(lossy_call(
            "Duel.SelectMatchingCard",
            &a(&["tp", "s.filter", "tp", "LOCATION_DECK", "0", "1", "1", "nil", "e", "tp"])
        ), None);
    }

    // ── stub-line matcher ────────────────────────────────────

    #[test]
    fn stub_line_matches_every_generator_spelling() {
        let cases = [
            ("special_summon (all, card, either controls)", Category::SpecialSummon),
            ("add_to_hand (all, card, either controls)", Category::ToHand),
            ("destroy (all, card, either controls)", Category::Destroy),
            ("send (all, card, either controls) to gy", Category::ToGrave),
            ("return (all, card, either controls) to deck", Category::ToDeck),
            ("send (all, card, either controls) to extra_deck", Category::ToExtra),
            ("banish (all, card, either controls)", Category::Remove),
            ("return (all, card, either controls) to hand", Category::ToHand),
            ("take_control (all, card, either controls)", Category::Control),
            ("discard (all, card, either controls)", Category::Discard),
        ];
        for (line, cat) in cases {
            assert_eq!(stub_line_category(line), Some(cat), "{}", line);
            assert_eq!(stub_line_category(&format!("    {}\n", line)), Some(cat));
        }
    }

    #[test]
    fn stub_line_refuses_real_lines_and_near_misses() {
        for line in [
            "special_summon self",
            "special_summon (1, card, you control, from deck)",
            "destroy (all, card, opponent controls)",
            "send (all, card, either controls)",             // send needs a dest
            "send (all, card, either controls) to gy extra", // trailing tokens
            "send (all, card, either controls) to banished", // unknown dest
            "banish (all, card, either controls) face_down",
            "draw (all, card, either controls)",
            "return (all, card, either controls) to extra_deck", // not admitted
            "special_summon",
            "",
        ] {
            assert_eq!(stub_line_category(line), None, "{}", line);
        }
    }

    // ── lone-body rule ───────────────────────────────────────

    #[test]
    fn lone_body_is_exactly_one_stub_line() {
        assert_eq!(
            lone_stub_category("\n            destroy (all, card, either controls)\n        "),
            Some(Category::Destroy),
        );
        assert_eq!(lone_stub_category("\n        "), None, "empty resolve is Pass A's");
        assert_eq!(
            lone_stub_category(
                "\n            special_summon (all, card, either controls)\n            destroy (all, card, either controls)\n        "
            ),
            None,
            "multi-stub body is out of scope",
        );
        assert_eq!(
            lone_stub_category(
                "\n            special_summon (all, card, either controls)\n            draw 1\n        "
            ),
            None,
            "stub beside a real line is hand-written content",
        );
        assert_eq!(lone_stub_category("\n            draw 1\n        "), None);
    }

    // ── category function ────────────────────────────────────

    #[test]
    fn rendered_line_category_from_verb_and_destination() {
        assert_eq!(line_class("special_summon self"), LineClass::Category(Category::SpecialSummon));
        assert_eq!(
            line_class("special_summon (1, card, you control, from deck) in defense_position"),
            LineClass::Category(Category::SpecialSummon)
        );
        assert_eq!(line_class("add_to_hand self"), LineClass::Category(Category::ToHand));
        assert_eq!(line_class("destroy target"), LineClass::Category(Category::Destroy));
        assert_eq!(line_class("send self to gy"), LineClass::Category(Category::ToGrave));
        assert_eq!(
            line_class("send (all, monster, you control, from gy) to deck"),
            LineClass::Category(Category::ToDeck)
        );
        assert_eq!(line_class("send self to extra_deck"), LineClass::Category(Category::ToExtra));
        assert_eq!(line_class("banish target from gy"), LineClass::Category(Category::Remove));
        assert_eq!(line_class("take_control target until end_of_turn"), LineClass::Category(Category::Control));
        assert_eq!(line_class("discard (1, card, you control, from hand)"), LineClass::Category(Category::Discard));
        // destination = token after the LAST ` to `
        assert_eq!(line_class("return self to deck shuffle"), LineClass::Category(Category::ToDeck));
        assert_eq!(line_class("return target to hand"), LineClass::Category(Category::ToHand));
        // unmapped destinations / verbs
        assert_eq!(line_class("return self to extra_deck"), LineClass::Unknown);
        assert_eq!(line_class("return target to owner"), LineClass::Unknown);
        assert_eq!(line_class("send self to banished"), LineClass::Unknown);
        assert_eq!(line_class("fusion_summon (2, monster, you control)"), LineClass::Unknown);
        assert_eq!(line_class("grant self cannot_attack until end_of_turn"), LineClass::Unknown);
        assert_eq!(line_class("install_watcher \"x\" { event: end_phase }"), LineClass::Unknown);
        assert_eq!(line_class(""), LineClass::Unknown);
    }

    #[test]
    fn riders_classify_with_their_keywords() {
        assert_eq!(
            line_class("restrict you cannot_special_summon except (race == Zombie) end_of_turn"),
            LineClass::Rider(&["cannot"])
        );
        assert_eq!(line_class("change_position self to defense_position"), LineClass::Rider(&["position"]));
        assert_eq!(line_class("modify_atk self + 500 until while_face_up"), LineClass::Rider(&["atk"]));
        assert_eq!(line_class("set (1, card, you control, from deck)"), LineClass::Rider(&["set"]));
    }

    // ── synonym table ────────────────────────────────────────

    #[test]
    fn synonym_table_joins_generator_and_translator_spellings() {
        // return-to-hand ≈ add_to_hand (CATEGORY_TOHAND)
        let stub = stub_line_category("return (all, card, either controls) to hand").unwrap();
        assert_eq!(line_class("add_to_hand self"), LineClass::Category(stub));
        assert_eq!(line_class("add_to_hand (1, card, you control, from deck)"), LineClass::Category(stub));
        // return-to-deck ≈ send-to-deck (CATEGORY_TODECK)
        let stub = stub_line_category("return (all, card, either controls) to deck").unwrap();
        assert_eq!(line_class("send self to deck"), LineClass::Category(stub));
        // the other direction is the same category too
        assert_eq!(
            stub_line_category("add_to_hand (all, card, either controls)"),
            stub_line_category("return (all, card, either controls) to hand")
        );
        // no synonym crosses a category boundary
        assert_ne!(
            stub_line_category("send (all, card, either controls) to gy"),
            stub_line_category("return (all, card, either controls) to deck")
        );
        assert_ne!(
            stub_line_category("send (all, card, either controls) to extra_deck"),
            stub_line_category("return (all, card, either controls) to deck")
        );
        // every table spelling is self-consistent with line_class
        for (key, cat) in SPELLINGS {
            let sample = if let Some((verb, dest)) = key.split_once(" to ") {
                format!("{} self to {}", verb, dest)
            } else {
                format!("{} self", key)
            };
            assert_eq!(line_class(&sample), LineClass::Category(*cat), "{}", key);
        }
    }

    // ── category match ───────────────────────────────────────

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn match_requires_the_stub_category_at_least_once() {
        let desc = Some("Special Summon 1 monster from your hand. Destroy 1 card.");
        assert!(category_match(Category::SpecialSummon, &s(&["special_summon self"]), None));
        assert!(category_match(Category::ToHand, &s(&["add_to_hand (1, card, you control, from deck)"]), None));
        assert!(category_match(Category::ToDeck, &s(&["send self to deck"]), None));
        // positional residue: the owner renders a different category
        assert!(!category_match(Category::SpecialSummon, &s(&["add_to_hand self"]), desc));
        assert!(!category_match(Category::Destroy, &s(&["special_summon self"]), desc));
        assert!(!category_match(Category::SpecialSummon, &[], desc));
    }

    #[test]
    fn foreign_category_and_rider_lines_need_card_text() {
        let both = &s(&["special_summon self", "destroy (1, card, either controls, from field)"]);
        assert!(category_match(
            Category::SpecialSummon, both,
            Some("Special Summon this card, then destroy 1 card on the field.")
        ));
        assert!(!category_match(Category::SpecialSummon, both, Some("Special Summon this card.")));
        assert!(!category_match(Category::SpecialSummon, both, None), "no cdb → no riders");

        let rider = &s(&[
            "special_summon self",
            "restrict you cannot_special_summon except (race == Zombie) end_of_turn",
        ]);
        assert!(category_match(
            Category::SpecialSummon, rider,
            Some("Special Summon this card. You cannot Special Summon monsters, except Zombie monsters.")
        ));
        assert!(!category_match(Category::SpecialSummon, rider, Some("Special Summon this card.")));
        // case-insensitive text match
        assert!(category_match(
            Category::ToHand,
            &s(&["add_to_hand self", "change_position self to defense_position"]),
            Some("ADD IT TO YOUR HAND AND CHANGE ITS BATTLE POSITION")
        ));
    }

    #[test]
    fn unadmitted_verbs_refuse_even_with_text() {
        let desc = Some("Special Summon it. It cannot attack this turn. Destroy it during the End Phase.");
        assert!(!category_match(
            Category::SpecialSummon,
            &s(&["special_summon self", "grant self cannot_attack until end_of_turn"]),
            desc
        ));
        assert!(!category_match(
            Category::SpecialSummon,
            &s(&["special_summon self as sp", "delayed on end_phase until end_of_turn {\n    destroy sp\n}"]),
            desc
        ));
        assert!(!category_match(
            Category::SpecialSummon,
            &s(&["fusion_summon (2, monster, you control)"]),
            desc
        ));
    }
}
