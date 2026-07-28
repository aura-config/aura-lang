# Time without surprises

## now() does not exist

Calling `now()` or `timestamp()` is error `E0533`. This is not an omission but a
design decision (D13): the current time would make a config irreproducible,
breaking `--dry-run` comparisons, caching, and the whole idea that two runs give
the same result.

If you genuinely need the build time, the **host** passes it in:

```ruby
built_at: env("BUILD_TIME", "unknown")
```

```console
BUILD_TIME=$(date -u +%FT%TZ) aura eval app.aura --allow-env=BUILD_TIME
```

The decision about time is made and recorded by whoever runs the tool, and the
config stays a pure function of its inputs.

## Durations

Timeouts and TTLs are strings with `d` / `h` / `m` / `s` units, parsed into
seconds as an `Int`:

```ruby
ttl = "1h30m".parse_duration() # → 5400
cache:
  ttl_seconds:   ttl
  refresh_every: (ttl / 3).format_duration() # → "30m"
end
```

From there it is ordinary integer arithmetic: adding, subtracting, converting a
day into hours. A malformed string is `E0319`.

## Dates

RFC 3339 ↔ epoch seconds, in UTC:

```ruby
start = "2026-07-18T22:00:00Z".parse_datetime()      # → an epoch Int
window:
  start: start
  end: (start + "4h".parse_duration()).format_datetime()
  # → "2026-07-19T02:00:00Z" — crossing midnight works correctly
end
```

Offsets are supported (`+02:00` denotes the same instant), as is the short date
form (`"2026-07-18"` is midnight UTC). A malformed date is `E0320`.

Higher-level calendar arithmetic — "plus three months", working days, time zones —
is deliberately outside the core. That is territory for community packages built
on top of these epoch primitives.
