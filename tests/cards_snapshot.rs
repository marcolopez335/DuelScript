// ============================================================
// Snapshot tests for the canonical card library.
//
// For each .ds file in cards/test/, compile it and serialize the
// result into a stable text format, then compare against a stored
// .snap file under tests/snapshots/. Any change to the compiler,
// type_mapper, or callback_gen that perturbs an effect's metadata
// will fail loudly here.
//
// To regenerate snapshots after an intentional change:
//
//     UPDATE_SNAPSHOTS=1 cargo test --test cards_snapshot
//
// First run also auto-creates any missing snapshot.
// ============================================================

use std::fs;
use std::path::{Path, PathBuf};

use duelscript::compiler::{compile_card, CompiledCard};
use duelscript::parser::parse;

const CARDS_DIR: &str = "cards/test";
const SNAPSHOTS_DIR: &str = "tests/snapshots";

fn render_compiled(c: &CompiledCard) -> String {
    let mut out = String::new();
    out.push_str(&format!("card_id: {}\n", c.card_id));
    out.push_str(&format!("name:    {}\n", c.name));
    out.push_str(&format!("effects: {}\n", c.effects.len()));
    out.push_str("--\n");
    for (i, e) in c.effects.iter().enumerate() {
        out.push_str(&format!("[effect {}]\n", i));
        out.push_str(&format!("  effect_type = 0x{:08x}\n", e.effect_type));
        out.push_str(&format!("  category    = 0x{:08x}\n", e.category));
        out.push_str(&format!("  code        = 0x{:08x}\n", e.code));
        out.push_str(&format!("  property    = 0x{:08x}\n", e.property));
        out.push_str(&format!("  range       = 0x{:08x}\n", e.range));
        if let Some(cl) = &e.count_limit {
            out.push_str(&format!("  count_limit = ({}, 0x{:08x})\n", cl.count, cl.code));
        }
        out.push_str(&format!("  has_condition = {}\n", e.callbacks.condition.is_some()));
        out.push_str(&format!("  has_cost      = {}\n", e.callbacks.cost.is_some()));
        out.push_str(&format!("  has_target    = {}\n", e.callbacks.target.is_some()));
        out.push_str(&format!("  has_operation = {}\n", e.callbacks.operation.is_some()));
    }
    out
}

fn compile_path(path: &Path) -> CompiledCard {
    let src = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    let file = parse(&src)
        .unwrap_or_else(|e| panic!("parse {}: {:?}", path.display(), e));
    let card = file.cards.into_iter().next()
        .unwrap_or_else(|| panic!("no card in {}", path.display()));
    compile_card(&card)
}

fn snapshot_path(card_path: &Path) -> PathBuf {
    let stem = card_path.file_stem().unwrap().to_string_lossy().to_string();
    PathBuf::from(SNAPSHOTS_DIR).join(format!("{}.snap", stem))
}

fn assert_snapshot(card_path: &Path) {
    let compiled = compile_path(card_path);
    let actual = render_compiled(&compiled);
    let snap_path = snapshot_path(card_path);
    fs::create_dir_all(SNAPSHOTS_DIR).expect("create snapshots dir");

    let update = std::env::var("UPDATE_SNAPSHOTS").ok().as_deref() == Some("1");

    if update || !snap_path.exists() {
        fs::write(&snap_path, &actual).unwrap_or_else(|e| {
            panic!("write {}: {}", snap_path.display(), e)
        });
        eprintln!("[snapshot] wrote {}", snap_path.display());
        return;
    }

    let expected = fs::read_to_string(&snap_path)
        .unwrap_or_else(|e| panic!("read {}: {}", snap_path.display(), e));

    if actual != expected {
        // Print a clear diff for human triage.
        eprintln!("\n=== SNAPSHOT MISMATCH: {} ===", snap_path.display());
        for diff in diff_lines(&expected, &actual) {
            eprintln!("{}", diff);
        }
        eprintln!("\nIf this change is intentional, regenerate with:");
        eprintln!("  UPDATE_SNAPSHOTS=1 cargo test --test cards_snapshot\n");
        panic!("snapshot mismatch for {}", card_path.display());
    }
}

/// Tiny line-level diff (no external crate). Yields one entry per
/// differing line. Adequate for snapshot triage.
fn diff_lines(expected: &str, actual: &str) -> Vec<String> {
    let exp: Vec<&str> = expected.lines().collect();
    let act: Vec<&str> = actual.lines().collect();
    let mut out = Vec::new();
    let n = exp.len().max(act.len());
    for i in 0..n {
        let e = exp.get(i).copied().unwrap_or("");
        let a = act.get(i).copied().unwrap_or("");
        if e != a {
            out.push(format!("- {}", e));
            out.push(format!("+ {}", a));
        }
    }
    out
}

#[test]
fn snapshot_all_canonical_cards() {
    let dir = Path::new(CARDS_DIR);
    if !dir.exists() {
        panic!("cards dir not found: {}", dir.display());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .expect("read cards/test")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
        .collect();
    paths.sort();

    if paths.is_empty() {
        panic!("no .ds files found in {}", dir.display());
    }

    let mut failures = Vec::new();
    for p in &paths {
        // Each card asserts independently; collect failures so we
        // see them all in one run instead of bailing on the first.
        let res = std::panic::catch_unwind(|| assert_snapshot(p));
        if let Err(e) = res {
            let msg = e.downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "(panic)".to_string());
            failures.push(format!("{}: {}", p.display(), msg));
        }
    }

    if !failures.is_empty() {
        for f in &failures { eprintln!("FAILED: {}", f); }
        panic!("{} snapshot mismatches across {} cards", failures.len(), paths.len());
    }
}
