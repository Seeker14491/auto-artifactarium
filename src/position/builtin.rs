use super::schema::Profile;

/// Field paths validated against the September 10 and 15 OSRELWin7.0.0 captures.
/// The embedded JSON is also a template for code-free protocol updates.
pub fn captured_7_0() -> Profile {
    Profile::parse(include_str!("../../profiles/position-7.0.json"))
        .expect("The built-in position profile must be valid")
}

/// Field paths validated against the September 23 OSRELWin7.1.0 capture.
pub fn captured_7_1() -> Profile {
    Profile::parse(include_str!("../../profiles/position-7.1.json"))
        .expect("The built-in position profile must be valid")
}
