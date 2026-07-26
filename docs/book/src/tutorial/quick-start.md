# Быстрый старт

## Установка

```console
git clone https://github.com/aura-config/aura-lang && cd aura-lang
cargo build --release
# бинарник: target/release/aura
```

## Первый манифест

Создайте `hello.aura`:

```ruby
app_name = "hello"
port     = 8080

service:
  name:     app_name
  url:      "http://localhost:#{port}"
  replicas: 2
end
```

```console
$ aura eval hello.aura
{
  "service": {
    "name": "hello",
    "url": "http://localhost:8080",
    "replicas": 2
  }
}
```

Обратите внимание: `app_name` и `port` (объявленные через `=`) в JSON **не попали** —
это приватные вычисления. В вывод идут только свойства (`имя:`) и блоки.
Подробнее — в следующей главе.

## Три команды, которые нужны каждый день

```console
aura eval app.aura            # вычислить → JSON в stdout
aura check app.aura --strict  # проверить без вычисления (линтер; для CI)
aura fmt app.aura             # канонизировать отступы
```

## Структура без боли

Aura не использует значимые отступы (травмы YAML лечатся здесь). Структуру
задают переносы строк и явное `end`:

```ruby
server:
  http:
    port: 8080
    timeouts:
      read: "30s".parse_duration()
    end
  end
end
```

Отступы — только для людей; `aura fmt` приводит их к канону автоматически
и гарантированно не меняет смысл (поток токенов до/после идентичен).
