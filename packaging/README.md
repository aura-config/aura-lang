# Packaging

Artifacts that are built here but published elsewhere.

## `setup-aura/`

A composite GitHub Action that installs the `aura` CLI from this repository's
Releases and puts it on `PATH`:

```yaml
- uses: aura-config/setup-aura@v1
  with:
    version: "0.1.0" # or "latest"
- run: aura check deploy.aura --strict
```

It resolves the version, maps the runner to a Rust target triple, downloads the
matching asset, **verifies its `.sha256`** before unpacking, and exports the
directory to `GITHUB_PATH`.

**Why it lives here for now.** Marketplace requires `action.yml` in a
repository *root*, and an action wants its own `@v1` tag moving independently of
the language's releases. So at repository-opening time this directory is
extracted into a separate public repo `aura-config/setup-aura`, tagged `v1` and
published. Until then it is developed here and exercised by `release.yml`'s
`self-test` job via `uses: ./packaging/setup-aura`, which covers everything
except anonymous download and the Marketplace listing.

The binaries themselves stay in `aura-config/aura-lang` Releases — the action
only fetches them (the same split as `setup-node` and nodejs.org).
