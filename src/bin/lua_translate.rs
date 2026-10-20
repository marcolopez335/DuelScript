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
        eprintln!("usage: lua_translate <dump|translate|apply|audit-fills|audit-owners|audit-orphans|audit-passives|explain> <path>");
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
            set_current_card_from_stem(Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or(""));
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
            //   lua_translate audit-fills <corpus_dir> <lua_dir> [cards.cdb] [--stub] [--shape <action>]
            //
            // T39 S2 — `--shape <action>` adds the RESHAPE class: a
            // shipped resolve the gate still ACCEPTS, but whose
            // `<action> …` lines no longer byte-match the current
            // rendering (a decoder that moved an action's rendering
            // — or refuses it as a TODO — without tripping a body
            // gate). `--stub` empties those too; `apply` refills them
            // under the corrected decoder where its gate allows.
            if args.len() < 4 {
                eprintln!("usage: lua_translate audit-fills <corpus_dir> <lua_dir> [cards.cdb] [--stub] [--shape <action>]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            let stub = args.iter().any(|a| a == "--stub");
            let shape = args.iter().position(|a| a == "--shape")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str);
            let cdb = args.get(4).map(String::as_str)
                .filter(|a| *a != "--stub" && *a != "--shape");
            load_card_names(cdb, lua_dir);
            audit_fills(corpus, lua_dir, stub, shape);
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
        "audit-orphans" => {
            // T39 S7 — generator blocks with no lua consumer: the .ds
            // has more `effect` blocks than the lua registers consumer
            // chains, and no translator pass can ever fill the surplus
            // (no lua body exists for it). Classified mechanically
            // (`block_match::orphan_blocks`): a surplus block beside a
            // ritual summon-helper consumer with an empty body and a
            // content-free header is the proc's material / once-per-
            // turn clause, already stated by the owner's
            // `ritual_summon … where total_level …` line — `--remove`
            // deletes exactly those. Every other class is report-only
            // (SetTarget-only real effects, union / pendulum / whole-
            // level-tribute procs with no DSL surface, headers with
            // cost / trigger content, hazard-unrescued consumers).
            //
            //   lua_translate audit-orphans <corpus_dir> <lua_dir> [cards.cdb] [--remove]
            if args.len() < 4 {
                eprintln!("usage: lua_translate audit-orphans <corpus_dir> <lua_dir> [cards.cdb] [--remove]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            let remove = args.iter().any(|a| a == "--remove");
            let cdb = args.get(4).map(String::as_str).filter(|a| *a != "--remove");
            load_card_names(cdb, lua_dir);
            audit_orphans(corpus, lua_dir, remove);
        }
        "audit-passives" => {
            // T39 S8 — the Phase 5d-era passive retrofit. The v1
            // generator rendered every `SetCode(EFFECT_DISABLE*)` /
            // `EFFECT_CANNOT_ACTIVATE` / `EFFECT_CANNOT_TRIGGER` chain
            // in a script — resolve-time or continuous, card- or
            // player-scoped — as `scope: self` + `grant: cannot_activate
            // effects`, which lowers to EFFECT_CANNOT_TRIGGER on the
            // card ITSELF. Every `initial_effect` chain on the three
            // codes is re-derived under the current emitter
            // (`passive_floodgate_verdict`, one CHAIN line each:
            // RENDER <body> | SKIP <gate>); every `passive` block whose
            // body is the legacy rendering is a machine emission the
            // flip invalidated — `--stub` rewrites it in place with the
            // current rendering owed to the card (name kept) or removes
            // it, and `apply` Pass B injects any surplus rendering
            // under the same body key (byte-idempotent fixed point).
            //
            //   lua_translate audit-passives <corpus_dir> <lua_dir> [cards.cdb] [--stub]
            if args.len() < 4 {
                eprintln!("usage: lua_translate audit-passives <corpus_dir> <lua_dir> [cards.cdb] [--stub]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            let stub = args.iter().any(|a| a == "--stub");
            let cdb = args.get(4).map(String::as_str).filter(|a| *a != "--stub");
            load_card_names(cdb, lua_dir);
            audit_passives(corpus, lua_dir, stub);
        }
        "explain" => {
            // T39 — per-block fill verdicts for the empty-resolve error
            // class. For every effect block whose resolve is empty (or
            // that has neither resolve nor choose), print ONE tab-
            // separated line naming the FIRST Pass A gate that refuses
            // the fill — the same `plan_fill` apply consumes — and
            // the gate's own datum:
            //
            //   card  block  owner|unassigned  reason  detail  type  stmts
            //
            //   lua_translate explain <corpus_dir> <lua_dir> [cards.cdb]
            if args.len() < 4 {
                eprintln!("usage: lua_translate explain <corpus_dir> <lua_dir> [cards.cdb]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            load_card_names(args.get(4).map(String::as_str), lua_dir);
            explain(corpus, lua_dir);
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
/// `--shape <action>` test (T39 S2): the shipped resolve carries at least
/// one `<action> …` line and the `<action>` lines, in order, differ from
/// the current rendering's. Lines of other actions are not compared —
/// a selector or duration drift in a sibling line is another slice's
/// retrofit, not this one's.
/// The DECL-class verdict for a shipped `target (…)` line (T39 S9 /
/// S10): equal to the current rendering → healthy; equal to one of the
/// `legacies` renderings (name, line) → a machine emission a policy
/// flip re-renders (`Rewrite` names the policy); equal to neither while
/// the current rendering exists → `Review` (hand content or an older
/// emitter's). A current rendering that is refused is never reviewed —
/// there is nothing to compare against.
#[derive(Debug, PartialEq, Eq)]
enum DeclVerdict {
    Healthy,
    Rewrite(&'static str),
    Review,
}

fn classify_decl(
    shipped: &str,
    current: Option<&str>,
    legacies: &[(&'static str, Option<String>)],
) -> DeclVerdict {
    if current == Some(shipped) {
        return DeclVerdict::Healthy;
    }
    if let Some((name, _)) = legacies.iter().find(|(_, l)| l.as_deref() == Some(shipped)) {
        return DeclVerdict::Rewrite(name);
    }
    if current.is_some() { DeclVerdict::Review } else { DeclVerdict::Healthy }
}

fn shape_lines_differ(shipped: &[String], rendered: &[String], action: &str) -> bool {
    let prefix = format!("{} ", action);
    let pick = |lines: &[String]| -> Vec<String> {
        lines.iter().filter(|l| l.starts_with(&prefix) || l.as_str() == action).cloned().collect()
    };
    let shipped_kw = pick(shipped);
    !shipped_kw.is_empty() && shipped_kw != pick(rendered)
}

fn audit_fills(corpus_dir: &str, lua_dir: &str, stub: bool, shape: Option<&str>) {
    let mut paths: Vec<std::path::PathBuf> = match fs::read_dir(corpus_dir) {
        Ok(e) => e.flatten().map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
            .collect(),
        Err(e) => { eprintln!("cannot read {}: {}", corpus_dir, e); return; }
    };
    paths.sort();
    let (mut stubs, mut reviews, mut reshapes, mut reshape_reviews) = (0usize, 0usize, 0usize, 0usize);
    let (mut decls, mut decl_reviews) = (0usize, 0usize);
    for path in paths {
        let Ok(txt) = fs::read_to_string(&path) else { continue };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        set_current_card_from_stem(&stem);
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
            // Offsets recomputed against new_txt — a stub earlier in this
            // file shifts every later block.
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else { continue };
            // T39 S9 — DECL class: a shipped `target (…)` line equal to
            // the pre-S9 decl rendering that the shared-mapper rendering
            // now writes differently is a machine-emitted declaration
            // the flip re-renders; `--stub` rewrites the line in place
            // (a decl has no empty form — stubbing it would orphan the
            // resolve's `target` references). A shipped line equal to
            // NEITHER rendering is hand content or an older emitter's
            // and is reported only.
            // T39 S10 adds the pre-T38 S1 Pass E rendering (no `except
            // self`) as a second legacy policy: a shipped decl equal to
            // it is the same machine-emission class.
            if let Some((d_lo, d_hi)) = shipped_target_decl_span(&new_txt, block_lo, block_hi) {
                let shipped_decl = new_txt[d_lo..d_hi].trim().to_string();
                let current = target_decl_selector(eff, &walk.functions);
                let legacy = lua_ast::with_legacy_decl_policy(|| target_decl_selector(eff, &walk.functions));
                let legacy_exc = lua_ast::with_legacy_decl_exc_policy(|| target_decl_selector(eff, &walk.functions));
                let current_line = current.as_ref().ok().map(|s| format!("target {}", s));
                let legacies = [
                    ("s9-decl", legacy.as_ref().ok().map(|s| format!("target {}", s))),
                    ("pre-s1-exc", legacy_exc.as_ref().ok().map(|s| format!("target {}", s))),
                ];
                match classify_decl(&shipped_decl, current_line.as_deref(), &legacies) {
                    DeclVerdict::Healthy => {}
                    DeclVerdict::Rewrite(policy) => {
                        decls += 1;
                        println!("DECL   {} block={} target={} legacy={} current={}",
                            stem, block_idx, eff.target_handler.as_deref().unwrap_or("-").trim(),
                            policy, current.as_ref().map(|_| "ok").unwrap_or("refused"));
                        println!("    shipped:  {}", shipped_decl);
                        match &current {
                            Ok(s) => println!("    rendered: target {}", s),
                            Err(e) => println!("    refused:  {}", e),
                        }
                        if stub {
                            if let Some(line) = &current_line {
                                new_txt = format!("{}{}{}", &new_txt[..d_lo], line, &new_txt[d_hi..]);
                                changed = true;
                            }
                        }
                    }
                    DeclVerdict::Review => {
                        decl_reviews += 1;
                        println!("DECL-REVIEW {} block={} target={}",
                            stem, block_idx, eff.target_handler.as_deref().unwrap_or("-").trim());
                        println!("    shipped:  {}", shipped_decl);
                        println!("    rendered: {}", current_line.as_deref().unwrap_or(""));
                    }
                }
            }
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else { continue };
            let Some((inner_lo, inner_hi)) = block_match::resolve_inner_span(&new_txt, block_lo, block_hi) else { continue };
            let shipped: Vec<String> = new_txt[inner_lo..inner_hi].lines()
                .map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
            if shipped.is_empty() { continue; } // already a stub
            let action_lines = |lines: &[lua_ast::DslLine]| -> Vec<String> {
                lines.iter().filter(|l| l.is_action())
                    .map(|l| l.clone().into_string("").trim().to_string()).collect()
            };
            let lines = lua_ast::translate_body_with_functions(body, &walk.functions);
            let rendered = action_lines(&lines);
            let drop = lua_ast::body_drop_reason(body, &walk.functions);
            if shipped == rendered && drop.is_none() { continue; } // healthy fill
            // `--shape <action>` RESHAPE class (T39 S2 ChangePosition
            // retrofit): a body whose `<action>` lines moved. The drop
            // verdict is reported, not consulted (T39 S4): a shipped
            // `<action>` line the current decoder would not write is
            // wrong whether or not the rest of the body lowers — a
            // `drop=` hit is stubbed and stays empty (apply refuses the
            // body), the skip-not-mis-emit outcome.
            if let Some(action) = shape {
                if shape_lines_differ(&shipped, &rendered, action) {
                    // T39 S10: the single-flip legacy rendering of the
                    // shape, when one exists — a shipped block equal to
                    // it (or to the all-flips rendering) is a machine
                    // fill the flip moved (`machine=yes`); anything else
                    // is hand content or an older emitter's (`no`). The
                    // stub is mechanical either way; the tag steers the
                    // hand verification.
                    let render = || action_lines(&lua_ast::translate_body_with_functions(body, &walk.functions));
                    let single_flip = match action {
                        "special_summon" => Some(lua_ast::with_legacy_summon_pos_policy(render)),
                        "add_to_hand" | "return" => Some(lua_ast::with_legacy_to_hand_policy(render)),
                        _ => None,
                    };
                    let all_flips = lua_ast::with_legacy_selector_policy(render);
                    // A shape WITH a single-flip policy stubs only its
                    // machine fills: the generator's category
                    // placeholders (`special_summon (all, card, either
                    // controls)`) and older drift under the same verb
                    // are the report-only class S4 / S5 left behind,
                    // not this flip's. A shape without one (S2
                    // change_position, S4 discard) keeps stubbing every
                    // hit.
                    let machine = match single_flip {
                        Some(l) if l == shipped || all_flips == shipped => "yes",
                        Some(_) => "no",
                        None => "-",
                    };
                    let class = if machine == "no" { "RESHAPE-REVIEW" } else { "RESHAPE" };
                    if machine == "no" { reshape_reviews += 1; } else { reshapes += 1; }
                    println!(
                        "{} {} block={} handler={} shape={} drop={} machine={}",
                        class, stem, block_idx, handler, action,
                        drop.as_ref().map(|d| d.code()).unwrap_or("-"),
                        machine
                    );
                    for l in &shipped  { println!("    shipped:  {}", l); }
                    for l in &rendered { println!("    rendered: {}", l); }
                    for l in lines.iter().filter(|l| !l.is_action()) {
                        println!("    todo:     {}", l.clone().into_string("").trim());
                    }
                    if stub && machine != "no" {
                        new_txt = format!("{}\n        {}", &new_txt[..inner_lo], &new_txt[inner_hi..]);
                        changed = true;
                    }
                    continue;
                }
            }
            // T39 S3 — the pre-gate rendering: selector filters resolved
            // only by the exact named-filter refinement on group-
            // provenance specs, unmapped filters rendered as the
            // superset, no fill gate. A shipped block equal to it is a
            // machine fill the policy flip invalidated — stubbed so
            // `apply` refills it under the current policy (tighter
            // where-clause) or leaves it empty (gate refuses).
            let legacy = lua_ast::with_legacy_selector_policy(|| {
                action_lines(&lua_ast::translate_body_with_functions(body, &walk.functions))
            });
            // T39 S5 — the Release-only legacy rendering: a block
            // refilled under S4's exact discard line (#171) but before
            // the Release refusal matches neither the current nor the
            // all-flips rendering (Megalith Anastasis).
            let legacy_release = lua_ast::with_legacy_release_policy(|| {
                action_lines(&lua_ast::translate_body_with_functions(body, &walk.functions))
            });
            // No gate refusal and no policy change on this body: the
            // mismatch pre-dates both — hand content or an older
            // translator's line — and is not this audit's business.
            if drop.is_none() && legacy == rendered && legacy_release == rendered { continue; }
            // `legacy-policy`: no gate refusal, the line-shape moved
            // (T39 S3 selector widening, S4 DiscardHand, S5 Release).
            let reason = match &drop {
                Some(d) => d.code(),
                None => "legacy-policy",
            };
            // The gate's own datum (T39 S5 census: the modal fork's
            // discriminant) plus the TODO lines the current rendering
            // carries — a shipped block equal to the action lines of a
            // rendering that ALSO has TODOs is a pre-gate-era fill
            // that dropped statements.
            let todo_n = lines.iter().filter(|l| !l.is_action()).count();
            let detail = drop.as_ref().map(|d| normalize_ws(d.detail())).unwrap_or_default();
            if shipped == rendered || shipped == legacy || shipped == legacy_release {
                stubs += 1;
                println!("STUB   {} block={} handler={} reason={} lines={} todo={} detail={}",
                    stem, block_idx, handler, reason, shipped.len(), todo_n, detail);
                for l in &shipped { println!("    - {}", l); }
                if stub {
                    new_txt = format!("{}\n        {}", &new_txt[..inner_lo], &new_txt[inner_hi..]);
                    changed = true;
                }
            } else {
                reviews += 1;
                println!("REVIEW {} block={} handler={} reason={} todo={} detail={}",
                    stem, block_idx, handler, reason, todo_n, detail);
                for l in &shipped  { println!("    shipped:  {}", l); }
                for l in &rendered { println!("    rendered: {}", l); }
                if legacy != rendered {
                    for l in &legacy { println!("    legacy:   {}", l); }
                }
                if legacy_release != rendered && legacy_release != legacy {
                    for l in &legacy_release { println!("    legacy-release: {}", l); }
                }
            }
        }
        if changed {
            if let Err(e) = fs::write(&path, &new_txt) {
                eprintln!("cannot write {}: {}", path.display(), e);
            }
        }
    }
    match shape {
        Some(action) => println!(
            "=== audit-fills: {} stub, {} review, {} reshape, {} reshape-review ({}) ===",
            stubs, reviews, reshapes, reshape_reviews, action
        ),
        None => println!("=== audit-fills: {} stub, {} review ===", stubs, reviews),
    }
    println!("=== audit-fills decls: {} decl, {} decl-review ===", decls, decl_reviews);
}

/// Byte span of the shipped `target (…)` declaration line of the
/// effect block at `[lo, hi)` — from the line's first non-blank byte to
/// its end — or None when the block declares none (same marker as Pass
/// E's idempotency check).
fn shipped_target_decl_span(txt: &str, lo: usize, hi: usize) -> Option<(usize, usize)> {
    let block = &txt[lo..hi];
    let at = block.find("\n        target (")? + 1;
    let rest = &block[at..];
    let start = lo + at + (rest.len() - rest.trim_start().len());
    let end = lo + at + rest.find('\n').unwrap_or(rest.len());
    Some((start, end))
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
        set_current_card_from_stem(&stem);
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

/// See the `audit-orphans` mode arm. One line per consumer-less effect
/// block; the verdict lives in `block_match::orphan_blocks` so it is
/// unit-testable. `--remove` deletes the `ORPHAN` class only — never a
/// block with a body, a choose, or header content — and is idempotent:
/// a rerun finds no surplus block on a card it already trimmed.
fn audit_orphans(corpus_dir: &str, lua_dir: &str, remove: bool) {
    use block_match::OrphanClass as C;
    let mut paths: Vec<std::path::PathBuf> = match fs::read_dir(corpus_dir) {
        Ok(e) => e.flatten().map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
            .collect(),
        Err(e) => { eprintln!("cannot read {}: {}", corpus_dir, e); return; }
    };
    paths.sort();
    let mut by_class: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let mut cards = std::collections::BTreeSet::new();
    let mut removed = 0usize;
    for path in paths {
        let Ok(txt) = fs::read_to_string(&path) else { continue };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        set_current_card_from_stem(&stem);
        let lua_path = Path::new(lua_dir).join(format!("{}.lua", stem));
        let Ok(lua_src) = fs::read_to_string(&lua_path) else { continue };
        let Ok(parsed) = full_moon::parse(&lua_src) else { continue };
        let walk = lua_ast::walk(&parsed);
        if walk.effects.is_empty() { continue; }
        let assign = block_match::compute_assignments(&walk, &txt);
        let hits = block_match::orphan_blocks(&walk, &assign, &txt, &lua_src);
        if hits.is_empty() { continue; }
        cards.insert(stem.clone());
        let ranges = block_match::effect_block_ranges(&txt);
        let n_consumers = walk.effects.iter()
            .filter(|e| e.is_summon_helper() || e.operation_handler.is_some()).count();
        let helpers: Vec<&str> = walk.effects.iter()
            .filter(|e| e.ritual_summon_spec)
            .map(|e| e.binding.as_str()).collect();
        let procs: Vec<&str> = block_match::NO_SURFACE_PROCS.iter().copied()
            .filter(|p| lua_src.contains(p)).collect();
        // Removal offsets are taken from the ORIGINAL text, highest
        // block first, so earlier removals never shift later spans.
        let mut new_txt = txt.clone();
        for h in hits.iter().rev() {
            *by_class.entry(h.class.tag()).or_default() += 1;
            let (lo, hi) = ranges[h.block];
            let block = &txt[lo..hi];
            let name = block.split('"').nth(1).unwrap_or("?");
            let header: Vec<&str> = block_match::parse_block_sigs(block).first()
                .map(|s| {
                    let mut v = Vec::new();
                    if let Some(sp) = s.speed { v.push(if sp == 1 { "speed:1" } else if sp == 2 { "speed:2" } else { "speed:3" }); }
                    if s.trigger_line { v.push("trigger"); }
                    if s.mandatory { v.push("mandatory"); }
                    if s.once_per_turn { v.push("once_per_turn"); }
                    if s.has_cost { v.push("cost"); }
                    if s.has_target { v.push("target"); }
                    if s.has_condition { v.push("condition"); }
                    v
                })
                .unwrap_or_default();
            println!("{} {} block={} name={:?} header=[{}] consumers={} blocks={} ritual_helpers=[{}] no_surface_procs=[{}]",
                h.class.tag(), stem, h.block, name, header.join(" "), n_consumers, ranges.len(),
                helpers.join(","), procs.join(","));
            if remove && h.class == C::Orphan {
                new_txt = block_match::remove_block(&new_txt, lo, hi);
                removed += 1;
            }
        }
        if new_txt != txt {
            if let Err(e) = fs::write(&path, &new_txt) {
                eprintln!("cannot write {}: {}", path.display(), e);
            }
        }
    }
    let total: usize = by_class.values().sum();
    let classes: Vec<String> = by_class.iter().map(|(k, v)| format!("{} {}", v, k)).collect();
    println!("=== audit-orphans: {} blocks / {} cards — {}; {} removed ===",
        total, cards.len(), classes.join(", "), removed);
}

/// Every `passive` block of a .ds: `(byte range, name, item lines)`.
/// Item lines are trimmed and blank-free, so two blocks compare equal
/// on content regardless of name or indentation.
fn passive_blocks(txt: &str) -> Vec<((usize, usize), String, Vec<String>)> {
    block_match::passive_block_ranges(txt).into_iter().map(|(lo, hi)| {
        let block = &txt[lo..hi];
        let name = block.split('"').nth(1).unwrap_or("").to_string();
        let open = block.find('{').map_or(block.len(), |i| i + 1);
        let close = block.rfind('}').unwrap_or(block.len());
        let body: Vec<String> = block[open..close.max(open)].lines()
            .map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
        ((lo, hi), name, body)
    }).collect()
}

/// True when some `passive` block in `txt` carries exactly `body`.
fn passive_body_present(txt: &str, body: &[String]) -> bool {
    passive_blocks(txt).iter().any(|(_, _, b)| b == body)
}

/// Verdict for one shipped `passive` block whose body is the Phase
/// 5d-era rendering (T39 S8 `audit-passives`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum PassiveRetrofit {
    /// A current rendering exists for one of the card's chains and no
    /// block carries it yet — rewritten in place (name kept).
    Rewrite(Vec<String>),
    /// No current rendering is owed to this block — removed.
    Remove,
    /// Legacy body on a card whose lua carries none of the three codes
    /// (no machine provenance) — report-only.
    Review,
}

/// One card's `audit-passives` plan: the per-chain verdicts (every
/// `initial_effect` chain on the three codes, rendered or refused) and
/// the per-block retrofit of each legacy-bodied `passive` block, plus
/// the text `--stub` writes. Pure — the mode arm does the I/O.
///
/// Pairing: legacy blocks are matched to the card's current renderings
/// in source order; renderings already carried by a non-legacy block
/// are not owed. Surplus renderings (more chains than legacy blocks)
/// are left for `apply` Pass B, which injects with the same body key —
/// so stub → apply → stub → apply is a fixed point.
fn plan_passive_retrofit(
    txt: &str,
    walk: &lua_ast::LuaReport,
    lua_src: &str,
) -> (Vec<(String, Result<Vec<String>, String>)>, Vec<(usize, String, PassiveRetrofit)>, String) {
    // The Phase 5d-era body — what every chain on the three codes
    // renders to under `with_legacy_passive_grant_policy` (pinned by
    // `legacy_passive_grant_policy_renders_phase_5d_block`), so one
    // constant stands for the whole legacy class.
    let legacy_body = lua_ast::PassiveFloodgateSpec::legacy().body_lines();
    let verdicts: Vec<(String, Result<Vec<String>, String>)> = walk.effects.iter()
        .filter(|e| e.first_arg_of("SetCode").is_some_and(|c| {
            matches!(c, "EFFECT_DISABLE" | "EFFECT_CANNOT_ACTIVATE" | "EFFECT_CANNOT_TRIGGER")
        }))
        .map(|e| (e.binding.clone(), e.passive_floodgate_verdict(walk).map(|s| s.body_lines())))
        .collect();
    // The v1 generator keyed on the code NAMES as substrings:
    // `EFFECT_DISABLE_FIELD` (Ojama King's unusable zones) produced the
    // same block as `EFFECT_DISABLE`.
    let provenance = ["EFFECT_DISABLE", "EFFECT_CANNOT_ACTIVATE", "EFFECT_CANNOT_TRIGGER"]
        .iter().any(|c| lua_src.contains(c));
    let blocks = passive_blocks(txt);
    let mut owed: Vec<Vec<String>> = Vec::new();
    for (_, v) in &verdicts {
        if let Ok(body) = v {
            let carried = blocks.iter().any(|(_, _, b)| b != &legacy_body && b == body);
            if !carried && !owed.contains(body) { owed.push(body.clone()); }
        }
    }
    let mut owed = owed.into_iter();
    let mut plan: Vec<(usize, String, PassiveRetrofit)> = Vec::new();
    for (i, (_, name, body)) in blocks.iter().enumerate() {
        if body != &legacy_body { continue; }
        let verdict = if !provenance {
            PassiveRetrofit::Review
        } else if let Some(b) = owed.next() {
            PassiveRetrofit::Rewrite(b)
        } else {
            PassiveRetrofit::Remove
        };
        plan.push((i, name.clone(), verdict));
    }
    // Edits applied highest block first so earlier spans stay valid.
    let mut new_txt = txt.to_string();
    for (i, name, verdict) in plan.iter().rev() {
        let ((lo, hi), _, _) = &blocks[*i];
        match verdict {
            PassiveRetrofit::Rewrite(body) => {
                let line_start = new_txt[..*lo].rfind('\n').map_or(0, |p| p + 1);
                let indent = new_txt[line_start..*lo].to_string();
                let spec_txt = render_passive_block(name, body, &indent);
                new_txt = format!("{}{}{}", &new_txt[..line_start], spec_txt, &new_txt[*hi..]);
            }
            PassiveRetrofit::Remove => {
                new_txt = block_match::remove_block(&new_txt, *lo, *hi);
            }
            PassiveRetrofit::Review => {}
        }
    }
    (verdicts, plan, new_txt)
}

/// `passive "<name>" { … }` from item lines — the layout
/// `PassiveFloodgateSpec::to_dsl_block` / Pass B emit.
fn render_passive_block(name: &str, body: &[String], indent: &str) -> String {
    let items: String = body.iter().map(|l| format!("{indent}    {l}\n")).collect();
    format!("{indent}passive \"{name}\" {{\n{items}{indent}}}")
}

/// See the `audit-passives` mode arm.
fn audit_passives(corpus_dir: &str, lua_dir: &str, stub: bool) {
    let mut paths: Vec<std::path::PathBuf> = match fs::read_dir(corpus_dir) {
        Ok(e) => e.flatten().map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
            .collect(),
        Err(e) => { eprintln!("cannot read {}: {}", corpus_dir, e); return; }
    };
    paths.sort();
    let (mut rendered, mut refused, mut rewrites, mut removes, mut reviews) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut refusals: std::collections::BTreeMap<String, usize> = Default::default();
    let mut cards = std::collections::BTreeSet::new();
    for path in paths {
        let Ok(txt) = fs::read_to_string(&path) else { continue };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        set_current_card_from_stem(&stem);
        let lua_path = Path::new(lua_dir).join(format!("{}.lua", stem));
        let Ok(lua_src) = fs::read_to_string(&lua_path) else { continue };
        let Ok(parsed) = full_moon::parse(&lua_src) else { continue };
        let walk = lua_ast::walk(&parsed);
        let (verdicts, plan, new_txt) = plan_passive_retrofit(&txt, &walk, &lua_src);
        for (binding, v) in &verdicts {
            let eff = walk.effects.iter().find(|e| &e.binding == binding);
            let code = eff.and_then(|e| e.first_arg_of("SetCode")).unwrap_or("-");
            let etype = eff.and_then(|e| e.first_arg_of("SetType")).unwrap_or("-");
            let range = eff.and_then(|e| e.args_of("SetTargetRange")).map(|a| a.join(",")).unwrap_or_else(|| "-".to_string());
            match v {
                Ok(body) => {
                    rendered += 1;
                    println!("CHAIN  {} {} code={} type={} range={} RENDER {}", stem, binding, code, etype, range, body.join(" | "));
                }
                Err(reason) => {
                    refused += 1;
                    let key = reason.split(':').next().unwrap_or(reason).to_string();
                    *refusals.entry(key).or_default() += 1;
                    println!("CHAIN  {} {} code={} type={} range={} SKIP {}", stem, binding, code, etype, range, reason);
                }
            }
        }
        if plan.is_empty() { continue; }
        cards.insert(stem.clone());
        for (block, name, verdict) in &plan {
            match verdict {
                PassiveRetrofit::Rewrite(body) => {
                    rewrites += 1;
                    println!("REWRITE {} block={} name={:?} -> {}", stem, block, name, body.join(" | "));
                }
                PassiveRetrofit::Remove => {
                    removes += 1;
                    println!("REMOVE  {} block={} name={:?}", stem, block, name);
                }
                PassiveRetrofit::Review => {
                    reviews += 1;
                    println!("REVIEW  {} block={} name={:?} (legacy body, no lua provenance)", stem, block, name);
                }
            }
        }
        if stub && new_txt != txt {
            if let Err(e) = fs::write(&path, &new_txt) {
                eprintln!("cannot write {}: {}", path.display(), e);
            }
        }
    }
    let by_reason: Vec<String> = refusals.iter().map(|(k, v)| format!("{} {}", v, k)).collect();
    println!("=== audit-passives: {} chains rendered, {} refused ({}); {} legacy blocks / {} cards — {} rewrite, {} remove, {} review{} ===",
        rendered, refused, by_reason.join(", "), rewrites + removes + reviews, cards.len(),
        rewrites, removes, reviews, if stub { " (stubbed)" } else { "" });
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
            // T39 S3 — `IsSetCard(SET_X)` / `IsCode(CARD_X)` filter atoms
            // resolve through the corpus's constant files (siblings of
            // the official/ script dir) and the cdb member names.
            load_lua_constants(lua_dir, &cdb);
        }
        Err(e) => eprintln!("note: cannot read {}: {:?} — change_name translation disabled", path.display(), e),
    }
}

/// Register the `SET_*` → archetype-name and `CARD_*` → passcode
/// tables from `<lua_dir>/../archetype_setcode_constants.lua` and
/// `<lua_dir>/../card_counter_constants.lua` (T39 S3). Missing files
/// are non-fatal: the affected filter atoms stay unmapped.
fn load_lua_constants(lua_dir: &str, cdb: &duelscript::cdb::CdbReader) {
    let base = Path::new(lua_dir).join("..");
    match fs::read_to_string(base.join("archetype_setcode_constants.lua")) {
        Ok(src) => {
            let consts = duelscript::setcode_names::parse_setcode_constants(&src);
            let cards = cdb.all_cards();
            let rows: Vec<duelscript::setcode_names::CardRow<'_>> = cards
                .iter()
                .map(|c| duelscript::setcode_names::CardRow {
                    setcode: c.setcode,
                    card_type: c.card_type,
                    name: c.name.as_str(),
                })
                .collect();
            lua_ast::register_setcode_archetypes(
                duelscript::setcode_names::derive_archetype_names(&consts, &rows),
            );
        }
        Err(e) => eprintln!("note: no archetype_setcode_constants.lua beside {}: {} — IsSetCard atoms outside the curated table stay unmapped", lua_dir, e),
    }
    match fs::read_to_string(base.join("card_counter_constants.lua")) {
        Ok(src) => lua_ast::register_lua_card_constants(
            duelscript::setcode_names::parse_card_constants(&src),
        ),
        Err(e) => eprintln!("note: no card_counter_constants.lua beside {}: {} — IsCode(CARD_*) atoms stay unmapped", lua_dir, e),
    }
}

/// Bind the translator's current card (`IsCode(id)` resolution) from
/// a corpus file stem (`c44095762`).
fn set_current_card_from_stem(stem: &str) {
    lua_ast::set_current_card(stem.strip_prefix('c').and_then(|s| s.parse().ok()));
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

/// Why Pass A declines to fill one effect's resolve: the FIRST gate
/// along the pass's decision path that refuses. Closed set — every
/// variant maps to exactly one `ApplyReport` skip counter through
/// [`FillReason::count`]. `apply` and `explain` consume the same
/// [`plan_fill`], so a code printed by `explain` IS the reason the
/// corpus block is empty, not a diagnostic approximation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FillReason {
    /// The chain has no `SetOperation` and is no summon helper — a pure
    /// passive with no effect block in the .ds. Counted `no map`.
    NoHandler,
    /// `EFFECT_*_REPLACE` chain: the op body belongs to the generated
    /// `replacement` block (T38 S4). Counted `incomplete`.
    ReplacementChain,
    /// No block assignment (Phase 20 / T38 S9): a hazard-flagged chain
    /// the matcher couldn't force, or a card whose binding order and
    /// source order disagree. Counted `align`.
    Unassigned,
    /// Summon helper (fusion/ritual factory, SummonEffOP / Operation)
    /// whose params don't decode to a line — detail names the helper
    /// gate (`unresolved params`, `frame: SetX`, `tg/op mismatch`,
    /// `params: …`). Counted `no map`.
    SummonHelperUndecodable,
    /// The `SetOperation` handler has no walked body — a library
    /// function (`aux.*`, `Cost.*`) or a body the walk collected
    /// nothing from. Counted `no map`.
    NoBody,
    /// `lua_ast::body_drop_reason` — a chain / method op / counter op /
    /// helper the translation would silently omit (T38 S1/S2b/S6).
    /// Counted `incomplete`.
    Drop(lua_ast::BodyDrop),
    /// Translation yielded no line at all — every collected Duel.*
    /// call is a UI / query / bookkeeping skip, or the walk collected
    /// nothing. Counted `todo`.
    NoLines,
    /// Translation yielded only TODO lines — every Duel.* statement is
    /// unmapped. Counted `todo`.
    TodoOnly,
    /// Mixed action + TODO lines — a sibling statement is unmapped
    /// (T38 S1 skip-not-mis-emit). Counted `incomplete`.
    Incomplete,
    /// The assigned block index is beyond the .ds's effect blocks.
    /// Not counted (apply's silent `continue`).
    NoBlock,
    /// The block's `cost { … }` doesn't re-derive from the lua cost
    /// handler (T38 S6 review, skip class 10). Counted `incomplete`.
    CostHeaderMismatch,
    /// Summon-helper fill onto a header that models no condition /
    /// cost frame the lua chain carries (T38 S5 review). Counted
    /// `incomplete`.
    HelperFrameMismatch,
    /// The fill references `target` and the block has no declaration
    /// the refined SetTarget extraction can co-emit (T38 S6). Counted
    /// `incomplete`.
    TargetDeclUndecodable,
}

impl FillReason {
    /// Stable kebab-case reason code — the `explain` column.
    fn code(&self) -> &'static str {
        match self {
            FillReason::NoHandler => "no-handler",
            FillReason::ReplacementChain => "replacement-chain",
            FillReason::Unassigned => "unassigned",
            FillReason::SummonHelperUndecodable => "summon-helper-undecodable",
            FillReason::NoBody => "no-body",
            FillReason::Drop(d) => d.code(),
            FillReason::NoLines => "no-lines",
            FillReason::TodoOnly => "todo-only",
            FillReason::Incomplete => "incomplete",
            FillReason::NoBlock => "no-block",
            FillReason::CostHeaderMismatch => "cost-header-mismatch",
            FillReason::HelperFrameMismatch => "helper-frame-mismatch",
            FillReason::TargetDeclUndecodable => "target-decl-undecodable",
        }
    }

    /// Bump the `ApplyReport` counter Pass A historically charged this
    /// skip to.
    fn count(&self, r: &mut ApplyReport) {
        match self {
            FillReason::NoHandler
            | FillReason::SummonHelperUndecodable
            | FillReason::NoBody => r.effects_no_handler += 1,
            FillReason::ReplacementChain
            | FillReason::Drop(_)
            | FillReason::Incomplete
            | FillReason::CostHeaderMismatch
            | FillReason::HelperFrameMismatch
            | FillReason::TargetDeclUndecodable => r.effects_incomplete += 1,
            FillReason::Unassigned => r.effects_alignment_hazard += 1,
            FillReason::NoLines | FillReason::TodoOnly => r.effects_todo_only += 1,
            FillReason::NoBlock => {}
        }
    }
}

/// Why Pass A would (not) fill an effect's resolve — ONE decision path
/// shared by Pass A (empty resolves), Pass A2 (blocks without a
/// resolve), Pass S (lone category stubs, T38) and `explain` (T39), so
/// the stub refinement can never admit a fill the empty-resolve path
/// refuses and the diagnostic can never name a gate the passes don't
/// apply. Refusals carry the closed [`FillReason`] vocabulary.
#[derive(Debug)]
enum FillPlan {
    /// Phase 16 — a rendered `choose { … }` block replaces the resolve.
    Choose(String),
    /// A complete fill: the action lines plus, when they reference
    /// `target` and the block has no declaration, the co-emitted
    /// target selector.
    Fill { lines: Vec<lua_ast::DslLine>, co_target: Option<String> },
    /// Declined. `choose_why` is set when the effect is choose-shaped
    /// (a SelectOption / label dispatch is present) but the Phase 16
    /// extraction refused — the pass then fell through to the plain
    /// path, whose gate `reason` names.
    Skip { reason: FillReason, detail: String, choose_why: Option<String> },
}

/// The gate sequence for one walk effect, in the order Pass A
/// evaluates them. `txt` is the CURRENT corpus text (earlier fills in
/// the same file shift block offsets; indices are stable).
fn plan_fill(
    eff_i: usize,
    eff: &lua_ast::EffectSkeleton,
    walk: &lua_ast::LuaReport,
    assign: &block_match::Assignments,
    txt: &str,
    card_id: Option<u32>,
) -> FillPlan {
    let skip = |reason: FillReason, detail: String| FillPlan::Skip {
        reason, detail: normalize_ws(&detail), choose_why: None,
    };
    if !eff.is_summon_helper() && eff.operation_handler.is_none() {
        return skip(FillReason::NoHandler, eff.binding.clone()); // pure-passive — no effect block in .ds
    }
    // T38 S4 — replacement-code chains (EFFECT_*_REPLACE): the op body
    // belongs to the corpus-generated `replacement` block; filling an
    // effect-block resolve with it duplicates the semantics in the
    // wrong surface (c39996157's shipped `banish self`). Consume the
    // index, decline the fill.
    if eff.is_replacement_chain() {
        let code = eff.first_arg_of("SetCode").unwrap_or("").to_string();
        return skip(FillReason::ReplacementChain, code);
    }
    let Some(block_idx) = assign.by_effect[eff_i] else {
        // Hazard-gated and not rescued: a clone / bare-activate chain
        // owns a .ds block before this one (or a continuous listener
        // consumed an index without owning a block — T38 S9) and the
        // block signatures don't force a unique home — filling would
        // risk landing in the wrong block.
        let why = if eff.block_alignment_hazard { "hazard unrescued" } else { "order-inconsistent" };
        return skip(FillReason::Unassigned, why.to_string());
    };
    // Phase 16 — SelectOption label-branch dispatch. When the target
    // handler picked an option (`Duel.SelectOption` + `e:SetLabel`)
    // and the operation handler splits on `e:GetLabel()` with one
    // fully-translatable arm per option, a `choose { option … }` block
    // REPLACES the empty resolve. Falls through to the normal fill
    // when the shape or a label doesn't resolve.
    let mut choose_why: Option<String> = None;
    if !eff.is_summon_helper() {
        let oh = eff.operation_handler.as_deref().unwrap_or_default().trim();
        if let Some(ob) = walk.functions.get(oh) {
            let tb = eff.target_handler.as_deref()
                .and_then(|th| walk.functions.get(th.trim()));
            match lua_ast::extract_choose_spec(tb, ob, &walk.functions) {
                Some(spec) => match card_id.and_then(|id| render_choose_block(&spec, id)) {
                    Some(text) => return FillPlan::Choose(text),
                    None => choose_why = Some("option label lookup failed".to_string()),
                },
                None => choose_why = choose_failure_detail(ob, tb),
            }
        }
    }
    let with_choose = |v: FillPlan| match v {
        FillPlan::Skip { reason, detail, .. } => FillPlan::Skip { reason, detail, choose_why },
        other => other,
    };
    // Special case: skeletons backed by a fusion/ritual summon helper —
    // the plain factories (Fusion.CreateSummonEff / Ritual.AddProc* /
    // Ritual.CreateProc, fixed line) and the parameterized SetOperation
    // forms (Fusion.SummonEffOP / Ritual.Operation, Phase 12). The
    // helper owns the UI / op pipeline, so a summon line replaces the
    // handler walk. Parameterized forms whose params don't decode skip
    // (the activation block exists in the .ds but its resolve stays an
    // empty stub — skip-not-mis-emit).
    let is_helper_fill = eff.is_summon_helper();
    let mut body: Option<&lua_ast::FunctionBody> = None;
    let lines: Vec<lua_ast::DslLine> = if is_helper_fill {
        match eff.summon_helper_verdict() {
            Ok(text) => vec![lua_ast::DslLine::Action(text)],
            Err(why) => return skip(FillReason::SummonHelperUndecodable, why),
        }
    } else {
        // Consumer without a helper spec has an operation handler
        // (checked at the top).
        let handler = eff.operation_handler.as_deref().unwrap_or_default().trim();
        match walk.functions.get(handler) {
            Some(b) => {
                // T38 S1 — completeness gate: a handler whose
                // translation drops a RegisterEffect chain would fill
                // an under-stated resolve. Skip the whole fill; the
                // dropped chain is the S1b/S8 backlog.
                let mut drops = lua_ast::body_drop_reasons(b, &walk.functions);
                if !drops.is_empty() {
                    let first = drops.remove(0);
                    // Remaining-blocker census (T39): the other drops
                    // plus the TODO lines the body walk would still
                    // emit — `+0 drops, 0 todo` means this ONE gate is
                    // the whole blocker.
                    let todo = lua_ast::translate_body_with_functions(b, &walk.functions)
                        .iter().filter(|l| !l.is_action()).count();
                    let detail = format!(
                        "{} | +{} drops, {} todo", first.detail(), drops.len(), todo
                    );
                    return with_choose(skip(FillReason::Drop(first), detail));
                }
                body = Some(b);
                lua_ast::translate_body_with_functions(b, &walk.functions)
            }
            None => return skip(FillReason::NoBody, handler.to_string()), // block exists, body unknown
        }
    };
    let n_actions = lines.iter().filter(|l| l.is_action()).count();
    if n_actions == 0 {
        if lines.is_empty() {
            let calls: Vec<&str> = body.map(|b| b.calls.iter().map(|c| c.method.as_str()).collect())
                .unwrap_or_default();
            let detail = if calls.is_empty() { "empty walk".to_string() }
                else { format!("calls: {}", calls.join(", ")) };
            return with_choose(skip(FillReason::NoLines, detail));
        }
        return with_choose(skip(FillReason::TodoOnly, todo_summary(&lines)));
    }
    // T38 S1 — a TODO line marks an untranslated Duel.* call in the
    // same handler; filling only the translated remainder silently
    // drops it. Skip-not-mis-emit.
    if n_actions < lines.len() {
        let detail = format!("{}/{} actions; {}", n_actions, lines.len(), todo_summary(&lines));
        return with_choose(skip(FillReason::Incomplete, detail));
    }
    let Some((block_lo, block_hi)) = nth_effect_block(txt, block_idx) else {
        return skip(FillReason::NoBlock, format!("block {}", block_idx));
    };
    let block = &txt[block_lo..block_hi];
    // T38 S6 review — cost-header parity: filling a resolve makes the
    // block's header LIVE, so a block-side cost must re-derive exactly
    // from the lua cost handler (skip class 10 — c9798352's `banish
    // self` header contradicts the lua's banish-a-Trap-from-Deck cost).
    if !cost_header_matches(block, eff, &walk.functions) {
        let detail = format!(
            "block {} vs lua cost {}",
            block_cost_text(block).map(|c| normalize_ws(&c)).unwrap_or_default(),
            eff.cost_handler.as_deref().unwrap_or("(none)").trim(),
        );
        return skip(FillReason::CostHeaderMismatch, detail);
    }
    // T38 S5 review — frame parity for summon-helper fills.
    if is_helper_fill && !helper_frame_matches(block, eff) {
        let detail = format!(
            "cond={} cost={}",
            eff.condition_handler.as_deref().unwrap_or("-").trim(),
            eff.cost_handler.as_deref().unwrap_or("-").trim(),
        );
        return skip(FillReason::HelperFrameMismatch, detail);
    }
    // T38 S6 — bare-target gate: a fill referencing `target` only
    // lands together with a target declaration; otherwise the
    // validator's bare-target check trades the empty-resolve error for
    // a new one. Blocks without one get the decl CO-EMITTED from the
    // SetTarget selector (refined extraction — exactly-mapped filters
    // only); when that fails, the whole fill skips.
    let mut co_target: Option<String> = None;
    if lines_reference_target(&lines) && !block_has_target_decl(block) {
        match target_decl_selector(eff, &walk.functions) {
            Ok(sel) => co_target = Some(sel),
            Err(detail) => return skip(FillReason::TargetDeclUndecodable, detail),
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
        set_current_card_from_stem(&stem);
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
                FillPlan::Skip { reason, .. } => { reason.count(&mut r); continue; }
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
        // T39 S8 — negate / cannot-trigger passives (EFFECT_DISABLE /
        // EFFECT_CANNOT_TRIGGER chains). Dedup key: the block's item
        // lines — a `passive` block anywhere in the file with the same
        // body (any name, any indent) counts as present, so a block
        // `audit-passives --stub` rewrote in place is never re-injected.
        for eff in &walk.effects {
            let Some(spec) = eff.passive_floodgate_spec(&walk) else { continue };
            if passive_body_present(&new_txt, &spec.body_lines()) { continue; }
            let name = format!("Passive {}", passive_idx);
            let block = spec.to_dsl_block(&name, "    ");
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

// ── T39 — explain ─────────────────────────────────────────────

/// Why the Phase 16 choose extraction refused a choose-SHAPED effect
/// (one whose op or target handler carries a SelectOption, or whose
/// op dispatches on `e:GetLabel()`). None for effects that aren't
/// choose-shaped at all.
fn choose_failure_detail(
    ob: &lua_ast::FunctionBody,
    tb: Option<&lua_ast::FunctionBody>,
) -> Option<String> {
    let sel = ob.select_option.as_ref().or_else(|| tb.and_then(|t| t.select_option.as_ref()));
    match sel {
        None if ob.choice_arms.is_empty() => None,
        None => Some("dynamic option list (no static SelectOption)".to_string()),
        Some(_) if ob.choice_arms.is_empty() => Some("no label dispatch arms".to_string()),
        Some(sel) if ob.choice_arms.len() != sel.string_ids.len() => Some(format!(
            "{} options vs {} arms", sel.string_ids.len(), ob.choice_arms.len()
        )),
        Some(_) => Some("an arm fails full-coverage translation".to_string()),
    }
}

/// The method name of a TODO line — `Duel.X(args) — why` → `Duel.X`.
fn todo_method(todo: &str) -> &str {
    todo.split('(').next().unwrap_or(todo).trim()
}

/// `todo: Duel.A, Duel.B | first: Duel.A(args) — why` — the distinct
/// unmapped methods in line order plus the first TODO verbatim.
fn todo_summary(lines: &[lua_ast::DslLine]) -> String {
    let todos: Vec<&str> = lines.iter().filter_map(|l| match l {
        lua_ast::DslLine::Todo(t) => Some(t.as_str()),
        lua_ast::DslLine::Action(_) => None,
    }).collect();
    let mut methods: Vec<&str> = Vec::new();
    for t in &todos {
        let m = todo_method(t);
        if !methods.contains(&m) { methods.push(m); }
    }
    format!("todo: {} | first: {}", methods.join(", "), todos.first().copied().unwrap_or(""))
}

/// Card types whose activation effect the validator exempts from the
/// empty-resolve error (a Continuous / Field / Equip card's activation
/// just places it; the real work is in `passive` blocks) — when the
/// block carries no trigger line and no cost.
fn is_permanent_passive_card(txt: &str) -> bool {
    let line = txt.lines().map(str::trim_start).find(|l| l.starts_with("type:")).unwrap_or("");
    ["Continuous Spell", "Field Spell", "Equip Spell", "Continuous Trap"]
        .iter().any(|t| line.contains(t))
}

/// Effect blocks the validator flags as "must have a resolve or choose
/// block": no `choose`, and either no `resolve` or an empty one.
/// Returns `(flagged block indices, exempt count)` — exempt blocks are
/// the permanent-passive activation shells the validator skips.
fn problem_blocks(txt: &str) -> (Vec<usize>, usize) {
    let passive_card = is_permanent_passive_card(txt);
    let mut out = Vec::new();
    let mut exempt = 0usize;
    for (idx, (lo, hi)) in block_match::effect_block_ranges(txt).into_iter().enumerate() {
        let block = &txt[lo..hi];
        if block.contains("choose {") { continue; }
        let empty = match block_match::resolve_inner_span(txt, lo, hi) {
            Some((a, b)) => txt[a..b].trim().is_empty(),
            None => true,
        };
        if !empty { continue; }
        if passive_card {
            let sig = &block_match::parse_block_sigs(&txt[lo..hi])[0];
            if !sig.trigger_line && !sig.has_cost {
                exempt += 1;
                continue;
            }
        }
        out.push(idx);
    }
    (out, exempt)
}

/// One `explain` line per flagged block of one card. `lua_src` is the
/// card's script when it exists. Lines are tab-separated:
/// `card  block  owner|unassigned  reason  detail  type  stmts`.
fn explain_card(stem: &str, txt: &str, lua_src: Option<&str>) -> (Vec<String>, usize) {
    let (problems, exempt) = problem_blocks(txt);
    if problems.is_empty() { return (Vec::new(), exempt); }
    let line = |b: usize, owner: &str, code: &str, detail: &str, etype: &str, stmts: &str| {
        format!("{}\t{}\t{}\t{}\t{}\t{}\t{}", stem, b, owner, code, normalize_ws(detail), etype, stmts)
    };
    let every = |code: &str, detail: &str| -> Vec<String> {
        problems.iter().map(|&b| line(b, "unassigned", code, detail, "-", "-")).collect()
    };
    let Some(lua_src) = lua_src else { return (every("unassigned", "no-lua"), exempt) };
    let Ok(parsed) = full_moon::parse(lua_src) else { return (every("unassigned", "lua-parse-error"), exempt) };
    let walk = lua_ast::walk(&parsed);
    if walk.effects.is_empty() { return (every("unassigned", "no-lua-effects"), exempt); }
    let assign = block_match::compute_assignments(&walk, txt);
    let n_blocks = block_match::effect_block_ranges(txt).len();
    let mut owner: Vec<Option<usize>> = vec![None; n_blocks];
    for (eff_i, b) in assign.by_effect.iter().enumerate() {
        if let Some(b) = b { if *b < n_blocks { owner[*b] = Some(eff_i); } }
    }
    let card_id: Option<u32> = stem.strip_prefix('c').and_then(|s| s.parse().ok());
    let consumers: Vec<&lua_ast::EffectSkeleton> = walk.effects.iter()
        .filter(|e| e.is_summon_helper() || e.operation_handler.is_some()).collect();
    let any_hazard = walk.effects.iter().any(|e| e.block_alignment_hazard);
    let unassigned: Vec<&str> = walk.effects.iter().enumerate()
        .filter(|(i, e)| (e.is_summon_helper() || e.operation_handler.is_some()) && assign.by_effect[*i].is_none())
        .map(|(_, e)| e.binding.as_str()).collect();
    let mut out = Vec::new();
    for &b in &problems {
        let Some(eff_i) = owner[b] else {
            let detail = if !consumers.is_empty() && assign.by_effect.iter().all(Option::is_none) && !any_hazard {
                format!("order-inconsistent ({} consumers, {} blocks)", consumers.len(), n_blocks)
            } else if !unassigned.is_empty() {
                format!("hazard unrescued; unassigned consumers: {}", unassigned.join(","))
            } else {
                format!("no consumer for block index ({} consumers, {} blocks)", consumers.len(), n_blocks)
            };
            out.push(line(b, "unassigned", "unassigned", &detail, "-", "-"));
            continue;
        };
        let eff = &walk.effects[eff_i];
        let etype = eff.first_arg_of("SetType").unwrap_or("-");
        let stmts = eff.operation_handler.as_deref()
            .and_then(|h| walk.functions.get(h.trim()))
            .map(|b| (b.calls.len() + b.helper_calls.len() + b.counter_ops.len()
                      + b.method_ops.len() + b.register_chains.len()).to_string())
            .unwrap_or_else(|| "-".to_string());
        let (code, detail) = match plan_fill(eff_i, eff, &walk, &assign, txt, card_id) {
            FillPlan::Choose(_) => ("ok".to_string(), "choose would fill".to_string()),
            FillPlan::Fill { lines, .. } => (
                "ok".to_string(),
                format!("would fill: {}", lines.into_iter().map(|l| l.into_string("")).collect::<Vec<_>>().join("; ")),
            ),
            FillPlan::Skip { reason, detail, choose_why: Some(why) } => (
                "choose-undecodable".to_string(),
                format!("{}; then {}: {}", why, reason.code(), detail),
            ),
            FillPlan::Skip { reason, detail, choose_why: None } => (reason.code().to_string(), detail),
        };
        out.push(line(b, &eff.binding, &code, &detail, etype, &stmts));
    }
    (out, exempt)
}

/// See the `explain` mode arm. Prints one line per flagged block to
/// stdout and a summary to stderr.
fn explain(corpus_dir: &str, lua_dir: &str) {
    let mut paths: Vec<std::path::PathBuf> = match fs::read_dir(corpus_dir) {
        Ok(e) => e.flatten().map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
            .collect(),
        Err(e) => { eprintln!("cannot read {}: {}", corpus_dir, e); return; }
    };
    paths.sort();
    let (mut blocks, mut cards, mut exempt_total) = (0usize, 0usize, 0usize);
    for path in paths {
        let Ok(txt) = fs::read_to_string(&path) else { continue };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        set_current_card_from_stem(&stem);
        let lua_src = fs::read_to_string(Path::new(lua_dir).join(format!("{}.lua", stem))).ok();
        let (lines, exempt) = explain_card(&stem, &txt, lua_src.as_deref());
        exempt_total += exempt;
        if lines.is_empty() { continue; }
        cards += 1;
        blocks += lines.len();
        for l in lines { println!("{}", l); }
    }
    eprintln!("=== explain: {} blocks across {} cards ({} permanent-passive activation shells exempt) ===",
        blocks, cards, exempt_total);
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
/// extraction — Phase 8's nil/aux.TRUE shapes plus custom filters
/// resolved through the shared selector mapper with the owning
/// operation handler's target-consuming actions as the transparent-
/// legality context (T39 S9). Err names the refusal ⇒ the fill skips.
fn target_decl_selector(
    eff: &lua_ast::EffectSkeleton,
    functions: &std::collections::BTreeMap<String, lua_ast::FunctionBody>,
) -> Result<String, String> {
    let th = eff.target_handler.as_deref().ok_or_else(|| "(no SetTarget)".to_string())?;
    let consumers = eff.operation_handler.as_deref()
        .and_then(|oh| functions.get(oh.trim()))
        .map(lua_ast::target_consumers)
        .unwrap_or_default();
    Ok(lua_ast::extract_target_decl_refined(th.trim(), functions, &consumers)?.to_dsl())
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

// ── T39 — fill-verdict tests ──────────────────────────────────────
//
// Every Pass A skip path on a minimal synthetic lua + .ds pair, plus
// the `explain` line format. Run with
// `cargo test --features lua_ast,cdb --bin lua_translate`.
#[cfg(test)]
mod tests {
    use super::*;

    fn walk_src(src: &str) -> lua_ast::LuaReport {
        lua_ast::walk(&full_moon::parse(src).expect("parse"))
    }

    // ── T39 S8 — audit-passives ────────────────────────────────

    /// Skill Drain's lua, minus comments.
    const S8_LUA: &str = "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_ACTIVATE)\n\te1:SetCode(EVENT_FREE_CHAIN)\n\tc:RegisterEffect(e1)\n\tlocal e2=Effect.CreateEffect(c)\n\te2:SetType(EFFECT_TYPE_FIELD)\n\te2:SetRange(LOCATION_SZONE)\n\te2:SetTargetRange(LOCATION_MZONE,LOCATION_MZONE)\n\te2:SetTarget(s.disable)\n\te2:SetCode(EFFECT_DISABLE)\n\tc:RegisterEffect(e2)\nend\nfunction s.disable(e,c)\n\treturn c:IsType(TYPE_EFFECT) or (c:GetOriginalType()&TYPE_EFFECT)==TYPE_EFFECT\nend\n";
    const S8_LEGACY: &str = "    passive \"Disable Monster Effects\" {\n        scope: self\n        grant: cannot_activate effects\n    }\n";
    const S8_CURRENT: &str = "    passive \"Disable Monster Effects\" {\n        scope: field\n        target: (all, card, either controls, from monster_zone, where is_effect)\n        negate_effects\n    }\n";

    fn s8_ds(blocks: &[&str]) -> String {
        format!("card \"Skill Drain\" {{\n    id: 82732705\n    type: Continuous Trap\n\n{}}}\n", blocks.join("\n"))
    }

    #[test]
    fn audit_passives_rewrites_legacy_block_in_place_and_removes_surplus() {
        // Two legacy blocks, one chain that renders: the first is
        // rewritten (name kept), the second removed; the activation
        // shell sees no CHAIN verdict (not on the three codes).
        let txt = s8_ds(&[S8_LEGACY, &S8_LEGACY.replace("Disable Monster Effects", "Effect 2")]);
        let walk = walk_src(S8_LUA);
        let (verdicts, plan, new_txt) = plan_passive_retrofit(&txt, &walk, S8_LUA);
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].0, "e2");
        assert!(verdicts[0].1.is_ok());
        assert_eq!(plan.len(), 2);
        assert!(matches!(plan[0].2, PassiveRetrofit::Rewrite(_)));
        assert_eq!(plan[0].1, "Disable Monster Effects");
        assert_eq!(plan[1].2, PassiveRetrofit::Remove);
        assert_eq!(new_txt, s8_ds(&[S8_CURRENT]));
        // Fixed point: the rewritten file has no legacy block and Pass
        // B's dedup sees the rendering as present.
        let (_, plan2, txt2) = plan_passive_retrofit(&new_txt, &walk, S8_LUA);
        assert!(plan2.is_empty());
        assert_eq!(txt2, new_txt);
        let body = walk.effects[1].passive_floodgate_spec(&walk).unwrap().body_lines();
        assert!(passive_body_present(&new_txt, &body));
        assert!(!passive_body_present(&txt, &body));
    }

    #[test]
    fn audit_passives_removes_legacy_block_when_no_chain_renders() {
        // Shell Knight class: the generator's block came from a chain
        // inside an operation handler — nothing in initial_effect
        // renders, the block is removed and the file keeps one blank
        // line between the survivors.
        let lua = "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_IGNITION)\n\te1:SetOperation(s.op)\n\tc:RegisterEffect(e1)\nend\nfunction s.op(e,tp,eg,ep,ev,re,r,rp)\n\tlocal e1=Effect.CreateEffect(e:GetHandler())\n\te1:SetType(EFFECT_TYPE_FIELD)\n\te1:SetCode(EFFECT_CANNOT_ACTIVATE)\n\te1:SetTargetRange(1,0)\n\te1:SetValue(1)\n\te1:SetReset(RESET_PHASE|PHASE_END)\n\tDuel.RegisterEffect(e1,tp)\nend\n";
        let effect = "    effect \"Effect 1\" {\n        speed: 1\n        resolve { }\n    }\n";
        let txt = s8_ds(&[&S8_LEGACY.replace("Disable Monster Effects", "Effect 2"), effect]);
        let walk = walk_src(lua);
        let (verdicts, plan, new_txt) = plan_passive_retrofit(&txt, &walk, lua);
        assert!(verdicts.is_empty());
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].2, PassiveRetrofit::Remove);
        assert_eq!(new_txt, s8_ds(&[effect]));
    }

    #[test]
    fn audit_passives_keeps_hand_content_and_reviews_unprovenanced_legacy_bodies() {
        // A hand-written passive with another body is never a hit; a
        // legacy body on a card whose lua names none of the codes is
        // REVIEW (report-only, text untouched).
        let hand = "    passive \"Hand\" {\n        scope: self\n        grant: cannot_attack\n    }\n";
        let lua = "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_SINGLE)\n\te1:SetCode(EFFECT_CANNOT_ATTACK)\n\tc:RegisterEffect(e1)\nend\n";
        let txt = s8_ds(&[hand, S8_LEGACY]);
        let walk = walk_src(lua);
        let (_, plan, new_txt) = plan_passive_retrofit(&txt, &walk, lua);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].2, PassiveRetrofit::Review);
        assert_eq!(new_txt, txt);
    }

    #[test]
    fn audit_passives_does_not_owe_a_rendering_a_non_legacy_block_already_carries() {
        // The current rendering already sits beside a legacy block (a
        // half-done hand retrofit): the legacy block is removed, not
        // rewritten into a duplicate.
        let txt = s8_ds(&[S8_CURRENT, &S8_LEGACY.replace("Disable Monster Effects", "Effect 2")]);
        let walk = walk_src(S8_LUA);
        let (_, plan, new_txt) = plan_passive_retrofit(&txt, &walk, S8_LUA);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].2, PassiveRetrofit::Remove);
        assert_eq!(new_txt, s8_ds(&[S8_CURRENT]));
    }

    /// A one-card .ds with `n` effect blocks; `extra` is injected into
    /// every block header (e.g. a `cost { … }`), and `resolve` picks
    /// between an empty resolve slot and none at all.
    fn ds(n: usize, extra: &str, resolve: bool, type_line: &str) -> String {
        let mut out = format!("card \"T\" {{\n    id: 1\n    type: {}\n", type_line);
        for i in 0..n {
            out.push_str(&format!("\n    effect \"Effect {}\" {{\n        speed: 1\n", i + 1));
            if !extra.is_empty() { out.push_str(&format!("        {}\n", extra)); }
            if resolve { out.push_str("        resolve { }\n"); }
            out.push_str("    }\n");
        }
        out.push_str("}\n");
        out
    }

    fn verdict(src: &str, ds_txt: &str, eff_i: usize) -> FillPlan {
        let walk = walk_src(src);
        let assign = block_match::compute_assignments(&walk, ds_txt);
        plan_fill(eff_i, &walk.effects[eff_i], &walk, &assign, ds_txt, Some(1))
    }

    fn skip_of(v: FillPlan) -> (FillReason, String, Option<String>) {
        match v {
            FillPlan::Skip { reason, detail, choose_why } => (reason, detail, choose_why),
            other => panic!("expected Skip, got {:?}", other),
        }
    }

    /// `s.initial_effect` with one ignition chain on `s.op` plus the
    /// handler body.
    fn ignition(op_body: &str) -> String {
        format!(
            "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_IGNITION)\n\te1:SetOperation(s.op)\n\tc:RegisterEffect(e1)\nend\nfunction s.op(e,tp,eg,ep,ev,re,r,rp)\n{}\nend\n",
            op_body
        )
    }

    #[test]
    fn verdict_no_handler_for_pure_passive() {
        let src = "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_SINGLE)\n\te1:SetCode(EFFECT_UPDATE_ATTACK)\n\te1:SetValue(500)\n\tc:RegisterEffect(e1)\nend\n";
        let (reason, detail, _) = skip_of(verdict(src, &ds(1, "", true, "Effect Monster"), 0));
        assert_eq!(reason, FillReason::NoHandler);
        assert_eq!(detail, "e1");
        assert_eq!(reason.code(), "no-handler");
    }

    #[test]
    fn verdict_replacement_chain() {
        let src = "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_CONTINUOUS+EFFECT_TYPE_SINGLE)\n\te1:SetCode(EFFECT_DESTROY_REPLACE)\n\te1:SetTarget(s.reptg)\n\te1:SetOperation(s.repop)\n\tc:RegisterEffect(e1)\nend\nfunction s.repop(e,tp,eg,ep,ev,re,r,rp)\n\tDuel.Draw(tp,1,REASON_EFFECT)\nend\n";
        let (reason, detail, _) = skip_of(verdict(src, &ds(1, "", true, "Effect Monster"), 0));
        assert_eq!(reason, FillReason::ReplacementChain);
        assert_eq!(detail, "EFFECT_DESTROY_REPLACE");
    }

    #[test]
    fn verdict_unassigned_order_inconsistent() {
        // Binding order (e1, e2) disagrees with source order (e2, e1):
        // the matcher refuses every assignment.
        let src = "function s.initial_effect(c)\n\tlocal e2=Effect.CreateEffect(c)\n\te2:SetType(EFFECT_TYPE_IGNITION)\n\te2:SetOperation(s.op2)\n\tc:RegisterEffect(e2)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_IGNITION)\n\te1:SetOperation(s.op1)\n\tc:RegisterEffect(e1)\nend\nfunction s.op1(e,tp,eg,ep,ev,re,r,rp)\n\tDuel.Draw(tp,1,REASON_EFFECT)\nend\nfunction s.op2(e,tp,eg,ep,ev,re,r,rp)\n\tDuel.Draw(tp,2,REASON_EFFECT)\nend\n";
        let txt = ds(2, "", true, "Effect Monster");
        let (reason, detail, _) = skip_of(verdict(src, &txt, 0));
        assert_eq!(reason, FillReason::Unassigned);
        assert_eq!(detail, "order-inconsistent");
    }

    #[test]
    fn verdict_summon_helper_undecodable() {
        // Own-level EQUAL ritual proc: inexpressible without a
        // summon_level atom — the helper decoder refuses.
        let src = "function s.initial_effect(c)\n\tlocal e1=Ritual.AddProcEqual{handler=c,filter=aux.FilterBoolFunction(Card.IsSetCard,SET_SHADDOLL)}\nend\n";
        let (reason, detail, _) = skip_of(verdict(src, &ds(1, "", true, "Ritual Spell"), 0));
        assert_eq!(reason, FillReason::SummonHelperUndecodable);
        assert!(detail.starts_with("params:"), "{}", detail);
    }

    #[test]
    fn verdict_no_body_for_library_handler() {
        let src = "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_IGNITION)\n\te1:SetOperation(aux.NoOperation)\n\tc:RegisterEffect(e1)\nend\n";
        let (reason, detail, _) = skip_of(verdict(src, &ds(1, "", true, "Effect Monster"), 0));
        assert_eq!(reason, FillReason::NoBody);
        assert_eq!(detail, "aux.NoOperation");
    }


    #[test]
    fn verdict_selector_filter_unmapped() {
        // T39 S3 — an action selector whose lua filter carries an
        // atom the DSL cannot state refuses the fill; the pre-gate
        // policy rendered the unfiltered superset.
        let src = format!(
            "function s.filter(c)\n\treturn c:IsSpecialSummoned()\nend\n{}",
            ignition("\tlocal g=Duel.GetMatchingGroup(s.filter,tp,0,LOCATION_MZONE,nil)\n\tDuel.Destroy(g,REASON_EFFECT)")
        );
        let (reason, detail, _) = skip_of(verdict(&src, &ds(1, "", true, "Effect Monster"), 0));
        assert!(matches!(reason, FillReason::Drop(lua_ast::BodyDrop::SelectorFilterUnmapped(_))), "{:?}", reason);
        assert_eq!(reason.code(), "selector-filter-unmapped");
        assert_eq!(detail, "Duel.Destroy(g): IsSpecialSummoned() | +0 drops, 1 todo");
        // A mapped position atom fills instead (Mirror Force shape).
        let ok = src.replace("c:IsSpecialSummoned()", "c:IsAttackPos()");
        match verdict(&ok, &ds(1, "", true, "Effect Monster"), 0) {
            FillPlan::Fill { lines, .. } => assert_eq!(
                lines.into_iter().map(|l| l.into_string("")).collect::<Vec<_>>(),
                vec!["destroy (all, card, opponent controls, from monster_zone, in attack_position)"],
            ),
            other => panic!("expected Fill, got {:?}", other),
        }
    }
    #[test]
    fn verdict_method_poison_names_the_statement() {
        let src = ignition("\tlocal c=e:GetHandler()\n\tc:RegisterFlagEffect(id,RESET_EVENT|RESETS_STANDARD,0,1)");
        let (reason, detail, _) = skip_of(verdict(&src, &ds(1, "", true, "Effect Monster"), 0));
        assert!(matches!(reason, FillReason::Drop(lua_ast::BodyDrop::MethodPoison(_))), "{:?}", reason);
        assert_eq!(reason.code(), "method-poison");
        assert!(detail.starts_with("c:RegisterFlagEffect(id,RESET_EVENT|RESETS_STANDARD,0,1) | +0 drops, 0 todo"), "{}", detail);
    }

    #[test]
    fn verdict_chain_unlowered_with_census() {
        let src = ignition("\tlocal c=e:GetHandler()\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_SINGLE)\n\te1:SetCode(EFFECT_UPDATE_ATTACK)\n\te1:SetValue(e:GetLabel())\n\te1:SetReset(RESET_EVENT|RESETS_STANDARD|RESET_PHASE|PHASE_END)\n\tc:RegisterEffect(e1)");
        let (reason, detail, _) = skip_of(verdict(&src, &ds(1, "", true, "Effect Monster"), 0));
        assert!(matches!(reason, FillReason::Drop(lua_ast::BodyDrop::ChainUnlowered(_))), "{:?}", reason);
        assert_eq!(reason.code(), "chain-unlowered");
        assert!(detail.contains("code=EFFECT_UPDATE_ATTACK type=EFFECT_TYPE_SINGLE"), "{}", detail);
        assert!(detail.contains("value=e:GetLabel()"), "{}", detail);
        assert!(detail.ends_with("| +0 drops, 0 todo"), "{}", detail);
    }

    #[test]
    fn verdict_no_lines_lists_skipped_calls() {
        let src = ignition("\tDuel.Hint(HINT_CARD,0,id)");
        let (reason, detail, _) = skip_of(verdict(&src, &ds(1, "", true, "Effect Monster"), 0));
        assert_eq!(reason, FillReason::NoLines);
        assert_eq!(detail, "calls: Duel.Hint");
    }

    #[test]
    fn verdict_todo_only_names_first_unmapped_call() {
        let src = ignition("\tDuel.SwapSequence(a,b)");
        let (reason, detail, _) = skip_of(verdict(&src, &ds(1, "", true, "Effect Monster"), 0));
        assert_eq!(reason, FillReason::TodoOnly);
        assert_eq!(detail, "todo: Duel.SwapSequence | first: Duel.SwapSequence(a, b)");
    }

    #[test]
    fn verdict_incomplete_counts_actions() {
        let src = ignition("\tDuel.Draw(tp,1,REASON_EFFECT)\n\tDuel.SwapSequence(a,b)");
        let (reason, detail, _) = skip_of(verdict(&src, &ds(1, "", true, "Effect Monster"), 0));
        assert_eq!(reason, FillReason::Incomplete);
        assert_eq!(detail, "1/2 actions; todo: Duel.SwapSequence | first: Duel.SwapSequence(a, b)");
    }

    #[test]
    fn verdict_no_block_beyond_ds_blocks() {
        let src = "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_IGNITION)\n\te1:SetOperation(s.op)\n\tc:RegisterEffect(e1)\n\tlocal e2=Effect.CreateEffect(c)\n\te2:SetType(EFFECT_TYPE_IGNITION)\n\te2:SetOperation(s.op)\n\tc:RegisterEffect(e2)\nend\nfunction s.op(e,tp,eg,ep,ev,re,r,rp)\n\tDuel.Draw(tp,1,REASON_EFFECT)\nend\n";
        let txt = ds(1, "", true, "Effect Monster");
        assert!(matches!(verdict(src, &txt, 0), FillPlan::Fill { .. }));
        let (reason, detail, _) = skip_of(verdict(src, &txt, 1));
        assert_eq!(reason, FillReason::NoBlock);
        assert_eq!(detail, "block 1");
    }

    #[test]
    fn verdict_cost_header_mismatch() {
        let src = ignition("\tDuel.Draw(tp,1,REASON_EFFECT)");
        let txt = ds(1, "cost { banish self }", true, "Effect Monster");
        let (reason, detail, _) = skip_of(verdict(&src, &txt, 0));
        assert_eq!(reason, FillReason::CostHeaderMismatch);
        assert_eq!(detail, "block cost { banish self } vs lua cost (none)");
    }

    #[test]
    fn verdict_helper_frame_mismatch() {
        // Variant-B fusion frame with a SetCondition the bare block
        // header (speed only) models nothing of.
        let src = "function s.initial_effect(c)\n\tlocal e1=Effect.CreateEffect(c)\n\te1:SetType(EFFECT_TYPE_ACTIVATE)\n\te1:SetCode(EVENT_FREE_CHAIN)\n\te1:SetCondition(s.cond)\n\te1:SetTarget(Fusion.SummonEffTG())\n\te1:SetOperation(Fusion.SummonEffOP())\n\tc:RegisterEffect(e1)\nend\n";
        let txt = ds(1, "", true, "Normal Spell");
        let (reason, detail, _) = skip_of(verdict(src, &txt, 0));
        assert_eq!(reason, FillReason::HelperFrameMismatch);
        assert_eq!(detail, "cond=s.cond cost=-");
    }

    #[test]
    fn verdict_target_decl_undecodable() {
        let src = ignition("\tlocal tc=Duel.GetFirstTarget()\n\tDuel.Destroy(tc,REASON_EFFECT)");
        let txt = ds(1, "", true, "Effect Monster");
        let (reason, detail, _) = skip_of(verdict(&src, &txt, 0));
        assert_eq!(reason, FillReason::TargetDeclUndecodable);
        assert_eq!(detail, "(no SetTarget)");
    }

    #[test]
    fn verdict_lines_when_every_gate_passes() {
        let src = ignition("\tDuel.Draw(tp,1,REASON_EFFECT)");
        match verdict(&src, &ds(1, "", true, "Effect Monster"), 0) {
            FillPlan::Fill { lines, co_target } => {
                assert_eq!(co_target, None);
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0].clone().into_string(""), "draw 1");
            }
            other => panic!("expected Fill, got {:?}", other),
        }
    }

    #[test]
    fn reason_counters_match_pass_a_history() {
        let mut r = ApplyReport::default();
        FillReason::NoHandler.count(&mut r);
        FillReason::SummonHelperUndecodable.count(&mut r);
        FillReason::NoBody.count(&mut r);
        assert_eq!(r.effects_no_handler, 3);
        FillReason::ReplacementChain.count(&mut r);
        FillReason::Drop(lua_ast::BodyDrop::CounterPoison).count(&mut r);
        FillReason::Incomplete.count(&mut r);
        FillReason::CostHeaderMismatch.count(&mut r);
        FillReason::HelperFrameMismatch.count(&mut r);
        FillReason::TargetDeclUndecodable.count(&mut r);
        assert_eq!(r.effects_incomplete, 6);
        FillReason::Unassigned.count(&mut r);
        assert_eq!(r.effects_alignment_hazard, 1);
        FillReason::NoLines.count(&mut r);
        FillReason::TodoOnly.count(&mut r);
        assert_eq!(r.effects_todo_only, 2);
        FillReason::NoBlock.count(&mut r);
        assert_eq!(
            r.effects_no_handler + r.effects_incomplete + r.effects_alignment_hazard + r.effects_todo_only,
            12,
        );
    }

    #[test]
    fn choose_failure_detail_classifies_shapes() {
        let plain = lua_ast::FunctionBody::default();
        assert_eq!(choose_failure_detail(&plain, None), None);
        let arms_no_sel = lua_ast::FunctionBody {
            choice_arms: vec![lua_ast::FunctionBody::default(), lua_ast::FunctionBody::default()],
            ..Default::default()
        };
        assert_eq!(
            choose_failure_detail(&arms_no_sel, None).as_deref(),
            Some("dynamic option list (no static SelectOption)"),
        );
        let sel = lua_ast::SelectOptionSpec { string_ids: vec![0, 1, 2] };
        let sel_no_arms = lua_ast::FunctionBody { select_option: Some(sel.clone()), ..Default::default() };
        assert_eq!(choose_failure_detail(&sel_no_arms, None).as_deref(), Some("no label dispatch arms"));
        let mismatch = lua_ast::FunctionBody {
            select_option: Some(sel),
            choice_arms: vec![lua_ast::FunctionBody::default(), lua_ast::FunctionBody::default()],
            ..Default::default()
        };
        assert_eq!(choose_failure_detail(&mismatch, None).as_deref(), Some("3 options vs 2 arms"));
    }

    #[test]
    fn explain_card_line_format() {
        let src = ignition("\tDuel.SwapSequence(a,b)");
        let txt = ds(1, "", true, "Effect Monster");
        let (lines, exempt) = explain_card("c1", &txt, Some(&src));
        assert_eq!(exempt, 0);
        assert_eq!(
            lines,
            vec!["c1\t0\te1\ttodo-only\ttodo: Duel.SwapSequence | first: Duel.SwapSequence(a, b)\tEFFECT_TYPE_IGNITION\t1".to_string()],
        );
    }

    #[test]
    fn explain_card_no_resolve_block_and_no_lua() {
        let txt = ds(1, "", false, "Effect Monster");
        let (lines, _) = explain_card("c1", &txt, None);
        assert_eq!(lines, vec!["c1\t0\tunassigned\tunassigned\tno-lua\t-\t-".to_string()]);
    }

    #[test]
    fn explain_card_unassigned_no_consumer_detail() {
        // Two .ds blocks, one lua consumer: block 1 has no owner.
        let src = ignition("\tDuel.Draw(tp,1,REASON_EFFECT)");
        let txt = ds(2, "", true, "Effect Monster");
        let (lines, _) = explain_card("c1", &txt, Some(&src));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("c1\t0\te1\tok\twould fill: draw 1\t"), "{}", lines[0]);
        assert_eq!(lines[1], "c1\t1\tunassigned\tunassigned\tno consumer for block index (1 consumers, 2 blocks)\t-\t-");
    }

    #[test]
    fn explain_card_exempts_permanent_passive_activation() {
        let src = ignition("\tDuel.SwapSequence(a,b)");
        let txt = ds(1, "", true, "Continuous Spell");
        let (lines, exempt) = explain_card("c1", &txt, Some(&src));
        assert!(lines.is_empty());
        assert_eq!(exempt, 1);
        // A trigger line makes the block a real effect again.
        let txt = ds(1, "trigger: when summoned", true, "Continuous Spell");
        let (lines, exempt) = explain_card("c1", &txt, Some(&src));
        assert_eq!(lines.len(), 1);
        assert_eq!(exempt, 0);
    }

    #[test]
    fn explain_card_skips_filled_blocks() {
        let src = ignition("\tDuel.SwapSequence(a,b)");
        let txt = ds(1, "", true, "Effect Monster").replace("resolve { }", "resolve {\n            draw 1\n        }");
        let (lines, exempt) = explain_card("c1", &txt, Some(&src));
        assert!(lines.is_empty());
        assert_eq!(exempt, 0);
    }

    #[test]
    fn audit_fills_shape_compares_only_the_named_action() {
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // Same change_position lines → no reshape, even with a sibling drift.
        assert!(!shape_lines_differ(
            &s(&["change_position target", "draw 1"]),
            &s(&["change_position target", "draw 2"]),
            "change_position",
        ));
        // Bare → `to` form moved: reshape.
        assert!(shape_lines_differ(
            &s(&["change_position target"]),
            &s(&["change_position target to defense_position"]),
            "change_position",
        ));
        // Rendering refused (TODO-only → no action lines): reshape.
        assert!(shape_lines_differ(&s(&["change_position self"]), &s(&[]), "change_position"));
        // No shipped line of the shape: never a reshape.
        assert!(!shape_lines_differ(&s(&["draw 1"]), &s(&[]), "change_position"));
        // Prefix discipline: `change_position_x` is not `change_position`.
        assert!(!shape_lines_differ(&s(&["change_positionx self"]), &s(&[]), "change_position"));
    }

    #[test]
    fn classify_decl_recognises_both_legacy_renderings() {
        let cur = "target (1, card, either controls, from field, except self)";
        let legacies = [
            ("s9-decl", Some("target (1, card, either controls, from field, where is_face_up)".to_string())),
            ("pre-s1-exc", Some("target (1, card, either controls, from field)".to_string())),
        ];
        assert_eq!(classify_decl(cur, Some(cur), &legacies), DeclVerdict::Healthy);
        assert_eq!(
            classify_decl("target (1, card, either controls, from field)", Some(cur), &legacies),
            DeclVerdict::Rewrite("pre-s1-exc")
        );
        assert_eq!(
            classify_decl("target (1, card, either controls, from field, where is_face_up)", Some(cur), &legacies),
            DeclVerdict::Rewrite("s9-decl")
        );
        assert_eq!(classify_decl("target (1, monster, you control)", Some(cur), &legacies), DeclVerdict::Review);
        // A refused current rendering: a legacy match still reports, anything else is silent.
        assert_eq!(
            classify_decl("target (1, card, either controls, from field)", None, &legacies),
            DeclVerdict::Rewrite("pre-s1-exc")
        );
        assert_eq!(classify_decl("target (1, monster, you control)", None, &legacies), DeclVerdict::Healthy);
    }
}
