// SPDX-License-Identifier: MIT
// LumaScript for Visual Studio Code: starts `lumalink lsp` and speaks the Language Server Protocol to it over its
// standard input and output. No packages: the protocol's framing and the few shapes VS Code needs are written here.
'use strict';

const vscode = require('vscode');
const cp = require('child_process');
const path = require('path');

const LANGUAGE = 'lumascript';
const SELECTOR = [{ language: LANGUAGE, scheme: 'file' }];

let client = null;
let output = null;
let diagnostics = null;

/// One running server: the process, the requests waiting for their answer and the documents it was told about.
class Client {
    constructor(command, args, cwd) {
        this.waiting = new Map();
        this.next = 1;
        this.buffer = Buffer.alloc(0);
        this.ready = null;
        this.stopped = false;
        this.process = cp.spawn(command, args, { cwd, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
        this.process.stdout.on('data', (chunk) => this.read(chunk));
        this.process.stderr.on('data', (chunk) => output.append(chunk.toString('utf8')));
        this.process.on('error', (error) => {
            output.appendLine(`LumaScript: ${command} could not start (${error.message}). Set lumascript.lumalink to LumaLink's program.`);
            void vscode.window.showErrorMessage(`LumaScript: LumaLink (${command}) could not start. Set "lumascript.lumalink" to its program.`);
            this.fail(error);
        });
        this.process.on('exit', (code) => {
            if (!this.stopped) {
                output.appendLine(`LumaScript: the server left (exit ${code}).`);
            }
            this.fail(new Error(`the server left (exit ${code})`));
        });
    }

    fail(error) {
        for (const [, w] of this.waiting) {
            w.reject(error);
        }
        this.waiting.clear();
        this.stopped = true;
    }

    /// Messages are `Content-Length: N\r\n\r\n` and N bytes of JSON.
    read(chunk) {
        this.buffer = Buffer.concat([this.buffer, chunk]);
        for (;;) {
            const end = this.buffer.indexOf('\r\n\r\n');
            if (end < 0) {
                return;
            }
            const header = this.buffer.slice(0, end).toString('ascii');
            const match = /Content-Length:\s*(\d+)/i.exec(header);
            if (!match) {
                this.buffer = this.buffer.slice(end + 4);
                continue;
            }
            const length = Number(match[1]);
            if (this.buffer.length < end + 4 + length) {
                return;
            }
            const body = this.buffer.slice(end + 4, end + 4 + length).toString('utf8');
            this.buffer = this.buffer.slice(end + 4 + length);
            let message;
            try {
                message = JSON.parse(body);
            } catch (e) {
                output.appendLine(`LumaScript: a message that is not JSON: ${e.message}`);
                continue;
            }
            this.take(message);
        }
    }

    take(message) {
        if (message.id !== undefined && message.method === undefined) {
            const w = this.waiting.get(message.id);
            if (w) {
                this.waiting.delete(message.id);
                if (message.error) {
                    w.reject(new Error(message.error.message));
                } else {
                    w.resolve(message.result);
                }
            }
            return;
        }
        if (message.method === 'textDocument/publishDiagnostics') {
            const p = message.params;
            diagnostics.set(vscode.Uri.parse(p.uri), p.diagnostics.map(toDiagnostic));
        } else if (message.method === 'window/logMessage' || message.method === 'window/showMessage') {
            output.appendLine(message.params.message);
        } else if (message.id !== undefined) {
            // A request from the server this client does not serve.
            this.write({ jsonrpc: '2.0', id: message.id, error: { code: -32601, message: 'not served' } });
        }
    }

    write(message) {
        if (this.stopped) {
            return;
        }
        const body = Buffer.from(JSON.stringify(message), 'utf8');
        this.process.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
        this.process.stdin.write(body);
    }

    request(method, params, token) {
        if (this.stopped) {
            return Promise.resolve(null);
        }
        const id = this.next++;
        const answer = new Promise((resolve, reject) => this.waiting.set(id, { resolve, reject }));
        this.write({ jsonrpc: '2.0', id, method, params });
        if (token) {
            token.onCancellationRequested(() => this.notify('$/cancelRequest', { id }));
        }
        return answer;
    }

    notify(method, params) {
        this.write({ jsonrpc: '2.0', method, params });
    }

    async start(root) {
        await this.request('initialize', {
            processId: process.pid,
            clientInfo: { name: 'vscode-lumascript' },
            rootUri: root ? root.toString() : null,
            capabilities: {
                general: { positionEncodings: ['utf-16'] },
                textDocument: {
                    synchronization: { didSave: true },
                    completion: { completionItem: { documentationFormat: ['markdown', 'plaintext'] } },
                    hover: { contentFormat: ['markdown', 'plaintext'] },
                    documentSymbol: { hierarchicalDocumentSymbolSupport: true },
                    rename: { prepareSupport: true },
                },
            },
        });
        this.notify('initialized', {});
    }

    async stop() {
        if (this.stopped) {
            return;
        }
        try {
            await Promise.race([this.request('shutdown', null), new Promise((r) => setTimeout(r, 2000))]);
            this.notify('exit', null);
        } catch (e) {
            // Already gone.
        }
        this.stopped = true;
        setTimeout(() => this.process.kill(), 1000);
    }
}

function position(p) {
    return new vscode.Position(p.line, p.character);
}

function range(r) {
    return new vscode.Range(position(r.start), position(r.end));
}

function location(l) {
    return new vscode.Location(vscode.Uri.parse(l.uri), range(l.range));
}

function markdown(content) {
    if (content === undefined || content === null) {
        return undefined;
    }
    if (typeof content === 'string') {
        return new vscode.MarkdownString(content);
    }
    if (Array.isArray(content)) {
        return content.map(markdown);
    }
    if (content.kind === 'plaintext') {
        const m = new vscode.MarkdownString();
        m.appendText(content.value);
        return m;
    }
    return new vscode.MarkdownString(content.value !== undefined ? content.value : '');
}

const SEVERITY = [undefined, vscode.DiagnosticSeverity.Error, vscode.DiagnosticSeverity.Warning, vscode.DiagnosticSeverity.Information, vscode.DiagnosticSeverity.Hint];

function toDiagnostic(d) {
    const out = new vscode.Diagnostic(range(d.range), d.message, SEVERITY[d.severity] !== undefined ? SEVERITY[d.severity] : vscode.DiagnosticSeverity.Error);
    if (d.code !== undefined) {
        out.code = d.code;
    }
    out.source = d.source || 'lumalink';
    return out;
}

function textEdit(e) {
    return new vscode.TextEdit(range(e.range), e.newText);
}

function workspaceEdit(w) {
    const out = new vscode.WorkspaceEdit();
    if (!w) {
        return out;
    }
    if (w.changes) {
        for (const uri of Object.keys(w.changes)) {
            out.set(vscode.Uri.parse(uri), w.changes[uri].map(textEdit));
        }
    }
    if (w.documentChanges) {
        for (const c of w.documentChanges) {
            if (c.textDocument && c.edits) {
                out.set(vscode.Uri.parse(c.textDocument.uri), c.edits.map(textEdit));
            }
        }
    }
    return out;
}

// LSP CompletionItemKind and SymbolKind count from 1; VS Code's from 0.
function completionItem(i) {
    const out = new vscode.CompletionItem(i.label, i.kind ? i.kind - 1 : undefined);
    out.detail = i.detail;
    out.documentation = markdown(i.documentation);
    out.sortText = i.sortText;
    out.filterText = i.filterText;
    if (i.textEdit) {
        out.range = range(i.textEdit.range || i.textEdit.insert);
        out.insertText = i.insertTextFormat === 2 ? new vscode.SnippetString(i.textEdit.newText) : i.textEdit.newText;
    } else if (i.insertText !== undefined) {
        out.insertText = i.insertTextFormat === 2 ? new vscode.SnippetString(i.insertText) : i.insertText;
    }
    if (i.additionalTextEdits) {
        out.additionalTextEdits = i.additionalTextEdits.map(textEdit);
    }
    return out;
}

function documentSymbol(s) {
    if (s.location) {
        return new vscode.SymbolInformation(s.name, s.kind - 1, s.containerName || '', location(s.location));
    }
    const out = new vscode.DocumentSymbol(s.name, s.detail || '', s.kind - 1, range(s.range), range(s.selectionRange));
    out.children = (s.children || []).map(documentSymbol);
    return out;
}

function at(document, where) {
    return { textDocument: { uri: document.uri.toString() }, position: { line: where.line, character: where.character } };
}

function served(document) {
    return document.languageId === LANGUAGE && document.uri.scheme === 'file';
}

function open(document) {
    if (client && served(document)) {
        client.notify('textDocument/didOpen', { textDocument: { uri: document.uri.toString(), languageId: LANGUAGE, version: document.version, text: document.getText() } });
    }
}

function settings() {
    const c = vscode.workspace.getConfiguration('lumascript');
    const args = ['lsp'];
    const names = c.get('names');
    if (names) {
        args.push('--names', names);
    }
    const sdk = c.get('sdk');
    if (sdk) {
        args.push('--sdk', sdk);
    }
    return { command: c.get('lumalink') || 'lumalink', args };
}

async function start() {
    const { command, args } = settings();
    const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
    const cwd = folder ? folder.uri.fsPath : undefined;
    output.appendLine(`LumaScript: ${command} ${args.join(' ')}`);
    client = new Client(command, args, cwd);
    try {
        await client.start(folder ? folder.uri : null);
    } catch (e) {
        output.appendLine(`LumaScript: the server did not answer initialize (${e.message}).`);
        return;
    }
    vscode.workspace.textDocuments.forEach(open);
}

async function restart() {
    if (client) {
        const old = client;
        client = null;
        await old.stop();
    }
    diagnostics.clear();
    await start();
}

function ask(method, params, token, convert) {
    if (!client) {
        return null;
    }
    return client.request(method, params, token).then((r) => (r === null || r === undefined ? null : convert(r)), () => null);
}

function activate(context) {
    output = vscode.window.createOutputChannel('LumaScript');
    diagnostics = vscode.languages.createDiagnosticCollection(LANGUAGE);
    context.subscriptions.push(output, diagnostics);

    context.subscriptions.push(
        vscode.workspace.onDidOpenTextDocument(open),
        vscode.workspace.onDidChangeTextDocument((e) => {
            if (client && served(e.document) && e.contentChanges.length) {
                client.notify('textDocument/didChange', { textDocument: { uri: e.document.uri.toString(), version: e.document.version }, contentChanges: [{ text: e.document.getText() }] });
            }
        }),
        vscode.workspace.onDidSaveTextDocument((d) => {
            if (client && (served(d) || path.basename(d.uri.fsPath) === 'luma.json')) {
                client.notify('textDocument/didSave', { textDocument: { uri: d.uri.toString() } });
            }
        }),
        vscode.workspace.onDidCloseTextDocument((d) => {
            if (client && served(d)) {
                client.notify('textDocument/didClose', { textDocument: { uri: d.uri.toString() } });
            }
        }),
        vscode.workspace.onDidChangeConfiguration((e) => {
            if (e.affectsConfiguration('lumascript')) {
                void restart();
            }
        }),
        vscode.commands.registerCommand('lumascript.restart', restart),
        vscode.languages.registerCompletionItemProvider(SELECTOR, {
            provideCompletionItems: (d, p, t) => ask('textDocument/completion', at(d, p), t, (r) => {
                const items = Array.isArray(r) ? r : r.items || [];
                return new vscode.CompletionList(items.map(completionItem), !Array.isArray(r) && r.isIncomplete === true);
            }),
        }, '.', ':', '`', '/'),
        vscode.languages.registerHoverProvider(SELECTOR, {
            provideHover: (d, p, t) => ask('textDocument/hover', at(d, p), t, (r) => {
                const contents = markdown(r.contents);
                return new vscode.Hover(Array.isArray(contents) ? contents : [contents], r.range ? range(r.range) : undefined);
            }),
        }),
        vscode.languages.registerDefinitionProvider(SELECTOR, {
            provideDefinition: (d, p, t) => ask('textDocument/definition', at(d, p), t, (r) => (Array.isArray(r) ? r.map(location) : location(r))),
        }),
        vscode.languages.registerReferenceProvider(SELECTOR, {
            provideReferences: (d, p, c, t) => ask('textDocument/references', Object.assign(at(d, p), { context: { includeDeclaration: c.includeDeclaration } }), t, (r) => r.map(location)),
        }),
        vscode.languages.registerRenameProvider(SELECTOR, {
            prepareRename: async (d, p, t) => {
                const r = await ask('textDocument/prepareRename', at(d, p), t, (x) => x);
                if (!r) {
                    throw new Error('This name cannot be renamed.');
                }
                return r.range ? { range: range(r.range), placeholder: r.placeholder } : range(r);
            },
            provideRenameEdits: (d, p, name, t) => ask('textDocument/rename', Object.assign(at(d, p), { newName: name }), t, workspaceEdit),
        }),
        vscode.languages.registerSignatureHelpProvider(SELECTOR, {
            provideSignatureHelp: (d, p, t) => ask('textDocument/signatureHelp', at(d, p), t, (r) => {
                const help = new vscode.SignatureHelp();
                help.signatures = (r.signatures || []).map((s) => {
                    const sig = new vscode.SignatureInformation(s.label, markdown(s.documentation));
                    sig.parameters = (s.parameters || []).map((q) => new vscode.ParameterInformation(q.label, markdown(q.documentation)));
                    return sig;
                });
                help.activeSignature = r.activeSignature || 0;
                help.activeParameter = r.activeParameter || 0;
                return help;
            }),
        }, { triggerCharacters: ['(', ','], retriggerCharacters: [','] }),
        vscode.languages.registerDocumentSymbolProvider(SELECTOR, {
            provideDocumentSymbols: (d, t) => ask('textDocument/documentSymbol', { textDocument: { uri: d.uri.toString() } }, t, (r) => r.map(documentSymbol)),
        }),
        vscode.languages.registerWorkspaceSymbolProvider({
            provideWorkspaceSymbols: (query, t) => ask('workspace/symbol', { query }, t, (r) => r.map(documentSymbol)),
        }),
        vscode.languages.registerDocumentFormattingEditProvider(SELECTOR, {
            provideDocumentFormattingEdits: (d, o, t) => ask('textDocument/formatting', { textDocument: { uri: d.uri.toString() }, options: { tabSize: o.tabSize, insertSpaces: o.insertSpaces } }, t, (r) => r.map(textEdit)),
        }),
    );
    void start();
}

async function deactivate() {
    if (client) {
        await client.stop();
    }
}

module.exports = { activate, deactivate };
