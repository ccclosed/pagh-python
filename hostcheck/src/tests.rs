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

fn checks() -> Vec<(&'static str, fn() -> Result<(), String>)> {
    alloc::vec![
        ("arithmetic", arithmetic as fn() -> Result<(), String>),
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
