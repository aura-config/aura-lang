# Встраивание и вызов из других языков

## Rust: библиотека

```toml
[dependencies]
aura-lang = { git = "https://github.com/aura-config/aura-lang" }
```

```rust
use aura_lang::facade::{eval_file, EvalOptions};

let opts = EvalOptions {
    strict: true,
    allow_read: vec!["config/".into()],
    ..Default::default()
};
let out = eval_file("config/app.aura".as_ref(), &opts)?;
let cfg: MyConfig = serde_json::from_value(out.json)?;      // сразу в свои структуры
for w in &out.warnings {
    log::warn!("{w}");                                       // Display: error[E..]: ... at file:line:col
}
if let Some(lock) = out.updated_lockfile {
    std::fs::write("config/aura.lock", lock)?;               // писать или нет — решает хост
}
```

Права задаёт хост-приложение — конфиг не получает больше, чем вы разрешили.
Диагностики приходят структурированными (`code`, `severity`, `file`, `line`,
`column`, `help`) — рендерите в свой лог как удобно.

Нюанс многопоточности: `Interpreter` не `Send`; каждый вызов `eval_file`
самодостаточен — в async-контексте заворачивайте вызов целиком
в `spawn_blocking`.

## Любой язык: subprocess

CLI-контракт стабилен и является API: JSON в stdout, диагностики с кодами
`E0xxx`/`W0xxx` в stderr, exit-коды `0` (успех) / `1` (диагностики) /
`2` (I/O, аргументы).

```python
import json, subprocess
r = subprocess.run(["aura", "eval", "app.aura", "--frozen"],
                   capture_output=True, text=True)
if r.returncode != 0:
    raise RuntimeError(r.stderr)
config = json.loads(r.stdout)
```

Рекомендации для прода:

- `--frozen` — зависимости строго по `aura.lock`;
- права минимальные и явные (`--allow-read`, `--allow-env=ИМЕНА`);
- `--format yaml|toml`, если потребителю удобнее.

## Мобильные и браузерные приложения

Правильный паттерн — вычисление на сервере/CI:

```text
configs.aura ──aura eval──▶ config.json ──▶ CDN / бандл
устройство читает готовый JSON штатным парсером платформы
```

Приложения — потребители *результата* Aura; валидация уже прошла в CI.

## Почему обёртки, а не нативные реализации

Для YAML в каждом языке есть свой парсер, и возникает вопрос, почему у Aura не
так. Потому что YAML — формат данных: его порт это парсер, байты → данные.
Aura — вычисляемый язык (функции, импорты, схемы, capability, детерминизм): её
порт это интерпретатор, который обязан **побайтово** совпадать с этим. Иначе
один и тот же манифест даст разные значения в CI и в сервисе — то есть в
проде. Расхождение возникает не на экзотике: наивная реализация на
`encoding/json` в Go выдаёт `1` там, где Aura выдаёт `1.0`, и
`1000000000000000000` там, где `1e+18`.

Поэтому реализация одна, а языки получают её через:

1. **Вычисление на этапе сборки** — основной путь. Рантайм Aura внутри вашего
   языка не нужен вовсе, а `aura types` вдобавок сгенерирует типы под ваш JSON.
2. **Обёртки вокруг того же ядра** — в роадмапе, по приоритету: WASM/npm
   (закрывает Node, браузер и playground одним артефактом), затем `wazero` для
   Go (чистый Go, без cgo) и wasmtime для Python; PyO3/UniFFI — по мере спроса.

Ядро уже собирается под `wasm32-unknown-unknown`; это проверяется в CI на
каждом коммите.
