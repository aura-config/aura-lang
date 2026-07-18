# Подсветка синтаксиса Aura

## VS Code

Скопируйте каталог как расширение и перезапустите редактор:

```console
cp -r editors/vscode ~/.vscode/extensions/aura-lang
```

Помимо подсветки расширение даёт автоотступы (`end` дедентится сам),
автозакрытие скобок и переключение комментариев (`Ctrl+/`).
Публикация в Marketplace — после релиза (см. роадмап).

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
