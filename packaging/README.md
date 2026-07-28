# Packaging

Artifacts that are built here but published elsewhere.

## `setup-aura/` — moved

The composite action that installs the `aura` CLI now lives in its own
repository, **[aura-config/setup-aura](https://github.com/aura-config/setup-aura)**,
and is used as:

```yaml
- uses: aura-config/setup-aura@v1
  with:
    version: "0.1.0" # or "latest"
- run: aura check deploy.aura --strict
```

It was developed here until the repository was opened. It moved because
Marketplace requires `action.yml` in a repository *root*, and because the
installer wants a `@v1` tag that moves independently of the language's releases.

**No copy is kept here.** This repository's release workflow consumes the
published `aura-config/setup-aura@v1` like anyone else would, so there is one
definition and nothing to drift.

The binaries stay in `aura-config/aura-lang` releases — the action only fetches
them, the same split as `setup-node` and nodejs.org.

### Note for release day

`release.yml` creates the release as a **draft**. A draft is visible only to
accounts with write access, so `setup-aura` cannot install from it for anyone
else, and `version: latest` will not resolve at all until the draft is published.
Publishing the release is therefore a required step, not a formality.
