# Примеры Aura

Каждый каталог — самодостаточный пример с ожидаемым выводом (`expected.*`).
Все команды запускаются **из каталога примера**; бинарник — `cargo build -p aura-cli`.

| Пример | Что демонстрирует | Команда |
| --- | --- | --- |
| [environments/](environments/) | Мульти-окружение одной функцией: dev/staging/prod без копипасты (функции, тернарник, интерполяция) | `aura eval environments.aura` |
| [k8s_deploy/](k8s_deploy/) | Kubernetes Deployment: схема ловит опечатку в типе до `kubectl apply`; строковые ключи `"app.kubernetes.io/name"`; YAML на выходе | `aura eval k8s_deploy.aura --format yaml` |
| [ci_matrix/](ci_matrix/) | Генерация CI-матрицы через `map` + `component` вместо ручного перечисления комбинаций | `aura eval ci_matrix.aura` |
| [feature_flags/](feature_flags/) | `assert` как защита прода: опасная комбинация флагов останавливает деплой (E0530) | `aura eval feature_flags.aura --allow-env=APP_ENV` |
| [service_catalog/](service_catalog/) | Данные из существующих файлов проекта: `parse_toml`/`parse_json`, индексация `teams[0]`, `.get` с fallback | `aura eval service_catalog.aura --allow-read=.` |
| [security_demo/](security_demo/) | Capability-модель: импортированный модуль пытается читать `/etc/passwd` и получает **E0310** — права корня на импорт не распространяются | `aura eval main.aura --allow-read=.` *(ожидаемо падает!)* |

Корневой [production_deploy.aura](production_deploy.aura) — эталонный манифест из [SPEC.md](../SPEC.md):
демонстрирует все конструкции языка сразу и обязан проходить полный пайплайн в CI.

```console
$ aura eval production_deploy.aura --allow-read=. --allow-env=APP_ENV --registry-dir=registry
```
