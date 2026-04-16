use tower_lsp::{LspService, Server};
use duelscript::v2::lsp::Backend;
use std::sync::Mutex;
use std::collections::HashMap;

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
