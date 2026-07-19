# Модули и пакеты

## Импорты

```ruby
import "templates/defaults.aura" as defaults      # файл, относительно текущего
import github/acme/aura-k8s@v1.2 as k8s           # пакет из registry, версия обязательна
```

Циклические импорты обнаруживаются с полной цепочкой:
`E0401: cyclic import: a.aura -> b.aura -> a.aura`.

## Что экспортирует модуль

Модуль отдаёт импортёру объект: свойства, блоки и **pub**-элементы:

```ruby
# validators.aura
pub type Service            # схема — часть API
  name: String
  port: Int
end

def valid_port(p)           # приватный хелпер: импортёру невидим
  ok: p > 0 && p < 65536
end

pub def service(name, port) # функция — часть API
  name: name
  port: valid_port(port).ok ? port : fail("invalid port #{port}")
end
```

```ruby
# использование
import "validators.aura" as v

api: v.service("api", 8080)
worker: new v.Service
  name: "worker"
  port: 9000
end
```

Ключевая гарантия безопасности: **экспортированная функция выполняется
с правами своего модуля, а не вызывающего**. Пакет не может «одолжить» ваши
права на чтение файлов, заставив вас вызвать его функцию.

## Установка пакетов: aura add

```console
aura add github/acme/aura-k8s@v1.2.3        # из сети (точная версия)
aura add pkg/internal@v1.0.0 --from ./pkg.aura   # из локального файла
```

`aura add` — **единственное место, где Aura обращается к сети**: пакет
скачивается, валидируется, кладётся в локальный кэш (`~/.aura/registry`)
и фиксируется в `aura.lock` с sha256-хэшем. `eval` всегда работает оффлайн.

## aura.lock и --frozen

- лок хранит точную версию + integrity каждого пакета;
- подмена содержимого пакета → `E0402 integrity mismatch`;
- в CI запускайте с `--frozen`: резолв строго по локу, отсутствие записи —
  ошибка `E0403`, лок не переписывается.

Диапазоны версий (`@v1`, `@v1.2`) резолвятся по локальному кэшу в максимальную
подходящую (`v1.2` → `1.2.*`).
