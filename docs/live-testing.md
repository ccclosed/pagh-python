# Живой прогон pythonc 0.1.0

Дата: 2026-11-05. Ядро:
`/home/oleg/pagh-riscv/target/riscv64gc-unknown-none-elf/debug/pagh`
(0.11.0, native ABI). QEMU `qemu-system-riscv64` 10.2.2, машина `virt`,
256 MiB, serial на Unix-сокете, `-bios default`, `-display none`.
Диск: 64 MiB ext2, создан на хосте

```sh
mke2fs -q -t ext2 -F -b 4096 -d rootfs disk.img 64M
```

В корне образа — `pythonc` (release, 646 872 байт) и скрипты из
[`../tests/live/`](../tests/live/). Одна загрузка, 15 команд подряд; после
каждого `run` ядро печатает `run: /mnt/pythonc exited with code N`.

Сырой транскрипт: [`live-pythonc.txt`](live-pythonc.txt).

## Итог

**13 PASS из 13 запусков `pythonc`.** Два дополнительных `cat` подтверждают,
что файлы, записанные через `open(..., 'w')`, реально дошли до ext2.

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
PASS float gap is explicit: run /mnt/pythonc /mnt/float_error.py -> exit 1
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

`float` в этой сборке отсутствует осознанно (ядро запускает U-mode без FPU);
ошибка типизированная и называет причину:

```text
run /mnt/pythonc /mnt/float_error.py
before float
Traceback (most recent call last):
  File "/mnt/float_error.py", line 2, in <module>
NotImplementedError: floating-point literal '1.5' is not supported: the pagh kernel runs U-mode with the FPU disabled, so this interpreter is integer-only
run: /mnt/pythonc exited with code 1
```

## Что не проверялось живьём

- Сеть, потоки, процессы и сигналы: их нет в языке/ABI этого проекта.
- `-c` с пробелами в коде: шелл ядра делит команду по пробелам, поэтому в
  живом прогоне `-c` проверен на `print(6*7)`; на хосте `hostcheck -c` с
  произвольным кодом покрыт unit-тестами.
- Долгие программы и исчерпание 1 MiB heap: в тестах участвуют только
  небольшие скрипты, рассчитанные на bump-heap ядра.

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