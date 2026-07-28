# Aura examples

Each directory is a self-contained example with its expected output (`expected.*`).
Every command is run **from the example's own directory**; build the binary with
`cargo build -p aura-lang`.

| Example | What it shows | Command |
| --- | --- | --- |
| [environments/](environments/) | Several environments from one function: dev/staging/prod with no copy-paste (functions, the ternary, interpolation) | `aura eval environments.aura` |
| [k8s_deploy/](k8s_deploy/) | A Kubernetes Deployment: the schema catches a type typo before `kubectl apply`; string keys like `"app.kubernetes.io/name"`; YAML on the way out | `aura eval k8s_deploy.aura --format yaml` |
| [ci_matrix/](ci_matrix/) | Generating a CI matrix with `map` instead of listing combinations by hand | `aura eval ci_matrix.aura` |
| [feature_flags/](feature_flags/) | `assert` as a production guard: a dangerous flag combination stops the deploy (E0530) | `aura eval feature_flags.aura --allow-env=APP_ENV` |
| [service_catalog/](service_catalog/) | Data from the project's existing files: `parse_toml` / `parse_json`, indexing `teams[0]`, `.get` with a fallback | `aura eval service_catalog.aura --allow-read=.` |
| [security_demo/](security_demo/) | The capability model: an imported module tries to read `/etc/passwd` and gets **E0310** — the root's rights do not reach imports | `aura eval main.aura --allow-read=.` *(fails on purpose)* |
| [i18n/](i18n/) | Assembling localisations: translators work with flat JSON, Aura validates (orphan keys via `keys` / `contains` / `filter`) and merges with a fallback to the base locale | `aura eval i18n.aura --allow-read=.` |
| [validators/](validators/) | A D12 package: `pub def` / `pub type` as the API (`v.service(...)`, `new v.Service`) with private helpers invisible; deterministic time through `parse_duration` / `format_duration`, and date arithmetic through `parse_datetime` | `aura eval deploy.aura` |
| [telegram_bot/](telegram_bot/) | A Telegram bot config: secrets kept out of the config (`token_env_var`), commands with a schema, dev/prod switching of mode and limits, duplicate admins caught by `assert`, localisation with string keys | `aura eval bot.aura --allow-env=BOT_ENV` |
| [nginx/](nginx/) | **Generating a non-JSON format**: block strings (D16) plus `map` / `join` produce a finished `nginx.conf` as a string value. nginx's nested `{}` and `;` are simply text. To get the file: `aura eval nginx.aura --format yaml` or `\| jq -r .nginx_conf` | `aura eval nginx.aura` |
| [showcase/](showcase/) | **A tour of the whole language in one manifest**: a module import (D12), private `=`, `shadow`, schemas with optional fields (D15), `cond` (D14), `range`, the ternary, block strings `text … end` (D16), lambdas with `map` / `filter`, string, list and numeric methods, deterministic time, `domain`, `assert`, and reading a file. The case that exercises the core end to end | `aura eval showcase.aura --allow-read=. --allow-env=APP_ENV` |

The one at the root, [production_deploy.aura](production_deploy.aura), is the
reference manifest from [SPEC.md](../SPEC.md): it demonstrates every construct at
once and is required to pass the full pipeline in CI.

```console
aura eval production_deploy.aura --allow-read=. --allow-env=APP_ENV --registry-dir=registry
```
