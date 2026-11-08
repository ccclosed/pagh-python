# pagh-python — нативный интерпретатор Python для pagh-riscv

Собственная реализация Python (не CPython) на Rust поверх `libpagh`:
лексер → indentation-aware парсер → AST → tree-walking интерпретатор.
Собирается в статический freestanding ELF для `riscv64gc-unknown-none-elf`
и запускается в U-mode:

```text
pagh:rv64> run /mnt/pythonc /mnt/script.py
```

## Статус

**Рабочая версия 0.4.0.** Язык и встроенные функции реализованы настолько,
насколько это нужно для небольших скриптов на 1 MiB bump-heap ядра pagh.
Полный suite 0.1.2 (30 PASS из 31 команды: float-арифметика и `math`, классы
с наследованием, исключения, файловый ввод-вывод, импорт локального модуля,
диагностика ошибок, lambda/`sorted`/`map`, `with`, `set`, `bytes` + бинарные
файлы, f-strings, `os`, `json`, `os.fork`/`os.execv`/`os.waitpid`) сохранён в
[`docs/live-pythonc.txt`](docs/live-pythonc.txt). Новые возможности 0.4.0 —
`*args`/`**kwargs` и `str.format` — проверены отдельными живыми прогонами на
ядре **0.16.2** скриптами `tests/live/star_args.py` и
`tests/live/str_format.py`: оба завершились с кодом 0 и напечатали
`STARARGS OK` / `FORMAT OK`; сырые логи —
[`docs/live-pythonc-starargs.txt`](docs/live-pythonc-starargs.txt) и
[`docs/live-pythonc-strformat.txt`](docs/live-pythonc-strformat.txt). PASS-лист
и детали — [`docs/live-testing.md`](docs/live-testing.md).

Размер release-ELF — 1 025 256 байт (≈1001 КиБ; лимит ядра — 1 МиБ). Потоков,
сигналов, сети и динамического загрузчика в языке нет; процессы (`fork`/
`execv`/`waitpid`) доступны через модуль `os` и проверены живьём.
Интерпретатор исполняет настоящие f64-инструкции в U-mode, поэтому ядро
обязано оставлять `sstatus.FS` включённым при `sret`.

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
`str` (UTF-8), `bytes` (неизменяемые байты), `list`, `tuple`, `dict`
(с сохранением порядка вставки), `set`, `range`, файловые объекты,
функции (включая `lambda`), классы и экземпляры.

**Синтаксис и операторы:** целые и float-литералы (`1.5`, `.5`, `1e3`,
`1.5e-2`, `1_0.5`), `bytes`-литералы (`b"..."`, `b"\x41"`, raw `rb"..."`),
set-литералы (`{1, 2, 3}`), имена, `+ - * / // % **` (включая унарные),
сравнения (`== != < <= > >= in not in is is not`), цепочки сравнений,
`and/or/not`, тернарное `a if cond else b`, срезы `a[start:stop:step]` для
`str`/`bytes`/`list`/`tuple`, индексация, атрибуты, вызовы с позиционными и
именованными аргументами, а также распаковкой `*iterable` и `**mapping` на
месте вызова. Присваивания (включая цепочки и `+=` и т.п.)
поддерживаются. `/` всегда возвращает float (`1 / 2 == 0.5`), `//` сохраняет
тип (int для int, float для float), `%` следует знаку делителя, `**`
поддерживает отрицательные и float-показатели. `bytes` сравниваются
лексикографически и поддерживают `+`, `*` и `in`; `set` — `|`-подобные
операции через методы и сравнения подмножества (`<`, `<=`, `>`, `>=`).

**f-strings:** `f"text {expr} text"`, выражения, `!r`/`!s`/`!a`,
спецификаторы `:d`, `:f`, `:e`, `:g`, `:x`, `:X`, `:b`, `:o`, `:%`, `:s`,
ширина/выравнивание/точность (`{value:.2f}`, `{value:>5d}`, `{value:08.3f}`),
`{{`/`}}`, неявная конкатенация f-строк. Вложенные кавычки внутри `{...}`,
обратные слэши в выражениях и вложенные replacement fields в спецификаторе
не поддерживаются (лексер сообщает `SyntaxError`). Встроенного `format()`
нет, но `str.format` поддерживается (см. ниже).

**`*args` / `**kwargs`:** `def f(a, b=2, *args, **kwargs)` и
`lambda *args, **kwargs: ...`; `*args` собирает лишние позиционные аргументы
в tuple, `**kwargs` — неиспользованные именованные в dict (порядок
вставки сохраняется). На месте вызова поддерживаются `f(*iterable)` и
`f(**mapping)` в любом сочетании, включая несколько распаковок, а также
проброс `def g(*args, **kwargs): return f(*args, **kwargs)`. Параметры
keyword-only после `*args` (`def f(*args, x)`) и голый `*` не
поддерживаются; `*args`/`**kwargs` не могут иметь значений по умолчанию.

**lambda:** `lambda args: expr` с параметрами по умолчанию, `*args`/
`**kwargs`, замыканиями и использованием как значения функции
(`sorted(..., key=lambda ...)`, `map(lambda ..., ...)`, `list.sort(key=...)`).
Тело — одно выражение, как в Python; `yield` не поддерживается.

**Управление:** `if/elif/else`, `while` с `break`/`continue`/`pass`,
`for` по любому итерируемому объекту, `def` с параметрами по умолчанию,
замыкания, рекурсия (лимит 56 Python-кадров, затем `RecursionError`),
`class` с наследованием, `try/except/else/finally`, `raise`, `assert`,
`global`/`nonlocal`, `with ... as ...` (включая несколько менеджеров через
запятую).

**`with`:** протокол `__enter__`/`__exit__`; `__exit__` получает
`(exc_type, exc_value, traceback)` или `(None, None, None)`; возврат
истинного значения подавляет исключение; менеджеры входят слева направо и
выходят справа налево; `open()` поддерживает `with` (закрывает файл и не
подавляет исключения). Если `__enter__`/`__exit__` нет, поднимается
`AttributeError`.

**Классы:** собственные `__init__`, методы, атрибуты экземпляра и класса,
одиночное и множественное наследование (обход баз слева направо, без C3),
`isinstance`, `type`, а также `__str__`, `__repr__`, `__len__`, `__iter__`,
`__getitem__`, `__setitem__`. Контекстные менеджеры используют обычные
`__enter__`/`__exit__`. Перегрузки арифметических операторов и `super()`
нет.

**Исключения:** иерархия встроенных классов (`BaseException`, `Exception`,
`ArithmeticError`, `ZeroDivisionError`, `OverflowError`, `LookupError`,
`IndexError`, `KeyError`, `RuntimeError`, `NotImplementedError`,
`RecursionError`, `ValueError`, `UnicodeError`, `UnicodeDecodeError`,
`UnicodeEncodeError`, `JSONDecodeError`, `TypeError`, `NameError`,
`AttributeError`, `OSError`, `FileNotFoundError`, `FileExistsError`,
`ImportError`, `ModuleNotFoundError`, `StopIteration`, `SyntaxError`,
`IndentationError`, `AssertionError`, `MemoryError`, `SystemExit`),
пользовательские подклассы, `except ... as e`, `else`, `finally` и traceback
с номерами строк.

**Модули:** `import a`, `import a.b`, `from a import b`, `from a import *`;
поиск в каталоге скрипта, текущем каталоге, `/mnt` и `/`; кеш модулей.
Встроенный `sys` даёт `argv`, `path`, `version`, `platform`, `maxsize` и
`float_info` (`max`, `min`, `epsilon`, `dig`, `mant_dig`, ...).

Встроенный `math` даёт `pi`, `e`, `inf`, `nan`, `sqrt`, `floor`, `ceil`,
`trunc`, `fabs`, `pow`, `fmod`, `sin`, `cos`, `tan`, `log` (включая
`log(x, base)`), `log10`, `exp`, `isnan`, `isinf`.

Встроенный `os` даёт `getpid`, `getppid`, `getcwd`, `listdir`, `mkdir`,
`makedirs` (`exist_ok`), `remove`, `rmdir`, `rename`, `stat` (`st_size`,
`st_mode`, `st_kind`, `st_isfile`, `st_isdir`), `environ` (снимок окружения,
включая `PWD`), `sep`, `linesep`, а также процессы `fork`, `execv`,
`waitpid(pid, options)`, `wait()`; `waitpid` принимает `WNOHANG` только как
число `1`. `os.execv` заменяет образ процесса (успешный вызов не
возвращается) и требует абсолютный путь; `os.system` типизированно
поднимает `NotImplementedError`, потому что в pagh нет shell.

Встроенный `json` даёт `dumps` и `loads` для `None`/`bool`/`int`/`float`/
`str`/`list`/`tuple`/`dict`. Кодирование ASCII (`ensure_ascii=True`),
экранирование `"`, `\`, control-символов и не-ASCII как `\uXXXX`; `loads`
понимает `true/false/null`, числа, строки с `\uXXXX` и суррогатными парами;
ошибки — `JSONDecodeError` (подкласс `ValueError`). Опций `default`,
`sort_keys`, `indent` нет; `Infinity`/`NaN` печатаются как в CPython.

**Встроенные функции:** `print` (`sep`, `end`, `file`), `len`, `range`,
`int` (включая `base` и усечение float), `float` (int/bool/str/float), `str`,
`bytes` (`bytes()`, `bytes(iterable)`, `bytes(n)`, `bytes(str, "utf-8")`),
`abs` (int и float), `min`, `max`, `sum`, `sorted` (`key`, `reverse`),
`enumerate`, `zip`, `map` (энергичный, возвращает список), `list`, `dict`,
`tuple`, `set`, `type`, `isinstance` (класс или кортеж классов), `open`
(текстовые и бинарные режимы), `chr`, `ord`, `repr`, `bool`, `iter`, `next`,
`any`, `all`, `divmod` (int и float), `pow` (включая трёхаргументный по
модулю), `round` (`ndigits`, ties-to-even), `hex`, `id`.

**Методы `str`:** `split` (`sep`, `maxsplit`), `join`, `strip`/`lstrip`/
`rstrip`, `upper`, `lower`, `replace`, `find`, `index`, `startswith`/`endswith`
(включая кортеж префиксов), `count`, `isdigit`, `title`, `splitlines`,
`encode("utf-8")`.

**`str.format`:** `{}` (автонумерация), `{0}` (позиционный индекс), `{name}`
(именованное поле), цепочки `.attr` и `[index]`/`["key"]`, преобразования
`!r`/`!s`/`!a`, спецификаторы формата f-строк (`:d`, `:f`, `:>5`, `:.2f`,
`:04d`, ...), экранирование `{{` и `}}`; `"{name}".format(**d)` работает
через распаковку вызова. Смешивать автонумерацию и явные индексы нельзя
(`ValueError`, как в CPython). Вложенные replacement fields в спецификаторе
(`"{:{width}}"`) не поддерживаются.

**Методы `bytes`:** `decode("utf-8")`, `hex`; индексация/срезы/`len`,
`in`, сравнения, итерация по int, `bytes(iterable)`, `bytes(n)`,
`bytes(str, "utf-8")`, `+`, `*`, `str.encode`. `bytearray`/`memoryview` нет.

**Методы `set`:** `add`, `remove` (`KeyError`), `discard`, `union`,
`intersection`, `difference`, `issubset`, `issuperset`, `copy`, `clear`;
`in`, `len`, итерация, `==`, сравнения подмножества. Порядок элементов —
порядок вставки (реализация на `Vec`; CPython не гарантирует порядок set).

**Comprehensions:** list (`[x for x in ...]`), set (`{x for x in ...}`),
dict (`{k: v for k in ...}`), с несколькими `for` и `if`.

**Методы `list`:** `append`, `extend`, `pop`, `sort` (`key`, `reverse`),
`insert`, `remove`, `index`, `reverse`, `clear`, `copy`.

**Методы `dict`:** `get`, `keys`, `values`, `items`, `pop`, `update`,
`setdefault`, `clear`, `copy`.

**Файлы:** `open(path, mode)` для текстовых режимов `r`, `w`, `a` и их `+`
вариантов, а также бинарных `rb`, `wb`, `ab`/`r+b`. Текстовые `read`,
`readline`, `readlines`, `write`, `writelines`, `close`, `flush`, итерация по
строкам (`for line in f`); бинарные возвращают/принимают `bytes` и итерируют
строки как `bytes`. Кодировка текста — UTF-8; данные буферизуются в памяти и
сбрасываются на диск при `close()` или уничтожении объекта. `seek`/`tell`
нет.

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
  `sstatus.FS` включённым при переходе в U-mode. Ядро с выключенным FPU
  уронит первую float-операцию по illegal instruction.
- **Нет `complex`, `decimal`, `fractions`, `bytearray`, `memoryview`.**
- **Нет CPython-стандартной библиотеки.** Есть только `sys`, `math`, `os`,
  `json` и локальные `.py`-модули. Нет `re`, `random`, `time`, `pathlib`,
  `typing`, `os.path`, `collections` и т.д.
- **Нет потоков, сигналов и сети.** ABI ядра их даёт, но языковой слой их не
  выставляет: `threading`, `multiprocessing`, `socket`, `subprocess`
  отсутствуют. Процессы доступны только через `os.fork`/`os.execv`/
  `os.waitpid`; `os.system` требует shell, которого в pagh нет.
- **Процессы — MVP ядра.** `fork` глубоко копирует адресное пространство,
  `execve` требует абсолютный путь, лимит ядра — 6 процессов; `waitpid`
  возвращает сырой status (`code << 8` либо номер сигнала). `os.environ` —
  снимок; `putenv` в ABI нет, поэтому запись в dict не меняет окружение
  процесса.
- **`os.stat` каталога.** Ядро не даёт `fstat` каталога через обычный
  `open`, поэтому каталог определяется listdir-пробой и получает
  `st_size == 0`, `st_mode == 0o755`, `st_kind == "dir"`. Для файлов размер и
  режим настоящие.
- **`map` энергичный.** Он сразу возвращает список, а не ленивый итератор;
  значения и вызовы функции совпадают с Python, момент вычисления — нет.
- **Нет `input()`** (консольный ввод ядра не выдаёт EOF) и нет `sys.stdin`.
- **Парсер не поддерживает** `yield`/генераторы, generator expressions,
  декораторы, относительные импорты, keyword-only параметры после `*args`
  и голый `*`, вложенные кавычки в f-string выражениях, вложенные
  replacement fields в `str.format`, встроенный `format()`.
- **`*args`/`**kwargs`:** нет keyword-only параметров; `**mapping` принимает
  только `dict` (не произвольный mapping с `keys()`), а `f(*a, x=1)` и
  `f(x=1, *a)` допустимы, но аргументы вычисляются в порядке «сначала
  позиционные, затем именованные», а не строго в исходном порядке.
- **`del` и присваивание срезу работают**: `del name`, `del list[i]`,
  `del dict[key]`, `del obj.attr`, `del list[a:b]`, а также
  `list[a:b] = iterable` (шаг 1; extended-slice deletion/assignment
  отклоняется `ValueError`, как и в этой реализации).
- **Нет перегрузки операторов** (`__add__`, `__eq__`, ...) и `super()`;
  равенство пользовательских экземпляров — идентичность.
- **Файловый I/O простой:** нет `seek`/`tell`, кодировки кроме UTF-8;
  `buffering` принимается, но игнорируется; бинарный режим не декодирует
  данные, текстовый требует валидный UTF-8.
- **Память и стек ограничены:** bump-heap 1 МиБ не освобождается, поэтому
  большие строки/списки/set и долгие циклы с накоплением данных упрутся в
  `MemoryError` или закончатся раньше. Рекурсия ограничена 56 кадрами.
- `str.isdigit()` проверяет ASCII-цифры, а не все Unicode `Nd`.
- **JSON:** нет `default=`, `sort_keys`, `indent`, `cls`; словари
  сериализуются в порядке вставки; `NaN`/`Infinity` печатаются
  non-standard-нотами, как в CPython.
- **f-strings:** нет вложенных replacement fields (`f"{x:{width}}"`),
  вложенных кавычек и обратных слэшей в выражениях; `=`-отладка
  (`f"{x=}"`) не поддерживается.

## Тесты

Хостовый harness собирает те же `src/*.rs`, подменяя только `platform.rs` на
`std`-версию:

```sh
cd hostcheck
cargo test                 # 22 unit-теста интерпретатора
cargo run -- --selftest    # те же проверки с PASS/FAIL-строками
cargo run -- ../tests/live/hello.py
```

`hostcheck --parse file.py` и `--tokens file.py` проверяют только фронтенд.
`hostcheck` не умеет `os.fork`/`execv`/`waitpid` (у него нет libpagh); эти
вызовы живьём проверены в QEMU (`tests/live/os_proc.py`).

Живой QEMU-прогон полного suite 0.1.2 (30 PASS), отдельные живые прогоны
`*args`/`**kwargs` и `str.format` на 0.4.0, PASS-листы и сырые логи:
[`docs/live-testing.md`](docs/live-testing.md).

## Устройство

```text
src/lexer.rs      токены, отступы, строковые/bytes/f-string литералы
src/parser.rs     рекурсивный спуск, AST, lambda/with/comprehensions
src/ast.rs        узлы AST
src/value.rs      значения, объектная модель, dict/set, итераторы, repr
src/float.rs      f64: парсинг/формат, sqrt, exp, log, trig, round без libm
src/json.rs       json.dumps/loads, escaping и рекурсивный разбор
src/interp.rs     вычисление AST, области видимости, классы, with, импорты
src/builtins.rs   встроенные функции, методы str/list/dict/set/bytes/file, os, json
src/platform.rs   единственный слой вызовов libpagh (файлы, stdout, argv, os, процессы)
src/main.rs       CLI, диагностика, коды возврата
hostcheck/        std-обвязка для тестов на хосте
tests/live/       скрипты и драйвер живого QEMU-прогона
```
