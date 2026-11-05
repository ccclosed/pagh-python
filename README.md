# pagh-python — нативный интерпретатор Python для pagh-riscv

Собственная реализация Python (не CPython) на Rust поверх `libpagh`:
лексер → indentation-aware парсер → AST → tree-walking интерпретатор.
Собирается в статический freestanding ELF для `riscv64gc-unknown-none-elf`
и запускается в U-mode:

```text
pagh:rv64> run /mnt/pythonc /mnt/script.py
```

## Статус

**Рабочая версия 0.1.0.** Язык и встроенные функции реализованы настолько,
насколько это нужно для небольших скриптов на 1 MiB bump-heap ядра pagh.
Интерпретатор проверен живым прогоном в QEMU на ядре 0.14.0: 21 проверка (live на 0.14.0)
из 21 прошла, включая float-арифметику и модуль `math`, классы с
наследованием, исключения, файловый ввод-вывод, импорт локального модуля и
диагностику ошибок. Сырой транскрипт — [`docs/live-pythonc.txt`](docs/live-pythonc.txt),
PASS-лист и детали — [`docs/live-testing.md`](docs/live-testing.md).

Размер release-ELF — 781 928 байт (≈763 КиБ; лимит ядра — 1 МиБ). Потоков,
процессов, сети и динамического загрузчика нет. Интерпретатор исполняет
настоящие f64-инструкции в U-mode, поэтому ядро обязано оставлять `sstatus.FS`
включённым при `sret` (в 0.14.0 это так; проверено живым запуском
`/home/oleg/pagh-api/examples/fp.rs`: `fp product=3.375`, `fp sum=5050`, exit 0).

## Сборка

```sh
cargo build --release --target riscv64gc-unknown-none-elf
```

`target/riscv64gc-unknown-none-elf/release/pythonc` — готовый ELF. Ядро
принимает образы не больше 1 МиБ; `panic = "abort"`, `strip = "debuginfo"` и
`overflow-checks = false` уже заданы в `Cargo.toml` (переполнения проверяются
вручную и превращаются в Python `OverflowError`).

## Запуск в QEMU

```sh
rm -rf /tmp/py-rootfs && mkdir /tmp/py-rootfs
cp target/riscv64gc-unknown-none-elf/release/pythonc tests/live/*.py /tmp/py-rootfs/
mke2fs -q -t ext2 -F -b 4096 -d /tmp/py-rootfs /tmp/py-live.img 64M

python3 tests/live/qemu_driver.py \
  --disk /tmp/py-live.img \
  --commands tests/live/live.cmds \
  --transcript /tmp/live.txt
```

Драйвер сам поднимает QEMU с virtio-диском и serial-сокетом, ждёт
`pagh:rv64>`, отправляет команды и сохраняет транскрипт. В интерактивном
QEMU достаточно `run /mnt/pythonc /mnt/hello.py`.

## CLI

```text
pythonc script.py [args...]     выполнить файл; sys.argv = [script.py, args...]
pythonc -c 'code' [args...]     выполнить строку; sys.argv = ['-c', args...]
pythonc                         краткая справка, код возврата 2
```

Коды возврата: `0` — успех, `1` — синтаксическая ошибка или необработанное
исключение, `2` — ошибка командной строки/открытия файла.

Синтаксическая ошибка печатается одной строкой с файлом, строкой и колонкой:

```text
/mnt/syntax_error.py:2:5: SyntaxError: expected a parameter name, found 'return'
```

Необработанное исключение печатается Python-образным traceback:

```text
Traceback (most recent call last):
  File "/mnt/uncaught.py", line 8, in <module>
  File "/mnt/uncaught.py", line 5, in middle
  File "/mnt/uncaught.py", line 2, in divide
ZeroDivisionError: integer division or modulo by zero
```

## Поддерживается

**Типы:** `None`, `bool`, `int` (64-битный знаковый), `float` (f64),
`str` (UTF-8), `list`, `tuple`, `dict` (с сохранением порядка вставки),
`range`, файловые объекты, функции, классы и экземпляры. `set`/`bytes`
отсутствуют (см. «Ограничения»).

**Синтаксис и операторы:** целые и float-литералы (`1.5`, `.5`, `1e3`,
`1.5e-2`, `1_0.5`), имена, `+ - * / // % **` (включая унарные), сравнения
(`== != < <= > >= in not in is is not`), цепочки сравнений, `and/or/not`,
тернарное `a if cond else b`, срезы `a[start:stop:step]` для
`str`/`list`/`tuple`, индексация, атрибуты, вызовы с позиционными и
именованными аргументами. Присваивания (включая цепочки и `+=` и т.п.)
поддерживаются. `/` всегда возвращает float (`1 / 2 == 0.5`), `//` сохраняет
тип (int для int, float для float), `%` следует знаку делителя, `**`
поддерживает отрицательные и float-показатели.

**Управление:** `if/elif/else`, `while` с `break`/`continue`/`pass`,
`for` по любому итерируемому объекту, `def` с параметрами по умолчанию,
замыкания, рекурсия (лимит 56 Python-кадров, затем `RecursionError`),
`class` с наследованием, `try/except/else/finally`, `raise`, `assert`,
`global`/`nonlocal`.

**Классы:** собственные `__init__`, методы, атрибуты экземпляра и класса,
одиночное и множественное наследование (обход баз слева направо, без C3),
`isinstance`, `type`, а также `__str__`, `__repr__`, `__len__`, `__iter__`,
`__getitem__`, `__setitem__`. Перегрузки арифметических операторов и
`super()` нет.

**Исключения:** иерархия встроенных классов (`BaseException`, `Exception`,
`ArithmeticError`, `ZeroDivisionError`, `OverflowError`, `LookupError`,
`IndexError`, `KeyError`, `RuntimeError`, `NotImplementedError`,
`RecursionError`, `ValueError`, `TypeError`, `NameError`, `AttributeError`,
`OSError`, `FileNotFoundError`, `ImportError`, `ModuleNotFoundError`,
`StopIteration`, `SyntaxError`, `IndentationError`, `AssertionError`,
`MemoryError`, `SystemExit`), пользовательские подклассы, `except ... as e`,
`else`, `finally` и traceback с номерами строк.

**Модули:** `import a`, `import a.b`, `from a import b`, `from a import *`;
поиск в каталоге скрипта, текущем каталоге, `/mnt` и `/`; кеш модулей.
Встроенный `sys` даёт `argv`, `path`, `version`, `platform`, `maxsize` и
`float_info` (`max`, `min`, `epsilon`, `dig`, `mant_dig`, ...). Встроенный
`math` даёт `pi`, `e`, `inf`, `nan`, `sqrt`, `floor`, `ceil`, `trunc`, `fabs`,
`pow`, `fmod`, `sin`, `cos`, `tan`, `log` (включая `log(x, base)`), `log10`,
`exp`, `isnan`, `isinf`.

**Встроенные функции:** `print` (`sep`, `end`, `file`), `len`, `range`,
`int` (включая `base` и усечение float), `float` (int/bool/str/float), `str`,
`abs` (int и float), `min`, `max`, `sum`, `sorted` (`key`, `reverse`),
`enumerate`, `zip`, `list`, `dict`, `tuple`, `type`, `isinstance` (класс или
кортеж классов), `open`, `chr`, `ord`, `repr`, `bool`, `iter`, `next`, `any`,
`all`, `divmod` (int и float), `pow` (включая трёхаргументный по модулю),
`round` (`ndigits`, ties-to-even), `hex`, `id`.

**Методы `str`:** `split` (`sep`, `maxsplit`), `join`, `strip`/`lstrip`/`rstrip`,
`upper`, `lower`, `replace`, `find`, `index`, `startswith`/`endswith`
(включая кортеж префиксов), `count`, `isdigit`, `title`, `splitlines`.

**Методы `list`:** `append`, `extend`, `pop`, `sort` (`key`, `reverse`),
`insert`, `remove`, `index`, `reverse`, `clear`, `copy`.

**Методы `dict`:** `get`, `keys`, `values`, `items`, `pop`, `update`,
`setdefault`, `clear`, `copy`.

**Файлы:** `open(path, mode)` для текстовых режимов `r`, `w`, `a` и их `+`
вариантов; `read`, `readline`, `readlines`, `write`, `writelines`, `close`,
`flush`, итерация по строкам (`for line in f`). Кодировка — UTF-8; данные
буферизуются в памяти и сбрасываются на диск при `close()` или уничтожении
объекта. Бинарный режим и `seek`/`tell` не поддерживаются.

## Ограничения

- **Float — f64, но не побитово CPython.** `str`/`repr` используют самый
  короткий round-trip формат Rust (`{:?}`) с Python-написанием экспоненты
  (`1e+20`, `1e-07`); на 500 случайных значениях он совпал с CPython. `sqrt`
  корректно округлён (целочисленный алгоритм). `exp`, `log`, `sin`, `cos`,
  `tan` — собственные полиномы без `libm`, поэтому могут отличаться от CPython
  на 1–2 ulp; `round(float, ndigits)` для очень больших величин и
  отрицательных `ndigits` — до 1 ulp. Для `|x| > 1e15` аргумент
  тригонометрических функций редуцируется через `fmod`, поэтому точность на
  больших аргументах хуже, чем у CPython. Decimal/fractions/complex нет.
- **Float → int ограничен i64.** `int(float)`, `math.floor`/`ceil`/`trunc` и
  `round(float)` возвращают `OverflowError` за пределами 64-битного диапазона;
  CPython в этих случаях вернул бы произвольное целое.
- **Зависимость от FPU.** f64-инструкции требуют, чтобы ядро оставляло
  `sstatus.FS` включённым при переходе в U-mode. На 0.14.0 это проверено;
  ядро с выключенным FPU уронит первую float-операцию по illegal instruction.
- **Нет `set`, `bytes`, `complex`, `decimal`, `fractions`.**
- **Нет CPython-стандартной библиотеки.** Нет `os`, `json`, `re`, `random`,
  `time`, `pathlib`, `typing` и т.д. Есть `sys`, `math` и локальные
  `.py`-модули.
- **Нет потоков, процессов, сигналов, сокетов и сети.** ABI ядра этого не
  даёт; `threading`, `multiprocessing`, `socket`, `subprocess` отсутствуют.
- **Нет `input()`** (консольный ввод ядра не выдаёт EOF) и нет `sys.stdin`.
- **Парсер не поддерживает** `lambda`, `with`, `del`, `yield`/генераторы,
  f-strings, bytes-литералы, set-литералы, dict/set comprehensions (list
  comprehensions поддерживаются), `*args`/`**kwargs`, присваивание срезу,
  относительные импорты, `for/else` и `while/else`.
- **Нет перегрузки операторов** (`__add__`, `__eq__`, ...) и `super()`;
  равенство пользовательских экземпляров — идентичность.
- **Файловый I/O текстовый и простой:** нет `seek`/`tell`, бинарного режима,
  `encoding` кроме UTF-8; `buffering` принимается, но игнорируется.
- **Память и стек ограничены:** bump-heap 1 МиБ не освобождается, поэтому
  большие строки/списки и долгие циклы с накоплением данных упрутся в
  `MemoryError` или закончатся раньше. Рекурсия ограничена 56 кадрами.
- `str.isdigit()` в этой реализации проверяет ASCII-цифры, а не все Unicode
  `Nd`.

## Тесты

Хостовый harness собирает те же `src/*.rs`, подменяя только `platform.rs` на
`std`-версию:

```sh
cd hostcheck
cargo test                 # 12 unit-тестов интерпретатора
cargo run -- --selftest    # те же проверки с PASS/FAIL-строками
cargo run -- ../tests/live/hello.py
```

`hostcheck --parse file.py` и `--tokens file.py` проверяют только фронтенд.
Живой QEMU-прогон (21 PASS, включая шесть float-скриптов) и его PASS-лист:
[`docs/live-testing.md`](docs/live-testing.md).

## Устройство

```text
src/lexer.rs      токены, отступы, строковые и числовые литералы
src/parser.rs     рекурсивный спуск, AST
src/ast.rs        узлы AST
src/value.rs      значения, объектная модель, dict, итераторы, repr
src/float.rs      f64: парсинг/формат, sqrt, exp, log, trig, round без libm
src/interp.rs     вычисление AST, области видимости, классы, исключения, импорты
src/builtins.rs   встроенные функции и методы str/list/dict/file
src/platform.rs   единственный слой вызовов libpagh (файлы, stdout, argv, exit)
src/main.rs       CLI, диагностика, коды возврата
hostcheck/        std-обвязка для тестов на хосте
tests/live/       скрипты и драйвер живого QEMU-прогона
```
