# Built-in methods

Methods are called with a dot and they chain. Every signature below was checked
against the implementation rather than transcribed.

## String

| Method | Result | Notes |
| --- | --- | --- |
| `upper()` / `lower()` | String | `"auth".upper()` → `"AUTH"` |
| `len()` | Int | counts characters: `"héllo".len()` → `5` |
| `trim()` | String | strips surrounding whitespace |
| `split(sep)` | List | `"a,b".split(",")` → `["a", "b"]` |
| `replace(from, to)` | String | every occurrence |
| `starts_with(prefix)` / `ends_with(suffix)` | Bool | |
| `contains(substr)` | Bool | `"hello".contains("ell")` → `true` |
| `to_int()` / `to_float()` | Int / Float | a malformed number is an error, not `null` |
| `to_str()` | String | identity, for use in chains |
| `parse_json()` / `parse_yaml()` / `parse_toml()` | Object | integers become `Int`; a failure is `E0314` |
| `parse_duration()` | Int (seconds) | `"1h30m"` → `5400`; units `d/h/m/s`; `E0319` |
| `parse_datetime()` | Int (epoch UTC) | RFC 3339 or `YYYY-MM-DD`; offsets `±HH:MM`; `E0320` |
| `sha256()` | String | lower-case hex digest |
| `base64()` / `base64_decode()` | String | standard alphabet with padding; invalid input is `E0321` |

## Int

| Method | Result | Notes |
| --- | --- | --- |
| `abs()` | Int | |
| `to_str()` | String | |
| `format_duration()` | String | `5400` → `"1h30m"`; `0` → `"0s"` |
| `format_datetime()` | String | RFC 3339 UTC: `946684800` → `"2000-01-01T00:00:00Z"` |

## Float

| Method | Result |
| --- | --- |
| `abs()` | Float |
| `to_str()` | String |

## Bool

| Method | Result |
| --- | --- |
| `to_str()` | String |

## List

| Method | Result | Notes |
| --- | --- | --- |
| `len()` | Int | |
| `first()` / `last()` | element | an empty list is `E0317` |
| `get(i)` / `get(i, default)` | element | a miss gives `null`, or the default if you supply one |
| `contains(item)` | Bool | structural equality |
| `compact()` | List | drops `null` |
| `uniq()` | List | deduplicates, keeping first occurrences in order |
| `map (x, i) -> ... end` | List | the callback receives the element and its index |
| `filter (x, i) -> Bool end` | List | a non-Bool from the callback is `E0306` |
| `join(sep)` | String | scalars only; containers are `E0307` |
| `sort()` | List | |
| `reverse()` | List | |
| `sum()` | Int | |
| `min()` / `max()` | element | |
| `flatten()` | List | one level: `[[1], [2, 3]]` → `[1, 2, 3]` |
| `slice(start, end)` | List | `end` exclusive: `[1, 2, 3].slice(1, 3)` → `[2, 3]` |
| `to_json()` / `to_yaml()` / `to_toml()` | String | `to_toml` on a list is `E0603` |

## Object

| Method | Result | Notes |
| --- | --- | --- |
| `len()` | Int | |
| `keys()` / `values()` | List | declaration order |
| `contains(key)` | Bool | |
| `get(key, default)` | value | a miss gives the default |
| `merge(other)` | Object | the right side's keys win |
| `to_json()` / `to_yaml()` / `to_toml()` | String | |

## Global functions

| Function | Right required | Behaviour |
| --- | --- | --- |
| `range(n)` | — | `[0, 1, … n-1]` |
| `env(name, default)` | `--allow-env` | an environment variable; absent → the default |
| `read_file(path)` | `--allow-read` | the file's contents as a string |
| `fail(message)` | — | stops with `E0531` and your message |

A method call takes precedence over an object field of the same name that happens
to hold a function. A package's exported functions are called with the same
syntax: `pkg.fn(...)`.
