# Функции и методы

## def — функции-конструкторы

`def` возвращает объект; тело — свойства:

```ruby
def make_env(name, replicas, debug)
  replicas: replicas
  log_level: debug ? "debug" : "warn"
  db_url: "postgres://db.#{name}.internal:5432/app"
end

environments:
  dev:  make_env("dev", 1, true)
  prod: make_env("prod", 6, false)
end
```

Один `def` вместо трёх скопированных YAML-файлов — главный анти-копипаст
инструмент языка.

## Лямбды

```ruby
double = (x) -> x * 2 end
up: ["a", "b"].map (s, i) -> s.upper() end     # trailing-лямбда
```

Колбэк `map`/`filter` получает элемент и индекс; лишние параметры можно
не объявлять.

## Методы

Вызываются через точку, образуют цепочки:

```ruby
active: services.compact().uniq().map (s, i) -> s.upper() end
```

Полный список — в [справочнике методов](../reference/methods.md).
Часто используемые:

- **списки**: `map` `filter` `compact` (убрать `null`) `uniq` `first` `last`
  `join(sep)` `contains(x)` `get(i, default)`
- **объекты**: `merge` (правый перекрывает) `keys` `values` `contains(key)`
  `get(key, default)`
- **строки**: `upper` `lower` `len` `contains(sub)` + парсеры форматов

## Тернарный оператор

Единственная ветвящаяся конструкция языка:

```ruby
mode: is_prod ? "webhook" : "long_polling"
```

Условие обязано быть `Bool` — «правдивости» (truthiness) в Aura нет,
`E0306` на любом другом типе.
