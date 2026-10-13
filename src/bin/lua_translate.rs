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

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: lua_translate <dump|translate|apply|audit-fills|refine-stubs|rederive> <path>");
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
        "refine-stubs" => {
            // Placeholder-resolve refinement: re-derive the aligned
            // handler of every effect block whose resolve is made only
            // of generator-era category placeholders and replace the
            // placeholder when the fill is complete and states every
            // category the placeholder did. Dry run by default; `--apply`
            // rewrites. See [`refine_stubs`].
            //
            //   lua_translate refine-stubs <corpus_dir> <lua_dir> [cards.cdb] [--apply]
            if args.len() < 4 {
                eprintln!("usage: lua_translate refine-stubs <corpus_dir> <lua_dir> [cards.cdb] [--apply]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            let apply = args.iter().any(|a| a == "--apply");
            let cdb = args.get(4).map(String::as_str).filter(|a| *a != "--apply");
            load_card_names(cdb, lua_dir);
            refine_stubs(corpus, lua_dir, apply);
        }
        "rederive" => {
            // Policy-flip retrofit audit for ACCEPTED fills (`audit-fills`
            // covers the refused ones): re-derive every shipped non-empty,
            // non-placeholder resolve under the current translator and
            // report the ones whose rendering differs. `--apply` rewrites
            // only the `UPGRADE` class — shipped lines equal to the
            // rendering under an earlier spelling (no zone list, no
            // summon position); `--stub-todo=<text>` empties fills the
            // gate now refuses with a TODO rider containing <text> when
            // the shipped lines are the translator's own action lines.
            // Everything else is report-only. See [`rederive`].
            //
            //   lua_translate rederive <corpus_dir> <lua_dir> [cards.cdb] [--apply] [--stub-todo=<text>]
            if args.len() < 4 {
                eprintln!("usage: lua_translate rederive <corpus_dir> <lua_dir> [cards.cdb] [--apply] [--stub-todo=<text>]");
                process::exit(2);
            }
            let corpus = &args[2];
            let lua_dir = &args[3];
            let apply = args.iter().any(|a| a == "--apply");
            let stub_todo = args.iter().find_map(|a| a.strip_prefix("--stub-todo="));
            let cdb = args.get(4).map(String::as_str).filter(|a| !a.starts_with("--"));
            load_card_names(cdb, lua_dir);
            rederive(corpus, lua_dir, apply, stub_todo);
        }
        _ => {
            eprintln!("unknown mode: {}", mode);
            process::exit(2);
        }
    }
}

/// Byte span of a `resolve { … }` body inside `[lo, hi)` — brace-matched
/// (install_watcher action lines carry `{ }` pairs), string-aware.
/// Returns `(after_open_brace, at_close_brace)`.
fn resolve_inner_span(txt: &str, lo: usize, hi: usize) -> Option<(usize, usize)> {
    let rel = txt[lo..hi].find("resolve {")?;
    let open = lo + rel + "resolve ".len(); // index of '{'
    let mut depth = 0i32;
    let mut in_string = false;
    for (i, c) in txt[open..hi].char_indices() {
        match c {
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + c.len_utf8(), open + i));
                }
            }
            _ => {}
        }
    }
    None
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
            let Some((inner_lo, inner_hi)) = resolve_inner_span(&new_txt, block_lo, block_hi) else { continue };
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
        // non-idempotent. The fill itself comes from [`derive_fill`], the
        // gate stack shared with Pass A2 and `refine-stubs`.
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            if !eff.is_summon_helper() && eff.operation_handler.is_none() {
                r.effects_no_handler += 1;
                continue; // pure-passive — no effect block in .ds
            }
            // T38 S4 — replacement-code chains (EFFECT_*_REPLACE): the
            // op body belongs to the corpus-generated `replacement`
            // block; filling an effect-block resolve with it duplicates
            // the semantics in the wrong surface (c39996157's shipped
            // `banish self`). Consume the index, decline the fill.
            if eff.is_replacement_chain() {
                r.effects_incomplete += 1;
                continue;
            }
            let Some(block_idx) = assign.by_effect[eff_i] else {
                // Hazard-gated and not rescued: a clone / bare-activate
                // chain owns a .ds block before this one (or a
                // continuous listener consumed an index without owning
                // a block — T38 S9) and the block signatures don't force
                // a unique home — filling would risk landing in the
                // wrong block.
                r.effects_alignment_hazard += 1;
                continue;
            };
            // Lines derive before the block lookup and the block-text
            // gates after it, so the skip counters classify every effect
            // the way the pre-refactor pass did (a positional index past
            // the last block still counts its todo / incomplete).
            let pre = match derive_fill_lines(eff, &walk, card_id) {
                Ok(pre) => pre,
                Err(FillSkip::NoHandler) => { r.effects_no_handler += 1; continue; }
                Err(FillSkip::TodoOnly) => { r.effects_todo_only += 1; continue; }
                Err(FillSkip::Incomplete) => { r.effects_incomplete += 1; continue; }
            };
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else {
                continue;
            };
            match pre {
                FillLines::Choose(choose_text) => {
                    // Phase 16 — a `choose { option … }` block REPLACES
                    // the empty resolve (the validator accepts either,
                    // never both).
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
                FillLines::Lines { lines, is_helper } => {
                    let co_target = match gate_fill_block(
                        &lines, is_helper, eff, &walk, &new_txt[block_lo..block_hi],
                    ) {
                        Ok(co_target) => co_target,
                        Err(_) => { r.effects_incomplete += 1; continue; }
                    };
                    if let Some((lo, hi)) = first_empty_resolve_within(&new_txt, block_lo, block_hi) {
                        let injection = render_fill(&lines, co_target.as_deref());
                        new_txt = format!("{}{}{}", &new_txt[..lo], injection, &new_txt[hi..]);
                        filled += 1;
                        if co_target.is_some() {
                            r.targets_injected += 1;
                        }
                    }
                }
            }
        }

        // Pass A2 — inject a fresh `resolve { … }` into effect blocks that
        // lack one entirely (validator: "must have a resolve or choose
        // block"). Only fires when the lua-ast translator emits at least
        // one ACTION line, so blocks without translator coverage stay
        // untouched. Effect blocks come from the shared Phase 20
        // assignment (pure-passive lua chains have no effect block and
        // are skipped upstream). Same gate stack as Pass A via
        // [`derive_fill`]; a choose-shaped handler already landed its
        // block in Pass A, so only plain line fills inject here.
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            if !eff.is_summon_helper() && eff.operation_handler.is_none() {
                continue; // pure-passive — no effect block in .ds
            }
            // T38 S4 — replacement-code chains: see Pass A.
            if eff.is_replacement_chain() { continue; }
            let Some(block_idx) = assign.by_effect[eff_i] else {
                continue; // see Pass A — no unambiguous block for this effect
            };
            let (block_lo, block_hi) = match nth_effect_block(&new_txt, block_idx) {
                Some(r) => r,
                None => continue,
            };
            let block = &new_txt[block_lo..block_hi];
            // Skip if the block already has a resolve or choose — Pass A
            // handles those, and we don't want to double-inject.
            if block.contains("resolve") || block.contains("choose") { continue; }
            let FillOutcome::Lines { lines, co_target } = derive_fill(eff, &walk, card_id, block) else {
                continue;
            };

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

        if filled > 0 || blocks_added > 0 || passives_added > 0 || conditions_added > 0 || costs_added > 0 || targets_added > 0 || retrofitted > 0 {
            r.effects_filled += filled;
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

// ── Shared fill derivation ───────────────────────────────────

/// Why a handler-derived fill was declined (Pass A report counters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FillSkip {
    /// Pure-passive chain, summon helper whose params don't decode, or
    /// a handler the walk never found — nothing to emit.
    NoHandler,
    /// The translator emitted nothing but TODO lines.
    TodoOnly,
    /// A gate refused the fill: replacement-code chain, dropped
    /// RegisterEffect chain, TODO rider beside translated lines,
    /// cost-header / helper-frame mismatch, or a bare `target` with no
    /// derivable declaration.
    Incomplete,
}

/// Stage one of a fill: what a walk effect derives on its own, before
/// any block-text gate.
enum FillLines {
    /// Phase 16 — `choose { … }` text sized to replace the resolve slot.
    Choose(String),
    /// Action lines; `is_helper` marks a summon-helper line (its frame
    /// parity gate runs in stage two).
    Lines { lines: Vec<lua_ast::DslLine>, is_helper: bool },
}

/// A resolve fill derived for one walk effect against its assigned
/// block — both stages applied.
enum FillOutcome {
    Choose(String),
    /// Action lines plus the target declaration to co-emit (T38 S6).
    Lines { lines: Vec<lua_ast::DslLine>, co_target: Option<String> },
    Skip(FillSkip),
}

/// Stage one: the lines a walk effect derives, with every gate that
/// needs no block text. Gate order is Pass A's: no-handler /
/// replacement chain → choose dispatch → summon-helper line →
/// completeness (dropped chains, TODO riders).
fn derive_fill_lines(
    eff: &lua_ast::EffectSkeleton,
    walk: &lua_ast::LuaReport,
    card_id: Option<u32>,
) -> Result<FillLines, FillSkip> {
    if !eff.is_summon_helper() && eff.operation_handler.is_none() {
        return Err(FillSkip::NoHandler); // pure-passive — no effect block in .ds
    }
    // T38 S4 — replacement-code chains: see Pass A.
    if eff.is_replacement_chain() {
        return Err(FillSkip::Incomplete);
    }
    // Phase 16 — SelectOption label-branch dispatch. When the target
    // handler picked an option (`Duel.SelectOption` + `e:SetLabel`) and
    // the operation handler splits on `e:GetLabel()` with one fully-
    // translatable arm per option, a `choose { option … }` block is the
    // fill. Falls through to the plain fill when the shape or a label
    // doesn't resolve.
    if !eff.is_summon_helper() {
        let choose_text = eff.operation_handler.as_deref().and_then(|oh| {
            let ob = walk.functions.get(oh.trim())?;
            let tb = eff.target_handler.as_deref()
                .and_then(|th| walk.functions.get(th.trim()));
            let spec = lua_ast::extract_choose_spec(tb, ob, &walk.functions)?;
            render_choose_block(&spec, card_id?)
        });
        if let Some(text) = choose_text {
            return Ok(FillLines::Choose(text));
        }
    }
    // Skeletons backed by a fusion/ritual summon helper — the plain
    // factories (Fusion.CreateSummonEff / Ritual.AddProc* /
    // Ritual.CreateProc, fixed line) and the parameterized SetOperation
    // forms (Fusion.SummonEffOP / Ritual.Operation, Phase 12). The
    // helper owns the UI / op pipeline, so a summon line replaces the
    // handler walk. Parameterized forms whose params don't decode stay
    // empty stubs (skip-not-mis-emit).
    let helper_line = eff.summon_helper_line();
    let is_helper = helper_line.is_some();
    let lines: Vec<lua_ast::DslLine> = if let Some(text) = helper_line {
        vec![lua_ast::DslLine::Action(text)]
    } else if eff.is_summon_helper() {
        return Err(FillSkip::NoHandler);
    } else {
        // Consumer without a helper spec has an operation handler
        // (checked at the top).
        let handler = eff.operation_handler.as_deref().unwrap_or_default();
        match walk.functions.get(handler.trim()) {
            Some(body) => {
                // T38 S1 — completeness gate: a handler whose
                // translation drops a RegisterEffect chain would fill an
                // under-stated resolve. Skip the whole fill; the dropped
                // chain is the S1b/S8 backlog.
                if lua_ast::body_drops_chains(body, &walk.functions) {
                    return Err(FillSkip::Incomplete);
                }
                lua_ast::translate_body_with_functions(body, &walk.functions)
            }
            None => return Err(FillSkip::NoHandler), // block exists, body unknown
        }
    };
    if !lines.iter().any(|l| l.is_action()) {
        return Err(FillSkip::TodoOnly);
    }
    // T38 S1 — a TODO line marks an untranslated Duel.* call in the
    // same handler; filling only the translated remainder silently
    // drops it. Skip-not-mis-emit.
    if lines.iter().any(|l| !l.is_action()) {
        return Err(FillSkip::Incomplete);
    }
    Ok(FillLines::Lines { lines, is_helper })
}

/// Stage two: the gates that read the assigned block's text — header
/// parity (cost, helper frame) and the bare-target declaration. Returns
/// the target declaration to co-emit, if the fill needs one.
fn gate_fill_block(
    lines: &[lua_ast::DslLine],
    is_helper: bool,
    eff: &lua_ast::EffectSkeleton,
    walk: &lua_ast::LuaReport,
    block: &str,
) -> Result<Option<String>, FillSkip> {
    // T38 S6 review — cost-header parity: filling a resolve makes the
    // block's header LIVE, so a block-side cost must re-derive exactly
    // from the lua cost handler (skip class 10 — c9798352's `banish
    // self` header contradicts the lua's banish-a-Trap-from-Deck cost).
    if !cost_header_matches(block, eff, &walk.functions) {
        return Err(FillSkip::Incomplete);
    }
    // T38 S5 review — frame parity for summon-helper fills.
    if is_helper && !helper_frame_matches(block, eff) {
        return Err(FillSkip::Incomplete);
    }
    // T38 S6 — bare-target gate: a fill referencing `target` only lands
    // together with a target declaration; otherwise the validator's
    // bare-target check trades the empty-resolve error for a new one.
    // Blocks without one get the decl CO-EMITTED from the SetTarget
    // selector (refined extraction — exactly-mapped filters only); when
    // that fails, the whole fill skips.
    if lines_reference_target(lines) && !block_has_target_decl(block) {
        return match target_decl_selector(eff, &walk.functions) {
            Some(sel) => Ok(Some(sel)),
            None => Err(FillSkip::Incomplete),
        };
    }
    Ok(None)
}

/// Both stages: the single gate stack shared by Pass A, Pass A2,
/// `refine-stubs` and `rederive`, so a placeholder refinement can never
/// land a line an empty-resolve fill would have refused.
fn derive_fill(
    eff: &lua_ast::EffectSkeleton,
    walk: &lua_ast::LuaReport,
    card_id: Option<u32>,
    block: &str,
) -> FillOutcome {
    match derive_fill_lines(eff, walk, card_id) {
        Err(skip) => FillOutcome::Skip(skip),
        Ok(FillLines::Choose(text)) => FillOutcome::Choose(text),
        Ok(FillLines::Lines { lines, is_helper }) => {
            match gate_fill_block(&lines, is_helper, eff, walk, block) {
                Ok(co_target) => FillOutcome::Lines { lines, co_target },
                Err(skip) => FillOutcome::Skip(skip),
            }
        }
    }
}

/// The text that replaces a `resolve { }` slot: the rendered body at
/// the standard 8-space block indent, preceded by the co-emitted target
/// declaration when the fill needs one.
fn render_fill(lines: &[lua_ast::DslLine], co_target: Option<&str>) -> String {
    let body = render_resolve_body(lines);
    match co_target {
        Some(sel) => format!("target {}\n        resolve {{\n{}        }}", sel, body),
        None => format!("resolve {{\n{}        }}", body),
    }
}

/// The trimmed action lines of a derived fill, one entry per rendered
/// line (a choose block contributes every line of its text).
fn rendered_lines(outcome: &FillOutcome) -> Vec<String> {
    match outcome {
        FillOutcome::Choose(text) => text.lines().map(|l| l.trim().to_string()).collect(),
        FillOutcome::Lines { lines, .. } => lines.iter()
            .map(|l| l.clone().into_string("").trim().to_string()).collect(),
        FillOutcome::Skip(_) => Vec::new(),
    }
}

/// The `resolve { … }` span (keyword through closing brace) and its
/// trimmed non-empty lines, for a block not resolved through a `choose`
/// (nested option resolves are Phase 16 output, never audited).
fn shipped_resolve(txt: &str, lo: usize, hi: usize) -> Option<(Vec<String>, (usize, usize))> {
    if txt[lo..hi].contains("choose {") { return None; }
    let (inner_lo, inner_hi) = resolve_inner_span(txt, lo, hi)?;
    let shipped: Vec<String> = txt[inner_lo..inner_hi].lines()
        .map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    let kw_start = inner_lo - "resolve {".len();
    debug_assert!(txt[kw_start..].starts_with("resolve {"));
    Some((shipped, (kw_start, inner_hi + 1)))
}

// ── refine-stubs — placeholder-resolve refinement ────────────

/// Generator-era placeholder resolve lines: the exact spellings the cdb
/// skeleton emitted from a chain's CATEGORY_* bits before any translator
/// ran — one line per category bit, the vacuous `(all, card, either
/// controls)` selector, no zone. A resolve made only of these STATES a
/// category; it does not describe the effect. Closed set (corpus census
/// 2026-10-13: 13,417 lines, 9,793 blocks made only of them). The
/// translator never emits the vacuous selector, so a refined block is
/// never a placeholder block again.
const PLACEHOLDER_LINES: &[&str] = &[
    "special_summon (all, card, either controls)",
    "add_to_hand (all, card, either controls)",
    "destroy (all, card, either controls)",
    "send (all, card, either controls) to gy",
    "return (all, card, either controls) to deck",
    "banish (all, card, either controls)",
    "return (all, card, either controls) to hand",
    "discard (all, card, either controls)",
    "take_control (all, card, either controls)",
    "send (all, card, either controls) to extra_deck",
];

fn is_placeholder_line(line: &str) -> bool {
    PLACEHOLDER_LINES.contains(&line.trim())
}

/// The category class an action line states, for the coverage and
/// parity gates. Verb families that state the same CATEGORY_* bit share
/// a class: every summon procedure states SPECIAL_SUMMON, `search`
/// states TOHAND like `add_to_hand`, `swap_control` states CONTROL.
/// The generator spelled TOGRAVE / HANDES / DECKDES placeholders as
/// `send … to gy` or `discard` interchangeably and TOEXTRA as `return
/// … to deck`, so the to-GY and to-Deck families each collapse into one
/// class. Lines outside the table state no gated category — they may
/// accompany a covering line, never cover on their own. A destination
/// reads from the LAST ` to ` (a where-clause card name may contain
/// the word).
fn line_category(line: &str) -> Option<&'static str> {
    let line = line.trim();
    let verb = line.split_whitespace().next()?;
    let dest = || line.rsplit(" to ").next().and_then(|d| d.split_whitespace().next());
    Some(match verb {
        "special_summon" | "fusion_summon" | "ritual_summon" | "synchro_summon"
        | "xyz_summon" => "special_summon",
        "add_to_hand" | "search" => "to_hand",
        "destroy" => "destroy",
        "banish" => "banish",
        "discard" | "mill" => "to_gy",
        "take_control" | "swap_control" => "control",
        "send" | "return" => match dest()? {
            "gy" => "to_gy",
            "hand" => "to_hand",
            "deck" | "extra_deck" => "to_deck",
            _ => return None,
        },
        _ => return None,
    })
}

/// The placeholder classes a chain's own `SetCategory` bits state. The
/// generator derived a block's placeholder lines from exactly these
/// bits, so a block whose placeholders state a different set was
/// generated from a DIFFERENT chain — the parity gate's alignment
/// evidence. Bits outside the placeholder table (DRAW, DAMAGE, …) drop
/// out; a chain with no SetCategory states the empty set.
fn chain_category_classes(eff: &lua_ast::EffectSkeleton) -> std::collections::BTreeSet<&'static str> {
    let mut out = std::collections::BTreeSet::new();
    for (method, args) in &eff.set_calls {
        if method != "SetCategory" { continue; }
        for bit in args.iter().flat_map(|a| a.split(['+', '|'])).map(str::trim) {
            let class = match bit {
                "CATEGORY_SPECIAL_SUMMON" => "special_summon",
                "CATEGORY_TOHAND" | "CATEGORY_SEARCH" => "to_hand",
                "CATEGORY_DESTROY" => "destroy",
                "CATEGORY_TOGRAVE" | "CATEGORY_HANDES" | "CATEGORY_DECKDES" => "to_gy",
                "CATEGORY_TODECK" | "CATEGORY_TOEXTRA" => "to_deck",
                "CATEGORY_REMOVE" => "banish",
                "CATEGORY_CONTROL" => "control",
                _ => continue,
            };
            out.insert(class);
        }
    }
    out
}

/// Coverage gate: every category the placeholder lines state must be
/// stated by at least one rendered line (nested lines of a multi-line
/// action count — a `delayed … { destroy sp }` still states DESTROY).
/// The rendering may state MORE (the generator's bits were a subset of
/// the op's actions) but never fewer: fewer means an incomplete
/// translation or a mis-aligned block.
fn placeholders_covered(shipped: &[String], rendered: &[String]) -> bool {
    let stated: std::collections::BTreeSet<&str> = rendered.iter()
        .flat_map(|l| l.lines())
        .filter_map(line_category)
        .collect();
    shipped.iter().all(|l| line_category(l).map_or(false, |c| stated.contains(c)))
}

/// The placeholder lines of an effect block whose resolve is made ONLY
/// of placeholder lines, with the span of the whole `resolve { … }`
/// block to replace. None for an empty resolve, a resolve carrying any
/// non-placeholder line, or a choose-resolved block.
fn placeholder_resolve_lines(txt: &str, lo: usize, hi: usize) -> Option<(Vec<String>, (usize, usize))> {
    let (shipped, span) = shipped_resolve(txt, lo, hi)?;
    if shipped.is_empty() || !shipped.iter().all(|l| is_placeholder_line(l)) {
        return None;
    }
    Some((shipped, span))
}

#[derive(Default, Debug, PartialEq, Eq)]
struct RefineStats {
    /// Effect blocks whose resolve is made only of placeholder lines.
    placeholder_blocks: usize,
    /// … of which no walk effect is assigned (alignment hazard, phantom
    /// ordinal, or a generator block with no lua chain).
    unreached: usize,
    refined: usize,
    refined_choose: usize,
    /// Chain SetCategory classes ≠ placeholder classes.
    parity_mismatch: usize,
    /// Chain header signature contradicts the block header.
    header_mismatch: usize,
    /// Rendering states fewer categories than the placeholder.
    not_covered: usize,
    skipped_no_handler: usize,
    skipped_todo: usize,
    skipped_incomplete: usize,
}

impl RefineStats {
    fn add(&mut self, o: &RefineStats) {
        self.placeholder_blocks += o.placeholder_blocks;
        self.unreached += o.unreached;
        self.refined += o.refined;
        self.refined_choose += o.refined_choose;
        self.parity_mismatch += o.parity_mismatch;
        self.header_mismatch += o.header_mismatch;
        self.not_covered += o.not_covered;
        self.skipped_no_handler += o.skipped_no_handler;
        self.skipped_todo += o.skipped_todo;
        self.skipped_incomplete += o.skipped_incomplete;
    }
}

/// Refine every pure-placeholder resolve in one .ds file whose aligned
/// handler derives a complete fill ([`derive_fill`] — the same gate
/// stack an empty resolve gets) stating every category the placeholder
/// did ([`placeholders_covered`]). Two alignment gates sit on top of the
/// positional assignment, because this pass rewrites NON-empty resolves
/// and the uncovered-case audit found generator-skipped chains (flip
/// effects, custom-event triggers, summon procedures) shifting every
/// later block one chain late: the chain's own SetCategory bits must
/// state exactly the placeholder's classes ([`chain_category_classes`])
/// and its header signature must agree with the block header
/// ([`block_match::header_agrees`]). Returns the rewritten text (equal
/// to `txt` when nothing refines), per-file stats, and a log line per
/// decision on a placeholder block. Byte-idempotent: a refined resolve
/// is no longer a placeholder resolve.
fn refine_file(
    txt: &str,
    walk: &lua_ast::LuaReport,
    card_id: Option<u32>,
    stem: &str,
) -> (String, RefineStats, Vec<String>) {
    let mut st = RefineStats::default();
    let mut log: Vec<String> = Vec::new();
    let assign = block_match::compute_assignments(walk, txt);
    let kind = block_match::card_kind_from_ds(txt);
    // Census first: placeholder blocks the effect loop never reaches
    // have no assigned walk effect — the alignment-hazard backlog.
    let mut reached: Vec<bool> = Vec::new();
    for (lo, hi) in block_match::effect_block_ranges(txt) {
        let is_ph = placeholder_resolve_lines(txt, lo, hi).is_some();
        if is_ph { st.placeholder_blocks += 1; }
        reached.push(!is_ph);
    }
    let mut new_txt = txt.to_string();
    for (eff_i, eff) in walk.effects.iter().enumerate() {
        let Some(block_idx) = assign.by_effect[eff_i] else { continue };
        // Offsets recomputed against new_txt — a refinement earlier in
        // this file shifts every later block.
        let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else { continue };
        let Some((shipped, span)) = placeholder_resolve_lines(&new_txt, block_lo, block_hi) else { continue };
        if let Some(r) = reached.get_mut(block_idx) { *r = true; }
        let handler = eff.operation_handler.as_deref().unwrap_or("<helper>").trim();
        // Parity: the chain must state at least one placeholder class
        // and never one the placeholder lacks (the generator also read
        // card text, so a placeholder may state MORE than the bits —
        // c12678870's "send the rest to the GY" line — but a chain with
        // no category bits, or with a bit the placeholder doesn't show,
        // generated some other block).
        let chain_classes = chain_category_classes(eff);
        let placeholder_classes: std::collections::BTreeSet<&str> =
            shipped.iter().filter_map(|l| line_category(l)).collect();
        if chain_classes.is_empty() || !chain_classes.is_subset(&placeholder_classes) {
            st.parity_mismatch += 1;
            log.push(format!("PARITY {} block={} handler={} chain={:?} placeholder={:?}",
                stem, block_idx, handler, chain_classes, placeholder_classes));
            continue;
        }
        let agrees = block_match::parse_block_sigs(&new_txt).get(block_idx)
            .is_some_and(|b| block_match::header_agrees(eff, kind, b));
        if !agrees {
            st.header_mismatch += 1;
            log.push(format!("HEADER {} block={} handler={}", stem, block_idx, handler));
            continue;
        }
        let outcome = derive_fill(eff, walk, card_id, &new_txt[block_lo..block_hi]);
        let (replacement, is_choose) = match &outcome {
            FillOutcome::Skip(reason) => {
                match reason {
                    FillSkip::NoHandler => st.skipped_no_handler += 1,
                    FillSkip::TodoOnly => st.skipped_todo += 1,
                    FillSkip::Incomplete => st.skipped_incomplete += 1,
                }
                log.push(format!("SKIP   {} block={} handler={} reason={:?}", stem, block_idx, handler, reason));
                continue;
            }
            FillOutcome::Choose(text) => (text.clone(), true),
            FillOutcome::Lines { lines, co_target } => (render_fill(lines, co_target.as_deref()), false),
        };
        let rendered = rendered_lines(&outcome);
        if !placeholders_covered(&shipped, &rendered) {
            st.not_covered += 1;
            log.push(format!("UNCOVERED {} block={} handler={}", stem, block_idx, handler));
            for l in &shipped  { log.push(format!("    shipped:  {}", l)); }
            for l in &rendered { log.push(format!("    rendered: {}", l)); }
            continue;
        }
        if rendered.iter().all(|l| is_placeholder_line(l)) {
            // Defensive — the translator never renders the vacuous
            // selector; a rendering made of placeholders would loop.
            continue;
        }
        log.push(format!("REFINE {} block={} handler={}{}", stem, block_idx, handler,
            if is_choose { " (choose)" } else { "" }));
        for l in &shipped  { log.push(format!("    - {}", l)); }
        for l in &rendered { log.push(format!("    + {}", l)); }
        new_txt = format!("{}{}{}", &new_txt[..span.0], replacement, &new_txt[span.1..]);
        if is_choose { st.refined_choose += 1; } else { st.refined += 1; }
    }
    st.unreached = reached.iter().filter(|r| !**r).count();
    (new_txt, st, log)
}

/// Walk the corpus like `audit-fills`: one (stem, text, walk, card id)
/// visit per .ds file that has a parseable lua twin with effects.
fn for_each_card(corpus_dir: &str, lua_dir: &str, mut visit: impl FnMut(&std::path::Path, &str, String, &lua_ast::LuaReport, Option<u32>)) {
    let mut paths: Vec<std::path::PathBuf> = match fs::read_dir(corpus_dir) {
        Ok(e) => e.flatten().map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
            .collect(),
        Err(e) => { eprintln!("cannot read {}: {}", corpus_dir, e); return; }
    };
    paths.sort();
    for path in paths {
        let Ok(txt) = fs::read_to_string(&path) else { continue };
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let lua_path = Path::new(lua_dir).join(format!("{}.lua", stem));
        let Ok(lua_src) = fs::read_to_string(&lua_path) else { continue };
        let Ok(parsed) = full_moon::parse(&lua_src) else { continue };
        let walk = lua_ast::walk(&parsed);
        if walk.effects.is_empty() { continue; }
        let card_id: Option<u32> = stem.strip_prefix('c').and_then(|s| s.parse().ok());
        visit(&path, &stem, txt, &walk, card_id);
    }
}

/// See the `refine-stubs` mode arm. Prints every decision and rewrites
/// files only with `--apply`.
fn refine_stubs(corpus_dir: &str, lua_dir: &str, apply: bool) {
    let mut total = RefineStats::default();
    let (mut files_scanned, mut files_changed) = (0usize, 0usize);
    for_each_card(corpus_dir, lua_dir, |path, stem, txt, walk, card_id| {
        files_scanned += 1;
        let (new_txt, st, log) = refine_file(&txt, walk, card_id, stem);
        for l in log { println!("{}", l); }
        total.add(&st);
        if new_txt != txt {
            files_changed += 1;
            if apply {
                if let Err(e) = fs::write(path, &new_txt) {
                    eprintln!("cannot write {}: {}", path.display(), e);
                }
            }
        }
    });
    println!("=== refine-stubs report ({}) ===", if apply { "applied" } else { "dry run" });
    println!("  files scanned:            {}", files_scanned);
    println!("  placeholder blocks:       {}", total.placeholder_blocks);
    println!("  unreached (no effect):    {}", total.unreached);
    println!("  refined (lines):          {}", total.refined);
    println!("  refined (choose):         {}", total.refined_choose);
    println!("  parity mismatch:          {}", total.parity_mismatch);
    println!("  header mismatch:          {}", total.header_mismatch);
    println!("  not covered (category):   {}", total.not_covered);
    println!("  skipped (no handler):     {}", total.skipped_no_handler);
    println!("  skipped (todo only):      {}", total.skipped_todo);
    println!("  skipped (incomplete):     {}", total.skipped_incomplete);
    println!("  files changed:            {}", files_changed);
}

// ── rederive — accepted-fill re-derivation audit ─────────────

/// The spelling an earlier translator gave `line`: multi-zone lists
/// removed (`, from a or b` segments drop, single-zone `from x` stays),
/// a special summon's `in defense_position` / `in face_down_defense`
/// suffix removed, and every `discard (…)` collapsed to the one fixed
/// string the pre-fix DiscardHand arm emitted. Equal to a shipped line
/// exactly when the only difference is what the compound-LOCATION,
/// summon-position and DiscardHand fixes now render.
fn old_spelling(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find(", from ") {
        let after = &rest[i + 2..]; // "from …"
        let seg_len = after.find([',', ')']).unwrap_or(after.len());
        if after[..seg_len].contains(" or ") {
            out.push_str(&rest[..i]);
        } else {
            out.push_str(&rest[..i + 2 + seg_len]);
        }
        rest = &rest[i + 2 + seg_len..];
    }
    out.push_str(rest);
    if out.starts_with("discard (") {
        // The pre-fix DiscardHand arm rendered one fixed string.
        return "discard (1+, card, you control, from hand)".to_string();
    }
    if let Some(mid) = out.strip_prefix("return ") {
        // The pre-fix SendtoDeck arm rendered `send … to deck` whatever
        // the sequence.
        for pos in [" to deck top", " to deck bottom", " to deck shuffle"] {
            if let Some(sel) = mid.strip_suffix(pos) {
                return format!("send {} to deck", sel);
            }
        }
    }
    if out.starts_with("special_summon ") {
        for suffix in [" in defense_position", " in face_down_defense"] {
            if let Some(stripped) = out.strip_suffix(suffix) {
                return stripped.to_string();
            }
            if let Some(i) = out.find(&format!("{} as ", suffix)) {
                return format!("{}{}", &out[..i], &out[i + suffix.len()..]);
            }
        }
    }
    out
}

/// A rendering that differs from the shipped lines only by spellings an
/// earlier translator lacked: same line count, every line either equal
/// or equal under [`old_spelling`], at least one differing. Unchanged
/// lines in a multi-line block don't disqualify the block, and a line
/// that already carries the new spelling (an earlier upgrade) compares
/// equal rather than through `old_spelling`.
fn is_mechanical_upgrade(rendered: &[String], shipped: &[String]) -> bool {
    rendered.len() == shipped.len()
        && rendered != shipped
        && rendered.iter().zip(shipped).all(|(r, s)| r == s || old_spelling(r) == *s)
}

/// The handler's rendering split into action lines and TODO riders —
/// the raw translator output, before any fill gate.
fn handler_render(eff: &lua_ast::EffectSkeleton, walk: &lua_ast::LuaReport) -> Option<(Vec<String>, Vec<String>)> {
    let handler = eff.operation_handler.as_deref()?.trim();
    let body = walk.functions.get(handler)?;
    let (mut actions, mut todos) = (Vec::new(), Vec::new());
    for l in lua_ast::translate_body_with_functions(body, &walk.functions) {
        let is_action = l.is_action();
        let text = l.into_string("").trim().to_string();
        if is_action { actions.push(text); } else { todos.push(text); }
    }
    Some((actions, todos))
}

/// See the `rederive` mode arm. Reports every shipped non-empty, non-
/// placeholder resolve against its current derivation: `SAME`, an
/// `UPGRADE` (differs only by a zone list / summon position the fixed
/// translator now renders — rewritten with `--apply`), a `REFUSED` fill
/// (the current gate declines it; `exact=yes` when the shipped lines
/// are the translator's own action lines — stubbed by `--stub-todo=X`
/// when a TODO rider contains `X`), or a `REVIEW` (differs otherwise —
/// report-only, for hand verification).
fn rederive(corpus_dir: &str, lua_dir: &str, apply: bool, stub_todo: Option<&str>) {
    let (mut examined, mut same, mut upgrades, mut review) = (0usize, 0usize, 0usize, 0usize);
    let (mut refused, mut refused_exact, mut stubbed) = (0usize, 0usize, 0usize);
    let mut files_changed = 0usize;
    for_each_card(corpus_dir, lua_dir, |path, stem, txt, walk, card_id| {
        let assign = block_match::compute_assignments(walk, &txt);
        let mut new_txt = txt.clone();
        let mut changed = false;
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            let Some(block_idx) = assign.by_effect[eff_i] else { continue };
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else { continue };
            let Some((shipped, span)) = shipped_resolve(&new_txt, block_lo, block_hi) else { continue };
            if shipped.is_empty() || shipped.iter().all(|l| is_placeholder_line(l)) { continue; }
            examined += 1;
            let handler = eff.operation_handler.as_deref().unwrap_or("<helper>").trim();
            let outcome = derive_fill(eff, walk, card_id, &new_txt[block_lo..block_hi]);
            let rendered = rendered_lines(&outcome);
            let (lines, co_target) = match &outcome {
                FillOutcome::Skip(reason) => {
                    refused += 1;
                    let (actions, todos) = handler_render(eff, walk).unwrap_or_default();
                    let exact = !actions.is_empty() && actions == shipped;
                    if exact { refused_exact += 1; }
                    println!("REFUSED {} block={} handler={} reason={:?} exact={}",
                        stem, block_idx, handler, reason, if exact { "yes" } else { "no" });
                    for t in &todos { println!("    todo: {}", t); }
                    if !exact {
                        for l in &shipped  { println!("    shipped:  {}", l); }
                        for l in &actions  { println!("    rendered: {}", l); }
                    }
                    if exact && stub_todo.is_some_and(|x| todos.iter().any(|t| t.contains(x))) {
                        let (inner_lo, inner_hi) = (span.0 + "resolve {".len(), span.1 - 1);
                        new_txt = format!("{}\n        {}", &new_txt[..inner_lo], &new_txt[inner_hi..]);
                        changed = true;
                        stubbed += 1;
                        println!("    STUBBED");
                    }
                    continue;
                }
                FillOutcome::Choose(_) => (None, None),
                FillOutcome::Lines { lines, co_target } => (Some(lines), co_target.as_deref()),
            };
            if rendered == shipped {
                same += 1;
                println!("SAME {} block={} handler={}", stem, block_idx, handler);
                continue;
            }
            let is_upgrade = lines.is_some() && co_target.is_none()
                && is_mechanical_upgrade(&rendered, &shipped);
            if is_upgrade {
                upgrades += 1;
                println!("UPGRADE {} block={} handler={}", stem, block_idx, handler);
                for l in &shipped  { println!("    - {}", l); }
                for l in &rendered { println!("    + {}", l); }
                if apply {
                    let replacement = render_fill(lines.unwrap(), None);
                    new_txt = format!("{}{}{}", &new_txt[..span.0], replacement, &new_txt[span.1..]);
                    changed = true;
                }
            } else {
                review += 1;
                println!("REVIEW {} block={} handler={}", stem, block_idx, handler);
                for l in &shipped  { println!("    shipped:  {}", l); }
                for l in &rendered { println!("    rendered: {}", l); }
            }
        }
        if changed {
            files_changed += 1;
            if let Err(e) = fs::write(path, &new_txt) {
                eprintln!("cannot write {}: {}", path.display(), e);
            }
        }
    });
    println!("=== rederive report ({}) ===", if apply || stub_todo.is_some() { "applied" } else { "dry run" });
    println!("  resolves examined:        {}", examined);
    println!("  identical:                {}", same);
    println!("  upgrades (zone/position): {}", upgrades);
    println!("  review (other diffs):     {}", review);
    println!("  refused by current gate:  {}", refused);
    println!("    of which exact fills:   {}", refused_exact);
    println!("    stubbed:                {}", stubbed);
    println!("  files changed:            {}", files_changed);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_lines_form_a_closed_set() {
        assert!(is_placeholder_line("special_summon (all, card, either controls)"));
        assert!(is_placeholder_line("    send (all, card, either controls) to gy"));
        assert!(!is_placeholder_line("special_summon (all, card, either controls, from gy)"));
        assert!(!is_placeholder_line("special_summon (all, card, either controls) in attack_position"));
        assert!(!is_placeholder_line("draw 1"));
        assert!(!is_placeholder_line("special_summon target"));
    }

    #[test]
    fn line_category_shares_a_class_across_verb_families() {
        assert_eq!(line_category("fusion_summon (1, card, you control, from extra_deck)"), Some("special_summon"));
        assert_eq!(line_category("special_summon self"), Some("special_summon"));
        assert_eq!(line_category("search (1, card, you control, from deck)"), Some("to_hand"));
        assert_eq!(line_category("return target to hand"), Some("to_hand"));
        assert_eq!(line_category("mill 2"), Some("to_gy"));
        assert_eq!(line_category("discard (1, card, you control, from hand)"), Some("to_gy"));
        assert_eq!(line_category("send (1, card, where name == \"Return to Hand\") to gy"), Some("to_gy"));
        assert_eq!(line_category("return (1, card, you control, from gy) to deck top"), Some("to_deck"));
        assert_eq!(line_category("send self to extra_deck"), Some("to_deck"));
        assert_eq!(line_category("swap_control self target"), Some("control"));
        assert_eq!(line_category("damage opponent 800"), None);
        assert_eq!(line_category("return self to field"), None);
        assert_eq!(line_category("}"), None);
    }

    #[test]
    fn coverage_requires_every_placeholder_category() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let shipped = s(&["special_summon (all, card, either controls)", "destroy (all, card, either controls)"]);
        assert!(!placeholders_covered(&shipped, &s(&["special_summon target"])));
        assert!(placeholders_covered(&shipped,
            &s(&["destroy (1, card, opponent controls, from monster_zone)", "xyz_summon self"])));
        // Extra categories in the rendering are fine — the generator's
        // bits were a subset of the op's actions.
        assert!(placeholders_covered(&s(&["destroy (all, card, either controls)"]),
            &s(&["destroy target", "damage opponent 500"])));
        // Nested lines of a multi-line action count.
        assert!(placeholders_covered(&s(&["destroy (all, card, either controls)"]),
            &s(&["delayed on end_phase until end_of_turn {\n    destroy sp\n}"])));
        // A choose block's lines count across options.
        assert!(placeholders_covered(&s(&["special_summon (all, card, either controls)", "take_control (all, card, either controls)"]),
            &s(&["choose {", "option \"A\" {", "resolve {", "take_control target", "}", "}",
                 "option \"B\" {", "resolve {", "special_summon target", "}", "}", "}"])));
    }

    #[test]
    fn chain_category_classes_reads_set_category_bits() {
        let mut eff = lua_ast::EffectSkeleton::default();
        assert!(chain_category_classes(&eff).is_empty());
        eff.set_calls.push(("SetCategory".to_string(),
            vec!["CATEGORY_SEARCH+CATEGORY_TOHAND+CATEGORY_DESTROY".to_string()]));
        let classes = chain_category_classes(&eff);
        assert_eq!(classes.into_iter().collect::<Vec<_>>(), vec!["destroy", "to_hand"]);
        let mut eff = lua_ast::EffectSkeleton::default();
        eff.set_calls.push(("SetCategory".to_string(),
            vec!["CATEGORY_SPECIAL_SUMMON|CATEGORY_DRAW".to_string()]));
        assert_eq!(chain_category_classes(&eff).into_iter().collect::<Vec<_>>(), vec!["special_summon"]);
        // The to-GY and to-Deck families collapse the way the
        // generator's placeholders did.
        let mut eff = lua_ast::EffectSkeleton::default();
        eff.set_calls.push(("SetCategory".to_string(),
            vec!["CATEGORY_HANDES+CATEGORY_DECKDES+CATEGORY_TOEXTRA".to_string()]));
        assert_eq!(chain_category_classes(&eff).into_iter().collect::<Vec<_>>(), vec!["to_deck", "to_gy"]);
    }

    const DS_PLACEHOLDER: &str = "\
card \"Test Monster\" {
    id: 1
    type: Effect Monster
    attribute: DARK
    race: Fiend
    level: 4
    atk: 1000
    def: 1000

    effect \"Effect 1\" {
        speed: 1
        once_per_turn: hard
        resolve {
            special_summon (all, card, either controls)
        }
    }
}
";

    #[test]
    fn placeholder_resolve_lines_spans_the_whole_resolve_block() {
        let (lo, hi) = block_match::effect_block_ranges(DS_PLACEHOLDER)[0];
        let (lines, span) = placeholder_resolve_lines(DS_PLACEHOLDER, lo, hi).unwrap();
        assert_eq!(lines, vec!["special_summon (all, card, either controls)".to_string()]);
        assert!(DS_PLACEHOLDER[span.0..].starts_with("resolve {"));
        assert_eq!(&DS_PLACEHOLDER[span.1 - 1..span.1], "}");
        assert_eq!(&DS_PLACEHOLDER[span.1..span.1 + 6], "\n    }");
        // A refined block is not a placeholder block.
        let refined = DS_PLACEHOLDER.replace("special_summon (all, card, either controls)", "special_summon self");
        let (lo, hi) = block_match::effect_block_ranges(&refined)[0];
        assert!(placeholder_resolve_lines(&refined, lo, hi).is_none());
        // A block resolved through a choose is never a placeholder block.
        let choose = DS_PLACEHOLDER.replace(
            "resolve {\n            special_summon (all, card, either controls)\n        }",
            "choose {\n            option \"A\" {\n                resolve {\n                    special_summon (all, card, either controls)\n                }\n            }\n        }");
        let (lo, hi) = block_match::effect_block_ranges(&choose)[0];
        assert!(placeholder_resolve_lines(&choose, lo, hi).is_none());
        // An empty resolve is Pass A's, not this pass's.
        let empty = DS_PLACEHOLDER.replace("\n            special_summon (all, card, either controls)\n        ", " ");
        let (lo, hi) = block_match::effect_block_ranges(&empty)[0];
        assert!(placeholder_resolve_lines(&empty, lo, hi).is_none());
    }

    #[test]
    fn old_spelling_drops_zone_lists_and_summon_positions() {
        assert_eq!(old_spelling("special_summon (1, card, you control, from hand or gy)"),
            "special_summon (1, card, you control)");
        assert_eq!(old_spelling("special_summon (1, card, you control, from hand)"),
            "special_summon (1, card, you control, from hand)");
        assert_eq!(old_spelling("send (1, card, you control, from deck or extra_deck, where is_monster) to gy"),
            "send (1, card, you control, where is_monster) to gy");
        assert_eq!(old_spelling("special_summon (1, card, you control, from deck) in defense_position"),
            "special_summon (1, card, you control, from deck)");
        assert_eq!(old_spelling("special_summon self in face_down_defense as sp"),
            "special_summon self as sp");
        assert_eq!(old_spelling("special_summon (1, card, you control, from hand or gy) in defense_position"),
            "special_summon (1, card, you control)");
        assert_eq!(old_spelling("discard (1, card, opponent controls, from hand)"),
            "discard (1+, card, you control, from hand)");
        assert_eq!(old_spelling("return self to deck shuffle"), "send self to deck");
        assert_eq!(old_spelling("return (1, card, you control, from gy) to deck bottom"),
            "send (1, card, you control, from gy) to deck");
        assert_eq!(old_spelling("return self to hand"), "return self to hand");
    }

    #[test]
    fn mechanical_upgrade_tolerates_unchanged_and_already_upgraded_lines() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // c13256226: one re-spelled line beside an unchanged one.
        assert!(is_mechanical_upgrade(
            &s(&["return self to deck shuffle", "special_summon (1, card, you control, from deck)"]),
            &s(&["send self to deck", "special_summon (1, card, you control, from deck)"])));
        // An earlier upgrade already in the shipped text compares equal.
        assert!(is_mechanical_upgrade(
            &s(&["special_summon (1, card, you control, from hand or gy)", "return self to deck top"]),
            &s(&["special_summon (1, card, you control, from hand or gy)", "send self to deck"])));
        // Identical, different length, or a real semantic difference: no.
        assert!(!is_mechanical_upgrade(&s(&["draw 1"]), &s(&["draw 1"])));
        assert!(!is_mechanical_upgrade(&s(&["draw 1", "draw 1"]), &s(&["draw 1"])));
        assert!(!is_mechanical_upgrade(&s(&["destroy target"]), &s(&["special_summon self"])));
        assert_eq!(old_spelling("draw 1"), "draw 1");
    }

    const LUA_SELF_SUMMON: &str = "\
local s,id=GetID()
function s.initial_effect(c)
	--Special Summon itself from the GY
	local e1=Effect.CreateEffect(c)
	e1:SetDescription(aux.Stringid(id,0))
	e1:SetCategory(CATEGORY_SPECIAL_SUMMON)
	e1:SetType(EFFECT_TYPE_IGNITION)
	e1:SetRange(LOCATION_GRAVE)
	e1:SetCountLimit(1,id)
	e1:SetTarget(s.sptg)
	e1:SetOperation(s.spop)
	c:RegisterEffect(e1)
end
function s.sptg(e,tp,eg,ep,ev,re,r,rp,chk)
	if chk==0 then return Duel.GetLocationCount(tp,LOCATION_MZONE)>0
		and e:GetHandler():IsCanBeSpecialSummoned(e,0,tp,false,false) end
	Duel.SetOperationInfo(0,CATEGORY_SPECIAL_SUMMON,e:GetHandler(),1,0,0)
end
function s.spop(e,tp,eg,ep,ev,re,r,rp)
	local c=e:GetHandler()
	if c:IsRelateToEffect(e) then
		Duel.SpecialSummon(c,0,tp,tp,false,false,POS_FACEUP)
	end
end
";

    fn walk(lua: &str) -> lua_ast::LuaReport {
        lua_ast::walk(&full_moon::parse(lua).expect("lua parses"))
    }

    #[test]
    fn refine_file_replaces_a_placeholder_with_the_derived_fill() {
        let w = walk(LUA_SELF_SUMMON);
        let (out, st, log) = refine_file(DS_PLACEHOLDER, &w, Some(1), "c1");
        assert_eq!(st.placeholder_blocks, 1, "{:?}", log);
        assert_eq!(st.refined, 1, "{:?}", log);
        assert_eq!(out, DS_PLACEHOLDER.replace(
            "special_summon (all, card, either controls)", "special_summon self"));
        assert!(log.iter().any(|l| l.starts_with("REFINE c1 block=0 handler=s.spop")));
        // Idempotent: the refined resolve is no longer a placeholder.
        let (again, st2, _) = refine_file(&out, &w, Some(1), "c1");
        assert_eq!(again, out);
        assert_eq!(st2, RefineStats::default());
    }

    #[test]
    fn refine_file_refuses_parity_and_header_mismatches() {
        let w = walk(LUA_SELF_SUMMON);
        // The chain states SPECIAL_SUMMON; a DESTROY placeholder was
        // generated from some other chain.
        let ds = DS_PLACEHOLDER.replace("special_summon (all, card, either controls)",
            "destroy (all, card, either controls)");
        let (out, st, log) = refine_file(&ds, &w, Some(1), "c1");
        assert_eq!(out, ds);
        assert_eq!(st.parity_mismatch, 1, "{:?}", log);
        assert_eq!(st.refined, 0);
        // An ignition chain expects no trigger line; a block carrying
        // one belongs to a different chain.
        let ds = DS_PLACEHOLDER.replace("        once_per_turn: hard\n",
            "        once_per_turn: hard\n        trigger: summoned\n");
        let (out, st, log) = refine_file(&ds, &w, Some(1), "c1");
        assert_eq!(out, ds);
        assert_eq!(st.header_mismatch, 1, "{:?}", log);
        // A once-per-turn chain against a block without the line, too.
        let ds = DS_PLACEHOLDER.replace("        once_per_turn: hard\n", "");
        let (out, st, _) = refine_file(&ds, &w, Some(1), "c1");
        assert_eq!(out, ds);
        assert_eq!(st.header_mismatch, 1);
    }

    #[test]
    fn refine_file_leaves_unassigned_placeholder_blocks_alone() {
        // Two placeholder blocks, one chain: the second block has no
        // walk effect and counts as unreached.
        let w = walk(LUA_SELF_SUMMON);
        let extra = "\
    effect \"Effect 2\" {
        speed: 1
        trigger: destroyed
        resolve {
            add_to_hand (all, card, either controls)
        }
    }
";
        let ds = DS_PLACEHOLDER.replace("    }\n}\n", &format!("    }}\n\n{}}}\n", extra));
        let (out, st, log) = refine_file(&ds, &w, Some(1), "c1");
        assert_eq!(st.placeholder_blocks, 2, "{:?}", log);
        assert_eq!(st.refined, 1, "{:?}", log);
        assert_eq!(st.unreached, 1, "{:?}", log);
        assert!(out.contains("add_to_hand (all, card, either controls)"));
        assert!(out.contains("special_summon self"));
    }
}
