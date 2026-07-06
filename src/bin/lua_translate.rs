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
        eprintln!("usage: lua_translate <dump|apply> <lua-file>");
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
        }
        _ => {
            eprintln!("unknown mode: {}", mode);
            process::exit(2);
        }
    }
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
    effects_no_handler: usize,
    effects_alignment_hazard: usize,
    match_positional: usize,
    match_rescued: usize,
    chooses_injected: usize,
    passives_injected: usize,
    conditions_injected: usize,
    costs_injected: usize,
    targets_injected: usize,
    no_lua: usize,
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
        // non-idempotent.
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            if !eff.is_summon_helper() && eff.operation_handler.is_none() {
                r.effects_no_handler += 1;
                continue; // pure-passive — no effect block in .ds
            }
            let Some(block_idx) = assign.by_effect[eff_i] else {
                // Hazard-gated and not rescued: a clone / bare-activate
                // chain owns a .ds block before this one and the block
                // signatures don't force a unique home — filling would
                // risk landing in the wrong block.
                r.effects_alignment_hazard += 1;
                continue;
            };
            // Phase 16 — SelectOption label-branch dispatch. When the
            // target handler picked an option (`Duel.SelectOption` +
            // `e:SetLabel`) and the operation handler splits on
            // `e:GetLabel()` with one fully-translatable arm per option,
            // a `choose { option … }` block REPLACES the empty resolve
            // (the validator accepts either, never both). Falls through
            // to the normal fill when the shape or a label doesn't
            // resolve.
            if !eff.is_summon_helper() {
                let choose_text = eff.operation_handler.as_deref()
                    .and_then(|oh| {
                        let ob = walk.functions.get(oh.trim())?;
                        let tb = eff.target_handler.as_deref()
                            .and_then(|th| walk.functions.get(th.trim()));
                        let spec = lua_ast::extract_choose_spec(tb, ob, &walk.functions)?;
                        render_choose_block(&spec, card_id?)
                    });
                if let Some(choose_text) = choose_text {
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
            }
            // Special case: skeletons backed by a fusion/ritual summon
            // helper — the plain factories (Fusion.CreateSummonEff /
            // Ritual.AddProc* / Ritual.CreateProc, fixed line) and the
            // parameterized SetOperation forms (Fusion.SummonEffOP /
            // Ritual.Operation, Phase 12). The helper owns the UI / op
            // pipeline, so a summon line replaces the handler walk.
            // Parameterized forms whose params don't decode return None
            // and fall through to the no-handler skip below.
            let helper_line = eff.summon_helper_line();
            let lines: Vec<lua_ast::DslLine> = if let Some(text) = helper_line {
                vec![lua_ast::DslLine::Action(text)]
            } else if eff.is_summon_helper() {
                // Helper params undecodable — the activation block exists
                // in the .ds but its resolve stays an empty stub
                // (skip-not-mis-emit).
                r.effects_no_handler += 1;
                continue;
            } else {
                // Consumer without a helper spec has an operation handler
                // (checked at the top of the loop).
                let handler = eff.operation_handler.as_deref().unwrap_or_default();
                match walk.functions.get(handler.trim()) {
                    Some(body) => lua_ast::translate_body_with_functions(body, &walk.functions),
                    None => {
                        r.effects_no_handler += 1; // block exists, body unknown
                        continue;
                    }
                }
            };
            if !lines.iter().any(|l| l.is_action()) {
                r.effects_todo_only += 1;
                continue;
            }
            let Some((block_lo, block_hi)) = nth_effect_block(&new_txt, block_idx) else {
                continue;
            };
            if let Some((lo, hi)) = first_empty_resolve_within(&new_txt, block_lo, block_hi) {
                let body = render_resolve_body(&lines);
                let injection = format!("resolve {{\n{}        }}", body);
                new_txt = format!("{}{}{}", &new_txt[..lo], injection, &new_txt[hi..]);
                filled += 1;
            }
        }

        // Pass A2 — inject a fresh `resolve { … }` into effect blocks that
        // lack one entirely (validator: "must have a resolve or choose
        // block"). Only fires when the lua-ast translator emits at least
        // one ACTION line, so blocks without translator coverage stay
        // untouched. Effect blocks come from the shared Phase 20
        // assignment (pure-passive lua chains have no effect block and
        // are skipped upstream).
        for (eff_i, eff) in walk.effects.iter().enumerate() {
            if !eff.is_summon_helper() && eff.operation_handler.is_none() {
                continue; // pure-passive — no effect block in .ds
            }
            let Some(block_idx) = assign.by_effect[eff_i] else {
                continue; // see Pass A — no unambiguous block for this effect
            };
            let helper_line = eff.summon_helper_line();
            let lines: Vec<lua_ast::DslLine> = if let Some(text) = helper_line {
                vec![lua_ast::DslLine::Action(text)]
            } else if eff.is_summon_helper() {
                // Block exists, but no line to emit — see Pass A.
                continue;
            } else {
                let handler = eff.operation_handler.as_deref().unwrap_or_default();
                match walk.functions.get(handler.trim()) {
                    Some(body) => lua_ast::translate_body_with_functions(body, &walk.functions),
                    None => continue,
                }
            };
            if !lines.iter().any(|l| l.is_action()) { continue; }

            let (block_lo, block_hi) = match nth_effect_block(&new_txt, block_idx) {
                Some(r) => r,
                None => continue,
            };
            let block = &new_txt[block_lo..block_hi];
            // Skip if the block already has a resolve or choose — Pass A
            // handles those, and we don't want to double-inject.
            if block.contains("resolve") || block.contains("choose") { continue; }

            let body_text = render_resolve_body(&lines);
            // Inject right before the block's closing `}`, after any
            // trailing whitespace, so the new resolve nests inside the
            // effect block with the standard 8-space indent.
            let close_brace = block_hi - 1; // points at `}`
            let mut inject_pos = close_brace;
            let bytes = new_txt.as_bytes();
            while inject_pos > block_lo && bytes[inject_pos - 1].is_ascii_whitespace() {
                inject_pos -= 1;
            }
            let injection = format!("\n        resolve {{\n{}        }}", body_text);
            new_txt = format!("{}{}{}", &new_txt[..inject_pos], injection, &new_txt[inject_pos..]);
            filled += 1;
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

        if filled > 0 || blocks_added > 0 || passives_added > 0 || conditions_added > 0 || costs_added > 0 || targets_added > 0 {
            r.effects_filled += filled;
            r.blocks_injected += blocks_added;
            r.passives_injected += passives_added;
            r.conditions_injected += conditions_added;
            r.costs_injected += costs_added;
            r.targets_injected += targets_added;
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
