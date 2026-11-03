//! Native external-generator fixture; never shipped with Harvest.
use std::{env, fs, process};
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let lock = fs::read_to_string("bun.lock").expect("generator working directory");
    fs::write("invocation.txt", format!("{}\n{lock}", args.join("\n"))).expect("invocation");
    if env::var_os("HARVEST_GENERATOR_FAIL").is_some() {
        process::exit(7);
    }
    let output = args.windows(2).find(|pair| pair[0] == "-o").expect("output argument");
    fs::write(&output[1], "{ generated = true; }\r\n\r\n").expect("generator output");
}
