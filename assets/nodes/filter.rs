pub fn execute(value: f32, threshold: f32) -> Option<f32> {
    if value >= threshold {
        Some(value)
    } else {
        None
    }
}
