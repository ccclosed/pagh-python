//! Host-side harness: includes the interpreter sources and runs scripts with a
//! `std` platform, or parses a file for front-end tests.
//!
//! Usage:
//!   hostcheck <script.py> [args...]   run a script (same CLI as pythonc)
//!   hostcheck -c 'code' [args...]     run code (same CLI as pythonc)
//!   hostcheck --parse <file>          parse only; print "ok" or the error
//!   hostcheck --tokens <file>         dump the token stream
//!   hostcheck --selftest              run the built-in interpreter checks

extern crate alloc;

mod platform;

#[path = "../../src/ast.rs"]
mod ast;
#[path = "../../src/builtins.rs"]
mod builtins;
#[path = "../../src/error.rs"]
mod error;
#[path = "../../src/interp.rs"]
mod interp;
#[path = "../../src/lexer.rs"]
mod lexer;
#[path = "../../src/parser.rs"]
mod parser;
#[path = "../../src/value.rs"]
mod value;

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::PyError;

fn main() {
    let args = platform::args();
    if args.len() >= 3 && args[1] == "--parse" {
        match std::fs::read_to_string(&args[2]) {
            Ok(source) => match parser::parse(&source) {
                Ok(module) => {
                    println!("ok {} statements", module.body.len());
                }
                Err(error) => {
                    println!(
                        "{}:{}:{}: {}: {}",
                        args[2], error.line, error.column, error.kind, error.message
                    );
                    platform::exit(1);
                }
            },
            Err(error) => {
                eprintln!("hostcheck: {}: {}", args[2], error);
                platform::exit(2);
            }
        }
        return;
    }
    if args.len() >= 3 && args[1] == "--tokens" {
        match std::fs::read_to_string(&args[2]) {
            Ok(source) => match lexer::tokenize(&source) {
                Ok(tokens) => {
                    for token in tokens {
                        println!("{:>3}:{:>2} {:?}", token.line, token.column, token.tok);
                    }
                }
                Err(error) => {
                    println!(
                        "{}:{}:{}: {}: {}",
                        args[2], error.line, error.column, error.kind, error.message
                    );
                    platform::exit(1);
                }
            },
            Err(error) => {
                eprintln!("hostcheck: {}: {}", args[2], error);
                platform::exit(2);
            }
        }
        return;
    }
    if args.len() >= 2 && args[1] == "--selftest" {
        tests::selftest();
        return;
    }
    platform::exit(run(&args));
}

fn usage() {
    platform::write_stderr("usage: hostcheck [-c code | script.py] [args...]\n");
}

fn run(args: &[String]) -> i32 {
    if args.len() < 2 {
        usage();
        return 2;
    }
    let command = &args[1];
    let (filename, source, script_dir, argv): (String, String, Option<String>, Vec<String>) =
        if command == "-c" {
            if args.len() < 3 {
                eprintln!("hostcheck: argument expected for the -c option");
                return 2;
            }
            (
                String::from("<string>"),
                args[2].clone(),
                None,
                args[1..].to_vec(),
            )
        } else if command.starts_with('-') {
            eprintln!("hostcheck: unknown option: {}", command);
            return 2;
        } else {
            let source = match platform::read_text(command) {
                Ok(source) => source,
                Err(message) => {
                    eprintln!("hostcheck: can't open file '{}': {}", command, message);
                    return 2;
                }
            };
            let script_dir = match command.rfind('/') {
                Some(0) => Some(String::from("/")),
                Some(index) => Some(String::from(&command[..index])),
                None => None,
            };
            (command.clone(), source, script_dir, args[1..].to_vec())
        };

    let module = match parser::parse(&source) {
        Ok(module) => module,
        Err(error) => {
            eprintln!(
                "{}:{}:{}: {}: {}",
                filename, error.line, error.column, error.kind, error.message
            );
            return 1;
        }
    };
    let mut interpreter = interp::Interp::new(argv, script_dir);
    match interpreter.run_module(&module.body, &filename) {
        Ok(()) => 0,
        Err(error) => {
            report(&error);
            1
        }
    }
}

fn report(error: &PyError) {
    use std::io::Write;
    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let _ = writeln!(lock, "Traceback (most recent call last):");
    for frame in &error.frames {
        let _ = writeln!(
            lock,
            "  File \"{}\", line {}, in {}",
            frame.filename, frame.line, frame.name
        );
    }
    if error.message.is_empty() {
        let _ = writeln!(lock, "{}", error.type_name());
    } else {
        let _ = writeln!(lock, "{}: {}", error.type_name(), error.message);
    }
}

mod tests;
