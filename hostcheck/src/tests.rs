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

fn decorators() -> Result<(), String> {
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

fn checks() -> Vec<(&'static str, fn() -> Result<(), String>)> {
    alloc::vec![
        ("arithmetic", arithmetic as fn() -> Result<(), String>),
        ("generators", generators),
        ("yield from", yield_from),
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
