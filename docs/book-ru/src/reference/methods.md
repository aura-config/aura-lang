# Встроенные методы

## String

| Метод | Результат | Пример |
| --- | --- | --- |
| `upper()` / `lower()` | String | `"auth".upper()` → `"AUTH"` |
| `len()` | Int (символы) | `"héllo".len()` → `5` |
| `contains(sub)` | Bool | `"hello".contains("ell")` → `true` |
| `parse_json()` / `parse_yaml()` / `parse_toml()` | Value | целые → `Int`; ошибка — `E0314` |
| `parse_duration()` | Int (секунды) | `"1h30m"` → `5400`; единицы `d/h/m/s`; `E0319` |
| `parse_datetime()` | Int (epoch UTC) | RFC3339 или `YYYY-MM-DD`; смещения `±HH:MM`; `E0320` |

## Int

| Метод | Результат | Пример |
| --- | --- | --- |
| `format_duration()` | String | `5400` → `"1h30m"`; `0` → `"0s"` |
| `format_datetime()` | String (RFC3339 UTC) | `946684800` → `"2000-01-01T00:00:00Z"` |

## List

| Метод | Результат | Пример |
| --- | --- | --- |
| `len()` | Int | |
| `map (x, i) -> ... end` | List | колбэк: элемент + индекс |
| `filter (x, i) -> Bool end` | List | не-Bool из колбэка — `E0306` |
| `compact()` | List | убирает `null` |
| `uniq()` | List | дедупликация, порядок первых вхождений |
| `first()` / `last()` | элемент | пустой список — `E0317` |
| `get(i, default)` | элемент/default | промах — default (или `null`) |
| `contains(x)` | Bool | структурное равенство |
| `join(sep)` | String | только скаляры; `E0307` на контейнерах |
| `to_json()` / `to_yaml()` / `to_toml()` | String | `to_toml` на списке — `E0603` |

## Object

| Метод | Результат | Пример |
| --- | --- | --- |
| `len()` | Int | |
| `merge(other)` | Object | правый перекрывает ключи левого |
| `keys()` / `values()` | List | порядок объявления |
| `contains(key)` | Bool | |
| `get(key, default)` | значение/default | |
| `to_json()` / `to_yaml()` / `to_toml()` | String | |

## Глобальные функции

| Функция | Права | Поведение |
| --- | --- | --- |
| `env(name, default)` | `--allow-env` | переменная окружения; отсутствует → default |
| `read_file(path)` | `--allow-read` | содержимое файла строкой |
| `fail(msg)` | — | остановка с `E0531` и вашим сообщением |

Вызов метода имеет приоритет над одноимённым полем-функцией объекта;
экспортированные функции пакетов вызываются тем же синтаксисом: `pkg.fn(...)`.
