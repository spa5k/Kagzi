/// Returns the default greeting used throughout the Kagzi project.
pub fn greeting() -> &'static str {
    "Welcome to Kagzi!"
}

#[cfg(test)]
mod tests {
    use super::greeting;

    #[test]
    fn greeting_is_consistent() {
        assert_eq!(greeting(), "Welcome to Kagzi!");
    }
}
