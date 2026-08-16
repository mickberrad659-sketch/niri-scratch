# niri-scratch

Надёжный менеджер scratchpad-пространств для Niri с интеграцией в Noctalia Shell.

## 1. Цель

Сделать для Niri поведение, близкое к special workspaces из Hyprland/HyDE:

- обычная повседневная нумерация остаётся `1`, `2`, `3`, `4`, `5`, `6`;
- одно нажатие открывает отдельное пространство с назначенным приложением;
- повторное нажатие возвращает пользователя точно на исходный workspace;
- приложение не запускается повторно, если его scratch-экземпляр уже существует;
- scratchpad не ломается после ручного переключения workspace, закрытия приложения,
  переподключения монитора или перезапуска демона;
- быстрые повторные нажатия и параллельные команды не создают гонок;
- Noctalia может показывать активный scratchpad и вызывать команды демона;
- проект работает как один небольшой нативный бинарник без shell-пайплайнов в hot path.

Niri не предоставляет настоящих скрытых `special workspace`, поэтому точную копию
внутреннего механизма Hyprland сделать нельзя без патча самого compositor. На уровне UX
можно получить то же переключение: обычные workspace 1–6 и зарезервированные именованные
scratch-workspace, скрытые из нашего меню/виджета Noctalia. В Overview Niri они могут быть
видны — это документированное ограничение первой версии.

## 2. Выбор стека

### Основной язык: Rust

Rust выбран вместо Go/C++ по следующим причинам:

- Niri и его IPC-модели написаны на Rust;
- можно использовать совместимые типы протокола `niri-ipc` либо держать небольшой
  собственный слой DTO поверх JSON IPC;
- один статически простой бинарник без runtime и garbage collector;
- безопасная модель владения для event loop, reconnect и конкурентных toggle-команд;
- низкое потребление памяти и практически нулевая задержка переключения;
- удобные property-based и интеграционные тесты конечного автомата;
- меньше риск use-after-free и data race, чем у ручной реализации на C++.

Производительность Go для этой задачи тоже достаточна: реальная задержка определяется
IPC Niri и запуском приложения, а не языком. Rust здесь выбирается прежде всего ради
надёжности, типизации протокола и удобной поставки одного бинарника.

### Планируемые зависимости

- `tokio` — однопоточный async runtime, Unix sockets, таймеры и сигналы;
- `serde`, `serde_json` — IPC и локальный JSON-протокол;
- `toml` — пользовательский конфиг;
- `clap` — CLI;
- `tracing`, `tracing-subscriber` — структурированные журналы;
- `thiserror` — типизированные ошибки;
- `directories` или `xdg` — корректные XDG-пути;
- `nix` — Unix socket, PID/process primitives, если стандартной библиотеки недостаточно;
- `fs2` либо lock-файл через `nix` — защита от двух экземпляров демона;
- `proptest` — тестирование конечного автомата;
- `tempfile` — изолированные тесты состояния.

Зависимости должны быть минимальными. Полный `niri-ipc` подключается только после
проверки совместимости с установленным Niri 26.04. Если его публичная crate-версия
не совпадает, реализуем узкий протокол поверх `$NIRI_SOCKET`: только запросы, ответы и
события, необходимые scratchpad-менеджеру. Не вызываем `niri msg` и `jq` на каждое
нажатие.

## 3. Состав проекта

```text
niri-scratch/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── PLAN.md
├── LICENSE
├── config/
│   └── example.toml
├── contrib/
│   ├── niri-scratch.service
│   ├── niri-bindings.kdl
│   └── noctalia/
│       ├── README.md
│       └── Scratchpads.qml
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── config.rs
│   ├── daemon.rs
│   ├── ipc/
│   │   ├── mod.rs
│   │   ├── niri.rs
│   │   └── control.rs
│   ├── model.rs
│   ├── reducer.rs
│   ├── runtime.rs
│   ├── launcher.rs
│   ├── persistence.rs
│   └── error.rs
└── tests/
    ├── reducer.rs
    ├── fake_niri.rs
    ├── reconnect.rs
    └── scenarios.rs
```

## 4. Пользовательский интерфейс

Основной интерфейс — клиентские команды к постоянно работающему демону:

```bash
niri-scratch toggle terminal
niri-scratch toggle web
niri-scratch toggle telegram
niri-scratch toggle codex
niri-scratch show web
niri-scratch hide web
niri-scratch hide-all
niri-scratch status --json
niri-scratch list --json
niri-scratch doctor
niri-scratch daemon
```

CLI-клиент соединяется с Unix socket демона, отправляет одну JSON-команду и завершается.
Запуск отдельного процесса на hotkey остаётся дешёвым, а единственный daemon сохраняет
историю фокуса и подписку на события Niri.

Пример биндов Niri:

```kdl
Mod+T repeat=false cooldown-ms=150 { spawn "niri-scratch" "toggle" "terminal"; }
Mod+S repeat=false cooldown-ms=150 { spawn "niri-scratch" "toggle" "web"; }
Mod+Slash repeat=false cooldown-ms=150 { spawn "niri-scratch" "toggle" "telegram"; }
Mod+C repeat=false cooldown-ms=150 { spawn "niri-scratch" "toggle" "codex"; }
```

## 5. Конфигурация

Путь: `$XDG_CONFIG_HOME/niri-scratch/config.toml`.

```toml
[daemon]
state_file = "${XDG_STATE_HOME}/niri-scratch/state.json"
socket = "${XDG_RUNTIME_DIR}/niri-scratch.sock"
focus_timeout_ms = 1200
launch_timeout_ms = 8000
reconnect_min_ms = 100
reconnect_max_ms = 5000

[scratchpads.terminal]
workspace = "scratch:terminal"
command = ["kitty", "--class", "niri-scratch-terminal"]
match_app_id = "^niri-scratch-terminal$"
launch_if_missing = true
focus_window_after_show = true
close_behavior = "keep-workspace"
preferred_output = "focused"

[scratchpads.web]
workspace = "scratch:web"
command = ["firefox", "--new-instance", "--profile", "${HOME}/.mozilla/firefox/scratch-web"]
match_app_id = "^firefox$"
match_title = "\\[scratch-web\\]"
launch_if_missing = true
preferred_output = "focused"

[scratchpads.telegram]
workspace = "scratch:telegram"
command = ["Telegram"]
match_app_id = "^org\\.telegram\\.desktop$"
launch_if_missing = true
singleton_policy = "adopt-existing"
preferred_output = "focused"

[scratchpads.codex]
workspace = "scratch:codex"
command = ["kitty", "--class", "niri-scratch-codex", "-e", "codex"]
match_app_id = "^niri-scratch-codex$"
launch_if_missing = true
preferred_output = "focused"
```

Перед реализацией Firefox-профиля нужно проверить фактический Wayland `app_id` и
возможность стабильной маркировки окна. Если Firefox нельзя отличить по app_id,
используем отдельный desktop file с `StartupWMClass`, профиль и управляемый суффикс
заголовка либо запускаем web scratchpad без попытки усыновить обычное окно Firefox.

## 6. Архитектура

### 6.1. Daemon

Один user-daemon владеет состоянием всех scratchpad. Он запускается как часть
графической Niri-сессии через systemd user unit.

Задачи демона:

1. подключиться к `$NIRI_SOCKET`;
2. получить начальные snapshots `workspaces` и `windows`;
3. открыть отдельное соединение `event-stream`;
4. поддерживать актуальную модель окон, workspace, output и focus;
5. принимать команды через `$XDG_RUNTIME_DIR/niri-scratch.sock`;
6. последовательно применять их через reducer/actor;
7. переподключаться при рестарте Niri;
8. атомарно сохранять только данные, полезные после рестарта.

Все команды сериализуются одним actor loop. Это исключает ситуацию, когда два быстрых
нажатия одновременно прочитали старое состояние и оба решили показать scratchpad.

### 6.2. Niri IPC adapter

Адаптер скрывает версию IPC от основной логики:

```rust
trait Compositor {
    async fn snapshot(&mut self) -> Result<Snapshot>;
    async fn focus_workspace(&mut self, target: WorkspaceRef) -> Result<()>;
    async fn focus_window(&mut self, id: WindowId) -> Result<()>;
    async fn move_window_to_workspace(
        &mut self,
        id: WindowId,
        target: WorkspaceRef,
    ) -> Result<()>;
    async fn spawn(&mut self, argv: Vec<String>) -> Result<()>;
    async fn events(&mut self) -> Result<EventStream>;
}
```

В первой версии не следует полагаться на вывод CLI. Прямой IPC позволяет отличать
ошибку запроса, разрыв socket и несовместимую версию протокола.

### 6.3. Control socket

Локальный newline-delimited JSON протокол:

```json
{"version":1,"request_id":"...","command":"toggle","scratchpad":"terminal"}
{"version":1,"request_id":"...","ok":true,"state":"visible"}
```

Socket создаётся с правами `0600`. Демон проверяет UID peer через `SO_PEERCRED`.
Команды имеют `request_id`, поэтому повтор после таймаута можно сделать идемпотентным.

### 6.4. Noctalia

Первая интеграция не должна связывать core с QML:

- Noctalia запускает `niri-scratch toggle <name>`;
- виджет периодически получает `niri-scratch status --json` либо подписывается на
  control socket;
- активный scratchpad обозначается отдельной иконкой/цветом;
- список стандартных workspace фильтрует префикс `scratch:` и оставляет 1–6;
- контекстное меню позволяет показать, скрыть, перезапустить или закрыть приложение.

После стабилизации CLI добавляется streaming-команда `watch --json`, чтобы QML не
делал polling.

## 7. Модель состояния

```rust
struct ScratchpadState {
    name: String,
    visibility: Visibility,
    workspace: Option<WorkspaceId>,
    windows: Vec<WindowId>,
    origin: Option<ReturnTarget>,
    transition: Option<Transition>,
    last_error: Option<String>,
}

enum Visibility {
    Hidden,
    Visible,
    Degraded,
}

struct ReturnTarget {
    workspace_id: WorkspaceId,
    workspace_name: Option<String>,
    output_name: String,
    focused_window_id: Option<WindowId>,
}

enum Transition {
    Showing { generation: u64 },
    Hiding { generation: u64 },
    Launching { generation: u64, deadline: Instant },
}
```

`generation` защищает от запоздавших событий: событие от старого запуска не должно
завершить более новую операцию toggle.

## 8. Семантика toggle

### Показ

1. Зафиксировать текущие workspace, output и focused window как `origin`.
2. Если scratchpad уже видим на текущем workspace, операция становится hide.
3. Найти scratch workspace и подходящие окна по стабильным идентификаторам.
4. Перейти на workspace.
5. Если окно существует — сфокусировать нужное окно.
6. Если окна нет — запустить команду без shell и ждать события `WindowOpened`.
7. Проверить matcher; ложное совпадение не принимать.
8. После события сфокусировать scratch-окно и отметить состояние `Visible`.
9. При timeout оставить workspace доступным, выставить `Degraded` и отправить
   уведомление с диагностикой.

### Скрытие

1. Проверить, что текущий workspace действительно scratch workspace.
2. Выбрать сохранённый `origin`.
3. Если origin workspace ещё существует — вернуться по ID.
4. Если ID исчез, попробовать имя на исходном output.
5. Если workspace исчез полностью — выбрать последний обычный workspace этого output.
6. В крайнем случае выполнить `focus-workspace-previous`.
7. Восстановить focused window, только если окно ещё существует на выбранном workspace.
8. Очистить transient origin и отметить `Hidden`.

### Поведение при переключении вручную

Если пользователь ушёл со scratch workspace обычным биндом, scratchpad становится
`Hidden`, но сохранённый origin не используется автоматически. Следующий `toggle`
снова запоминает новое место вызова и показывает существующее scratch-окно.

### Несколько scratchpad

При вызове scratchpad B из scratchpad A доступны две политики:

- `stack` — B запоминает A как origin, hide B возвращает в A;
- `replace` — A считается скрытым, hide B возвращает на последний обычный workspace.

По умолчанию используется `stack`, максимальная глубина ограничена, например, 8.

## 9. Окна и процессы

Нельзя считать PID единственным идентификатором:

- GUI-приложение может форкнуться;
- Telegram/Firefox могут передать запрос уже работающему процессу;
- Electron может создать несколько окон;
- PID теряется после рестарта демона.

Приоритет сопоставления:

1. сохранённый `WindowId`, подтверждённый текущим snapshot;
2. точный/regex `app_id`;
3. дополнительный matcher заголовка;
4. PID/descendant PID только во время первоначального запуска;
5. ручной выбор политики при нескольких совпадениях.

Команды запускаются через IPC action `spawn` самого Niri с готовым массивом аргументов,
без `/bin/sh -c`. Так приложение получает нормальный контекст графической сессии и не
наследует sandbox демона. Переменные `${HOME}` и XDG раскрываются собственным ограниченным
interpolator — без command substitution.

## 10. Workspace и мониторы

Niri держит отдельный набор workspace на каждом output, поэтому ID и имя нельзя считать
глобально вечными.

Политики размещения:

- `focused` — scratchpad появляется на текущем output;
- `fixed = "eDP-1"` — всегда на конкретном output;
- `follow-window` — использовать output существующего scratch-окна;
- `origin` — создавать/искать scratch workspace на output вызова.

При отключении монитора daemon принимает новое размещение из событий Niri и не пытается
немедленно вернуть workspace обратно. После переподключения authoritative state всегда
берётся у compositor.

## 11. Персистентность и восстановление

Постоянно сохраняются только:

- версия схемы;
- известные matcher и последние WindowId как hint;
- пользовательские настройки/статистика ошибок;
- последняя нормальная версия конфигурации.

Не сохраняются как истина:

- `Visible/Hidden`;
- workspace ID;
- PID;
- переходы `Showing/Hiding`.

После запуска daemon делает reconciliation по snapshot Niri. Файл состояния записывается
через temp file, `fsync`, rename и права `0600`. Повреждённый файл переименовывается в
backup и не мешает старту.

## 12. Отказоустойчивость

- потеря Niri socket: exponential backoff с jitter, CLI отвечает `temporarily unavailable`;
- рестарт Niri: новый snapshot, сброс transient transitions, повторное сопоставление окон;
- закрытие окна: scratchpad остаётся скрытым/пустым и запускается заново при toggle;
- зависший запуск: timeout, уведомление, команда `restart`;
- два демона: lock + проверка занятого control socket;
- зависший старый socket: проверка peer, только затем безопасная замена;
- несовместимый IPC: понятная ошибка `doctor`, без циклического crash/restart;
- повреждённый config: daemon сохраняет последнюю рабочую конфигурацию и сообщает ошибку;
- быстрый double toggle: последовательная очередь и generations;
- suspend/resume: переснять snapshot после resume;
- отсутствие Noctalia: core полностью продолжает работать.

## 13. Systemd user service

```ini
[Unit]
Description=Niri scratchpad manager
PartOf=graphical-session.target
After=graphical-session.target
ConditionEnvironment=NIRI_SOCKET

[Service]
Type=notify
ExecStart=%h/.local/bin/niri-scratch daemon
Restart=on-failure
RestartSec=250ms
TimeoutStopSec=2s
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=%t %S/niri-scratch
StateDirectory=niri-scratch
MemoryMax=96M
TasksMax=32

[Install]
WantedBy=graphical-session.target
```

Hardening проверяется через `systemd-analyze security --user`. Приложения запускаются
через Niri IPC action `spawn`, поэтому они не наследуют sandbox user service. Если
конкретная версия IPC не поддержит передачу argv, fallback делается через отдельный
минимальный launcher service, а не ослаблением sandbox основного демона.

## 14. Безопасность

- control socket `0600` и проверка UID клиента;
- конфиг не поддерживает произвольный shell по умолчанию;
- журналы не содержат environment, аргументы с секретами и заголовки приватных окон;
- QML получает только имя и состояние scratchpad, а не список всех окон;
- matcher имеет лимиты длины; regex компилируются при загрузке конфига;
- ограничение размера JSON frame и числа pending requests;
- Unix socket размещается только в `$XDG_RUNTIME_DIR`;
- daemon никогда не выполняется от root.

## 15. Наблюдаемость

`niri-scratch doctor` проверяет:

- наличие и версию Niri;
- доступность `$NIRI_SOCKET`;
- совместимость IPC;
- валидность конфига и regex;
- наличие команд приложений;
- уникальность matchers;
- видимость user service;
- доступность Noctalia-интеграции;
- конфликт имён scratch workspace с обычными workspace 1–6.

Журнал доступен через:

```bash
journalctl --user -u niri-scratch.service -f
```

Уровень логирования задаётся через `RUST_LOG`, но по умолчанию остаётся компактным.

## 16. Тестирование

### Unit tests

- parsing и validation TOML;
- matcher окон;
- reducer для всех переходов;
- fallback return target;
- generation и дедупликация request ID;
- migration state/config schema.

### Property tests

Генерируются случайные последовательности событий:

- toggle/show/hide;
- WindowOpened/Closed/Focused;
- WorkspaceActivated/Destroyed;
- OutputConnected/Disconnected;
- IPC disconnected/reconnected;
- process timeout.

Инварианты:

- одновременно не больше одного активного transition на scratchpad;
- hide никогда не возвращает на scratch workspace самого себя;
- один request ID не применяет действие дважды;
- compositor snapshot всегда сильнее локального persisted hint;
- обычные workspace 1–6 не переименовываются и не перемещаются.

### Fake Niri

Интеграционные тесты используют mock Unix socket с записанными JSON fixtures. Проверяются
гонки, reconnect, устаревшие события и ошибки запросов без запуска графической сессии.

### Реальная проверка

Отдельный checklist выполняется внутри Niri:

1. открыть/скрыть каждый scratchpad 50 раз;
2. double tap и удержание hotkey;
3. переключиться вручную до hide;
4. закрыть приложение в видимом и скрытом состоянии;
5. перезапустить daemon;
6. reload Niri config;
7. suspend/resume;
8. подключить/отключить внешний монитор;
9. рестарт приложения с несколькими окнами;
10. убедиться, что обычные workspace 1–6 сохраняют порядок.

## 17. Производительность

Целевые показатели на ноутбуке:

- idle CPU: практически 0%;
- RSS: до 20–30 MiB, жёсткий предел service 96 MiB;
- CLI → IPC acknowledgement: менее 10 ms без учёта compositor animation;
- отсутствие polling Niri и процессов `jq` на каждое нажатие;
- один event-stream и одна in-memory модель;
- release-сборка с LTO, strip и `panic = "abort"` после стабилизации.

Пример профиля release:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
opt-level = 3
panic = "abort"
strip = "symbols"
```

Главная оптимизация — не язык сам по себе, а постоянное IPC-соединение, отсутствие shell,
polling и лишних процессов.

## 18. Этапы реализации

### Этап 0 — исследование протокола

- зафиксировать версию Niri 26.04;
- получить реальные fixtures `workspaces`, `windows`, `event-stream` в Niri-сессии;
- проверить аргументы `focus-workspace`, `focus-window`, `move-window-to-workspace`;
- проверить доступную версию `niri-ipc` crate;
- проверить фильтрацию workspace в Noctalia 4.7.7.

Результат: ADR с выбранным вариантом IPC и каталог fixtures.

### Этап 1 — минимальный вертикальный срез

- Cargo workspace/binary;
- config parser;
- прямое соединение с Niri;
- snapshot workspaces/windows;
- `toggle terminal` для одного scratchpad;
- systemd user unit;
- unit и fake-IPC tests.

Критерий: 100 последовательных переключений terminal без дубликатов.

### Этап 2 — надёжный daemon

- control socket;
- actor/reducer;
- event-stream;
- reconciliation;
- reconnect/backoff;
- корректный origin stack;
- несколько scratchpad;
- `status`, `list`, `doctor`.

### Этап 3 — приложения

- Kitty terminal;
- Codex terminal;
- Telegram singleton/adoption;
- отдельный Firefox scratch-profile;
- правила Niri для размеров, плавающего/тайлингового режима и app_id.

### Этап 4 — Noctalia

- QML widget/menu;
- streaming `watch --json`;
- индикаторы visible/launching/error;
- скрытие `scratch:` из обычного списка workspace, если API Noctalia позволяет;
- тема из текущей палитры Noctalia без собственных hardcoded цветов.

### Этап 5 — production hardening

- property tests;
- fuzz config/control protocol;
- systemd sandbox;
- packaging PKGBUILD;
- shell completions и man page;
- CI: fmt, clippy, test, deny/audit;
- release binary и rollback-инструкция.

## 19. Definition of Done

Проект считается готовым, когда:

- обычные workspace визуально и логически остаются 1–6;
- каждый scratchpad открывается и скрывается одним и тем же хоткеем;
- hide возвращает на фактическое место вызова, включая нужный монитор;
- double tap не создаёт процесс или окно-дубликат;
- закрытие приложения и restart daemon восстанавливаются автоматически;
- нет `bash`, `jq` и polling в рабочем пути;
- все unit/property/integration tests проходят;
- `niri-scratch doctor` не показывает ошибок;
- Noctalia отображает состояние и не показывает scratch workspace среди 1–6;
- есть пакет, systemd unit, пример конфига и инструкция отката;
- старый `~/.local/bin/niri-scratch-ws` больше не используется биндами.

## 20. Решения, которые не принимаются

- набор независимых shell-скриптов без общего состояния;
- `focus-workspace-previous` как единственный механизм возврата;
- поиск окна только по PID или только по заголовку;
- polling `niri msg` несколько раз в секунду;
- хранение workspace ID как вечного идентификатора;
- выполнение пользовательской строки через shell по умолчанию;
- изменение или переименование обычных workspace 1–6;
- патч Niri compositor на первом этапе.

## 21. Первый практический target

Начинать следует с `terminal`, потому что Kitty позволяет задать уникальный app_id и
даёт полностью детерминированный тест. После подтверждения автомата добавляются Codex,
Telegram и Firefox — в таком порядке. Это отделяет проблемы scratchpad-движка от
особенностей singleton-приложений и браузерных профилей.
