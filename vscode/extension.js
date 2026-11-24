// Freebird VS Code 扩展：语法高亮 + LSP（诊断/hover/跳转/符号/补全）
const vscode = require('vscode');
const { LanguageClient, TransportKind } = require('vscode-languageclient/node');

let client;

function activate(context) {
    const config = vscode.workspace.getConfiguration('freebird');
    const command = config.get('serverPath') || 'freebird';

    /** @type {import('vscode-languageclient/node').ServerOptions} */
    const serverOptions = {
        command,
        args: ['lsp'],
        transport: TransportKind.stdio,
    };

    /** @type {import('vscode-languageclient/node').LanguageClientOptions} */
    const clientOptions = {
        documentSelector: [{ scheme: 'file', language: 'freebird' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.fb'),
        },
    };

    client = new LanguageClient(
        'freebird',
        'Freebird Language Server',
        serverOptions,
        clientOptions
    );
    client.start();
    context.subscriptions.push({
        dispose: () => {
            if (client) {
                client.stop();
                client = undefined;
            }
        },
    });
}

function deactivate() {
    if (!client) {
        return undefined;
    }
    const stopping = client.stop();
    client = undefined;
    return stopping;
}

module.exports = { activate, deactivate };
