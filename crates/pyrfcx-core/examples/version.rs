// B3 DoD verification: load the SDK and print the version.
//
// Run with: cargo run -p pyrfcx-core --example version
// Requires SAPNWRFC_HOME pointing to the SDK installation.

fn main() {
    match pyrfcx_core::ffi::get_version() {
        Ok((major, minor, patch, desc)) => {
            println!("SAP NW RFC SDK loaded successfully.");
            println!("Version: {major}.{minor}.{patch}");
            println!("Description: {desc}");
        }
        Err(e) => {
            eprintln!("ERROR: {e}");
            std::process::exit(1);
        }
    }
}
