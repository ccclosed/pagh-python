# pagh-python — Python-компилятор/интерпретатор для pagh-riscv

Собственная реализация Python (не CPython) на Rust поверх `libpagh`:
лексер → парсер → AST → байткод → стековая VM. Запускается в U-mode через
`run /mnt/pythonc /mnt/script.py`.

## Цель

Полноценный **язык Python** в объёме, который реально помещается в hobby-ОС:
типы `None/bool/int/float/str/list/tuple/dict/set`, операторы и срезы,
`if/while/for/break/continue`, функции и замыкания, `def/lambda`, классы с
наследованием, исключения и `try/except/finally`, модули/импорты (встроенные и
из `/mnt`), builtins (`len/range/print/input/open/…`), файловый I/O, `sys`,
`os`-подмножество, `math`, `json` (по мере готовности). Это не CPython и не
претендует на 100% стандартной библиотеки; но это настоящий интерпретатор
Python-синтаксиса и семантики, а не «утилита, похожая на Python».

## Почему не CPython

CPython — ~800 тыс. строк C и требует libc, `mmap`, потоки, сокеты,
динамический загрузчик и POSIX. У нас 22 syscall'а и один процесс; порт CPython —
отдельный многомесячный проект. Реализация собственного интерпретатора даёт
рабочий Python на нашем ABI уже сейчас и растёт инкрементально.

## Сборка и запуск

```sh
cargo build --release --target riscv64gc-unknown-none-elf
python3 tools/package.py            # .ppkg для pagh-repo (позже)
# гость: run /mnt/pythonc /mnt/hello.py
```
