pub fn execute(lhs: f32, rhs: f32, operation: &str) -> f32 {
    match operation {
        "sub" => lhs - rhs,
        "mul" => lhs * rhs,
        "div" if rhs != 0.0 => lhs / rhs,
        _ => lhs + rhs,
    }
}
