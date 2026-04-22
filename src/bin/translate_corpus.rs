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
        // Order matters: the canonical M.0 cluster runs first; M.1
        // sub-clusters follow; M.2 destroyed-trigger sub-cluster last.
        // One rewrite per file (see `break` in `run`), so an earlier
        // cluster wins any overlap.
        let all: Vec<Box<dyn Cluster>> = vec![
            Box::new(RecruiterBattleDamage),
            Box::new(RecruiterBattleDamageArchetype),
            Box::new(RecruiterBattleDamageArchetypeNoStat),
            Box::new(RecruiterBattleDamageNamed),
            Box::new(RecruiterDestroyedArchetypeNoStat),
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

            if !src.contains(old_line) {
                return Err("expected placeholder line not found".into());
            }
            Ok(src.replacen(old_line, &new_line, 1))
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
            if !src.contains(old_line) {
                return Err("expected placeholder line not found".into());
            }
            Ok(src.replacen(old_line, &new_line, 1))
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
            if !src.contains(old_line) {
                return Err("expected placeholder line not found".into());
            }
            Ok(src.replacen(old_line, &new_line, 1))
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
            if !src.contains(old_line) {
                return Err("expected placeholder line not found".into());
            }
            Ok(src.replacen(old_line, &new_line, 1))
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
            if !src.contains(old_line) {
                return Err("expected placeholder line not found".into());
            }
            Ok(src.replacen(old_line, &new_line, 1))
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
        has_placeholder_with_trigger(src, "battle_damage")
    }

    /// Generalised form: return true if `src` contains an effect block with:
    ///   - trigger: <trigger_value>
    ///   - single-line resolve body exactly
    ///     `special_summon (all, card, either controls)`
    ///
    /// The trigger value is matched by prefix (matches M.0/M.1 semantics
    /// where `body_has_line_starting_with` is also prefix-based).
    fn has_placeholder_with_trigger(src: &str, trigger_value: &str) -> bool {
        // Cheap text scan: find each effect block, check trigger + body.
        // We walk effect by effect using brace matching.
        let bytes = src.as_bytes();

        let mut i = 0;
        while i < bytes.len() {
            // Find next "effect \"" start.
            let Some(start) = find_from(src, i, "effect \"") else { break };
            // Advance past the name + opening brace.
            let Some(brace) = src[start..].find('{') else { break };
            let body_start = start + brace + 1;
            // Match the brace.
            let Some(body_end) = match_brace(src, body_start) else { break };
            let body = &src[body_start..body_end];
            i = body_end + 1;

            // Must have the requested trigger.
            if !body_has_line_starting_with(body, "trigger:", trigger_value) {
                continue;
            }
            // Find resolve { ... } inside body.
            let Some(r_brace) = body.find("resolve") else { continue };
            let Some(r_brace_open) = body[r_brace..].find('{') else { continue };
            let rb_start = r_brace + r_brace_open + 1;
            let Some(rb_end) = match_brace(body, rb_start) else { continue };
            let rbody = body[rb_start..rb_end].trim();
            if rbody == "special_summon (all, card, either controls)" {
                return true;
            }
        }
        false
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
}
