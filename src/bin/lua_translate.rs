// ============================================================
// lua_translate — Lua-AST → DSL translator
//
// Walks a Yu-Gi-Oh card's Lua script (e.g. CardScripts/official/cXXXX.lua),
// finds the Effect.CreateEffect / SetOperation chains in `s.initial_effect`,
// and extracts the operation-function bodies to emit DSL effect blocks.
//
// Phase 0: dump-only. Reads one .lua file, prints what it found —
// effects, their categories/triggers, and the Duel.* calls in each
// operation handler. Apply mode comes after the extractor is correct.
//
// Usage:
//
//     cargo run --features lua_ast,cdb --bin lua_translate -- \
//         dump <path/to/cXXXX.lua>
//
// ============================================================

use std::env;
use std::fs;
use std::path::Path;
use std::process;

use duelscript::block_match;
use duelscript::lua_ast;
use duelscript::stub_refine;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: lua_translate <dump|translate|apply|audit-fills|audit-owners> <path>");
        process::exit(2);
    }
    let mode = args[1].as_str();
    let path = &args[2];
    // dump/translate read a single file; apply takes dirs and reads them itself.
    let src = if mode == "dump" || mode == "translate" {
        match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => { eprintln!("cannot read {}: {}", path, e); process::exit(1); }
        }
    } else {
        String::new()
    };
    match mode {
        "dump" => {
            let report = lua_ast::analyze(&src);
            println!("{}", report);
        }
        "translate" => {
            // Parse, walk, and emit one draft DSL `resolve` block per
            // effect that has a SetOperation handler. Uses the same
            // functions-aware translation path and card-name table as
            // apply mode so its output is a faithful per-file preview.
            load_card_names(args.get(3).map(String::as_str),
                Path::new(path).parent().and_then(Path::to_str).unwrap_or("."));
            let parsed = match full_moon::parse(&src) {
                Ok(a) => a,
                Err(e) => { eprintln!("parse error: {:?}", e); process::exit(1); }
            };
            let walk = lua_ast::walk(&parsed);
            for (i, eff) in walk.effects.iter().enumerate() {
                println!("// effect[{}] binding={}", i, eff.binding);
                if let Some(handler) = &eff.operation_handler {
                    if let Some(body) = walk.functions.get(handler.trim()) {
                        let lines = lua_ast::translate_body_with_functions(body, &walk.functions);
                        println!("resolve {{");
                        for l in lines {
                            println!("{}", l.into_string("    "));
                        }
                        println!("}}");
                    } else {
                        println!("// no body found for handler {}", handler);
                    }
                } else {
                    println!("// no SetOperation handler");
                }
                println!();
            }
        }
        "apply" => {
            // Apply mode: walk a corpus directory of .ds files. For each
            // file with empty `resolve { }` blocks, find the matching .lua
            // (./CardScripts/official/cXXXX.lua), translate effects, and
            // inject filled-in resolve bodies. Conservative — only fills
            // when the lua-ast translator emits at least one ACTION line
            // (TODO-only outputs are skipped to avoid corpus pollution).
            //
            //   lua_translate apply <corpus_dir> <lua_dir> [cards.cdb]
            if args.len() < 4 {
                eprintln!("usage: lua_translate apply <corpus_dir> <lua_dir> [cards.cdb]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            load_card_names(args.get(4).map(String::as_str), lua_dir);
            let report = apply(corpus, lua_dir);
            println!("=== lua_translate apply report ===");
            println!("  files scanned:           {}", report.scanned);
            println!("  files with empty resolve: {}", report.had_empty);
            println!("  lua-ast translated:      {}", report.translated);
            println!("  effects filled:          {}", report.effects_filled);
            println!("  blocks injected:         {}", report.blocks_injected);
            println!("  effects skipped (todo):  {}", report.effects_todo_only);
            println!("  effects skipped (incomplete): {}", report.effects_incomplete);
            println!("  effects skipped (no map): {}", report.effects_no_handler);
            println!("  effects skipped (align):  {}", report.effects_alignment_hazard);
            println!("  blocks matched (positional): {}", report.match_positional);
            println!("  blocks matched (rescued):    {}", report.match_rescued);
            println!("  effects skipped (no lua): {}", report.no_lua);
            println!("  choose blocks injected:  {}", report.chooses_injected);
            println!("  passives injected:       {}", report.passives_injected);
            println!("  conditions injected:     {}", report.conditions_injected);
            println!("  costs injected:          {}", report.costs_injected);
            println!("  targets injected:        {}", report.targets_injected);
            println!("  durations retrofitted:   {}", report.durations_retrofitted);
            println!("  retrofit conflicts:      {}", report.retrofit_conflicts);
            println!("  retrofit removal cands:  {}", report.retrofit_removals);
            println!("  lone stubs seen:         {}", report.stubs_seen);
            println!("  stubs refined:           {}", report.stubs_refined);
            for (cat, n) in &report.stubs_refined_by_category {
                println!("    {:22} {}", cat, n);
            }
            println!("  stubs skipped (category mismatch): {}", report.stubs_skipped_category);
            println!("  stubs skipped (gate):    {}", report.stubs_skipped_gate);
            println!("  stubs skipped (lossy translation): {}", report.stubs_skipped_lossy);
            println!("  stubs skipped (unassigned block): {}", report.stubs_unassigned);
            println!("  stubs already exact:     {}", report.stubs_exact);
        }
        "audit-fills" => {
            // T38 S2b — policy-flip retrofit audit: re-derive every
            // shipped (non-empty) resolve under the current translator
            // and report the ones the fill gate now refuses. A hit whose
            // shipped lines byte-match the current rendering is a machine
            // fill the gate has invalidated (`--stub` rewrites those back
            // to empty resolves); mismatches print both sides and stay
            // report-only for hand verification.
            //
            //   lua_translate audit-fills <corpus_dir> <lua_dir> [cards.cdb] [--stub]
            if args.len() < 4 {
                eprintln!("usage: lua_translate audit-fills <corpus_dir> <lua_dir> [cards.cdb] [--stub]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            let stub = args.iter().any(|a| a == "--stub");
            let cdb = args.get(4).map(String::as_str).filter(|a| *a != "--stub");
            load_card_names(cdb, lua_dir);
            audit_fills(corpus, lua_dir, stub);
        }
        "audit-owners" => {
            // T38 S9 backlog (b) — Phase-2-era positional residue: a
            // shipped resolve that byte-matches what the translator
            // renders for a consumer OTHER than the block's assigned
            // owner (`block_match::compute_assignments`). Classified
            // mechanically (`block_match::classify_foreign_fill`):
            // coincidental equality (owner renders the same — clone /
            // sibling pairs) is never touched; owner-unassigned blocks
            // are report-only; true misplacements (owner renders a
            // different or empty body) are emptied by `--stub` so that
            // `apply` can refill the right owner's lines where its gate
            // allows. Hand-verify the misplaced set before stubbing.
            //
            //   lua_translate audit-owners <corpus_dir> <lua_dir> [cards.cdb] [--stub]
            if args.len() < 4 {
                eprintln!("usage: lua_translate audit-owners <corpus_dir> <lua_dir> [cards.cdb] [--stub]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            let stub = args.iter().any(|a| a == "--stub");
            let cdb = args.get(4).map(String::as_str).filter(|a| *a != "--stub");
            load_card_names(cdb, lua_dir);
            audit_owners(corpus, lua_dir, stub);
        }
        _ => {
            eprintln!("unknown mode: {}", mode);
            process::exit(2);
        }
    }
}

/// See the `audit-fills` mode arm. Walks handler-derived effect blocks
/// only — summon-helper and replacement-chain fills don't flow through
/// `body_drops_chains`, so the gate flip can't invalidate them.
fn audit_fills(corpus_dir: &str, lua_dir: &str, stub: bool) {
    let mut paths: Vec<std::path::PathBuf> = match fs::read_dir(corpus_dir) {
        Ok(e) => e.flatten().map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
            .collect(),
        Err(e) => { eprintln!("cannot read {}: {}", corpus_dir, e); return; }
    };
    paths.sort();
    let (mut stubs, mut reviews) = (0usize, 0usize);
    for path in paths {
        let Ok(txt) = fs::read_to_string(&path) else { continue };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let lua_path = Path::new(lua_dir).join(format!("{}.lua", stem));
        let Ok(lua_src) = fs::read_to_string(&lua_path) else { continue };
        let Ok(parsed) = full_moon::parse(&lua_src) else { continue };
        let walk = lua_ast::walk(&parsed);
        if walk.effects.is_empty() { continue; }
        let assign = block_match::compute_assignments(&walk, &txt);
        let mut new_txt = txt.clone();
        let mut changed = false;
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            if eff.is_summon_helper() || eff.is_replacement_chain() { continue; }
            let Some(handler) = eff.operation_handler.as_deref() else { continue };
            let handler = handler.trim();
            let Some(block_idx) = assign.by_effect[eff_i] else { continue };
            let Some(body) = walk.functions.get(handler) else { continue };
            if !lua_ast::body_drops_chains(body, &walk.functions) { continue; }
            // Offsets recomputed against new_txt — a stub earlier in this
            // file shifts every later block.
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else { continue };
            let Some((inner_lo, inner_hi)) = block_match::resolve_inner_span(&new_txt, block_lo, block_hi) else { continue };
            let shipped: Vec<String> = new_txt[inner_lo..inner_hi].lines()
                .map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
            if shipped.is_empty() { continue; } // already a stub
            let lines = lua_ast::translate_body_with_functions(body, &walk.functions);
            let rendered: Vec<String> = lines.iter().filter(|l| l.is_action())
                .map(|l| l.clone().into_string("").trim().to_string()).collect();
            let reason = if body.method_ops_poisoned { "method-poison" }
                else if body.counter_ops_poisoned { "counter-poison" }
                else { "drop" };
            if shipped == rendered {
                stubs += 1;
                println!("STUB   {} block={} handler={} reason={} lines={}",
                    stem, block_idx, handler, reason, shipped.len());
                for l in &shipped { println!("    - {}", l); }
                if stub {
                    new_txt = format!("{}\n        {}", &new_txt[..inner_lo], &new_txt[inner_hi..]);
                    changed = true;
                }
            } else {
                reviews += 1;
                println!("REVIEW {} block={} handler={} reason={}", stem, block_idx, handler, reason);
                for l in &shipped  { println!("    shipped:  {}", l); }
                for l in &rendered { println!("    rendered: {}", l); }
            }
        }
        if changed {
            if let Err(e) = fs::write(&path, &new_txt) {
                eprintln!("cannot write {}: {}", path.display(), e);
            }
        }
    }
    println!("=== audit-fills: {} stub, {} review ===", stubs, reviews);
}

/// See the `audit-owners` mode arm. Walks every effect block with a
/// non-empty resolve; the classification and the per-consumer rendering
/// live in `block_match` so the verdicts are unit-testable.
fn audit_owners(corpus_dir: &str, lua_dir: &str, stub: bool) {
    use block_match::ForeignFillClass as C;
    let mut paths: Vec<std::path::PathBuf> = match fs::read_dir(corpus_dir) {
        Ok(e) => e.flatten().map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
            .collect(),
        Err(e) => { eprintln!("cannot read {}: {}", corpus_dir, e); return; }
    };
    paths.sort();
    let (mut coincidental, mut unassigned, mut suspect, mut differs, mut empty) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut cards = std::collections::BTreeSet::new();
    let mut stubbed = 0usize;
    for path in paths {
        let Ok(txt) = fs::read_to_string(&path) else { continue };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let lua_path = Path::new(lua_dir).join(format!("{}.lua", stem));
        let Ok(lua_src) = fs::read_to_string(&lua_path) else { continue };
        let Ok(parsed) = full_moon::parse(&lua_src) else { continue };
        let walk = lua_ast::walk(&parsed);
        if walk.effects.is_empty() { continue; }
        let assign = block_match::compute_assignments(&walk, &txt);
        let hits = block_match::foreign_fills(&walk, &assign, &txt);
        if hits.is_empty() { continue; }
        let n_blocks = block_match::effect_block_ranges(&txt).len();
        let n_consumers = walk.effects.iter()
            .filter(|e| e.is_summon_helper() || e.operation_handler.is_some()).count();
        cards.insert(stem.clone());
        let mut new_txt = txt.clone();
        let mut changed = false;
        for h in &hits {
            let tag = match h.class {
                C::Coincidental => { coincidental += 1; "KEEP   " }
                C::OwnerUnassigned => { unassigned += 1; "NOOWNER" }
                C::MisplacedOwnerDiffers => { differs += 1; "MISPLACED-DIFFERS" }
                C::MisplacedOwnerEmpty => { empty += 1; "MISPLACED-EMPTY" }
                C::AlignmentSuspect => { suspect += 1; "SUSPECT" }
            };
            let matched: Vec<String> = h.matches.iter()
                .map(|&i| walk.effects[i].binding.clone()).collect();
            let owner = h.owner.map(|o| walk.effects[o].binding.as_str()).unwrap_or("-");
            let owner_handler = h.owner
                .and_then(|o| walk.effects[o].operation_handler.as_deref())
                .map(str::trim)
                .map(|h| if h.starts_with("function") { "<closure>" } else { h })
                .unwrap_or("-");
            let header = match (h.owner_header_score, h.owner_header_veto) {
                (None, _) => "-".to_string(),
                (Some(_), true) => "veto".to_string(),
                (Some(sc), false) => sc.to_string(),
            };
            println!("{} {} block={} owner={} handler={} header={} blocks={} consumers={} matches={}",
                tag, stem, h.block, owner, owner_handler, header, n_blocks, n_consumers, matched.join(","));
            for l in &h.shipped { println!("    shipped: {}", l); }
            if let Some(o) = h.owner {
                if h.owner_rendering.is_empty() {
                    println!("    owner:   (renders nothing)");
                }
                for l in &h.owner_rendering { println!("    owner:   {}", l); }
                // The owner's untranslated remainder — the hand-verifier's
                // view of what else its handler does.
                if let Some(body) = walk.effects[o].operation_handler.as_deref()
                    .and_then(|hn| walk.functions.get(hn.trim()))
                {
                    for l in lua_ast::translate_body_with_functions(body, &walk.functions) {
                        if !l.is_action() { println!("    owner-todo: {}", l.into_string("").trim()); }
                    }
                }
            } else {
                println!("    owner:   (unassigned)");
            }
            if stub && h.class.is_misplaced() {
                // Offsets recomputed against new_txt — a stub earlier in
                // this file shifts every later block.
                let Some((lo, hi)) = block_match::effect_block_ranges(&new_txt).into_iter().nth(h.block) else { continue };
                let Some((ilo, ihi)) = block_match::resolve_inner_span(&new_txt, lo, hi) else { continue };
                new_txt = format!("{}\n        {}", &new_txt[..ilo], &new_txt[ihi..]);
                changed = true;
                stubbed += 1;
            }
        }
        if changed {
            if let Err(e) = fs::write(&path, &new_txt) {
                eprintln!("cannot write {}: {}", path.display(), e);
            }
        }
    }
    println!("=== audit-owners: {} blocks / {} cards — {} coincidental (keep), {} owner-unassigned (report), {} alignment-suspect (report), {} misplaced/owner-differs, {} misplaced/owner-empty; {} stubbed ===",
        coincidental + unassigned + suspect + differs + empty, cards.len(),
        coincidental, unassigned, suspect, differs, empty, stubbed);
}

/// Populate the lua_ast passcode → card-name table used by the
/// EFFECT_CHANGE_CODE → `change_name` translation (Phase 11).
///
/// Uses the explicit `[cards.cdb]` CLI arg when given; otherwise probes
/// the workspace-sibling convention `<lua_dir>/../../BabelCdb/cards.cdb`
/// (lua_dir is CardScripts/official, BabelCdb is its repo sibling).
/// Missing or unreadable cdb is non-fatal — change-code chains simply
/// skip when no name resolves.
fn load_card_names(cli_path: Option<&str>, lua_dir: &str) {
    let path = match cli_path {
        Some(p) => std::path::PathBuf::from(p),
        None => Path::new(lua_dir).join("../../BabelCdb/cards.cdb"),
    };
    if !path.exists() {
        eprintln!("note: no cards.cdb at {} — change_name translation disabled", path.display());
        return;
    }
    match duelscript::cdb::CdbReader::open(&path) {
        Ok(cdb) => {
            lua_ast::register_card_names(
                cdb.all_cards().into_iter().map(|c| (c.id as u32, c.name.clone())),
            );
            // Phase 16 — strs feed choose-option labels (aux.Stringid).
            lua_ast::register_card_strings(
                cdb.all_cards().into_iter().map(|c| (c.id as u32, c.strings.clone())),
            );
            // T38 Pass S — card text backs the rider-line evidence check.
            let _ = CARD_DESCS.set(
                cdb.all_cards().into_iter().map(|c| (c.id as u32, c.desc.clone())).collect(),
            );
        }
        Err(e) => eprintln!("note: cannot read {}: {:?} — change_name translation disabled", path.display(), e),
    }
}

#[derive(Default)]
struct ApplyReport {
    scanned: usize,
    had_empty: usize,
    translated: usize,
    effects_filled: usize,
    blocks_injected: usize,
    effects_todo_only: usize,
    effects_incomplete: usize,
    effects_no_handler: usize,
    effects_alignment_hazard: usize,
    match_positional: usize,
    match_rescued: usize,
    chooses_injected: usize,
    passives_injected: usize,
    conditions_injected: usize,
    costs_injected: usize,
    targets_injected: usize,
    durations_retrofitted: usize,
    retrofit_conflicts: usize,
    retrofit_removals: usize,
    no_lua: usize,
    // T38 stub refinement (Pass S).
    stubs_seen: usize,
    stubs_refined: usize,
    stubs_refined_by_category: std::collections::BTreeMap<&'static str, usize>,
    stubs_skipped_category: usize,
    stubs_skipped_gate: usize,
    stubs_unassigned: usize,
    stubs_exact: usize,
    stubs_skipped_lossy: usize,
}

/// Card passcode → full card text (`desc`), from the cdb — Pass S's
/// rider evidence (`stub_refine::category_match`). Absent without a
/// cdb, which admits no rider lines (conservative).
static CARD_DESCS: std::sync::OnceLock<std::collections::BTreeMap<u32, String>> =
    std::sync::OnceLock::new();

fn card_desc(card_id: Option<u32>) -> Option<&'static str> {
    CARD_DESCS.get()?.get(&card_id?).map(String::as_str)
}

/// Why Pass A would (not) fill an effect's resolve — ONE decision path
/// shared by Pass A (empty resolves), Pass A2 (blocks without a
/// resolve) and Pass S (lone category stubs, T38), so the stub
/// refinement can never admit a fill the empty-resolve path refuses.
enum FillPlan {
    /// Pure passive, undecodable helper, or unknown handler body —
    /// nothing to emit (`effects_no_handler`).
    NoHandler,
    /// Replacement chain, dropped chain, TODO-mixed body, or a header
    /// parity / bare-target gate failure (`effects_incomplete`).
    Incomplete,
    /// No unambiguous block for this effect (`effects_alignment_hazard`).
    Hazard,
    /// Translator emitted no action line (`effects_todo_only`).
    TodoOnly,
    /// The assigned block index doesn't exist in the text.
    NoBlock,
    /// Phase 16 — a `choose { … }` block replaces the resolve.
    Choose(String),
    /// A complete fill: the action lines plus, when they reference
    /// `target` and the block has no declaration, the co-emitted
    /// target selector.
    Fill { lines: Vec<lua_ast::DslLine>, co_target: Option<String> },
}

fn plan_fill(
    eff_i: usize,
    eff: &lua_ast::EffectSkeleton,
    walk: &lua_ast::LuaReport,
    assign: &block_match::Assignments,
    txt: &str,
    card_id: Option<u32>,
) -> FillPlan {
    if !eff.is_summon_helper() && eff.operation_handler.is_none() {
        return FillPlan::NoHandler; // pure-passive — no effect block in .ds
    }
    // T38 S4 — replacement-code chains (EFFECT_*_REPLACE): the op body
    // belongs to the corpus-generated `replacement` block; filling an
    // effect-block resolve with it duplicates the semantics in the
    // wrong surface (c39996157's shipped `banish self`). Consume the
    // index, decline the fill.
    if eff.is_replacement_chain() {
        return FillPlan::Incomplete;
    }
    let Some(block_idx) = assign.by_effect[eff_i] else {
        // Hazard-gated and not rescued: a clone / bare-activate chain
        // owns a .ds block before this one (or a continuous listener
        // consumed an index without owning a block — T38 S9) and the
        // block signatures don't force a unique home — filling would
        // risk landing in the wrong block.
        return FillPlan::Hazard;
    };
    // Phase 16 — SelectOption label-branch dispatch. When the target
    // handler picked an option (`Duel.SelectOption` + `e:SetLabel`)
    // and the operation handler splits on `e:GetLabel()` with one
    // fully-translatable arm per option, a `choose { option … }` block
    // REPLACES the empty resolve (the validator accepts either, never
    // both). Falls through to the normal fill when the shape or a
    // label doesn't resolve.
    if !eff.is_summon_helper() {
        let choose_text = eff.operation_handler.as_deref().and_then(|oh| {
            let ob = walk.functions.get(oh.trim())?;
            let tb = eff.target_handler.as_deref()
                .and_then(|th| walk.functions.get(th.trim()));
            let spec = lua_ast::extract_choose_spec(tb, ob, &walk.functions)?;
            render_choose_block(&spec, card_id?)
        });
        if let Some(choose_text) = choose_text {
            return FillPlan::Choose(choose_text);
        }
    }
    // Special case: skeletons backed by a fusion/ritual summon helper —
    // the plain factories (Fusion.CreateSummonEff / Ritual.AddProc* /
    // Ritual.CreateProc, fixed line) and the parameterized SetOperation
    // forms (Fusion.SummonEffOP / Ritual.Operation, Phase 12). The
    // helper owns the UI / op pipeline, so a summon line replaces the
    // handler walk. Parameterized forms whose params don't decode
    // return None and fall through to the no-handler skip.
    let helper_line = eff.summon_helper_line();
    let is_helper_fill = helper_line.is_some();
    let lines: Vec<lua_ast::DslLine> = if let Some(text) = helper_line {
        vec![lua_ast::DslLine::Action(text)]
    } else if eff.is_summon_helper() {
        // Helper params undecodable — the activation block exists in
        // the .ds but its resolve stays an empty stub (skip-not-mis-emit).
        return FillPlan::NoHandler;
    } else {
        // Consumer without a helper spec has an operation handler
        // (checked at the top).
        let handler = eff.operation_handler.as_deref().unwrap_or_default();
        match walk.functions.get(handler.trim()) {
            Some(body) => {
                // T38 S1 — completeness gate: a handler whose
                // translation drops a RegisterEffect chain would fill
                // an under-stated resolve. Skip the whole fill; the
                // dropped chain is the S1b/S8 backlog.
                if lua_ast::body_drops_chains(body, &walk.functions) {
                    return FillPlan::Incomplete;
                }
                lua_ast::translate_body_with_functions(body, &walk.functions)
            }
            None => return FillPlan::NoHandler, // block exists, body unknown
        }
    };
    if !lines.iter().any(|l| l.is_action()) {
        return FillPlan::TodoOnly;
    }
    // T38 S1 — a TODO line marks an untranslated Duel.* call in the
    // same handler; filling only the translated remainder silently
    // drops it. Skip-not-mis-emit.
    if lines.iter().any(|l| !l.is_action()) {
        return FillPlan::Incomplete;
    }
    let Some((block_lo, block_hi)) = nth_effect_block(txt, block_idx) else {
        return FillPlan::NoBlock;
    };
    let block = &txt[block_lo..block_hi];
    // T38 S6 review — cost-header parity: filling a resolve makes the
    // block's header LIVE, so a block-side cost must re-derive exactly
    // from the lua cost handler (skip class 10 — c9798352's `banish
    // self` header contradicts the lua's banish-a-Trap-from-Deck cost).
    if !cost_header_matches(block, eff, &walk.functions) {
        return FillPlan::Incomplete;
    }
    // T38 S5 review — frame parity for summon-helper fills.
    if is_helper_fill && !helper_frame_matches(block, eff) {
        return FillPlan::Incomplete;
    }
    // T38 S6 — bare-target gate: a fill referencing `target` only lands
    // together with a target declaration; otherwise the validator's
    // bare-target check trades the empty-resolve error for a new one.
    // Blocks without one get the decl CO-EMITTED from the SetTarget
    // selector (refined extraction — exactly-mapped filters only);
    // when that fails, the whole fill skips.
    let mut co_target: Option<String> = None;
    if lines_reference_target(&lines) && !block_has_target_decl(block) {
        match target_decl_selector(eff, &walk.functions) {
            Some(sel) => co_target = Some(sel),
            None => return FillPlan::Incomplete,
        }
    }
    FillPlan::Fill { lines, co_target }
}

/// The text that replaces an empty / stub `resolve { }` slot: the
/// rendered body, preceded by the co-emitted target declaration when
/// the plan carries one (8-space block indent).
fn render_fill(lines: &[lua_ast::DslLine], co_target: Option<&str>) -> String {
    let body = render_resolve_body(lines);
    match co_target {
        Some(sel) => format!("target {}\n        resolve {{\n{}        }}", sel, body),
        None => format!("resolve {{\n{}        }}", body),
    }
}

/// The pre-fix substring mapping `reset_to_duration_kw` shipped with —
/// kept ONLY so Pass R can locate the durations earlier applies emitted.
/// Not a translation path; do not add cases.
fn legacy_reset_duration_kw(reset: Option<&str>) -> Option<&'static str> {
    let s = reset?;
    if s.contains("PHASE_END") || s.contains("RESETS_STANDARD") {
        return Some("end_of_turn");
    }
    if s.contains("PHASE_DAMAGE") {
        return Some("end_of_damage_step");
    }
    None
}

/// Rewrite every line in `block` whose trailing token is the duration
/// keyword `from` to end in `to` instead. Returns the rewritten block
/// and the number of lines changed. `install_watcher` lines never end
/// in a bare duration (they close with `} }`), so the trailing match
/// leaves them alone by construction.
fn rewrite_trailing_duration(block: &str, from: &str, to: &str) -> (String, usize) {
    let mut n = 0usize;
    let mut out: Vec<String> = Vec::new();
    for line in block.split('\n') {
        let t = line.trim_end();
        let suffix = format!(" {}", from);
        if t.ends_with(&suffix) {
            n += 1;
            out.push(format!("{}{}", &t[..t.len() - from.len()], to));
        } else {
            out.push(line.to_string());
        }
    }
    (out.join("\n"), n)
}

fn apply(corpus_dir: &str, lua_dir: &str) -> ApplyReport {
    let mut r = ApplyReport::default();
    let entries = match fs::read_dir(corpus_dir) {
        Ok(e) => e,
        Err(e) => { eprintln!("cannot read {}: {}", corpus_dir, e); return r; }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("ds") { continue; }
        r.scanned += 1;
        let txt = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let had_empty = has_empty_resolve(&txt);
        if had_empty { r.had_empty += 1; }

        // Match to lua via filename stem (cXXXX.ds → cXXXX.lua)
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let lua_path = Path::new(lua_dir).join(format!("{}.lua", stem));
        let lua_src = match fs::read_to_string(&lua_path) {
            Ok(s) => s,
            Err(_) => {
                if had_empty { r.no_lua += 1; }
                continue;
            }
        };
        let parsed = match full_moon::parse(&lua_src) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let walk = lua_ast::walk(&parsed);
        if walk.effects.is_empty() { continue; }
        r.translated += 1;

        // Phase 20 — signature-based handler→block matching. Hazard-free
        // cards get the historical positional mapping (the i-th index-
        // consuming walk effect → the i-th `effect "Effect N"` block);
        // hazard-gated effects fill only when the matcher forces an
        // unambiguous, order-consistent block for them. All passes below
        // consult the same per-effect assignment.
        let assign = block_match::compute_assignments(&walk, &txt);
        r.match_positional += assign.positional;
        r.match_rescued += assign.rescued;

        // T38 Pass S census — lone category stubs in blocks no effect
        // is assigned to. Never rewritten (there is no owner handler to
        // derive a fill from); reported so the backlog stays visible.
        for (idx, (lo, hi)) in block_match::effect_block_ranges(&txt).into_iter().enumerate() {
            if assign.by_effect.iter().any(|b| *b == Some(idx)) { continue; }
            if txt[lo..hi].contains("choose {") { continue; }
            if let Some((ilo, ihi)) = block_match::resolve_inner_span(&txt, lo, hi) {
                if stub_refine::lone_stub_category(&txt[ilo..ihi]).is_some() {
                    r.stubs_unassigned += 1;
                }
            }
        }

        let mut new_txt = txt.clone();
        let mut filled = 0usize;
        // Card passcode from the filename stem (`c1006081` → 1006081) —
        // Phase 16 choose-option labels resolve via the card's own strs.
        let card_id: Option<u32> = stem.strip_prefix('c').and_then(|s| s.parse().ok());

        // Pass A — fill empty resolve blocks via translated handler bodies.
        // Each fill targets the empty resolve INSIDE the block assigned to
        // that effect. Filling "the first empty resolve anywhere" instead
        // would, on rerun, inject an earlier effect's lines into a later
        // untranslatable effect's still-empty resolve — wrong content and
        // non-idempotent. The fill decision itself is `plan_fill`, shared
        // with Pass A2 and Pass S.
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            let (lines, co_target) = match plan_fill(eff_i, eff, &walk, &assign, &new_txt, card_id) {
                FillPlan::NoHandler => { r.effects_no_handler += 1; continue; }
                FillPlan::Incomplete => { r.effects_incomplete += 1; continue; }
                FillPlan::Hazard => { r.effects_alignment_hazard += 1; continue; }
                FillPlan::TodoOnly => { r.effects_todo_only += 1; continue; }
                FillPlan::NoBlock => continue,
                FillPlan::Choose(choose_text) => {
                    let Some(block_idx) = assign.by_effect[eff_i] else { continue };
                    if let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) {
                        if let Some((lo, hi)) = first_empty_resolve_within(&new_txt, block_lo, block_hi) {
                            new_txt = format!("{}{}{}", &new_txt[..lo], choose_text, &new_txt[hi..]);
                            filled += 1;
                            r.chooses_injected += 1;
                        } else if !new_txt[block_lo..block_hi].contains("resolve")
                            && !new_txt[block_lo..block_hi].contains("choose")
                        {
                            // Block lacks a resolve slot entirely (the
                            // "must have a resolve or choose" class) —
                            // inject before the closing brace, Pass A2
                            // style. Idempotent: the injected choose
                            // makes this branch unreachable on rerun.
                            let close_brace = block_hi - 1;
                            let mut inject_pos = close_brace;
                            let bytes = new_txt.as_bytes();
                            while inject_pos > block_lo && bytes[inject_pos - 1].is_ascii_whitespace() {
                                inject_pos -= 1;
                            }
                            let injection = format!("\n        {}", choose_text);
                            new_txt = format!("{}{}{}", &new_txt[..inject_pos], injection, &new_txt[inject_pos..]);
                            filled += 1;
                            r.chooses_injected += 1;
                        }
                    }
                    continue;
                }
                FillPlan::Fill { lines, co_target } => (lines, co_target),
            };
            let Some(block_idx) = assign.by_effect[eff_i] else { continue };
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else { continue };
            if let Some((lo, hi)) = first_empty_resolve_within(&new_txt, block_lo, block_hi) {
                let injection = render_fill(&lines, co_target.as_deref());
                new_txt = format!("{}{}{}", &new_txt[..lo], injection, &new_txt[hi..]);
                filled += 1;
                if co_target.is_some() {
                    r.targets_injected += 1;
                }
            }
        }

        // Pass A2 — inject a fresh `resolve { … }` into effect blocks that
        // lack one entirely (validator: "must have a resolve or choose
        // block"). Same decision path as Pass A (`plan_fill`); only a
        // complete fill lands, so blocks without translator coverage stay
        // untouched. Effect blocks come from the shared Phase 20
        // assignment (pure-passive lua chains have no effect block and
        // are skipped upstream).
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            let FillPlan::Fill { lines, co_target } =
                plan_fill(eff_i, eff, &walk, &assign, &new_txt, card_id)
            else { continue };
            let Some(block_idx) = assign.by_effect[eff_i] else { continue };
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else { continue };
            let block = &new_txt[block_lo..block_hi];
            // Skip if the block already has a resolve or choose — Pass A
            // handles those, and we don't want to double-inject.
            if block.contains("resolve") || block.contains("choose") { continue; }

            // Inject right before the block's closing `}`, after any
            // trailing whitespace, so the new resolve nests inside the
            // effect block with the standard 8-space indent.
            let close_brace = block_hi - 1; // points at `}`
            let mut inject_pos = close_brace;
            let bytes = new_txt.as_bytes();
            while inject_pos > block_lo && bytes[inject_pos - 1].is_ascii_whitespace() {
                inject_pos -= 1;
            }
            let injection = format!("\n        {}", render_fill(&lines, co_target.as_deref()));
            new_txt = format!("{}{}{}", &new_txt[..inject_pos], injection, &new_txt[inject_pos..]);
            filled += 1;
            if co_target.is_some() {
                r.targets_injected += 1;
            }
        }

        // Pass A3 — synthesize the whole activation block for bare card
        // stubs. Cards whose lua is a lone self-registering summon helper
        // (`Fusion.RegisterSummonEff(c, ...)`) never got a skeleton
        // `effect "Effect 1"` block at corpus generation, so Pass A / A2
        // have nothing to fill. When the .ds has NO effect blocks and the
        // walk yields exactly one summon-helper skeleton whose params
        // decode to a line, inject the standard activation block. Helpers
        // whose params don't decode stay bare stubs (skip-not-mis-emit).
        // Idempotent: the injected block makes the no-effect-blocks guard
        // fail on rerun.
        let mut blocks_added = 0usize;
        if nth_effect_block(&new_txt, 0).is_none() && walk.effects.len() == 1 {
            let eff = &walk.effects[0];
            if eff.is_summon_helper() {
                if let Some(line) = eff.summon_helper_line() {
                    if let Some(pos) = card_close_brace(&new_txt) {
                        let block = format!(
                            "    effect \"Effect 1\" {{\n        speed: 1\n        mandatory\n        resolve {{\n            {}\n        }}\n    }}",
                            line,
                        );
                        let prefix = new_txt[..pos].trim_end().to_string();
                        let suffix = &new_txt[pos..];
                        new_txt = format!("{}\n\n{}\n{}", prefix, block, suffix);
                        blocks_added += 1;
                    }
                }
            }
        }

        // Pass S — T38 stub refinement. The corpus generator left one
        // placeholder category line per effect (`special_summon (all,
        // card, either controls)`, …) in resolves it could not
        // translate; they validate, so Pass A never revisits them. A
        // block whose resolve body is EXACTLY one such line is refilled
        // when (1) the owner handler yields a complete fill under every
        // Pass A gate (`plan_fill` — hazard-gated effects have no
        // assignment and never reach here) and (2) the rendered lines
        // state the stub's category (`stub_refine::category_match`:
        // synonym table + card-text-evidenced riders; anything else
        // skips). Byte-idempotent by construction — a refined body is no
        // longer a lone stub. Blocks the matcher left unassigned are
        // never visited (counted above as `stubs_unassigned`); choose-
        // bearing blocks and multi-line stub bodies are out of scope.
        let mut refined = 0usize;
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            let Some(block_idx) = assign.by_effect[eff_i] else { continue };
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else { continue };
            if new_txt[block_lo..block_hi].contains("choose {") { continue; }
            let Some((inner_lo, inner_hi)) = block_match::resolve_inner_span(&new_txt, block_lo, block_hi) else { continue };
            let Some(stub_cat) = stub_refine::lone_stub_category(&new_txt[inner_lo..inner_hi]) else { continue };
            r.stubs_seen += 1;
            let FillPlan::Fill { lines, co_target } =
                plan_fill(eff_i, eff, &walk, &assign, &new_txt, card_id)
            else {
                r.stubs_skipped_gate += 1;
                continue;
            };
            let rendered: Vec<String> = lines.iter().filter(|l| l.is_action())
                .map(|l| l.clone().into_string("").trim().to_string()).collect();
            if !stub_refine::category_match(stub_cat, &rendered, card_desc(card_id)) {
                r.stubs_skipped_category += 1;
                eprintln!("stub category mismatch ({}): block {} stub={} rendered={:?}",
                    path.display(), block_idx, stub_cat.name(), rendered);
                continue;
            }
            // Hand-review refusals: calls whose current translation is
            // known lossy (see `stub_refine::lossy_call`) — Pass A's
            // policy on empty resolves is untouched; a stub rewrite
            // must not add more instances.
            let lossy = eff.operation_handler.as_deref()
                .and_then(|h| walk.functions.get(h.trim()))
                .and_then(|b| b.calls.iter()
                    .find_map(|c| stub_refine::lossy_call(&c.method, &c.args)));
            if let Some(why) = lossy {
                r.stubs_skipped_lossy += 1;
                eprintln!("stub lossy translation ({}): block {} stub={} why={} rendered={:?}",
                    path.display(), block_idx, stub_cat.name(), why, rendered);
                continue;
            }
            // The owner's complete fill IS the generator's spelling
            // (a genuine "all cards either player controls" selector):
            // the stub was exact all along — nothing to rewrite, and
            // counting it as a refinement would make every re-run
            // report phantom work.
            if rendered.len() == 1 && rendered[0] == new_txt[inner_lo..inner_hi].trim() {
                r.stubs_exact += 1;
                continue;
            }
            // `resolve_inner_span` found the literal `resolve {`, so the
            // keyword starts exactly 9 bytes before the body.
            let resolve_lo = inner_lo - "resolve {".len();
            let injection = render_fill(&lines, co_target.as_deref());
            new_txt = format!("{}{}{}", &new_txt[..resolve_lo], injection, &new_txt[inner_hi + 1..]);
            refined += 1;
            *r.stubs_refined_by_category.entry(stub_cat.name()).or_default() += 1;
            if co_target.is_some() {
                r.targets_injected += 1;
            }
        }

        // Pass B — Phase 5 passive injection. For every effect skeleton
        // whose chain is a stat-modifier passive (no SetOperation
        // / SetTarget / SetCondition / SetCost; literal SetValue or a
        // T34 overlay/counter closure), emit a `passive { … }` block
        // before the card's closing brace. Skips chains whose DSL
        // text already exists in the file (avoids duplicate injection on
        // re-runs of the apply tool).
        let mut passives_added = 0usize;
        let mut passive_idx = next_passive_index(&new_txt);
        for eff in &walk.effects {
            let Some(spec) = eff.passive_modifier_spec() else { continue };
            let name = format!("Passive {}", passive_idx);
            let block = spec.to_dsl_block(&name, "    ");
            // Skip if a passive with the exact same modifier line already
            // exists — prevents double-injection on re-runs.
            if new_txt.contains(&spec.modifier_line()) { continue; }
            if let Some(pos) = card_close_brace(&new_txt) {
                new_txt = format!("{}\n\n{}\n{}", &new_txt[..pos], block, &new_txt[pos..]);
                passives_added += 1;
                passive_idx += 1;
            } else {
                break;
            }
        }

        // Pass C — Phase 6 condition injection. For each active effect (with an
        // operation handler) that also has a translatable condition handler, inject
        // `condition: <dsl_expr>` before `resolve {` in the matching .ds block.
        // The block comes from the shared Phase 20 assignment.
        let mut conditions_added = 0usize;
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            if eff.operation_handler.is_none() && !eff.is_summon_helper() {
                // Purely passive — no corresponding effect block in .ds.
                continue;
            }
            // T38 S4 — replacement-code chains: see Pass A.
            if eff.is_replacement_chain() { continue; }
            let Some(effect_block_idx) = assign.by_effect[eff_i] else {
                continue; // see Pass A — no unambiguous block for this effect
            };

            let cond_handler = match &eff.condition_handler {
                Some(h) => h.trim().to_string(),
                None => continue,
            };
            let cond_body = match walk.functions.get(&cond_handler) {
                Some(b) => b,
                None => continue,
            };
            let dsl_expr = match lua_ast::extract_condition_expr(cond_body) {
                Some(e) => e,
                None => continue,
            };
            if let Some(pos) = condition_inject_pos(&new_txt, effect_block_idx) {
                // Insert `condition: <expr>\n` with 8-space indent before `resolve {`.
                let injection = format!("condition: {}\n        ", dsl_expr);
                new_txt = format!("{}{}{}", &new_txt[..pos], injection, &new_txt[pos..]);
                conditions_added += 1;
            }
        }

        // Pass D — Phase 7 cost injection. For each active effect (with an
        // operation handler) that also has a translatable cost handler, inject
        // a `cost { … }` block before `resolve {` in the matching .ds block.
        // Inserts after any existing `condition:` line (Pass C already runs above).
        // Idempotent — skips effects whose .ds block already contains `cost {`.
        let mut costs_added = 0usize;
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            if eff.operation_handler.is_none() && !eff.is_summon_helper() {
                continue; // purely passive — no effect block in .ds
            }
            // T38 S4 — replacement-code chains: see Pass A.
            if eff.is_replacement_chain() { continue; }
            let Some(effect_block_idx) = assign.by_effect[eff_i] else {
                continue; // see Pass A — no unambiguous block for this effect
            };

            let cost_handler = match &eff.cost_handler {
                Some(h) => h.trim().to_string(),
                None => continue,
            };
            let spec = match lua_ast::extract_cost_block(&cost_handler, &walk.functions) {
                Some(s) => s,
                None => continue,
            };
            if let Some(pos) = cost_inject_pos(&new_txt, effect_block_idx) {
                let block = spec.to_dsl_block("        ");
                let injection = format!("{}\n        ", block);
                new_txt = format!("{}{}{}", &new_txt[..pos], injection, &new_txt[pos..]);
                costs_added += 1;
            }
        }

        // Pass E — Phase 8 target injection. For each active effect (with an
        // operation handler) that also has a translatable target handler, inject
        // a `target <selector>` line before `cost {` or `resolve {` in the
        // matching .ds block. Idempotent — skips blocks that already contain
        // a target declaration.
        let mut targets_added = 0usize;
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            if eff.operation_handler.is_none() && !eff.is_summon_helper() {
                continue; // purely passive — no effect block in .ds
            }
            // T38 S4 — replacement-code chains: see Pass A.
            if eff.is_replacement_chain() { continue; }
            let Some(effect_block_idx) = assign.by_effect[eff_i] else {
                continue; // see Pass A — no unambiguous block for this effect
            };

            let tgt_handler = match &eff.target_handler {
                Some(h) => h.trim().to_string(),
                None => continue,
            };
            let spec = match lua_ast::extract_target_decl(&tgt_handler, &walk.functions) {
                Some(s) => s,
                None => continue,
            };
            if let Some(pos) = target_inject_pos(&new_txt, effect_block_idx) {
                let injection = format!("target {}\n        ", spec.to_dsl());
                new_txt = format!("{}{}{}", &new_txt[..pos], injection, &new_txt[pos..]);
                targets_added += 1;
            }
        }

        // Pass R — duration retrofit for the exact-token reset mapping.
        // Earlier applies emitted durations through a substring mapping
        // that sent every RESETS_STANDARD reset to `end_of_turn`; the
        // exact-token `reset_to_duration_kw` distinguishes while_face_up /
        // next_standby_phase / end_of_damage_step lifetimes. Rewrite the
        // duration keyword on lines those earlier applies produced.
        //
        // Safety filter — rewrites happen per effect block and only when
        // unambiguous: every chain in the block's handler whose LEGACY
        // keyword is K must map to the same new keyword K' under the new
        // mapping, and no chain may still legitimately produce K. Blocks
        // where chains disagree are skipped and counted; chains whose
        // line the new mapping would not emit at all (two-turn lifetimes,
        // battle-phase resets, …) need line REMOVAL, which is left to a
        // manual audit — rewriting their keyword would still be wrong.
        let mut retrofitted = 0usize;
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            let Some(handler) = eff.operation_handler.as_deref() else { continue };
            let Some(block_idx) = assign.by_effect[eff_i] else { continue };
            let Some(body) = walk.functions.get(handler.trim()) else { continue };
            let mut moves: std::collections::BTreeMap<&str, std::collections::BTreeSet<Option<&str>>> =
                std::collections::BTreeMap::new();
            for ch in &body.register_chains {
                let Some(legacy) = legacy_reset_duration_kw(ch.reset.as_deref()) else { continue };
                let new = lua_ast::reset_to_duration_kw(
                    ch.reset.as_deref(), ch.reset_count.as_deref());
                moves.entry(legacy).or_default().insert(new);
            }
            for (legacy, targets) in moves {
                if targets.len() != 1 {
                    r.retrofit_conflicts += 1;
                    eprintln!("retrofit conflict ({}): {} maps to {:?}",
                        path.display(), legacy, targets);
                    continue;
                }
                let target = targets.into_iter().next().unwrap();
                let Some(new_kw) = target else {
                    r.retrofit_removals += 1;
                    eprintln!("retrofit removal candidate ({}): legacy {} now unmapped",
                        path.display(), legacy);
                    continue;
                };
                if new_kw == legacy { continue; }
                let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else {
                    continue;
                };
                let (rewritten, n) =
                    rewrite_trailing_duration(&new_txt[block_lo..block_hi], legacy, new_kw);
                if n > 0 {
                    new_txt = format!("{}{}{}", &new_txt[..block_lo], rewritten, &new_txt[block_hi..]);
                    retrofitted += n;
                }
            }
        }

        if filled > 0 || refined > 0 || blocks_added > 0 || passives_added > 0 || conditions_added > 0 || costs_added > 0 || targets_added > 0 || retrofitted > 0 {
            r.effects_filled += filled;
            r.stubs_refined += refined;
            r.blocks_injected += blocks_added;
            r.passives_injected += passives_added;
            r.conditions_injected += conditions_added;
            r.costs_injected += costs_added;
            r.targets_injected += targets_added;
            r.durations_retrofitted += retrofitted;
            if let Err(e) = fs::write(&path, new_txt) {
                eprintln!("write {} failed: {}", path.display(), e);
            }
        }
    }
    r
}

/// Locate the closing brace of the top-level `card "<name>" { … }` block.
/// We assume one card per file (true for cards/official/) and find the
/// matching `}` by walking from the start counting `{`/`}` depth.
fn card_close_brace(txt: &str) -> Option<usize> {
    let start = txt.find("card ")?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut chars = txt.char_indices().skip_while(|(i, _)| *i < start);
    for (i, c) in chars.by_ref() {
        match c {
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 { return Some(i); }
            }
            _ => {}
        }
    }
    None
}

/// Inspect existing `passive "Passive N"` / `effect "Effect N"` titles
/// and return the next free integer to avoid name collisions.
fn next_passive_index(txt: &str) -> u32 {
    let mut max = 0u32;
    for marker in ["passive \"Passive ", "effect \"Effect "] {
        let mut i = 0;
        while let Some(p) = txt[i..].find(marker) {
            let abs = i + p + marker.len();
            let rest = &txt[abs..];
            let end = rest.find('"').unwrap_or(0);
            if let Ok(n) = rest[..end].parse::<u32>() {
                if n > max { max = n; }
            }
            i = abs + end;
        }
    }
    max + 1
}

/// Find the byte range of the `idx`-th (0-based) `effect "..." { ... }`
/// block in `txt`. Returns `(start_of_effect_keyword, position_after_closing_brace)`.
/// Delegates to the Phase 20 matcher's scanner so the apply passes and
/// the block-signature parser agree on what counts as a block.
fn nth_effect_block(txt: &str, idx: usize) -> Option<(usize, usize)> {
    block_match::effect_block_ranges(txt).into_iter().nth(idx)
}

fn has_empty_resolve(txt: &str) -> bool {
    // Match `resolve {` followed only by whitespace/newlines until `}`
    let mut i = 0;
    while let Some(start) = txt[i..].find("resolve {") {
        let abs = i + start + "resolve {".len();
        let rest = &txt[abs..];
        let close = match rest.find('}') { Some(c) => c, None => return false };
        let inner = &rest[..close];
        if inner.chars().all(|c| c.is_whitespace()) { return true; }
        i = abs + close + 1;
    }
    false
}

/// Find the first empty `resolve { }` whose byte range falls inside
/// `[lo, hi)` — used by Pass A to bind a fill to its own effect block.
/// Returns absolute offsets `(start_of_resolve_keyword, after_closing_brace)`.
fn first_empty_resolve_within(txt: &str, lo: usize, hi: usize) -> Option<(usize, usize)> {
    let mut i = lo;
    while let Some(start) = txt[i..hi].find("resolve {") {
        let abs_start = i + start;
        let abs_open = abs_start + "resolve {".len();
        let close = txt[abs_open..hi].find('}')?;
        let inner = &txt[abs_open..abs_open + close];
        if inner.chars().all(|c| c.is_whitespace()) {
            return Some((abs_start, abs_open + close + 1));
        }
        i = abs_open + close + 1;
    }
    None
}

/// Find the byte offset of `resolve {` inside the `effect_idx`-th (0-based)
/// `effect "..."` block in `txt`, for condition injection. Returns None when:
/// - The block doesn't exist.
/// - The block already contains `condition:` (idempotent — no double-inject).
/// - Another `effect "` appears between the block opening and `resolve {`
///   (which would indicate a nested or mis-counted block).
fn condition_inject_pos(txt: &str, effect_idx: usize) -> Option<usize> {
    let mut count = 0usize;
    let mut search = 0usize;
    loop {
        let rel = txt[search..].find("effect \"")?;
        let eff_pos = search + rel;
        if count == effect_idx {
            let after_eff = eff_pos + "effect \"".len();
            let rel_res = txt[after_eff..].find("resolve {")?;
            let resolve_pos = after_eff + rel_res;
            // Safety: no other `effect "` block between opening and resolve.
            if txt[after_eff..resolve_pos].contains("effect \"") { return None; }
            // Phase 16 — the first `resolve {` after a `choose {` is
            // NESTED inside an option block; injecting there would land
            // mid-choose. Choose-bearing effects skip.
            if txt[after_eff..resolve_pos].contains("choose {") { return None; }
            // Idempotent: skip if already has condition: in this block.
            if txt[eff_pos..resolve_pos].contains("condition:") { return None; }
            return Some(resolve_pos);
        }
        count += 1;
        search = eff_pos + "effect \"".len();
    }
}

/// Find the byte offset of `resolve {` inside the `effect_idx`-th (0-based)
/// `effect "..."` block in `txt`, for cost injection. Returns None when:
/// - The block doesn't exist.
/// - The block already contains `cost {` (idempotent — no double-inject).
/// - Another `effect "` appears between the block opening and `resolve {`.
/// - The `resolve { }` block is empty — adding cost to an empty resolve
///   would trigger "has cost but no resolve/choose" checker warnings.
fn cost_inject_pos(txt: &str, effect_idx: usize) -> Option<usize> {
    let mut count = 0usize;
    let mut search = 0usize;
    loop {
        let rel = txt[search..].find("effect \"")?;
        let eff_pos = search + rel;
        if count == effect_idx {
            let after_eff = eff_pos + "effect \"".len();
            let rel_res = txt[after_eff..].find("resolve {")?;
            let resolve_pos = after_eff + rel_res;
            // Safety: no other `effect "` block between opening and resolve.
            if txt[after_eff..resolve_pos].contains("effect \"") { return None; }
            // Phase 16 — see condition_inject_pos: a `resolve {` after
            // `choose {` is nested inside an option block.
            if txt[after_eff..resolve_pos].contains("choose {") { return None; }
            // Idempotent: skip if cost block already present in this effect.
            if txt[eff_pos..resolve_pos].contains("cost {") { return None; }
            // Skip empty resolve — would cause "has cost but no resolve" warning.
            let after_resolve = resolve_pos + "resolve {".len();
            let close = txt[after_resolve..].find('}')?;
            let inner = &txt[after_resolve..after_resolve + close];
            if inner.chars().all(|c| c.is_whitespace()) { return None; }
            return Some(resolve_pos);
        }
        count += 1;
        search = eff_pos + "effect \"".len();
    }
}

/// Find the byte offset to inject `target <sel>\n        ` in the
/// `effect_idx`-th (0-based) `effect "..."` block, for target injection.
/// Inserts before `cost {` when present, else before `resolve {`.
/// Returns None when:
/// - The block doesn't exist.
/// - The block already contains a `target` declaration (idempotent).
/// - Another `effect "` appears between the block opening and the injection
///   point (mis-count guard).
/// - The `resolve { }` block is empty (skip to avoid checker warnings).
fn target_inject_pos(txt: &str, effect_idx: usize) -> Option<usize> {
    let mut count = 0usize;
    let mut search = 0usize;
    loop {
        let rel = txt[search..].find("effect \"")?;
        let eff_pos = search + rel;
        if count == effect_idx {
            let after_eff = eff_pos + "effect \"".len();
            // Prefer injecting before `cost {`, else before `resolve {`.
            let resolve_rel = txt[after_eff..].find("resolve {")?;
            let inject_rel = txt[after_eff..].find("cost {")
                .filter(|&cr| cr < resolve_rel)
                .unwrap_or(resolve_rel);
            let inject_pos = after_eff + inject_rel;
            // Safety: no other `effect "` block between opening and injection point.
            if txt[after_eff..inject_pos].contains("effect \"") { return None; }
            // Phase 16 — see condition_inject_pos: a `resolve {` after
            // `choose {` is nested inside an option block.
            if txt[after_eff..inject_pos].contains("choose {") { return None; }
            // Idempotent: skip if a target declaration already exists.
            if txt[eff_pos..inject_pos].contains("\n        target ") { return None; }
            // Skip empty resolve — would trigger "has target but no resolve" checker warning.
            let resolve_pos = after_eff + resolve_rel;
            let after_resolve = resolve_pos + "resolve {".len();
            let close = txt[after_resolve..].find('}')?;
            let inner = &txt[after_resolve..after_resolve + close];
            if inner.chars().all(|c| c.is_whitespace()) { return None; }
            return Some(inject_pos);
        }
        count += 1;
        search = eff_pos + "effect \"".len();
    }
}

/// Render a Phase 16 [`lua_ast::ChooseSpec`] as a DSL `choose { … }`
/// block sized to replace the empty `resolve { }` slot (8-space base
/// indent). Returns None when any option label fails to resolve from
/// the card's CDB strs — the whole choose skips, not mis-labels.
fn render_choose_block(spec: &lua_ast::ChooseSpec, card_id: u32) -> Option<String> {
    let mut out = String::from("choose {\n");
    for (idx, lines) in &spec.options {
        let label = lua_ast::lookup_card_string(card_id, *idx)?;
        out.push_str(&format!("            option \"{}\" {{\n", label));
        out.push_str("                resolve {\n");
        for l in lines.iter().filter(|l| l.is_action()) {
            out.push_str(&l.clone().into_string("                    "));
            out.push('\n');
        }
        out.push_str("                }\n");
        out.push_str("            }\n");
    }
    out.push_str("        }");
    Some(out)
}

/// Cost-header parity gate for resolve fills (T38 S6 review; spec skip
/// class 10). Filling a resolve makes the block's whole header LIVE, so
/// a block-side `cost { … }` must be re-derivable from the lua chain's
/// cost handler and match it exactly:
///   - block has a cost but the chain has no cost handler → mismatch;
///   - block has a cost the extractor can't re-derive → unverifiable
///     header (c9798352's `banish self` vs the lua's
///     banish-a-Trap-from-Deck cost — the shipped text predates the
///     current extractor and contradicts the script);
///   - both present → whitespace-normalized content must be identical.
/// A block WITHOUT a cost passes even when the lua has one — the
/// incomplete-header class is corpus-wide pre-existing (conservative
/// cost extraction) and gating it would diverge from every earlier
/// phase's fills.
/// T38 S5 review — frame parity for summon-helper fills (c29280589,
/// c58657303). The Variant-B frame exemption assumes the .ds block
/// header models the chain's frame; for SetCondition (an activation
/// window: opponent's-Main-Phase-only, Necrovalley-only) and SetCost,
/// the pre-Phase-10 bare headers (speed / once_per_turn only) model
/// NOTHING — filling the resolve would make the header live and
/// silently drop the window. Require a modeling surface to exist at
/// all; exact cost parity stays with [`cost_header_matches`], whose
/// no-block-cost arm passes and so never catches this hole for
/// helper fills.
fn helper_frame_matches(block: &str, eff: &lua_ast::EffectSkeleton) -> bool {
    // set_calls double-checks the handler fields — inline closures
    // (`SetCondition(function() return Duel.IsMainPhase() end)`,
    // c92003832 / c95515789) must gate exactly like named handlers.
    let has = |method: &str| eff.set_calls.iter().any(|(m, _)| m == method);
    if (eff.condition_handler.is_some() || has("SetCondition"))
        && !(block.contains("trigger:")
            || block.contains("timing:")
            || block.contains("condition"))
    {
        return false;
    }
    if (eff.cost_handler.is_some() || has("SetCost")) && !block.contains("cost {") {
        return false;
    }
    true
}

fn cost_header_matches(
    block: &str,
    eff: &lua_ast::EffectSkeleton,
    functions: &std::collections::BTreeMap<String, lua_ast::FunctionBody>,
) -> bool {
    let Some(block_cost) = block_cost_text(block) else { return true };
    let Some(handler) = eff.cost_handler.as_deref() else { return false };
    let handler = handler.trim();
    // Inline Cost.* factories with fixed semantics (utility.lua:1485-,
    // 1630-) — decoded for the parity check only; Pass D's injection
    // surface is unchanged.
    //   - Cost.SelfBanish  → banish self (Duel.Remove(c, …, REASON_COST))
    //   - Cost.SelfTribute → tribute self (Duel.Release(c, REASON_COST))
    //   - Cost.SelfToGrave → send self to gy (Duel.SendtoGrave(c,
    //     REASON_COST)) — T38 S5, c11493868's Variant-B fusion frame.
    //   - Cost.DetachFromSelf(n), single literal arg → detach n from
    //     self; min/max or function-valued forms stay unverifiable.
    match handler {
        "Cost.SelfBanish" => {
            return normalize_ws(&block_cost) == "cost { banish self }";
        }
        "Cost.SelfTribute" => {
            return normalize_ws(&block_cost) == "cost { tribute self }";
        }
        "Cost.SelfToGrave" => {
            return normalize_ws(&block_cost) == "cost { send self to gy }";
        }
        _ => {}
    }
    if let Some(rest) = handler.strip_prefix("Cost.DetachFromSelf(") {
        if let Some(n) = rest.strip_suffix(')') {
            let n = n.trim();
            if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) {
                return normalize_ws(&block_cost)
                    == format!("cost {{ detach {} from self }}", n);
            }
        }
        return false;
    }
    let Some(spec) = lua_ast::extract_cost_block(handler, functions) else {
        return false;
    };
    normalize_ws(&spec.to_dsl_block("")) == normalize_ws(&block_cost)
}

/// The `cost { … }` substring of an effect block, brace-matched.
fn block_cost_text(block: &str) -> Option<String> {
    let start = block.find("cost {")?;
    let mut depth = 0i32;
    for (i, c) in block[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(block[start..start + i + 1].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True when any emitted action line references the effect's declared
/// target — the bare `target` selector token or a `target.<stat>` read
/// (T38 S6). Such lines only validate inside a block that carries a
/// target declaration or a choose block.
fn lines_reference_target(lines: &[lua_ast::DslLine]) -> bool {
    lines.iter().any(|l| match l {
        lua_ast::DslLine::Action(t) => t
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '(' | ')' | ','))
            .any(|tok| tok == "target" || tok.starts_with("target.")),
        lua_ast::DslLine::Todo(_) => false,
    })
}

/// True when the block text already declares a target (same marker as
/// Pass E's idempotency check) or resolves targets through a choose.
fn block_has_target_decl(block: &str) -> bool {
    block.contains("\n        target ") || block.contains("choose {")
}

/// The target-declaration selector to co-emit with a bare-target fill
/// (T38 S6): the effect's SetTarget handler through the REFINED
/// extraction — Phase 8's nil/aux.TRUE shapes plus exactly-mapped
/// custom filters. None ⇒ the fill must skip.
fn target_decl_selector(
    eff: &lua_ast::EffectSkeleton,
    functions: &std::collections::BTreeMap<String, lua_ast::FunctionBody>,
) -> Option<String> {
    let th = eff.target_handler.as_deref()?;
    Some(lua_ast::extract_target_decl_refined(th.trim(), functions)?.to_dsl())
}

fn render_resolve_body(lines: &[lua_ast::DslLine]) -> String {
    // Only emit ACTION lines into corpus — TODO comments aren't valid DSL
    // syntax (no comment lexer in grammar yet) and would break parsing.
    let mut out = String::new();
    for l in lines.iter().filter(|l| l.is_action()) {
        out.push_str(&l.clone().into_string("            "));
        out.push('\n');
    }
    out
}
