# Живой прогон pythonc 0.1.2 на ядре 0.16.1

Дата: 2026-11-05. Ядро:
`/home/oleg/pagh-riscv/target/riscv64gc-unknown-none-elf/debug/pagh`
(0.16.1, native ABI, `fork`/`execve`/`wait4`). QEMU `qemu-system-riscv64`
10.2.2, машина `virt`, 256 MiB, serial на Unix-сокете, `-bios default`,
`-display none`. Диск: 64 MiB ext2, создан на хосте

```sh
mke2fs -q -t ext2 -F -b 4096 -d rootfs disk.img 64M
```

В корне образа — `pythonc` (release, 987 008 байт) и скрипты из
[`../tests/live/`](../tests/live/). Одна загрузка, 31 команда подряд; после
каждого `run` ядро печатает `run: /mnt/pythonc exited with code N`.

Сырой транскрипт: [`live-pythonc.txt`](live-pythonc.txt).

## Итог

**30 PASS из 31 команды.** Единственная не-PASS команда — `cat /mnt/hello.py`:
она просто показывает исходник и не является проверкой. Двадцать одна
прежняя проверка (hello, functions, collections, classes, exceptions,
file I/O, import, argv, `-c`, usage, syntax error, uncaught exception и семь
float-проверок) проходит без изменений. Добавлены девять новых скриптов:
lambda/`sorted`/`map`, `with open` и протокол контекстного менеджера, `set`,
`bytes`, бинарный файл, f-strings, `os`, `json` и `os.fork`/`os.execv`/
`os.waitpid`.

## PASS-строки

```text
PASS hello: run /mnt/pythonc /mnt/hello.py -> exit 0
PASS functions+recursion: run /mnt/pythonc /mnt/functions.py -> exit 0
PASS list/dict/str/slicing: run /mnt/pythonc /mnt/collections_demo.py -> exit 0
PASS class+inheritance: run /mnt/pythonc /mnt/classes.py -> exit 0
PASS try/except/finally: run /mnt/pythonc /mnt/exceptions.py -> exit 0
PASS file write/read: run /mnt/pythonc /mnt/fileio.py -> exit 0
PASS file persisted: cat /mnt/py.txt
PASS print(file=...): cat /mnt/print.txt
PASS local import: run /mnt/pythonc /mnt/import_main.py -> exit 0
PASS script args/sys.argv: run /mnt/pythonc /mnt/argv.py one two -> exit 0
PASS -c mode: run /mnt/pythonc -c print(6*7) -> 42, exit 0
PASS usage: run /mnt/pythonc -> exit 2
PASS syntax error: run /mnt/pythonc /mnt/syntax_error.py -> exit 1
PASS uncaught exception: run /mnt/pythonc /mnt/uncaught.py -> exit 1
PASS float arithmetic/literals: run /mnt/pythonc /mnt/float_arithmetic.py -> exit 0
PASS float division/floor/mod: run /mnt/pythonc /mnt/float_division.py -> exit 0
PASS float formatting/round: run /mnt/pythonc /mnt/float_format.py -> exit 0
PASS float comparison/NaN: run /mnt/pythonc /mnt/float_compare.py -> exit 0
PASS math module: run /mnt/pythonc /mnt/float_math.py -> exit 0
PASS typed float errors: run /mnt/pythonc /mnt/float_errors.py -> exit 0
PASS -c float: run /mnt/pythonc -c print(1/2) -> 0.5, exit 0
PASS lambda/defaults/map/sorted: run /mnt/pythonc /mnt/lambda_map.py -> exit 0
PASS with open/multiple/suppression: run /mnt/pythonc /mnt/with_open.py -> exit 0
PASS set literal/methods/in/eq: run /mnt/pythonc /mnt/sets_demo.py -> exit 0
PASS bytes literal/decode/encode: run /mnt/pythonc /mnt/bytes_demo.py -> exit 0
PASS binary file read/write: run /mnt/pythonc /mnt/bytes_file.py -> exit 0
PASS f-strings/specs/conversion: run /mnt/pythonc /mnt/fstrings_demo.py -> exit 0
PASS os listdir/mkdir/stat/rename/env: run /mnt/pythonc /mnt/os_demo.py -> exit 0
PASS json dumps/loads/escaping: run /mnt/pythonc /mnt/json_demo.py -> exit 0
PASS os.fork/execv/waitpid: run /mnt/pythonc /mnt/os_proc.py -> exit 0
```

## Что именно проверено

### Прежние проверки

Двенадцать прежних языковых проверок (hello, functions, collections,
classes, exceptions, file I/O, import, argv, `-c`, usage, syntax error,
uncaught exception) и семь float-проверок по-прежнему проходят. Все float-
скрипты завершаются `assert`-проверками, поэтому код возврата 0 означает
совпадение значений, а не только напечатанные строки.

### Lambda, defaults, sorted и map

```text
run /mnt/pythonc /mnt/lambda_map.py
lambda defaults: 15 16
lambda map: [2, 4, 6]
sorted key: ['os', 'pagh', 'interpreter']
sort key: [('a', 1), ('b', 2), ('c', 3)]
lambda ok
run: /mnt/pythonc exited with code 0
```

Проверены: `lambda` с параметрами по умолчанию, замыкание над локальной
переменной внешней функции (`make_adder`), `sorted(..., key=lambda ...)`,
`list.sort(key=lambda ...)` и `map` с несколькими итерируемыми.

### with и протокол контекстного менеджера

```text
run /mnt/pythonc /mnt/with_open.py
with file: alpha|beta|
enter first
enter second
body first second
exit second (None, None)
exit first (None, None)
events: [(None, None)] [(None, None)]
suppressed: ('ValueError', 'suppressed')
file exit propagates: not suppressed
with ok
run: /mnt/pythonc exited with code 0
```

Проверены: `with open(...) as f` (файл закрывается `__exit__`),
`f.closed == True` после блока, несколько менеджеров в одном `with`
(вход слева направо, выход справа налево), передача `(None, None, None)` в
`__exit__` при нормальном выходе, подавление исключения при
`__exit__ -> True` и проброс исключения, когда `open()`-файл возвращает
`None` из `__exit__`.

### set

```text
run /mnt/pythonc /mnt/sets_demo.py
literal: {3, 1, 2} len: 3 in: True
add/remove: [2, 3, 4]
union: [1, 2, 3, 4]
intersection: [3]
difference: [1, 2]
constructor: {1, 2, 3}
set comp: [0, 4, 16]
dict comp: {'0': 0, '1': 10, '2': 20}
iter 1
iter 3
iter 5
sets ok
run: /mnt/pythonc exited with code 0
```

Проверены литерал `{1, 2, 3}` с дедупликацией, `set(iterable)`, `len`,
`in`, `==` без учёта порядка, `add`, `remove`, `discard`, `union`,
`intersection`, `difference`, `issubset`, `issuperset`, итерация, `KeyError`
у `remove` и set/dict comprehensions. Порядок обхода — порядок вставки
(реализация на `Vec`, см. README).

### bytes и бинарные файлы

```text
run /mnt/pythonc /mnt/bytes_demo.py
literal: b'pagh' len: 4 index: 112 slice: b'ag'
escaped: b'line\nA\xff'
encode/decode: b'hello' hello
bytes(): b'ABC' b'\x00\x00'
hex: 414243
bytes ok
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/bytes_file.py
binary write: 7 bytes
binary read: b'\x00\x01pagh\xff' len: 7
binary file ok
run: /mnt/pythonc exited with code 0
```

Проверены литералы `b"..."` с `\n`, `\x41`, `\xff`, индексация (int),
срезы, `len`, конкатенация, повторение, сравнения, `in`, `bytes([65,66,67])`,
`bytes(n)`, `bytes(str, "utf-8")`, `str.encode`/`bytes.decode`, `hex`,
итерация по байтам и запись/чтение `open(..., "wb"/"rb")` с байтами
`\x00\x01pagh\xff`. В текстовом режиме данные по-прежнему обязаны быть
UTF-8; бинарный режим не декодирует содержимое.

### f-strings

```text
run /mnt/pythonc /mnt/fstrings_demo.py
hello pagh
pi=3.14 count=42
repr='pagh' width=   42
braces {ok}
fstrings ok
run: /mnt/pythonc exited with code 0
```

Проверены подстановки `{name}`, выражения `{2 + 3}`, спецификаторы
`:.2f`, `:d`, `:>5d`, `!r`, экранирование `{{`/`}}` и неявная конкатенация
f-строк. Вложенные кавычки внутри `{...}` намеренно не поддерживаются
(лексер сообщает об этом `SyntaxError`).

### os

```text
run /mnt/pythonc /mnt/os_demo.py
listdir: ['file.txt', 'nested']
stat: 8 file True False
rename: ['nested', 'renamed.txt']
cwd stat: dir True
pid: True True
cwd: /
os ok
run: /mnt/pythonc exited with code 0
```

Проверены `getpid`, `getppid`, `getcwd`, `listdir`, `mkdir`, `makedirs`,
`rename`, `remove`, `rmdir`, `stat` (size/kind/is_file/is_dir, включая
каталог), `environ["PWD"]` и `FileNotFoundError` после удаления. Каталог —
`/mnt/osdemo`, поэтому проверка работает на ext2.

### os.fork / os.execv / os.waitpid

```text
run /mnt/pythonc /mnt/os_proc.py
hello from pagh-python
arithmetic: 7 3 1 1024
builtins: 5 7 3 4
sep/end:-a-b-c.
child: True status: 0
proc ok
run: /mnt/pythonc exited with code 0
```

`os.fork()` создаёт ребёнка; ребёнок вызывает
`os.execv("/mnt/pythonc", ["/mnt/pythonc", "/mnt/hello.py"])`, поэтому
`execve` заменяет его образ и печатает hello. Родитель ждёт
`os.waitpid(pid, 0)` и получает status 0. Это живая проверка того, что
`fork`/`execv`/`waitpid` из `libpagh` действительно работают из
интерпретатора, а не только компилируются.

### json

```text
run /mnt/pythonc /mnt/json_demo.py
dumps: {"name": "pagh", "ok": true, "count": 3, "ratio": 1.5, "items": [1, 2, null, "x"]}
loads: {'name': 'pagh', 'ok': True, 'count': 3, 'ratio': 1.5, 'items': [1, 2, None, 'x']}
escaped: {"quote": "he said \"hi\"", "newline": "a\nb"}
json ok
run: /mnt/pythonc exited with code 0
```

Проверены `dumps`/`loads` для None/bool/int/float/str/list/dict, порядок
ключей, `true/false/null`, экранирование `"` и `\n`, round-trip,
`json.loads` чисел/строк/массивов/объектов и `JSONDecodeError` (подкласс
`ValueError`) на некорректном вводе. Не-ASCII символы кодируются как
`\uXXXX` (`ensure_ascii=True`, как в CPython по умолчанию).

## Диагностика ошибок

Синтаксическая ошибка — одна строка с файлом, строкой, колонкой и типом:

```text
run /mnt/pythonc /mnt/syntax_error.py
/mnt/syntax_error.py:2:5: SyntaxError: expected a parameter name, found 'return'
run: /mnt/pythonc exited with code 1
```

Непойманное исключение — Python-образный traceback с кадрами от модуля к
виновнику и кодом возврата 1:

```text
run /mnt/pythonc /mnt/uncaught.py
before
Traceback (most recent call last):
  File "/mnt/uncaught.py", line 8, in <module>
  File "/mnt/uncaught.py", line 5, in middle
  File "/mnt/uncaught.py", line 2, in divide
ZeroDivisionError: integer division or modulo by zero
run: /mnt/pythonc exited with code 1
```

## 0.4.0: `*args`/`**kwargs` и `str.format` (живьём, ядро 0.16.2)

Дата: 2026-11-08. Ядро:
`/home/oleg/pagh-riscv/target/riscv64gc-unknown-none-elf/debug/pagh`
(0.16.2), QEMU 10.2.2, `virt`, 256 MiB. Release-ELF `pythonc` 0.4.0 —
1 025 256 байт (лимит ядра 1 МиБ). Каждый прогон — отдельная загрузка;
логи сняты `tools/run.py` из `pagh-riscv` (репозиторий ядра не менялся):

```sh
rm -rf /tmp/py-rootfs && mkdir /tmp/py-rootfs
cp target/riscv64gc-unknown-none-elf/release/pythonc /tmp/py-rootfs/
cp tests/live/star_args.py /tmp/py-rootfs/test.py     # затем str_format.py
truncate -s 64M /tmp/py.img
mke2fs -q -t ext2 -F -b 4096 -d /tmp/py-rootfs /tmp/py.img

python3 tools/run.py --disk /tmp/py.img \
  --keys 'run /mnt/pythonc /mnt/test.py\nhalt\n' --settle 40 \
  --serial-log /tmp/py-serial.log --shot /tmp/py.png \
  --serial /tmp/s.sock --monitor /tmp/m.sock
```

`run.py` завершается с кодом 2 на попытке снять скриншот: `halt` уже
выключил QEMU, и monitor-сокет исчез. Это не сбой теста — serial-лог полон,
и именно он служит доказательством; обе команды `run` напечатали
`exited with code 0`.

**`*args`/`**kwargs` (`tests/live/star_args.py`)**

```text
pagh:rv64> run /mnt/pythonc /mnt/test.py
collect: ((1, 'two', True), {'name': 'pagh', 'n': 3})
mixed-default: (1, 20, (), {})
mixed-full: (1, 2, (3, 4), {'z': 5})
star-list: 30
star-dict: 70
both: 3
forward: (1, 2, (3, 4), {'z': 5})
lambda: ((7,), {'k': 8})
STARARGS OK
run: /mnt/pythonc exited with code 0
pagh:rv64> halt
halt: powering off
```

Сырой лог: [`live-pythonc-starargs.txt`](live-pythonc-starargs.txt).

**`str.format` (`tests/live/str_format.py`)**

```text
pagh:rv64> run /mnt/pythonc /mnt/test.py
auto: 2 + 3 = 5
manual: b/a/b
named: pagh-7
escape: {literal} { }
spec: 0007 3.142    ok
index: one
key: v
attr: 2,9
conversion: 'hi'
FORMAT OK
run: /mnt/pythonc exited with code 0
pagh:rv64> halt
halt: powering off
```

Сырой лог: [`live-pythonc-strformat.txt`](live-pythonc-strformat.txt).

Итог: **2 PASS из 2** новых живых проверок (плюс сохранённый suite 0.1.2
выше). Скрипты добавлены в `tests/live/live.cmds` для будущего полного
прогона.

## Что не проверялось живьём

- `os.system()`: в pagh нет shell-бинарника, поэтому функция типизированно
  поднимает `NotImplementedError`; см. README.
- Негативные пути `os.execv` (несуществующий файл, не-ELF) и `os.waitpid`
  с `WNOHANG`: на хосте они недоступны, живьём проверен успешный сценарий.
- Ошибки `bytes.decode` на невалидном UTF-8, `set` с нехешируемым
  элементом, синтаксические ошибки f-строк: они покрыты hostcheck-тестами,
  но не отдельными live-скриптами.
- Сеть, потоки, сигналы: в языке/ABI этого проекта их нет.
- Долгие программы и исчерпание 1 MiB heap: в тестах только небольшие
  скрипты.

## Воспроизведение

```sh
cargo build --release --target riscv64gc-unknown-none-elf
rm -rf /tmp/py-rootfs && mkdir /tmp/py-rootfs
cp target/riscv64gc-unknown-none-elf/release/pythonc tests/live/*.py /tmp/py-rootfs/
mke2fs -q -t ext2 -F -b 4096 -d /tmp/py-rootfs /tmp/py-live.img 64M
python3 tests/live/qemu_driver.py \
  --disk /tmp/py-live.img \
  --commands tests/live/live.cmds \
  --transcript docs/live-pythonc.txt
```

Драйвер поднимает QEMU с serial-сокетом, ждёт приглашение `pagh:rv64>`,
отправляет команды из `live.cmds` и сохраняет ответ ядра.

---

# Живой прогон pythonc 0.5.0 на ядре 0.19.0

Дата: 2026-11-09. Ядро:
`/home/oleg/pagh-riscv/target/riscv64gc-unknown-none-elf/debug/pagh`
(0.19.0), QEMU `qemu-system-riscv64` 10.2.2, машина `virt`, 256 MiB, serial на
Unix-сокете, драйвер — **`tools/run.py` самого ядра** (не
`tests/live/qemu_driver.py`). Диск: 64 MiB ext2,

```sh
truncate -s 64M /tmp/py-live-0.5.0.img
mke2fs -q -t ext2 -F -b 4096 -d /tmp/py-rootfs /tmp/py-live-0.5.0.img
```

а каждая команда отправлялась отдельной загрузкой:

```sh
python3 tools/run.py --disk /tmp/py-live-0.5.0.img \
  --keys "run /mnt/pythonc /mnt/generators.py\n" \
  --serial-log /tmp/live050-generators.txt --timeout 90
```

Сырой транскрипт (8 блоков, по блоку на загрузку):
[`live-pythonc-050.txt`](live-pythonc-050.txt). Скрипты —
[`../tests/live/`](../tests/live/), список команд —
[`../tests/live/live05.cmds`](../tests/live/live05.cmds), обёртка —
[`../tests/live/live05.sh`](../tests/live/live05.sh).

## Итог 0.5.0

| Скрипт | Что проверяет | Результат |
|---|---|---|
| `generators.py` | `yield`/`yield`-выражение, `next()`, `StopIteration`, `for/else`, ленивость, `finally`, вложенные генераторы, повторный вход, арность, рецикл стеков | `GENERATORS OK`, exit 0 |
| `genexpr.py` | generator expressions: ленивость, фильтры, вложенные `for`, голый вид в вызове, однократность | `GENEXPR OK`, exit 0 |
| `decorators.py` | `@dec` при определении, порядок снизу вверх, замена значения, метод, генераторная функция, `@staticmethod`, класс | `DECORATORS OK`, exit 0 |
| `percent_format.py` | `%s %r %d %f %e %x %X %o %b %c %%`, флаги/ширина/точность, три ошибки CPython | `PERCENT OK`, exit 0 |
| `stdlib_modules.py` | `os.path` (join/basename/dirname/exists/isfile/isdir/sep), `time` (time/monotonic/sleep), `random` (seed/random/randint/choice/shuffle/uniform) | `STDLIB OK`, exit 0 |
| `stack_depth.py` | предохранитель стека файбера: слишком глубокая рекурсия в генераторе — `RecursionError`, а не порча памяти | `recursion ok`, exit 0 |
| `generator_syntax.py` | `yield` вне функции — ошибка фронтенда | `SyntaxError: 'yield' outside a function…`, exit 1 |
| `decorator_syntax.py` | `@` перед не-определением — ошибка фронтенда | `SyntaxError: only a function or class definition can be decorated…`, exit 1 |

Ключевые строки серийного лога (полностью — в транскрипте):

```text
list: [0, 1, 2, 3]                     # generators.py
lazy: [0, 1, 2, 3, 4]
reentrant: generator already executing
recycled: 1200
GENERATORS OK

squares: [0, 1, 4, 9, 16]              # genexpr.py
lazy: [0, 2, 4, 6]
GENEXPR OK

order: value ['top', 'bottom']         # decorators.py
staticmethod: 3 7 10
DECORATORS OK

widths:    42|42   |00042              # percent_format.py
floats: 3.14|   3.142|+7
PERCENT OK

join: a/b/c /root/x /abs/b             # stdlib_modules.py
sleep: 0.04999999999999982 True
deterministic: True [0.8202466665411988, 0.9282901565044228, 0.08934959275158794]
STDLIB OK

max depth inside a generator: 10       # stack_depth.py
generator failure: generator stack exhausted (5408 bytes left of 64 KiB)
```

`random` детерминирован: те же три числа для `seed(7)` печатает и hostcheck на
хосте — xorshift64* не зависит от платформы.

## Регрессия старого suite на бинарнике 0.5.0

30 команд из [`live.cmds`](../tests/live/live.cmds) (0.1.2–0.4.0) прогнаны на
том же бинарнике 0.5.0: 27 из них завершились кодом 0, ожидаемые диагностики
— `run /mnt/pythonc` без аргументов (код 2), `syntax_error.py` (1) и
`uncaught.py` (1) — как и раньше. Исключение:

- `os_proc.py` (`os.fork`/`os.execv`/`os.waitpid`) на ядре 0.19.0 не печатает
  ничего даже за 240 с. Тот же скрипт с **бинарником 0.4.0**, собранным из
  предыдущего коммита, ведёт себя так же — то есть это расхождение ядра и
  ABI, а не регрессия 0.5.0; в отчёте о релизе это записано как непроверенное.

## Честные пробелы 0.5.0

- **Рекурсия на стеке процесса.** `RECURSION_LIMIT = 56` недостижим: кадры
  интерпретатора стоят несколько КиБ, 64 КиБ стека хватает примерно на
  двенадцать вложенных вызовов, глубже ядро убивает процесс по store page
  fault. Это унаследовано от 0.4.0 и в 0.5.0 не менялось; проверка стека есть
  только у генераторов (там переполнение испортило бы кучу).
- **Генераторы:** не более 8 одновременно приостановленных (у каждого стек
  64 КиБ), нет `send`/`close`/`throw`/`yield from`; брошенный
  (не исчерпанный) генератор, достижимый из своей же области видимости,
  держит слот до конца процесса, и его `finally` не исполняется.
- **`os_proc.py`** — см. выше: живьём на 0.19.0 не подтверждён ни одним из
  бинарников.
- Не проверялись живьём: `os.path` на путях со спецсимволами, `time.sleep`
  под нагрузкой планировщика, генераторы с исключением в `finally`,
  одновременные 8 генераторов (проверено только «6 из 8» и лимит).

---

# Живой прогон pythonc 0.6.0 на ядре 0.21.0

Дата: 2026-11-12. Ядро: `pagh-riscv` 0.21.0, debug ELF,
`sha256 abaeee3f76fe…`; QEMU `qemu-system-riscv64` 10.2.2, машина `virt`,
256 MiB, serial на Unix-сокете. Диск: 64 MiB ext2, собран на хосте

```sh
truncate -s 64M /tmp/py-live-0.6.0.img
mke2fs -q -t ext2 -F -b 4096 -d /tmp/py-rootfs /tmp/py-live-0.6.0.img
```

**Одна загрузка**, все команды подряд; драйвер — собственный
[`../tests/live/qemu_driver.py`](../tests/live/qemu_driver.py), обёртка —
[`../tests/live/live06.sh`](../tests/live/live06.sh), список команд —
[`../tests/live/live06.cmds`](../tests/live/live06.cmds). Сырой транскрипт:
[`live-pythonc-060.txt`](live-pythonc-060.txt).

## Итог 0.6.0

**40 запусков pythonc: 34 с кодом 0, 6 намеренных диагностик** (usage — 2;
`syntax_error.py`, `uncaught.py`, `generator_syntax.py`,
`decorator_syntax.py`, `yield_from_syntax.py` — 1), плюс три `cat`, которые
показывают staged-скрипт и два файла, записанных `fileio.py` и
`print(file=...)`. Все языковые проверки — assert-скрипты, поэтому код 0
означает совпадение значений, а не только напечатанные строки.

Новое в 0.6.0:

| Скрипт | Что проверяет | Результат |
|---|---|---|
| `yield_from.py` | `yield from` генератору, `range`, списку, generator expression и объекту с `__iter__`; ленивость, вложенность, `finally`, `TypeError` на не-iterable, однократное потребление, рецикл 200 стеков | `YIELD FROM OK`, exit 0 |

Закрытая находка 0.5.0:

- `os_proc.py` (`os.fork`/`os.execv`/`os.waitpid`) на ядре **0.19.0** не
  доходил до вывода ни одним бинарником (0.4.0 и 0.5.0) и был записан как
  расхождение ядра и ABI. На ядре **0.21.0** он проходит:
  `child: True status: 0`, `proc ok`, exit 0 — расхождение было в ядре, а не
  в интерпретаторе.

## Ключевые строки 0.6.0

```text
outer: ['before', 0, 1, 2, 'after']        # yield_from.py
range: [0, 1, 2] [7, 8] [0, 1, 4] ['a', 'b']
value: [1, 2, None]
lazy: [0, 1, 2, 3, 4]
finally: ['start', 1, 2, 'end'] ['cleanup']
non-iterable: TypeError True
recycled: 600
YIELD FROM OK

child: True status: 0                       # os_proc.py на 0.21.0
proc ok

max depth inside a generator: 10            # stack_depth.py
generator failure: generator stack exhausted (3872 bytes left of 64 KiB)
```

## Честные пробелы 0.6.0

- **`send`/`throw`/`close` у генераторов нет** (закрыто в 0.7.0), поэтому
  делегация проверена только по итерационному протоколу: `yield from` не
  передаёт значения обратно в подгенератор и не даёт его `return`-значение
  (выражение — `None`).
- **Глубина цепочки делегирования** больше 8 не проверялась: каждый уровень
  держит приостановленный файбер, лимит 0.5.0 (`не больше 8`) не менялся.
- Не проверялись живьём: `yield from` внутри `try/finally` самого внешнего
  генератора, делегирование объекту, чей `__iter__` возвращает не итератор
  (ошибка `TypeError` покрыта hostcheck).

---

# Живой прогон pythonc 0.7.0 на ядре 0.21.2

Дата: 2026-11-12. Ядро: `pagh-riscv` 0.21.2, коммит
`a0933ee` (`Merge pull request #8 from ccclosed/night/20261112-tls-stack`),
debug ELF, `sha256 2e8ae74637fae1d83445b843d887bfcc5ccc371ccd6174e16ff64afee88cc0b1`;
QEMU `qemu-system-riscv64` 10.2.2, машина `virt`, 256 MiB, serial на
Unix-сокете. Диск: 64 MiB ext2, собран на хосте

```sh
truncate -s 64M /tmp/py-live-0.7.0.img
mke2fs -q -t ext2 -F -b 4096 -d /tmp/py-rootfs /tmp/py-live-0.7.0.img
```

Ядро собрано из **git-архива** коммита `a0933ee` в `/tmp`:

```sh
mkdir -p /tmp/pagh-kernel-a0933ee
git -C /home/oleg/pagh-riscv archive a0933ee | tar -x -C /tmp/pagh-kernel-a0933ee
cd /tmp/pagh-kernel-a0933ee && cargo build --locked
```

Чекаут `/home/oleg/pagh-riscv` при этом не изменялся, а ELF, лежавший в его
`target/`, был собран раньше коммита (00:41 против 17:43), поэтому для
транскрипта взята сборка именно `a0933ee`; её `sha256` совпал с отдельной
сборкой с явным `--target-dir`. Путь к ядру в шапке транскрипта — этот
архив.

**Одна загрузка**, все команды подряд; драйвер — собственный
[`../tests/live/qemu_driver.py`](../tests/live/qemu_driver.py), обёртка —
[`../tests/live/live07.sh`](../tests/live/live07.sh), список команд —
[`../tests/live/live07.cmds`](../tests/live/live07.cmds). Сырой транскрипт:
[`live-pythonc-070.txt`](live-pythonc-070.txt).

## Итог 0.7.0

**43 запуска pythonc: 37 с кодом 0, 6 намеренных диагностик** (usage — 2;
`syntax_error.py`, `uncaught.py`, `generator_syntax.py`,
`decorator_syntax.py`, `yield_from_syntax.py` — 1), плюс три `cat`, которые
показывают staged-скрипт и два файла, записанных `fileio.py` и
`print(file=...)`. Все языковые проверки — assert-скрипты, поэтому код 0
означает совпадение значений, а не только напечатанные строки.

Новое в 0.7.0 (три скрипта, см. «Честные пробелы» про 1 МиБ):

| Скрипт | Что проверяет | Результат |
|---|---|---|
| `gen_methods.py` | `send(value)`/`send(None)`/`send` в незапущенный и исчерпанный генератор, `throw` экземпляром/классом/класс+аргументы (включая пользовательский класс), необработанный `throw`, `throw` в незапущенный, `close()` с `finally`, `close()` незапущенного, `RuntimeError` на теле, игнорирующем `GeneratorExit`, иерархия `GeneratorExit`, `PEP 479` (`throw(StopIteration)` → `RuntimeError`) | `GEN METHODS OK`, exit 0 |
| `gen_delegation.py` | `send`/`throw`/`close` через `yield from`, значение `return` делегированного генератора (однократное), особый случай `throw(GeneratorExit)` через `yield from`, вложенность, сужение для обычного итератора | `GEN DELEGATION OK`, exit 0 |
| `gen_fibers.py` | рецикл 200 стеков, лимит 8 приостановленных генераторов и его снятие через `close()` | `GEN FIBERS OK`, exit 0 |

Регрессия: все прежние проверки 0.6.0 (включая `YIELD FROM OK`,
`GENERATORS OK`, `os_proc.py` с `child: True status: 0`/`proc ok`) проходят
на ядре 0.21.2 без изменений.

## Ключевые строки 0.7.0

```text
send: ready one True                                     # gen_methods.py
send fresh: can't send non-None value to a just-started generator
send driven: 0 2 5
send exhausted: StopIteration
throw instance: value:boom
throw class / class+args: key, value:msg
throw user class: mine:custom mine:made
throw escapes: escaping
throw unstarted: body not run
close finally: ['cleanup']
close unstarted: nothing ran
close ignored: generator ignored GeneratorExit
close: GeneratorExit is BaseException
pep479: generator raised StopIteration
GEN METHODS OK

delegate send: sub return                                # gen_delegation.py
delegate throw: handled bad
delegate close: ['inner', 'outer']
delegate return value: answer
delegate nested: deep level1 down
delegate plain send: AttributeError
delegate plain throw: caught plain
recycled: 200 generators
limit: cannot start a generator: 8 are already suspended (64 KiB each)
GEN DELEGATION OK

child: True status: 0                                    # os_proc.py
proc ok
```

## Честные пробелы 0.7.0

- **Трёхаргументная форма `throw(type, value, traceback)` не поддерживается**
  и отвергается `TypeError` (у исключений этого интерпретатора нет
  traceback-объектов); CPython сам объявил её deprecated.
- **Делегация через `yield from` полная только для pagh-генератора.** Для
  обычного итератора (`range`, список) `send` не-`None` даёт `AttributeError`
  (hostcheck/live), а `throw`/`close` поднимаются в точке `yield from`.
- **Значение `return`** доступно только как значение `yield from`; через
  `StopIteration.value` оно не выставляется (у исключений нет атрибута
  `value`), и у генератора нет `gi_frame`/`gi_code`/`gi_yieldfrom` и метода
  `__next__`.
- **Проверки разделены по процессам из-за 1 МиБ.**
  Проверка лимита 8 требует 8 × 64 КиБ стеков (512 КиБ) — больше половины
  bump-heap ядра, а AST скрипта и уже созданные строки тоже занимают память.
  Первая версия была одним файлом на ~330 строк: `send`/`throw`/`close`
  проходили, а цикл на 200 генераторов в том же процессе получал
  `pythonc: out of memory (brk failed); requested 104 bytes` (код 102).
  Затем лимит переехал в общий скрипт с делегацией, и тот стал падать на
  девятом стеке (`requested 65536 bytes`). Поэтому suite разделён на три
  маленьких скрипта, у каждого свой процесс; `gen_fibers.py` — самый
  короткий, чтобы 512 КиБ стеков гарантированно помещались.
- Не проверялись живьём: `send`/`throw`/`close` на generator expression
  (hostcheck: `send` отбрасывается, `throw`/`close` поднимаются в `yield`),
  цепочка делегации длиннее двух уровней (hostcheck — два уровня),
  `close()` во время выполнения `finally` самого генератора, `throw`
  в генератор, который в этот момент уже выполняется (hostcheck:
  `ValueError: generator already executing`).

---

# Живой прогон pythonc 0.8.0 на ядре 0.21.2

Дата: 2026-11-13. Ядро: `pagh-riscv` 0.21.2, коммит
`a0933ee`, debug ELF,
`sha256 2e8ae74637fae1d83445b843d887bfcc5ccc371ccd6174e16ff64afee88cc0b1`
(собран из git-архива коммита, как и для 0.7.0: ELF в чекауте ядра был старше
коммита; сам чекаут не изменялся). QEMU `qemu-system-riscv64` 10.2.2, машина
`virt`, 256 MiB, serial на Unix-сокете, 64 MiB ext2.

**Одна загрузка**, все команды подряд; драйвер — собственный
[`../tests/live/qemu_driver.py`](../tests/live/qemu_driver.py), обёртка —
[`../tests/live/live08.sh`](../tests/live/live08.sh), список команд —
[`../tests/live/live08.cmds`](../tests/live/live08.cmds). Сырой транскрипт:
[`live-pythonc-080.txt`](live-pythonc-080.txt).

## Итог 0.8.0

**44 запуска pythonc: 38 с кодом 0, 6 намеренных диагностик** (usage — 2;
`syntax_error.py`, `uncaught.py`, `generator_syntax.py`,
`decorator_syntax.py`, `yield_from_syntax.py` — 1), плюс три `cat`.

Новое в 0.8.0 (один скрипт; генераторная часть 0.7.0 остаётся тремя
маленькими скриптами из-за 512 КиБ под 8 стеков — см. 0.7.0):

| Скрипт | Что проверяет | Результат |
|---|---|---|
| `dict_set.py` | `dict_keys`/`dict_values`/`dict_items` как живые view-объекты (`len`, итерация, `in`, `repr`, имена типов, отражение мутации, `dict(d.items())`, равенство), побитовые `& | ^ ~ << >>` с приоритетами и ошибками, set-алгебра `| & - ^`, in-place `|= &= -= ^=` с алиасами, `update`-семейство, `symmetric_difference`, `isdisjoint`, `pop` и диагностика | `DICT SET OK`, exit 0 |

Регрессия: все прежние проверки (включая `GEN METHODS OK`,
`GEN DELEGATION OK`, `GEN FIBERS OK`, `YIELD FROM OK`, `GENERATORS OK`,
`os_proc.py` с `child: True status: 0`/`proc ok`) проходят на 0.21.2.

## Ключевые строки 0.8.0

```text
views: dict_keys(['a', 'b']) dict_values([1, 2]) dict_items([('a', 1), ('b', 2)])
views are live: dict_keys(['b', 'c'])
views round-trip: dict(items) and for-over-values
bitwise: 2 7 5 -7 16 8
bitwise errors: integer left shift overflow | negative shift count | unsupported operand type(s) for &: 'str' and 'int'
set operators: {1, 2, 3, 4} {3} {1, 2} {1, 2, 4}
in-place set operators: aliases see the update
set update methods: {2, 6}
symmetric_difference/isdisjoint/subset/popping worked
set errors: pop from an empty set | unsupported operand type(s) for |: 'set' and 'list'
DICT SET OK
```

Видимое изменение поведения для существующих скриптов: `keys()`/`values()`/
`items()` больше не возвращают список, поэтому `print(counts.items())`
(`collections_demo.py`) печатает `dict_items([...])`, а не `[('a', 1), ...]`.
Итерация, `sorted()`, `list()`, `dict()` и `in` работают как раньше.

## Честные пробелы 0.8.0

- **View узкие:** нет set-операций над `dict_keys`/`dict_items`
  (`d.keys() | {…}`), нет сравнения view с `set`, нет `.mapping`; `values`
  view сравнивается только по идентичности объекта (как и в CPython, где у
  `dict_values` нет содержательного равенства).
- **Set-операторы требуют два `set`:** `set | list` — `TypeError`; методы
  (`union`, `update`, …) принимают любые iterable, как в CPython.
  `frozenset` нет.
- **Побитовые операторы 64-битные:** `1 << 64` — `OverflowError` (CPython
  вернул бы произвольное целое); `~True` = `-2` без DeprecationWarning.
- **Размер словаря во время итерации.** CPython поднимает
  `RuntimeError: dictionary changed size during iteration`; здесь итератор
  view/словаря идёт по индексу и ошибку не проверяет — мутация во время
  итерации даёт неопределённый, но не портящий память результат. Не
  проверялось живьём отдельным скриптом.
- Не проверялись живьём: `dict.keys() | set` (не поддерживается — см. выше),
  огромные сдвиги на границе i64, view после `clear()`/`update()` словаря в
  тех же выражениях (hostcheck покрывает мутацию/удаление после создания
  view).

---

# Живой прогон pythonc 0.9.0 на ядре 0.21.2

Дата: 2026-11-13. Ядро: `pagh-riscv` 0.21.2, коммит
`a0933ee`, debug ELF,
`sha256 2e8ae74637fae1d83445b843d887bfcc5ccc371ccd6174e16ff64afee88cc0b1`
(сборка из git-архива коммита, чекаут ядра не изменялся — как и для 0.7.0/
0.8.0). QEMU `qemu-system-riscv64` 10.2.2, машина `virt`, 256 MiB, serial
на Unix-сокете, 64 MiB ext2.

**Одна загрузка**, все команды подряд; драйвер — собственный
[`../tests/live/qemu_driver.py`](../tests/live/qemu_driver.py), обёртка —
[`../tests/live/live09.sh`](../tests/live/live09.sh), список команд —
[`../tests/live/live09.cmds`](../tests/live/live09.cmds). Сырой транскрипт:
[`live-pythonc-090.txt`](live-pythonc-090.txt).

## Итог 0.9.0

**45 запусков pythonc: 39 с кодом 0, 6 намеренных диагностик** (usage — 2;
`syntax_error.py`, `uncaught.py`, `generator_syntax.py`,
`decorator_syntax.py`, `yield_from_syntax.py` — 1), плюс три `cat`.

| Скрипт | Что проверяет | Результат |
|---|---|---|
| `keyword_only.py` | `def f(a, *, b)`/`def f(*args, x=1, **kw)`: keyword-only заполняются только по имени, `missing ... keyword-only argument`, голый `*`, чередование default'ов, методы/лямбды/генераторы, `**mapping` на вызове | `KEYWORD ONLY OK`, exit 0 |

Регрессия: все прежние проверки (включая `DICT SET OK`, `GEN METHODS OK`,
`GEN DELEGATION OK`, `GEN FIBERS OK`, `YIELD FROM OK`, `GENERATORS OK`,
`os_proc.py` с `child: True status: 0`/`proc ok`) проходят на 0.21.2.

## Ключевые строки 0.9.0

```text
def: (1, 2, 3) (1, 2, 9)
positional refused: f() takes 1 positional arguments but 2 were given
missing: f() missing 1 required keyword-only argument: 'b'
bare star: 7 -1
args/kwonly/kwargs: ((1, 2), 5, 1, {'z': 9})
defaults: (1, 5, 2) (9, 5, 2) (1, 2)
method: 6 12
lambda: 7 4
generator: [0, 1, 2]
call unpacking: 3
KEYWORD ONLY OK
```

## Честные пробелы 0.9.0

- **Только keyword-only часть сигнатур.** Нет позиционных-only параметров
  (`/` в CPython), нет аннотаций и `__annotations__`, нет перегрузки имён по
  позиции/ключу за пределами описанной модели; дубликаты имён отвергаются,
  как в CPython.
- **Замыкания и defaults** остались прежними: default вычисляется один раз
  при `def` (Python-семантика), поздних binding'ов нет.
- **`**mapping` в вызове** по-прежнему требует именно `dict` (не
  произвольный mapping с `keys()`), а смешанные `f(*a, x=1)`/`f(x=1, *a)`
  вычисляются в порядке «позиционные, затем именованные», а не строго в
  исходном порядке — это ограничение вызова, не сигнатуры, оно записано в
  README и не менялось.
- Не проверялись живьём: keyword-only параметры в генераторных выражениях
  (у них нет сигнатуры), очень длинные сигнатуры с `*args`+`**kwargs`+
  keyword-only в одном `def` (hostcheck покрывает), `staticmethod` с
  keyword-only (hostcheck: методы и `staticmethod` используют один binder).

---

# Живой прогон pythonc 0.9.1 на ядре 0.24.1

Дата: 2026-11-13. Ядро: `pagh-riscv` 0.24.1, коммит `aa8f1a9`, debug ELF,
`sha256 21bd93cfc9c8368031ded80c937c1bc42c5714bcdd4737cf336f924e6b1d485e`.
QEMU `qemu-system-riscv64` 10.2.2, машина `virt`, 256 MiB, serial на
Unix-сокете, 64 MiB ext2. `pythonc` — release, 582 056 байт,
`sha256 560f9997e5963d5335879916c8b275f04f839a571a6e73bb53e178322e03c5fd`.

**Одна загрузка**, все команды подряд; драйвер — собственный
[`../tests/live/qemu_driver.py`](../tests/live/qemu_driver.py), обёртка —
[`../tests/live/live091.sh`](../tests/live/live091.sh), список команд —
[`../tests/live/live091.cmds`](../tests/live/live091.cmds). Сырой транскрипт:
[`live-pythonc-091.txt`](live-pythonc-091.txt).

## Итог 0.9.1

**54 запуска pythonc: 43 с кодом 0, 11 намеренных диагностик** (usage — 2;
прежние `syntax_error.py`, `uncaught.py`, `generator_syntax.py`,
`decorator_syntax.py`, `yield_from_syntax.py` и новые `nesting_*` — 1), плюс
четыре `cat`.

Новое в 0.9.1 — аудит `F-rt-4`: предел вложенности парсера и проверка остатка
стека. Раньше 506-байтный PoC (`nesting_parens_200.py`, 200 скобок)
заканчивался `[user] pythonc killed: store page fault`; теперь это
`SyntaxError` и код 1.

| Скрипт | Что проверяет | Результат |
|---|---|---|
| `nesting_ok.py` | 100 уровней скобок, `not`, условных выражений, лямбд и вложенных `if` — принятая глубина разбирается и исполняется | `NESTING OK`, exit 0 |
| `nesting_parens_150.py` | 150 вложенных скобок на 1 МиБ стека цели | `PARENS150 OK 1`, exit 0 |
| `nesting_operators_200.py` | 200 унарных `not` = ровно `MAX_PARSE_DEPTH` | `NOT200 OK True`, exit 0 |
| `nesting_operators_201.py` | 201-й уровень — счётчик парсера | `SyntaxError: maximum recursion depth exceeded`, exit 1 |
| `nesting_blocks_200.py` | 200 вложенных `if` = ровно `MAX_PARSE_DEPTH` | `BLOCKS200 OK`, exit 0 |
| `nesting_blocks_201.py` | 201 вложенный блок | `SyntaxError: maximum recursion depth exceeded`, exit 1 |
| `nesting_parens_200.py` | PoC аудита verbatim: 506 байт, 200 скобок (до 0.9.1 — stack fault) | `SyntaxError: maximum recursion depth exceeded`, exit 1 |
| `nesting_parens_201.py` | 201 скобка — предел лексера | `SyntaxError: too many nested parentheses`, exit 1 |
| `nesting_unbalanced_300.py` | PoC «валидной программы нет»: 300 открывающих скобок | `SyntaxError: too many nested parentheses`, exit 1 |

Регрессия: весь suite 0.1.2–0.9.0 (включая `KEYWORD ONLY OK`, `DICT SET OK`,
`GEN METHODS OK`, `GEN DELEGATION OK`, `GEN FIBERS OK`, `YIELD FROM OK`,
`GENERATORS OK`, `child: True status: 0`/`proc ok`) проходит на ядре 0.24.1
без изменений.

## Ключевые строки 0.9.1

```text
NESTING OK
PARENS150 OK 1
NOT200 OK True
/mnt/nesting_operators_201.py:3:809: SyntaxError: maximum recursion depth exceeded
BLOCKS200 OK
/mnt/nesting_blocks_201.py:203:209: SyntaxError: maximum recursion depth exceeded
/mnt/nesting_parens_200.py:2:192: SyntaxError: maximum recursion depth exceeded
/mnt/nesting_parens_201.py:3:205: SyntaxError: too many nested parentheses
/mnt/nesting_unbalanced_300.py:2:205: SyntaxError: too many nested parentheses
```

`nesting_parens_200.py` — это ровно тот файл из аудита: его `cat` в
транскрипте показывает 506 байт и 200 вложенных скобок, а `run` сразу после
— `SyntaxError` с кодом 1 и без строки `killed`.

## Честные пробелы 0.9.1

- **Эффективная глубина скобок на цели меньше 200.** Языковой предел — 200
  уровней, и hostcheck проверяет его точно; на pagh-riscv 1 МиБ user-стека
  вмещает примерно 186 вложенных скобок (один уровень стоит несколько КиБ
  Rust-кадров), дальше stack-headroom guard поднимает тот же `SyntaxError`.
  Число зависит от сборки и кадров парсера и контрактом не является; дешёвые
  уровни (`not`, блоки) доходят до 200 и на цели — это видно в таблице.
- **Два предела независимы.** Скобки ограничены лексером, остальные
  конструкции — счётчиком парсера; «суммарного 200» на смесь «много лямбд +
  много скобок» нет. На хосте каждый предел проверяется отдельно, на цели
  физический предохранитель один — остаток стека.
- **Парсер на heap-стеке файбера.** Если `import` выполняется внутри
  генератора, `parser::parse` работает на 64 КиБ heap-стеке файбера, а не на
  process stack: адресная проверка `USER_STACK_FLOOR` к такой памяти
  неприменима (она пропускается), счётчики вложенности при этом действуют.
  Живьём этот путь в 0.9.1 не проверялся; отдельного теста на глубокий
  импорт из генератора нет.
- **Константы ядра.** Нижняя граница стека (`0x3ff0_0000`) и запас 128 КиБ
  записаны в `src/platform.rs`/`src/parser.rs` по контракту pagh-riscv 0.24.1
  (`pagh-riscv/src/user/elf.rs`: `USER_STACK_FLOOR`, 1 МиБ); при изменении
  контракта ядра их нужно пересмотреть.

---

# Живой прогон pythonc 0.9.2 на ядре 0.24.1

Дата: 2026-11-14. Ядро: `pagh-riscv` 0.24.1, чистое дерево `origin/main` на
`a3f3a70`, debug ELF, `sha256 2e0dfca19185933603992c321c9a5d2221183ac3236e7bde7ea56a70f54db740`.
QEMU `qemu-system-riscv64` 10.2.2, машина `virt`, 256 MiB, serial на
Unix-сокете, 64 MiB ext2. `pythonc` — release, 582 056 байт,
`sha256 5cd9c9d4eac4e1f53e283cba5d83045f41598ed7ae80035dbcae0a86ddab8a21`.

**Одна загрузка**, все команды подряд; драйвер — собственный
[`../tests/live/qemu_driver.py`](../tests/live/qemu_driver.py), обёртка —
[`../tests/live/live092.sh`](../tests/live/live092.sh), список команд —
[`../tests/live/live092.cmds`](../tests/live/live092.cmds). Сырой транскрипт:
[`live-pythonc-092.txt`](live-pythonc-092.txt).

## Итог 0.9.2

**69 запусков pythonc: 51 с кодом 0, 18 намеренных диагностик** (usage — 2;
семнадцать диагностик — 1: прежние пять, пять 0.9.1 nesting-сценариев и семь
новых evaluator-сценариев), плюс пять `cat`. После последнего запуска
транскрипт показывает `SUITE ALIVE`: машина жива после всех «бомб».

Новое в 0.9.2 — аудит `F-r2-py-1/2`: фикс 0.9.1 ограничивал только
рекурсивный спуск парсера, а лево-ассоциативная цепочка `1+1+…` строится
парсером в цикле и затем рекурсивно обходится `eval_expr`. 740 термов
убивали интерпретатор на `USER_STACK_FLOOR` (`store page fault`,
`stval=0x3feffd68`), а то же выражение в генераторе переполняло 64-КиБ
heap-фибер без guard-страницы (wild-pointer падения `stval=0x1/0x0/0x44800000008/0x1fffffe8`).
Теперь глубина вычисления ограничена 256 уровнями AST, а перед каждым узлом
проверяется остаток стека (процесс — 1 МиБ с запасом 64 КиБ, файбер — 64 КиБ
с запасом 8 КиБ). Все PoC-цепочки — чистые `RecursionError` и код 1.

| Скрипт | Что проверяет | Результат |
|---|---|---|
| `eval_chain_200.py` | 200 термов `1+1+…` — цепочка, которую парсер свернул циклом | `EVAL CHAIN 200 OK 200`, exit 0 |
| `eval_chain_256.py` | ровно `EXPR_RECURSION_LIMIT` = 256 уровней вычисления | `EVAL CHAIN 256 OK 256`, exit 0 |
| `eval_chain_257.py` | 257-й уровень | `RecursionError: maximum recursion depth exceeded (256 expression levels)`, exit 1 |
| `eval_chain_720.py` | порог аудита: 720 термов ещё «работали» на 0.9.1 случайно | тот же `RecursionError`, exit 1 |
| `eval_chain_740.py` | 740 термов — на 0.9.1 stack guard kill (`stval=0x3feffd68`) | тот же `RecursionError`, exit 1 |
| `eval_chain_800.py` | PoC `F-r2-py-1` verbatim: 800 термов, 1626 байт | тот же `RecursionError`, exit 1 |
| `eval_chain_func_uncalled.py` | 3000-термная цепочка в невызываемой функции: парсер её по-прежнему разбирает | `PARSE OK`, exit 0 |
| `eval_chain_caught.py` | `RecursionError` ловится, интерпретатор продолжает работу | `CAUGHT: …`, `EVAL RECOVERED OK`, exit 0 |
| `eval_chain_interleave.py` | генератор, приостановленный на `yield` внутри выражения, не отнимает глубину у вызывающего | `INTERLEAVE OK 1 200 43`, exit 0 |
| `eval_chain_attr.py` | attribute-цепочка `c.self.self…` (300 звеньев) — та же лево-глубая форма | `RecursionError`, exit 1 |
| `eval_chain_sub.py` | subscript-цепочка `l[0][0]…` (300 звеньев) | `RecursionError`, exit 1 |
| `gen_chain_20.py` | короткая цепочка в генераторе работает | `GEN CHAIN 20 OK 20`, exit 0 |
| `gen_chain_100.py` | PoC `F-r2-py-2`: 100 термов в генераторе (было wild-pointer падение) | `GEN GUARD OK: generator stack exhausted (7688 bytes left of 64 KiB)`, `ALIVE AFTER GEN BOMB`, exit 0 |
| `gen_chain_300.py` | необработанный вариант: 300 термов в генераторе | `RecursionError: generator stack exhausted`, exit 1 |
| `recursion_limit.py` | регрессия рекурсии вызовов: 55 кадров работают, 56-й — `RecursionError` | `RECURSION LIMIT OK`, exit 0 |

Регрессия: весь suite 0.1.2–0.9.1 проходит на 0.24.1 без изменений —
`KEYWORD ONLY OK`, `DICT SET OK`, `GEN METHODS OK`, `GEN DELEGATION OK`,
`GEN FIBERS OK`, `YIELD FROM OK`, `GENERATORS OK`, `NESTING OK`,
`PARENS150 OK`, `NOT200 OK`, `BLOCKS200 OK`,
`child: True status: 0`/`proc ok` на месте. Отдельно прогнаны сами PoC из
`attack/r2/runtime/pocs/python` на финальной сборке: `eval-bomb.py` (800
термов; control 700 термов теперь тоже ошибка, потому что предел —
256, а не порог стека), все четыре `fiber-bomb-*`, `gen_add_100/200/500` —
чистые `RecursionError` и код 1, `func_add_3000.py` — `PARSE OK` и код 0,
после чего `echo ALIVE AFTER ATTACK POCS` печатается.

## Ключевые строки 0.9.2

```text
EVAL CHAIN 200 OK 200
EVAL CHAIN 256 OK 256
RecursionError: maximum recursion depth exceeded (256 expression levels)   [eval_chain_257/720/740/800, attr, sub]
PARSE OK
CAUGHT: maximum recursion depth exceeded (256 expression levels)
EVAL RECOVERED OK
INTERLEAVE OK 1 200 43
GEN CHAIN 20 OK 20
GEN GUARD OK: generator stack exhausted (7688 bytes left of 64 KiB)
ALIVE AFTER GEN BOMB
RecursionError: generator stack exhausted (7688 bytes left of 64 KiB)      [gen_chain_300]
CALL LIMIT CAUGHT: maximum recursion depth exceeded (56 Python frames)
RECURSION LIMIT OK
NOT200 OK True
SUITE ALIVE
```

## Честные пробелы 0.9.2

- **256 уровней — меньше CPython'овских 1000.** Один уровень — это один узел
  AST (бинарная операция, имя, литерал), а не один Rust-кадр парсера:
  `1+1+…` из 257 термов — уже `RecursionError`, хотя CPython такое выражение
  компилирует. Для длинных однотипных цепочек в скрипте лучше цикл/`sum()`;
  предел документирован и одинаков на хосте и на цели.
- **Под 64-КиБ файбером по-прежнему нет guard-страницы.** Защита —
  программная (`Fiber::headroom` + `STACK_MARGIN` 8 КиБ на каждом узле
  `eval_expr` и на каждом `exec_stmt`), а не аппаратная. Промах в оценке
  размера кадра теоретически всё ещё может дописать за конец `Vec<u8>`;
  отдельной mmap-guard-страницы в ABI нет.
- **Проверка process stack — только на цели.** На хосте
  `platform::process_stack_bounds()` возвращает `None` (у потока теста нет
  документированного окна), поэтому hostcheck проверяет точную границу по
  счётчику, а фактический остаток стека — только на pagh-riscv.
- **Глубина `exec_stmt` не считается отдельно.** Вложенные блоки
  ограничивает парсер (200 уровней), а на файбере — headroom-проверка; на
  process stack глубокие блоки не превышают парсерного предела и в 1 МиБ
  укладываются.
- **Константы ядра.** Окно `0x3ff0_0000..0x4000_0000` и запас 64 КиБ
  записаны в `src/platform.rs`/`src/interp.rs` по контракту pagh-riscv 0.24.1
  (`pagh-riscv/src/user/elf.rs`: `USER_STACK_FLOOR`, `USER_STACK_MAX_SIZE`);
  при изменении контракта их нужно пересмотреть.

---

# Живой прогон pythonc 0.9.3 на ядре 0.24.2

Дата: 2026-11-14. Ядро: `pagh-riscv` 0.24.2, чистое дерево `origin/main` на
`23fe825`, debug ELF,
`sha256 29f0d29c76770bc88c7b5fc27accfcaaf538e149fd4cfc4d1f6ee5079e80ebdc`.
QEMU `qemu-system-riscv64` 10.2.2, машина `virt`, 256 MiB, serial на
Unix-сокете, 64 MiB ext2. Ядро собрано из git-архива коммита в `/tmp`
(чекаут `/home/oleg/pagh-riscv` при этом не изменялся). `pythonc` — release,
586 152 байт, `sha256 a65f512899644a64c7ea00a0a63924b592d6d440ccd9b45c9c5567fcc1a60cf7`.

**Одна загрузка**, все команды подряд; драйвер — собственный
[`../tests/live/qemu_driver.py`](../tests/live/qemu_driver.py), обёртка —
[`../tests/live/live093.sh`](../tests/live/live093.sh), список команд —
[`../tests/live/live093.cmds`](../tests/live/live093.cmds). Сырой транскрипт:
[`live-pythonc-093.txt`](live-pythonc-093.txt).

## Итог 0.9.3

**80 запусков pythonc: 62 с кодом 0, 18 намеренных диагностик** (usage — 2;
семнадцать диагностик — 1: пять прежних, пять 0.9.1 nesting-сценариев и семь
0.9.2 evaluator-сценариев), плюс пять `cat`. После последнего запуска
транскрипт показывает `SUITE ALIVE`: машина жива после всех «бомб», включая
deep-drop PoC.

Новое в 0.9.3 — аудит `F-r3-rt-1`: лимиты 0.9.2 считали узлы AST и проверяли
остаток стека перед каждым узлом, но освобождение `Rc`-цепочек — нативный
рекурсивный drop glue, который не проходит через `eval_expr`/`exec_stmt`.
`a = None` для 2000-вложенного списка внутри генератора переполнял 64-КиБ
heap-фибер без guard-страницы и портил кучу интерпретатора; 1800 уровней
выживало. Теперь `Value::drop` и `Env::drop` дренируют граф объектов явным
worklist'ом; `Set`-сценарий дополнительно требует итеративного
`is_hashable` (иначе tuple-цепочка переполняет файбер ещё при построении
`set`).

| Скрипт | Что проверяет | Результат |
|---|---|---|
| `drop_sweep_1800.py` | контроль прежнего порога: 1800 вложенных списков | `SURVIVED 1800`, exit 0 |
| `drop_fiber_2000.py` | PoC `F-r3-rt-1` verbatim: 2000-deep list, `a = None` в генераторе | `FIBER DROP OK 2000`, exit 0 |
| `drop_fiber_4000.py` | 4000-deep list (0.9.2: `store page fault`, stval=0x1ffffff8) | `FIBER DROP OK 4000`, exit 0 |
| `drop_fiber_tuple_4000.py` | 4000-deep tuple chain | `TUPLE FIBER OK`, exit 0 |
| `drop_fiber_dict_2000.py` | 2000-deep dict chain | `DICT FIBER OK`, exit 0 |
| `drop_fiber_set_2000.py` | `set` с 2000-deep tuple-ключом (плюс `is_hashable`) | `SET FIBER OK`, exit 0 |
| `drop_fiber_env_4000.py` | 4000-deep локальная переменная освобождается при возврате функции (`Env`) | `ENV FIBER OK`, exit 0 |
| `drop_fiber_instance_2000.py` | 2000-звенная цепочка `Instance.attrs` | `INSTANCE FIBER OK`, exit 0 |
| `drop_fiber_function_1000.py` | 1000 closure-звеньев `Function → Env → Function` | `FUNCTION FIBER OK`, exit 0 |
| `drop_fiber_iterator_4000.py` | iterator — последний владелец 4000-deep list | `ITERATOR FIBER OK`, exit 0 |
| `drop_fiber_view_2000.py` | `dict_items`-view — последний владелец 2000-deep dict | `VIEW FIBER OK`, exit 0 |

## Ключевые строки 0.9.3

```text
SURVIVED 1800
FIBER DROP OK 2000
FIBER DROP OK 4000
TUPLE FIBER OK
DICT FIBER OK
SET FIBER OK
ENV FIBER OK
INSTANCE FIBER OK
FUNCTION FIBER OK
ITERATOR FIBER OK
VIEW FIBER OK
ALIVE AFTER DROP TESTS
SUITE ALIVE
```

Регрессия: весь suite 0.1.2–0.9.2 проходит без изменений — `EVAL CHAIN 200 OK`,
`EVAL CHAIN 256 OK`, `EVAL RECOVERED OK`, `PARSE OK`, `INTERLEAVE OK`,
`GEN CHAIN 20 OK`, `GEN GUARD OK`, `ALIVE AFTER GEN BOMB`,
`RECURSION LIMIT OK`, `KEYWORD ONLY OK`, `DICT SET OK`, `GEN METHODS OK`,
`GEN DELEGATION OK`, `GEN FIBERS OK`, `YIELD FROM OK`, `NESTING OK`,
`PARENS150 OK`, `NOT200 OK`, `BLOCKS200 OK` и `child: True status: 0` /
`proc ok` на месте.

## Baseline: stock 0.9.2 на том же ядре

Отдельная загрузка того же ядра с бинарником 0.9.2 (release, 582 056 байт;
собран из git-архива `origin/dev` без изменений 0.9.3) на тех же скриптах:

```text
run /mnt/pythonc /mnt/drop_sweep_1800.py
BUILT
DROPPED
SURVIVED 1800
run: /mnt/pythonc exited with code 0
run /mnt/pythonc /mnt/drop_fiber_2000.py
BUILD
BUILT
[user] /mnt/pythonc killed: load page fault (cause=13, stval=0x5204c1930, sepc=0x2ed92)
run /mnt/pythonc /mnt/drop_fiber_4000.py
BUILD
BUILT
[user] /mnt/pythonc killed: store page fault (cause=15, stval=0x1ffffff8, sepc=0x2881a)
run /mnt/pythonc /mnt/drop_fiber_tuple_4000.py
TUPLE BUILT
[user] /mnt/pythonc killed: store page fault (cause=15, stval=0x1ffffff8, sepc=0x2874e)
```

Шелл после каждого kill остаётся жив (`ALIVE AFTER STOCK BOMBS`), то есть
это падение процесса интерпретатора, а не машины.

## Честные пробелы 0.9.3

- **Глубже 4000 живьём не проверялось.** На 1 МиБ bump-heap сама цепочка
  8000-deep list занимает около половины памяти, поэтому acceptance-набор
  остаётся на 1800/2000/4000 из аудита; итеративность teardown от глубины не
  зависит. Hostcheck проверяет 4000 для всех контейнерных видов.
- **Остались рекурсивными** парсер и `eval_expr` (со своими лимитами 200/256),
  `repr` (глубина 12) и обход AST. Это не drop-путь и в F-r3-rt-1 не входило.
- **Сборщика циклов нет:** контейнер, ссылающийся на себя, по-прежнему не
  освобождается (утечка до конца процесса), как и в 0.9.2.
- **Стек процесса:** тот же deep-drop на process stack работал и до фикса
  (1 МиБ, не 64 КиБ); проверка живьём сосредоточена на генераторном файбере,
  где и был баг.
