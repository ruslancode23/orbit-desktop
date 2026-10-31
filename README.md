# orbit-desktop

Оболочка Orbit для Windows и macOS: Tauri 2, окно на тот же домен, что и в браузере.
Веб-приложение — [`orbit-web`](../orbit-web), API — [`orbit-server`](../orbit-server); здесь только
оболочка, без единой строки UI.

Почему окно открывает удалённый домен, а не собранный фронт внутри бинарника, — [ADR-0006](docs/adr/0006-desktop-shell-remote-origin.md).

## Что делает DESK-1

- Окно открывает удалённый origin: в dev — `http://localhost:5173`, в релизе — домен, зашитый при сборке.
- IPC (Tauri capabilities) разрешён только своим origin'ам. Встроенная страница-заглушка и чужие
  сайты в этом окне команд не получают.
- Проба для spike: `ORBIT_DESKTOP_PROBE=1` печатает в stdout измеренные возможности WebView
  (микрофон, экран, WASM, IndexedDB, WebRTC, AudioWorklet) — из них собрана таблица
  [docs/desk-1-spike.md](docs/desk-1-spike.md).

## Что оболочка даёт web (DESK-3)

Веб-приложение узнаёт оболочку по `window.__TAURI_INTERNALS__` и подменяет `PlatformAdapter`
на `createTauriPlatform()` (`orbit-web/src/shared/platform/tauri-platform.ts`). Из нативного сюда
приходит ровно четыре вещи, и все они выданы узкой capability:

| Что | Плагин | Права |
| --- | --- | --- |
| Системные уведомления, клик возвращает окно | `tauri-plugin-notification` | `notification:default`, `core:window:allow-set-focus/show/unminimize` |
| Автозапуск при входе в систему | `tauri-plugin-autostart` | `autostart:default` |
| Версия приложения | ядро Tauri | `core:app:allow-version` |
| ОС и архитектура для отчёта об ошибке | `tauri-plugin-os` | ничего: значения вшиты в страницу при сборке |

`install` и `push` в оболочке отвечают «не поддерживается»: PWA-установка и Web Push — браузерные
механизмы, desktop показывает уведомления сам (DESK-4).

### Уведомления (DESK-4)

Пока окно свёрнуто или скрыто, сообщения и упоминания приходят системными уведомлениями — решение
принимает тот же код, что и в браузере (`features/notifications`), поэтому «только упоминания» и
заглушённые каналы учитываются одинаково. Клик по уведомлению возвращает окно и открывает канал:
каждому уведомлению выдаётся числовой `id`, он возвращается в событии клика, и по нему находится
переход. Групповой замены уведомлений на macOS и Windows нет — плагин не передаёт идентификатор
в систему, поэтому несколько сообщений одного канала копятся.

Проверить доставку можно без ожидания сообщения:

```bash
ORBIT_DESKTOP_PROBE=1 ORBIT_DESKTOP_NOTIFY=1 pnpm dev
```

Оболочка отправит одно проверочное уведомление и напишет об этом в stdout. В dev-режиме macOS
показывает его от имени «Terminal»: плагин подставляет `com.apple.Terminal`, пока приложение не
собрано в `.app`.

## Запуск

Нужны Node.js 22+, pnpm 11, Rust 1.82+ и (на macOS) Xcode Command Line Tools: `xcode-select --install`.

```bash
pnpm install
cd ../orbit-web && pnpm dev      # отдельный терминал, http://localhost:5173
pnpm dev                         # в этом репозитории
```

`pnpm dev` собирает debug-оболочку и открывает окно на Vite. Без запущенного orbit-web окно
покажет ошибку загрузки — это вариант A из ADR-0006, офлайн-оболочки нет.

### Замеры в отладочной сборке

| Переменная | Что делает |
| --- | --- |
| `ORBIT_DESKTOP_PROBE=1` | после загрузки страницы печатает в stdout одну строку `[probe] {…}` с возможностями WebView |
| `ORBIT_DESKTOP_TICKS=1` | считает секундные тики, сворачивает окно и печатает счётчик — видно, троттлит ли WebView таймеры в скрытом окне (DESK-2) |
| `ORBIT_DESKTOP_TICKS_SECONDS=390` | сколько секунд окно остаётся свёрнутым (по умолчанию 75) |

Обе переменные работают только в debug и только с командой `report_spike`, которой в релизе нет.
Результаты замеров — [docs/desk-1-spike.md](docs/desk-1-spike.md) и
[docs/desk-2-hidden-window.md](docs/desk-2-hidden-window.md).

Сборка под домен:

```bash
ORBIT_DESKTOP_ORIGIN=https://orbit.example pnpm build
```

Сборка проверяет, что домен есть в `src-tauri/capabilities/main-remote.json`: без этого окно
открылось бы без IPC, а это видно только по отвалившимся уведомлениям и автозапуску.

## Origin

| Откуда берётся | Значение | Когда |
| --- | --- | --- |
| `ORBIT_DESKTOP_URL` | любой http(s) адрес | всегда главнее остальных, для dev и QA |
| `http://localhost:5173` | Vite в orbit-web | debug-сборка без переменных |
| `ORBIT_DESKTOP_ORIGIN` | домен, зашитый в бинарь | release-сборка |
| — | локальная страница-объяснение | домена нет |

`https://orbit.invalid` в capability — заглушка (RFC 2606, домен не существует). Настоящий домен
появится вместе с прод-стендом (REL-3); до него desktop живёт на dev-сборках.

## Скрипты

| Команда | Что делает |
| --- | --- |
| `pnpm dev` | debug-сборка и окно на Vite |
| `pnpm build` | release-сборка `.app`/`.dmg` (macOS) |
| `pnpm build:debug` | debug-сборка приложения и бандла |
| `pnpm check` | `cargo fmt --check`, clippy с `-D warnings`, тесты — как в CI |
| `pnpm test` | юнит-тесты Rust (в том числе выбор origin) |
| `pnpm icon` | перегенерировать иконки из исходника |

## Устройство

```
src-tauri/
  src/main.rs      точка входа: окно, IPC, страница-заглушка
  src/origin.rs    откуда берётся адрес окна (с тестами)
  src/probe.rs     проба для spike, только debug
  spike/probe.js   что именно спрашиваем у WebView
  capabilities/    кому разрешён IPC: main-remote.json
  tauri.conf.json  окно, бандл, CSP
  Info.plist       NSMicrophoneUsageDescription и NSCameraUsageDescription
shell/index.html   страница-заглушка, когда домен не задан
docs/              ADR и отчёт по spike
```

## Что дальше

Порядок задач — в `desktop-tasks.md` в корне рабочего пространства: DESK-2 (голос в фоне),
DESK-3 (`createTauriPlatform()`), DESK-5 (трей), DESK-9…11 (CI, подпись, автообновление).
Windows-сборка идёт только в GitHub Actions: локальную кросс-сборку не делаем.
