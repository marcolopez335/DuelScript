// ============================================================
// duelscript_lsp — Language Server Protocol implementation.
//
// Speaks JSON-RPC over stdio. On every textDocument/didOpen,
// didChange, and didSave, runs the duelscript parser and
// validator and publishes the resulting diagnostics back to the
// editor.
//
// Build:   cargo build --features lsp --bin duelscript_lsp
// Run:     duelscript_lsp                  (editor connects via stdio)
//
// VS Code config snippet (for the generic LSP client extension):
//
//     "duelscript.serverPath": "/path/to/duelscript_lsp"
//
// Neovim (with nvim-lspconfig):
//
//     require'lspconfig'.configs.duelscript = {
//       default_config = {
//         cmd = { "/path/to/duelscript_lsp" },
//         filetypes = { "duelscript" },
//         root_dir = function(fname) return vim.fn.getcwd() end,
//       }
//     }
//     require'lspconfig'.duelscript.setup{}
// ============================================================

#![cfg(feature = "lsp")]

use std::collections::HashMap;
use std::sync::Mutex;

use duelscript::parse;
use duelscript::validator::{validate_card, Severity};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

struct Backend {
    client: Client,
    /// Per-URI document text. Stored so didChange can replace and
    /// re-validate without reading from disk.
    documents: Mutex<HashMap<Url, String>>,
}

impl Backend {
    fn store(&self, uri: Url, text: String) {
        self.documents.lock().unwrap().insert(uri, text);
    }
    fn get(&self, uri: &Url) -> Option<String> {
        self.documents.lock().unwrap().get(uri).cloned()
    }

    /// Parse + validate the document and return LSP diagnostics.
    /// Parse errors override validator output: if the file doesn't
    /// parse, validation can't run.
    fn diagnostics_for(&self, source: &str) -> Vec<Diagnostic> {
        match parse(source) {
            Err(e) => {
                let msg = format!("{}", e);
                // Pest errors embed `--> line:col` in the message.
                let (line, col) = extract_pest_position(&msg).unwrap_or((0, 0));
                vec![Diagnostic {
                    range: Range {
                        start: Position { line, character: col },
                        end:   Position { line, character: col + 1 },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("duelscript".to_string()),
                    message: msg,
                    ..Default::default()
                }]
            }
            Ok(file) => {
                let mut out = Vec::new();
                for card in &file.cards {
                    let mut errs = Vec::new();
                    validate_card(card, &mut errs);
                    // Anchor each error at the card's declaration line
                    // (best-effort: search for `card "<name>"` in the source).
                    let anchor = find_card_line(source, &card.name).unwrap_or(0);
                    for e in errs {
                        let sev = match e.severity {
                            Severity::Error   => DiagnosticSeverity::ERROR,
                            Severity::Warning => DiagnosticSeverity::WARNING,
                        };
                        out.push(Diagnostic {
                            range: Range {
                                start: Position { line: anchor, character: 0 },
                                end:   Position { line: anchor, character: 1 },
                            },
                            severity: Some(sev),
                            source: Some("duelscript".to_string()),
                            message: e.message,
                            ..Default::default()
                        });
                    }
                }
                out
            }
        }
    }

    async fn publish(&self, uri: Url) {
        let Some(text) = self.get(&uri) else { return };
        let diagnostics = self.diagnostics_for(&text);
        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![":".to_string(), " ".to_string()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "duelscript-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "duelscript-lsp ready")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.store(uri.clone(), params.text_document.text);
        self.publish(uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // FULL sync mode: every change carries the entire new document.
        if let Some(change) = params.content_changes.into_iter().next() {
            let uri = params.text_document.uri.clone();
            self.store(uri.clone(), change.text);
            self.publish(uri).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let Some(text) = params.text {
            let uri = params.text_document.uri.clone();
            self.store(uri.clone(), text);
            self.publish(uri).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.lock().unwrap().remove(&uri);
        // Clear diagnostics for the closed document.
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let text = match self.get(uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        // Find the current line and the word before the cursor
        let line_text = text.lines().nth(pos.line as usize).unwrap_or("");
        let before_cursor = &line_text[..std::cmp::min(pos.character as usize, line_text.len())];

        let items = get_completions(before_cursor);
        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let text = match self.get(uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        let line_text = text.lines().nth(pos.line as usize).unwrap_or("");
        let col = pos.character as usize;
        let word = extract_word_at(line_text, col);

        if let Some(doc) = keyword_docs(&word) {
            Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc.to_string(),
                }),
                range: None,
            }))
        } else {
            Ok(None)
        }
    }

    async fn shutdown(&self) -> Result<()> { Ok(()) }
}

/// Extract the word under the cursor position.
fn extract_word_at(line: &str, col: usize) -> String {
    let bytes = line.as_bytes();
    let col = col.min(line.len());
    let start = (0..col).rev()
        .find(|&i| !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'_')
        .map(|i| i + 1)
        .unwrap_or(0);
    let end = (col..line.len())
        .find(|&i| !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'_')
        .unwrap_or(line.len());
    line[start..end].to_string()
}

/// Context-aware completions based on what's before the cursor.
fn get_completions(before: &str) -> Vec<CompletionItem> {
    let trimmed = before.trim();

    // After "trigger:" → suggest trigger names
    if trimmed.ends_with("trigger:") || trimmed.ends_with("trigger: ") {
        return make_items(&[
            ("when_summoned", "Triggers when this card is summoned"),
            ("when_destroyed", "Triggers when this card is destroyed"),
            ("when_attacked", "Triggers when this card is attacked"),
            ("when_sent_to gy", "Triggers when sent to the GY"),
            ("when_flipped", "Triggers when flipped face-up"),
            ("when_leaves_field", "Triggers when leaving the field"),
            ("when_banished", "Triggers when banished"),
            ("during_end_phase", "Triggers during the End Phase"),
            ("during_standby_phase", "Triggers during the Standby Phase"),
            ("opponent_activates [...]", "Triggers when opponent activates a card/effect"),
            ("when attack_declared", "Triggers when any attack is declared"),
        ]);
    }

    // After "speed:" → suggest speeds
    if trimmed.ends_with("speed:") || trimmed.ends_with("speed: ") {
        return make_items(&[
            ("spell_speed_1", "Normal speed (spells, ignition effects)"),
            ("spell_speed_2", "Quick speed (traps, quick effects, hand traps)"),
            ("spell_speed_3", "Counter trap speed"),
        ]);
    }

    // After "grant:" → suggest abilities
    if trimmed.ends_with("grant:") || trimmed.ends_with("grant: ") {
        return make_items(&[
            ("piercing", "Inflict piercing battle damage"),
            ("direct_attack", "Can attack directly"),
            ("double_attack", "Can attack twice"),
            ("cannot_be_destroyed_by_battle", "Cannot be destroyed by battle"),
            ("cannot_be_destroyed_by_effect", "Cannot be destroyed by card effects"),
            ("cannot_be_targeted_by_card_effects", "Cannot be targeted"),
            ("unaffected_by_card_effects", "Unaffected by other card effects"),
            ("cannot_attack", "Cannot declare attacks"),
            ("cannot_be_tributed", "Cannot be tributed"),
            ("lp_cost_zero", "LP costs become 0"),
        ]);
    }

    // After "type:" → suggest card types
    if trimmed.ends_with("type:") || trimmed.ends_with("type: ") {
        return make_items(&[
            ("Effect Monster", "Monster with effects"),
            ("Normal Monster", "Vanilla monster"),
            ("Normal Spell", "Normal spell card"),
            ("Quick-Play Spell", "Quick-play spell"),
            ("Continuous Spell", "Stays on field"),
            ("Equip Spell", "Equips to a monster"),
            ("Field Spell", "Field spell"),
            ("Normal Trap", "Normal trap card"),
            ("Counter Trap", "Spell speed 3 trap"),
            ("Continuous Trap", "Stays on field"),
            ("Ritual Monster", "Ritual summoned monster"),
            ("Fusion Monster", "Fusion summoned"),
            ("Synchro Monster", "Synchro summoned"),
            ("Xyz Monster", "Xyz summoned"),
            ("Link Monster", "Link summoned"),
        ]);
    }

    // After "condition:" → suggest conditions
    if trimmed.ends_with("condition:") || trimmed.ends_with("condition: ") {
        return make_items(&[
            ("on_field", "This card is on the field"),
            ("in_gy", "This card is in the GY"),
            ("you_control_no_monsters", "You control no monsters"),
            ("chain_link_includes [...]", "Chain includes specific categories"),
            ("you_control (1, monster)", "You control a matching card"),
            ("your_lp >= 2000", "Your LP is at or above threshold"),
        ]);
    }

    // Top-level block suggestions
    if trimmed.is_empty() || trimmed.ends_with("{") {
        return make_items(&[
            ("effect \"Name\" {", "Trigger/ignition/quick effect block"),
            ("continuous_effect \"Name\" {", "Always-active field/self effect"),
            ("flip_effect \"Name\" {", "FLIP effect block"),
            ("replacement_effect \"Name\" {", "\"Instead of X, do Y\" effect"),
            ("redirect_effect \"Name\" {", "Redirect cards going to zone X to zone Y"),
            ("summon_condition {", "Summon restrictions"),
            ("materials {", "Extra deck summoning materials"),
            ("cost {", "Activation cost"),
            ("on_resolve {", "Effect resolution actions"),
        ]);
    }

    vec![]
}

fn make_items(pairs: &[(&str, &str)]) -> Vec<CompletionItem> {
    pairs.iter().map(|(label, detail)| CompletionItem {
        label: label.to_string(),
        detail: Some(detail.to_string()),
        kind: Some(CompletionItemKind::KEYWORD),
        ..Default::default()
    }).collect()
}

/// Hover documentation for DuelScript keywords.
fn keyword_docs(word: &str) -> Option<&'static str> {
    Some(match word {
        "spell_speed_1" => "**Spell Speed 1** — Normal activation speed. Used by Normal/Continuous/Field Spells, Ignition effects, and Trigger effects. Cannot chain to Spell Speed 2+.",
        "spell_speed_2" => "**Spell Speed 2** — Quick activation speed. Used by Quick-Play Spells, Trap Cards, Quick Effects. Can chain to Spell Speed 1-2.",
        "spell_speed_3" => "**Spell Speed 3** — Counter Trap speed. Only Counter Traps can chain to this.",
        "once_per_turn" => "**Once Per Turn** — This effect can only be activated once per turn.\n- `soft`: resets if the card leaves and returns to field\n- `hard`: tracked by card name, doesn't reset",
        "when_summoned" => "**When Summoned** — Triggers when this card is successfully summoned. Add `by_special_summon`, `by_normal_summon`, etc. to narrow.",
        "when_destroyed" => "**When Destroyed** — Triggers when this card is destroyed. Add `by battle` or `by card_effect` to narrow.",
        "when_attacked" => "**When Attacked** — Triggers when this monster is selected as an attack target.",
        "on_resolve" => "**On Resolve** — The actions that happen when this effect resolves. Contains game actions like `draw`, `destroy`, `special_summon`, etc.",
        "on_activate" => "**On Activate** — Targeting and setup actions when the effect is activated (before resolution).",
        "cost" => "**Cost** — Must be paid to activate the effect. Common costs: `tribute self`, `discard self`, `pay_lp N`, `detach N overlay_unit from self`.",
        "continuous_effect" => "**Continuous Effect** — Always active while this card is face-up. Use `scope: field` for effects on other cards, `scope: self` for self-only.",
        "replacement_effect" => "**Replacement Effect** — \"If X would happen, do Y instead.\" Uses `instead_of:` + `do:` blocks.",
        "redirect_effect" => "**Redirect Effect** — Cards going to one zone get sent to another instead (e.g., Dimensional Fissure sends GY-bound cards to banished).",
        "grant" => "**Grant** — Give an ability to affected cards. Examples: `piercing`, `cannot_be_destroyed_by_battle`, `direct_attack`.",
        "template" => "**Template** — Reusable effect pattern. Define once, use in multiple cards via `use <name>`.",
        "while_on_field" => "**While On Field** — Duration that lasts as long as this card remains on the field.",
        "raw_effect" => "**Raw Effect** — Low-level effect block with explicit EDOPro bitfields. Used when the semantic `effect` block can't express the pattern.",
        _ => return None,
    })
}

/// Pest error messages contain `  --> line:col` somewhere. Pull the
/// numbers out so we can anchor the diagnostic precisely.
fn extract_pest_position(msg: &str) -> Option<(u32, u32)> {
    let arrow = msg.find("-->")?;
    let rest = &msg[arrow + 3..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let pos_str = rest[..line_end].trim();
    let mut parts = pos_str.split(':');
    let line: u32 = parts.next()?.trim().parse().ok()?;
    let col:  u32 = parts.next()?.trim().parse().ok()?;
    // pest is 1-indexed; LSP is 0-indexed.
    Some((line.saturating_sub(1), col.saturating_sub(1)))
}

/// Find the 0-indexed line containing `card "<name>"` in the source.
fn find_card_line(source: &str, name: &str) -> Option<u32> {
    let needle = format!("card \"{}\"", name);
    for (i, line) in source.lines().enumerate() {
        if line.contains(&needle) {
            return Some(i as u32);
        }
    }
    None
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: Mutex::new(HashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
