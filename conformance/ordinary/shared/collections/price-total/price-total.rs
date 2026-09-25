pub async fn main() -> f64 {
    let price = 10.0;
    let quantity = 3.0;
    let tax = 2.0;
    let limit = 20.0;
    let active = true;
    let subtotal = price * quantity;
    let total = subtotal + tax;
    if total > limit && active {
        total
    } else {
        0.0
    }
}
