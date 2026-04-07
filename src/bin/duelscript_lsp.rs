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

    async fn shutdown(&self) -> Result<()> { Ok(()) }
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
