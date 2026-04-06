// ============================================================
// parse_card.rs — Example: parsing a .ds file and printing the AST
// ============================================================

use duelscript::parse;

fn main() {
    let source = include_str!("../cards/ash_blossom.ds");

    match parse(source) {
        Ok(file) => {
            for card in &file.cards {
                println!("=== Card: {} ===", card.name);
                println!("  Types:     {:?}", card.card_types);
                println!("  Attribute: {:?}", card.attribute);
                println!("  Race:      {:?}", card.race);
                println!("  Level:     {:?}", card.level);
                println!("  ATK/DEF:   {:?} / {:?}", card.stats.atk, card.stats.def);
                println!("  Effects:   {}", card.effects.len());

                for (i, effect) in card.effects.iter().enumerate() {
                    println!("\n  [Effect {}] {:?}", i + 1, effect.name);
                    println!("    Speed:        {}", effect.body.speed);
                    println!("    Frequency:    {:?}", effect.body.frequency);
                    println!("    Condition:    {:?}", effect.body.condition);
                    println!("    Trigger:      {:?}", effect.body.trigger);
                    println!("    Cost:         {:?}", effect.body.cost);
                    println!("    On Resolve:   {:?}", effect.body.on_resolve);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to parse: {}", e);
            std::process::exit(1);
        }
    }
}
