# Freebird Language Support

VSCode extension for the Freebird programming language.

## Features

- **Syntax highlighting** for `.fb` files
- Keywords: `fn`, `let`, `mut`, `if`, `else`, `match`, `loop`, `while`, `for`, `return`, `struct`, `enum`, `extern`, `use`, `unsafe`
- Types: `i32`, `i64`, `f64`, `bool`, `char`, `String`, `Vec`, `Option`, `Result`
- String, character, and number literals
- Escape sequences (`\n`, `\t`, `\u{...}`)
- Line comments (`//`) and block comments (`/* */`)
- `stardust!` macro highlighting
- Attribute highlighting (`#[...]`)
- Bracket matching and auto-closing pairs

## Installation

1. Copy the `vscode/` folder to `~/.vscode/extensions/freebird-language/`
2. Restart VSCode
3. Open any `.fb` file

## Development

```bash
# Symlink for development
ln -s $(pwd)/vscode ~/.vscode/extensions/freebird-language
```

## Freebird Language

Freebird is a stack-based systems programming language with a dual-stack VM.
See the [Freebird Language Specification](../../readme.md) for details.
