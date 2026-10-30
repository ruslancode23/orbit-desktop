# AGENTS.md — orbit-desktop

Инструкции для AI-агентов и людей. Коротко: это оболочка, а не приложение.

Orbit — приложение с постоянными голосовыми комнатами для команд. Web-клиент — `orbit-web`,
API — `orbit-server`; этот репозиторий — Tauri 2 обёртка для Windows и macOS. Источник требований —
`desktop-tasks.md` в корне рабочего пространства, решение об архитектуре —
[docs/adr/0006](docs/adr/0006-desktop-shell-remote-origin.md).

## Команды

```bash
pnpm install
cd ../orbit-web && pnpm dev   # приложение, отдельный терминал
pnpm dev                      # окно на http://localhost:5173
pnpm check                    # fmt + clippy -D warnings + тесты (как в CI)
ORBIT_DESKTOP_ORIGIN=https://orbit.example pnpm build
```

Node 22+, pnpm 11, Rust 1.82+, на macOS — Xcode Command Line Tools. Lockfile только `pnpm-lock.yaml`
и `Cargo.lock`; `Cargo.lock` коммитится (это приложение, а не библиотека).

## Правила

- **Здесь нет UI и нет бизнес-логики.** Экран, чат, голос — в `orbit-web`. Если в оболочке
  понадобилась логика приложения, скорее всего её место в `PlatformAdapter` (ADR-0004), а сюда
  приходит только нативная команда.
- **Новая нативная функция — это разрешение.** Команду из Rust недостаточно зарегистрировать:
  объяви её в `AppManifest::commands` в `build.rs` и выдай `allow-<имя>` в
  `src-tauri/capabilities/*.json`. Удалённая страница получает IPC только потому, что домен
  перечислен в `remote.urls`; расширять этот список «на время» нельзя.
- **Новые домены** — только через `ORBIT_DESKTOP_ORIGIN` и запись в `capabilities/main-remote.json`.
  `build.rs` специально падает, если одно есть, а другого нет.
- **CSP настраивает `orbit-web`**, не этот репозиторий: Tauri не вставляет политику в удалённые
  страницы. Не добавляй сюда `security.csp` «для порядка» — он не применится.
- **Debug-удобства не должны доезжать до релиза.** Проба (`src-tauri/spike`, `probe.rs`) —
  `#[cfg(debug_assertions)]` и переменная окружения; в release-сборке команды `report_spike` нет.
- Комментарии в коде — по-английски, тексты интерфейса и документация — по-русски.
- Комментарий объясняет «почему», а не «что»: `// 5173 because orbit-web pins strictPort` полезен,
  `// create a window` — нет.
- Ничего не блокирующего делать нельзя: `unwrap()` на пути запуска допустим только там, где
  ошибка означает сломанный конфиг (нет окна в `tauri.conf.json`). Для остального — `?` и
  внятное сообщение.
- Решение уровня стека (плагин, схема бандла, способ доставки обновлений) фиксируй в
  `docs/adr/`; нумерация продолжает `orbit-web`.

## Как проверять

- `pnpm check` — обязательный минимум. Юнит-тесты покрывают выбор origin (`src/origin.rs`);
  меняешь правила выбора — меняй тесты.
- Окно проверяется руками на macOS: `ORBIT_DESKTOP_PROBE=1 pnpm dev` печатает JSON с
  возможностями WebView. Это дешёвый способ узнать, что WebView умеет, не открывая devtools.
- `ORBIT_DESKTOP_TICKS=1 pnpm dev` сворачивает окно на 75 секунд и печатает счётчик тиков:
  так проверяется, троттлит ли WebView таймеры в скрытом окне (DESK-2,
  [docs/desk-2-hidden-window.md](docs/desk-2-hidden-window.md)). `ORBIT_DESKTOP_TICKS_SECONDS`
  меняет длительность.
- Windows локально не собирается осознанно: сборка идёт в GitHub Actions (`windows-latest`),
  установщик оттуда ставится на машину вручную. Кросс-сборку (`cargo-xwin`) не заводим.
- Сквозные сценарии (вход, чат, голос, экран) — вручную по чеклисту
  [docs/desk-1-spike.md](docs/desk-1-spike.md); автотестов на живое медиа здесь нет.
