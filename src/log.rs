use colored::Colorize;

pub fn pretty_print(direction: &str, message: &str, quiet: bool) {
    if quiet {
        return;
    }
    let label = if direction.contains("Client") {
        format!("{}", direction.cyan())
    } else {
        format!("{}", direction.green())
    };
    match serde_json::from_str::<serde_json::Value>(message) {
        Ok(val) => {
            eprintln!("{}: {}", label, serde_json::to_string_pretty(&val).unwrap());
        }
        Err(_) => {
            eprintln!("{}: {}", label, message);
        }
    }
}

pub fn log(msg: &str, quiet: bool) {
    if !quiet {
        eprintln!("{} {}", "[stdio-to-ws]".bold(), msg);
    }
}

pub fn log_error(msg: &str, quiet: bool) {
    if !quiet {
        eprintln!("{} {}", "[stdio-to-ws]".red().bold(), msg);
    }
}
