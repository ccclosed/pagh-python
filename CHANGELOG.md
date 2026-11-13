# История изменений

Формат — по мотивам [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/);
версии — [SemVer](https://semver.org/lang/ru/). Подробные живые прогоны,
PASS-листы и транскрипты каждой версии — в [README.md](README.md) и
[docs/live-testing.md](docs/live-testing.md).

## [0.9.1] — 2026-11-13

Исправление безопасности по аудиту runtime-стека Pagh (`F-rt-4`, low,
verified): рекурсивный спуск парсера не имел счётчика вложенности, и
506-байтный файл `x = ` + 200 `(` исчерпывал 1 МиБ user-стека `pythonc` до
`USER_STACK_FLOOR` — процесс умирал по `store page fault` вместо
`SyntaxError`.

### Безопасность

- **Скобки ограничены 200 уровнями на уровне лексера** (`MAX_BRACKET_DEPTH`).
  Проверка итеративная и выполняется до рекурсии: 200 вложенных `()`, `[]`,
  `{}` (включая скобки вызовов, подписок, tuple-целей и comprehension)
  разбираются, 201-я отвергается как `SyntaxError: too many nested
  parentheses` — тот же предел и то же сообщение, что у токенизатора CPython
  (`MAXLEVEL = 200`).
- **Явный счётчик глубины парсера — 200 уровней** (`MAX_PARSE_DEPTH`) на все
  рекурсивные ветки, которые не ограничены скобочным токеном: цепочки
  `not`/унарных операторов, правая часть `**`, ветка `else` условного
  выражения, тело лямбды, блок операторов (`if`/`for`/`while`/`def`/`class`/
  `try`/`with`), цель присваивания и операнд `yield`. 201-й уровень —
  `SyntaxError: maximum recursion depth exceeded`, не паника и не
  переполнение стека. F-string-фрагмент наследует глубину внешнего
  выражения, comprehension/yield-guard сохранён.
- **Проверка остатка стека на pagh-riscv.** Перед каждым уровнем парсер
  сверяет `sp` с нижней границей user-стека (`USER_STACK_FLOOR`, 1 МиБ от
  `USER_TOP`) и запасом 128 КиБ: 200 скобок, которые физически не помещаются
  в стек, теперь дают тот же `SyntaxError` с кодом 1 вместо
  `[user] pythonc killed: store page fault`. На хосте (`hostcheck`) граница
  199/200/201 проверяется точно.

### Тесты

- `hostcheck`: новые проверки `parser bracket nesting` и
  `parser recursion depth` — 199 и 200 уровней каждой конструкции
  разбираются, 201-й отвергается (всего 42 проверки, 42 `PASS` в
  `--selftest`).
- Живой QEMU-прогон `tests/live/live091.sh` на ядре pagh-riscv 0.24.1:
  54 запуска, 43 с кодом 0, 11 намеренных диагностик; PoC аудита
  (`tests/live/nesting_parens_200.py`, 506 байт) — `SyntaxError` и код 1,
  `nesting_parens_201.py`/`nesting_unbalanced_300.py` — `too many nested
  parentheses`, 201 вложенный `not`/`if` — `maximum recursion depth
  exceeded`. Транскрипт: [docs/live-pythonc-091.txt](docs/live-pythonc-091.txt).

### Документация

- README: раздел о пределах вложенности парсера и стека, обновлены статус,
  счётчики тестов, ссылка на новый live-suite и транскрипт.
- Новый файл CHANGELOG.md.

## [0.9.0] — 2026-11-13

### Добавлено

- Keyword-only параметры: `def f(a, *, b)`, `def f(a, *args, b=2, **kwargs)`,
  лямбды, методы и `staticmethod`; голый `*` без имён, повторный `*`/`**`,
  параметр после `**kwargs` и дубликаты имён отвергаются как в CPython.
  Живой прогон — `tests/live/live09.sh`, маркер `KEYWORD ONLY OK`.

## [0.8.0] — 2026-11-12

### Добавлено

- `dict.keys()`/`values()`/`items()` — живые view-объекты (`len`, итерация,
  `in`, `repr`, отражение мутации, `dict(d.items())`), побитовые операторы
  `& | ^ ~ << >>` с приоритетами и 64-битной семантикой, set-алгебра
  `| & - ^`, in-place формы, `update`-семейство и `symmetric_difference`.
  Живой прогон — `tests/live/live08.sh`, маркер `DICT SET OK`.

## [0.7.0]

### Добавлено

- Протокол генераторов: `send()`, `throw()`, `close()` и делегирование через
  `yield from` (`send`/`throw`/`close`, значение `return`), лимит
  8 приостановленных генераторов и рецикл 200 стеков. Живой прогон —
  `tests/live/live07.sh`, маркеры `GEN METHODS OK`, `GEN DELEGATION OK`,
  `GEN FIBERS OK`.

## [0.6.0]

### Добавлено

- `yield from` для генераторов, `range`, списков, generator expressions и
  объектов с `__iter__`; ленивость, вложенность, `finally` и ошибки
  не-iterable. Живой прогон — `tests/live/live06.sh`, маркер `YIELD FROM OK`.

## [0.5.0]

### Добавлено

- Генераторы (`yield`, `next()`, `StopIteration`, `for/else`, `finally`,
  повторный вход), generator expressions, декораторы, `%`-форматирование,
  модули `os.path`/`time`/`random`; предохранитель стека файбера
  (`RecursionError` вместо порчи кучи). Живой прогон —
  `tests/live/live05.sh`.

## [0.1.0] — [0.4.0]

Ранние версии: лексер/парсер/интерпретатор, классы и исключения, `with`,
`set`, `bytes`, f-strings, `os`, `json`, `os.fork`/`execv`/`wait4`,
float-арифметика; 0.4.0 — `*args`/`**kwargs` и `str.format`. Транскрипты и
PASS-листы — `tests/live/live05.cmds` и более ранние разделы
[docs/live-testing.md](docs/live-testing.md).
