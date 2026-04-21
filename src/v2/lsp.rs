#![cfg(feature = "lsp")]

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use std::sync::Mutex;
use std::collections::HashMap;

pub struct Backend {
    pub client: Client,
    pub documents: Mutex<HashMap<Url, String>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![" ".to_string(), ":".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "duelscript-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client.log_message(MessageType::INFO, "DuelScript LSP initialized").await;
    }

    async fn shutdown(&self) -> Result<()> { Ok(()) }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        self.documents.lock().unwrap().insert(uri.clone(), text.clone());
        self.publish_diagnostics(uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.into_iter().next() {
            let text = change.text;
            self.documents.lock().unwrap().insert(uri.clone(), text.clone());
            self.publish_diagnostics(uri, &text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.lock().unwrap().remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let docs = self.documents.lock().unwrap();
        let text = match docs.get(&uri) {
            Some(t) => t.clone(),
            None => return Ok(None),
        };
        drop(docs);
        let word = word_at_position(&text, pos);
        Ok(hover_help(&word).map(|content| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        }))
    }

    async fn completion(&self, _: CompletionParams) -> Result<Option<CompletionResponse>> {
        let items: Vec<CompletionItem> = KEYWORDS.iter().map(|kw| CompletionItem {
            label: kw.0.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(kw.1.to_string()),
            ..Default::default()
        }).collect();
        Ok(Some(CompletionResponse::Array(items)))
    }
}

impl Backend {
    async fn publish_diagnostics(&self, uri: Url, text: &str) {
        let diagnostics = compute_diagnostics(text);
        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

pub fn compute_diagnostics(source: &str) -> Vec<Diagnostic> {
    use crate::v2::parser::parse_v2;
    use crate::v2::validator::{validate_v2, Severity};

    let mut diagnostics = Vec::new();
    match parse_v2(source) {
        Err(e) => {
            let msg = e.to_string();
            // Try to extract line/col from pest error
            let (line, col) = extract_line_col(&msg);
            diagnostics.push(Diagnostic {
                range: Range::new(
                    Position::new(line.saturating_sub(1) as u32, col.saturating_sub(1) as u32),
                    Position::new(line.saturating_sub(1) as u32, (col + 1).saturating_sub(1) as u32),
                ),
                severity: Some(DiagnosticSeverity::ERROR),
                message: msg,
                source: Some("duelscript".into()),
                ..Default::default()
            });
        }
        Ok(file) => {
            let report = validate_v2(&file);
            for err in &report.errors {
                let sev = match err.severity {
                    Severity::Error => DiagnosticSeverity::ERROR,
                    Severity::Warning => DiagnosticSeverity::WARNING,
                };
                diagnostics.push(Diagnostic {
                    range: Range::new(Position::new(0, 0), Position::new(0, 1)),
                    severity: Some(sev),
                    message: err.message.clone(),
                    source: Some("duelscript".into()),
                    ..Default::default()
                });
            }
        }
    }
    diagnostics
}

fn extract_line_col(msg: &str) -> (usize, usize) {
    // Pest errors look like: " --> 5:12"
    if let Some(pos) = msg.find(" --> ") {
        let rest = &msg[pos + 5..];
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        if parts.len() == 2 {
            let line = parts[0].trim().parse().unwrap_or(1);
            let col_str = parts[1].split_whitespace().next().unwrap_or("1");
            let col = col_str.parse().unwrap_or(1);
            return (line, col);
        }
    }
    (1, 1)
}

fn word_at_position(text: &str, pos: Position) -> String {
    let line = match text.lines().nth(pos.line as usize) {
        Some(l) => l,
        None => return String::new(),
    };
    let col = pos.character as usize;
    if col >= line.len() { return String::new(); }
    let bytes = line.as_bytes();
    let mut start = col;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    let mut end = col;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    line[start..end].to_string()
}

fn hover_help(word: &str) -> Option<String> {
    match word {
        "card" => Some("**card** — Top-level card declaration.\n\n```\ncard \"Name\" {\n    id: 12345\n    type: Normal Spell\n}\n```".into()),
        "effect" => Some("**effect** — Activated or trigger effect.\n\nProperties: speed, trigger, condition, cost, resolve, mandatory, once_per_turn".into()),
        "passive" => Some("**passive** — Continuous effect (always on while face-up).".into()),
        "restriction" => Some("**restriction** — Grant or deny abilities.".into()),
        "replacement" => Some("**replacement** — Replace events with alternate outcomes.".into()),
        "summon" => Some("**summon** — Summon block: tributes, materials, special summon procedures.".into()),
        "speed" => Some("**speed** — Spell speed.\n\n- `1` — Ignition/Trigger (Normal Spells)\n- `2` — Quick (Traps, Quick-Play)\n- `3` — Counter Traps only".into()),
        "mandatory" => Some("**mandatory** — This effect must activate when triggered.".into()),
        "once_per_turn" => Some("**once_per_turn: hard | soft**\n\n- `hard` — tied to card name, can't retry if negated\n- `soft` — can retry if negated".into()),
        "trigger" => Some("**trigger** — What activates this effect.\n\nExamples: `summoned`, `destroyed_by_battle`, `end_phase`, `opponent_activates [negate]`, `you_activates [discard]`, `any_activates [equip]`".into()),
        "condition" => Some("**condition** — Must be true to activate.\n\nExamples: `lp >= 1000`, `cards_in_gy >= 5`, `on_field`".into()),
        "cost" => Some("**cost** — Paid on activation.\n\nExamples: `pay_lp 1000`, `discard (1, card)`, `tribute self`".into()),
        "resolve" => Some("**resolve** — Actions performed when effect resolves.".into()),
        "draw" => Some("**draw N** — Draw N cards from deck.".into()),
        "destroy" => Some("**destroy (selector)** — Destroy matching cards.".into()),
        "banish" => Some("**banish (selector)** — Banish (remove from play).".into()),
        "special_summon" => Some("**special_summon (selector)** — Special Summon from hand/gy/deck/banished.".into()),
        "negate" => Some("**negate** or **negate and destroy** — Negate activation and optionally destroy.".into()),
        "damage" => Some("**damage player amount** — Inflict damage.\n\nExample: `damage opponent 1000`".into()),
        "ritual_summon" => Some("**ritual_summon (selector) using (materials) where total_level >= N**".into()),
        "fusion_summon" => Some("**fusion_summon (selector) using (materials)**".into()),
        _ => None,
    }
}

const KEYWORDS: &[(&str, &str)] = &[
    ("card", "card declaration"), ("effect", "effect block"), ("passive", "continuous effect"),
    ("restriction", "restriction block"), ("replacement", "replacement block"), ("summon", "summon block"),
    ("id", "card passcode"), ("type", "card type"), ("attribute", "monster attribute"),
    ("race", "monster race"), ("level", "monster level"), ("rank", "xyz rank"),
    ("link", "link rating"), ("scale", "pendulum scale"), ("atk", "attack"), ("def", "defense"),
    ("speed", "spell speed"), ("mandatory", "mandatory trigger"), ("once_per_turn", "frequency"),
    ("trigger", "activation trigger"), ("condition", "activation condition"), ("cost", "cost block"),
    ("resolve", "resolve block"), ("target", "target declaration"),
    ("draw", "draw cards"), ("destroy", "destroy cards"), ("banish", "banish cards"),
    ("send", "send to zone"), ("return", "return to zone"), ("search", "search deck"),
    ("add_to_hand", "add to hand"), ("special_summon", "special summon"),
    ("normal_summon", "normal summon"), ("negate", "negate activation"),
    ("damage", "deal damage"), ("gain_lp", "gain LP"), ("pay_lp", "pay LP"),
    ("modify_atk", "modify ATK"), ("modify_def", "modify DEF"),
    ("discard", "discard"), ("mill", "mill from deck"),
    ("ritual_summon", "ritual summon"), ("fusion_summon", "fusion summon"),
    ("synchro_summon", "synchro summon"), ("xyz_summon", "xyz summon"),
    ("swap_control", "swap control"), ("swap_stats", "swap ATK/DEF"),
    ("grant", "grant ability"), ("change_level", "change level"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostics_for_valid_source() {
        let source = r#"card "Test" { id: 1 type: Normal Spell effect "Draw" { speed: 1 resolve { draw 2 } } }"#;
        let diags = compute_diagnostics(source);
        assert!(diags.is_empty(), "Expected no diagnostics for valid source, got: {:?}", diags);
    }

    #[test]
    fn test_diagnostics_for_invalid_source() {
        let source = "not valid duelscript at all";
        let diags = compute_diagnostics(source);
        assert!(!diags.is_empty(), "Expected parse error diagnostics");
    }

    #[test]
    fn test_hover_known_keywords() {
        assert!(hover_help("card").is_some());
        assert!(hover_help("draw").is_some());
        assert!(hover_help("speed").is_some());
        assert!(hover_help("xyzfoo").is_none());
    }

    #[test]
    fn test_completion_list() {
        assert!(KEYWORDS.iter().any(|(k, _)| *k == "card"));
        assert!(KEYWORDS.iter().any(|(k, _)| *k == "ritual_summon"));
    }
}
