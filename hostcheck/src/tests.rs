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
                println!("FAIL {}: {}", name, message);
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
