# Editor support for Aura

[Русская версия](README.ru.md)

## VS Code

The extension gives syntax highlighting, auto-indentation (`end` dedents by
itself), bracket auto-closing, comment toggling (`Ctrl+/`) **and live diagnostics
through the language server** (`aura-lsp`): `E0xxx` errors and `W0xxx` warnings
are shown in the editor as you type, without running `aura eval`.

The server also gives completion, hover, signature help, a document outline,
formatting (`aura fmt`), go-to-definition, find references and rename (F2).
Hovering an `import "host:..."` line explains that the module comes from the
application that embeds the script, and lists what the file calls from it.

Completion inside `new` offers **exactly the members of the enum** for a field
of enum type - also when the schema is imported from another file
(`new lib.Endpoint`): the declarations are read from the module's file.

Go-to-definition works on imports too: a file import opens the file, and a
registry import (`org/pkg@v1.2`) opens the cached package. The version is chosen
by the same resolver as the interpreter's, so `@v1.2` picks exactly the file
that will really be loaded. The cache directory is set by `aura.registryDir`
(default `~/.aura/registry`).

### Rename

Go-to-definition, find references and rename **respect scopes**: `x` inside a
lambda and `x` at the top level are different bindings, and uses inside `#{...}`
are found as well.

Rename is deliberately careful - editing a config changes what goes to
production - so instead of guessing, the server refuses and says why:

| Refusal | Why |
|---|---|
| the file has a syntax error | the scopes are unknown |
| the cursor is not on a binding | a method name, a schema key, a literal - there is nothing to rename |
| the new name is a keyword or not an identifier | the result would not compile |
| the new name is already taken in an overlapping scope | the rename would change what some occurrence refers to |
| the name is declared `pub` | it is the module's API, and its importers are not visible from this file |

The capture check is conservative on purpose: it refuses even when no specific
occurrence would actually be captured. A refusal costs one manual edit - a wrong
rename costs an incident.

### The quick way (highlighting only)

```console
cp -r editors/vscode ~/.vscode/extensions/aura-lang
```

### With the language server (diagnostics as you type)

1. Install the server, which puts the `aura-lsp` binary on `PATH`:

   ```console
   cargo install aura-lsp
   ```

   Or set its path in the settings: `"aura.server.path": "/path/to/aura-lsp"`.

2. Build the `.vsix` and install it (needs Node.js and `@vscode/vsce`):

   ```console
   cd editors/vscode
   npm install
   npx @vscode/vsce package        # -> aura-lang-0.1.0.vsix
   code --install-extension aura-lang-0.1.0.vsix
   ```

The client (`extension.js`) starts `aura-lsp` over stdio for `.aura` files.
To turn the server off: `"aura.server.enable": false`. Publishing to the
Marketplace comes after the repository is opened.

## Vim / Neovim

```console
cp -r editors/vim/* ~/.vim/           # Vim
cp -r editors/vim/* ~/.config/nvim/   # Neovim
```

## nano

Add to `~/.nanorc`:

```text
include "/path/to/aura-lang/editors/nano/aura.nanorc"
```

## Plans

- A tree-sitter grammar (Helix, Zed, modern Neovim, highlighting on GitHub) -
  see "Status" in the root README; the TextMate grammar in `vscode/syntaxes/` is
  reused for the mdBook documentation and GitHub Linguist.
