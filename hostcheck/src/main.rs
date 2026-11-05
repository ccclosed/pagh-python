//! Host-side harness: includes the interpreter sources and runs scripts with a
//! `std` platform, or parses a file for front-end tests.
//!
//! Usage:
//!   hostcheck <script.py> [args...]   run a script (same CLI as pythonc)
//!   hostcheck --parse <file>          parse only; print "ok" or the error
//!   hostcheck --tokens <file>         dump the token stream

extern crate alloc;

mod platform;

#[path = "../../src/ast.rs"]
mod ast;
#[path = "../../src/error.rs"]
mod error;
#[path = "../../src/lexer.rs"]
mod lexer;
#[path = "../../src/parser.rs"]
mod parser;
#[path = "../../src/value.rs"]
mod value;

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
                    std::process::exit(1);
                }
            },
            Err(error) => {
                eprintln!("hostcheck: {}: {}", args[2], error);
                std::process::exit(2);
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
                    std::process::exit(1);
                }
            },
            Err(error) => {
                eprintln!("hostcheck: {}: {}", args[2], error);
                std::process::exit(2);
            }
        }
        return;
    }
    eprintln!("hostcheck: the evaluator is not wired up yet");
    std::process::exit(2);
}