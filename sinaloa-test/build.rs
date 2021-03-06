//! Sinaloa-test build script
//!
//! ## Authors
//!
//! The Veracruz Development Team.
//!
//! ## Licensing and copyright notice
//!
//! See the `LICENSE.markdown` file in the Veracruz root directory for
//! information on licensing and copyright.

use std::process::Command;

fn main() {
    // TODO: build.rs is run before the rest of the build. What we really need
    // is for the test collateral and the database to be built **after** the
    //rest of the build

    // Build the test collateral
    let make_result = Command::new("make")
        .current_dir("../test-collateral")
        .status()
        .unwrap();
    if !make_result.success() {
        panic!("sinaloa-test:build.rs: failed to make test-collateral");
    }

    // Destroy, and then re-create and repopulate, the Tabasco database
    Command::new("bash")
        .args(&["./populate_test_database.sh"])
        .output()
        .unwrap();
}
