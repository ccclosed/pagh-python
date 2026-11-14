//! Host-side checks for the interpreter core.
//!
//! The same functions power `hostcheck --selftest` and `cargo test`: they run
//! real Python source through the parser and evaluator and inspect the module
//! namespace, so a failure names both the construct and the value it produced.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::PyError;
use crate::interp::Interp;
use crate::parser;
use crate::value::{env_lookup, Value};

/// Generators are fibers, and the fiber pool plus the 8-live limit are
/// process-wide statics. `cargo test` runs checks on several threads, so every
/// check that can hold a suspended generator takes this lock: otherwise one
/// check's live fibers are counted against another's and the grid measures the
/// test runner instead of the interpreter. (The freestanding interpreter has
/// no threads, so this is a harness concern only.)
static FIBER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fiber_guard() -> std::sync::MutexGuard<'static, ()> {
    FIBER_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn describe(error: &PyError) -> String {
    if error.message.is_empty() {
        error.type_name().to_string()
    } else {
        format!("{}: {}", error.type_name(), error.message)
    }
}

fn run_source(source: &str, directory: Option<&str>) -> Result<Interp, String> {
    let module = parser::parse(source).map_err(|error| {
        format!(
            "syntax error at {}:{}: {}",
            error.line, error.column, error.message
        )
    })?;
    let mut interpreter = Interp::new(Vec::new(), directory.map(String::from));
    interpreter
        .run_module(&module.body, "<selftest>")
        .map_err(|error| describe(&error))?;
    Ok(interpreter)
}

fn expect_repr(source: &str, name: &str, expected: &str) -> Result<(), String> {
    expect_repr_in(source, name, expected, None)
}

fn expect_repr_in(
    source: &str,
    name: &str,
    expected: &str,
    directory: Option<&str>,
) -> Result<(), String> {
    let interpreter = run_source(source, directory)?;
    match env_lookup(&interpreter.module_env, name) {
        Some(value) => {
            let actual = value.to_repr();
            if actual == expected {
                Ok(())
            } else {
                Err(format!("{}: expected {}, got {}", name, expected, actual))
            }
        }
        None => Err(format!("{} was not defined", name)),
    }
}

fn expect_syntax_error(source: &str, needle: &str) -> Result<(), String> {
    match parser::parse(source) {
        Ok(_) => Err(format!(
            "expected a syntax error containing {:?}, but the source parsed",
            needle
        )),
        Err(error) => {
            if error.message.contains(needle) {
                Ok(())
            } else {
                Err(format!(
                    "expected {:?} in the syntax error, got {:?}",
                    needle, error.message
                ))
            }
        }
    }
}

fn arithmetic() -> Result<(), String> {
    expect_repr("x = 2 + 3 * 4", "x", "14")?;
    expect_repr("x = 7 // 2", "x", "3")?;
    expect_repr("x = -7 // 2", "x", "-4")?;
    expect_repr("x = 7 % -2", "x", "-1")?;
    expect_repr("x = 2 ** 10", "x", "1024")?;
    expect_repr("x = divmod(17, 5)", "x", "(3, 2)")?;
    expect_repr("x = pow(2, 10, 1000)", "x", "24")?;
    expect_repr("x = abs(-9)", "x", "9")?;
    expect_repr("x = hex(255)", "x", "'0xff'")?;
    Ok(())
}

fn floats() -> Result<(), String> {
    // literals: decimal point, leading dot, exponent, underscores, signs
    expect_repr("x = 1.5", "x", "1.5")?;
    expect_repr("x = .5", "x", "0.5")?;
    expect_repr("x = 1e3", "x", "1000.0")?;
    expect_repr("x = 1.5e-2", "x", "0.015")?;
    expect_repr("x = 1.", "x", "1.0")?;
    expect_repr("x = 1_0.5", "x", "10.5")?;
    expect_repr("x = -2.5 + 1.25", "x", "-1.25")?;

    // true division always returns a float; floor division keeps the type
    expect_repr("x = 1 / 2", "x", "0.5")?;
    expect_repr("x = 7 / 2", "x", "3.5")?;
    expect_repr("x = 7 // 2", "x", "3")?;
    expect_repr("x = 7.0 // 2", "x", "3.0")?;
    expect_repr("x = -7.0 // 2", "x", "-4.0")?;
    expect_repr("x = 7.5 % 2", "x", "1.5")?;
    expect_repr("x = -7.5 % 2", "x", "0.5")?;
    expect_repr("x = 7.5 % -2", "x", "-0.5")?;
    expect_repr("x = 2 ** -1", "x", "0.5")?;
    expect_repr("x = 2.0 ** 0.5", "x", "1.4142135623730951")?;
    expect_repr("x = (-2.0) ** 3", "x", "-8.0")?;
    expect_repr("x = abs(-1.5)", "x", "1.5")?;

    // conversions and truthiness
    expect_repr("x = int(-3.99)", "x", "-3")?;
    expect_repr("x = float(3)", "x", "3.0")?;
    expect_repr("x = float(True)", "x", "1.0")?;
    expect_repr(r#"x = float("  -1.5e2 ")"#, "x", "-150.0")?;
    expect_repr("x = bool(0.0)", "x", "False")?;
    expect_repr("x = bool(-0.0)", "x", "False")?;
    expect_repr("x = bool(1.5)", "x", "True")?;

    // shortest round-trip formatting, Python exponent spelling
    expect_repr("x = str(1.0)", "x", "'1.0'")?;
    expect_repr("x = repr(1.0)", "x", "'1.0'")?;
    expect_repr("x = str(1e20)", "x", "'1e+20'")?;
    expect_repr("x = str(1e-7)", "x", "'1e-07'")?;
    expect_repr("x = str(1.5e-7)", "x", "'1.5e-07'")?;
    expect_repr("x = str(0.1 + 0.2)", "x", "'0.30000000000000004'")?;
    expect_repr("x = str(-0.0)", "x", "'-0.0'")?;

    // mixed comparisons and exact int/float equality above 2^53
    expect_repr("x = 1 == 1.0", "x", "True")?;
    expect_repr("x = 1 < 1.5", "x", "True")?;
    expect_repr("x = 9007199254740993 == 9007199254740992.0", "x", "False")?;
    expect_repr("x = 9007199254740993 > 9007199254740992.0", "x", "True")?;
    expect_repr(r#"x = float("nan") == float("nan")"#, "x", "False")?;
    expect_repr(r#"x = float("nan") < 1.0"#, "x", "False")?;

    // round: int/float, optional digits, ties to even
    expect_repr("x = round(2.5)", "x", "2")?;
    expect_repr("x = round(3.5)", "x", "4")?;
    expect_repr("x = round(-2.5)", "x", "-2")?;
    expect_repr("x = round(1.5, 0)", "x", "2.0")?;
    expect_repr("x = round(2.675, 2)", "x", "2.67")?;
    expect_repr("x = round(1234, -2)", "x", "1200")?;
    expect_repr("x = round(15, -1)", "x", "20")?;

    // math module
    expect_repr("import math\nx = math.pi", "x", "3.141592653589793")?;
    expect_repr("import math\nx = math.e", "x", "2.718281828459045")?;
    expect_repr("import math\nx = math.sqrt(2)", "x", "1.4142135623730951")?;
    expect_repr("import math\nx = math.sqrt(4)", "x", "2.0")?;
    expect_repr("import math\nx = math.floor(-2.1)", "x", "-3")?;
    expect_repr("import math\nx = math.ceil(-2.1)", "x", "-2")?;
    expect_repr("import math\nx = math.trunc(-2.9)", "x", "-2")?;
    expect_repr("import math\nx = math.fabs(-2.5)", "x", "2.5")?;
    expect_repr("import math\nx = math.fmod(-7.5, 2)", "x", "-1.5")?;
    expect_repr("import math\nx = math.pow(-2, 3)", "x", "-8.0")?;
    expect_repr("import math\nx = math.sin(0)", "x", "0.0")?;
    expect_repr("import math\nx = math.cos(0)", "x", "1.0")?;
    expect_repr("import math\nx = math.log(math.e)", "x", "1.0")?;
    expect_repr("import math\nx = math.log10(1000)", "x", "3.0")?;
    expect_repr("import math\nx = math.exp(0)", "x", "1.0")?;
    expect_repr("import math\nx = math.isnan(math.nan)", "x", "True")?;
    expect_repr("import math\nx = math.isinf(math.inf)", "x", "True")?;
    expect_repr("import sys\nx = sys.float_info.dig", "x", "15")?;
    Ok(())
}

fn strings() -> Result<(), String> {
    expect_repr(r#"x = "Hello, World".lower()"#, "x", "'hello, world'")?;
    expect_repr(r#"x = "Hello".upper()"#, "x", "'HELLO'")?;
    expect_repr(r#"x = "-".join(["a", "b", "c"])"#, "x", "'a-b-c'")?;
    expect_repr(r#"x = "  hi  ".strip()"#, "x", "'hi'")?;
    expect_repr(r#"x = "a,b,c".split(",")"#, "x", "['a', 'b', 'c']")?;
    expect_repr(r#"x = "  a   b  ".split(None, 1)"#, "x", "['a', 'b  ']")?;
    expect_repr(r#"x = "abcabc".find("bc")"#, "x", "1")?;
    expect_repr(r#"x = "abcabc".count("bc")"#, "x", "2")?;
    expect_repr(r#"x = "abc".startswith(("x", "a"))"#, "x", "True")?;
    expect_repr(r#"x = "abc".endswith("bc")"#, "x", "True")?;
    expect_repr(r#"x = "a1".isdigit()"#, "x", "False")?;
    expect_repr(r#"x = "hello world".title()"#, "x", "'Hello World'")?;
    expect_repr(r#"x = "a\nb\n".splitlines()"#, "x", "['a', 'b']")?;
    Ok(())
}

fn lists() -> Result<(), String> {
    expect_repr("a = [3, 1, 2]\na.sort()\nx = a", "x", "[1, 2, 3]")?;
    expect_repr(
        "a = [3, 1, 2]\na.sort(reverse=True)\nx = a",
        "x",
        "[3, 2, 1]",
    )?;
    expect_repr(
        "a = [\"bb\", \"a\", \"ccc\"]\na.sort(key=len)\nx = a",
        "x",
        "['a', 'bb', 'ccc']",
    )?;
    expect_repr("a = [1, 2]\na.extend([3, 4])\nx = a", "x", "[1, 2, 3, 4]")?;
    expect_repr("a = [1, 2, 3]\nx = a.pop(0)", "x", "1")?;
    expect_repr("a = [1, 2, 3]\na.insert(1, 9)\nx = a", "x", "[1, 9, 2, 3]")?;
    expect_repr("a = [1, 2, 1]\na.remove(1)\nx = a", "x", "[2, 1]")?;
    expect_repr("a = [1, 2]\nx = a.copy()", "x", "[1, 2]")?;
    Ok(())
}

fn dicts() -> Result<(), String> {
    expect_repr(
        "d = {\"a\": 1}\nd[\"b\"] = 2\nx = sorted(d.keys())",
        "x",
        "['a', 'b']",
    )?;
    expect_repr("d = {\"a\": 1}\nx = d.get(\"z\", 9)", "x", "9")?;
    expect_repr("d = {\"a\": 1}\nx = d.setdefault(\"b\", 2)", "x", "2")?;
    expect_repr(
        "d = {\"a\": 1}\nd.update({\"b\": 2})\nx = sorted(d.items())",
        "x",
        "[('a', 1), ('b', 2)]",
    )?;
    expect_repr("d = {\"a\": 1, \"b\": 2}\nx = d.pop(\"a\")", "x", "1")?;
    expect_repr("d = {\"a\": 1}\nx = d.copy()", "x", "{'a': 1}")?;
    Ok(())
}

fn slicing() -> Result<(), String> {
    expect_repr("s = \"abcdef\"\nx = s[1:4]", "x", "'bcd'")?;
    expect_repr("x = [0, 1, 2, 3][::-1]", "x", "[3, 2, 1, 0]")?;
    expect_repr("x = (1, 2, 3)[1:]", "x", "(2, 3)")?;
    expect_repr("x = list(range(3))", "x", "[0, 1, 2]")?;
    expect_repr(
        "x = list(enumerate([\"a\", \"b\"], 1))",
        "x",
        "[(1, 'a'), (2, 'b')]",
    )?;
    expect_repr("x = list(zip([1, 2], [3, 4]))", "x", "[(1, 3), (2, 4)]")?;
    Ok(())
}

fn functions() -> Result<(), String> {
    expect_repr(
        "def fact(n):\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)\nx = fact(6)",
        "x",
        "720",
    )?;
    expect_repr(
        "def add(a, b=2):\n    return a + b\nx = add(3) + add(3, 4)",
        "x",
        "12",
    )?;
    Ok(())
}

fn variadics() -> Result<(), String> {
    expect_repr(
        "def gather(*args, **kwargs):\n    return (args, kwargs)\nx = gather(1, 2, name=\"n\")",
        "x",
        "((1, 2), {'name': 'n'})",
    )?;
    expect_repr(
        "def mixed(a, b=10, *rest, **kw):\n    return (a, b, rest, kw)\nx = mixed(1, 2, 3, 4, z=5)",
        "x",
        "(1, 2, (3, 4), {'z': 5})",
    )?;
    expect_repr(
        "def add(a, b):\n    return a + b\nx = (add(*[1, 2]), add(**{\"a\": 3, \"b\": 4}), add(*[1], **{\"b\": 5}))",
        "x",
        "(3, 7, 6)",
    )?;
    expect_repr(
        "def f(a, b, c=0):\n    return a + b + c\nx = f(*[1, 2], c=3)",
        "x",
        "6",
    )?;
    expect_repr(
        "f = lambda *args, **kwargs: (args, kwargs)\nx = f(1, k=2)",
        "x",
        "((1,), {'k': 2})",
    )?;
    expect_repr(
        "def empty(*args, **kwargs):\n    return (args, kwargs)\nx = empty()",
        "x",
        "((), {})",
    )?;
    Ok(())
}

fn formatting() -> Result<(), String> {
    expect_repr(
        r#"x = "{} {}".format("a", 1)"#,
        "x",
        "'a 1'",
    )?;
    expect_repr(
        r#"x = "{0}-{1}-{0}".format("a", "b")"#,
        "x",
        "'a-b-a'",
    )?;
    expect_repr(
        r#"x = "{name}={value}".format(name="n", value=3)"#,
        "x",
        "'n=3'",
    )?;
    expect_repr(r#"x = "{{}} {}".format(5)"#, "x", "'{} 5'")?;
    expect_repr(r#"x = "{:.2f}".format(3.14159)"#, "x", "'3.14'")?;
    expect_repr(r#"x = "{0[1]}".format([10, 20, 30])"#, "x", "'20'")?;
    expect_repr(
        r#"x = "{d[z]}".format(d={"z": 9})"#,
        "x",
        "'9'",
    )?;
    expect_repr(
        r#"class P:
    def __init__(self):
        self.name = "Rex"
p = P()
x = "{p.name}".format(p=p)"#,
        "x",
        "'Rex'",
    )?;
    expect_repr(r#"x = "{:>6}".format("x")"#, "x", "'     x'")?;
    expect_repr(r#"x = "{0!r}".format("x")"#, "x", "'\\'x\\''")?;
    Ok(())
}

fn classes() -> Result<(), String> {
    expect_repr(
        "class Animal:\n    def __init__(self, name):\n        self.name = name\n    def speak(self):\n        return self.name + \" makes a sound\"\nclass Dog(Animal):\n    def speak(self):\n        return self.name + \" barks\"\nd = Dog(\"Rex\")\nx = d.speak()\ny = isinstance(d, Animal)",
        "x",
        "'Rex barks'",
    )?;
    let interpreter = run_source(
        "class Animal:\n    pass\nclass Dog(Animal):\n    pass\nx = isinstance(Dog(), Animal)",
        None,
    )?;
    match env_lookup(&interpreter.module_env, "x") {
        Some(Value::Bool(true)) => Ok(()),
        other => Err(format!(
            "isinstance result: {:?}",
            other.map(|v| v.to_repr())
        )),
    }
}

fn exceptions() -> Result<(), String> {
    expect_repr(
        "try:\n    raise ValueError(\"bad\")\nexcept ValueError as error:\n    x = str(error)",
        "x",
        "'bad'",
    )?;
    expect_repr(
        "try:\n    1 // 0\nexcept ZeroDivisionError:\n    x = \"caught\"",
        "x",
        "'caught'",
    )?;
    expect_repr(
        "def fail():\n    raise RuntimeError(\"boom\")\ntry:\n    fail()\nexcept RuntimeError as error:\n    x = str(error)",
        "x",
        "'boom'",
    )?;
    Ok(())
}

fn uncaught() -> Result<(), String> {
    match run_source("def f():\n    return 1 // 0\nf()", None) {
        Err(message) if message.starts_with("ZeroDivisionError") => Ok(()),
        Err(message) => Err(format!("wrong error: {}", message)),
        Ok(_) => Err(String::from("expected an uncaught ZeroDivisionError")),
    }
}

fn files() -> Result<(), String> {
    let mut path = std::env::temp_dir();
    path.push(format!("pagh-python-selftest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("cannot create {}: {}", path.display(), error))?;
    let file = path.join("py.txt");
    let script = format!(
        "f = open(\"{}\", \"w\")\nf.write(\"hello\\n\")\nf.writelines([\"second\\n\"])\nf.close()\ng = open(\"{}\")\nx = g.read()\ny = g.readline()\ng.close()",
        file.display(),
        file.display()
    );
    let result = run_source(&script, None);
    if let Err(message) = result {
        let _ = std::fs::remove_dir_all(&path);
        return Err(message);
    }
    let contents = std::fs::read_to_string(&file)
        .map_err(|error| format!("cannot read {}: {}", file.display(), error))?;
    let _ = std::fs::remove_dir_all(&path);
    if contents != "hello\nsecond\n" {
        return Err(format!("unexpected file contents: {:?}", contents));
    }
    Ok(())
}

fn imports() -> Result<(), String> {
    let mut path = std::env::temp_dir();
    path.push(format!("pagh-python-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("cannot create {}: {}", path.display(), error))?;
    std::fs::write(
        path.join("mymod.py"),
        "answer = 42\ndef double(x):\n    return x * 2\n",
    )
    .map_err(|error| format!("cannot write module: {}", error))?;
    let directory = path.to_string_lossy().into_owned();
    let result = expect_repr_in(
        "import mymod\nx = mymod.answer + mymod.double(4)",
        "x",
        "50",
        Some(&directory),
    );
    let result = result.and_then(|_| {
        let interpreter = run_source("from mymod import double\nx = double(21)", Some(&directory))?;
        match env_lookup(&interpreter.module_env, "x") {
            Some(Value::Int(42)) => Ok(()),
            other => Err(format!("imported value: {:?}", other.map(|v| v.to_repr()))),
        }
    });
    let _ = std::fs::remove_dir_all(&path);
    result
}

fn lambdas() -> Result<(), String> {
    expect_repr("f = lambda x: x + 1\nx = f(4)", "x", "5")?;
    expect_repr("f = lambda x, y=10: x + y\nx = (f(5), f(5, 1))", "x", "(15, 6)")?;
    expect_repr(
        "def outer(n):\n    return lambda x: x + n\nf = outer(7)\nx = f(3)",
        "x",
        "10",
    )?;
    expect_repr(
        "x = sorted([\"ccc\", \"a\", \"bb\"], key=lambda value: len(value))",
        "x",
        "['a', 'bb', 'ccc']",
    )?;
    expect_repr(
        "x = list(map(lambda value: value * 2, [1, 2, 3]))",
        "x",
        "[2, 4, 6]",
    )?;
    expect_repr(
        "x = list(map(lambda a, b: a + b, [1, 2], [10, 20, 30]))",
        "x",
        "[11, 22]",
    )?;
    expect_repr("x = (lambda: 42)()", "x", "42")?;
    Ok(())
}

fn with_statement() -> Result<(), String> {
    let mut path = std::env::temp_dir();
    path.push(format!("pagh-python-with-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("cannot create {}: {}", path.display(), error))?;
    let file = path.join("with.txt");
    let script = format!(
        r#"p = "{file}"
with open(p, "w") as out:
    out.write("alpha\nbeta\n")
with open(p) as source:
    x = source.read()
y = open(p).read()
with open(p, "w") as out:
    out.write("closed")
z = source.closed"#,
        file = file.display()
    );
    let interpreter = run_source(&script, None)?;
    for (name, expected) in [
        ("x", "'alpha\\nbeta\\n'"),
        ("y", "'alpha\\nbeta\\n'"),
        ("z", "True"),
    ] {
        match env_lookup(&interpreter.module_env, name) {
            Some(value) => {
                let actual = value.to_repr();
                if actual != expected {
                    let _ = std::fs::remove_dir_all(&path);
                    return Err(format!("{}: expected {}, got {}", name, expected, actual));
                }
            }
            None => {
                let _ = std::fs::remove_dir_all(&path);
                return Err(format!("{} was not defined", name));
            }
        }
    }
    let contents = std::fs::read_to_string(&file)
        .map_err(|error| format!("cannot read {}: {}", file.display(), error))?;
    let _ = std::fs::remove_dir_all(&path);
    if contents != "closed" {
        return Err(format!("unexpected file contents: {:?}", contents));
    }
    expect_repr(
        r#"class Box:
    def __init__(self):
        self.events = []
    def __enter__(self):
        self.events.append("enter")
        return self
    def __exit__(self, kind, value, tb):
        self.events.append((kind, value, tb))
        return False
b = Box()
with b:
    b.events.append("body")
x = b.events"#,
        "x",
        "['enter', 'body', (None, None, None)]",
    )?;
    expect_repr(
        r#"class Suppress:
    def __enter__(self):
        return self
    def __exit__(self, kind, value, tb):
        self.seen = (kind.__name__, str(value))
        return True
with Suppress() as s:
    raise ValueError("boom")
x = s.seen"#,
        "x",
        "('ValueError', 'boom')",
    )?;
    expect_repr(
        r#"class Name:
    def __init__(self, value):
        self.value = value
    def __enter__(self):
        return self.value
    def __exit__(self, kind, value, tb):
        return False
with Name("a") as a, Name("b") as b:
    x = a + b"#,
        "x",
        "'ab'",
    )?;
    Ok(())
}

// ── the fiber primitive ────────────────────────────────────────────────────
//
// The generator tests exercise these coroutines through the evaluator; this
// one drives the switch itself, so a broken save/restore is named as such
// instead of surfacing as a confusing generator result.

use crate::fiber::Fiber;
use core::ptr;

static mut FIBER_SLOT: *mut Fiber = ptr::null_mut();
static mut FIBER_LOG: [i64; 8] = [0; 8];
static mut FIBER_STEPS: usize = 0;

fn fiber_steps() -> usize {
    // SAFETY: single-threaded test, no reference outlives the call.
    unsafe { ptr::read_volatile(ptr::addr_of!(FIBER_STEPS)) }
}

fn fiber_log(index: usize) -> i64 {
    // SAFETY: as `fiber_steps`; `index` is always in bounds.
    unsafe { ptr::read_volatile(ptr::addr_of!(FIBER_LOG).cast::<i64>().add(index)) }
}

unsafe extern "C" fn fiber_probe(_argument: *mut u8) -> ! {
    let fiber = ptr::read_volatile(ptr::addr_of!(FIBER_SLOT));
    for value in 0..3i64 {
        ptr::write_volatile(ptr::addr_of_mut!(FIBER_LOG).cast::<i64>().add(fiber_steps()), value);
        ptr::write_volatile(ptr::addr_of_mut!(FIBER_STEPS), fiber_steps() + 1);
        (*fiber).yield_to_caller();
    }
    (*fiber).finished = true;
    (*fiber).yield_to_caller();
    loop {
        (*fiber).yield_to_caller();
    }
}

fn fibers() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // SAFETY: a single fiber is alive and one thread runs the test; the fiber
    // only reaches statics this test owns.
    unsafe {
        let mut fiber = Fiber::new().ok_or_else(|| String::from("no fiber capacity"))?;
        ptr::write_volatile(ptr::addr_of_mut!(FIBER_SLOT), &mut *fiber as *mut Fiber);
        fiber.start(fiber_probe, ptr::null_mut());
        if fiber_steps() != 1 || fiber_log(0) != 0 {
            return Err(format!("first entry logged {} steps", fiber_steps()));
        }
        if fiber.finished {
            return Err(String::from("fiber finished after the first yield"));
        }
        fiber.resume();
        fiber.resume();
        if fiber_steps() != 3 {
            return Err(format!("expected 3 yields, got {}", fiber_steps()));
        }
        fiber.resume();
        if !fiber.finished {
            return Err(String::from("fiber did not report completion"));
        }
        let log = [fiber_log(0), fiber_log(1), fiber_log(2)];
        if log != [0, 1, 2] {
            return Err(format!("fiber log is {:?}", log));
        }
        // A finished fiber keeps its stack only until it is dropped; dropping
        // must return it to the pool and keep the live count honest.
        drop(fiber);
        let mut second = Fiber::new().ok_or_else(|| String::from("pool did not recycle"))?;
        second.finished = true;
        drop(second);
    }
    Ok(())
}

fn generators() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // Basic lazy iteration, `next`, and the StopIteration contract.
    expect_repr(
        "def counter(n):\n    i = 0\n    while i < n:\n        yield i\n        i += 1\nx = list(counter(3))",
        "x",
        "[0, 1, 2]",
    )?;
    expect_repr(
        "def counter(n):\n    i = 0\n    while i < n:\n        yield i\n        i += 1\ng = counter(3)\nx = (next(g), next(g), next(g))",
        "x",
        "(0, 1, 2)",
    )?;
    expect_repr(
        "def counter(n):\n    i = 0\n    while i < n:\n        yield i\n        i += 1\ng = counter(1)\nnext(g)\ntry:\n    next(g)\n    x = 'no error'\nexcept StopIteration:\n    x = 'stopped'",
        "x",
        "'stopped'",
    )?;
    // `for ... else` over a generator, and the default of `next(g, default)`.
    expect_repr(
        "def two():\n    yield 1\n    yield 2\nseen = []\nfor value in two():\n    seen.append(value)\nelse:\n    seen.append('else')\nx = seen",
        "x",
        "[1, 2, 'else']",
    )?;
    expect_repr(
        "def one():\n    yield 1\ng = one()\nnext(g)\nx = next(g, 'default')",
        "x",
        "'default'",
    )?;
    // Real laziness: the loop never ends, so only a lazy generator finishes.
    expect_repr(
        "def naturals():\n    n = 0\n    while True:\n        yield n\n        n += 1\ng = naturals()\nx = [next(g) for _ in range(4)]\ndel g",
        "x",
        "[0, 1, 2, 3]",
    )?;
    // A `yield` expression evaluates to the value the caller sent (None).
    expect_repr(
        "def echo():\n    first = yield 1\n    yield first\ng = echo()\na = next(g)\nb = next(g)\ndel g\nx = (a, b)",
        "x",
        "(1, None)",
    )?;
    // `finally` runs when the generator is exhausted.
    expect_repr(
        "log = []\ndef guarded():\n    try:\n        yield 'a'\n    finally:\n        log.append('cleanup')\nx = (list(guarded()), log)",
        "x",
        "(['a'], ['cleanup'])",
    )?;
    // An exception inside the generator surfaces from `next()` and ends it.
    expect_repr(
        "def boom():\n    yield 1\n    raise ValueError('bad')\ng = boom()\nnext(g)\ntry:\n    next(g)\n    x = 'no error'\nexcept ValueError as error:\n    x = str(error)",
        "x",
        "'bad'",
    )?;
    // Generators nest: the inner one is resumed from inside the outer one.
    expect_repr(
        "def inner():\n    yield 1\n    yield 2\ndef outer():\n    for value in inner():\n        yield value * 10\nx = list(outer())",
        "x",
        "[10, 20]",
    )?;
    // A generator is its own iterator: `iter(g) is g`, and it is not a list.
    expect_repr(
        "def one():\n    yield 1\ng = one()\nx = (iter(g) is g, type(g).__name__)\ndel g",
        "x",
        "(True, 'generator')",
    )?;
    // Calling a generator function binds its parameters; a bad call is a
    // TypeError at call time, before any body runs.
    expect_repr(
        "def one(a):\n    yield a\ntry:\n    one()\n    x = 'no error'\nexcept TypeError:\n    x = 'arity'",
        "x",
        "'arity'",
    )?;
    // Re-entering a running generator is refused instead of corrupting it.
    expect_repr(
        "me = None\ndef selfish():\n    yield next(me)\nme = selfish()\ntry:\n    next(me)\n    x = 'no error'\nexcept ValueError as error:\n    x = str(error)\ndel me",
        "x",
        "'generator already executing'",
    )?;
    // The stack of an exhausted generator is recycled: this loop creates and
    // finishes 300 generators without running out of fibers.
    expect_repr(
        "def pair():\n    yield 1\n    yield 2\ntotal = 0\nfor _ in range(300):\n    for value in pair():\n        total += value\nx = total",
        "x",
        "900",
    )?;
    Ok(())
}

fn generator_expressions() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    expect_repr("x = list(v * v for v in range(4))", "x", "[0, 1, 4, 9]")?;
    expect_repr("x = sum(v for v in range(6) if v % 2 == 0)", "x", "6")?;
    // The parenthesized form is a lazy iterator, not a list.
    expect_repr("x = type(v for v in range(2)).__name__", "x", "'generator'")?;
    expect_repr("x = sorted(v for v in [3, 1, 2])", "x", "[1, 2, 3]")?;
    expect_repr(
        "x = list((a, b) for a in range(2) for b in range(2))",
        "x",
        "[(0, 0), (0, 1), (1, 0), (1, 1)]",
    )?;
    // Laziness over an unbounded source.
    expect_repr(
        "def naturals():\n    n = 0\n    while True:\n        yield n\n        n += 1\ng = (v * 2 for v in naturals())\nx = [next(g) for _ in range(3)]\ndel g",
        "x",
        "[0, 2, 4]",
    )?;
    // A generator expression captures the surrounding scope at creation time.
    expect_repr(
        "def make(limit):\n    return (v for v in range(limit))\nx = list(make(3))",
        "x",
        "[0, 1, 2]",
    )?;
    // ... and it is consumed only once.
    expect_repr(
        "g = (v for v in range(3))\nx = (list(g), list(g))",
        "x",
        "([0, 1, 2], [])",
    )?;
    // `yield` in a comprehension is a compile-time error, as in CPython. The
    // element is parsed before the `for` is known, so this is the finished-tree
    // check rather than the clause guard.
    expect_syntax_error(
        "def f():\n    return [yield 1 for _ in range(2)]",
        "'yield' inside a comprehension",
    )?;
    expect_syntax_error(
        "def f():\n    return {i: (yield i) for i in range(2)}",
        "'yield' inside a comprehension",
    )?;
    expect_syntax_error(
        "def f():\n    return (yield from [1] for _ in range(2))",
        "'yield' inside a comprehension",
    )?;
    Ok(())
}

fn yield_from() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // Delegation to another generator function: the delegated values come out
    // of the outer generator in order, between the outer `yield`s.
    expect_repr(
        "def inner():\n    yield 1\n    yield 2\ndef outer():\n    yield 0\n    yield from inner()\n    yield 3\nx = list(outer())",
        "x",
        "[0, 1, 2, 3]",
    )?;
    // The operand is any iterable, not only a generator: a range, a generator
    // expression, and an instance whose `__iter__` returns an iterator.
    expect_repr(
        "def rng():\n    yield from range(3)\nx = list(rng())",
        "x",
        "[0, 1, 2]",
    )?;
    expect_repr(
        "def genexp():\n    yield from (v * 2 for v in range(3))\nx = list(genexp())",
        "x",
        "[0, 2, 4]",
    )?;
    expect_repr(
        "class Box:\n    def __iter__(self):\n        return iter([7, 8])\ndef take():\n    yield from Box()\nx = list(take())",
        "x",
        "[7, 8]",
    )?;
    // `__iter__` may also be a generator function: a generator is an iterator
    // (`iter(g) is g`), so both values are legal iterator protocol results.
    expect_repr(
        "class Box:\n    def __iter__(self):\n        yield 'a'\n        yield 'b'\ndef take():\n    yield from Box()\nx = list(take())",
        "x",
        "['a', 'b']",
    )?;
    // The delegated expression evaluates to None (there is no generator
    // `return` value), so `got = yield from ...` binds None after the loop.
    expect_repr(
        "def f():\n    got = yield from [1]\n    yield got\nx = list(f())",
        "x",
        "[1, None]",
    )?;
    // Two levels of delegation, and laziness: nothing past the first value is
    // produced until the resumer asks for it.
    expect_repr(
        "def inner():\n    yield 1\n    yield 2\ndef middle():\n    yield from inner()\ndef outer():\n    yield from middle()\ng = outer()\nx = (next(g), next(g), next(g, 'done'))\ndel g",
        "x",
        "(1, 2, 'done')",
    )?;
    // A non-iterable operand is a TypeError from the iterator protocol, and it
    // surfaces from the first `next()` rather than at definition time.
    expect_repr(
        "def f():\n    yield from 5\ntry:\n    list(f())\n    x = 'no error'\nexcept TypeError as error:\n    x = (type(error).__name__, 'not iterable' in str(error))",
        "x",
        "('TypeError', True)",
    )?;
    // Syntax: delegation needs a `def` body and an operand.
    expect_syntax_error(
        "yield from [1]",
        "'yield' outside a function",
    )?;
    expect_syntax_error(
        "def f():\n    yield from",
        "expected an expression",
    )?;
    Ok(())
}

fn generator_send() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // `send(value)` makes the suspended `yield` expression evaluate to it, and
    // sending into a generator whose body already ended is a StopIteration.
    expect_repr(
        "def echo():\n    first = yield 'ready'\n    second = yield first\n    return second\ng = echo()\na = g.send(None)\nb = g.send('one')\ntry:\n    g.send('two')\n    x = 'no error'\nexcept StopIteration:\n    x = (a, b) + ('stopped',)\ndel g",
        "x",
        "('ready', 'one', 'stopped')",
    )?;
    // `send(None)` is `next()`; any other value delivered to a generator that
    // has not started is a TypeError, exactly as in CPython.
    expect_repr(
        "def one():\n    yield 'first'\ng = one()\na = g.send(None)\ng2 = one()\ntry:\n    g2.send(1)\n    x = 'no error'\nexcept TypeError as error:\n    x = (a, str(error))\ndel g",
        "x",
        "('first', 'can\\'t send non-None value to a just-started generator')",
    )?;
    // A generator driven entirely by send; the value replaces `yield`'s result.
    expect_repr(
        "def counter():\n    total = 0\n    while True:\n        value = yield total\n        total += value\ng = counter()\na = next(g)\nb = g.send(2)\nc = g.send(3)\ng.close()\nx = (a, b, c)",
        "x",
        "(0, 2, 5)",
    )?;
    // Re-entering a running generator through `send` is refused like `next`.
    expect_repr(
        "me = None\ndef selfish():\n    yield me.send('x')\nme = selfish()\ntry:\n    me.send(None)\n    x = 'no error'\nexcept ValueError as error:\n    x = str(error)\ndel me",
        "x",
        "'generator already executing'",
    )?;
    // A generator expression has no name for the sent value: it is discarded
    // (CPython does the same), while the iteration itself is unaffected.
    expect_repr(
        "g = (v for v in range(3))\na = next(g)\nb = g.send(99)\nc = next(g)\nd = list(g)\ndel g\nx = (a, b, c, d)",
        "x",
        "(0, 1, 2, [])",
    )?;
    Ok(())
}

fn generator_throw() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // A caught `throw` is raised at the suspended `yield`; if the body then
    // returns, `throw` itself reports the normal end as StopIteration.
    expect_repr(
        "def catcher():\n    try:\n        yield 'ready'\n    except ValueError as error:\n        return 'caught ' + str(error)\ng = catcher()\na = next(g)\nmessage = ''\ntry:\n    g.throw(ValueError('boom'))\nexcept StopIteration:\n    message = 'stopped'\nx = (a, message)",
        "x",
        "('ready', 'stopped')",
    )?;
    // An exception that the body does not catch escapes from `throw`.
    expect_repr(
        "def plain():\n    yield 1\ng = plain()\nnext(g)\ntry:\n    g.throw(RuntimeError('escaping'))\n    x = 'no error'\nexcept RuntimeError as error:\n    x = str(error)",
        "x",
        "'escaping'",
    )?;
    // The accepted forms: an instance, a class, and a class with arguments
    // (a tuple is unpacked, a single value is passed as one argument).
    expect_repr(
        "def catcher():\n    try:\n        yield 'ready'\n    except KeyError:\n        yield 'key'\n    except ValueError as error:\n        yield ('value', error.args)\ng = catcher()\nnext(g)\na = g.throw(KeyError)\ng.close()\ng = catcher()\nnext(g)\nb = g.throw(ValueError, ('a', 'b'))\ng.close()\ng = catcher()\nnext(g)\nc = g.throw(ValueError('instance'))\ng.close()\nx = (a, b, c)",
        "x",
        "('key', ('value', ('a', 'b')), ('value', ('instance',)))",
    )?;
    // Throwing into a generator that never started does not run the body: the
    // exception is raised in the caller and the generator is closed.
    expect_repr(
        "log = []\ndef guarded():\n    try:\n        yield 1\n    finally:\n        log.append('finally')\ng = guarded()\ntry:\n    g.throw(RuntimeError('early'))\nexcept RuntimeError:\n    pass\nx = (log, next(g, 'done'))",
        "x",
        "([], 'done')",
    )?;
    // Throwing into an exhausted generator raises the given exception in the
    // caller (CPython 3.13+ behaviour), unlike `close`/`send`.
    expect_repr(
        "def one():\n    yield 1\ng = one()\nlist(g)\ntry:\n    g.throw(ValueError('late'))\n    x = 'no error'\nexcept ValueError as error:\n    x = str(error)",
        "x",
        "'late'",
    )?;
    // Arity and type errors: no arguments, a non-exception, and the
    // three-argument form this interpreter does not implement.
    expect_repr(
        "def one():\n    yield 1\ng = one()\nnext(g)\ntry:\n    g.throw()\nexcept TypeError as error:\n    a = str(error)\ntry:\n    g.throw(5)\nexcept TypeError as error:\n    b = str(error)\ntry:\n    g.throw(ValueError, 'v', 'tb')\nexcept TypeError as error:\n    c = str(error)\ng.close()\nx = (a, b, c)",
        "x",
        "('throw expected at least 1 argument, got 0', 'exceptions must derive from BaseException, not \\'int\\'', 'throw(type, value, traceback) is not supported: pass an exception instance or a class and its arguments')",
    )?;
    // PEP 479: a StopIteration escaping the body is an error, not the end of
    // the iteration, so it becomes RuntimeError("generator raised
    // StopIteration") both when thrown in and when raised by the body.
    expect_repr(
        "def plain():\n    yield 1\np = plain()\nnext(p)\ntry:\n    p.throw(StopIteration('stop'))\n    x = 'no error'\nexcept RuntimeError as error:\n    x = str(error)",
        "x",
        "'generator raised StopIteration'",
    )?;
    expect_repr(
        "def raises():\n    yield 1\n    raise StopIteration('x')\ng = raises()\nnext(g)\ntry:\n    next(g)\n    x = 'no error'\nexcept RuntimeError as error:\n    x = str(error)",
        "x",
        "'generator raised StopIteration'",
    )?;
    // `throw(class, value)` uses the value as-is when it already is an
    // instance of the class, instead of wrapping it again.
    expect_repr(
        "def catcher():\n    try:\n        yield 'ready'\n    except ValueError as error:\n        yield ('args', error.args)\ng = catcher()\nnext(g)\nx = g.throw(ValueError, ValueError('pre'))\ng.close()",
        "x",
        "('args', ('pre',))",
    )?;
    // CPython's `gen_throw` special case: throwing GeneratorExit through a
    // `yield from` whose delegate swallows it and yields means the delegate
    // "ignored GeneratorExit"; if the delegate returns instead, the
    // GeneratorExit is raised at this `yield from`.
    expect_repr(
        "log = []\ndef sub():\n    try:\n        yield 1\n    except GeneratorExit:\n        log.append('sub')\n        yield 'caught'\ndef outer():\n    try:\n        yield from sub()\n    finally:\n        log.append('outer')\ng = outer()\nnext(g)\ntry:\n    g.throw(GeneratorExit())\n    message = 'no error'\nexcept RuntimeError as error:\n    message = str(error)\nx = (message, log)",
        "x",
        "('generator ignored GeneratorExit', ['sub', 'outer'])",
    )?;
    expect_repr(
        "def sub():\n    try:\n        yield 1\n    except GeneratorExit:\n        return 'done'\ndef outer():\n    yield from sub()\ng = outer()\nnext(g)\ntry:\n    g.throw(GeneratorExit())\n    x = 'no error'\nexcept GeneratorExit:\n    x = 'exit'",
        "x",
        "'exit'",
    )?;
    // `throw(GeneratorExit())` is an ordinary injected exception when the
    // generator is *not* delegating: when it is not caught it reaches the
    // caller and ends the generator.
    expect_repr(
        "def plain():\n    yield 1\ng = plain()\nnext(g)\ntry:\n    g.throw(GeneratorExit())\n    x = 'no error'\nexcept GeneratorExit:\n    x = ('exit', next(g, 'done'))",
        "x",
        "('exit', 'done')",
    )?;
    Ok(())
}

fn generator_close() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // `close()` raises GeneratorExit at the `yield`, so `finally` runs and the
    // generator cannot be resumed again.
    expect_repr(
        "log = []\ndef guarded():\n    try:\n        yield 1\n        yield 2\n    finally:\n        log.append('cleanup')\ng = guarded()\na = next(g)\nr = g.close()\nafter = next(g, 'done')\nx = (a, r, log, after)",
        "x",
        "(1, None, ['cleanup'], 'done')",
    )?;
    // Closing a generator that never started does not run its body at all.
    expect_repr(
        "log = []\ndef guarded():\n    try:\n        yield 1\n    finally:\n        log.append('finally')\ng = guarded()\nr = g.close()\nx = (r, log, next(g, 'done'))",
        "x",
        "(None, [], 'done')",
    )?;
    // A body that swallows GeneratorExit and yields again is refused.
    expect_repr(
        "def stubborn():\n    try:\n        yield 1\n    except GeneratorExit:\n        yield 2\ng = stubborn()\nnext(g)\ntry:\n    g.close()\n    x = 'no error'\nexcept RuntimeError as error:\n    x = str(error)\ng.close()\ndel g",
        "x",
        "'generator ignored GeneratorExit'",
    )?;
    // `GeneratorExit` derives from BaseException, not Exception (as in
    // CPython), so an `except Exception` handler must not swallow a close.
    expect_repr(
        "def guarded():\n    try:\n        yield 1\n    except Exception:\n        yield 'caught by Exception'\ng = guarded()\nnext(g)\nr = g.close()\nx = (r, isinstance(GeneratorExit(), BaseException), isinstance(GeneratorExit(), Exception))",
        "x",
        "(None, True, False)",
    )?;
    // Closing an exhausted generator is a no-op, and a running one is refused.
    expect_repr(
        "def one():\n    yield 1\ng = one()\nlist(g)\na = g.close()\nme = None\ndef selfish():\n    yield me.close()\nme = selfish()\ntry:\n    next(me)\n    b = 'no error'\nexcept ValueError as error:\n    b = str(error)\ndel me\nx = (a, b)",
        "x",
        "(None, 'generator already executing')",
    )?;
    // A `yield` inside `finally` runs during close(), so the body yields
    // while closing and is refused.
    expect_repr(
        "def finally_yield():\n    try:\n        yield 1\n    finally:\n        yield 2\ng = finally_yield()\nnext(g)\ntry:\n    g.close()\n    x = 'no error'\nexcept RuntimeError as error:\n    x = str(error)\ng.close()\ndel g",
        "x",
        "'generator ignored GeneratorExit'",
    )?;
    // Close delegates to a `yield from`: a delegate that swallows
    // GeneratorExit and yields again makes the outer close a RuntimeError,
    // and the outer `finally` still runs.
    expect_repr(
        "log = []\ndef sub():\n    try:\n        yield 1\n    except GeneratorExit:\n        log.append('sub')\n        yield 'caught'\ndef outer():\n    try:\n        yield from sub()\n    finally:\n        log.append('outer')\ng = outer()\nnext(g)\ntry:\n    g.close()\n    message = 'no error'\nexcept RuntimeError as error:\n    message = str(error)\nx = (message, log)",
        "x",
        "('generator ignored GeneratorExit', ['sub', 'outer'])",
    )?;
    // A generator expression is a generator too: close() and throw() reach
    // its internal `yield`.
    expect_repr(
        "g = (v for v in range(4))\nnext(g)\na = g.close()\ng = (v for v in range(4))\nnext(g)\ntry:\n    g.throw(KeyError('genexp'))\n    b = 'no error'\nexcept KeyError:\n    b = 'keyerror'\nx = (a, b)",
        "x",
        "(None, 'keyerror')",
    )?;
    // The 8-suspended-generator limit is verified in the live script
    // (tests/live/gen_methods.py), where one clean process owns the pool: here
    // the leaked fibers of earlier checks make the count order-dependent.
    Ok(())
}

fn generator_delegation() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // A suspended generator owns a fiber slot until it is finished or
    // dropped; that leak is invisible from Python, so the check measures the
    // process-wide counter across the whole check and requires it to come
    // back to where it started.
    let fibers_before = crate::fiber::live_fibers();
    // `yield from` routes `send` to the delegated generator, and the value of
    // the delegated expression is the delegate's `return` value.
    expect_repr(
        "def sub():\n    while True:\n        v = yield\n        if v == 'stop':\n            return 'sub return'\ndef outer():\n    r = yield from sub()\n    yield r\ng = outer()\na = next(g)\nb = g.send('x')\nc = g.send('stop')\ng.close()\nx = (a, b, c)",
        "x",
        "(None, None, 'sub return')",
    )?;
    expect_repr(
        "def inner():\n    yield 1\n    return 'answer'\ndef outer():\n    r = yield from inner()\n    yield r\nx = list(outer())",
        "x",
        "[1, 'answer']",
    )?;
    // `throw` is routed to the delegate; a class argument works too.
    expect_repr(
        "def sub():\n    try:\n        yield 1\n    except ValueError as error:\n        yield 'handled ' + str(error)\ndef outer():\n    yield from sub()\ng = outer()\nnext(g)\na = g.throw(ValueError('bad'))\ng.close()\ndef sub2():\n    try:\n        yield 1\n    except KeyError:\n        yield 'key'\ndef outer2():\n    yield from sub2()\ng = outer2()\nnext(g)\nb = g.throw(KeyError)\ng.close()\nx = (a, b)",
        "x",
        "('handled bad', 'key')",
    )?;
    // `close` closes the delegate first (its `finally` runs), then raises
    // GeneratorExit at the outer `yield from`, so the outer `finally` runs too.
    expect_repr(
        "log = []\ndef inner():\n    try:\n        yield 10\n    finally:\n        log.append('inner')\ndef outer():\n    try:\n        yield from inner()\n    finally:\n        log.append('outer')\ng = outer()\nnext(g)\nr = g.close()\nx = (r, log)",
        "x",
        "(None, ['inner', 'outer'])",
    )?;
    // Delegation nests: the request travels down the whole chain.
    expect_repr(
        "def first():\n    v = yield 'deep'\n    yield ('first', v)\ndef second():\n    yield from first()\ndef third():\n    yield from second()\ng = third()\na = next(g)\nb = g.send('down')\ng.close()\nx = (a, b)",
        "x",
        "('deep', ('first', 'down'))",
    )?;
    // A plain iterator has no send/throw: sending a value is an AttributeError
    // raised at the `yield from`, and close still runs the outer `finally`.
    expect_repr(
        "def plain():\n    try:\n        yield from [1, 2]\n    except AttributeError as error:\n        yield 'send' in str(error)\ng = plain()\nnext(g)\na = g.send('value')\ng.close()\nlog = []\ndef closer():\n    try:\n        yield from [1, 2]\n    finally:\n        log.append('outer')\ng = closer()\nnext(g)\nr = g.close()\nx = (a, r, log)",
        "x",
        "(True, None, ['outer'])",
    )?;
    // Throwing into a `yield from` over a plain iterator raises at the outer
    // yield point, where the outer body can catch it.
    expect_repr(
        "def outer():\n    try:\n        yield from [1, 2]\n    except ValueError as error:\n        yield 'caught ' + str(error)\ng = outer()\nnext(g)\nx = g.throw(ValueError('plain'))\ng.close()",
        "x",
        "'caught plain'",
    )?;
    // The delegated `return` value is taken once: a second delegation to the
    // same exhausted generator sees None, like StopIteration.value.
    expect_repr(
        "def inner():\n    yield 1\n    return 'answer'\ndef capture():\n    got = yield from inner()\n    yield got\ndone = inner()\nlist(done)\ndef again():\n    got = yield from done\n    yield got\nx = (list(capture()), list(again()))",
        "x",
        "([1, 'answer'], [None])",
    )?;
    // A delegated generator that raises still propagates its exception.
    expect_repr(
        "def boom():\n    yield 1\n    raise ValueError('inner')\ndef wrap():\n    yield from boom()\ng = wrap()\nfirst = next(g)\ntry:\n    next(g)\n    x = (first, 'no error')\nexcept ValueError as error:\n    x = (first, str(error))",
        "x",
        "(1, 'inner')",
    )?;
    // Nothing above may leave a suspended generator (and its fiber slot)
    // behind.
    let fibers_after = crate::fiber::live_fibers();
    if fibers_after != fibers_before {
        return Err(format!(
            "fiber slots leaked: {} live before the check, {} after",
            fibers_before, fibers_after
        ));
    }
    Ok(())
}

fn dict_views() -> Result<(), String> {
    // `keys()`/`values()`/`items()` are view objects, not lists: they have
    // their own type name and repr, support len/iter/membership, and read the
    // dict live (a view sees a mutation made after it was obtained).
    expect_repr(
        "d = {'a': 1, 'b': 2}\nk = d.keys()\nv = d.values()\ni = d.items()\nx = (len(k), len(v), len(i), list(k), list(v), list(i), 'a' in k, 1 in v, ('a', 1) in i)",
        "x",
        "(2, 2, 2, ['a', 'b'], [1, 2], [('a', 1), ('b', 2)], True, True, True)",
    )?;
    expect_repr(
        "d = {'a': 1}\nx = (d.keys(), d.values(), d.items())",
        "x",
        "(dict_keys(['a']), dict_values([1]), dict_items([('a', 1)]))",
    )?;
    expect_repr(
        "d = {'a': 1, 'b': 2}\nx = (type(d.keys()).__name__, type(d.values()).__name__, type(d.items()).__name__)",
        "x",
        "('dict_keys', 'dict_values', 'dict_items')",
    )?;
    // Views are live: mutation after the view was created is visible.
    expect_repr(
        "d = {'a': 1}\nk = d.keys()\nd['b'] = 2\nx = (list(k), len(k))",
        "x",
        "(['a', 'b'], 2)",
    )?;
    expect_repr(
        "d = {'a': 1}\nk = d.keys()\ndel d['a']\nx = list(k)",
        "x",
        "[]",
    )?;
    // Views compare as sets for keys/items; `dict_values` has no meaningful
    // equality, so two values views of the same dict are unequal.
    expect_repr(
        "d = {'a': 1, 'b': 2}\nx = (d.keys() == d.keys(), d.values() == d.values())",
        "x",
        "(True, False)",
    )?;
    // `dict(d.items())` round-trips, and values views iterate like any iterable.
    expect_repr(
        "d = {'a': 1, 'b': 2}\nrebuilt = dict(d.items())\ntotal = 0\nfor value in d.values():\n    total += value\nx = (rebuilt, total)",
        "x",
        "({'a': 1, 'b': 2}, 3)",
    )?;
    // A view is unhashable and is not JSON-serializable; both fail loudly
    // instead of silently snapshotting.
    expect_repr(
        "import json\nd = {'a': 1}\ntry:\n    x = {d.keys(): 1}\nexcept TypeError as error:\n    x = str(error)",
        "x",
        "'unhashable type: \\'dict_keys\\''",
    )?;
    Ok(())
}

fn bitwise_operators() -> Result<(), String> {
    expect_repr(
        "x = (6 & 3, 6 | 3, 6 ^ 3, ~6, 1 << 4, 32 >> 2)",
        "x",
        "(2, 7, 5, -7, 16, 8)",
    )?;
    // Python precedence: | < ^ < & < shifts < additive.
    expect_repr(
        "x = (1 | 2 & 3, 1 ^ 2 | 4, 1 << 2 + 1, ~-1)",
        "x",
        "(3, 7, 8, 0)",
    )?;
    // Augmented forms rebind like the rest of the arithmetic operators.
    expect_repr("a = 5\na |= 2\na <<= 1\nx = a", "x", "14")?;
    // bool/bool keeps bool (CPython 3.13+); mixed bool/int is an int.
    expect_repr(
        "x = (True | False, type(True | False).__name__, True | 1, type(True | 1).__name__, ~True)",
        "x",
        "(True, 'bool', 1, 'int', -2)",
    )?;
    // Right shift is arithmetic and saturates; left shift never wraps.
    expect_repr(
        "x = (-8 >> 1, 1 >> 100, -1 >> 100, 0 << 70, 1 << 10)",
        "x",
        "(-4, 0, -1, 0, 1024)",
    )?;
    expect_repr(
        "try:\n    x = 1 << 64\nexcept OverflowError as error:\n    x = str(error)",
        "x",
        "'integer left shift overflow'",
    )?;
    expect_repr(
        "try:\n    x = 1 << -1\nexcept ValueError as error:\n    x = str(error)",
        "x",
        "'negative shift count'",
    )?;
    expect_repr(
        "try:\n    x = 'a' & 1\nexcept TypeError as error:\n    x = str(error)",
        "x",
        "'unsupported operand type(s) for &: \\'str\\' and \\'int\\''",
    )?;
    expect_repr(
        "try:\n    x = ~'a'\nexcept TypeError as error:\n    x = str(error)",
        "x",
        "'bad operand type for unary ~: \\'str\\''",
    )?;
    Ok(())
}

fn set_operations() -> Result<(), String> {
    // The operators Python spells with the bitwise tokens on sets.
    expect_repr(
        "a = {1, 2, 3}\nb = {3, 4}\nx = (a | b, a & b, a - b, a ^ b)",
        "x",
        "({1, 2, 3, 4}, {3}, {1, 2}, {1, 2, 4})",
    )?;
    // `|=`, `&=`, `-=`, `^=` mutate the set in place, so aliases see it.
    expect_repr(
        "a = {1, 2}\nalias = a\na |= {3}\nx = (a, alias)",
        "x",
        "({1, 2, 3}, {1, 2, 3})",
    )?;
    expect_repr(
        "a = {1, 2, 3}\nalias = a\na &= {2, 3, 4}\nx = (a, alias)",
        "x",
        "({2, 3}, {2, 3})",
    )?;
    expect_repr(
        "a = {1, 2, 3}\nalias = a\na -= {2}\nx = (a, alias)",
        "x",
        "({1, 3}, {1, 3})",
    )?;
    expect_repr(
        "a = {1, 2, 3}\nalias = a\na ^= {3, 4}\nx = (a, alias)",
        "x",
        "({1, 2, 4}, {1, 2, 4})",
    )?;
    // The mutating methods return None and accept any iterables.
    expect_repr("a = {1, 2}\nx = (a.update([3]), a)", "x", "(None, {1, 2, 3})")?;
    expect_repr(
        "a = {1, 2}\na.update({3}, [4])\nx = a",
        "x",
        "{1, 2, 3, 4}",
    )?;
    expect_repr(
        "a = {1, 2, 3}\nx = (a.difference_update([1, 2]), a)",
        "x",
        "(None, {3})",
    )?;
    expect_repr(
        "a = {1, 2, 3}\nx = (a.intersection_update([2, 3, 4]), a)",
        "x",
        "(None, {2, 3})",
    )?;
    expect_repr(
        "a = {1, 2, 3}\nx = (a.symmetric_difference_update([3, 4]), a)",
        "x",
        "(None, {1, 2, 4})",
    )?;
    expect_repr(
        "x = ({1, 2, 3}.symmetric_difference([3, 4]), {1, 2}.isdisjoint([3, 4]), {1, 2}.isdisjoint([2, 3]))",
        "x",
        "({1, 2, 4}, True, False)",
    )?;
    // `pop()` removes from the front (this implementation keeps insertion
    // order), an empty set is a KeyError, and a non-set operand is a TypeError.
    expect_repr("a = {1, 2, 3}\nx = (a.pop(), a)", "x", "(1, {2, 3})")?;
    expect_repr(
        "try:\n    set().pop()\nexcept KeyError as error:\n    x = str(error)",
        "x",
        "'pop from an empty set'",
    )?;
    expect_repr(
        "try:\n    x = {1} | [2]\nexcept TypeError as error:\n    x = str(error)",
        "x",
        "'unsupported operand type(s) for |: \\'set\\' and \\'list\\''",
    )?;
    Ok(())
}

fn keyword_only_parameters() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // `*` / `*args` switch the rest of the signature to keyword-only: those
    // parameters can never be filled positionally, and defaults may be
    // interleaved freely on the keyword-only side.
    expect_repr(
        "def f(a, *, b, c=3):\n    return (a, b, c)\nx = (f(1, b=2), f(1, b=2, c=9))",
        "x",
        "((1, 2, 3), (1, 2, 9))",
    )?;
    expect_repr(
        "def f(a, *, b):\n    return (a, b)\ntry:\n    f(1, 2)\nexcept TypeError as error:\n    x = str(error)",
        "x",
        "'f() takes 1 positional arguments but 2 were given'",
    )?;
    // A missing keyword-only argument is its own error, not a positional one.
    expect_repr(
        "def f(a, *, b, c):\n    return (a, b, c)\ntry:\n    f(1)\nexcept TypeError as error:\n    x = str(error)",
        "x",
        "'f() missing 2 required keyword-only arguments: \\'b\\' and \\'c\\''",
    )?;
    // `*args`, keyword-only parameters and `**kwargs` in one signature;
    // `**kwargs` collects only the names nothing else claimed.
    expect_repr(
        "def h(*args, x, y=1, **kw):\n    return (args, x, y, kw)\nx = (h(1, 2, x=5, z=9), h(x=0), h(1, x=3, y=4))",
        "x",
        "(((1, 2), 5, 1, {'z': 9}), ((), 0, 1, {}), ((1,), 3, 4, {}))",
    )?;
    // Defaults may be interleaved on the keyword-only side, and a positional
    // default before the star does not force keyword-only defaults.
    expect_repr(
        "def mixed(a=1, *, b, c=2):\n    return (a, b, c)\ndef kworder(*, a=1, b):\n    return (a, b)\nx = (mixed(b=5), mixed(9, b=5), kworder(b=2))",
        "x",
        "((1, 5, 2), (9, 5, 2), (1, 2))",
    )?;
    // Methods, lambdas and generator functions all get the same binding.
    expect_repr(
        "class Box:\n    def __init__(self, *, size=1):\n        self.size = size\n    def scale(self, factor, *, twice=False):\n        if twice:\n            return self.size * factor * 2\n        return self.size * factor\nbox = Box(size=3)\nx = (box.scale(2), box.scale(2, twice=True), (lambda *, q: q)(q=7))",
        "x",
        "(6, 12, 7)",
    )?;
    expect_repr(
        "def gen(*, n):\n    i = 0\n    while i < n:\n        yield i\n        i += 1\nx = list(gen(n=3))",
        "x",
        "[0, 1, 2]",
    )?;
    // A mapping unpacked at the call site fills keyword-only parameters too.
    expect_repr(
        "def total(*, a, b):\n    return a + b\nvalues = {'a': 1, 'b': 2}\nx = (total(**values), total(*[], **values))",
        "x",
        "(3, 3)",
    )?;
    Ok(())
}

fn keyword_only_errors() -> Result<(), String> {
    // The parser rejects a bare `*` that names no keyword-only parameter, a
    // parameter after `**kwargs`, a second `*`/`**`, and duplicate names.
    expect_syntax_error("def f(*): pass", "named arguments must follow bare *")?;
    expect_syntax_error("def f(*, **kw): pass", "named arguments must follow bare *")?;
    expect_syntax_error(
        "def f(**kw, x): pass",
        "parameter follows the '**kwargs' parameter",
    )?;
    expect_syntax_error(
        "def f(*a, *b): pass",
        "'*args' must appear at most once and before '**kwargs'",
    )?;
    expect_syntax_error(
        "def f(a, a): pass",
        "duplicate argument 'a' in function definition",
    )?;
    expect_syntax_error(
        "def f(*, x, x): pass",
        "duplicate argument 'x' in function definition",
    )?;
    expect_syntax_error(
        "(lambda a, a: a)",
        "duplicate argument 'a' in function definition",
    )?;
    Ok(())
}

fn decorators() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    expect_repr(
        "def double(f):\n    def wrapper(*args, **kwargs):\n        return f(*args, **kwargs) * 2\n    return wrapper\n@double\ndef five():\n    return 5\nx = five()",
        "x",
        "10",
    )?;
    // Bottom-up order: the decorator written last is applied first.
    expect_repr(
        "log = []\ndef mark(label):\n    def decorate(f):\n        def wrapper(*args, **kwargs):\n            log.append(label)\n            return f(*args, **kwargs)\n        return wrapper\n    return decorate\n@mark('top')\n@mark('bottom')\ndef target():\n    return 'value'\nx = (target(), log)",
        "x",
        "('value', ['top', 'bottom'])",
    )?;
    // A decorator may replace the function with a non-function callable or a
    // value: the name binds whatever the decorator returns.
    expect_repr(
        "def constant(f):\n    return 42\n@constant\ndef ignored():\n    return 1\nx = ignored",
        "x",
        "42",
    )?;
    // A decorator that returns the function unchanged still works, and the
    // decorated function keeps its parameters.
    expect_repr(
        "def passthrough(f):\n    return f\n@passthrough\ndef add(a, b=2):\n    return a + b\nx = (add(1), add(1, 5))",
        "x",
        "(3, 6)",
    )?;
    // Decorating a method: the wrapper receives self like any function.
    expect_repr(
        "def double(f):\n    def wrapper(*args, **kwargs):\n        return f(*args, **kwargs) * 2\n    return wrapper\nclass Point:\n    def __init__(self, x):\n        self.x = x\n    @double\n    def scaled(self):\n        return self.x\nx = Point(3).scaled()",
        "x",
        "6",
    )?;
    // A decorated generator function is still a generator function.
    expect_repr(
        "def passthrough(f):\n    return f\n@passthrough\ndef counter(n):\n    for i in range(n):\n        yield i\nx = list(counter(3))",
        "x",
        "[0, 1, 2]",
    )?;
    // @staticmethod: no implicit self, callable from the class and instance.
    expect_repr(
        "class Tools:\n    @staticmethod\n    def add(a, b):\n        return a + b\n    def twice(self, value):\n        return value * 2\nx = (Tools.add(1, 2), Tools().add(3, 4), Tools().twice(5))",
        "x",
        "(3, 7, 10)",
    )?;
    // Decorating a class binds the decorator's result.
    expect_repr(
        "def rename(cls):\n    cls.label = 'decorated'\n    return cls\n@rename\nclass Thing:\n    pass\nx = Thing.label",
        "x",
        "'decorated'",
    )?;
    // Only definitions may be decorated.
    let error = run_source("@double\nx = 1", None).err().unwrap_or_default();
    if !error.contains("decorated") {
        return Err(format!("expected a decoration error, got {:?}", error));
    }
    Ok(())
}

fn percent_formatting() -> Result<(), String> {
    expect_repr(r#"x = "%s and %d" % ("text", 42)"#, "x", "'text and 42'")?;
    expect_repr(r#"x = "%5d|%-5d|%05d" % (42, 42, 42)"#, "x", "'   42|42   |00042'")?;
    expect_repr(r#"x = "%.2f|%8.3f" % (3.14159, 3.14159)"#, "x", "'3.14|   3.142'")?;
    expect_repr(r#"x = "%x %X %o %b" % (255, 255, 8, 5)"#, "x", "'ff FF 10 101'")?;
    expect_repr(r#"x = "%s %r" % (3.5, "quo")"#, "x", r#"'3.5 \'quo\''"#)?;
    expect_repr(r#"x = "%d%%" % 50"#, "x", "'50%'")?;
    expect_repr(r#"x = "%c%c" % (65, "b")"#, "x", "'Ab'")?;
    expect_repr(r#"x = "%s" % ((1, 2),)"#, "x", "'(1, 2)'")?;
    expect_repr(r#"x = "%s" % [1, 2]"#, "x", "'[1, 2]'")?;
    expect_repr(r#"x = "%+d % d" % (5, 5)"#, "x", "'+5  5'")?;
    expect_repr(r#"x = "%e" % 12345.678"#, "x", "'1.234568e+04'")?;
    expect_repr(
        r#"try:
    "%d" % "text"
except TypeError:
    x = "type"
"#,
        "x",
        "'type'",
    )?;
    expect_repr(
        r#"try:
    "%d %d" % (1,)
except TypeError:
    x = "missing"
"#,
        "x",
        "'missing'",
    )?;
    expect_repr(
        r#"try:
    "%d" % (1, 2)
except TypeError:
    x = "extra"
"#,
        "x",
        "'extra'",
    )?;
    expect_repr(
        r#"try:
    "%q" % 1
except ValueError:
    x = "bad char"
"#,
        "x",
        "'bad char'",
    )?;
    // `%` between integers is still modulo, and `%` on floats still works.
    expect_repr("x = (7 % 3, 7.5 % 2)", "x", "(1, 1.5)")?;
    Ok(())
}

fn path_module() -> Result<(), String> {
    expect_repr("from os.path import join\nx = join('a', 'b', 'c')", "x", "'a/b/c'")?;
    expect_repr("import os.path\nx = os.path.join('/root', 'x')", "x", "'/root/x'")?;
    expect_repr("from os.path import join\nx = join('a', '/abs', 'b')", "x", "'/abs/b'")?;
    expect_repr("from os.path import join\nx = join('dir/', 'file')", "x", "'dir/file'")?;
    expect_repr(
        "from os.path import basename, dirname\nx = (basename('/mnt/test.py'), dirname('/mnt/test.py'))",
        "x",
        "('test.py', '/mnt')",
    )?;
    expect_repr(
        "from os.path import basename, dirname\nx = (basename('x'), dirname('x'))",
        "x",
        "('x', '')",
    )?;
    expect_repr(
        "from os.path import basename, dirname\nx = (basename('/'), dirname('/'), dirname('a/b/'))",
        "x",
        "('', '/', 'a')",
    )?;
    expect_repr("import os.path\nx = os.path.sep", "x", "'/'")?;
    // The existence probes run against the real filesystem.
    let path = "/tmp/pagh-python-pathcheck";
    let _ = std::fs::remove_dir_all(path);
    std::fs::create_dir(path).map_err(|error| format!("mkdir: {}", error))?;
    std::fs::write(format!("{}/file.txt", path), b"x").map_err(|e| format!("write: {}", e))?;
    let source = format!(
        "from os.path import exists, isfile, isdir\nx = (exists('{0}'), isfile('{0}/file.txt'), isdir('{0}'), exists('{0}/nope'), isfile('{0}'))",
        path
    );
    let result = expect_repr(&source, "x", "(True, True, True, False, False)");
    let _ = std::fs::remove_dir_all(path);
    result?;
    Ok(())
}

fn time_module() -> Result<(), String> {
    // `time()` is a positive wall clock; `monotonic()` never goes backwards
    // and `sleep` really waits on the monotonic clock.
    expect_repr("import time\nx = time.time() > 0", "x", "True")?;
    expect_repr(
        "import time\nstart = time.monotonic()\ntime.sleep(0.01)\nx = time.monotonic() >= start",
        "x",
        "True",
    )?;
    expect_repr(
        "import time\nstart = time.monotonic()\ntime.sleep(0.02)\nx = time.monotonic() - start >= 0.015",
        "x",
        "True",
    )?;
    expect_repr("import time\nx = time.monotonic() >= 0.0", "x", "True")?;
    expect_repr(
        "import time\ntry:\n    time.sleep(-1)\n    x = 'no error'\nexcept ValueError:\n    x = 'negative'",
        "x",
        "'negative'",
    )?;
    Ok(())
}

fn random_module() -> Result<(), String> {
    // Deterministic: the same seed gives the same sequence, and the default
    // seed is fixed, so two runs of the same script agree.
    expect_repr(
        "import random\nrandom.seed(7)\na = [random.random() for _ in range(3)]\nrandom.seed(7)\nb = [random.random() for _ in range(3)]\nx = a == b",
        "x",
        "True",
    )?;
    expect_repr(
        "import random\nrandom.seed(7)\na = random.random()\nrandom.seed(7)\nb = random.random()\nx = a == b",
        "x",
        "True",
    )?;
    expect_repr(
        "import random\nx = 0.0 <= random.random() < 1.0",
        "x",
        "True",
    )?;
    expect_repr(
        "import random\nrandom.seed(1)\nvalues = [random.randint(1, 6) for _ in range(200)]\nx = (min(values) >= 1, max(values) <= 6, len(values))",
        "x",
        "(True, True, 200)",
    )?;
    expect_repr(
        "import random\nrandom.seed(3)\nvalues = [random.randint(5, 5) for _ in range(10)]\nx = values",
        "x",
        "[5, 5, 5, 5, 5, 5, 5, 5, 5, 5]",
    )?;
    expect_repr(
        "import random\nrandom.seed(2)\nx = random.choice('abc') in 'abc'",
        "x",
        "True",
    )?;
    expect_repr(
        "import random\nrandom.seed(2)\nitems = [1, 2, 3, 4, 5, 6]\nrandom.shuffle(items)\nx = (sorted(items), len(items))",
        "x",
        "([1, 2, 3, 4, 5, 6], 6)",
    )?;
    expect_repr(
        "import random\nrandom.seed(11)\na = [1, 2, 3, 4, 5]\nrandom.shuffle(a)\nrandom.seed(11)\nb = [1, 2, 3, 4, 5]\nrandom.shuffle(b)\nx = a == b",
        "x",
        "True",
    )?;
    expect_repr(
        "import random\nrandom.seed(5)\nvalue = random.uniform(1.0, 2.0)\nx = 1.0 <= value <= 2.0",
        "x",
        "True",
    )?;
    expect_repr(
        "import random\ntry:\n    random.choice([])\n    x = 'no error'\nexcept IndexError:\n    x = 'empty'",
        "x",
        "'empty'",
    )?;
    expect_repr(
        "import random\ntry:\n    random.randint(5, 1)\n    x = 'no error'\nexcept ValueError:\n    x = 'reversed'",
        "x",
        "'reversed'",
    )?;
    Ok(())
}

fn sets() -> Result<(), String> {
    expect_repr("x = {1, 2, 2, 3}", "x", "{1, 2, 3}")?;
    expect_repr("x = set([1, 2, 2])", "x", "{1, 2}")?;
    expect_repr("x = len({1, 1, 2})", "x", "2")?;
    expect_repr("x = 2 in {1, 2, 3}", "x", "True")?;
    expect_repr("x = {1, 2} == {2, 1}", "x", "True")?;
    expect_repr("x = sorted({3, 1, 2})", "x", "[1, 2, 3]")?;
    expect_repr(
        "a = {1, 2}\na.add(3)\na.discard(9)\na.remove(1)\nx = sorted(a)",
        "x",
        "[2, 3]",
    )?;
    expect_repr(
        "x = sorted({1, 2}.union({2, 3})), sorted({1, 2, 3}.intersection({2, 3, 4})), sorted({1, 2, 3}.difference({2}))",
        "x",
        "([1, 2, 3], [2, 3], [1, 3])",
    )?;
    expect_repr(
        "x = ({1, 2}.issubset({1, 2, 3}), {1, 2, 3}.issuperset({1, 2}), {1, 2}.issubset([1, 2, 3]))",
        "x",
        "(True, True, True)",
    )?;
    expect_repr("x = sorted({value * value for value in range(5) if value % 2 == 0})", "x", "[0, 4, 16]")?;
    expect_repr("x = {key: value for key, value in [(1, 'a'), (2, 'b')]}", "x", "{1: 'a', 2: 'b'}")?;
    expect_repr(
        "try:\n    {1, 2}.remove(9)\nexcept KeyError:\n    x = 'missing'",
        "x",
        "'missing'",
    )?;
    Ok(())
}

fn bytes_values() -> Result<(), String> {
    expect_repr(r#"x = b"abc""#, "x", "b'abc'")?;
    expect_repr(r#"x = b"a\nb\x41\t""#, "x", "b'a\\nbA\\t'")?;
    expect_repr(r#"x = len(b"abc")"#, "x", "3")?;
    expect_repr(r#"x = b"abc"[1]"#, "x", "98")?;
    expect_repr(r#"x = b"abcdef"[1:4]"#, "x", "b'bcd'")?;
    expect_repr(r#"x = b"abc" + b"de""#, "x", "b'abcde'")?;
    expect_repr(r#"x = b"ab" * 2"#, "x", "b'abab'")?;
    expect_repr(r#"x = b"abc" == b"abc", b"abc" == b"abd""#, "x", "(True, False)")?;
    expect_repr(r#"x = b"abc" < b"abd""#, "x", "True")?;
    expect_repr(r#"x = b"abc".decode(), "hello".encode()"#, "x", "('abc', b'hello')")?;
    expect_repr(r#"x = bytes([65, 66, 67])"#, "x", "b'ABC'")?;
    expect_repr(r#"x = bytes(3)"#, "x", "b'\\x00\\x00\\x00'")?;
    expect_repr(r#"x = bytes("hi", "utf-8")"#, "x", "b'hi'")?;
    expect_repr(r#"x = list(b"ab")"#, "x", "[97, 98]")?;
    expect_repr(r#"x = b"abc" in b"xxabcxx", 97 in b"abc""#, "x", "(True, True)")?;
    Ok(())
}

fn fstrings() -> Result<(), String> {
    expect_repr(r#"name = "pagh"
x = f"hello {name}""#, "x", "'hello pagh'")?;
    expect_repr("value = 3.14159\nx = f\"{value:.2f}\"", "x", "'3.14'")?;
    expect_repr("x = f\"{42:d}\"", "x", "'42'")?;
    expect_repr("name = 'pagh'\nx = f\"{name!r}\" == repr(name)", "x", "True")?;
    expect_repr("x = f\"{{literal}}\"", "x", "'{literal}'")?;
    expect_repr("x = f\"{7:>5d}|{7:<5d}|{7:^5d}\"", "x", "'    7|7    |  7  '")?;
    expect_repr("x = f\"{255:x}\"", "x", "'ff'")?;
    expect_repr("x = f\"{1.0}\"", "x", "'1.0'")?;
    expect_repr("x = f\"a\" f\"b\"", "x", "'ab'")?;
    Ok(())
}

fn json_module() -> Result<(), String> {
    expect_repr(
        "import json\nv = {\"a\": [1, 2.5, True, None], \"s\": 'q\"x\\\\n', \"u\": \"привет\"}\nt = json.dumps(v)\nx = (json.loads(t) == v, t.startswith('{\"a\": [1, 2.5, true, null]'), '\\\\u043f' in t)",
        "x",
        "(True, True, True)",
    )?;
    expect_repr("import json\nx = json.dumps([1, -2, 3.0, False, None])", "x", "'[1, -2, 3.0, false, null]'")?;
    expect_repr(
        "import json\nx = json.loads('{\"x\": 1, \"y\": [true, false, null, -0.5e2]}')",
        "x",
        "{'x': 1, 'y': [True, False, None, -50.0]}",
    )?;
    expect_repr("import json\nx = json.loads('\"hi\\\\nthere\"')", "x", "'hi\\nthere'")?;
    expect_repr("import json\nx = json.loads('null')", "x", "None")?;
    expect_repr(
        "import json\ntry:\n    json.loads('{bad}')\nexcept ValueError as error:\n    x = type(error).__name__",
        "x",
        "'JSONDecodeError'",
    )?;
    Ok(())
}

fn os_module() -> Result<(), String> {
    let mut path = std::env::temp_dir();
    path.push(format!("pagh-python-os-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("cannot create {}: {}", path.display(), error))?;
    let script = format!(
        "import os\n\
         os.mkdir(\"{0}/one\")\n\
         os.makedirs(\"{0}/two/three\")\n\
         x1 = sorted(os.listdir(\"{0}\"))\n\
         p = \"{0}/f.txt\"\n\
         f = open(p, \"w\")\n\
         f.write(\"xy\")\n\
         f.close()\n\
         st = os.stat(p)\n\
         x2 = (st.st_size, st.st_kind, st.st_isfile, st.st_isdir)\n\
         os.rename(p, \"{0}/g.txt\")\n\
         x3 = os.stat(\"{0}/g.txt\").st_size\n\
         os.remove(\"{0}/g.txt\")\n\
         os.rmdir(\"{0}/one\")\n\
         x4 = sorted(os.listdir(\"{0}\"))\n\
         x5 = (\"PWD\" in os.environ, os.getpid() > 0, os.getppid() >= 0, len(os.getcwd()) > 0)\n\
         x6 = sorted(os.listdir(\"{0}/two\"))\n\
         d = os.stat(\"{0}/two\")\n\
         x7 = (d.st_kind, d.st_isdir, d.st_isfile)",
        path.display()
    );
    let interpreter = match run_source(&script, None) {
        Ok(interpreter) => interpreter,
        Err(message) => {
            let _ = std::fs::remove_dir_all(&path);
            return Err(message);
        }
    };
    let expected = [
        ("x1", "['one', 'two']"),
        ("x2", "(2, 'file', True, False)"),
        ("x3", "2"),
        ("x4", "['two']"),
        ("x5", "(True, True, True, True)"),
        ("x6", "['three']"),
        ("x7", "('dir', True, False)"),
    ];
    for (name, value) in expected {
        let result = match env_lookup(&interpreter.module_env, name) {
            Some(actual) => {
                let actual = actual.to_repr();
                if actual == value {
                    Ok(())
                } else {
                    Err(format!("{}: expected {}, got {}", name, value, actual))
                }
            }
            None => Err(format!("{} was not defined", name)),
        };
        if let Err(message) = result {
            let _ = std::fs::remove_dir_all(&path);
            return Err(message);
        }
    }
    let _ = std::fs::remove_dir_all(&path);
    Ok(())
}

fn binary_files() -> Result<(), String> {
    let mut path = std::env::temp_dir();
    path.push(format!("pagh-python-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("cannot create {}: {}", path.display(), error))?;
    let file = path.join("data.bin");
    let script = format!(
        "p = \"{}\"\n\
         with open(p, \"wb\") as out:\n\
         \x20   written = out.write(b\"\\x00\\x01hi\\xff\")\n\
         with open(p, \"rb\") as source:\n\
         \x20   data = source.read()\n\
         x = (written, data, len(data), data[2:4])",
        file.display()
    );
    let interpreter = match run_source(&script, None) {
        Ok(interpreter) => interpreter,
        Err(message) => {
            let _ = std::fs::remove_dir_all(&path);
            return Err(message);
        }
    };
    let actual = match env_lookup(&interpreter.module_env, "x") {
        Some(value) => value.to_repr(),
        None => String::from("x was not defined"),
    };
    let expected = "(5, b'\\x00\\x01hi\\xff', 5, b'hi')";
    if actual != expected {
        let _ = std::fs::remove_dir_all(&path);
        return Err(format!("x: expected {}, got {}", expected, actual));
    }
    let contents = std::fs::read(&file).map_err(|error| format!("cannot read file: {}", error))?;
    let _ = std::fs::remove_dir_all(&path);
    if contents != b"\x00\x01hi\xff".to_vec() {
        return Err(format!("unexpected bytes on disk: {:?}", contents));
    }
    Ok(())
}

// ── parser nesting limits (audit F-rt-4) ──────────────────────────────────
//
// The interpreter's recursive-descent parser used to accept any nesting and a
// 506-byte script (`x = ` + 200 `(`) exhausted the process stack on pagh-riscv
// and killed `pythonc`. The front end now has two deterministic limits, and
// these checks pin both at their exact boundary: 199 and 200 nesting levels
// parse, the 201st is a `SyntaxError` -- never a crash. Brackets are rejected
// iteratively by the lexer with CPython's "too many nested parentheses"
// message; constructs that are not bracket-delimited are counted by the parser
// and rejected with "maximum recursion depth exceeded".

/// Parse `source` and require success. The checks are parse-only: a 200-deep
/// AST is not worth evaluating just to prove that the front end accepted it.
fn expect_parses(source: &str) -> Result<(), String> {
    match parser::parse(source) {
        Ok(_) => Ok(()),
        Err(error) => Err(format!(
            "expected the source to parse, got {:?} at {}:{}",
            error.message, error.line, error.column
        )),
    }
}

fn nested_parentheses(depth: usize) -> String {
    format!("x = {}1{}\n", "(".repeat(depth), ")".repeat(depth))
}

fn nested_lists(depth: usize) -> String {
    format!("x = {}1{}\n", "[".repeat(depth), "]".repeat(depth))
}

fn nested_calls(depth: usize) -> String {
    format!("x = {}1{}\n", "f(".repeat(depth), ")".repeat(depth))
}

fn nested_comprehensions(depth: usize) -> String {
    // One `[x for x in …]` layer per level; the innermost `[]` is one extra
    // bracket, so `depth` here means `depth + 1` open brackets in total.
    format!(
        "x = {}[]{}\n",
        "[x for x in ".repeat(depth),
        "]".repeat(depth)
    )
}

fn nested_nots(depth: usize) -> String {
    format!("x = {}True\n", "not ".repeat(depth))
}

fn nested_unary_minus(depth: usize) -> String {
    format!("x = {}1\n", "-".repeat(depth))
}

fn nested_powers(depth: usize) -> String {
    format!("x = {}1\n", "2**".repeat(depth))
}

fn chained_conditionals(depth: usize) -> String {
    format!("x = {}3\n", "1 if True else ".repeat(depth))
}

fn nested_lambdas(depth: usize) -> String {
    format!("x = {}1\n", "lambda: ".repeat(depth))
}

fn nested_blocks(depth: usize) -> String {
    let mut source = String::new();
    for level in 0..depth {
        for _ in 0..level {
            source.push(' ');
        }
        source.push_str("if True:\n");
    }
    for _ in 0..depth {
        source.push(' ');
    }
    source.push_str("x = 1\n");
    source
}

fn nested_yields(depth: usize) -> String {
    format!("def f():\n    x = {}1\n", "yield ".repeat(depth))
}

fn nested_targets(depth: usize) -> String {
    format!(
        "for {}a{} in []:\n    pass\n",
        "(".repeat(depth),
        ")".repeat(depth)
    )
}

/// Bracket nesting is bounded by the lexer before the parser recurses, exactly
/// at CPython's tokenizer boundary (`MAXLEVEL = 200`): 199 and 200 are valid,
/// 201 is `SyntaxError: too many nested parentheses`.
fn parser_bracket_nesting() -> Result<(), String> {
    for depth in [199, 200] {
        expect_parses(&nested_parentheses(depth))?;
        expect_parses(&nested_lists(depth))?;
        expect_parses(&nested_calls(depth))?;
        expect_parses(&nested_targets(depth))?;
    }
    // 199 comprehension layers plus the innermost `[]` are exactly 200 open
    // brackets; 200 layers plus `[]` are 201 and must be refused.
    expect_parses(&nested_comprehensions(199))?;
    for source in [
        nested_parentheses(201),
        nested_lists(201),
        nested_calls(201),
        nested_targets(201),
        nested_comprehensions(200),
    ] {
        expect_syntax_error(&source, "too many nested parentheses")?;
    }
    Ok(())
}

/// Constructs that recurse without bracket tokens (unary chains, `**`,
/// conditional expressions, lambdas, statement blocks, targets, `yield`
/// operands) are bounded by the parser's explicit depth counter.
fn parser_recursion_depth() -> Result<(), String> {
    for depth in [199, 200] {
        expect_parses(&nested_nots(depth))?;
        expect_parses(&nested_unary_minus(depth))?;
        expect_parses(&nested_powers(depth))?;
        expect_parses(&chained_conditionals(depth))?;
        expect_parses(&nested_lambdas(depth))?;
        expect_parses(&nested_blocks(depth))?;
    }
    for source in [
        nested_nots(201),
        nested_unary_minus(201),
        nested_powers(201),
        chained_conditionals(201),
        nested_lambdas(201),
        nested_blocks(201),
    ] {
        expect_syntax_error(&source, "maximum recursion depth exceeded")?;
    }
    // The `def` body is itself one nesting level (`parse_block`), so 199
    // nested `yield` operands reach the 200-level budget and the 200th does not.
    expect_parses(&nested_yields(199))?;
    expect_syntax_error(&nested_yields(200), "maximum recursion depth exceeded")?;
    expect_syntax_error(&nested_yields(201), "maximum recursion depth exceeded")?;

    // The existing comprehension/yield guard is independent of the depth
    // counter and must keep its specific message.
    expect_syntax_error(
        "def f():\n    return [yield 1 for x in []]\n",
        "'yield' inside a comprehension is not supported",
    )?;
    Ok(())
}

fn checks() -> Vec<(&'static str, fn() -> Result<(), String>)> {
    alloc::vec![
        ("arithmetic", arithmetic as fn() -> Result<(), String>),
        ("generators", generators),
        ("yield from", yield_from),
        ("generator send", generator_send),
        ("generator throw", generator_throw),
        ("generator close", generator_close),
        ("generator delegation", generator_delegation),
        ("generator expressions", generator_expressions),
        ("decorators", decorators),
        ("percent format", percent_formatting),
        ("os.path", path_module),
        ("time", time_module),
        ("random", random_module),
        ("fibers", fibers),
        ("floats", floats),
        ("strings", strings),
        ("lists", lists),
        ("dicts", dicts),
        ("dict views", dict_views),
        ("bitwise operators", bitwise_operators),
        ("keyword-only parameters", keyword_only_parameters),
        ("keyword-only errors", keyword_only_errors),
        ("set operations", set_operations),
        ("slicing", slicing),
        ("functions", functions),
        ("classes", classes),
        ("exceptions", exceptions),
        ("uncaught", uncaught),
        ("files", files),
        ("imports", imports),
        ("lambdas", lambdas),
        ("variadics", variadics),
        ("format", formatting),
        ("with", with_statement),
        ("sets", sets),
        ("bytes", bytes_values),
        ("fstrings", fstrings),
        ("json", json_module),
        ("os", os_module),
        ("binary files", binary_files),
        ("parser bracket nesting", parser_bracket_nesting),
        ("parser recursion depth", parser_recursion_depth),
        ("evaluation depth", eval_recursion_depth),
        ("evaluation depth generator", eval_recursion_generator),
        ("recursive drop on fiber", recursive_drop_fiber),
    ]
}

pub fn selftest() {
    let mut failed = 0;
    for (name, check) in checks() {
        match check() {
            Ok(()) => println!("PASS {}", name),
            Err(message) => {
                println!("FAIL {}: {} (fibers {})", name, message, crate::fiber::live_fibers());
                failed += 1;
            }
        }
    }
    if failed > 0 {
        eprintln!("{} check(s) failed", failed);
        std::process::exit(1);
    }
    println!("all {} checks passed", checks().len());
}

#[test]
fn arithmetic_works() {
    arithmetic().unwrap();
}

#[test]
fn floats_work() {
    floats().unwrap();
}

#[test]
fn strings_work() {
    strings().unwrap();
}

#[test]
fn lists_work() {
    lists().unwrap();
}

#[test]
fn dicts_work() {
    dicts().unwrap();
}

#[test]
fn slicing_works() {
    slicing().unwrap();
}

#[test]
fn functions_work() {
    functions().unwrap();
}

#[test]
fn classes_work() {
    classes().unwrap();
}

#[test]
fn exceptions_work() {
    exceptions().unwrap();
}

#[test]
fn uncaught_errors_work() {
    uncaught().unwrap();
}

#[test]
fn files_work() {
    files().unwrap();
}

#[test]
fn imports_work() {
    imports().unwrap();
}

#[test]
fn lambdas_work() {
    lambdas().unwrap();
}

#[test]
fn variadics_work() {
    variadics().unwrap();
}

#[test]
fn formatting_works() {
    formatting().unwrap();
}

#[test]
fn with_statement_works() {
    with_statement().unwrap();
}

#[test]
fn sets_work() {
    sets().unwrap();
}

#[test]
fn bytes_work() {
    bytes_values().unwrap();
}

#[test]
fn fstrings_work() {
    fstrings().unwrap();
}

#[test]
fn json_works() {
    json_module().unwrap();
}

#[test]
fn os_works() {
    os_module().unwrap();
}

#[test]
fn binary_files_work() {
    binary_files().unwrap();
}

#[test]
fn fibers_work() {
    fibers().unwrap();
}

#[test]
fn generators_work() {
    generators().unwrap();
}

#[test]
fn generator_expressions_work() {
    generator_expressions().unwrap();
}

#[test]
fn yield_from_works() {
    yield_from().unwrap();
}

#[test]
fn generator_send_works() {
    generator_send().unwrap();
}

#[test]
fn generator_throw_works() {
    generator_throw().unwrap();
}

#[test]
fn generator_close_works() {
    generator_close().unwrap();
}

#[test]
fn generator_delegation_works() {
    generator_delegation().unwrap();
}

#[test]
fn dict_views_work() {
    dict_views().unwrap();
}

#[test]
fn keyword_only_parameters_work() {
    keyword_only_parameters().unwrap();
}

#[test]
fn keyword_only_errors_work() {
    keyword_only_errors().unwrap();
}

#[test]
fn bitwise_operators_work() {
    bitwise_operators().unwrap();
}

#[test]
fn set_operations_work() {
    set_operations().unwrap();
}

#[test]
fn decorators_work() {
    decorators().unwrap();
}

#[test]
fn percent_formatting_works() {
    percent_formatting().unwrap();
}

#[test]
fn path_module_works() {
    path_module().unwrap();
}

#[test]
fn time_module_works() {
    time_module().unwrap();
}

#[test]
fn random_module_works() {
    random_module().unwrap();
}

#[test]
fn parser_bracket_nesting_works() {
    parser_bracket_nesting().unwrap();
}

#[test]
fn parser_recursion_depth_works() {
    parser_recursion_depth().unwrap();
}

// ── evaluator recursion limits (audit F-r2-py-1/2) ─────────────────────────
//
// The parser builds a left-associative chain (`1+1+...1`) as a left-deep tree
// in a loop, so its nesting counter never grows, and the evaluator used to
// recurse once per node with no limit: 720 terms walked the process stack into
// the kernel's guard page, and the same expression inside a generator overran
// the 64 KiB heap fiber stack (wild-pointer faults -- there is no guard page
// under it). `eval_expr` now charges one level per node against
// EXPR_RECURSION_LIMIT and checks the running stack's headroom; these checks
// pin the depth boundary, the clean generator error, and the fact that the
// interpreter stays usable and the generator counter stays per-stack.

fn chained_binary(operator: &str, terms: usize) -> String {
    let mut source = String::from("x = 1");
    for _ in 1..terms {
        source.push_str(operator);
        source.push('1');
    }
    source.push('\n');
    source
}

/// The expression part of [`chained_binary`] for contexts such as `return`,
/// `yield` or a call argument.
fn chained_expr(operator: &str, terms: usize) -> String {
    let source = chained_binary(operator, terms);
    String::from(source.trim_start_matches("x = ").trim_end())
}

fn expect_run_error(source: &str, kind: &str, needle: &str) -> Result<(), String> {
    match run_source(source, None) {
        Ok(_) => Err(format!(
            "expected {} containing {:?}, but the source ran",
            kind, needle
        )),
        Err(message) => {
            if message.starts_with(kind) && message.contains(needle) {
                Ok(())
            } else {
                Err(format!(
                    "expected {} containing {:?}, got {:?}",
                    kind, needle, message
                ))
            }
        }
    }
}

fn eval_recursion_depth() -> Result<(), String> {
    // The limit is the evaluator's, not the parser's: 256 terms are exactly
    // 256 eval levels (255 binary nodes plus the leaf) and evaluate; the 257th
    // term is the 257th level and raises. The limit sits above the parser's
    // 200 so that 200 nested unary operators (201 eval levels) still run. The
    // same boundary applies to every operator the parser folds in a loop, and
    // to attribute/subscript chains.
    expect_repr(&chained_binary("+", 256), "x", "256")?;
    expect_repr(&chained_binary("*", 256), "x", "1")?;
    for terms in [257, 720, 800] {
        expect_run_error(
            &chained_binary("+", terms),
            "RecursionError",
            "maximum recursion depth exceeded",
        )?;
    }

    // Attribute and subscript chains are the other productions the parser
    // folds in a loop. 255 links plus the base name are 256 eval levels; the
    // 256th link would be one level too deep. The identity check is a separate
    // statement so it does not add a level to the chain under test.
    let class = "class C:\n    pass\n\nc = C()\nc.self = c\n";
    expect_repr(
        &format!(
            "{}ok = c{}\nok_is_c = ok is c\n",
            class,
            ".self".repeat(255)
        ),
        "ok_is_c",
        "True",
    )?;
    expect_run_error(
        &format!("{}bad = c{}\n", class, ".self".repeat(256)),
        "RecursionError",
        "maximum recursion depth exceeded",
    )?;
    let list = "l = []\nl.append(l)\n";
    expect_repr(
        &format!("{}ok = l{}\nok_is_l = ok is l\n", list, "[0]".repeat(255)),
        "ok_is_l",
        "True",
    )?;
    expect_run_error(
        &format!("{}bad = l{}\n", list, "[0]".repeat(256)),
        "RecursionError",
        "maximum recursion depth exceeded",
    )?;
    Ok(())
}

fn eval_recursion_generator() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    // The 64 KiB fiber cannot hold 200 Rust frames, so a generator hits its
    // headroom guard first -- cleanly, with the interpreter and heap intact.
    // The short chain proves the guard does not eat ordinary generator work.
    let short = format!(
        "def g():\n    yield {}\nv = next(g())\n",
        chained_expr("+", 20)
    );
    expect_repr(&short, "v", "20")?;

    let bomb = format!(
        "def g():\n    yield {}\n\
         caught = False\n\
         try:\n\
         \x20   next(g())\n\
         except RecursionError:\n\
         \x20   caught = True\n\
         after = 6 * 7\n\
         assert caught and after == 42\n",
        chained_expr("+", 300)
    );
    run_source(&bomb, None)?;

    // A generator expression is the same fiber path.
    let genexpr = format!(
        "caught = False\n\
         try:\n\
         \x20   next({} for _ in [0])\n\
         except RecursionError:\n\
         \x20   caught = True\n\
         after = 6 * 7\n\
         assert caught and after == 42\n",
        chained_expr("+", 300)
    );
    run_source(&genexpr, None)?;

    // A deep chain in a function that is never called parses and runs: the
    // parser's iterative productions are not evaluation (the audit's
    // `func_add_3000.py` control), while calling it raises cleanly.
    let deep_body = format!(
        "def f():\n    return {}\n",
        chained_expr("+", 3000)
    );
    run_source(&deep_body, None)?;
    let called = format!(
        "def f():\n    return {}\nprint(f())\n",
        chained_expr("+", 300)
    );
    expect_run_error(
        &called,
        "RecursionError",
        "maximum recursion depth exceeded",
    )?;

    // Catching the error leaves the interpreter fully usable.
    let caught_chain = format!(
        "caught = False\n\
         try:\n\
         \x20   x = {}\n\
         except RecursionError:\n\
         \x20   caught = True\n\
         assert caught and 1 + 1 == 2\n",
        chained_expr("+", 800)
    );
    run_source(&caught_chain, None)?;

    // The depth counter is per running stack: a generator suspended inside a
    // deep expression must not steal the caller's budget (or lose its own on
    // resume). `(yield 1)` leaves four eval wrappers parked on the fiber.
    let interleave = format!(
        "def g():\n\
         \x20   x = (1 + 1 + 1 + (yield 1))\n\
         \x20   yield x\n\
         a = g()\n\
         first = next(a)\n\
         full = {}\n\
         second = a.send(40)\n\
         assert first == 1 and full == 256 and second == 43\n",
        chained_expr("+", 256)
    );
    run_source(&interleave, None)?;
    Ok(())
}

#[test]
fn eval_recursion_depth_works() {
    eval_recursion_depth().unwrap();
}

#[test]
fn eval_recursion_generator_works() {
    eval_recursion_generator().unwrap();
}

/// F-r3-rt-1: a deep Python value graph released inside a generator must not
/// recurse through the native drop glue. The generator body runs on a 64 KiB
/// heap fiber with no guard page, so a recursive teardown would overrun the
/// stack and corrupt the heap -- the audit PoC died at ~1900 nested lists.
/// Every container kind that used to recurse is exercised here (list, tuple,
/// dict, set, instance attributes, function closures, iterator state, a dict
/// view and a function-frame environment).
fn recursive_drop_fiber() -> Result<(), String> {
    let _fiber_guard = fiber_guard();
    let script = r#"
class C:
    pass

def build_list(n):
    a = []
    for _ in range(n):
        a = [a]
    return a

def build_tuple(n):
    a = ()
    for _ in range(n):
        a = (a,)
    return a

def build_dict(n):
    a = None
    for _ in range(n):
        a = {"k": a}
    return a

def build_instance(n):
    a = C()
    for _ in range(n):
        b = C()
        b.next = a
        a = b
    return a

def make(prev):
    def f():
        return prev
    return f

def build_function(n):
    f = None
    for _ in range(n):
        f = make(f)
    return f

def release_env(n):
    a = build_list(n)
    return len(a)

def gen(n):
    a = build_list(n)
    a = None
    print("DROP LIST OK")
    a = build_tuple(n)
    a = None
    print("DROP TUPLE OK")
    a = build_dict(n)
    a = None
    print("DROP DICT OK")
    a = build_tuple(n)
    s = {a}
    s = None
    a = None
    print("DROP SET OK")
    a = build_instance(n)
    a = None
    print("DROP INSTANCE OK")
    f = build_function(n)
    f = None
    print("DROP FUNCTION OK")
    a = build_list(n)
    it = iter(a)
    a = None
    it = None
    print("DROP ITERATOR OK")
    d = build_dict(n)
    v = d.items()
    d = None
    v = None
    print("DROP VIEW OK")
    release_env(n)
    print("DROP ENV OK")
    yield 1

for _v in gen(4000):
    pass
ok = 1 + 1 == 2
"#;
    expect_repr(script, "ok", "True")
}

#[test]
fn recursive_drop_fiber_works() {
    recursive_drop_fiber().unwrap();
}
