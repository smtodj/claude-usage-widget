//! Prints current usage to the terminal: `cargo run -p usage-core --example usage`
fn main() {
    match usage_core::current_usage() {
        Ok(u) => {
            for w in &u.windows {
                println!(
                    "{:<18} {:>5.1}% 남음  ({})",
                    w.label,
                    w.remaining_percent,
                    usage_core::format_reset(w.resets_at, u.fetched_at)
                );
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
