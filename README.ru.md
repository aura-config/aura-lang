# Aura

**Конфигурационный язык, который читает свои входные данные — и не даёт делать то же
самое ничему, что импортирует.**

Aura компилирует читаемые манифесты в JSON, YAML или TOML — со схемами, enum,
проверками `assert`, которые роняют сборку, а не деплой, и capability-моделью, в
которой `env()` и `read_file()` выдаются на запуск, а выданное право не наследуется
импортированными модулями. Один статический бинарник, 16 ключевых слов, 1.7 МБ на скачивание.

<!-- Захардкоженный счётчик тестов устаревает молча: здесь стояло 117, когда в
     наборе было уже 211. Заменить на настоящий бейдж статуса CI после открытия
     репозитория, чтобы он обновлялся сам. -->

[![CI](https://github.com/aura-config/aura-lang/actions/workflows/ci.yml/badge.svg)](https://github.com/aura-config/aura-lang/actions/workflows/ci.yml)
![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
![Status](https://img.shields.io/badge/status-v0.1%20preview-blue)

---

## Почему Aura

| Проблема                                               | Решение Aura                                                                                            |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| YAML ломается от одного лишнего пробела                | Нет значимых отступов: структура задаётся переносами строк и явным `end`                                |
| Конфиги «зависят от машины»                            | Детерминизм по построению: без явных флагов у манифеста **нет** доступа к файлам и переменным окружения |
| Скопированный из интернета модуль читает `/etc/passwd` | Импортированные модули изолированы от I/O (Deno-style capabilities)                                     |
| «Почему в проде другой порт?»                          | Значения иммутабельны; затенение — только явное `shadow`                                                |
| Дрейф версий зависимостей в CI                         | Версионируемые импорты + `aura.lock` с хешем потока токенов и режимом `--frozen`                         |
| Мёртвые куски конфига живут годами                     | Статический анализ: неиспользуемые переменные и импорты, `--strict` для CI                              |

## Пример

```ruby
import "templates/k8s_defaults.aura" as defaults

base_port = 8000 # приватное вычисление
is_prod   = env("APP_ENV", "production") == "production"

type ServiceMeta
  name: String
  port: Int
end

def build_labels(app_name, tier)
  name:       app_name
  tier:       tier
  managed_by: "aura-engine"
end

domain "production-eu"
  replicas: is_prod ? 3 : 1 # свойство — попадает в JSON

  cargo_data  = read_file("./Cargo.toml").parse_toml()
  app_version = cargo_data.package.version

  services = ["auth", "billing", "frontend", null]
  active   = services.compact().uniq()

  meta: new ServiceMeta
    name: "auth".upper()
    port: base_port + 1
  end

  apps: active.map (name, index) ->
    name:   name
    image:  "company/#{name}:#{app_version}"
    labels: build_labels(name, "backend").merge(defaults.global_labels)
  end

  assert active.len() >= 1, "Domain must have at least 1 service"
end
```

```console
aura eval production_deploy.aura --allow-read=. --allow-env=APP_ENV
```

```json
{
  "production-eu": {
    "replicas": 3,
    "meta": { "name": "AUTH", "port": 8001 },
    "apps": [
      {
        "name": "auth",
        "image": "company/auth:1.2.3",
        "labels": { "name": "auth", "tier": "backend", "managed_by": "aura-engine", "team": "core" }
      }
    ]
  }
}
```

## Быстрый старт

```console
git clone https://github.com/aura-config/aura-lang && cd aura-lang
cargo build --release
cd examples
../target/release/aura eval production_deploy.aura \
    --allow-read=. --allow-env=APP_ENV --registry-dir=registry
```

Проверка без вычисления (lex + parse + статический анализ):

```console
aura check production_deploy.aura --strict
```

## Тур по языку

### Вычисления и вывод — разные вещи

Ключевое правило Aura (аналог locals vs outputs в Terraform):

```ruby
tmp = base * 2 #  =  приватная переменная: НЕ попадает в JSON
port: tmp + 1  #  :  свойство: попадает в JSON
```

Благодаря этому анализ мёртвого кода точен: неиспользуемая `=`‑переменная — это
всегда настоящий мусор (`W0501`), а не «может быть, кому-то нужен этот вывод».

### Иммутабельность и явное затенение

```ruby
path = "/etc/global.config"

domain "prod"
  path        = "/var/log" # E0302: затенение требует ключевого слова
  shadow path = "/var/log" # ок — намерение явное
end
```

Повторное присваивание в одной области — всегда ошибка (`E0301`).

### Схемы

```ruby
type ServiceMeta
  name: String
  port: Int
end

meta: new ServiceMeta
  name: "auth"
  port: "8001" # E0512: ожидался Int
end
```

Отсутствующее поле — `E0511`; лишнее поле — `E0513` в `--strict`.
`Int` и `Float` — раздельные типы: байтовые лимиты и 64-битные ID не теряют точность.

Поле с `= default` опционально — пропустите его, и применится дефолт (он может
ссылаться на переменные модуля). Nullable-полей нет: опциональность не вводит `null`.

```ruby
type Service
  name: String
  port: Int    = 8080 # опционально: пропущено → 8080
  tier: String = "backend"
end

api: new Service
  name: "api" # port и tier берут дефолты
end
```

### Закрытые наборы через `enum`

Поле `String` принимает любую строку, поэтому опечатка `"backand"` доезжала до
прода. `enum` делает набор закрытым — при этом член остаётся обычной строкой, и
JSON не меняется:

```ruby
enum Tier
  "frontend"
  "backend"
  "cache"
end

type Service
  name: String
  tier: Tier
end

svc: new Service
  name: "api"
  tier: "backand" # E0514: did you mean "backend"? members: ...
end
```

`pub enum` пересекает границу модуля (D12), а члены резолвятся там, где объявлена
схема — импортированная схема валидируется против enum своего модуля.

### Типы для вашего сервиса: `aura types`

Та же схема, что валидирует конфиг, может типизировать сервис, который потребляет
его JSON — руками поддерживать структуры не нужно:

```console
aura types config.aura --lang rust   # или ts | go
```

```ruby
enum Scheme
  "https"
  "http"
end

type Endpoint
  host: String
  port: Int      = 443
  scheme: Scheme = "https"
end
```

превращается для TypeScript в:

```ts
export type Scheme = "https" | "http";

export interface Endpoint {
  host: string;
  port: number;
  scheme: Scheme;
}
```

Для Rust — `serde`-структура и enum с `#[serde(rename)]`; для Go — структура с
`json:`-тегами плюс тип-строка и типизированные константы. Только парсинг —
манифест не вычисляется, capability не задействованы.

### Функции, лямбды, методы

```ruby
def labels(app) # def возвращает объект
  app: app
end

up = (s) -> s.upper() end # лямбда-выражение

xs.compact().uniq().map (item, index) ->
  "#{index}: #{item}"
end
```

Встроенные методы: `upper` `lower` `len` `trim` `split` `replace` `starts_with`
`ends_with` `to_int` `to_float` `to_str` `compact` `uniq` `map` `filter` `sort`
`reverse` `sum` `min` `max` `flatten` `slice` `first` `last` `get` `contains`
`join` `merge` `keys` `values` `abs` `parse_toml` `parse_json` `parse_yaml`
`to_json` `to_yaml` `to_toml` `parse_duration` `format_duration` `parse_datetime`
`format_datetime` `sha256` `base64` `base64_decode`. Реестр расширяем без
изменений парсера.

Глобальная `range(n)` порождает `[0, 1, ..., n-1]` — удобно генерировать N штук
(шарды, реплики), а не перечислять руками:

```ruby
shards: range(3).map (i, _) ->
  name:       "shard-#{i}"
  replica_of: "primary"
end
```

### Многоветвевой выбор через `cond`

Для 3+ веток, где вложенные тернарники нечитаемы, `cond` берёт первую истинную
ветку; `else` обязателен (его отсутствие — ошибка парсинга):

```ruby
tier = cond
  region == "eu" -> "frankfurt"
  region == "us" -> "virginia"
  else -> "singapore"
end
```

Слева от каждого `->` — `Bool`, справа — любое выражение. Без деструктуризации
по образцу — намеренно проще, чем `match`.

### Многострочные значения через `text … end`

Блок `text … end` — это обычная строка, просто многострочная, поэтому любое
свойство принимает и `"однострочник"`, и блок. Интерполяция `#{}` и эскейпы
работают как обычно; общий отступ срезается, строки соединяются через `\n`:

```ruby
domain "worker"
  entrypoint: text
    #!/bin/sh
    echo "starting #{app_name}"
    exec ./server --port #{port}
  end
end
```

Закрывающий `end` стоит на отступе `entrypoint:`; контент отступлен глубже,
поэтому вложенный `end` (блок shell/Ruby) — это просто текст. Для многих или
больших скриптов лучше `read_file(...)`, чем инлайн-блок.

### Время — только детерминированное

`now()` в Aura не существует и не появится (D13) — невоспроизводимый конфиг
невозможно написать по построению. Зато длительности и даты — первоклассные:

```ruby
ttl = "1h30m".parse_duration()             # → 5400 (секунды)
refresh:    (ttl / 3).format_duration()    # → "30m"
window_end: ("2026-07-18T22:00:00Z".parse_datetime()
+ "4h".parse_duration()).format_datetime() # → "2026-07-19T02:00:00Z"
```

Время сборки, если оно нужно, передаёт хост: `env("BUILD_TIME", ...)` под `--allow-env`.

### Доступ к данным

Точка — для полей, скобки — только для индексов списков; один оператор на одну операцию:

```ruby
loaded = read_file("./data.json").parse_json()

version:  loaded.package.version          # обычные ключи
port:     loaded.servers."eu west".port   # ключ с пробелом/точкой — строка
dynamic:  loaded.envs."#{region}".url     # динамический ключ
first:    loaded.apps[0].name             # индекс списка (за границами — E0317)
optional: loaded.get("maybe", "fallback") # безопасный доступ без ошибки
```

Опечатка в ключе — ошибка `E0308` с позицией, а не молчаливый `null`.

### Aura как конвертер форматов

Поскольку язык читает TOML/JSON/YAML и пишет во все три, конвертация — однострочник:

```ruby
# convert.aura
config: read_file("./legacy.toml").parse_toml()
```

```console
aura eval convert.aura --allow-read=. --format yaml   # TOML → YAML
```

В отличие от `yq`/`jq`, по пути можно валидировать схемой, мержить несколько
источников и добавлять `assert`-проверки — конвертация с гарантиями.

### Модули

```ruby
import github/actions/rust-cache@v1.2 as rust # версия обязательна
import "templates/k8s_defaults.aura" as defaults
```

- Циклические импорты обнаруживаются с полной цепочкой: `E0401: cyclic import: a.aura -> b.aura -> a.aura`.
- Каждый модуль загружается, парсится и вычисляется ровно один раз.
- Точные версии и sha256-хэши фиксируются в `aura.lock`; в CI используйте `--frozen`.

### Валидация

```ruby
assert active.len() >= 1, "Domain must have at least 1 service"
value: broken ? fail("unreachable config") : 42
```

## Модель безопасности

По умолчанию манифест **не может ничего**: ни читать файлы, ни переменные окружения.
Права выдаются флагами CLI и не распространяются на импортированные модули.

| Флаг                 | Что разрешает                                                                         |
| -------------------- | ------------------------------------------------------------------------------------- |
| `--allow-read=<dir>` | `read_file()` внутри каталога (можно повторять; пути канонизируются, `..` не сбегает) |
| `--allow-env[=A,B]`  | `env()` для перечисленных переменных (без списка — для всех)                          |
| `--allow-imports-io` | выдать импортированным модулям права корня                                            |

Вызов без прав — ошибка `E0310` с подсказкой, какой флаг добавить. Эффектный вызов
в импортированном модуле дополнительно ловится статически (`W0512`).

## CLI

```text
aura eval <file.aura>  [--strict] [--dry-run] [--frozen]
                       [--allow-read=<dir>] [--allow-env[=A,B]] [--allow-imports-io]
                       [--format json|json-flat|yaml|toml] [-o out.json] [--registry-dir=<dir>]
aura check <file.aura> [--strict]
aura fmt <files...> [--check]
aura add <path>@vX.Y.Z [--from <file>] [--registry-dir=<dir>]
```

`aura add` — единственное место, где Aura ходит в сеть: пакет скачивается
(конвенция `github/<owner>/<repo>` → `package.aura` тега `vX.Y.Z`), валидируется,
кладётся в локальный кэш и фиксируется в `aura.lock` с sha256. **`eval` работает
оффлайн всегда** — результат не зависит от сети по построению.

| Режим                    | Поведение                                                                                                |
| ------------------------ | -------------------------------------------------------------------------------------------------------- |
| `--strict`               | предупреждения анализа становятся ошибками; лишние поля схем запрещены                                   |
| `--dry-run`              | полное вычисление, но ни JSON, ни `aura.lock` не записываются — только отчёт `[dry-run] would write ...` |
| `--frozen`               | резолв зависимостей строго по `aura.lock` (расхождение — ошибка), лок не переписывается                  |
| `--format json-flat`     | плоский вывод: `production-eu.metrics.port = 9090`                                                       |
| `--format yaml` / `toml` | тот же результат в YAML или TOML (TOML требует объект на верхнем уровне)                                 |

Exit-коды: `0` — успех, `1` — диагностики, `2` — ошибки I/O и аргументов.

## Использование из других языков

Aura следует паттерну terraform/jq/pandoc: стабильный CLI-контракт — это API.
JSON в stdout, диагностики со стабильными кодами (`E0xxx`) в stderr,
exit-коды `0`/`1`/`2`. Из любого языка:

```python
# Python
import json, subprocess
r = subprocess.run(["aura", "eval", "app.aura", "--frozen"], capture_output=True, text=True)
if r.returncode != 0:
    raise RuntimeError(r.stderr)
config = json.loads(r.stdout)
```

```javascript
// Node.js
const { execFileSync } = require("node:child_process");
const config = JSON.parse(execFileSync("aura", ["eval", "app.aura", "--frozen"]));
```

```go
// Go
out, err := exec.Command("aura", "eval", "app.aura", "--frozen").Output()
if err != nil { log.Fatal(err) }
var config map[string]any
json.Unmarshal(out, &config)
```

Рекомендации для прода: `--frozen` (лок-файл обязателен), права только явные,
`--format yaml|toml` — если потребителю удобнее другой формат. Для Rust-проектов
есть прямое встраивание без сабпроцесса — `aura_lang::facade::eval_file()`.
Мобильные приложения — потребители *результата*: сервер/CI вычисляет `.aura`,
клиент читает готовый JSON. Нативные обёртки (WASM/npm, PyO3) — в роадмапе.

## Диагностика

Ошибки указывают файл, строку, колонку, подсвечивают код и предлагают исправление:

```text
[E0302] Error: 'global_file_path' shadows an outer variable
    ╭─[ production_deploy.aura:24:3 ]
 24 │   global_file_path = "/var/log/aura.log"
    │   ─────────┬────────
    │            ╰── add `shadow`
    │
    │   Help: write `shadow global_file_path = ...` to make the shadowing explicit
────╯
```

Каждая ошибка имеет стабильный код (`E0xxx` / `W0xxx`) — удобно для grep в CI-логах.

## Архитектура

```text
исходник ──▶ лексер ──▶ парсер ──▶ статический анализ ──▶ вычисление ──▶ JSON
 &'a str    Vec<Token>    AST        Vec<Diagnostic>        Value
            (zero-copy: токены и AST заимствуют память исходника)
```

```text
crates/
├── aura-lang          # библиотека + CLI `aura` (один публикуемый крейт)
│   ├── lexer/         # zero-copy ДКА, нормализация переносов строк
│   ├── parser/        # рекурсивный спуск + Pratt-выражения
│   ├── analysis/      # dead code, undefined vars, правила shadow
│   ├── eval/          # tree-walking интерпретатор, Environment, реестр методов
│   ├── vfs/           # FileResolver, детекция циклов, aura.lock
│   ├── serialize/     # Value -> serde_json (Int без потери точности)
│   └── main.rs        # бинарник `aura` (clap + ariadne); библиотека остаётся
│                      #   без clap/ariadne и готова к WASM/LSP
└── aura-lsp           # language server (поставляется в VS Code-расширении)
```

Ключевые инварианты:

- **Zero-copy**: ни лексер, ни парсер не копируют строки — только срезы `&'a str`.
- **Детерминизм**: порядок ключей JSON = порядок объявления (`IndexMap`); два прогона дают побайтно одинаковый вывод.
- **Иммутабельность**: контейнеры в `Arc`, клонирование значений — O(1).

## Производительность

Criterion-бенчмарки на эталонном манифесте (`cargo bench -p aura-lang`):

| Этап                                 | Результат           |
| ------------------------------------ | ------------------- |
| Лексер                               | ~200 МБ/с           |
| Лексер + парсер                      | ~120 МБ/с           |
| Полный пайплайн (lex + parse + eval) | ~37 мкс на манифест |

## Разработка

```console
cargo test    # юниты, conformance-suite, property-тесты, golden-снапшоты
cargo bench   # бенчмарки лексера, парсера, резолвера, полного пайплайна
```

Coverage-guided фаззинг (лексер, парсер, полный пайплайн) — в
[fuzz/](fuzz/README.md): nightly + `cargo-fuzz`, гоняется в non-blocking CI-джобе.
Рекурсивный спуск защищён от DoS: глубоко-вложенный вход даёт `E0208`, а не
переполнение стека.

Полная спецификация языка и архитектуры — [SPEC.ru.md](SPEC.ru.md). Эталонный манифест,
который обязан проходить весь пайплайн, — [examples/production_deploy.aura](examples/production_deploy.aura).
Девять тематических примеров (k8s, CI-матрица, feature-флаги, каталог сервисов,
i18n, телеграм-бот, пакет валидаторов, демонстрация capability-модели) — в
[examples/](examples/README.md).

## Статус и дорожная карта

Aura находится в стадии рабочего превью (v0.1): все шесть фаз спецификации реализованы.

- [x] Zero-copy лексер и Pratt-парсер
- [x] Рантайм с capability-моделью и схемами
- [x] Модули, детекция циклов, `aura.lock`
- [x] Статический анализ и `--strict` / `--dry-run`
- [x] JSON-экспорт и CLI с диагностикой ariadne
- [x] Чтение и запись JSON/YAML/TOML (`parse_*` / `to_*`, `--format yaml|toml`)
- [x] Индексация и доступ к произвольным ключам (`xs[0]`, `obj."eu west"`, `.get`)
- [x] `aura fmt` — канонизация отступов с гарантией неизменности потока токенов
- [x] Детерминированное время: `parse_duration`/`format_duration`,
      `parse_datetime`/`format_datetime`; `now()` запрещён по построению (D13)
- [x] Расширение стандартной библиотеки методов (`sort`, `split`, `trim`, `join`,
      `slice`, `flatten`, `reverse`, …) — [справочник методов](docs/book-ru/src/reference/methods.md)
- [x] LSP-сервер: автодополнение, hover, переход к определению, поиск ссылок,
      символы документа, переименование (с `prepare`), подсказка сигнатуры и
      форматирование при сохранении — [crates/aura-lsp/](crates/aura-lsp/)

**Экосистема и дистрибуция**:

- [x] Релизный конвейер: тег `v*` собирает бинарники под шесть целей (Linux
      gnu/musl, aarch64, macOS Intel и Apple silicon, Windows), к каждому
      `.sha256`, после сверки тега с `Cargo.toml` —
      [release.yml](.github/workflows/release.yml)
- [x] GitHub Action `setup-aura`: определяет версию, проверяет контрольную сумму
      и кладёт `aura` в `PATH` — [packaging/setup-aura/](packaging/README.md).
      Джоба `self-test` в релизном workflow ставит бинарник из настоящего релиза
      и запускает его на Linux, macOS и Windows
- [ ] Публикация: crates.io и вынос `setup-aura` в отдельный репозиторий для
      Marketplace — оба шага выполняются в момент открытия репозитория
- [x] Подсветка синтаксиса: VS Code (TextMate + автоотступы), Vim/Neovim,
      nano — [editors/](editors/README.md)
- [ ] tree-sitter-грамматика (Helix, Zed, Neovim, GitHub Linguist)
- [x] Книга документации ([docs/book-ru/](docs/book-ru/), основная версия —
      [английская](docs/book/)): учебник (6 глав),
      руководства (безопасность, форматы, встраивание), справочник CLI/методов
      и полный каталог кодов ошибок; сборка в CI, деплой на Pages — после
      публикации репозитория
- [x] Playground в браузере — [playground/](playground/): настоящий компилятор
      как WebAssembly, несколько файлов, `aura fmt` и диагностика; ничего не
      устанавливается и ничего не покидает страницу. Деплоится на Pages вместе с
      книгой, после публикации репозитория
- [x] Использование из других языков: subprocess-паттерн (Python/Node/Go) —
      см. раздел выше. WebAssembly-модуль, на котором работает playground,
      собран и проверяется в CI ([crates/aura-wasm/](crates/aura-wasm/));
      публикация в npm, затем PyO3 и C ABI — по спросу
- [x] Кросс-платформенность: `cargo check` матрица (freebsd, aarch64-linux,
      musl, wasm32) в CI; macOS в тестовую матрицу

**v1.3 — пакетная экосистема**:

- [x] D12: `pub def` / `pub type` — экспорт функций и схем из модулей
      (`pkg.fn(...)`, `new pkg.Schema ... end`); экспортированные функции
      выполняются с правами своего модуля, а не вызывающего
- [x] `aura add <pkg>@vX.Y.Z` — установка пакета (сеть только здесь; `eval`
      всегда оффлайн), валидация перед установкой, integrity в `aura.lock`

## Участие в разработке

Issues и PR приветствуются. Перед PR: `cargo test --workspace`, `cargo fmt --all`,
`cargo clippy --workspace --all-targets -- -D warnings`, `aura fmt --check` на
изменённых `.aura`-файлах — тот же набор проверок гоняет CI. Комментарии в коде —
на английском (см. [SPEC.ru.md](SPEC.ru.md) для формальной спецификации языка).

## Лицензия

Aura распространяется на выбор по [MIT](LICENSE-MIT) или [Apache License 2.0](LICENSE-APACHE).

Если явно не указано иное, любой вклад (issue/PR), намеренно предложенный для
включения в проект, лицензируется на тех же условиях без дополнительных требований.

---

*[English version: README.md](README.md)*
