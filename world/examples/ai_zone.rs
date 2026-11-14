//! 打印 AI 线上落在给定包围盒内的点：`ai_zone <fast_lane.ai> <x0> <z0> <x1> <z1> [步长]`
use vettura_world::AiSpline;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("用法: ai_zone <fast_lane.ai> <x0> <z0> <x1> <z1> [步长_m]");
        std::process::exit(2);
    };
    let nums: Vec<f32> = args.filter_map(|v| v.parse::<f32>().ok()).collect();
    if nums.len() < 4 {
        eprintln!("需要 x0 z0 x1 z1");
        std::process::exit(2);
    }
    let (x0, z0, x1, z1) = (nums[0], nums[1], nums[2], nums[3]);
    let step = nums.get(4).copied().unwrap_or(2.0);

    let line = AiSpline::from_path(&path).expect("解析失败");
    let length = line.lap_length_m();
    println!("lap_length={length:.1} zone=({x0},{z0})-({x1},{z1}) step={step}");
    println!(
        "{:>9} {:>9} {:>9} {:>9} {:>9}",
        "d_m", "x", "z", "v_mps", "v_kmh"
    );

    let mut distance = 0.0f32;
    while distance < length {
        if let Some(sample) = line.sample_at_distance(distance) {
            let p = sample.position;
            if p.x >= x0 && p.x <= x1 && p.z >= z0 && p.z <= z1 {
                let v = sample.speed_mps.unwrap_or(f32::NAN);
                println!(
                    "{:>9.1} {:>9.1} {:>9.1} {:>9.2} {:>9.1}",
                    distance,
                    p.x,
                    p.z,
                    v,
                    v * 3.6
                );
            }
        }
        distance += step;
    }
}
