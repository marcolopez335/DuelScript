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
use std::process;

use duelscript::lua_ast;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: lua_translate <dump|apply> <lua-file>");
        process::exit(2);
    }
    let mode = args[1].as_str();
    let path = &args[2];
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("cannot read {}: {}", path, e); process::exit(1); }
    };
    match mode {
        "dump" => {
            let report = lua_ast::analyze(&src);
            println!("{}", report);
        }
        "apply" => {
            eprintln!("apply: not yet implemented");
            process::exit(2);
        }
        _ => {
            eprintln!("unknown mode: {}", mode);
            process::exit(2);
        }
    }
}
