// ============================================================
// Sprint 59: LLM-assisted card generation.
//
// Generates DuelScript .ds files from English card text by calling
// the Claude API with few-shot examples. The generated output is
// validated through parse → validate → compile.
// ============================================================

#![cfg(feature = "llm")]

use std::path::Path;
use std::fs;

/// Build the system prompt + few-shot examples for the LLM.
pub fn build_prompt(
    card_name: &str,
    card_text: &str,
    card_type: &str,
    stats: &str,
    examples_dir: &Path,
) -> String {
    let mut prompt = String::new();

    prompt.push_str(
        "You are a DuelScript expert. DuelScript is a domain-specific language for \
         defining Yu-Gi-Oh card mechanics. Given a card's name, type, stats, and \
         effect text, generate the complete .ds file.\n\n\
         Rules:\n\
         - Use exact DuelScript syntax (it's PEG-parsed, so syntax errors = failure)\n\
         - Use `id:` for the card's passcode number\n\
         - Use named constants like Activate, Destroy, FreeChain (not raw numbers)\n\
         - Effect blocks: `effect \"Name\" { speed: ... trigger: ... on_resolve { ... } }`\n\
         - Continuous effects: `continuous_effect \"Name\" { scope: field apply_to: ... grant: ... }`\n\
         - Cost blocks go inside effect: `cost { pay_lp 1000 }` or `cost { discard self }`\n\
         - Triggers: when_summoned, when_destroyed, when_attacked, during_end_phase, opponent_activates [...]\n\
         - Actions: draw N, destroy (...), special_summon (...) from zone, negate activation, banish (...), etc.\n\
         - Target expressions: (count, filter, controller controls, zone, qualifiers)\n\
         - Grant abilities: cannot_be_destroyed_by_battle, piercing, direct_attack, etc.\n\
         - Output ONLY the .ds file content, nothing else. No markdown fences.\n\n"
    );

    // Add few-shot examples from cards/test/
    let mut examples = Vec::new();
    if examples_dir.exists() {
        if let Ok(entries) = fs::read_dir(examples_dir) {
            let mut paths: Vec<_> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
                .collect();
            paths.sort();
            // Pick a diverse sample: first 8 cards
            for p in paths.iter().take(8) {
                if let Ok(content) = fs::read_to_string(p) {
                    examples.push(content);
                }
            }
        }
    }

    if !examples.is_empty() {
        prompt.push_str("Here are example .ds files showing the correct syntax:\n\n");
        for (i, ex) in examples.iter().enumerate() {
            prompt.push_str(&format!("--- Example {} ---\n{}\n\n", i + 1, ex));
        }
    }

    prompt.push_str(&format!(
        "Now generate the .ds file for this card:\n\
         Name: {}\n\
         Type: {}\n\
         Stats: {}\n\
         Effect Text: \"{}\"\n\n\
         Output the complete .ds file:",
        card_name, card_type, stats, card_text
    ));

    prompt
}

/// Call the Claude API and return the generated .ds content.
pub async fn call_claude(
    prompt: &str,
    api_key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 2048,
        "messages": [
            {
                "role": "user",
                "content": prompt
            }
        ]
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await?;

    if !status.is_success() {
        return Err(format!("API error {}: {}", status, text).into());
    }

    // Parse the response to extract the text content
    let json: serde_json::Value = serde_json::from_str(&text)?;
    let content = json["content"][0]["text"]
        .as_str()
        .ok_or("No text in response")?
        .to_string();

    // Strip any markdown fences the model might add
    let cleaned = content
        .trim()
        .strip_prefix("```duelscript")
        .or_else(|| content.trim().strip_prefix("```"))
        .unwrap_or(content.trim())
        .strip_suffix("```")
        .unwrap_or(content.trim())
        .trim()
        .to_string();

    Ok(cleaned)
}

/// Validate generated .ds content through parse → validate → compile.
/// Returns Ok(()) if all steps pass, or Err with the first error message.
pub fn validate_generated(ds_content: &str) -> Result<(), String> {
    // Step 1: parse
    let file = crate::parse(ds_content)
        .map_err(|e| format!("Parse error: {}", e))?;

    // Step 2: validate
    let errors = crate::validator::validate(&file);
    let real_errors: Vec<_> = errors.iter()
        .filter(|e| e.severity == crate::validator::Severity::Error)
        .collect();
    if !real_errors.is_empty() {
        return Err(format!("Validation error: {}", real_errors[0]));
    }

    // Step 3: compile each card
    for card in &file.cards {
        let _compiled = crate::compiler::compile_card(card);
    }

    Ok(())
}
