pub fn execute(condition: bool, value: String) -> (Option<String>, Option<String>) {
    if condition {
        (Some(value), None)
    } else {
        (None, Some(value))
    }
}
