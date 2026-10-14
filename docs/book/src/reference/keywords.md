# Keywords

Aura has sixteen reserved words. None of them can name a binding, a function, a
schema or an unquoted key: `type = 1` is a syntax error. A key that has to be one
of these words is written as a string, and read the same way (D11):
`"type": "service"`, then `o."type"`.

| Word | What it does | Example | Decision |
| --- | --- | --- | --- |
| `import` | Brings in another module, a registry package with a pinned version, or a module the host provides | `import "./lib.aura" as lib` | D8, D12, D35 |
| `as` | Names what `import` brought in; only after an import | `import org/net@v1.2 as net` | D12 |
| `type` | Declares a schema: fields with types, defaults and invariants | `type Port` … `end` | D15, D26, D27, D28 |
| `enum` | Declares a closed set of strings for a schema field | `enum Tier` … `end` | D18 |
| `def` | Declares a named function | `def labels(team)` … `end` | D17 |
| `end` | Closes every block: objects, `def`, `type`, `enum`, `domain`, `cond`, `new`, a multi-line lambda and a `text` string | `tls: enabled: true end` | D3, D38 |
| `domain` | A named section with its own scope: its label becomes the key, and its body may compute | `domain "production"` … `end` | D17 |
| `new` | Builds a schema instance and checks it | `new Port number: 443 end` | D4, D38 |
| `assert` | A check that stops evaluation with a message; inside a `type`, an invariant carried with the schema | `assert replicas > 0, "need one"` | D5, D28 |
| `shadow` | Rebinds a name from an outer scope - without it, that is `E0302` | `shadow port = 9090` | D7 |
| `pub` | Makes a `def`, `type` or `enum` importable; properties are always visible | `pub def service(name)` | D12 |
| `cond` | A choice between several values, the first true arm wins | `cond` … `else -> "dev"` … `end` | D14 |
| `else` | The last arm of a `cond`, required so every path has a value | `else -> "dev"` | D14 |
| `true` | The Bool true. There is no truthiness: a condition that is not a Bool is `E0306` | `tls: true` | - |
| `false` | The Bool false | `debug: false` | - |
| `null` | The absence of a value. A schema field takes it only when its type is marked `?`, and it is never rendered as text | `quota: Int? = null` | D27, D31 |

## Not reserved

| Word | What it does | Example | Decision |
| --- | --- | --- | --- |
| `text` | Opens a block string, but only where a value is expected - `key: text` or `x = text` at the end of a line. Everywhere else it is an ordinary name, so a field called `text` keeps working | `body: text` … `end` | D16 |

The type names - `String`, `Int`, `Float`, `Bool`, `List`, `Object` - are not
keywords either: they are names the schema checker knows. The decisions in the
last column are in the decision log of the specification.
