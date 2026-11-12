//! Headless command-line front end.

use rap_domain::APP_NAME;

/// One-line identification of the program and its version.
fn banner() -> String {
    format!("{APP_NAME} v{}", env!("CARGO_PKG_VERSION"))
}

fn main() {
    println!("{}", banner());
}

#[cfg(test)]
mod tests {
    use super::banner;

    #[test]
    fn banner_names_program_and_version() {
        let banner = banner();
        assert!(banner.starts_with("Realistic armor penetration by SernikNaZimno"));
        assert!(banner.ends_with(env!("CARGO_PKG_VERSION")));
    }
}
