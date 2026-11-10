use std::time::Duration;
use tokio::time::Instant;
#[tokio::main]
async fn main() {
    let n: u64 = std::env::args().nth(1).map_or(40, |s| s.parse().unwrap());
    let mut over = Vec::new();
    for i in 0..n {
        // Off-tick start, as a requeue computed mid-reconcile is.
        tokio::time::sleep(Duration::from_micros(137 * (i % 7) + 50)).await;
        let delay = Duration::from_millis(50) + Duration::from_micros((i * 9973) % 1000);
        let at = Instant::now() + delay;
        tokio::time::sleep_until(at).await;
        over.push((Instant::now() - at).as_secs_f64() * 1000.0);
    }
    over.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean: f64 = over.iter().sum::<f64>() / over.len() as f64;
    println!("tokio overshoot ms: n={} mean={:.3} median={:.3} min={:.3} max={:.3}", over.len(), mean, over[over.len() / 2], over[0], over[over.len() - 1]);
}
