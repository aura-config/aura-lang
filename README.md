# Aura

**Безопасный, детерминированный конфигурационный язык корпоративного уровня.**

Aura компилирует читаемые манифесты в JSON — со схемами, модулями, статическим анализом
и capability-моделью доступа к окружению. Никаких фигурных скобок, никаких значимых
отступов, никаких сюрпризов в проде.

![Rust 1.75+](https://img.shields.io/badge/rust-1.75%2B-orange)
![Tests](https://img.shields.io/badge/tests-54%20passing-brightgreen)
![Status](https://img.shields.io/badge/status-v0.1%20preview-blue)

---

## Почему Aura

| Проблема | Решение Aura |
| --- | --- |
| YAML ломается от одного лишнего пробела | Нет значимых отступов: структура задаётся переносами строк и явным `end` |
| Конфиги «зависят от машины» | Детерминизм по построению: без явных флагов у манифеста **нет** доступа к файлам и переменным окружения |
| Скопированный из интернета модуль читает `/etc/passwd` | Импортированные модули изолированы от I/O (Deno-style capabilities) |
| «Почему в проде другой порт?» | Значения иммутабельны; затенение — только явное `shadow` |
| Дрейф версий зависимостей в CI | Версионируемые импорты + `aura.lock` c sha256-integrity и режимом `--frozen` |
| Мёртвые куски конфига живут годами | Статический анализ: неиспользуемые переменные и импорты, `--strict` для CI |

## Пример

```ruby
import "templates/k8s_defaults.aura" as defaults

base_port = 8000                                  # приватное вычисление
is_prod   = env("APP_ENV", "production") == "production"

type ServiceMeta
  name: String
  port: Int
end

def build_labels(app_name, tier)
  name: app_name
  tier: tier
  managed_by: "aura-engine"
end

domain "production-eu"
  replicas: is_prod ? 3 : 1                       # свойство — попадает в JSON

  cargo_data  = read_file("./Cargo.toml").parse_toml()
  app_version = cargo_data.package.version

  services = ["auth", "billing", "frontend", null]
  active   = services.compact().uniq()

  meta: new ServiceMeta
    name: "auth".upper()
    port: base_port + 1
  end

  apps: active.map (name, index) ->
    component name
      image: "company/#{name}:#{app_version}"
      labels: build_labels(name, "backend").merge(defaults.global_labels)
    end
  end

  assert active.len() >= 1, "Domain must have at least 1 service"
end
```

```console
$ aura eval production_deploy.aura --allow-read=. --allow-env=APP_ENV
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
$ git clone https://github.com/<you>/aura-lang && cd aura-lang
$ cargo build --release
$ cd examples
$ ../target/release/aura eval production_deploy.aura \
      --allow-read=. --allow-env=APP_ENV --registry-dir=registry
```

Проверка без вычисления (lex + parse + статический анализ):

```console
$ aura check production_deploy.aura --strict
```

## Тур по языку

### Вычисления и вывод — разные вещи

Ключевое правило Aura (аналог locals vs outputs в Terraform):

```ruby
tmp = base * 2        #  =  приватная переменная: НЕ попадает в JSON
port: tmp + 1         #  :  свойство: попадает в JSON
```

Благодаря этому анализ мёртвого кода точен: неиспользуемая `=`‑переменная — это
всегда настоящий мусор (`W0501`), а не «может быть, кому-то нужен этот вывод».

### Иммутабельность и явное затенение

```ruby
path = "/etc/global.config"

domain "prod"
  path = "/var/log"          # E0302: затенение требует ключевого слова
  shadow path = "/var/log"   # ок — намерение явное
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
  port: "8001"      # E0512: ожидался Int
end
```

Отсутствующее поле — `E0511`; лишнее поле — `E0513` в `--strict`.
`Int` и `Float` — раздельные типы: байтовые лимиты и 64-битные ID не теряют точность.

### Функции, лямбды, методы

```ruby
def labels(app)              # def возвращает объект
  app: app
end

up = (s) -> s.upper() end    # лямбда-выражение

xs.compact().uniq().map (item, index) ->
  "#{index}: #{item}"
end
```

Встроенные методы: `upper` `lower` `len` `compact` `uniq` `map` `filter` `merge`
`first` `last` `get` `keys` `values` `contains` `join` `parse_toml` `parse_json`
`parse_yaml` `to_json` `to_yaml` `to_toml` `parse_duration` `format_duration`
`parse_datetime` `format_datetime`. Реестр расширяем без изменений парсера.

### Время — только детерминированное

`now()` в Aura не существует и не появится (D13) — невоспроизводимый конфиг
невозможно написать по построению. Зато длительности и даты — первоклассные:

```ruby
ttl = "1h30m".parse_duration()                       # → 5400 (секунды)
refresh: (ttl / 3).format_duration()                 # → "30m"
window_end: ("2026-07-18T22:00:00Z".parse_datetime()
  + "4h".parse_duration()).format_datetime()         # → "2026-07-19T02:00:00Z"
```

Время сборки, если оно нужно, передаёт хост: `env("BUILD_TIME", ...)` под `--allow-env`.

### Доступ к данным

Точка — для полей, скобки — только для индексов списков; один оператор на одну операцию:

```ruby
loaded = read_file("./data.json").parse_json()

version:  loaded.package.version           # обычные ключи
port:     loaded.servers."eu west".port    # ключ с пробелом/точкой — строка
dynamic:  loaded.envs."#{region}".url      # динамический ключ
first:    loaded.apps[0].name              # индекс списка (за границами — E0317)
optional: loaded.get("maybe", "fallback")  # безопасный доступ без ошибки
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
import github/actions/rust-cache@v1.2 as rust    # версия обязательна
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

| Флаг | Что разрешает |
| --- | --- |
| `--allow-read=<dir>` | `read_file()` внутри каталога (можно повторять; пути канонизируются, `..` не сбегает) |
| `--allow-env[=A,B]` | `env()` для перечисленных переменных (без списка — для всех) |
| `--allow-imports-io` | выдать импортированным модулям права корня |

Вызов без прав — ошибка `E0310` с подсказкой, какой флаг добавить. Эффектный вызов
в импортированном модуле дополнительно ловится статически (`W0512`).

## CLI

```text
aura eval <file.aura>  [--strict] [--dry-run] [--frozen]
                       [--allow-read=<dir>] [--allow-env[=A,B]] [--allow-imports-io]
                       [--format json|json-flat|yaml|toml] [-o out.json] [--registry-dir=<dir>]
aura check <file.aura> [--strict]
aura fmt <files...> [--check]
```

| Режим | Поведение |
| --- | --- |
| `--strict` | предупреждения анализа становятся ошибками; лишние поля схем запрещены |
| `--dry-run` | полное вычисление, но ни JSON, ни `aura.lock` не записываются — только отчёт `[dry-run] would write ...` |
| `--frozen` | резолв зависимостей строго по `aura.lock` (расхождение — ошибка), лок не переписывается |
| `--format json-flat` | плоский вывод: `production-eu.metrics.port = 9090` |
| `--format yaml` / `toml` | тот же результат в YAML или TOML (TOML требует объект на верхнем уровне) |

Exit-коды: `0` — успех, `1` — диагностики, `2` — ошибки I/O и аргументов.

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
├── aura-core          # библиотека: без зависимостей на CLI/рендеринг (готова к WASM/LSP)
│   ├── lexer/         # zero-copy ДКА, нормализация переносов строк
│   ├── parser/        # рекурсивный спуск + Pratt-выражения
│   ├── analysis/      # dead code, undefined vars, правила shadow
│   ├── eval/          # tree-walking интерпретатор, Environment, реестр методов
│   ├── vfs/           # FileResolver, детекция циклов, aura.lock
│   └── serialize/     # Value -> serde_json (Int без потери точности)
└── aura-cli           # clap + ariadne
```

Ключевые инварианты:

- **Zero-copy**: ни лексер, ни парсер не копируют строки — только срезы `&'a str`.
- **Детерминизм**: порядок ключей JSON = порядок объявления (`IndexMap`); два прогона дают побайтно одинаковый вывод.
- **Иммутабельность**: контейнеры в `Arc`, клонирование значений — O(1).

## Производительность

Criterion-бенчмарки на эталонном манифесте (`cargo bench -p aura-core`):

| Этап | Результат |
| --- | --- |
| Лексер | ~200 МБ/с |
| Лексер + парсер | ~120 МБ/с |
| Полный пайплайн (lex + parse + eval) | ~37 мкс на манифест |

## Разработка

```console
$ cargo test          # 54 теста: юниты + интеграционные по эталонному манифесту
$ cargo bench         # бенчмарки лексера, парсера, полного пайплайна
```

Полная спецификация языка и архитектуры — [SPEC.md](SPEC.md). Эталонный манифест,
который обязан проходить весь пайплайн, — [examples/production_deploy.aura](examples/production_deploy.aura).
Шесть тематических примеров (k8s, CI-матрица, feature-флаги, каталог сервисов,
демонстрация capability-модели) — в [examples/](examples/README.md).

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
- [ ] Расширение стандартной библиотеки методов (`sort`, `split`, `trim`, …)
- [ ] LSP-сервер

**v1.3 — пакетная экосистема**:

- [x] D12: `pub def` / `pub type` — экспорт функций и схем из модулей
      (`pkg.fn(...)`, `new pkg.Schema ... end`); экспортированные функции
      выполняются с правами своего модуля, а не вызывающего
- [ ] Сетевые registry-импорты (`HttpResolver`) поверх готовых `aura.lock`/integrity
- [ ] `aura add <pkg>@<ver>` — установка пакета в кэш с записью лока
