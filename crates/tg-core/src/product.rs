//! Product metadata and naming constants.
//!
//! Defined once here and mirrored in `macos/Config/Product.xcconfig`.
//! Do not hardcode these names anywhere else.

pub const DISPLAY_NAME: &str = "Niwa";
pub const CLI_BIN: &str = "niwa";
pub const CLI_CRATE: &str = "niwa-garden";
pub const HOMEBREW_FORMULA: &str = "niwa";
pub const HOMEBREW_CASK: &str = "niwa-garden";
pub const BUNDLE_ID: &str = "io.github.quiinthieu.niwa";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_constants_match_plan_spec() {
        assert_eq!(DISPLAY_NAME, "Niwa");
        assert_eq!(CLI_BIN, "niwa");
        assert_eq!(CLI_CRATE, "niwa-garden");
        assert_eq!(HOMEBREW_FORMULA, "niwa");
        assert_eq!(HOMEBREW_CASK, "niwa-garden");
        assert_eq!(BUNDLE_ID, "io.github.quiinthieu.niwa");
        assert_eq!(VERSION.split('.').count(), 3);
    }
}
