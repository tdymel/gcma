fn main() {
    // A closed pipe (`ghma plan | head`) is not an error worth a panic message.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = info.to_string();
        if !text.contains("Broken pipe") {
            default_hook(info);
        }
    }));
    match std::panic::catch_unwind(ghma::main) {
        Ok(code) => std::process::exit(code),
        Err(payload) => {
            let broken_pipe = payload
                .downcast_ref::<String>()
                .is_some_and(|m| m.contains("Broken pipe"));
            std::process::exit(if broken_pipe { 141 } else { 101 });
        }
    }
}
