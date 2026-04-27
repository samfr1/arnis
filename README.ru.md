# ARNIS GLOBAL — генерация Minecraft-мира из OpenStreetMap

Этот форк добавляет в [Arnis](https://github.com/louis-e/arnis) **тайловый движок** и
**локальную web-админку**: можно сгенерировать любой регион Земли (хоть весь мир)
не упираясь в RAM. Каждый тайл — отдельный дочерний процесс, готовые куски
**склеиваются на лету в один большой мир Minecraft**, прогресс пишется в
манифест на диск, прерывание процесса не страшно — можно продолжить с того же
места.

По умолчанию открывается на **центре Санкт-Петербурга**, область 5×5 км. Этот
дефолт настраивается в файле `~/.config/arnis/config.toml`.

```
$ arnis --web
Arnis admin panel listening on http://127.0.0.1:7373/
```

Открывается браузер → одна кнопка «Сгенерировать (Питер 5×5 км)» → Minecraft
мир в `./arnis-world/`. Готово.

---

## Содержание

- [Что нового по сравнению с upstream](#что-нового-по-сравнению-с-upstream)
- [Установка на Linux](#установка-на-linux)
- [Установка на Windows](#установка-на-windows)
- [Конфигурация](#конфигурация)
- [Использование админки](#использование-админки)
- [Снапшоты «на бегу»](#снапшоты-на-бегу)
- [Архитектура и инварианты](#архитектура-и-инварианты)
- [Часто задаваемые вопросы](#часто-задаваемые-вопросы)

---

## Что нового по сравнению с upstream

| фича | как работает |
| --- | --- |
| **Тайловый движок** | bbox делится на сетку 5×5 км (настраивается). Тайлы идут последовательно в дочерних процессах. Память освобождается после каждого тайла. |
| **Один склеенный мир** | После каждого готового тайла его region-файлы (`r.X.Z.mca`) сливаются в мастер-мир (`./arnis-world/region/`). Если region уже есть — переписываются только нужные чанки. Получается **один Minecraft world** на весь объём. |
| **Манифест и resume** | Для каждой задачи на диске лежит JSON-манифест `world.arnis-job.json` с состоянием каждого тайла. Запись атомарная (`tmp + rename`). Если процесс убили — при следующем `--web` всё восстанавливается, висящие тайлы возвращаются в `pending`. |
| **Web-админка** | Локальный HTTP+WebSocket сервер `127.0.0.1:7373` (или `0.0.0.0` через конфиг). Карта Leaflet, прогресс-сетка по тайлам, лог в реальном времени, кнопки Pause/Resume/Cancel/Restore Backup. |
| **Снапшоты** | Кнопка «Save snapshot» копирует мастер-мир в `<world>_snapshot_<timestamp>/` фоновым потоком. Генерация при этом не останавливается. |
| **Skip-ocean** | Перед запуском дочернего процесса делается лёгкий count-запрос в Overpass. Если данных нет (океан, Антарктида) — тайл помечается `skipped`, дочерний процесс не стартует. |
| **Конфиг-файл** | `~/.config/arnis/config.toml` (Linux/macOS) / `%APPDATA%\arnis\config.toml` (Windows). Создаётся автоматически при первом запуске. |
| **CLI-флаги для админки** | `--web --port 7373 --host 0.0.0.0` — переопределяют значения из конфига. |

---

## Установка на Linux

Тестировано на Ubuntu 22.04 / 24.04 и Debian 12. Подойдут любые derivatives.

### 1. Зависимости

Минимум: Rust ≥ 1.79, gcc/clang, OpenSSL dev, pkg-config, X11 libs (для GUI),
ALSA/GTK (если хотите ещё и desktop GUI).

```bash
sudo apt update
sudo apt install -y \
    build-essential pkg-config curl git \
    libssl-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
    libxkbcommon-dev libgtk-3-dev libsoup-3.0-dev libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev
```

Если нужен **только web-режим** (рекомендую для серверов без X11), хватит этого:

```bash
sudo apt install -y build-essential pkg-config libssl-dev curl git
```

### 2. Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup default stable
```

### 3. Сборка

```bash
git clone https://github.com/samfr1/arnis.git
cd arnis
git checkout devin/1777246671-tile-engine-web-panel

# Web-режим, без GUI (быстро, минимум зависимостей):
cargo build --release --features web --no-default-features

# Полный билд (web + desktop GUI):
cargo build --release
```

Бинарь будет в `./target/release/arnis`.

### 4. Запуск

```bash
./target/release/arnis --web
# открывается http://127.0.0.1:7373/
```

Если нужен доступ из локальной сети (`0.0.0.0`):

```bash
./target/release/arnis --web --host 0.0.0.0 --port 7373
```

> Внимание: на `0.0.0.0` админка **без аутентификации**. Не выставляйте её в
> публичный интернет.

---

## Установка на Windows

Тестировано на Windows 10 и 11 (x64).

### 1. Visual Studio Build Tools

Rust на Windows требует MSVC toolchain. Скачайте установщик
[Build Tools for Visual Studio 2022](https://visualstudio.microsoft.com/visual-cpp-build-tools/),
выберите workload **«Desktop development with C++»** → ставьте.

### 2. Rust

Скачайте `rustup-init.exe` с https://rustup.rs и запустите. Принимайте все
дефолты (MSVC, stable).

После установки откройте новый PowerShell:

```powershell
rustc --version
cargo --version
```

### 3. Git

Если ещё нет — поставьте [Git for Windows](https://git-scm.com/download/win).

### 4. Сборка

```powershell
git clone https://github.com/samfr1/arnis.git
cd arnis
git checkout devin/1777246671-tile-engine-web-panel

# Web-режим (рекомендую):
cargo build --release --features web --no-default-features

# Полный билд (web + desktop GUI на eframe):
cargo build --release
```

Бинарь: `target\release\arnis.exe`.

### 5. Запуск

```powershell
.\target\release\arnis.exe --web
```

Откроется браузер на `http://127.0.0.1:7373/`. Чтобы биндилось на 0.0.0.0
(локальная сеть):

```powershell
.\target\release\arnis.exe --web --host 0.0.0.0 --port 7373
```

---

## Конфигурация

При первом запуске `arnis --web` создаётся конфиг:

- **Linux / macOS**: `~/.config/arnis/config.toml`
- **Windows**: `%APPDATA%\arnis\config.toml` (обычно `C:\Users\<user>\AppData\Roaming\arnis\config.toml`)

Дефолтное содержимое:

```toml
# Arnis admin panel configuration
# See https://github.com/louis-e/arnis for documentation.
# Override defaults below; CLI flags --host/--port still take priority.

bind = "127.0.0.1"
port = 7373
default_bbox = [
    59.911842,
    30.290273,
    59.956758,
    30.379927,
]
default_world_path = ""
default_tile_size_km = 5.0
auto_open_browser = true
```

| ключ | смысл |
| --- | --- |
| `bind` | На каком IP биндить web-сервер. `127.0.0.1` — только localhost (безопасно), `0.0.0.0` — все интерфейсы (LAN). |
| `port` | TCP-порт. По умолчанию `7373`. |
| `default_bbox` | `[min_lat, min_lng, max_lat, max_lng]`. По умолчанию — квадрат 5×5 км вокруг центра Питера (59.9343°N, 30.3351°E). Замените на свой регион — кнопка «Сгенерировать» будет использовать его. |
| `default_world_path` | Куда писать мастер-мир. Пусто = `<cwd>/arnis-world`. Можно указать любой абсолютный путь. |
| `default_tile_size_km` | Размер тайла в км. 1–100. Чем меньше — тем меньше памяти на тайл, но больше overhead на запуск процесса. 5 — sweet spot для городов. |
| `auto_open_browser` | Открывать ли браузер автоматически. Удобно отключить на серверах без X11. |

**CLI-флаги перезаписывают конфиг**:

```bash
arnis --web --port 8080         # конфиг говорит 7373, но запустим на 8080
arnis --web --host 0.0.0.0      # биндим на все интерфейсы независимо от конфига
```

---

## Использование админки

После запуска `arnis --web` открывается веб-страница:

### Setup (стартовый экран)

- **«Сгенерировать (Питер 5×5 км)»** — главная синяя кнопка. Берёт
  `default_bbox` и `default_tile_size_km` из конфига и стартует. Один клик.
- **«Весь мир»** — серая. Делит планету на тайлы по 100 км и идёт лево-направо
  сверху-вниз. **Долгая** задача (76 тыс. тайлов, дни-недели на одной машине).
  Подтверждение перед стартом.
- **World folder** — путь к мастер-миру. Пусто = `arnis-world` рядом с бинарём.
- **Tile size (км)** — ползунок 1–100.
- **Advanced** — раскрывающийся блок: ручной bbox через карту/числовые поля,
  тумблеры terrain / interiors / roofs / land cover, scale.

### Live progress map

Карта OSM с прозрачными прямоугольниками-тайлами поверх. Цвета:
- серый — pending
- синий пульсирующий — running
- зелёный — done
- красный — failed
- очень тёмный серый — canceled / skipped

Карта автоцентрируется на текущий тайл.

### Control

- **Start / Pause / Resume / Cancel** — управление executor'ом. Все они
  «мягкие»: текущий тайл всегда дописывается до конца, потом срабатывает
  команда. Мир остаётся в консистентном состоянии.
- **Save snapshot** — копия мастер-мира в `<world>_snapshot_<ts>/` фоновым
  потоком. Генерация продолжается. Появится в блоке «Snapshots».
- **Restore Backup** — заменяет мир бэкапом, который снимался в самом начале
  работы (до первого тайла). Доступно только когда job на pause или canceled.

### Stats / Log

Прогресс, ETA, текущий тайл, скиппнутые/упавшие. Лог стримится по WebSocket
(`/ws/events`) — последние 300 строк, автоскролл.

---

## Снапшоты «на бегу»

Кнопка **Save snapshot** делает рекурсивную копию мастер-мира в
соседнюю папку `<world>_snapshot_<unix_ts>/`. Копирование идёт **в отдельном
потоке**, executor продолжает работать.

```
arnis-world/                       <-- живой мир, executor пишет в него
arnis-world_snapshot_1714150000/   <-- замороженная копия, можете играть
arnis-world_snapshot_1714153200/   <-- следующая
```

Чтобы поиграть в Minecraft пока генерация идёт:

1. Жмёте **Save snapshot**.
2. Через минуту-две (зависит от размера) в блоке Snapshots появится новая
   папка.
3. Копируете её в `~/.minecraft/saves/MyArnisWorld` (или эквивалент в Windows).
4. Запускаете Minecraft → Singleplayer → играете.
5. Тем временем executor продолжает генерить в `arnis-world/`.

Лучшая консистентность снапшота — сначала Pause, потом Snapshot, потом
Resume. Снапшот «на бегу» технически может зацепить тайл, который как раз
мержился — там будет несколько частично записанных region-файлов.

---

## Архитектура и инварианты

```
┌───────────────────┐     ┌─────────────────────┐
│  axum web server  │     │   tile_engine       │
│  127.0.0.1:7373   │ ◄──►│   ┌──────────────┐  │
│  + WebSocket      │     │   │  Executor    │  │
└────────┬──────────┘     │   │  (std thread)│  │
         │                │   └──────┬───────┘  │
         │                │          │ spawn    │
         │                │          ▼          │
         │                │   ┌──────────────┐  │
         │                │   │ child arnis  │  │
         │                │   │ for tile N   │  │
         │                │   └──────┬───────┘  │
         │                │          │ writes   │
         │                │          ▼          │
         │                │   <world>_tiles/    │
         │                │      r000-c000/     │
         │                │       Arnis World/  │
         │                │        region/*.mca │
         │                │          │          │
         │                │          ▼ merge    │
         │                │   <world>/region/   │
         │                │     r.X.Z.mca       │
         │                │   (один большой     │
         │                │    мастер-мир)      │
         │                └─────────────────────┘
         │
         ▼
   admin panel HTML (embedded)
```

- Каждый тайл генерится **изолированным дочерним процессом** — если он
  упал/съел всю память, родитель просто помечает тайл failed и идёт дальше.
- После успеха: `tile_engine::merge::merge_tile_into_master()` копирует
  region-файлы в мастер-мир. Если мастер-region не существует — fast-path
  `fs::copy()`. Если есть — slow-path: открываем оба `.mca` через `fastanvil`
  и переписываем по чанкам.
- `level.dat` копируется один раз с первого успешного тайла.
- Манифест сохраняется максимум раз в 2.5 секунды (троттлинг). Без
  троттлинга на «весь мир» (76к тайлов) запись манифеста уходила в терабайты
  ввода-вывода.
- Skipped-тайлы (Overpass говорит «нет данных») не запускают дочерний процесс
  вообще. Экономит часы на океанах.

---

## Часто задаваемые вопросы

**Q: А если процесс убили в середине?**
A: Запустите `arnis --web` снова. Манифест на диске → executor видит
старую задачу, тайлы со статусом `running` сбрасывает в `pending`, и можно
жать Resume. Никакие чанки не теряются — мастер-мир пишется атомарно после
каждого успешного тайла.

**Q: Сколько RAM ест один тайл?**
A: 5×5 км в среднем — 1–3 ГБ. Большой город на 10×10 км — до 6–8 ГБ.
Регулируется ползунком Tile size.

**Q: Можно ли запустить на сервере без X11/GUI?**
A: Да. Соберите с `--no-default-features --features web`. Всё работает через
браузер на вашей рабочей машине, указав `--host 0.0.0.0` и подключившись по
LAN/SSH-туннелю.

**Q: Куда писать issues по тайловому движку?**
A: В этот форк: https://github.com/samfr1/arnis/issues. По обычному пайплайну
Arnis (генерация одного bbox) — в upstream https://github.com/louis-e/arnis.

**Q: Как сменить порт без правки конфига?**
A: `arnis --web --port 8080`. Можно так же `--host`.

**Q: Может ли админка работать одновременно на двух bbox?**
A: Нет. Один executor, один манифест. Можно завести вторую задачу
только после `done`/`canceled` предыдущей. Если очень нужен параллелизм —
запустите два процесса arnis на разных портах с разными `default_world_path`.

---

## Лицензия

Этот форк наследует лицензию upstream — см. [LICENSE](LICENSE).
