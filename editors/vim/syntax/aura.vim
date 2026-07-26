" Подсветка Aura для Vim/Neovim.
" Установка: скопируйте editors/vim/* в ~/.vim/ (или ~/.config/nvim/)

if exists("b:current_syntax")
  finish
endif

syn keyword auraKeyword import as type def end domain new assert shadow pub
syn keyword auraBoolean true false null
syn keyword auraBuiltin env read_file fail

syn match auraNumber "\<\d\+\(\.\d\+\)\?\>"
syn match auraOperator "\(+\|-\|\*\|/\|%\|==\|!=\|<=\|>=\|<\|>\|&&\|||\|!\|?\|->\)"
syn match auraType "\<[A-Z][A-Za-z0-9_]*\>"
syn match auraProperty "^\s*\zs[A-Za-z_][A-Za-z0-9_]*\ze\s*:"

syn region auraInterp matchgroup=auraInterpDelim start="#{" end="}" contained
syn region auraString start=+"+ skip=+\\"+ end=+"+ contains=auraInterp

syn match auraComment "#\(\s.*\|$\)" contains=@Spell
syn match auraComment "^#.*$" contains=@Spell

hi def link auraKeyword     Keyword
hi def link auraBoolean     Boolean
hi def link auraBuiltin     Function
hi def link auraNumber      Number
hi def link auraOperator    Operator
hi def link auraType        Type
hi def link auraProperty    Identifier
hi def link auraString      String
hi def link auraInterp      Special
hi def link auraInterpDelim Delimiter
hi def link auraComment     Comment

let b:current_syntax = "aura"
