# pagh-python — нативный интерпретатор Python для pagh-riscv

Собственная реализация Python (не CPython) на Rust поверх `libpagh`:
лексер → indentation-aware парсер → AST → tree-walking интерпретатор.
Собирается в статический freestanding ELF для `riscv64gc-unknown-none-elf`
и запускается в U-mode:

```text
pagh:rv64> run /mnt/pythonc /mnt/script.py
```

## Статус

**Рабочая версия 0.7.0.** Язык и встроенные функции реализованы настолько,
насколько это нужно для небольших скриптов на 1 MiB bump-heap ядра pagh.

Новое в 0.7.0 — полный протокол генератора поверх тех же файберов:

- **`send(value)`, `throw(exc)`, `close()`.** Генератор стал корутиной.
  `send(v)` резюмирует приостановленный `yield` так, что выражение `yield`
  даёт `v`; `send(None)` — это `next()`, а не-`None` в ещё не запущенный
  генератор — `TypeError` (в исчерпанный — `StopIteration`). `throw`
  поднимает исключение в точке `yield`; формы — экземпляр, класс, класс +
  аргументы (кортеж распаковывается), трёхаргументная форма CPython с
  traceback не поддерживается и отвергается `TypeError`. `close()` поднимает
  `GeneratorExit` в точке `yield`, поэтому выполняются `finally`; тело,
  которое после этого снова выдало значение, получает
  `RuntimeError: generator ignored GeneratorExit`. `GeneratorExit` наследует
  `BaseException`, как в CPython, поэтому `except Exception` его не ловит.
- **`yield from` делегирует `send`/`throw`/`close`** и вкладывается цепочкой:
  `send` уходит в подгенератор, `throw` поднимается в нём, `close` сначала
  закрывает подгенератор, а затем поднимает `GeneratorExit` во внешнем.
  Выражение `yield from` теперь даёт значение `return` подгенератора (`None`
  для обычного итератора). Для не-генератора делегация сужена и
  задокументирована: `send` не-`None` — `AttributeError`, `throw` и `close`
  поднимаются в точке `yield from`.
- **Лимиты не изменились:** генераторы по-прежнему файберы на 64 КиБ,
  одновременно приостановленных не больше 8 (девятый — `MemoryError`), а
  стек и кадр завершённого/закрытого генератора возвращаются в пулы — цикл
  на 200 генераторов с `send`/`close` проходит.

Живая проверка 0.7.0 — на ядре **0.21.2** (`pagh-riscv`, коммит `a0933ee`,
QEMU `virt`, ext2 64 MiB, драйвер `tests/live/qemu_driver.py`, одна
загрузка): полный suite [`tests/live/live07.cmds`](tests/live/live07.cmds) —
42 запуска, 36 с кодом 0, шесть намеренных диагностик (usage — код 2;
`syntax_error.py`, `uncaught.py`, `generator_syntax.py`,
`decorator_syntax.py`, `yield_from_syntax.py` — код 1), и две новые проверки:
[`tests/live/gen_methods.py`](tests/live/gen_methods.py) печатает
`GEN METHODS OK` (`send`/`throw`/`close`, иерархия `GeneratorExit`),
[`tests/live/gen_delegation.py`](tests/live/gen_delegation.py) — 
`GEN DELEGATION OK` (делегация трёх методов, значение `return`, лимит 8
приостановленных генераторов и рецикл стеков). Прежние маркеры
`YIELD FROM OK`, `GENERATORS OK` и `child: True status: 0`/`proc ok` на месте.
Сырой транскрипт:
[`docs/live-pythonc-070.txt`](docs/live-pythonc-070.txt), PASS-лист и детали —
[`docs/live-testing.md`](docs/live-testing.md).

Размер release-ELF — 553 384 байта (541 КиБ; лимит ядра — 1 МиБ): профиль
`release` теперь использует `strip = "symbols"`, потому что одна таблица
символов занимала ~540 КиБ из 1085 КиБ файла 0.4.0. Потоков, сигналов, сети и
динамического загрузчика в языке нет. Интерпретатор исполняет настоящие
f64-инструкции в U-mode, поэтому ядро обязано оставлять `sstatus.FS`
включённым при `sret`.

## Сборка

Интерпретатор зависит от `libpagh` — соседнего чекаута
[`pagh-api`](https://github.com/ccclosed/pagh-api), как и `pagh-rust`:
зависимость и linker script заданы путём `../pagh-api`, поэтому репозитории
должны лежать рядом:

```text
<parent>/pagh-api
<parent>/pagh-python
```

Тулчейн пинован в `rust-toolchain.toml` (nightly-2026-07-12 с целью
`riscv64gc-unknown-none-elf`), а `.cargo/config.toml` уже выбирает эту цель:

```sh
cargo build --release
```

`target/riscv64gc-unknown-none-elf/release/pythonc` — готовый ELF. Ядро
принимает образы не больше 1 МиБ; `panic = "abort"`, `strip = "debuginfo"` и
`overflow-checks = false` уже заданы в `Cargo.toml` (переполнения проверяются
вручную и превращаются в Python `OverflowError`).

## Запуск в QEMU

Один прогон suite 0.7.0 (сборка, образ, QEMU, транскрипт) — одной командой:

```sh
sh tests/live/live07.sh
```

Вручную:

```sh
rm -rf /tmp/py-rootfs && mkdir /tmp/py-rootfs
cp target/riscv64gc-unknown-none-elf/release/pythonc tests/live/*.py /tmp/py-rootfs/
mke2fs -q -t ext2 -F -b 4096 -d /tmp/py-rootfs /tmp/py-live.img 64M

python3 tests/live/qemu_driver.py \
  --disk /tmp/py-live.img \
  --commands tests/live/live07.cmds \
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
Тело — одно выражение, как в Python; `yield` внутри `lambda` отвергается
парсером с отдельным сообщением.

**Генераторы:** генераторной становится любая функция, в теле которой есть
`yield` (вложенные `def` считаются отдельно; `yield` в `lambda` и в
comprehension отвергается парсером). Вызов генераторной функции связывает
параметры (ошибка арности — сразу `TypeError`) и возвращает генератор; тело
не исполняется до первого `next()`. Поддерживаются `yield value`, `yield` без
значения, `yield a, b` (кортеж), `x = yield value` (значение приходит из
`send`), `next(gen)`, `next(gen, default)`, `send(value)`, `throw(exc)`,
`close()`, `StopIteration`, `for`/`in`/`list()`/`sum()` по генератору,
`finally` при исчерпании и при `close()`, распространение исключения из тела
в вызывающий `next()`/`throw()`, вложенные генераторы (внешний резюмирует
внутренний), `iter(gen) is gen` и `yield from` для любого iterable
(делегация `send`/`throw`/`close` и значение `return` — см. 0.7.0).

Точные формы протокола (0.7.0): `send(None)` = `next()`, `send(v)` в ещё не
запущенный генератор — `TypeError` (`can't send non-None value to a
just-started generator`), в исчерпанный — `StopIteration`; `throw(instance)`,
`throw(class)`, `throw(class, value)` (экземпляр класса используется как
есть, кортеж распаковывается в аргументы) — необработанное исключение
выходит из `throw`, а если тело после обработки завершилось, `throw` даёт
`StopIteration`; `throw` в незапущенный генератор тело не исполняет, а в
исчерпанный — поднимает переданное исключение в вызывающем (поведение
CPython 3.13+); `close()` возвращает `None`, `GeneratorExit` можно поймать,
но если тело после этого снова выдаст значение — `RuntimeError`. Значение
`return` самого генератора доступно только как значение `yield from`, но не
через `StopIteration.value` (у исключений этого интерпретатора нет атрибута
`value`); повторный `next()` после исключения даёт `StopIteration`.

**Generator expressions:** `(element for target in iterable if cond ...)` —
ленивый генератор, а не список: поддерживаются несколько `for`/`if`, захват
объемлющей области видимости и голый вид как единственный аргумент вызова
(`sum(x * x for x in values)`); повторный проход по тому же объекту пуст, как
в Python.

**Декораторы:** `@expr` перед `def` или `class`, несколько подряд (снизу
вверх); декоратор — любой вызываемый объект, имя привязывается к его
результату (функция, обёртка, значение или класс). Поддерживается
`@staticmethod` — обёртка, которую `getattr` не привязывает к экземпляру,
поэтому `Tools.add(1, 2)` и `Tools().add(1, 2)` вызывают функцию без `self`.
Декорировать можно и генераторную функцию: генератором она быть не перестаёт.

**`%`-форматирование:** `"%s and %d" % (name, count)`; конверсии `s r a d i u
f F e E g G x X o b c %`, флаги `-`, `+`, пробел, `#`, `0`, ширина и точность
(`%5d`, `%-5d`, `%05d`, `%.2f`, `%8.3f`), аргумент-кортеж (несколько
значений) или одиночное значение (в том числе список), `%%` как литерал.
Ошибки как в CPython: `TypeError` при нехватке или лишних аргументах и
`ValueError` на неизвестной конверсии.

**Модули стандартной библиотеки:** `sys`, `math`, `os` (файлы, каталоги,
`stat`, `environ`, процессы), `os.path`, `time`, `random`, `json`, плюс
локальные `.py`-модули и импорт по точечному имени. `time.time()` и
`time.monotonic()` читают два клока ABI (0 — realtime/uptime, 1 — монотонный;
разрешение 1 мс), `time.sleep()` вызывает `nanosleep` (округление вверх до
10 мс тика). `random` — детерминированный xorshift64* с фиксированным seed по
умолчанию, поэтому один и тот же скрипт печатает одни и те же числа на хосте
и в госте.

**Управление:** `if/elif/else`, `while` с `break`/`continue`/`pass`,
`for` по любому итерируемому объекту (включая генераторы и generator
expressions), `def` с параметрами по умолчанию, замыкания, рекурсия (лимит
56 Python-кадров, затем `RecursionError`; внутри генератора раньше
срабатывает лимит его стека — тоже `RecursionError`),
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
  `os.path`, `time`, `random`, `json` и локальные `.py`-модули. Нет `re`,
  `pathlib`, `typing`, `collections`, `datetime`, `subprocess` и т.д.
  `os.path` — только `join`/`basename`/`dirname`/`exists`/`isfile`/`isdir` и
  `sep` (нет `abspath`, `normpath`, `splitext`, `isdir('')`-краевых случаев
  Windows-путей); `join` не сворачивает `..` и повторные слэши. `time` — три
  функции, нет `strftime`/`localtime`/`perf_counter`/`time_ns`; разрешение
  часов 1 мс, `sleep` округляет вверх до 10 мс тика и не прерывается
  сигналом. `random` — не криптографический xorshift64*, нет `randrange`,
  `sample`, `choices`, `gauss`, класса `Random` и `seed(str)`; по умолчанию
  seed фиксирован, поэтому последовательность одинакова в каждом запуске.
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
- **Парсер не поддерживает** `yield` внутри `lambda` и
  comprehension, относительные импорты, keyword-only параметры после `*args`
  и голый `*`, `match`, walrus `:=`, `async`/`await`, аннотации типов,
  вложенные кавычки в f-string выражениях, вложенные replacement fields в
  `str.format`, встроенный `format()`. `yield` вне функции и `@decorator`
  перед не-определением — `SyntaxError` с отдельным сообщением.
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
- **Генераторы — файберы, и это видно.** Стек 64 КиБ выделяется при первом
  `next()` (генератор, который только создали, ничего не стоит), одновременно
  «приостановленных» генераторов не больше 8 — на девятом `MemoryError`
  (`cannot start a generator: 8 are already suspended`); стек возвращается в
  пул, как только тело завершилось, поэтому цикл на 200 генераторов работает.
  Рекурсия внутри генератора ограничена его стеком: на ядре 0.19.0
  `tests/live/stack_depth.py` доходит до 10 вложенных вызовов и на следующем
  получает `RecursionError: generator stack exhausted (… bytes left of 64 KiB)`
  — чистую ошибку вместо порчи кучи. Сборщика циклов
  нет: генератор, который приостановили и не исчерпали, оставаясь при этом
  достижимым из собственной области видимости (например `g = counter(10)` на
  уровне модуля), держит свой слот до конца процесса — `del g`, `close()` или
  исчерпание (`list(g)`) его освобождают; `finally` такого брошенного
  генератора не исполняется. `send`/`throw`/`close` есть (0.7.0), но с
  задокументированными сужениями: у `throw` нет трёхаргументной формы
  `(type, value, traceback)`; `PEP 479` не реализован — необработанный
  `StopIteration` (например, из `throw(StopIteration)`) завершает генератор и
  выходит как `StopIteration`, а не превращается в `RuntimeError`; через
  `yield from` делегируется только pagh-генератор — для обычного итератора
  `send` не-`None` даёт `AttributeError`, а `throw`/`close` поднимаются в
  точке `yield from`; значение `return` видно только как значение `yield
  from`, но не как `StopIteration.value`; у генератора нет `gi_frame`/
  `gi_code`/`gi_yieldfrom` и метода `__next__` (только `send`/`throw`/
  `close`); генератор нельзя скопировать или сериализовать.
- **Рекурсия упирается в стек, а не в константу.** `RECURSION_LIMIT = 56`
  кадров недостижим в этой сборке: Rust-кадры интерпретатора стоят несколько
  КиБ на Python-вызов, поэтому 64 КиБ стека процесса хватает примерно на
  двенадцать вложенных вызовов (`fib(10)` из `tests/live/functions.py`
  проходит у самого края). Глубже ядро убивает процесс по store page fault
  (`[user] /mnt/pythonc killed: store page fault`), а не сообщает
  `RecursionError` — это унаследованное ограничение 0.4.0, в 0.5.0 оно не
  менялось; проверка стека есть только у файберов генераторов, потому что там
  переполнение испортило бы кучу интерпретатора. Полноценное лечение —
  уменьшить кадры интерпретатора или исполнять модуль на большом файбере.
- **Декораторы:** кроме `staticmethod` нет `classmethod`, `property`,
  `functools.wraps`/`lru_cache` и дескрипторного протокола; декоратор
  вызывается как обычная функция (без `__set_name__` и т.п.).
- **`%`-форматирование:** нет словарной формы `%(name)s`, звёздочной ширины
  `%*d` и `%.*f`; `%a` печатает `repr` (встроенного `ascii()` нет), `%b` и
  `%u` приняты как расширения (`%u` = `%d`, `%b` — двоичный, как в f-string).
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
cargo test                 # 35 unit-тестов интерпретатора
cargo run -- --selftest    # те же проверки с PASS/FAIL-строками (35 PASS)
cargo run -- ../tests/live/hello.py
```

Проверки 0.7.0: `generator send` (значение в точке `yield`, `send(None)`,
`TypeError` в незапущенный генератор, `StopIteration` в исчерпанный,
повторный вход, generator expression), `generator throw` (экземпляр, класс,
класс + аргументы, необработанное исключение, незапущенный и исчерпанный
генератор, arity/типы), `generator close` (`finally`, `close()` незапущенного,
`RuntimeError` на «игнорирующем» теле, иерархия `GeneratorExit`, повторный
вход) и `generator delegation` (`send`/`throw`/`close` через `yield from`,
значение `return`, вложенность, сужение для обычного итератора) — плюс 31
проверка 0.6.0 (`fibers`, `generators`, `yield from`, `generator
expressions`, `decorators`, `percent format`, `os.path`, `time`, `random` и
прежние).

`hostcheck --parse file.py` и `--tokens file.py` проверяют только фронтенд.
`hostcheck` не умеет `os.fork`/`execv`/`waitpid` (у него нет libpagh); эти
вызовы живьём проверены в QEMU (`tests/live/os_proc.py`).

Живой QEMU-прогон suite 0.1.2 (30 PASS), отдельные прогоны `*args`/`**kwargs`
и `str.format` (0.4.0), прогон 0.5.0 на ядре 0.19.0, полный прогон 0.6.0 на
ядре 0.21.0 (40 запусков, 34 PASS, 6 намеренных диагностик; `YIELD FROM OK`)
и прогон 0.7.0 на ядре 0.21.2 (42 запуска, 36 PASS, 6 намеренных
диагностик; `GEN METHODS OK`, `GEN DELEGATION OK`) — PASS-листы и сырые
логи: [`docs/live-testing.md`](docs/live-testing.md), транскрипты 0.5.0,
0.6.0 и 0.7.0 — [`docs/live-pythonc-050.txt`](docs/live-pythonc-050.txt),
[`docs/live-pythonc-060.txt`](docs/live-pythonc-060.txt) и
[`docs/live-pythonc-070.txt`](docs/live-pythonc-070.txt), скрипты —
[`tests/live/`](tests/live/).

## CI

`.github/workflows/ci.yml` чекаутит соседний `pagh-api` (main), ставит
тулчейн из `rust-toolchain.toml` и гоняет два гейта: `cargo test` в
`hostcheck` (35 проверок) и `cargo build --release` для
`riscv64gc-unknown-none-elf` с проверкой, что `pythonc` не превышает лимит
ядра в 1 МиБ. Живой QEMU-прогон в CI намеренно не входит: для него нужны
ядро и QEMU, а артефакт приёмки — транскрипт `tests/live/live07.sh`.

## Устройство

```text
src/lexer.rs      токены, отступы, строковые/bytes/f-string литералы
src/parser.rs     рекурсивный спуск, AST, lambda/with/comprehensions
src/ast.rs        узлы AST
src/value.rs      значения, объектная модель, dict/set, итераторы, repr
src/fiber.rs      файберы генераторов: переключение стека (asm), пул стеков
src/float.rs      f64: парсинг/формат, sqrt, exp, log, trig, round без libm
src/json.rs       json.dumps/loads, escaping и рекурсивный разбор
src/interp.rs     вычисление AST, области видимости, классы, with, импорты
src/builtins.rs   встроенные функции, методы str/list/dict/set/bytes/file, os, json
src/platform.rs   единственный слой вызовов libpagh (файлы, stdout, argv, os, процессы)
src/main.rs       CLI, диагностика, коды возврата
hostcheck/        std-обвязка для тестов на хосте
tests/live/       скрипты и драйвер живого QEMU-прогона
```
