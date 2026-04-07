#!/usr/bin/env python3
"""
Manual end-to-end check for the duelscript LSP server.

Spawns the duelscript_lsp binary, drives it through a full
initialize → didOpen → publishDiagnostics roundtrip, and prints
the response. Used for human verification of the JSON-RPC pipeline
when the Rust unit tests can't easily orchestrate the binary.

Build first:
    cargo build --features lsp --bin duelscript_lsp

Run:
    python3 scripts/lsp_manual_check.py

Expected output:
    - initialize response
    - window/logMessage "duelscript-lsp ready"
    - publishDiagnostics with a parse error pointing at line 1
"""
import subprocess, json, time, sys

def frame(s):
    return f"Content-Length: {len(s)}\r\n\r\n{s}".encode()

p = subprocess.Popen(
    ["target/debug/duelscript_lsp"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
)

msgs = [
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}},
    {"jsonrpc": "2.0", "method": "initialized", "params": {}},
    {"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
        "textDocument": {
            "uri": "file:///tmp/bad.ds",
            "languageId": "duelscript",
            "version": 1,
            "text": 'card "Bad" { type: Normal Spell password: 1 '
                    'effect "x" { on_resolve { not_a_real_action } } }',
        }
    }},
]

for m in msgs:
    p.stdin.write(frame(json.dumps(m)))
    p.stdin.flush()
    time.sleep(0.2)

time.sleep(1.0)
p.kill()
out, err = p.communicate(timeout=2)

print("=== STDOUT ===")
print(out.decode(errors="replace"))
if err:
    print("=== STDERR ===")
    print(err.decode(errors="replace"))

# Spot-check
assert b'"id":1' in out, "no initialize response"
assert b"publishDiagnostics" in out, "no diagnostics"
assert b"file:///tmp/bad.ds" in out, "wrong URI"
print("\nOK — LSP smoke check passed.")
