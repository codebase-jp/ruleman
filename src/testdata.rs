//! Fixtures shared by the modules' test suites.

/// SHA-256 of "abc".
pub(crate) const DIGEST_ABC: &str =
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
/// A well-formed digest that no real file has.
pub(crate) const DIGEST_ZEROS: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
