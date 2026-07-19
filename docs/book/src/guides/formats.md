# Форматы: TOML, JSON, YAML

## Чтение

Три парсера — методы строки; результат — обычные объекты/списки Aura:

```ruby
cargo = read_file("./Cargo.toml").parse_toml()
team  = read_file("./team.json").parse_json()
lint  = read_file("./.rules.yaml").parse_yaml()

service:
  name: cargo.package.name
  owner: team.teams[0].lead
end
```

Целые числа всех форматов приходят как `Int` (без потери точности),
ошибки парсинга — `E0314` с сообщением исходной библиотеки.

## Запись

Из CLI — флагом:

```console
aura eval app.aura --format json       # по умолчанию, pretty
aura eval app.aura --format json-flat  # плоские ключи: a.b.c = 1
aura eval app.aura --format yaml
aura eval app.aura --format toml       # требует объект на верхнем уровне
```

Изнутри языка — методами (полезно для вложенных конфигов-строк):

```ruby
configmap:
  "app-config.yaml": settings.to_yaml()
end
```

Ограничения TOML честно превращаются в ошибки `E0603`: нет `null`,
на верхнем уровне обязан быть объект.

## Aura как конвертер

Поскольку язык читает и пишет все три формата, миграция — однострочник:

```ruby
# convert.aura
config: read_file("./legacy.toml").parse_toml()
```

```console
aura eval convert.aura --allow-read=. --format yaml
```

В отличие от `yq`/`jq`, по пути доступны схемы, `assert` и `merge`
нескольких источников — конвертация с гарантиями.
