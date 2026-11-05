//! `pythonc` — a native Python interpreter for the pagh-riscv native ABI.
//!
//! The entry point is deliberately tiny: parse the command line, read the
//! source through the platform layer, hand it to the lexer/parser, and run the
//! module. Diagnostics are the visible contract of the CLI (line-numbered
//! syntax errors and Python-shaped tracebacks), so they live here next to the
//! exit codes.

#![no_std]
#![no_main]

extern crate alloc;

mod ast;
mod builtins;
mod error;
mod float;
mod interp;
mod lexer;
mod parser;
mod platform;
mod value;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::PyError;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let args = platform::args();
    let status = run(&args);
    platform::exit(status)
}

fn usage() {
    platform::write_stderr(
        "usage: pythonc [-c code | script.py] [args...]\n\
         \n\
         Runs the Python subset implemented by this interpreter. There is no\n\
         CPython standard library, no threads, no processes and no network.\n",
    );
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
                platform::write_stderr("pythonc: argument expected for the -c option\n");
                return 2;
            }
            (
                String::from("<string>"),
                args[2].clone(),
                None,
                args[1..].to_vec(),
            )
        } else if command.starts_with('-') {
            platform::write_stderr(&format!("pythonc: unknown option: {}\n", command));
            usage();
            return 2;
        } else {
            let source = match platform::read_text(command) {
                Ok(source) => source,
                Err(message) => {
                    platform::write_stderr(&format!(
                        "pythonc: can't open file '{}': {}\n",
                        command, message
                    ));
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
            platform::write_stderr(&format!(
                "{}:{}:{}: {}: {}\n",
                filename, error.line, error.column, error.kind, error.message
            ));
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
    platform::write_stderr("Traceback (most recent call last):\n");
    for frame in &error.frames {
        platform::write_stderr(&format!(
            "  File \"{}\", line {}, in {}\n",
            frame.filename, frame.line, frame.name
        ));
    }
    if error.message.is_empty() {
        platform::write_stderr(&format!("{}\n", error.type_name()));
    } else {
        platform::write_stderr(&format!("{}: {}\n", error.type_name(), error.message));
    }
}
