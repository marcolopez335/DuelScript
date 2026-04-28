// ============================================================
// translate_corpus — M-phase pattern-based translator
//
// Rewrites empty-stub / placeholder-resolve effect blocks in the
// card corpus using BabelCdb card-text as the source of truth.
//
// Each cluster is a struct implementing `Cluster`:
//   - name:        stable cluster identifier
//   - matches:     does this .ds file + desc belong to the cluster?
//   - rewrite:     produce the new .ds source text
//
// Invocation:
//   cargo run --features cdb --bin translate_corpus -- \
//     [--dry-run|--apply]                     \
//     [--cluster <name>]                      \
//     [--limit <N>]                           \
//     <corpus_dir>                            \
//     <cards.cdb>
//
// Example:
//   cargo run --features cdb --bin translate_corpus -- \
//     --dry-run                                        \
//     --cluster recruiter_battle_damage                \
//     /Users/marco/git/duelscript/cards/official       \
//     /Users/marco/git/BabelCdb/cards.cdb
//
// Safety: in --apply mode, each rewritten file is re-parsed via
// parse_v2 before being written. Any parse failure aborts that
// single file (remaining files are unaffected).
// ============================================================
//
// This binary only builds with the `cdb` feature. Without it, the
// main function is replaced with a one-liner error.
// ============================================================

#[cfg(not(feature = "cdb"))]
fn main() {
    eprintln!(
        "translate_corpus requires the `cdb` feature.\n\
         Rebuild with:  cargo run --features cdb --bin translate_corpus -- ..."
    );
    std::process::exit(2);
}

#[cfg(feature = "cdb")]
fn main() {
    cdb_mode::run()
}

// ── Real implementation ─────────────────────────────────────

#[cfg(feature = "cdb")]
mod cdb_mode {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process,
    };

    use duelscript::cdb::{CdbCard, CdbReader};
    use duelscript::parse_v2;

    // ── CLI ─────────────────────────────────────────────────

    struct Args {
        apply:        bool,
        cluster:      Option<String>,
        limit:        Option<usize>,
        corpus_dir:   PathBuf,
        cdb_path:     PathBuf,
    }

    fn parse_args() -> Args {
        let raw: Vec<String> = env::args().skip(1).collect();
        let mut apply = false;
        let mut cluster: Option<String> = None;
        let mut limit:   Option<usize>  = None;
        let mut positional: Vec<String> = Vec::new();

        let mut i = 0;
        while i < raw.len() {
            match raw[i].as_str() {
                "--apply"    => { apply = true; i += 1; }
                "--dry-run"  => { apply = false; i += 1; }
                "--cluster"  => {
                    if i + 1 >= raw.len() {
                        eprintln!("--cluster needs a value"); process::exit(2);
                    }
                    cluster = Some(raw[i + 1].clone());
                    i += 2;
                }
                "--limit"    => {
                    if i + 1 >= raw.len() {
                        eprintln!("--limit needs a value"); process::exit(2);
                    }
                    limit = Some(raw[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("--limit must be a number"); process::exit(2);
                    }));
                    i += 2;
                }
                "-h" | "--help" => {
                    print_help();
                    process::exit(0);
                }
                _ => {
                    positional.push(raw[i].clone());
                    i += 1;
                }
            }
        }

        if positional.len() != 2 {
            eprintln!("usage: translate_corpus [--dry-run|--apply] [--cluster <name>] [--limit N] <corpus_dir> <cards.cdb>");
            process::exit(2);
        }

        Args {
            apply,
            cluster,
            limit,
            corpus_dir: PathBuf::from(&positional[0]),
            cdb_path:   PathBuf::from(&positional[1]),
        }
    }

    fn print_help() {
        println!("{}",
"translate_corpus — M-phase pattern-based .ds translator

USAGE:
    translate_corpus [OPTIONS] <corpus_dir> <cards.cdb>

OPTIONS:
    --dry-run             (default) report stubs hit + per-file diffs
    --apply               rewrite files in place (parse-checked first)
    --cluster <name>      run only the named cluster
    --limit <N>           stop after N file hits (across all clusters)
    -h, --help            show this help

CLUSTERS:
    recruiter_battle_damage
        trigger: battle_damage + placeholder special_summon body;
        matches descs like 'Special Summon 1 FIRE monster with
        1500 or less ATK from your Deck'.
    recruiter_battle_damage_archetype
        same trigger shape; archetype-locked variant:
        'Special Summon 1 \"Blackwing\" monster with 1500 or less
        ATK from your Deck'.
    recruiter_battle_damage_archetype_nostat
        same trigger shape; archetype-locked without stat cap:
        'Special Summon 1 \"Melodious\" monster from your Deck'.
    recruiter_battle_damage_named
        same trigger shape; single named monster (self or other):
        'Special Summon 1 \"Hydrogeddon\" from your Deck'.
    recruiter_destroyed_archetype_nostat
        trigger: destroyed + placeholder special_summon body;
        archetype-locked without stat cap, broader trigger-anchor
        (accepts 'is destroyed by battle or card effect' etc.);
        descs like 'Special Summon 1 \"Lunalight\" monster from
        your Deck'.
    search_battle_damage_archetype_monster
        trigger: battle_damage + placeholder add_to_hand body;
        archetype-locked monster search:
        'add 1 \"Beetrooper\" monster from your Deck to your hand'.
    search_battle_damage_archetype_card
        trigger: battle_damage + placeholder add_to_hand body;
        archetype-locked card (not monster-restricted) search:
        'add 1 \"Archfiend\" card from your Deck to your hand'.
    search_destroyed_archetype_monster
        trigger: destroyed + placeholder add_to_hand body;
        archetype-locked monster search variant.
    search_destroyed_archetype_card
        trigger: destroyed + placeholder add_to_hand body;
        archetype-locked card (spans S/T) variant.
    search_sent_to_gy_archetype_monster
        trigger: sent_to gy + placeholder add_to_hand body;
        archetype-locked monster search with sent-to-GY /
        tributed anchor in desc text.
    search_sent_to_gy_archetype_card
        trigger: sent_to gy + placeholder add_to_hand body;
        archetype-locked card (spans S/T) variant.
    search_sent_to_gy_subtype_archetype_monster
        M.7 / MMM-II: subtype-adjective variant of the archetype
        monster search. Matches descs like:
        'Add 1 Warrior \"Nekroz\" Ritual Monster from your Deck'.
        Emits: `where archetype == X and race == Warrior and is_ritual`.
    search_archetype_spell_trap
        M.5-ext / OOO-II: Spell/Trap type-predicate variant of the
        archetype search family. Trigger-agnostic: fires on any
        supported trigger block that contains the canonical
        add_to_hand placeholder AND whose desc phrases the search
        as `Add 1 \"<Arch>\" Spell/Trap from your Deck`.
        Emits: `where archetype == X and not is_monster` (the
        grammar-supported equivalent — see parser.rs comment).
    search_archetype_monster_any_trigger
        M.8 / TTT-II: trigger-agnostic variant of the archetype-
        monster search family. Desc shape
        `Add 1 \"<Arch>\" monster from your Deck`. Registered
        AFTER the anchor-gated per-trigger clusters; sweeps
        residual summoned / summoned-by-special / standby_phase /
        leaves_field / destroyed_by_battle blocks.
        Emits: `where archetype == X`.
    search_archetype_card_any_trigger
        M.8 / TTT-II: same as above for the `card` target shape
        `Add 1 \"<Arch>\" card from your Deck`.
        Emits: `where archetype == X`.
    destroy_all_opponent_monsters_any_trigger
        P3 prototype: first non-search/non-recruiter cluster. Trigger-
        agnostic destroy variant. Matches canonical phrase
        \"Destroy all monsters your opponent controls\" + placeholder
        body `destroy (all, card, either controls)`.
        Emits: `destroy (all, monster, opponent controls)`.
");
    }

    pub fn run() {
        let args = parse_args();

        eprintln!(
            "translate_corpus: {} mode, corpus={}, cdb={}",
            if args.apply { "APPLY" } else { "dry-run" },
            args.corpus_dir.display(),
            args.cdb_path.display(),
        );

        // ── Load BabelCdb ───────────────────────────────────
        let cdb = match CdbReader::open(&args.cdb_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("failed to open cards.cdb: {e:?}");
                process::exit(1);
            }
        };
        eprintln!("loaded {} CDB rows", cdb.len());

        // ── Walk corpus ─────────────────────────────────────
        let files = collect_ds_files(&args.corpus_dir);
        eprintln!("scanning {} .ds files", files.len());

        let clusters: Vec<Box<dyn Cluster>> = build_clusters(&args.cluster);
        if clusters.is_empty() {
            eprintln!("no clusters matched --cluster filter");
            process::exit(2);
        }
        for c in &clusters {
            eprintln!("  cluster active: {}", c.name());
        }

        let mut report = Report::default();

        'outer: for path in &files {
            let src = match fs::read_to_string(path) {
                Ok(s)  => s,
                Err(_) => continue,
            };

            // Find the passcode id in the .ds (first `id: N`).
            let id = match extract_id(&src) {
                Some(id) => id,
                None     => continue,
            };
            let cdb_row = match cdb.get(id) {
                Some(r) => r,
                None    => continue,
            };

            for cluster in &clusters {
                if !cluster.matches(&src, cdb_row) {
                    continue;
                }
                report.matched_by_cluster(cluster.name());

                // Perform rewrite.
                let new_src = match cluster.rewrite(&src, cdb_row) {
                    Ok(s)  => s,
                    Err(e) => {
                        eprintln!("[SKIP] {}: rewrite failed: {}", path.display(), e);
                        report.skip(cluster.name(), "rewrite_failed");
                        continue;
                    }
                };

                if new_src == src {
                    // Cluster said yes but produced no change (shouldn't happen, but handle).
                    report.skip(cluster.name(), "no_change");
                    continue;
                }

                // Safety: parse-check the candidate.
                if let Err(e) = parse_v2(&new_src) {
                    eprintln!("[SKIP] {}: post-rewrite parse failed: {}", path.display(), e);
                    report.skip(cluster.name(), "parse_failed");
                    continue;
                }

                if args.apply {
                    match fs::write(path, &new_src) {
                        Ok(_) => {
                            report.apply(cluster.name());
                        }
                        Err(e) => {
                            eprintln!("[SKIP] {}: write failed: {}", path.display(), e);
                            report.skip(cluster.name(), "write_failed");
                            continue;
                        }
                    }
                } else {
                    report.dry(cluster.name());
                    print_diff(path, &src, &new_src);
                }

                // One cluster-rewrite per file; don't double-match.
                if let Some(l) = args.limit {
                    if report.total_hits() >= l {
                        eprintln!("reached --limit {l}");
                        break 'outer;
                    }
                }
                break;
            }
        }

        report.print();
    }

    // ── Shared helpers ──────────────────────────────────────

    fn collect_ds_files(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = Vec::new();
        let Ok(rd) = fs::read_dir(dir) else {
            eprintln!("could not read corpus dir: {}", dir.display());
            return files;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("ds") {
                files.push(p);
            }
        }
        files.sort();
        files
    }

    fn extract_id(src: &str) -> Option<u64> {
        for line in src.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("id:") {
                return rest.trim().parse().ok();
            }
        }
        None
    }

    fn print_diff(path: &Path, old: &str, new: &str) {
        println!("\n--- {} ---", path.display());
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();
        // Print only the differing region (cheap line-by-line diff).
        for (i, (a, b)) in old_lines.iter().zip(new_lines.iter()).enumerate() {
            if a != b {
                println!("  line {:3}: - {}", i + 1, a);
                println!("  line {:3}: + {}", i + 1, b);
            }
        }
        if old_lines.len() != new_lines.len() {
            println!("  (line count changed: {} -> {})",
                old_lines.len(), new_lines.len());
        }
    }

    // ── Report ──────────────────────────────────────────────

    #[derive(Default)]
    struct Report {
        matched:     std::collections::BTreeMap<String, usize>,
        applied:     std::collections::BTreeMap<String, usize>,
        dry:         std::collections::BTreeMap<String, usize>,
        skipped:     std::collections::BTreeMap<(String, String), usize>,
    }

    impl Report {
        fn matched_by_cluster(&mut self, c: &str) { *self.matched.entry(c.to_string()).or_default() += 1; }
        fn apply(&mut self, c: &str)              { *self.applied.entry(c.to_string()).or_default() += 1; }
        fn dry(&mut self, c: &str)                { *self.dry.entry(c.to_string()).or_default()     += 1; }
        fn skip(&mut self, c: &str, reason: &str) {
            *self.skipped.entry((c.to_string(), reason.to_string())).or_default() += 1;
        }
        fn total_hits(&self) -> usize {
            self.applied.values().sum::<usize>() + self.dry.values().sum::<usize>()
        }

        fn print(&self) {
            println!();
            println!("=== translate_corpus report ===");
            for (c, n) in &self.matched {
                println!("  matched[{c}]: {n}");
            }
            for (c, n) in &self.applied {
                println!("  applied[{c}]: {n}");
            }
            for (c, n) in &self.dry {
                println!("  dry[{c}]:     {n}");
            }
            for ((c, reason), n) in &self.skipped {
                println!("  skipped[{c}/{reason}]: {n}");
            }
        }
    }

    // ── Cluster trait ───────────────────────────────────────

    trait Cluster {
        fn name(&self) -> &'static str;
        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool;
        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String>;
    }

    fn build_clusters(filter: &Option<String>) -> Vec<Box<dyn Cluster>> {
        // Order matters: M.0 recruiter_battle_damage first; M.1
        // sub-clusters (archetype stat/nostat/named); M.2 destroyed-
        // trigger; M.3 search-placeholder clusters last. One rewrite
        // per file (see `break` in `run`), so an earlier cluster wins
        // any overlap. The monster-search variant runs before the card-
        // search variant so "X" monster" desc shapes get the tighter
        // rewrite.
        let all: Vec<Box<dyn Cluster>> = vec![
            Box::new(RecruiterBattleDamage),
            Box::new(RecruiterBattleDamageArchetype),
            Box::new(RecruiterBattleDamageArchetypeNoStat),
            Box::new(RecruiterBattleDamageNamed),
            Box::new(RecruiterDestroyedArchetypeNoStat),
            Box::new(SearchBattleDamageArchetypeMonster),
            Box::new(SearchBattleDamageArchetypeCard),
            Box::new(SearchDestroyedArchetypeMonster),
            Box::new(SearchDestroyedArchetypeCard),
            Box::new(SearchSentToGyArchetypeMonster),
            Box::new(SearchSentToGyArchetypeCard),
            Box::new(SearchSentToGySubtypeArchetypeMonster),
            Box::new(SearchArchetypeSpellTrap),
            Box::new(SearchArchetypeMonsterAnyTrigger),
            Box::new(SearchArchetypeCardAnyTrigger),
            Box::new(DestroyAllOpponentMonstersAnyTrigger),
            Box::new(SendAllOpponentMonstersToGyAnyTrigger),
            Box::new(BanishAllOpponentMonstersAnyTrigger),
            Box::new(ReturnAllOpponentMonstersToHandAnyTrigger),
        ];
        match filter {
            None        => all,
            Some(name)  => all.into_iter().filter(|c| c.name() == name).collect(),
        }
    }

    // ── Cluster: recruiter_battle_damage ────────────────────
    //
    // Shape to hit:
    //   effect "X" {
    //       speed: 1
    //       [mandatory | timing: ... | once_per_turn: ...]  (any)
    //       trigger: battle_damage
    //       resolve {
    //           special_summon (all, card, either controls)
    //       }
    //   }
    //
    // BabelCdb desc regex (case-insensitive):
    //   Special Summon 1 <ATTRIBUTE> monster with <N> or less ATK
    //   Special Summon 1 <RACE> monster with <N> or less DEF
    //
    // Rewrite: replace the placeholder resolve line with
    //   special_summon (1, monster, where attribute == X and atk <= N) from deck in attack_position
    // or the DEF variant / race variant as appropriate.

    struct RecruiterBattleDamage;

    const ATTRIBUTES: &[&str] = &["LIGHT", "DARK", "FIRE", "WATER", "EARTH", "WIND", "DIVINE"];

    // DuelScript race tokens that appear in BabelCdb desc text.
    const RACES: &[&str] = &[
        "Warrior", "Spellcaster", "Fairy", "Fiend", "Zombie",
        "Machine", "Aqua", "Pyro", "Rock", "Winged Beast",
        "Plant", "Insect", "Thunder", "Dragon", "Beast",
        "Beast-Warrior", "Dinosaur", "Fish", "Sea Serpent", "Reptile",
        "Psychic", "Divine-Beast", "Creator God", "Wyrm", "Cyberse",
        "Illusion",
    ];

    fn ds_race_token(human: &str) -> &'static str {
        // DuelScript `race` tokens are the same words with spaces preserved
        // and hyphens (e.g. "Winged Beast", "Beast-Warrior", "Sea Serpent").
        // For our `race == X` predicate we want the exact DuelScript token.
        // Inspection of grammar/duelscript.pest shows `race` is parsed as one
        // of these literals. The map is identity for most, explicit for
        // compound words.
        match human {
            "Warrior"       => "Warrior",
            "Spellcaster"   => "Spellcaster",
            "Fairy"         => "Fairy",
            "Fiend"         => "Fiend",
            "Zombie"        => "Zombie",
            "Machine"       => "Machine",
            "Aqua"          => "Aqua",
            "Pyro"          => "Pyro",
            "Rock"          => "Rock",
            "Winged Beast"  => "Winged-Beast",
            "Plant"         => "Plant",
            "Insect"        => "Insect",
            "Thunder"       => "Thunder",
            "Dragon"        => "Dragon",
            "Beast"         => "Beast",
            "Beast-Warrior" => "Beast-Warrior",
            "Dinosaur"      => "Dinosaur",
            "Fish"          => "Fish",
            "Sea Serpent"   => "Sea-Serpent",
            "Reptile"       => "Reptile",
            "Psychic"       => "Psychic",
            "Divine-Beast"  => "Divine-Beast",
            "Creator God"   => "Creator-God",
            "Wyrm"          => "Wyrm",
            "Cyberse"       => "Cyberse",
            "Illusion"      => "Illusion",
            _               => "",
        }
    }

    /// Match the canonical classic-recruiter desc.
    /// Returns (filter_expr, position_clause).
    fn match_recruit_desc(desc: &str) -> Option<(String, &'static str)> {
        // Normalise whitespace.
        let d = desc.replace('\n', " ").replace('\r', " ");

        // ATK variant.
        //   Special Summon 1 <ATTR> monster with <N> or less ATK from your Deck
        for attr in ATTRIBUTES {
            let needle_a = format!("Special Summon 1 {attr} monster with ");
            if let Some(start) = d.find(&needle_a) {
                let rest = &d[start + needle_a.len()..];
                if let Some(stop) = rest.find(" or less ATK") {
                    let num_s = rest[..stop].trim();
                    if let Ok(n) = num_s.parse::<u32>() {
                        if d[start..].contains("from your Deck") {
                            let position = pick_position(&d[start..]);
                            let expr = format!("attribute == {attr} and atk <= {n}");
                            return Some((expr, position));
                        }
                    }
                }
            }
        }

        // DEF variant.
        //   Special Summon 1 <ATTR> monster with <N> or less DEF from your Deck
        for attr in ATTRIBUTES {
            let needle_d = format!("Special Summon 1 {attr} monster with ");
            if let Some(start) = d.find(&needle_d) {
                let rest = &d[start + needle_d.len()..];
                if let Some(stop) = rest.find(" or less DEF") {
                    let num_s = rest[..stop].trim();
                    if let Ok(n) = num_s.parse::<u32>() {
                        if d[start..].contains("from your Deck") {
                            let position = pick_position(&d[start..]);
                            let expr = format!("attribute == {attr} and def <= {n}");
                            return Some((expr, position));
                        }
                    }
                }
            }
        }

        // Race / DEF variant (Pyramid Turtle pattern).
        //   Special Summon 1 <RACE> monster with <N> or less DEF from your Deck
        for race in RACES {
            let tok = ds_race_token(race);
            if tok.is_empty() { continue; }
            let needle = format!("Special Summon 1 {race} monster with ");
            if let Some(start) = d.find(&needle) {
                let rest = &d[start + needle.len()..];
                if let Some(stop) = rest.find(" or less DEF") {
                    let num_s = rest[..stop].trim();
                    if let Ok(n) = num_s.parse::<u32>() {
                        if d[start..].contains("from your Deck") {
                            let position = pick_position(&d[start..]);
                            let expr = format!("race == {tok} and def <= {n}");
                            return Some((expr, position));
                        }
                    }
                }
            }
            // Also the ATK / RACE variant (Flamvell Firedog etc.).
            let needle_r = format!("Special Summon 1 {race} monster with ");
            if let Some(start) = d.find(&needle_r) {
                let rest = &d[start + needle_r.len()..];
                if let Some(stop) = rest.find(" or less ATK") {
                    let num_s = rest[..stop].trim();
                    if let Ok(n) = num_s.parse::<u32>() {
                        if d[start..].contains("from your Deck") {
                            let position = pick_position(&d[start..]);
                            let expr = format!("race == {tok} and atk <= {n}");
                            return Some((expr, position));
                        }
                    }
                }
            }
        }

        None
    }

    fn pick_position(segment: &str) -> &'static str {
        // Default: attack_position. The canonical recruiter texts all
        // summon in Attack Position. If the desc explicitly says
        // "face-down Defense Position" we'd fall back to defense_position;
        // no known classic recruiter does this.
        if segment.contains("Defense Position") && !segment.contains("face-up Attack Position") {
            "defense_position"
        } else {
            "attack_position"
        }
    }

    impl Cluster for RecruiterBattleDamage {
        fn name(&self) -> &'static str { "recruiter_battle_damage" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            // Must contain the exact placeholder effect block pattern.
            if !has_battle_damage_placeholder(src) { return false; }

            // BabelCdb desc must match a canonical recruiter shape.
            match_recruit_desc(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, position) = match_recruit_desc(&cdb_row.desc)
                .ok_or_else(|| "desc no longer matches".to_string())?;

            let new_line = format!(
                "            special_summon (1, monster, where {filter_expr}) from deck in {position}"
            );
            let old_line = "            special_summon (all, card, either controls)";

            let range = find_placeholder_body_range(
                src, "battle_damage",
                "special_summon (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Shared helpers for M.1 sub-clusters ─────────────────────
    //
    // `trigger_anchor_before` checks that one of the canonical
    // battle-trigger phrases appears in the desc before `cursor`.
    // This guards against cards like Tanngnjostr where a placeholder
    // battle_damage effect in the .ds file is unrelated to the
    // "Special Summon 1 \"X\" ..." sentence elsewhere in the desc.

    fn trigger_anchor_before(desc: &str, cursor: usize) -> bool {
        // The canonical battle triggers for classic-recruiter-shaped
        // effects. Match any of these occurring before `cursor`.
        //
        // We check for a small family of phrasings — "this card is
        // destroyed by battle", "this card destroys ... by battle",
        // "this face-up Attack Position card ... destroyed by battle",
        // "this card you control is destroyed by battle", etc.
        // All variants share "this" + "card" + "by battle".
        //
        // Kept deliberately conservative: we scan the prefix for one
        // of the substrings "card is destroyed by battle",
        // "card you control is destroyed by battle",
        // "card destroys an opponent" (+"by battle"),
        // "card destroys a monster" (+"by battle"),
        // "card, when destroyed by battle".
        let prefix = &desc[..cursor];
        let lower = prefix.to_lowercase();
        // Common destroyed-by-battle phrasings.
        if lower.contains("card is destroyed by battle") { return true; }
        if lower.contains("card you control is destroyed by battle") { return true; }
        if lower.contains("card, when destroyed by battle") { return true; }
        // Destroys-by-battle phrasings.
        if lower.contains("card destroys an opponent") && lower.contains("by battle") { return true; }
        if lower.contains("card destroys a monster") && lower.contains("by battle") { return true; }
        // "this face-up Attack Position card you control is destroyed by battle"
        if lower.contains("attack position card you control is destroyed by battle") { return true; }
        false
    }

    // ── Cluster: recruiter_battle_damage_archetype ──────────────
    //
    // Shape to hit:
    //   same effect-block shape as M.0.
    //   desc: 'Special Summon 1 "<Archetype>" monster with <N>
    //          or less (ATK|DEF) from your Deck'
    //
    // Rewrite:
    //   special_summon (1, monster, where archetype == "<Archetype>"
    //     and (atk|def) <= <N>) from deck in <position>

    struct RecruiterBattleDamageArchetype;

    /// Find the canonical archetype+stat recruiter sentence.
    /// Returns (filter_expr, position, sentence_start).
    fn match_archetype_stat_desc(desc: &str) -> Option<(String, &'static str, usize)> {
        let needle_head = "Special Summon 1 \"";
        let mut cursor = 0;
        while let Some(off) = desc[cursor..].find(needle_head) {
            let start = cursor + off;
            let after_head = start + needle_head.len();
            // Extract the quoted archetype.
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            // Expect " monster with <N> or less (ATK|DEF) from your Deck"
            let after_quote = tail;
            let with_head = " monster with ";
            if let Some(ws) = after_quote.find(with_head) {
                let after_with = &after_quote[ws + with_head.len()..];
                // Extract number up to space.
                let num_end = after_with.find(' ').unwrap_or(after_with.len());
                let num_s = &after_with[..num_end];
                if let Ok(n) = num_s.parse::<u32>() {
                    let rest2 = &after_with[num_end..];
                    // " or less ATK from your Deck" or DEF variant.
                    let stat = if rest2.starts_with(" or less ATK from your Deck") {
                        Some("atk")
                    } else if rest2.starts_with(" or less DEF from your Deck") {
                        Some("def")
                    } else {
                        None
                    };
                    if let Some(st) = stat {
                        // Trigger anchor guard.
                        if trigger_anchor_before(desc, start) {
                            // Position: look at the sentence after "from your Deck".
                            let segment = &desc[start..];
                            let position = pick_position(segment);
                            let expr = format!("archetype == \"{arch}\" and {st} <= {n}");
                            return Some((expr, position, start));
                        }
                    }
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for RecruiterBattleDamageArchetype {
        fn name(&self) -> &'static str { "recruiter_battle_damage_archetype" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_battle_damage_placeholder(src) { return false; }
            match_archetype_stat_desc(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, position, _) = match_archetype_stat_desc(&cdb_row.desc)
                .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            special_summon (1, monster, where {filter_expr}) from deck in {position}"
            );
            let old_line = "            special_summon (all, card, either controls)";
            let range = find_placeholder_body_range(
                src, "battle_damage",
                "special_summon (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Cluster: recruiter_battle_damage_archetype_nostat ───────
    //
    // Shape to hit:
    //   same effect-block shape.
    //   desc: 'Special Summon 1 "<Archetype>" monster from your Deck'
    //   (no stat cap)
    //
    // Rewrite:
    //   special_summon (1, monster, where archetype == "<Archetype>")
    //     from deck in <position>

    struct RecruiterBattleDamageArchetypeNoStat;

    fn match_archetype_nostat_desc(desc: &str) -> Option<(String, &'static str, usize)> {
        let needle_head = "Special Summon 1 \"";
        let mut cursor = 0;
        while let Some(off) = desc[cursor..].find(needle_head) {
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            // Expect " monster from your Deck" literally (no "with <N>").
            if tail.starts_with(" monster from your Deck") {
                if trigger_anchor_before(desc, start) {
                    let segment = &desc[start..];
                    let position = pick_position(segment);
                    let expr = format!("archetype == \"{arch}\"");
                    return Some((expr, position, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for RecruiterBattleDamageArchetypeNoStat {
        fn name(&self) -> &'static str { "recruiter_battle_damage_archetype_nostat" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_battle_damage_placeholder(src) { return false; }
            match_archetype_nostat_desc(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, position, _) = match_archetype_nostat_desc(&cdb_row.desc)
                .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            special_summon (1, monster, where {filter_expr}) from deck in {position}"
            );
            let old_line = "            special_summon (all, card, either controls)";
            let range = find_placeholder_body_range(
                src, "battle_damage",
                "special_summon (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Cluster: recruiter_battle_damage_named ──────────────────
    //
    // Shape to hit:
    //   same effect-block shape.
    //   desc: 'Special Summon 1 "<Name>" from your Deck'
    //   (no "monster" token between the quote and "from")
    //
    // Rewrite:
    //   special_summon (1, monster, where name == "<Name>")
    //     from deck in <position>

    struct RecruiterBattleDamageNamed;

    fn match_named_desc(desc: &str) -> Option<(String, &'static str, usize)> {
        let needle_head = "Special Summon 1 \"";
        let mut cursor = 0;
        while let Some(off) = desc[cursor..].find(needle_head) {
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let name = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            // Expect " from your Deck" literally (no "monster " in between).
            if tail.starts_with(" from your Deck") {
                if trigger_anchor_before(desc, start) {
                    let segment = &desc[start..];
                    let position = pick_position(segment);
                    let expr = format!("name == \"{name}\"");
                    return Some((expr, position, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for RecruiterBattleDamageNamed {
        fn name(&self) -> &'static str { "recruiter_battle_damage_named" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_battle_damage_placeholder(src) { return false; }
            match_named_desc(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, position, _) = match_named_desc(&cdb_row.desc)
                .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            special_summon (1, monster, where {filter_expr}) from deck in {position}"
            );
            let old_line = "            special_summon (all, card, either controls)";
            let range = find_placeholder_body_range(
                src, "battle_damage",
                "special_summon (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Cluster: recruiter_destroyed_archetype_nostat (M.2) ──────
    //
    // Shape to hit:
    //   effect "X" {
    //       speed: 1
    //       [mandatory | timing: ... | once_per_turn: ...]  (any)
    //       trigger: destroyed
    //       resolve {
    //           special_summon (all, card, either controls)
    //       }
    //   }
    //
    //   desc: 'Special Summon 1 "<Archetype>" monster from your Deck'
    //   (no stat cap)
    //
    //   Anchor (broader than M.1's trigger_anchor_before): the sentence
    //   must be preceded in the desc by a destroyed-trigger clause such
    //   as "is destroyed by battle", "is destroyed by battle or card
    //   effect", "is destroyed by card effect", or "is destroyed by an
    //   opponent's card".
    //
    // Rewrite: identical to M.1's archetype_nostat cluster —
    //   special_summon (1, monster, where archetype == "<Archetype>")
    //     from deck in <position>

    struct RecruiterDestroyedArchetypeNoStat;

    fn destroyed_trigger_anchor_before(desc: &str, cursor: usize) -> bool {
        let prefix = &desc[..cursor];
        let lower  = prefix.to_lowercase();
        // Canonical destroyed-trigger phrasings. Accept any occurrence of
        // "is destroyed" followed by one of the destroy-source phrases
        // within a short radius. We do this by checking for a few
        // pre-composed substrings — robust enough without parsing.
        if lower.contains("is destroyed by battle") { return true; }
        if lower.contains("is destroyed by card effect") { return true; }
        if lower.contains("is destroyed by an opponent's card") { return true; }
        // Common variant without explicit source: "this card is destroyed:"
        // (matches "this card is destroyed and sent", "this card, when
        // destroyed").
        if lower.contains("this card is destroyed") { return true; }
        if lower.contains("card you control is destroyed") { return true; }
        if lower.contains("card on the field is destroyed") { return true; }
        if lower.contains("card in its owner's possession is destroyed") { return true; }
        false
    }

    /// Same regex as M.1's `match_archetype_nostat_desc`, but uses the
    /// broader destroyed-trigger anchor. Returns (filter_expr, position,
    /// sentence_start).
    fn match_archetype_nostat_desc_for_destroyed(
        desc: &str,
    ) -> Option<(String, &'static str, usize)> {
        let needle_head = "Special Summon 1 \"";
        let mut cursor = 0;
        while let Some(off) = desc[cursor..].find(needle_head) {
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            // Expect " monster from your Deck" literally (no "with <N>").
            if tail.starts_with(" monster from your Deck") {
                if destroyed_trigger_anchor_before(desc, start) {
                    let segment = &desc[start..];
                    let position = pick_position(segment);
                    let expr = format!("archetype == \"{arch}\"");
                    return Some((expr, position, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for RecruiterDestroyedArchetypeNoStat {
        fn name(&self) -> &'static str { "recruiter_destroyed_archetype_nostat" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_placeholder_with_trigger(src, "destroyed") { return false; }
            match_archetype_nostat_desc_for_destroyed(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, position, _) =
                match_archetype_nostat_desc_for_destroyed(&cdb_row.desc)
                    .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            special_summon (1, monster, where {filter_expr}) from deck in {position}"
            );
            let old_line = "            special_summon (all, card, either controls)";
            let range = find_placeholder_body_range(
                src, "destroyed",
                "special_summon (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Cluster: search_battle_damage_archetype_monster (M.3) ────────
    //
    // Shape to hit:
    //   effect "X" {
    //       trigger: battle_damage
    //       resolve {
    //           add_to_hand (all, card, either controls)
    //       }
    //   }
    //
    //   desc: 'add 1 "<Archetype>" monster from your Deck to your hand'
    //
    // Rewrite:
    //   add_to_hand (1, monster, where archetype == "<Archetype>") from deck

    struct SearchBattleDamageArchetypeMonster;

    /// Find "add 1 \"<Arch>\" monster from your Deck [to your hand]" and
    /// require the battle-trigger anchor before it.
    fn match_search_archetype_monster_desc(desc: &str) -> Option<(String, usize)> {
        let needle_head_caps = "Add 1 \"";
        let needle_head_low  = "add 1 \"";
        let mut cursor = 0;
        while cursor < desc.len() {
            // Case-insensitive find by trying both casings.
            let off_caps = desc[cursor..].find(needle_head_caps);
            let off_low  = desc[cursor..].find(needle_head_low);
            let (off, needle_head) = match (off_caps, off_low) {
                (Some(a), Some(b)) if a < b => (a, needle_head_caps),
                (_, Some(b))                => (b, needle_head_low),
                (Some(a), None)             => (a, needle_head_caps),
                (None, None)                => break,
            };
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" monster from your Deck") {
                if trigger_anchor_before(desc, start) {
                    let expr = format!("archetype == \"{arch}\"");
                    return Some((expr, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for SearchBattleDamageArchetypeMonster {
        fn name(&self) -> &'static str { "search_battle_damage_archetype_monster" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_placeholder_with_trigger_and_body(
                src, "battle_damage", "add_to_hand (all, card, either controls)"
            ) { return false; }
            match_search_archetype_monster_desc(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, _) =
                match_search_archetype_monster_desc(&cdb_row.desc)
                    .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, monster, where {filter_expr}) from deck"
            );
            let old_line = "            add_to_hand (all, card, either controls)";
            let range = find_placeholder_body_range(
                src, "battle_damage",
                "add_to_hand (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Cluster: search_battle_damage_archetype_card (M.3) ──────────
    //
    // Same shape as the monster variant, but desc says "add 1 \"<Arch>\"
    // card from your Deck". We use `card` (not `monster`) as the card
    // filter since the desc explicitly allows Spell/Trap archetype
    // members (e.g. "Archfiend" cards spans monsters + Spells/Traps).
    //
    // Rewrite:
    //   add_to_hand (1, card, where archetype == "<Archetype>") from deck

    struct SearchBattleDamageArchetypeCard;

    fn match_search_archetype_card_desc(desc: &str) -> Option<(String, usize)> {
        let needle_head_caps = "Add 1 \"";
        let needle_head_low  = "add 1 \"";
        let mut cursor = 0;
        while cursor < desc.len() {
            let off_caps = desc[cursor..].find(needle_head_caps);
            let off_low  = desc[cursor..].find(needle_head_low);
            let (off, needle_head) = match (off_caps, off_low) {
                (Some(a), Some(b)) if a < b => (a, needle_head_caps),
                (_, Some(b))                => (b, needle_head_low),
                (Some(a), None)             => (a, needle_head_caps),
                (None, None)                => break,
            };
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" card from your Deck") {
                if trigger_anchor_before(desc, start) {
                    let expr = format!("archetype == \"{arch}\"");
                    return Some((expr, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for SearchBattleDamageArchetypeCard {
        fn name(&self) -> &'static str { "search_battle_damage_archetype_card" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_placeholder_with_trigger_and_body(
                src, "battle_damage", "add_to_hand (all, card, either controls)"
            ) { return false; }
            // The other cluster runs first (registered earlier). If the desc
            // matches a "monster" variant, skip here — one-rewrite-per-file
            // semantics handle it.
            match_search_archetype_card_desc(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, _) =
                match_search_archetype_card_desc(&cdb_row.desc)
                    .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, card, where {filter_expr}) from deck"
            );
            let old_line = "            add_to_hand (all, card, either controls)";
            let range = find_placeholder_body_range(
                src, "battle_damage",
                "add_to_hand (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Cluster: search_destroyed_archetype_monster (M.4 / WW-II) ───
    //
    // Mirror of SearchBattleDamageArchetypeMonster but uses the broader
    // `destroyed_trigger_anchor_before` anchor (from M.2) instead of
    // `trigger_anchor_before` (M.1). Handles cards like Raidraptors and
    // Fire King recursion search-on-destroy effects whose BabelCdb desc
    // reads: 'When this card is destroyed, ... add 1 "X" monster from
    // your Deck to your hand.'
    //
    // Shape to hit:
    //   effect "X" {
    //       trigger: destroyed
    //       resolve {
    //           add_to_hand (all, card, either controls)
    //       }
    //   }
    //
    // Rewrite:
    //   add_to_hand (1, monster, where archetype == "<Archetype>") from deck

    struct SearchDestroyedArchetypeMonster;

    /// Analogue of `match_search_archetype_monster_desc` with the
    /// destroyed-trigger anchor. Returns (filter_expr, position).
    fn match_search_archetype_monster_desc_for_destroyed(
        desc: &str,
    ) -> Option<(String, usize)> {
        let needle_head_caps = "Add 1 \"";
        let needle_head_low  = "add 1 \"";
        let mut cursor = 0;
        while cursor < desc.len() {
            let off_caps = desc[cursor..].find(needle_head_caps);
            let off_low  = desc[cursor..].find(needle_head_low);
            let (off, needle_head) = match (off_caps, off_low) {
                (Some(a), Some(b)) if a < b => (a, needle_head_caps),
                (_, Some(b))                => (b, needle_head_low),
                (Some(a), None)             => (a, needle_head_caps),
                (None, None)                => break,
            };
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" monster from your Deck") {
                if destroyed_trigger_anchor_before(desc, start) {
                    let expr = format!("archetype == \"{arch}\"");
                    return Some((expr, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for SearchDestroyedArchetypeMonster {
        fn name(&self) -> &'static str { "search_destroyed_archetype_monster" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_placeholder_with_trigger_and_body(
                src, "destroyed", "add_to_hand (all, card, either controls)"
            ) { return false; }
            match_search_archetype_monster_desc_for_destroyed(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, _) =
                match_search_archetype_monster_desc_for_destroyed(&cdb_row.desc)
                    .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, monster, where {filter_expr}) from deck"
            );
            let old_line = "            add_to_hand (all, card, either controls)";
            let range = find_placeholder_body_range(
                src, "destroyed",
                "add_to_hand (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Cluster: search_destroyed_archetype_card (M.4 / WW-II) ──────
    //
    // Same shape as the monster variant, but desc says "add 1 \"<Arch>\"
    // card from your Deck" (card, not monster — spans Spell/Trap
    // archetype members).
    //
    // Rewrite:
    //   add_to_hand (1, card, where archetype == "<Archetype>") from deck

    struct SearchDestroyedArchetypeCard;

    fn match_search_archetype_card_desc_for_destroyed(
        desc: &str,
    ) -> Option<(String, usize)> {
        let needle_head_caps = "Add 1 \"";
        let needle_head_low  = "add 1 \"";
        let mut cursor = 0;
        while cursor < desc.len() {
            let off_caps = desc[cursor..].find(needle_head_caps);
            let off_low  = desc[cursor..].find(needle_head_low);
            let (off, needle_head) = match (off_caps, off_low) {
                (Some(a), Some(b)) if a < b => (a, needle_head_caps),
                (_, Some(b))                => (b, needle_head_low),
                (Some(a), None)             => (a, needle_head_caps),
                (None, None)                => break,
            };
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" card from your Deck") {
                if destroyed_trigger_anchor_before(desc, start) {
                    let expr = format!("archetype == \"{arch}\"");
                    return Some((expr, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for SearchDestroyedArchetypeCard {
        fn name(&self) -> &'static str { "search_destroyed_archetype_card" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_placeholder_with_trigger_and_body(
                src, "destroyed", "add_to_hand (all, card, either controls)"
            ) { return false; }
            // Monster variant runs first (registered earlier). If the desc
            // matches a "monster" variant, this cluster's matches() will
            // still fire, but one-rewrite-per-file semantics let the
            // monster cluster's earlier-registered rewrite win.
            match_search_archetype_card_desc_for_destroyed(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, _) =
                match_search_archetype_card_desc_for_destroyed(&cdb_row.desc)
                    .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, card, where {filter_expr}) from deck"
            );
            let old_line = "            add_to_hand (all, card, either controls)";
            let range = find_placeholder_body_range(
                src, "destroyed",
                "add_to_hand (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            splice_block_scoped(src, range, old_line, &new_line)
        }
    }

    // ── Cluster: search_sent_to_gy_archetype_monster (M.5 / YY-II) ──
    //
    // Same shape as M.3/M.4 search clusters, but with `trigger:
    // sent_to gy` in the .ds file and a tributed / sent-to-GY
    // anchor in the BabelCdb desc.
    //
    // Shape to hit:
    //   effect "X" {
    //       trigger: sent_to gy
    //       resolve {
    //           add_to_hand (all, card, either controls)
    //       }
    //   }
    //
    //   desc anchor (any of): "if this card is tributed", "if this
    //   card is sent to the gy", "if this card is tributed and sent
    //   to the gy", "if this card is tributed by a card effect",
    //   "if this card is tributed for".
    //
    //   desc search phrase: 'Add 1 "<Arch>" monster from your Deck'.
    //
    // Rewrite:
    //   add_to_hand (1, monster, where archetype == "<Arch>") from deck

    struct SearchSentToGyArchetypeMonster;

    fn sent_to_gy_trigger_anchor_before(desc: &str, cursor: usize) -> bool {
        let prefix = &desc[..cursor];
        let lower  = prefix.to_lowercase();
        // "Tributed" is the dominant surface form for sent-to-GY-
        // triggered effects in BabelCdb desc text. "Sent to the GY"
        // is the explicit form. Both compile to `trigger: sent_to gy`
        // in the .ds effect block.
        if lower.contains("if this card is tributed") { return true; }
        if lower.contains("if this card is sent to the gy") { return true; }
        if lower.contains("if this card is sent to the graveyard") { return true; }
        // Less common but in-scope: tributed for X (ritual / tribute
        // summon) — still a `sent_to gy` event producer.
        if lower.contains("when this card is tributed") { return true; }
        if lower.contains("this card is tributed") { return true; }
        // "this card on the field is Tributed and sent to the GY"
        // (Evoltile Elginero pattern — qualifier between "card" and
        // "is tributed"). Only fire if BOTH "this card" and "tributed"
        // appear with "tributed" strictly after the "this card" start.
        if let Some(card_off) = lower.find("this card") {
            let after = &lower[card_off..];
            // Search within a short radius (avoid cross-paragraph).
            let radius = std::cmp::min(after.len(), 80);
            if after[..radius].contains("tributed") {
                return true;
            }
        }
        false
    }

    fn match_search_archetype_monster_desc_for_sent_to_gy(
        desc: &str,
    ) -> Option<(String, usize)> {
        let needle_head_caps = "Add 1 \"";
        let needle_head_low  = "add 1 \"";
        let mut cursor = 0;
        while cursor < desc.len() {
            let off_caps = desc[cursor..].find(needle_head_caps);
            let off_low  = desc[cursor..].find(needle_head_low);
            let (off, needle_head) = match (off_caps, off_low) {
                (Some(a), Some(b)) if a < b => (a, needle_head_caps),
                (_, Some(b))                => (b, needle_head_low),
                (Some(a), None)             => (a, needle_head_caps),
                (None, None)                => break,
            };
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" monster from your Deck") {
                if sent_to_gy_trigger_anchor_before(desc, start) {
                    let expr = format!("archetype == \"{arch}\"");
                    return Some((expr, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for SearchSentToGyArchetypeMonster {
        fn name(&self) -> &'static str { "search_sent_to_gy_archetype_monster" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            // M.6 / III-II: line-scoped locator allows multi-line
            // resolve bodies (e.g. placeholder stacks like
            // take_control → add_to_hand → destroy). The trigger +
            // line presence check is the only structural gate.
            if !has_placeholder_line_for_trigger(
                src, "sent_to gy", "add_to_hand (all, card, either controls)"
            ) { return false; }
            match_search_archetype_monster_desc_for_sent_to_gy(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, _) =
                match_search_archetype_monster_desc_for_sent_to_gy(&cdb_row.desc)
                    .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, monster, where {filter_expr}) from deck"
            );
            let range = find_placeholder_line_range(
                src, "sent_to gy",
                "add_to_hand (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            Ok(splice_placeholder_line(src, range, &new_line))
        }
    }

    // ── Cluster: search_sent_to_gy_archetype_card (M.5 / YY-II) ─────
    //
    // Same shape as the monster variant, but desc says "add 1 \"<Arch>\"
    // card from your Deck" (card, not monster — spans Spell/Trap
    // archetype members).

    struct SearchSentToGyArchetypeCard;

    fn match_search_archetype_card_desc_for_sent_to_gy(
        desc: &str,
    ) -> Option<(String, usize)> {
        let needle_head_caps = "Add 1 \"";
        let needle_head_low  = "add 1 \"";
        let mut cursor = 0;
        while cursor < desc.len() {
            let off_caps = desc[cursor..].find(needle_head_caps);
            let off_low  = desc[cursor..].find(needle_head_low);
            let (off, needle_head) = match (off_caps, off_low) {
                (Some(a), Some(b)) if a < b => (a, needle_head_caps),
                (_, Some(b))                => (b, needle_head_low),
                (Some(a), None)             => (a, needle_head_caps),
                (None, None)                => break,
            };
            let start = cursor + off;
            let after_head = start + needle_head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" card from your Deck") {
                if sent_to_gy_trigger_anchor_before(desc, start) {
                    let expr = format!("archetype == \"{arch}\"");
                    return Some((expr, start));
                }
            }
            cursor = start + needle_head.len();
        }
        None
    }

    impl Cluster for SearchSentToGyArchetypeCard {
        fn name(&self) -> &'static str { "search_sent_to_gy_archetype_card" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            // M.6 / III-II: multi-line resolve bodies allowed.
            if !has_placeholder_line_for_trigger(
                src, "sent_to gy", "add_to_hand (all, card, either controls)"
            ) { return false; }
            match_search_archetype_card_desc_for_sent_to_gy(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, _) =
                match_search_archetype_card_desc_for_sent_to_gy(&cdb_row.desc)
                    .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, card, where {filter_expr}) from deck"
            );
            let range = find_placeholder_line_range(
                src, "sent_to gy",
                "add_to_hand (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            Ok(splice_placeholder_line(src, range, &new_line))
        }
    }

    // ── Cluster: search_sent_to_gy_subtype_archetype_monster (M.7 / MMM-II) ──
    //
    // Same trigger+placeholder shape as M.5's archetype-monster variant,
    // but BabelCdb desc includes a race/type subtype adjective before
    // the archetype quote AND/OR a monster-subtype keyword (Ritual /
    // Fusion / Synchro / Xyz / Pendulum / Link) after it.
    //
    // Shape to hit:
    //   Add 1 <Race>[-Type]? \"<Arch>\" [Ritual|Fusion|Synchro|Xyz|
    //       Pendulum|Link]? Monster from your Deck
    //
    // Concrete examples (sent_to_gy trigger anchor required):
    //   "add 1 Warrior \"Nekroz\" Ritual Monster from your Deck"    — Shurit
    //   "add 1 Dragon-Type \"Nekroz\" Ritual Monster from your Deck" — Exa
    //   "add 1 Spellcaster-Type \"Nekroz\" Ritual Monster from
    //    your Deck"                                                 — Great Sorcerer
    //
    // Rewrite:
    //   add_to_hand (1, monster, where archetype == \"<Arch>\" and
    //       race == <Race>[ and is_ritual][ and is_fusion]…) from deck
    //
    // The RACE token is reused from the DRY grammar-aligned helper
    // `desc_race_to_predicate_token` below; monster-subtype keywords
    // map to predicate atoms `is_ritual`/`is_fusion`/etc.

    /// Map a BabelCdb race adjective to the grammar-exact DuelScript
    /// `race == X` literal. Returns `None` if the text is not a
    /// recognised race. Distinct from the buggy `ds_race_token`
    /// helper (which hyphenates "Winged Beast" / "Sea Serpent" —
    /// grammar requires the space form).
    fn desc_race_to_predicate_token(human: &str) -> Option<&'static str> {
        match human {
            "Dragon"        => Some("Dragon"),
            "Spellcaster"   => Some("Spellcaster"),
            "Zombie"        => Some("Zombie"),
            "Warrior"       => Some("Warrior"),
            "Beast-Warrior" => Some("Beast-Warrior"),
            "Beast"         => Some("Beast"),
            "Winged Beast"  => Some("Winged Beast"),
            "Fiend"         => Some("Fiend"),
            "Fairy"         => Some("Fairy"),
            "Insect"        => Some("Insect"),
            "Dinosaur"      => Some("Dinosaur"),
            "Reptile"       => Some("Reptile"),
            "Fish"          => Some("Fish"),
            "Sea Serpent"   => Some("Sea Serpent"),
            "Aqua"          => Some("Aqua"),
            "Pyro"          => Some("Pyro"),
            "Thunder"       => Some("Thunder"),
            "Rock"          => Some("Rock"),
            "Plant"         => Some("Plant"),
            "Machine"       => Some("Machine"),
            "Psychic"       => Some("Psychic"),
            "Divine-Beast"  => Some("Divine-Beast"),
            "Wyrm"          => Some("Wyrm"),
            "Cyberse"       => Some("Cyberse"),
            "Illusion"      => Some("Illusion"),
            _ => None,
        }
    }

    /// Strip an optional trailing "-Type" suffix (common BabelCdb
    /// phrasing, e.g. "Dragon-Type" → "Dragon").
    fn strip_type_suffix(s: &str) -> &str {
        s.strip_suffix("-Type").unwrap_or(s)
    }

    /// Try to match a race adjective ending at byte offset `end`
    /// in `desc`, scanning backwards. Returns `(grammar_race_token,
    /// adj_start_offset)` on success. Only fires when a grammar-valid
    /// race is present — callers must still verify the adjective is
    /// bracketed by the expected surrounding text.
    fn match_race_adjective_before(
        desc: &str, end: usize,
    ) -> Option<(&'static str, usize)> {
        // Try multi-word races first (grammar ordered-choice analog).
        const MULTI: &[&str] = &[
            "Winged Beast", "Beast-Warrior", "Sea Serpent", "Divine-Beast",
        ];
        const MULTI_WITH_TYPE: &[&str] = &[
            "Winged Beast-Type", "Beast-Warrior-Type",
            "Sea Serpent-Type", "Divine-Beast-Type",
        ];
        for m in MULTI_WITH_TYPE {
            let len = m.len();
            if end >= len && &desc[end - len..end] == *m {
                let bare = strip_type_suffix(m);
                if let Some(tok) = desc_race_to_predicate_token(bare) {
                    return Some((tok, end - len));
                }
            }
        }
        for m in MULTI {
            let len = m.len();
            if end >= len && &desc[end - len..end] == *m {
                if let Some(tok) = desc_race_to_predicate_token(m) {
                    return Some((tok, end - len));
                }
            }
        }
        // Single-word races: walk back over word chars and an optional
        // "-Type" suffix.
        let bytes = desc.as_bytes();
        let mut i = end;
        // Optional "-Type" suffix.
        if i >= 5 && &desc[i - 5..i] == "-Type" {
            i -= 5;
        }
        // Walk back over [A-Za-z] for a single word.
        let word_end = i;
        while i > 0 {
            let c = bytes[i - 1] as char;
            if c.is_ascii_alphabetic() { i -= 1; } else { break; }
        }
        if i == word_end { return None; }
        let word = &desc[i..word_end];
        desc_race_to_predicate_token(word).map(|tok| (tok, i))
    }

    /// Monster-subtype keyword after the archetype quote.
    /// Returns `(is_atom, keyword_len_incl_space)` if found.
    fn match_monster_subtype_kw(tail: &str) -> Option<(&'static str, usize)> {
        const KEYS: &[(&str, &str)] = &[
            ("Ritual",   "is_ritual"),
            ("Fusion",   "is_fusion"),
            ("Synchro",  "is_synchro"),
            ("Xyz",      "is_xyz"),
            ("Pendulum", "is_pendulum"),
            ("Link",     "is_link"),
        ];
        // `tail` starts immediately after the closing quote.
        // Accept forms like ` Ritual Monster from your Deck`.
        let stripped = tail.strip_prefix(' ')?;
        for (kw, atom) in KEYS {
            if let Some(rest) = stripped.strip_prefix(kw) {
                if rest.starts_with(" Monster from your Deck") {
                    return Some((atom, 1 + kw.len()));
                }
            }
        }
        None
    }

    struct SearchSentToGySubtypeArchetypeMonster;

    fn match_subtype_archetype_monster_desc_for_sent_to_gy(
        desc: &str,
    ) -> Option<(String, usize)> {
        let needle_head_caps = "Add 1 ";
        let needle_head_low  = "add 1 ";
        let mut cursor = 0;
        while cursor < desc.len() {
            let off_caps = desc[cursor..].find(needle_head_caps);
            let off_low  = desc[cursor..].find(needle_head_low);
            let (off, needle_head) = match (off_caps, off_low) {
                (Some(a), Some(b)) if a < b => (a, needle_head_caps),
                (_, Some(b))                => (b, needle_head_low),
                (Some(a), None)             => (a, needle_head_caps),
                (None, None)                => break,
            };
            let start = cursor + off;
            let after_head = start + needle_head.len();
            // Look for the opening quote of the archetype name.
            let rest = &desc[after_head..];
            let Some(q_open_rel) = rest.find('"') else { break };
            // Slice between `after_head` and the quote — must be a
            // valid race adjective (with optional -Type and trailing
            // space).
            let adj_end = after_head + q_open_rel;
            // Expect exactly one trailing space between adjective and quote.
            if adj_end == 0 || desc.as_bytes()[adj_end - 1] as char != ' ' {
                cursor = after_head;
                continue;
            }
            let (race_tok, _adj_start) = match match_race_adjective_before(
                desc, adj_end - 1,
            ) {
                Some(v) => v,
                None => { cursor = after_head; continue; }
            };
            // Extract archetype inside quotes.
            let arch_start = adj_end + 1;
            let rest_after_q = &desc[arch_start..];
            let Some(q_close_rel) = rest_after_q.find('"') else { break };
            let arch = &rest_after_q[..q_close_rel];
            let tail = &rest_after_q[q_close_rel + 1..];
            // Match optional " <Subtype> Monster from your Deck" or
            // " Monster from your Deck".
            let subtype_atom = match match_monster_subtype_kw(tail) {
                Some((atom, _)) => Some(atom),
                None => {
                    // Bare " Monster from your Deck" (no subtype kw).
                    if tail.starts_with(" Monster from your Deck") {
                        None
                    } else {
                        cursor = after_head;
                        continue;
                    }
                }
            };
            // Trigger anchor must be before `start` in desc.
            if !sent_to_gy_trigger_anchor_before(desc, start) {
                cursor = after_head;
                continue;
            }
            // Build predicate expression.
            let expr = match subtype_atom {
                Some(atom) => format!(
                    "archetype == \"{arch}\" and race == {race_tok} and {atom}"
                ),
                None => format!(
                    "archetype == \"{arch}\" and race == {race_tok}"
                ),
            };
            return Some((expr, start));
        }
        None
    }

    impl Cluster for SearchSentToGySubtypeArchetypeMonster {
        fn name(&self) -> &'static str {
            "search_sent_to_gy_subtype_archetype_monster"
        }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !has_placeholder_line_for_trigger(
                src, "sent_to gy", "add_to_hand (all, card, either controls)"
            ) { return false; }
            match_subtype_archetype_monster_desc_for_sent_to_gy(&cdb_row.desc).is_some()
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let (filter_expr, _) =
                match_subtype_archetype_monster_desc_for_sent_to_gy(&cdb_row.desc)
                    .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, monster, where {filter_expr}) from deck"
            );
            let range = find_placeholder_line_range(
                src, "sent_to gy",
                "add_to_hand (all, card, either controls)",
            ).ok_or_else(|| "matched effect block no longer found".to_string())?;
            Ok(splice_placeholder_line(src, range, &new_line))
        }
    }

    // ── Cluster: search_archetype_spell_trap (M.5-ext / OOO-II) ──
    //
    // Extends the M.5 archetype-search family to cover descs that
    // phrase the search target as `Add 1 \"<Arch>\" Spell/Trap from
    // your Deck` (i.e. a type-restricted search covering BOTH
    // Spell-type and Trap-type archetype members). Grammar has no
    // bare `Spell` / `Trap` card-type literal — `parse_card_type`
    // only accepts subtype-qualified forms (Normal Spell, Counter
    // Trap, etc.). The correct predicate is the parenthesised
    // disjunction `(is_spell or is_trap)`, which grammar supports
    // via `pred_atom = "(" ~ predicate ~ ")"`.
    //
    // Trigger-agnostic: fires on any of a set of supported trigger
    // values (sent_to gy, destroyed, battle_damage, summoned,
    // `summoned by special`, standby_phase, leaves_field,
    // destroyed_by_battle) that has the canonical placeholder.
    // Tries each trigger in order and takes the first match.
    //
    // Rewrite:
    //   add_to_hand (1, card,
    //       where archetype == \"<Arch>\" and (is_spell or is_trap))
    //       from deck

    struct SearchArchetypeSpellTrap;

    /// Match `Add 1 "<Arch>" Spell/Trap from your Deck` in desc.
    /// Returns the extracted archetype string. No trigger-anchor
    /// gate because the desc phrasing is highly specific
    /// (appears only in sentences that describe exactly this
    /// search mode).
    fn match_archetype_spell_trap_desc(desc: &str) -> Option<String> {
        let heads = ["Add 1 \"", "add 1 \""];
        let mut cursor = 0;
        while cursor < desc.len() {
            let hits: Vec<(usize, &str)> = heads.iter()
                .filter_map(|h| desc[cursor..].find(h).map(|o| (o, *h)))
                .collect();
            let (off, head) = *hits.iter().min_by_key(|(o, _)| *o)?;
            let start = cursor + off;
            let after_head = start + head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" Spell/Trap from your Deck") {
                return Some(arch.to_string());
            }
            cursor = after_head;
        }
        None
    }

    /// Trigger values that the Spell/Trap cluster will probe for
    /// a canonical placeholder. Order matters: earlier triggers
    /// win on first-match. `sent_to gy` first (highest-fidelity
    /// anchor signal in the pool), then summon-family, then the
    /// battle family. The rewriter checks each trigger until it
    /// finds a placeholder line to replace.
    const SPELL_TRAP_TRIGGERS: &[&str] = &[
        "sent_to gy",
        "summoned by special",
        "summoned",
        "destroyed_by_battle",
        "destroyed",
        "battle_damage",
        "standby_phase",
        "leaves_field",
    ];

    impl Cluster for SearchArchetypeSpellTrap {
        fn name(&self) -> &'static str { "search_archetype_spell_trap" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if match_archetype_spell_trap_desc(&cdb_row.desc).is_none() {
                return false;
            }
            // Need at least one supported trigger with the placeholder.
            SPELL_TRAP_TRIGGERS.iter().any(|t| {
                has_placeholder_line_for_trigger(
                    src, t, "add_to_hand (all, card, either controls)",
                )
            })
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let arch = match_archetype_spell_trap_desc(&cdb_row.desc)
                .ok_or_else(|| "desc no longer matches".to_string())?;
            // Grammar has no bare `Spell` / `Trap` card-type literal,
            // AND the parser rejects nested OR-predicates (parser.rs
            // `parse_pred_atom` accepts `(predicate)` only when it
            // flattens to a single atom or pure AND conjunction — see
            // FF-I fork rule; grammar tweak is out of M-phase scope).
            // `not is_monster` is the grammar-supported equivalent
            // under the closed-world assumption that every .cdb card
            // row is monster, spell, or trap.
            let new_line = format!(
                "            add_to_hand (1, card, where archetype == \"{arch}\" and not is_monster) from deck"
            );
            // Find the first supported trigger with the placeholder
            // and splice there.
            for trig in SPELL_TRAP_TRIGGERS {
                if let Some(range) = find_placeholder_line_range(
                    src, trig, "add_to_hand (all, card, either controls)",
                ) {
                    return Ok(splice_placeholder_line(src, range, &new_line));
                }
            }
            Err("no supported trigger block with canonical placeholder".to_string())
        }
    }

    // ── Cluster: search_archetype_monster_any_trigger (M.8 / TTT-II) ──
    //
    // Trigger-agnostic variant of the archetype-monster search family.
    // Mirrors `SearchArchetypeSpellTrap` (OOO-II) for the `monster`
    // target. Fires on any supported trigger block carrying the
    // canonical placeholder when the desc phrases the search as
    // `Add 1 "<Arch>" monster from your Deck`.
    //
    // Rationale (M.8 residual audit / Task #28):
    //   Cards like "Mitsurugi no Mikoto, Aramasa" have a compound
    //   trigger ("If this card is Normal or Special Summoned, or if
    //   this card is Tributed") that lands as separate effect blocks
    //   with triggers `summoned`, `summoned by special`, and
    //   `sent_to gy`. The existing sent_to_gy variant
    //   (YY-II / ZZ-II) catches the third block; the summoned blocks
    //   need this trigger-agnostic variant.
    //
    // Order: registered AFTER the anchor-gated monster clusters
    // (search_{battle_damage,destroyed,sent_to_gy}_archetype_monster)
    // so those win first on their respective triggers. This cluster
    // sweeps the residual summoned / summoned-by-special /
    // standby_phase / leaves_field / destroyed_by_battle blocks that
    // would otherwise remain placeholders.
    //
    // Rewrite:
    //   add_to_hand (1, monster, where archetype == "<Arch>") from deck

    struct SearchArchetypeMonsterAnyTrigger;

    /// Match `Add 1 "<Arch>" monster from your Deck` in desc, with no
    /// trigger anchor (the tight `monster from your Deck` suffix is
    /// specific enough to stand alone — same anchor-free reasoning as
    /// `match_archetype_spell_trap_desc`).
    fn match_archetype_monster_any_desc(desc: &str) -> Option<String> {
        let heads = ["Add 1 \"", "add 1 \""];
        let mut cursor = 0;
        while cursor < desc.len() {
            let hits: Vec<(usize, &str)> = heads.iter()
                .filter_map(|h| desc[cursor..].find(h).map(|o| (o, *h)))
                .collect();
            let (off, head) = *hits.iter().min_by_key(|(o, _)| *o)?;
            let start = cursor + off;
            let after_head = start + head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" monster from your Deck") {
                return Some(arch.to_string());
            }
            cursor = after_head;
        }
        None
    }

    /// Trigger set probed by the trigger-agnostic clusters. Identical
    /// to `SPELL_TRAP_TRIGGERS` (same supported set). `sent_to gy`
    /// listed first for source-order-consistent sweep; earlier
    /// trigger-anchored clusters in `build_clusters` will have already
    /// claimed that block before this cluster fires.
    const ANY_TRIGGER_SET: &[&str] = &[
        "sent_to gy",
        "summoned by special",
        "summoned",
        "destroyed_by_battle",
        "destroyed",
        "battle_damage",
        "standby_phase",
        "leaves_field",
    ];

    impl Cluster for SearchArchetypeMonsterAnyTrigger {
        fn name(&self) -> &'static str { "search_archetype_monster_any_trigger" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if match_archetype_monster_any_desc(&cdb_row.desc).is_none() {
                return false;
            }
            ANY_TRIGGER_SET.iter().any(|t| {
                has_placeholder_line_for_trigger(
                    src, t, "add_to_hand (all, card, either controls)",
                )
            })
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let arch = match_archetype_monster_any_desc(&cdb_row.desc)
                .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, monster, where archetype == \"{arch}\") from deck"
            );
            for trig in ANY_TRIGGER_SET {
                if let Some(range) = find_placeholder_line_range(
                    src, trig, "add_to_hand (all, card, either controls)",
                ) {
                    return Ok(splice_placeholder_line(src, range, &new_line));
                }
            }
            Err("no supported trigger block with canonical placeholder".to_string())
        }
    }

    // ── Cluster: search_archetype_card_any_trigger (M.8 / TTT-II) ──
    //
    // Same structure as the monster variant, but for the `card` target
    // phrasing `Add 1 "<Arch>" card from your Deck`. Mirrors the
    // M.5 sent_to_gy card variant trigger-agnostically. Emits:
    //   add_to_hand (1, card, where archetype == "<Arch>") from deck
    //
    // Note: the card-target desc shape also covers spell/trap members
    // by name, but this emits no `not is_monster` predicate — the
    // BabelCdb phrasing `"<Arch>" card` (not `"<Arch>" Spell/Trap`)
    // denotes a cross-type search permitting monster too, so the
    // unconstrained `archetype == X` predicate is correct.

    struct SearchArchetypeCardAnyTrigger;

    /// Match `Add 1 "<Arch>" card from your Deck` in desc.
    fn match_archetype_card_any_desc(desc: &str) -> Option<String> {
        let heads = ["Add 1 \"", "add 1 \""];
        let mut cursor = 0;
        while cursor < desc.len() {
            let hits: Vec<(usize, &str)> = heads.iter()
                .filter_map(|h| desc[cursor..].find(h).map(|o| (o, *h)))
                .collect();
            let (off, head) = *hits.iter().min_by_key(|(o, _)| *o)?;
            let start = cursor + off;
            let after_head = start + head.len();
            let rest = &desc[after_head..];
            let Some(q_end) = rest.find('"') else { break };
            let arch = &rest[..q_end];
            let tail = &rest[q_end + 1..];
            if tail.starts_with(" card from your Deck") {
                return Some(arch.to_string());
            }
            cursor = after_head;
        }
        None
    }

    impl Cluster for SearchArchetypeCardAnyTrigger {
        fn name(&self) -> &'static str { "search_archetype_card_any_trigger" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if match_archetype_card_any_desc(&cdb_row.desc).is_none() {
                return false;
            }
            ANY_TRIGGER_SET.iter().any(|t| {
                has_placeholder_line_for_trigger(
                    src, t, "add_to_hand (all, card, either controls)",
                )
            })
        }

        fn rewrite(&self, src: &str, cdb_row: &CdbCard) -> Result<String, String> {
            let arch = match_archetype_card_any_desc(&cdb_row.desc)
                .ok_or_else(|| "desc no longer matches".to_string())?;
            let new_line = format!(
                "            add_to_hand (1, card, where archetype == \"{arch}\") from deck"
            );
            for trig in ANY_TRIGGER_SET {
                if let Some(range) = find_placeholder_line_range(
                    src, trig, "add_to_hand (all, card, either controls)",
                ) {
                    return Ok(splice_placeholder_line(src, range, &new_line));
                }
            }
            Err("no supported trigger block with canonical placeholder".to_string())
        }
    }

    // ── Cluster: destroy_all_opponent_monsters_any_trigger (P3) ──
    //
    // First non-search/non-recruiter cluster — proves the existing
    // translator architecture extends cleanly to the destroy family
    // (~2,300 placeholder hits in the corpus pre-translation).
    //
    // Shape to hit:
    //   trigger: <any of supported set>
    //   resolve { destroy (all, card, either controls) }
    //   desc: "Destroy all monsters your opponent controls"
    //         (case-insensitive, exact phrase match)
    //
    // Rewrite:
    //   destroy (all, monster, opponent controls)
    //
    // Conservative match: requires the exact canonical phrase. Cards
    // with conditional language ("If <cond>, destroy ...") still match
    // because the rewrite ignores the trigger condition — that lives in
    // the existing `trigger:` line, not in the resolve body.

    struct DestroyAllOpponentMonstersAnyTrigger;

    /// Match the canonical "Destroy all monsters your opponent controls"
    /// phrase, case-insensitive. Returns true if found.
    fn match_destroy_all_opp_monsters_desc(desc: &str) -> bool {
        let lower = desc.to_lowercase();
        lower.contains("destroy all monsters your opponent controls")
    }

    impl Cluster for DestroyAllOpponentMonstersAnyTrigger {
        fn name(&self) -> &'static str { "destroy_all_opponent_monsters_any_trigger" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !match_destroy_all_opp_monsters_desc(&cdb_row.desc) {
                return false;
            }
            ANY_TRIGGER_SET.iter().any(|t| {
                has_placeholder_line_for_trigger(
                    src, t, "destroy (all, card, either controls)",
                )
            })
        }

        fn rewrite(&self, src: &str, _cdb_row: &CdbCard) -> Result<String, String> {
            let new_line = "            destroy (all, monster, opponent controls)";
            for trig in ANY_TRIGGER_SET {
                if let Some(range) = find_placeholder_line_range(
                    src, trig, "destroy (all, card, either controls)",
                ) {
                    return Ok(splice_placeholder_line(src, range, new_line));
                }
            }
            Err("no supported trigger block with canonical placeholder".to_string())
        }
    }

    // ── Cluster: send_all_opponent_monsters_to_gy_any_trigger (P3) ──
    //
    // Sister cluster to destroy_all_opponent_monsters. Same architecture,
    // different action (`send ... to gy` vs `destroy`).
    //
    // Shape:
    //   resolve { send (all, card, either controls) to gy }
    //   desc: "Send all monsters your opponent controls to the (Graveyard|GY)"
    //
    // Rewrite:
    //   send (all, monster, opponent controls) to gy

    struct SendAllOpponentMonstersToGyAnyTrigger;

    fn match_send_all_opp_monsters_to_gy_desc(desc: &str) -> bool {
        let lower = desc.to_lowercase();
        lower.contains("send all monsters your opponent controls to the graveyard")
            || lower.contains("send all monsters your opponent controls to the gy")
    }

    impl Cluster for SendAllOpponentMonstersToGyAnyTrigger {
        fn name(&self) -> &'static str { "send_all_opponent_monsters_to_gy_any_trigger" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !match_send_all_opp_monsters_to_gy_desc(&cdb_row.desc) {
                return false;
            }
            ANY_TRIGGER_SET.iter().any(|t| {
                has_placeholder_line_for_trigger(
                    src, t, "send (all, card, either controls) to gy",
                )
            })
        }

        fn rewrite(&self, src: &str, _cdb_row: &CdbCard) -> Result<String, String> {
            let new_line = "            send (all, monster, opponent controls) to gy";
            for trig in ANY_TRIGGER_SET {
                if let Some(range) = find_placeholder_line_range(
                    src, trig, "send (all, card, either controls) to gy",
                ) {
                    return Ok(splice_placeholder_line(src, range, new_line));
                }
            }
            Err("no supported trigger block with canonical placeholder".to_string())
        }
    }

    // ── Cluster: banish_all_opponent_monsters_any_trigger (P3) ──
    //
    // Sister cluster — banish action variant.
    //
    // Shape:
    //   resolve { banish (all, card, either controls) }
    //   desc: "Banish all monsters your opponent controls"

    struct BanishAllOpponentMonstersAnyTrigger;

    fn match_banish_all_opp_monsters_desc(desc: &str) -> bool {
        let lower = desc.to_lowercase();
        lower.contains("banish all monsters your opponent controls")
    }

    impl Cluster for BanishAllOpponentMonstersAnyTrigger {
        fn name(&self) -> &'static str { "banish_all_opponent_monsters_any_trigger" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !match_banish_all_opp_monsters_desc(&cdb_row.desc) {
                return false;
            }
            ANY_TRIGGER_SET.iter().any(|t| {
                has_placeholder_line_for_trigger(
                    src, t, "banish (all, card, either controls)",
                )
            })
        }

        fn rewrite(&self, src: &str, _cdb_row: &CdbCard) -> Result<String, String> {
            let new_line = "            banish (all, monster, opponent controls)";
            for trig in ANY_TRIGGER_SET {
                if let Some(range) = find_placeholder_line_range(
                    src, trig, "banish (all, card, either controls)",
                ) {
                    return Ok(splice_placeholder_line(src, range, new_line));
                }
            }
            Err("no supported trigger block with canonical placeholder".to_string())
        }
    }

    // ── Cluster: return_all_opponent_monsters_to_hand_any_trigger (P3) ──
    //
    // "Bounce" cluster — return monsters to hand.
    //
    // Shape:
    //   resolve { return (all, card, either controls) to hand }
    //   desc: "Return all monsters your opponent controls to the hand"

    struct ReturnAllOpponentMonstersToHandAnyTrigger;

    fn match_return_all_opp_monsters_to_hand_desc(desc: &str) -> bool {
        let lower = desc.to_lowercase();
        lower.contains("return all monsters your opponent controls to the hand")
            || lower.contains("return all monsters your opponent controls to their hand")
            || lower.contains("return all monsters your opponent controls to the owner")
    }

    impl Cluster for ReturnAllOpponentMonstersToHandAnyTrigger {
        fn name(&self) -> &'static str { "return_all_opponent_monsters_to_hand_any_trigger" }

        fn matches(&self, src: &str, cdb_row: &CdbCard) -> bool {
            if !match_return_all_opp_monsters_to_hand_desc(&cdb_row.desc) {
                return false;
            }
            ANY_TRIGGER_SET.iter().any(|t| {
                has_placeholder_line_for_trigger(
                    src, t, "return (all, card, either controls) to hand",
                )
            })
        }

        fn rewrite(&self, src: &str, _cdb_row: &CdbCard) -> Result<String, String> {
            let new_line = "            return (all, monster, opponent controls) to hand";
            for trig in ANY_TRIGGER_SET {
                if let Some(range) = find_placeholder_line_range(
                    src, trig, "return (all, card, either controls) to hand",
                ) {
                    return Ok(splice_placeholder_line(src, range, new_line));
                }
            }
            Err("no supported trigger block with canonical placeholder".to_string())
        }
    }

    /// Return true if `src` contains an effect block with:
    ///   - trigger: battle_damage
    ///   - single-line resolve body exactly
    ///     `special_summon (all, card, either controls)`
    ///
    /// Thin wrapper for backward compat with M.0/M.1 call sites. Prefer
    /// `has_placeholder_with_trigger` for new clusters.
    fn has_battle_damage_placeholder(src: &str) -> bool {
        has_placeholder_with_trigger_and_body(
            src, "battle_damage",
            "special_summon (all, card, either controls)",
        )
    }

    /// Generalised form: return true if `src` contains an effect block with:
    ///   - trigger: <trigger_value>
    ///   - single-line resolve body exactly
    ///     `special_summon (all, card, either controls)`
    ///
    /// Thin wrapper for `has_placeholder_with_trigger_and_body` with the
    /// M.0/M.1/M.2 canonical placeholder body. M.3+ uses the more general
    /// form.
    fn has_placeholder_with_trigger(src: &str, trigger_value: &str) -> bool {
        has_placeholder_with_trigger_and_body(
            src, trigger_value,
            "special_summon (all, card, either controls)",
        )
    }

    /// Fully generalised form: return true if `src` contains an effect
    /// block with:
    ///   - trigger: <trigger_value>
    ///   - single-line resolve body exactly equal to `body_value`
    ///
    /// The trigger value is matched by prefix (matches M.0/M.1 semantics
    /// where `body_has_line_starting_with` is also prefix-based). The
    /// body value is matched literally after trimming.
    fn has_placeholder_with_trigger_and_body(
        src: &str, trigger_value: &str, body_value: &str,
    ) -> bool {
        find_placeholder_body_range(src, trigger_value, body_value).is_some()
    }

    /// Block-scoped locator: find the first effect block in `src`
    /// whose trigger matches `trigger_value` AND whose resolve body
    /// (trimmed) equals `body_value`. Returns the absolute byte
    /// range `(rb_start, rb_end)` of that resolve body's inner
    /// content (the span between the resolve `{` and matching `}`).
    ///
    /// QQ-II: cluster `rewrite()` uses this range to scope
    /// `replacen` to the matched effect block — the bare
    /// `src.replacen(old, new, 1)` previously used was a latent
    /// multi-effect bug (replaced first textual occurrence across
    /// the whole file, not the occurrence inside the matched effect
    /// block).
    fn find_placeholder_body_range(
        src: &str, trigger_value: &str, body_value: &str,
    ) -> Option<(usize, usize)> {
        let bytes = src.as_bytes();

        let mut i = 0;
        while i < bytes.len() {
            // Find next "effect \"" start.
            let start = find_from(src, i, "effect \"")?;
            // Advance past the name + opening brace.
            let brace = src[start..].find('{')?;
            let body_start = start + brace + 1;
            // Match the brace.
            let body_end = match_brace(src, body_start)?;
            let body = &src[body_start..body_end];
            i = body_end + 1;

            // Must have the requested trigger.
            if !body_has_line_starting_with(body, "trigger:", trigger_value) {
                continue;
            }
            // Find resolve { ... } inside body.
            let Some(r_brace) = body.find("resolve") else { continue };
            let Some(r_brace_open) = body[r_brace..].find('{') else { continue };
            let rb_start_rel = r_brace + r_brace_open + 1;
            let Some(rb_end_rel) = match_brace(body, rb_start_rel) else { continue };
            let rbody = body[rb_start_rel..rb_end_rel].trim();
            if rbody == body_value {
                // Convert to absolute range in `src`.
                let abs_start = body_start + rb_start_rel;
                let abs_end   = body_start + rb_end_rel;
                return Some((abs_start, abs_end));
            }
        }
        None
    }

    /// Helper used by every cluster `rewrite()` to splice a new
    /// resolve-body line into the specific effect block that matched.
    ///
    /// Given a `src`, the absolute byte range `(body_start, body_end)`
    /// of the matched resolve body (from
    /// `find_placeholder_body_range`), and the `old_line` /
    /// `new_line` pair, this performs `replacen(old_line, new_line,
    /// 1)` ONLY within `src[body_start..body_end]`, preserving
    /// bytes outside that range verbatim.
    ///
    /// Returns `Err` if `old_line` doesn't appear within the matched
    /// body (which would indicate a logic bug in the caller).
    fn splice_block_scoped(
        src: &str,
        body_range: (usize, usize),
        old_line: &str,
        new_line: &str,
    ) -> Result<String, String> {
        let (body_start, body_end) = body_range;
        let body = &src[body_start..body_end];
        if !body.contains(old_line) {
            return Err("expected placeholder line not found in matched block".into());
        }
        let replaced = body.replacen(old_line, new_line, 1);
        let mut out = String::with_capacity(src.len() + new_line.len());
        out.push_str(&src[..body_start]);
        out.push_str(&replaced);
        out.push_str(&src[body_end..]);
        Ok(out)
    }

    fn find_from(src: &str, start: usize, needle: &str) -> Option<usize> {
        src[start..].find(needle).map(|n| start + n)
    }

    /// Given index `i` just past a '{', find the matching '}' index.
    fn match_brace(src: &str, i: usize) -> Option<usize> {
        let bytes = src.as_bytes();
        let mut depth: i32 = 1;
        let mut j = i;
        while j < bytes.len() {
            match bytes[j] as char {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 { return Some(j); }
                }
                _   => {}
            }
            j += 1;
        }
        None
    }

    /// Return true if any non-empty line inside `body` starts with
    /// `prefix` (after trimming leading whitespace) and its remainder
    /// begins with `expected_value` (after ':' trim).
    fn body_has_line_starting_with(body: &str, prefix: &str, expected_value: &str) -> bool {
        for line in body.lines() {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix(prefix) {
                let v = rest.trim().trim_start_matches(':').trim();
                if v.starts_with(expected_value) {
                    return true;
                }
            }
        }
        false
    }

    // ── M.6 / III-II: line-scoped locator for multi-line resolve bodies ──
    //
    // `find_placeholder_body_range` requires the resolve body (trimmed)
    // to equal `body_value` verbatim — one action, nothing else. That
    // skips 34 sent_to_gy files whose resolve body stacks multiple
    // placeholder actions (e.g. take_control → add_to_hand → destroy)
    // even though the target line itself is present.
    //
    // `find_placeholder_line_range` is the generalised form. It walks
    // effect blocks, checks the trigger, then scans the resolve body
    // line-by-line looking for a line whose *trimmed* content equals
    // `placeholder_line.trim()`. On hit it returns the absolute byte
    // range `(line_start, line_end_exclusive)` where `line_end_exclusive`
    // points to the start of the next line (i.e. just past the trailing
    // '\n'). Splicing this range with `new_line + "\n"` preserves every
    // sibling line in the resolve body.

    /// Find the absolute byte range of a specific line within an
    /// effect block's resolve body. Returns `(line_start,
    /// line_end_exclusive)`; `line_end_exclusive` is the index just
    /// past the line's trailing '\n' (or == `src.len()` for the
    /// unterminated final line).
    fn find_placeholder_line_range(
        src: &str, trigger_value: &str, placeholder_line: &str,
    ) -> Option<(usize, usize)> {
        let bytes = src.as_bytes();
        let needle = placeholder_line.trim();

        let mut i = 0;
        while i < bytes.len() {
            let start = find_from(src, i, "effect \"")?;
            let brace = src[start..].find('{')?;
            let body_start = start + brace + 1;
            let body_end = match_brace(src, body_start)?;
            let body = &src[body_start..body_end];
            i = body_end + 1;

            if !body_has_line_starting_with(body, "trigger:", trigger_value) {
                continue;
            }
            let Some(r_at) = body.find("resolve") else { continue };
            let Some(r_open_rel) = body[r_at..].find('{') else { continue };
            let rb_start_rel = r_at + r_open_rel + 1;
            let Some(rb_end_rel) = match_brace(body, rb_start_rel) else { continue };
            let rbody_abs_start = body_start + rb_start_rel;
            let rbody_abs_end   = body_start + rb_end_rel;

            // Scan lines in the resolve body (by absolute byte offsets
            // so we can return a range usable against `src`).
            let mut line_start = rbody_abs_start;
            while line_start < rbody_abs_end {
                // Find end of this line (either newline or rbody end).
                let mut line_end = line_start;
                while line_end < rbody_abs_end && bytes[line_end] as char != '\n' {
                    line_end += 1;
                }
                // `line_end_exclusive` includes the newline if present.
                let line_end_exclusive = if line_end < rbody_abs_end {
                    line_end + 1
                } else {
                    line_end
                };
                let line = &src[line_start..line_end];
                if line.trim() == needle {
                    return Some((line_start, line_end_exclusive));
                }
                line_start = line_end_exclusive;
            }
        }
        None
    }

    /// True iff the file contains an effect block with `trigger:
    /// <trigger_value>` whose resolve body contains a line equal to
    /// `placeholder_line` (trimmed). Multi-line resolve bodies allowed.
    fn has_placeholder_line_for_trigger(
        src: &str, trigger_value: &str, placeholder_line: &str,
    ) -> bool {
        find_placeholder_line_range(src, trigger_value, placeholder_line).is_some()
    }

    /// Splice a `new_line` into a line range produced by
    /// `find_placeholder_line_range`. `new_line` must NOT include a
    /// trailing '\n' — the helper re-attaches one only if the replaced
    /// line had one (preserves the "final line has no newline"
    /// edge case).
    fn splice_placeholder_line(
        src: &str, line_range: (usize, usize), new_line: &str,
    ) -> String {
        let (ls, le) = line_range;
        let had_newline = le > ls && src.as_bytes()[le - 1] == b'\n';
        let mut out = String::with_capacity(src.len() + new_line.len());
        out.push_str(&src[..ls]);
        out.push_str(new_line);
        if had_newline {
            out.push('\n');
        }
        out.push_str(&src[le..]);
        out
    }

    // ── Inline tests for the new locator ─────────────────────────
    #[cfg(test)]
    mod line_locator_tests {
        use super::*;

        const MULTI_BODY: &str = "\
card \"X\" {
    id: 1
    type: Spell

    effect \"Effect 1\" {
        trigger: sent_to gy
        resolve {
            take_control (all, card, either controls)
            add_to_hand (all, card, either controls)
            destroy (all, card, either controls)
        }
    }
}
";

        const SINGLE_BODY: &str = "\
card \"X\" {
    id: 1
    type: Spell

    effect \"Effect 1\" {
        trigger: sent_to gy
        resolve {
            add_to_hand (all, card, either controls)
        }
    }
}
";

        #[test]
        fn finds_line_in_multi_line_resolve_body() {
            let r = find_placeholder_line_range(
                MULTI_BODY, "sent_to gy",
                "add_to_hand (all, card, either controls)",
            );
            assert!(r.is_some(), "should locate line in multi-line body");
            let (ls, le) = r.unwrap();
            let span = &MULTI_BODY[ls..le];
            assert!(span.contains("add_to_hand"), "span = {:?}", span);
            assert!(span.ends_with('\n'), "must include trailing newline");
        }

        #[test]
        fn finds_line_in_single_line_resolve_body() {
            let r = find_placeholder_line_range(
                SINGLE_BODY, "sent_to gy",
                "add_to_hand (all, card, either controls)",
            );
            assert!(r.is_some());
        }

        #[test]
        fn returns_none_when_trigger_mismatches() {
            let r = find_placeholder_line_range(
                MULTI_BODY, "destroyed",
                "add_to_hand (all, card, either controls)",
            );
            assert!(r.is_none());
        }

        #[test]
        fn returns_none_when_line_absent() {
            let r = find_placeholder_line_range(
                MULTI_BODY, "sent_to gy",
                "banish (all, card, either controls)",
            );
            assert!(r.is_none());
        }

        #[test]
        fn splice_preserves_sibling_lines() {
            let range = find_placeholder_line_range(
                MULTI_BODY, "sent_to gy",
                "add_to_hand (all, card, either controls)",
            ).unwrap();
            let out = splice_placeholder_line(
                MULTI_BODY, range,
                "            add_to_hand (1, monster, where archetype == \"Foo\") from deck",
            );
            assert!(out.contains("take_control (all, card, either controls)"));
            assert!(out.contains("destroy (all, card, either controls)"));
            assert!(out.contains("where archetype == \"Foo\""));
            assert!(!out.contains("add_to_hand (all, card, either controls)"));
        }
    }

    // ── M.7 / MMM-II inline tests for subtype-archetype matcher ──
    #[cfg(test)]
    mod subtype_archetype_matcher_tests {
        use super::*;

        #[test]
        fn matches_warrior_nekroz_ritual() {
            let desc = "If this card is Tributed by a card effect: You can add 1 Warrior \"Nekroz\" Ritual Monster from your Deck to your hand.";
            let (expr, _) = match_subtype_archetype_monster_desc_for_sent_to_gy(desc)
                .expect("should match");
            assert_eq!(expr, "archetype == \"Nekroz\" and race == Warrior and is_ritual");
        }

        #[test]
        fn matches_dragon_type_nekroz_ritual() {
            let desc = "If this card is Tributed by a card effect: You can add 1 Dragon-Type \"Nekroz\" Ritual Monster from your Deck to your hand.";
            let (expr, _) = match_subtype_archetype_monster_desc_for_sent_to_gy(desc)
                .expect("should match");
            assert_eq!(expr, "archetype == \"Nekroz\" and race == Dragon and is_ritual");
        }

        #[test]
        fn matches_spellcaster_type_nekroz_ritual() {
            let desc = "If this card is Tributed by a card effect: You can add 1 Spellcaster-Type \"Nekroz\" Ritual Monster from your Deck to your hand.";
            let (expr, _) = match_subtype_archetype_monster_desc_for_sent_to_gy(desc)
                .expect("should match");
            assert_eq!(expr, "archetype == \"Nekroz\" and race == Spellcaster and is_ritual");
        }

        #[test]
        fn matches_bare_race_no_subtype() {
            // Race adjective + archetype quote + bare " Monster from your Deck"
            // (no Ritual/Fusion/etc. keyword).
            let desc = "If this card is Tributed: You can add 1 Warrior \"Foo\" Monster from your Deck to your hand.";
            let (expr, _) = match_subtype_archetype_monster_desc_for_sent_to_gy(desc)
                .expect("should match");
            assert_eq!(expr, "archetype == \"Foo\" and race == Warrior");
        }

        #[test]
        fn rejects_missing_anchor() {
            // No sent_to_gy anchor; must not match.
            let desc = "You can add 1 Warrior \"Nekroz\" Ritual Monster from your Deck to your hand.";
            assert!(match_subtype_archetype_monster_desc_for_sent_to_gy(desc).is_none());
        }

        #[test]
        fn rejects_unknown_race_adjective() {
            // "Angelic" is not a grammar race token.
            let desc = "If this card is Tributed: You can add 1 Angelic \"Foo\" Ritual Monster from your Deck to your hand.";
            assert!(match_subtype_archetype_monster_desc_for_sent_to_gy(desc).is_none());
        }

        #[test]
        fn matches_winged_beast_multiword_race() {
            let desc = "If this card is Tributed: You can add 1 Winged Beast \"Foo\" Monster from your Deck to your hand.";
            let (expr, _) = match_subtype_archetype_monster_desc_for_sent_to_gy(desc)
                .expect("should match");
            assert_eq!(expr, "archetype == \"Foo\" and race == Winged Beast");
        }
    }

    // ── M.5-ext / OOO-II inline tests for Spell/Trap matcher ──
    #[cfg(test)]
    mod archetype_spell_trap_matcher_tests {
        use super::*;

        #[test]
        fn matches_simple_archetype_spell_trap() {
            let desc = "If this card is sent to the GY: You can add 1 \"Vendread\" Spell/Trap from your Deck to the hand.";
            assert_eq!(match_archetype_spell_trap_desc(desc).as_deref(), Some("Vendread"));
        }

        #[test]
        fn matches_lowercase_add() {
            let desc = "When summoned: You can add 1 \"Mitsurugi\" Spell/Trap from your Deck to your hand.";
            assert_eq!(match_archetype_spell_trap_desc(desc).as_deref(), Some("Mitsurugi"));
        }

        #[test]
        fn rejects_monster_suffix() {
            let desc = "You can add 1 \"Foo\" monster from your Deck to your hand.";
            assert!(match_archetype_spell_trap_desc(desc).is_none());
        }

        #[test]
        fn rejects_card_suffix() {
            let desc = "You can add 1 \"Foo\" card from your Deck to your hand.";
            assert!(match_archetype_spell_trap_desc(desc).is_none());
        }
    }

    // ── M.8 / TTT-II inline tests for trigger-agnostic archetype
    //    monster + card matchers ──
    #[cfg(test)]
    mod archetype_any_trigger_matcher_tests {
        use super::*;

        #[test]
        fn monster_matches_simple_archetype() {
            let desc = "If this card is Tributed: You can add 1 \"Mitsurugi\" monster from your Deck to your hand.";
            assert_eq!(
                match_archetype_monster_any_desc(desc).as_deref(),
                Some("Mitsurugi")
            );
        }

        #[test]
        fn monster_matches_compound_trigger_anchor() {
            // Aramasa-style compound trigger: anchor-free, so this
            // matches regardless of the particular trigger phrase.
            let desc = "If this card is Normal or Special Summoned, or if this card is Tributed: You can add 1 \"Mitsurugi\" monster from your Deck to your hand.";
            assert_eq!(
                match_archetype_monster_any_desc(desc).as_deref(),
                Some("Mitsurugi")
            );
        }

        #[test]
        fn monster_rejects_spell_trap_suffix() {
            let desc = "You can add 1 \"Foo\" Spell/Trap from your Deck to your hand.";
            assert!(match_archetype_monster_any_desc(desc).is_none());
        }

        #[test]
        fn monster_rejects_card_suffix() {
            let desc = "You can add 1 \"Foo\" card from your Deck to your hand.";
            assert!(match_archetype_monster_any_desc(desc).is_none());
        }

        #[test]
        fn card_matches_simple_archetype() {
            let desc = "When this card is Normal Summoned: You can add 1 \"Archfiend\" card from your Deck to your hand.";
            assert_eq!(
                match_archetype_card_any_desc(desc).as_deref(),
                Some("Archfiend")
            );
        }

        #[test]
        fn card_rejects_monster_suffix() {
            let desc = "You can add 1 \"Foo\" monster from your Deck to your hand.";
            assert!(match_archetype_card_any_desc(desc).is_none());
        }

        #[test]
        fn card_rejects_spell_trap_suffix() {
            let desc = "You can add 1 \"Foo\" Spell/Trap from your Deck to your hand.";
            assert!(match_archetype_card_any_desc(desc).is_none());
        }

        #[test]
        fn monster_lowercase_add_ok() {
            let desc = "When summoned: You can add 1 \"Foo\" monster from your Deck to your hand.";
            assert_eq!(
                match_archetype_monster_any_desc(desc).as_deref(),
                Some("Foo")
            );
        }
    }
}
