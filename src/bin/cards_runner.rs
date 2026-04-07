// ============================================================
// cards_runner — load and execute a single card via the
// MockRuntime test harness, then print what happened.
//
// Usage:
//   cargo run --bin cards_runner -- <path-to-card.ds>
//
// Loads the .ds file, compiles it, runs each effect's operation
// callback against a fresh MockRuntime, and prints the recorded
// call log so you can see exactly what the closures did.
//
// This is a development/debugging tool, not a real duel simulator.
// For permanent regression checks, use tests/cards.rs instead.
// ============================================================

use std::env;
use std::path::Path;
use std::process::ExitCode;

use duelscript::test_harness::{compile_file, MockRuntime};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <card.ds>", args[0]);
        return ExitCode::from(2);
    }

    let path = Path::new(&args[1]);
    let compiled = match compile_file(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("compile failed: {}", e);
            return ExitCode::FAILURE;
        }
    };

    println!("╔════════════════════════════════════════════════════════════");
    println!("║ Card: {} (id {})", compiled.name, compiled.card_id);
    println!("║ Effects: {}", compiled.effects.len());
    println!("╚════════════════════════════════════════════════════════════");

    let mut total_passed = 0;
    let mut total_run = 0;

    for (i, effect) in compiled.effects.iter().enumerate() {
        println!();
        println!("── Effect #{} ─────────────────────────────", i + 1);
        println!("  effect_type = 0x{:08x}", effect.effect_type);
        println!("  category    = 0x{:08x}", effect.category);
        println!("  code        = 0x{:08x}", effect.code);
        println!("  property    = 0x{:08x}", effect.property);
        println!("  range       = 0x{:08x}", effect.range);
        if let Some(cl) = &effect.count_limit {
            println!("  count_limit = count={} code=0x{:x}", cl.count, cl.code);
        }

        // Run the four callbacks against a fresh runtime to see what they do.
        let mut rt = MockRuntime::new();
        rt.effect_card_id = compiled.card_id;

        // Stock the deck so `draw` actions actually move cards.
        rt.state.players[0].deck = (1000..1100).collect();
        rt.state.players[1].deck = (2000..2100).collect();
        // Put the activator card in hand so spell-from-hand checks pass.
        rt.state.players[0].hand.push(compiled.card_id);

        let cond_ok = effect.callbacks.condition.as_ref()
            .map(|cb| cb(&rt))
            .unwrap_or(true);
        println!("  condition() → {}", cond_ok);
        if !cond_ok {
            continue;
        }

        if let Some(cost) = &effect.callbacks.cost {
            let payable = cost(&mut rt, true);
            println!("  cost(check_only=true) → {}", payable);
            if payable {
                let _ = cost(&mut rt, false);
                println!("  cost(check_only=false) executed");
            }
        }
        if let Some(target) = &effect.callbacks.target {
            let valid = target(&mut rt, true);
            println!("  target(check_only=true) → {}", valid);
            if valid {
                let _ = target(&mut rt, false);
                println!("  target(check_only=false) executed");
            }
        }
        if let Some(operation) = &effect.callbacks.operation {
            operation(&mut rt);
            println!("  operation() executed");
        }
        total_run += 1;

        println!();
        println!("  Runtime calls:");
        if rt.calls.is_empty() {
            println!("    (none)");
        } else {
            print!("{}", rt.dump_calls());
        }

        println!();
        println!("  Final state:");
        for p in 0..2 {
            let ps = &rt.state.players[p];
            println!("    p{}: lp={} hand={} deck={} gy={} field_m={} field_s={}",
                p, ps.lp, ps.hand.len(), ps.deck.len(),
                ps.graveyard.len(), ps.field_monsters.len(), ps.field_spells.len());
        }
        total_passed += 1;
    }

    println!();
    println!("── Summary ────────────────────────────────");
    println!("  effects run: {}/{}", total_passed, total_run);
    if total_passed == total_run {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
