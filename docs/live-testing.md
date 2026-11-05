# Живой прогон pythonc 0.1.0

Дата: 2026-11-05. Ядро:
`/home/oleg/pagh-riscv/target/riscv64gc-unknown-none-elf/debug/pagh`
(0.11.0, native ABI). QEMU `qemu-system-riscv64` 10.2.2, машина `virt`,
256 MiB, serial на Unix-сокете, `-bios default`, `-display none`.
Диск: 64 MiB ext2, создан на хосте

```sh
mke2fs -q -t ext2 -F -b 4096 -d rootfs disk.img 64M
```

В корне образа — `pythonc` (release, 781 928 байт) и скрипты из
[`../tests/live/`](../tests/live/). Одна загрузка, 21 команда подряд; после
каждого `run` ядро печатает `run: /mnt/pythonc exited with code N`.

Сырой транскрипт: [`live-pythonc.txt`](live-pythonc.txt).

## Итог

**21 PASS из 21 команды.** Двенадцать прежних языковых проверок (hello,
functions, collections, classes, exceptions, file I/O, import, argv, `-c`,
usage, syntax error, uncaught exception) по-прежнему проходят. Старая проверка
«float gap is explicit» удалена: она утверждала ложное ограничение. Вместо неё
добавлены шесть float-скриптов и `-c print(1/2)`, то есть семь новых PASS.

Два `cat` подтверждают, что файлы, записанные через `open(..., 'w')`, реально
дошли до ext2.

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
```

## Что именно проверено

```text
run /mnt/pythonc /mnt/hello.py
hello from pagh-python
arithmetic: 7 3 1 1024
builtins: 5 7 3 4
sep/end:-a-b-c.
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/functions.py
fact(6) = 720
fib(10) = 55
nested calls: 16
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/collections_demo.py
list: [2, 3, 4, 5, 6] pop: 1 slice: [3, 4, 5] reversed: [6, 5, 4, 3, 2]
dict: {'apple': 2, 'pear': 1, 'plum': 3} keys: ['apple', 'pear', 'plum']
...
sorted: [1, 2, 5, 9] ['b', 'a']
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/classes.py
cat makes a sound
Rex barks
Animal(cat) isinstance: True False
counter: 1 2 3
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/exceptions.py
caught ValueError: bad value
caught ZeroDivisionError
propagated: from fail
try body
finally runs
caught KeyError: 'key'
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/fileio.py
file lines: ['first line', 'second line', 'third line']
readline: first line
rest: second line|third line|
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/import_main.py
mymod.answer = 42
mymod.double(4) = 8
from-import double(21) = 42
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/argv.py one two
script: /mnt/argv.py
args: ['one', 'two']
count: 3
run: /mnt/pythonc exited with code 0
```

`fileio.py` пишет `/mnt/py.txt` тремя строками, затем открывает его на чтение,
читает итерацией по файлу и через `readline`/`read`; отдельно
`print("print to file", file=out)` проверяет запись в файл из `print`.
`cat /mnt/py.txt` и `cat /mnt/print.txt` после этого показывают ожидаемый текст,
то есть закрытие файла действительно сбрасывает буфер в ext2.

### Float

Все float-скрипты завершаются `assert`-проверками, поэтому код возврата 0
означает, что значения совпали с ожидаемыми, а не только что программа
допечатала строки:

```text
run /mnt/pythonc /mnt/float_arithmetic.py
literals: 1.5 0.5 1000.0 0.015 1.0 10.5
mixed: 3.5 5.0 3.75 -1.5 1.5
float arithmetic ok
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/float_division.py
true division: 0.5 3.5 -3.5
floor/mod: 3.0 -4.0 1.5 0.5 -0.5
caught integer division by zero
caught float floor division by zero
caught float modulo by zero
float division ok
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/float_format.py
str: 1.0 1.5 1e+20 1e-07 1.5e-07
repr: 1.0 0.30000000000000004 -0.0
float formatting ok
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/float_compare.py
mixed compare: True True True
nan compare: False False
float comparison ok
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/float_math.py
pi/e: 3.141592653589793 2.718281828459045
sqrt/pow: 1.4142135623730951 1.4142135623730951
trig: 0.8414709848078965 0.5403023058681397 1.5574077246549025
log/exp: 1.0 3.0 2.7182818284590446
rounding: -3 -2 -2 2.5
float math ok
run: /mnt/pythonc exited with code 0

run /mnt/pythonc /mnt/float_errors.py
typed float errors: 6
float errors ok
run: /mnt/pythonc exited with code 0

run /mnt/pythonc -c print(1/2)
0.5
run: /mnt/pythonc exited with code 0
```

Проверены decimal/exponent-литералы (`.5`, `1.`, `1e3`, `1.5e-2`, `1_0.5`),
смешанная арифметика int/float, `%` с знаком делителя, `//` с float-результатом,
`divmod` с float, отрицательные и float-показатели `**`, `round` с ties-to-even
и `ndigits`, `int(float)` (усечение к нулю), `float(int/bool/str)`, `bool(float)`,
NaN-сравнения, точное сравнение `int` с `float` выше 2^53, `min`/`max`/`sorted`/
`sum` с float, `type(1.0)`, `isinstance(1.5, float)`.

Модуль `math` проверен целиком: `pi`, `e`, `sqrt`, `floor`, `ceil`, `trunc`,
`fabs`, `pow`, `fmod`, `sin`, `cos`, `tan`, `log` (включая `log(x, base)`),
`log10`, `exp`, `inf`, `nan`, `isnan`, `isinf`; `sys.float_info.dig` читается.
Отдельно проверено, что доменные ошибки типизированы: `int(nan)` → `ValueError`,
`int(inf)` → `OverflowError`, `math.sqrt(-1)`/`math.log(0)` → `ValueError`,
`0.0 ** -1` → `ZeroDivisionError`, `(-8) ** (1/3)` → `ValueError` (комплексных
чисел в интерпретаторе нет).

### Проверка FPU до реализации

Утверждение «ядро 0.11.0 исполняет f64 в U-mode» проверено не по символам, а
живым запуском `/home/oleg/pagh-api/examples/fp.rs`: `1.5 * 2.25` и сумма
`1..=100` дали `fp product=3.375`, `fp sum=5050`, exit 0. Дополнительно
временная U-mode программа проверила `fdiv`, сравнения, `fcvt` в `i64` и
`core::fmt` (`display=1.5 debug=1.5 big=1e20 small=1.5e-7`), тоже exit 0.
Интерпретатор использует те же инструкции: `NaN`/`inf` не ловятся ядром.

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

Float-ошибки используют те же типизированные исключения; отдельного
`NotImplementedError` для float больше нет:

```text
run /mnt/pythonc /mnt/float_errors.py
typed float errors: 6
float errors ok
run: /mnt/pythonc exited with code 0
```

## Зависимость от FPU

`pythonc` теперь содержит настоящие f64-инструкции. Ядро обязано оставлять
`sstatus.FS` включённым при `sret` в U-mode (как в 0.11.0). Если ядро выключит
FPU, первая же float-операция упадёт по illegal instruction; это не ошибка
интерпретатора, а нарушение контракта ядра. `hostcheck` на x86-64 проверяет ту
же семантику без QEMU.

## Что не проверялось живьём

- Сеть, потоки, процессы и сигналы: их нет в языке/ABI этого проекта.
- `-c` с пробелами в коде: шелл ядра делит команду по пробелам, поэтому в
  живом прогоне `-c` проверен на `print(6*7)` и `print(1/2)`; на хосте
  `hostcheck -c` с произвольным кодом покрыт unit-тестами.
- Долгие программы и исчерпание 1 MiB heap: в тестах участвуют только
  небольшие скрипты, рассчитанные на bump-heap ядра.
- Точное соответствие CPython для трансцендентных функций: см.
  [`../README.md`](../README.md), раздел «Ограничения» (`exp`/`cos`/`tan` могут
  отличаться на 1–2 ulp, `round(float, ndigits)` — на 1 ulp для очень больших
  величин и отрицательных `ndigits`).

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