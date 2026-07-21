# Примеры Aura

Каждый каталог — самодостаточный пример с ожидаемым выводом (`expected.*`).
Все команды запускаются **из каталога примера**; бинарник — `cargo build -p aura-lang`.

| Пример | Что демонстрирует | Команда |
| --- | --- | --- |
| [environments/](environments/) | Мульти-окружение одной функцией: dev/staging/prod без копипасты (функции, тернарник, интерполяция) | `aura eval environments.aura` |
| [k8s_deploy/](k8s_deploy/) | Kubernetes Deployment: схема ловит опечатку в типе до `kubectl apply`; строковые ключи `"app.kubernetes.io/name"`; YAML на выходе | `aura eval k8s_deploy.aura --format yaml` |
| [ci_matrix/](ci_matrix/) | Генерация CI-матрицы через `map` + `component` вместо ручного перечисления комбинаций | `aura eval ci_matrix.aura` |
| [feature_flags/](feature_flags/) | `assert` как защита прода: опасная комбинация флагов останавливает деплой (E0530) | `aura eval feature_flags.aura --allow-env=APP_ENV` |
| [service_catalog/](service_catalog/) | Данные из существующих файлов проекта: `parse_toml`/`parse_json`, индексация `teams[0]`, `.get` с fallback | `aura eval service_catalog.aura --allow-read=.` |
| [security_demo/](security_demo/) | Capability-модель: импортированный модуль пытается читать `/etc/passwd` и получает **E0310** — права корня на импорт не распространяются | `aura eval main.aura --allow-read=.` *(ожидаемо падает!)* |
| [i18n/](i18n/) | Сборка локализаций: переводчики работают с плоскими JSON, Aura валидирует (ключи-сироты через `keys`/`contains`/`filter`) и мержит с fallback на базовую локаль | `aura eval i18n.aura --allow-read=.` |
| [validators/](validators/) | Пакет на D12: `pub def`/`pub type` как API (`v.service(...)`, `new v.Service`), приватные хелперы невидимы; детерминированное время — `parse_duration`/`format_duration`, арифметика дат через `parse_datetime` | `aura eval deploy.aura` |
| [telegram_bot/](telegram_bot/) | Конфиг телеграм-бота: секреты вне конфига (`token_env_var`), команды со схемой, dev/prod-переключение режима и лимитов, анти-дубль админов через `assert`, локализация со строковыми ключами | `aura eval bot.aura --allow-env=BOT_ENV` |
| [nginx/](nginx/) | **Генерация не-JSON формата**: block strings (D16) + `map`/`join` → готовый `nginx.conf` как строковое значение. Вложенные `{}`/`;` nginx — просто текст. Получить файл: `aura eval nginx.aura --format yaml` или `\| jq -r .nginx_conf` | `aura eval nginx.aura` |
| [showcase/](showcase/) | **Полный тур по языку в одном манифесте**: импорт модуля (D12), приватные `=`, `shadow`, схемы с опциональными полями (D15), `cond` (D14), `range`, тернарник, блочные строки `text … end` (D16), лямбды/`map`/`filter`, строковые/списочные/числовые методы, детерминированное время, `domain`/`component`, `assert`, чтение файла. Кейс для полного тестирования ядра | `aura eval showcase.aura --allow-read=. --allow-env=APP_ENV` |

Корневой [production_deploy.aura](production_deploy.aura) — эталонный манифест из [SPEC.ru.md](../SPEC.ru.md):
демонстрирует все конструкции языка сразу и обязан проходить полный пайплайн в CI.

```console
aura eval production_deploy.aura --allow-read=. --allow-env=APP_ENV --registry-dir=registry
```
