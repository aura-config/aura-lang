# Поддержка редакторов для Aura

## VS Code

Расширение даёт подсветку синтаксиса, автоотступы (`end` дедентится сам),
автозакрытие скобок, переключение комментариев (`Ctrl+/`) **и живую диагностику
через language server** (`aura-lsp`): ошибки `E0xxx` и предупреждения `W0xxx`
подсвечиваются прямо в редакторе по мере набора, без запуска `aura eval`.

### Быстрый способ (только подсветка)

```console
cp -r editors/vscode ~/.vscode/extensions/aura-lang
```

### С language server (диагностика на лету)

1. Собрать и поставить сервер на PATH:

   ```console
   cargo install --path crates/aura-lsp     # ставит бинарник `aura-lsp`
   ```

   Либо указать путь в настройках: `"aura.server.path": "/путь/к/aura-lsp"`.

2. Собрать `.vsix` и установить (нужны Node.js + `@vscode/vsce`):

   ```console
   cd editors/vscode
   npm install
   npx @vscode/vsce package        # -> aura-lang-0.1.0.vsix
   code --install-extension aura-lang-0.1.0.vsix
   ```

Клиент (`extension.js`) запускает `aura-lsp` по stdio для `.aura`-файлов.
Отключить сервер: `"aura.server.enable": false`. Публикация в Marketplace —
после открытия репозитория (см. роадмап).

## Vim / Neovim

```console
cp -r editors/vim/* ~/.vim/           # Vim
cp -r editors/vim/* ~/.config/nvim/   # Neovim
```

## nano

Добавьте в `~/.nanorc`:

```text
include "/путь/до/aura-lang/editors/nano/aura.nanorc"
```

## Планы

- tree-sitter-грамматика (Helix, Zed, современный Neovim, подсветка на GitHub) —
  см. роадмап в корневом README; TextMate-грамматика из `vscode/syntaxes/`
  переиспользуется для mdBook-документации и GitHub Linguist.
