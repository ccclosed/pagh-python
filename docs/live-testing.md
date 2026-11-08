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
